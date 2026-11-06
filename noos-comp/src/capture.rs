//! Capture d'écran pour les autres programmes : protocole `ext-image-copy-capture-v1`.
//!
//! C'est ce qu'utilise xdg-desktop-portal-wlr pour le partage d'écran (navigateurs, visio,
//! OBS via PipeWire) et grim pour les captures. Les applis Flatpak n'y ont pas accès
//! directement : elles doivent passer par le portail, qui demande l'accord de l'utilisateur.

use std::{cell::RefCell, time::Duration};

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier},
        renderer::{
            Bind, BufferType, ExportMem, Offscreen, Renderer, buffer_type,
            damage::OutputDamageTracker,
            element::{Element, Kind},
            gles::{GlesRenderbuffer, GlesRenderer},
        },
    },
    output::{Output, WeakOutput},
    reexports::wayland_server::{Client, protocol::wl_buffer::WlBuffer, protocol::wl_shm},
    utils::{Buffer as BufferCoords, IsAlive, Physical, Rectangle, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler, OutputCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason as Fail, DmabufConstraints, Frame, FrameRef,
            ImageCopyCaptureHandler, ImageCopyCaptureState, Session, SessionRef,
        },
        shm::with_buffer_contents_mut,
    },
};
use tracing::{debug, warn};

use crate::{render::NoosElement, state::NoosState};

/// Formats proposés au client (ordre de préférence).
const SHM_FORMATS: &[wl_shm::Format] =
    &[wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888, wl_shm::Format::Xbgr8888, wl_shm::Format::Abgr8888];
const DMA_FORMATS: &[Fourcc] = &[Fourcc::Xrgb8888, Fourcc::Argb8888, Fourcc::Xbgr8888, Fourcc::Abgr8888];

pub struct CaptureState {
    pub copy: ImageCopyCaptureState,
    pub output_source: OutputCaptureSourceState,
    sessions: Vec<Session>,
    frames: Vec<(SessionRef, Frame)>,
}

impl CaptureState {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> Self {
        let allowed =
            |client: &Client| client.get_data::<crate::state::ClientState>().is_some_and(|d| !d.sandboxed);
        Self {
            copy: ImageCopyCaptureState::new_with_filter::<NoosState, _>(dh, allowed.clone()),
            output_source: OutputCaptureSourceState::new_with_filter::<NoosState, _>(dh, allowed),
            sessions: Vec::new(),
            frames: Vec::new(),
        }
    }

    pub fn has_pending(&self) -> bool {
        !self.frames.is_empty()
    }
}

/// Client Flatpak/Snap ? Calculé à la connexion (le filtre de globals est appelé avec le
/// verrou interne de wayland-server : impossible d'y interroger le DisplayHandle).
pub fn stream_is_sandboxed(stream: &std::os::unix::net::UnixStream) -> bool {
    let Ok(cred) = smithay::reexports::rustix::net::sockopt::socket_peercred(stream) else { return true };
    let pid = cred.pid.as_raw_nonzero();
    if std::path::Path::new(&format!("/proc/{pid}/root/.flatpak-info")).exists() {
        return true;
    }
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .map(|c| c.contains("app-flatpak-") || c.contains("snap."))
        .unwrap_or(false)
}

fn source_output(source: &ImageCaptureSource) -> Option<Output> {
    source.user_data().get::<WeakOutput>().and_then(|w| w.upgrade())
}

/// Taille du tampon de capture : le mode de l'écran, orienté comme à l'affichage.
pub fn buffer_size(output: &Output) -> Option<Size<i32, Physical>> {
    let mode = output.current_mode()?;
    Some(output.current_transform().transform_size(mode.size))
}

impl NoosState {
    fn capture_constraints_for(&self, output: &Output) -> Option<BufferConstraints> {
        let size = buffer_size(output)?;
        let dma = self.backend.capture_dma_formats().map(|(node, formats)| DmabufConstraints {
            node,
            formats: DMA_FORMATS
                .iter()
                .filter_map(|code| {
                    let mods: Vec<Modifier> =
                        formats.iter().filter(|f| f.code == *code).map(|f| f.modifier).collect();
                    (!mods.is_empty()).then_some((*code, mods))
                })
                .collect(),
        });
        Some(BufferConstraints { size: (size.w, size.h).into(), shm: SHM_FORMATS.to_vec(), dma })
    }

    /// Une capture attend-elle cet écran ? (le backend doit alors appeler `process_captures`)
    pub fn capture_wanted(&self, output: &Output) -> bool {
        self.capture
            .frames
            .iter()
            .any(|(s, _)| source_output(&s.source()).as_ref() == Some(output))
    }

    /// Remplit les tampons des captures en attente pour `output`, avec les éléments
    /// qui viennent d'être affichés. Une image n'est envoyée que si l'écran a changé
    /// depuis la précédente (sinon elle attend la prochaine frame).
    pub fn process_captures(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        elements: &[NoosElement],
        clear: [f32; 4],
    ) {
        if self.capture.frames.is_empty() {
            return;
        }
        let now: Duration = self.clock.now().into();
        let pending = std::mem::take(&mut self.capture.frames);
        for (session, frame) in pending {
            if !session.alive() {
                continue;
            }
            let Some(target) = source_output(&session.source()) else {
                frame.fail(Fail::Stopped);
                continue;
            };
            if &target != output {
                self.capture.frames.push((session, frame));
                continue;
            }
            let Some(size) = buffer_size(output) else {
                frame.fail(Fail::Unknown);
                continue;
            };
            // L'écran a changé de résolution/orientation : nouvelles contraintes.
            if session.current_constraints().map(|c| c.size) != Some((size.w, size.h).into()) {
                if let Some(c) = self.capture_constraints_for(output) {
                    session.update_constraints(c);
                }
                frame.fail(Fail::BufferConstraints);
                continue;
            }

            let draw_cursor = session.draw_cursor();
            let elems: Vec<&NoosElement> =
                elements.iter().filter(|e| draw_cursor || e.kind() != Kind::Cursor).collect();
            let scale = output.current_scale().fractional_scale();

            // Suivi des changements propre à chaque session.
            let changed = {
                session
                    .user_data()
                    .insert_if_missing(|| RefCell::new(OutputDamageTracker::new(size, scale, Transform::Normal)));
                let tracker = session.user_data().get::<RefCell<OutputDamageTracker>>().unwrap();
                let mut tracker = tracker.borrow_mut();
                match tracker.damage_output(1, &elems) {
                    Ok((Some(d), _)) => !d.is_empty(),
                    Ok((None, _)) => false,
                    Err(_) => true,
                }
            };
            let first = session.user_data().get::<FirstFrameDone>().is_none();
            if !changed && !first {
                self.capture.frames.push((session, frame));
                continue;
            }
            session.user_data().insert_if_missing(|| FirstFrameDone);

            match render_into(renderer, &frame.buffer(), size, scale, &elems, clear) {
                Ok(()) => {
                    let full = Rectangle::<i32, BufferCoords>::from_size((size.w, size.h).into());
                    frame.success(Transform::Normal, vec![full], now);
                }
                Err(reason) => frame.fail(reason),
            }
        }
    }
}

struct FirstFrameDone;

fn render_into(
    renderer: &mut GlesRenderer,
    buffer: &WlBuffer,
    size: Size<i32, Physical>,
    scale: f64,
    elements: &[&NoosElement],
    clear: [f32; 4],
) -> Result<(), Fail> {
    let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
    match buffer_type(buffer) {
        Some(BufferType::Dma) => {
            let mut dmabuf = get_dmabuf(buffer).map_err(|_| Fail::BufferConstraints)?.clone();
            let mut fb = renderer.bind(&mut dmabuf).map_err(|e| {
                warn!("capture : liaison du dmabuf impossible : {e:?}");
                Fail::Unknown
            })?;
            let sync = tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| {
                    warn!("capture : rendu impossible : {e:?}");
                    Fail::Unknown
                })?
                .sync
                .clone();
            let _ = renderer.wait(&sync);
            Ok(())
        }
        Some(BufferType::Shm) => {
            let pixels = render_offscreen(renderer, size, scale, elements, clear).ok_or(Fail::Unknown)?;
            copy_to_shm(buffer, &pixels, size)
        }
        _ => Err(Fail::BufferConstraints),
    }
}

/// Rendu hors écran → pixels RGBA (lignes de haut en bas).
pub fn render_offscreen(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    scale: f64,
    elements: &[&NoosElement],
    clear: [f32; 4],
) -> Option<Vec<u8>> {
    let mut tracker = OutputDamageTracker::new(size, scale, Transform::Normal);
    let buf_size: Size<i32, BufferCoords> = (size.w, size.h).into();
    let mut rb: GlesRenderbuffer = renderer
        .create_buffer(Fourcc::Abgr8888, buf_size)
        .map_err(|e| warn!("capture : tampon hors écran impossible : {e:?}"))
        .ok()?;
    let mut fb = renderer.bind(&mut rb).ok()?;
    tracker
        .render_output(renderer, &mut fb, 0, elements, clear)
        .map_err(|e| warn!("capture : rendu impossible : {e:?}"))
        .ok()?;
    let mapping = renderer.copy_framebuffer(&fb, Rectangle::from_size(buf_size), Fourcc::Abgr8888).ok()?;
    renderer.map_texture(&mapping).ok().map(|p| p.to_vec())
}

impl NoosState {
    /// Capture demandée par l'IPC (`Screenshot`) en mode DRM : rendu hors écran → PNG.
    pub fn take_pending_screenshot(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        elements: &[NoosElement],
        clear: [f32; 4],
    ) {
        let Some(path) = self.pending_screenshot.take() else { return };
        let Some(size) = buffer_size(output) else { return };
        let elems: Vec<&NoosElement> = elements.iter().filter(|e| e.kind() != Kind::Cursor).collect();
        let scale = output.current_scale().fractional_scale();
        match render_offscreen(renderer, size, scale, &elems, clear) {
            Some(pixels) => crate::screenshot::write_rgba(&path, size.w as u32, size.h as u32, &pixels),
            None => warn!("capture d'écran impossible"),
        }
    }
}

/// Copie RGBA (ligne par ligne, en respectant le pas du tampon client) avec permutation
/// R/B pour les formats ARGB/XRGB (BGRA en mémoire).
fn copy_to_shm(buffer: &WlBuffer, rgba: &[u8], size: Size<i32, Physical>) -> Result<(), Fail> {
    with_buffer_contents_mut(buffer, |ptr, len, data| {
        let w = size.w as usize;
        let h = size.h as usize;
        let stride = data.stride as usize;
        let offset = data.offset as usize;
        if data.width < size.w || data.height < size.h || offset + stride * (h - 1) + w * 4 > len {
            return Err(Fail::BufferConstraints);
        }
        let swap = matches!(data.format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888);
        // SAFETY : bornes vérifiées ci-dessus ; le pool reste valide pendant l'appel.
        let dst = unsafe { std::slice::from_raw_parts_mut(ptr.add(offset), len - offset) };
        for y in 0..h {
            let src = &rgba[y * w * 4..(y + 1) * w * 4];
            let row = &mut dst[y * stride..y * stride + w * 4];
            if swap {
                for (d, s) in row.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
            } else {
                row.copy_from_slice(src);
            }
        }
        Ok(())
    })
    .map_err(|_| Fail::BufferConstraints)?
}

// ── Gestionnaires de protocole ──

impl ImageCaptureSourceHandler for NoosState {}

impl OutputCaptureSourceHandler for NoosState {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.capture.output_source
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ImageCopyCaptureHandler for NoosState {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.capture.copy
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = source_output(source)?;
        self.capture_constraints_for(&output)
    }

    fn new_session(&mut self, session: Session) {
        debug!("nouvelle session de capture d'écran");
        self.capture.sessions.push(session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        // Traitée au prochain passage de rendu de l'écran (au plus une frame plus tard).
        self.capture.frames.push((session.clone(), frame));
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        self.capture.frames.retain(|(_, f)| *f != frame);
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.capture.frames.retain(|(s, _)| *s != session);
        self.capture.sessions.retain(|s| s.as_ref() != session);
    }
}

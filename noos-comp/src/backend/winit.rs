//! Backend imbriqué : NOOS s'affiche dans une fenêtre de la session courante (GNOME…).
//! Idéal pour développer sans quitter son bureau : `cargo run -p noos-comp -- --winit`.
//! `NOOS_WINIT_OUTPUTS=2` (ou 3) : la fenêtre est découpée en plusieurs écrans côte à côte,
//! pour tester le multi-écrans sans second moniteur.

use std::time::Duration;

use noos_core::ipc::{ModeInfo, OutputInfo};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        egl::EGLDevice,
        renderer::{
            ImportDma, ImportMemWl,
            damage::OutputDamageTracker,
            element::utils::{Relocate, RelocateRenderElement},
            gles::GlesRenderer,
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{calloop::EventLoop, wayland_server::Display},
    utils::{Logical, Point, Transform},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufState},
};
use tracing::warn;

use crate::{backend::Backend, render, state::NoosState};

pub struct WinitData {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    /// Écrans simulés, de gauche à droite dans la fenêtre.
    pub outputs: Vec<Output>,
    pub damage_tracker: OutputDamageTracker,
    last_render: std::time::Instant,
    swap_interval_set: bool,
}

impl WinitData {
    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        self.backend.renderer().import_dmabuf(dmabuf, None).is_ok()
    }

    /// Mode imbriqué : la « résolution » est la taille de la fenêtre hôte, que l'on change à la
    /// souris (winit applique les redimensionnements programmés sans prévenir Smithay).
    pub fn set_mode(&mut self, _width: i32, _height: i32) -> Result<Output, String> {
        Err("en mode imbriqué, redimensionnez la fenêtre de développement à la souris".into())
    }

    /// Position d'un point de la fenêtre hôte dans l'espace des écrans.
    pub fn window_to_global(&self, p: Point<f64, Logical>, space: &smithay::desktop::Space<smithay::desktop::Window>) -> Point<f64, Logical> {
        let n = self.outputs.len().max(1) as f64;
        let w = self.backend.window_size().w as f64 / n;
        let k = ((p.x / w).floor().max(0.0) as usize).min(self.outputs.len() - 1);
        let o = &self.outputs[k];
        let g = space.output_geometry(o).unwrap_or_default();
        Point::from((g.loc.x as f64 + p.x - w * k as f64, g.loc.y as f64 + p.y))
    }

    pub fn output_infos(&self) -> Vec<OutputInfo> {
        self.outputs.iter().map(|o| Self::output_info(o)).collect()
    }

    fn output_info(output: &Output) -> OutputInfo {
        let mode = output.current_mode().unwrap_or(Mode { size: (0, 0).into(), refresh: 60_000 });
        let modes = vec![ModeInfo { width: mode.size.w, height: mode.size.h, refresh: mode.refresh, preferred: true }];
        OutputInfo {
            name: output.name(),
            make: "NOOS".into(),
            model: "Fenêtre de développement".into(),
            width: mode.size.w,
            height: mode.size.h,
            refresh: mode.refresh,
            scale: output.current_scale().fractional_scale(),
            modes,
            transform: "normal".into(),
            can_set_mode: false,
            vrr_capable: false,
            vrr_active: false,
            hdr_capable: false,
            hdr_active: false,
            format: "ARGB8888".into(),
            nested: true,
            ..Default::default()
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    let mut event_loop: EventLoop<'static, NoosState> = EventLoop::try_new()?;
    let display: Display<NoosState> = Display::new()?;
    let dh = display.handle();

    // NOOS_WINIT_SIZE=2560x800 : taille de la fenêtre de développement (tests multi-écrans).
    let size = std::env::var("NOOS_WINIT_SIZE").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
    });
    let (mut backend, winit) = match size {
        Some((w, h)) => winit::init_from_attributes::<GlesRenderer>(
            smithay::reexports::winit::window::WindowAttributes::default()
                .with_surface_size(smithay::reexports::winit::dpi::LogicalSize::new(w as f64, h as f64))
                .with_title("NOOS — compositeur (mode imbriqué)")
                .with_visible(true),
        ),
        None => winit::init::<GlesRenderer>(),
    }
    .map_err(|e| anyhow::anyhow!("initialisation winit : {e}"))?;
    backend.window().set_title("NOOS — compositeur (mode imbriqué)");

    let count = std::env::var("NOOS_WINIT_OUTPUTS").ok().and_then(|v| v.parse::<i32>().ok()).unwrap_or(1).clamp(1, 3);
    let win = backend.window_size();
    let mut outputs = vec![];
    for k in 0..count {
        let mode = Mode { size: (win.w / count, win.h).into(), refresh: 60_000 };
        let output = Output::new(
            format!("NOOS-{}", k + 1),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "NOOS".into(),
                model: if count > 1 { format!("Écran simulé {}", k + 1) } else { "winit".into() },
                serial_number: k.to_string(),
            },
        );
        std::mem::forget(output.create_global::<NoosState>(&dh));
        // Un seul écran : transformation Flipped180 (rendu GL à l'envers dans la fenêtre).
        // Plusieurs : rendu commun, la transformation est appliquée par le suivi des dégâts.
        let t = if count == 1 { Transform::Flipped180 } else { Transform::Normal };
        output.change_current_state(Some(mode), Some(t), None, Some((mode.size.w * k, 0).into()));
        output.set_preferred(mode);
        outputs.push(output);
    }
    let output = outputs[0].clone();

    // linux-dmabuf : indispensable pour les clients GTK4 accélérés (GL/Vulkan).
    let mut dmabuf_state = DmabufState::new();
    let formats = backend.renderer().dmabuf_formats();
    let render_node = EGLDevice::device_for_display(backend.renderer().egl_context().display())
        .and_then(|d| d.try_get_render_node());
    let dmabuf_global = match render_node {
        Ok(Some(node)) => {
            let feedback = DmabufFeedbackBuilder::new(node.dev_id(), formats).build().unwrap();
            dmabuf_state.create_global_with_default_feedback::<NoosState>(&dh, &feedback)
        }
        _ => {
            warn!("pas de render node : dmabuf v3");
            dmabuf_state.create_global::<NoosState>(&dh, formats)
        }
    };

    let damage_tracker = if count == 1 { OutputDamageTracker::from_output(&output) } else { OutputDamageTracker::new(win, 1.0, Transform::Flipped180) };
    let shm_formats = backend.renderer().shm_formats();
    let data = WinitData { backend, outputs: outputs.clone(), damage_tracker, last_render: std::time::Instant::now(), swap_interval_set: false };

    let mut state = NoosState::new(display, event_loop.handle(), event_loop.get_signal(), Backend::Winit(data));
    state.dmabuf_state = Some((dmabuf_state, dmabuf_global));
    state.shm_state.update_formats(shm_formats);
    for o in &outputs {
        let loc = o.current_location();
        state.space.map_output(o, loc);
    }
    state.apply_output_layout();

    event_loop
        .handle()
        .insert_source(winit, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } => {
                let n = outputs.len() as i32;
                for o in &outputs {
                    let mode = Mode { size: (size.w / n, size.h).into(), refresh: 60_000 };
                    o.change_current_state(Some(mode), None, None, None);
                    o.set_preferred(mode);
                }
                if n > 1 {
                    if let Backend::Winit(d) = &mut state.backend {
                        d.damage_tracker = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
                    }
                }
                state.apply_output_layout();
                state.update_vrr();
            }
            WinitEvent::Input(event) => state.process_input_event(event),
            WinitEvent::Redraw => {}
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })
        .map_err(|e| anyhow::anyhow!("source winit : {e}"))?;

    crate::startup(&mut state);

    // Rendu cadencé par la boucle (~60 Hz) et non par les événements Redraw de winit :
    // ceux-ci attendent un « frame callback » de l'hôte, qui n'arrive jamais si la frame
    // précédente n'avait rien à afficher (pas de commit) → gel de l'affichage.
    event_loop.run(Some(Duration::from_millis(4)), &mut state, |state| {
        let due = match &state.backend {
            Backend::Winit(d) => d.last_render.elapsed() >= Duration::from_micros(16_600),
            _ => false,
        };
        if due {
            state.render_winit();
        }
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}

impl NoosState {
    fn render_winit(&mut self) {
        let Backend::Winit(mut data) = std::mem::replace(&mut self.backend, Backend::Detached) else {
            return;
        };
        data.last_render = std::time::Instant::now();
        let outputs = data.outputs.clone();
        let clear = render::clear_color(self);
        // Capture demandée : rendu complet pour ne pas lire un tampon périmé.
        let age = if self.pending_screenshot.is_some() { 0 } else { data.backend.buffer_age().unwrap_or(0) };
        let size = data.backend.window_size();
        let slice = size.w / outputs.len().max(1) as i32;

        let swap_set = data.swap_interval_set;
        let mut swap_now_set = false;
        let result = match data.backend.bind() {
            Ok((renderer, mut fb)) => {
                // Mesa bloque eglSwapBuffers jusqu'au « frame callback » de l'hôte (intervalle 1
                // par défaut, que Smithay ne modifie pas). Si la fenêtre est cachée côté GNOME,
                // tout le compositeur se figeait (IPC compris) : on passe l'intervalle à 0.
                // Dans une fenêtre, c'est le compositeur hôte qui dessine le curseur par défaut.
                let res = if outputs.len() == 1 {
                    let elements = render::output_elements(self, renderer, &outputs[0], None);
                    data.damage_tracker.render_output(renderer, &mut fb, age, &elements, clear).map(|r| r.damage.cloned()).map_err(|e| format!("{e:?}"))
                } else {
                    // Plusieurs écrans simulés : chacun décalé dans sa tranche de la fenêtre.
                    let mut elements = vec![];
                    for (k, o) in outputs.iter().enumerate() {
                        let off: Point<i32, smithay::utils::Physical> = (slice * k as i32, 0).into();
                        elements.extend(
                            render::output_elements(self, renderer, o, None)
                                .into_iter()
                                .map(|e| RelocateRenderElement::from_element(e, off, Relocate::Relative)),
                        );
                    }
                    data.damage_tracker.render_output(renderer, &mut fb, age, &elements, clear).map(|r| r.damage.cloned()).map_err(|e| format!("{e:?}"))
                };
                // Après le rendu : le contexte est alors forcément actif sur la surface de la
                // fenêtre (avant, Smithay ne l'active pas toujours → EGL_BAD_CONTEXT). On
                // réessaie à chaque frame tant que l'appel échoue.
                if !swap_set {
                    let display = renderer.egl_context().display().get_display_handle();
                    let ok = unsafe { smithay::backend::egl::ffi::egl::SwapInterval(display.handle, 0) };
                    swap_now_set = ok == 1;
                }
                if let Some(path) = self.pending_screenshot.take() {
                    crate::screenshot::capture(renderer, &fb, size, true, &path);
                }
                res
            }
            Err(e) => Err(format!("{e:?}")),
        };
        match result {
            Ok(Some(damage)) => {
                if let Err(e) = data.backend.submit(Some(&damage)) {
                    warn!("submit : {e}");
                }
            }
            Ok(None) => {}
            Err(e) => warn!("rendu winit : {e}"),
        }
        if swap_now_set {
            data.swap_interval_set = true;
        }
        // Captures (après submit : on ne touche pas au tampon de la fenêtre).
        for output in &outputs {
            if self.capture.has_pending() && self.capture_wanted(output) {
                let renderer = data.backend.renderer();
                let elements = render::output_elements(self, renderer, output, None);
                self.process_captures(renderer, output, &elements, clear);
            }
        }
        let cursor_visible = matches!(self.cursor_status, smithay::input::pointer::CursorImageStatus::Named(_));
        data.backend.window().set_cursor_visible(cursor_visible);
        self.backend = Backend::Winit(data);

        for output in &outputs {
            self.send_frames(output);
        }
    }
}

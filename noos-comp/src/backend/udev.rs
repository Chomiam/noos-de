//! Backend natif : DRM/KMS + libinput + libseat. C'est ce qui tourne dans la vraie session
//! lancée depuis GDM.
//!
//! Simplification volontaire du prototype : un seul GPU (le principal). Les écrans branchés
//! sur un autre GPU sont ignorés.
//!
//! Gaming :
//! - **VRR** : propriété `VRR_ENABLED` du CRTC, pilotée par la politique de `noos.toml`
//!   (`off` / `always` / `fullscreen`).
//! - **Scanout direct** : quand une fenêtre est en plein écran, seule elle est rendue ; le
//!   `DrmCompositor` de Smithay la place alors directement sur le plan primaire, sans copie.
//! - **10 bits** : on essaie d'abord les formats `ARGB2101010`.
//! - **HDR** : détection (EDID + pilote). La sortie HDR elle-même demande un pipeline couleur
//!   (encodage PQ, métadonnées `HDR_OUTPUT_METADATA`, protocole color-management) qui n'est
//!   pas encore écrit.

use std::{collections::HashMap, path::Path, time::Duration};

use noos_core::{
    NoosConfig,
    ipc::{ModeInfo, OutputInfo},
};
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            dmabuf::Dmabuf,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, NodeType, VrrSupport,
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDevice, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma, ImportMemWl, element::memory::MemoryRenderBuffer, gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu},
    },
    desktop::{Space, Window, layer_map_for_output, utils::OutputPresentationFeedback},
    output::{Mode as WlMode, Output, PhysicalProperties, Scale},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device as _, ModeTypeFlags, connector, crtc},
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{Display, DisplayHandle, backend::GlobalId, protocol::wl_surface::WlSurface},
    },
    utils::{DeviceFd, Transform},
    wayland::{
        dmabuf::{DmabufFeedbackBuilder, DmabufState},
        presentation::Refresh,
    },
};
use smithay_drm_extras::{
    display_info,
    drm_scanner::{DrmScanEvent, DrmScanner},
};
use tracing::{error, info, warn};

use crate::{
    backend::Backend,
    cursor::Cursor,
    render::{self, NoosElement},
    state::NoosState,
};

const FORMATS_10BIT: &[Fourcc] = &[Fourcc::Abgr2101010, Fourcc::Argb2101010, Fourcc::Abgr8888, Fourcc::Argb8888];
const FORMATS_8BIT: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type Feedback = Option<OutputPresentationFeedback>;
type NoosDrmOutput = DrmOutput<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;
type NoosDrmOutputManager =
    DrmOutputManager<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, Feedback, DrmDeviceFd>;

struct Surface {
    dh: DisplayHandle,
    output: Output,
    global: Option<GlobalId>,
    drm_output: NoosDrmOutput,
    vrr_support: VrrSupport,
    vrr_active: bool,
    hdr_capable: bool,
    modes: Vec<ModeInfo>,
    drm_modes: Vec<smithay::reexports::drm::control::Mode>,
    make: String,
    model: String,
    /// Un rendu est déjà programmé (évite les doublons de timers).
    render_scheduled: bool,
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.output.leave_all();
        if let Some(global) = self.global.take() {
            self.dh.remove_global::<NoosState>(global);
        }
    }
}

struct Gpu {
    node: DrmNode,
    /// Rendu logiciel : attente explicite de la fin du dessin (voir render_surface).
    software: bool,
    renderer: GlesRenderer,
    output_manager: NoosDrmOutputManager,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    _token: RegistrationToken,
}

pub struct UdevData {
    session: LibSeatSession,
    primary_gpu: DrmNode,
    gpu: Option<Gpu>,
    libinput: Libinput,
    /// Claviers branchés : leurs voyants (Verr Num, Verr Maj, Arrêt Défil) suivent l'état xkb.
    keyboards: Vec<smithay::reexports::input::Device>,
    cursor: Cursor,
    cursor_cache: Vec<(xcursor::parser::Image, MemoryRenderBuffer)>,
    ten_bit: bool,
}

impl UdevData {
    /// Allume / éteint les voyants de tous les claviers (le noyau ne le fait pas tout seul :
    /// c'est au compositeur de répercuter l'état de xkb).
    pub fn set_leds(&mut self, leds: smithay::input::keyboard::LedState) {
        let leds = led_flags(leds);
        for kb in &mut self.keyboards {
            kb.led_update(leds);
        }
    }

    pub fn seat_name(&self) -> String {
        self.session.seat()
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        // Import anticipé des buffers : utile en multi-GPU uniquement.
        let _ = surface;
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        let Some(gpu) = self.gpu.as_mut() else { return false };
        let ok = gpu.renderer.import_dmabuf(dmabuf, None).is_ok();
        if ok && dmabuf.node().is_none() {
            dmabuf.set_node(self.primary_gpu);
        }
        ok
    }

    pub fn capture_dma_formats(&self) -> Option<(DrmNode, smithay::backend::allocator::format::FormatSet)> {
        let gpu = self.gpu.as_ref()?;
        let node = gpu.node.node_with_type(NodeType::Render).and_then(Result::ok).unwrap_or(gpu.node);
        Some((node, gpu.renderer.egl_context().dmabuf_render_formats().clone()))
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            warn!(vt, "changement de VT impossible : {e}");
        }
    }

    pub fn set_vrr(&mut self, output: &Output, enabled: bool) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        for surface in gpu.surfaces.values_mut() {
            if &surface.output != output || surface.vrr_support == VrrSupport::NotSupported {
                continue;
            }
            if surface.vrr_active == enabled {
                continue;
            }
            match surface.drm_output.with_compositor(|c| c.use_vrr(enabled)) {
                Ok(()) => {
                    surface.vrr_active = enabled;
                    info!(output = output.name(), enabled, "VRR");
                }
                Err(e) => warn!(output = output.name(), "VRR : {e:?}"),
            }
        }
    }

    /// DPMS : éteint (clear) ou rallume (le prochain rendu réactive l'écran).
    pub fn set_power(&mut self, on: bool) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        for s in gpu.surfaces.values_mut() {
            if !on {
                if let Err(e) = s.drm_output.with_compositor(|c| c.clear()) {
                    warn!(output = s.output.name(), "extinction : {e:?}");
                }
            }
            s.render_scheduled = false;
        }
    }

    pub fn set_mode(&mut self, name: &str, width: i32, height: i32, refresh: i32) -> Result<Output, String> {
        let gpu = self.gpu.as_mut().ok_or("aucun GPU")?;
        let (crtc, mode, output) = {
            let (crtc, s) = gpu.surfaces.iter().find(|(_, s)| s.output.name() == name).ok_or("écran inconnu")?;
            let mode = s
                .drm_modes
                .iter()
                .filter(|m| m.size() == (width as u16, height as u16))
                .min_by_key(|m| (refresh_mhz(m) as i64 - refresh as i64).abs())
                .copied()
                .ok_or("mode non proposé par cet écran")?;
            (*crtc, mode, s.output.clone())
        };
        gpu.output_manager
            .lock()
            .use_mode::<_, NoosElement>(&crtc, mode, &mut gpu.renderer, &DrmOutputRenderElements::default())
            .map_err(|e| format!("changement de mode refusé : {e}"))?;
        output.change_current_state(Some(WlMode::from(mode)), None, None, None);
        info!(output = name, width, height, refresh, "nouveau mode");
        Ok(output)
    }

    pub fn apply_config(&mut self, config: &NoosConfig) {
        self.ten_bit = config.gaming.ten_bit;
        let Some(gpu) = self.gpu.as_mut() else { return };
        for surface in gpu.surfaces.values() {
            if let Some(scale) = config.output(&surface.output.name()).and_then(|o| o.scale) {
                surface.output.change_current_state(None, None, Some(Scale::Fractional(scale)), None);
                layer_map_for_output(&surface.output).arrange();
            }
        }
    }

    pub fn output_infos(&self, _space: &Space<Window>, _config: &NoosConfig) -> Vec<OutputInfo> {
        let Some(gpu) = self.gpu.as_ref() else { return vec![] };
        let mut infos: Vec<OutputInfo> = gpu
            .surfaces
            .values()
            .map(|s| {
                let mode = s.output.current_mode().unwrap_or(WlMode { size: (0, 0).into(), refresh: 0 });
                let format = s.drm_output.format();
                let name = s.output.name();
                OutputInfo {
                    hdr_active: false,
                    transform: crate::wm::transform_name(s.output.current_transform()).into(),
                    can_set_mode: true,
                    vrr_capable: s.vrr_support != VrrSupport::NotSupported,
                    vrr_active: s.vrr_active,
                    hdr_capable: s.hdr_capable,
                    width: mode.size.w,
                    height: mode.size.h,
                    refresh: mode.refresh,
                    scale: s.output.current_scale().fractional_scale(),
                    modes: s.modes.clone(),
                    make: s.make.clone(),
                    model: s.model.clone(),
                    format: format!("{format}"),
                    nested: false,
                    name,
                }
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }
}

pub fn run() -> anyhow::Result<()> {
    let mut event_loop: EventLoop<'static, NoosState> = EventLoop::try_new()?;
    let display: Display<NoosState> = Display::new()?;

    let (session, notifier) =
        LibSeatSession::new().map_err(|e| anyhow::anyhow!("impossible d'ouvrir une session libseat : {e}"))?;
    let seat = session.seat();

    let primary_gpu = primary_gpu(&seat)?
        .and_then(|p| DrmNode::from_path(p).ok()?.node_with_type(NodeType::Render)?.ok())
        .or_else(|| all_gpus(&seat).ok()?.into_iter().find_map(|p| DrmNode::from_path(p).ok()))
        .ok_or_else(|| anyhow::anyhow!("aucun GPU trouvé"))?;
    info!(%primary_gpu, "GPU principal");

    let mut libinput = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|_| anyhow::anyhow!("libinput : siège {seat} introuvable"))?;

    let config = NoosConfig::load();
    let data = UdevData {
        session,
        primary_gpu,
        gpu: None,
        libinput: libinput.clone(),
        keyboards: Vec::new(),
        cursor: Cursor::load(),
        cursor_cache: Vec::new(),
        ten_bit: config.gaming.ten_bit,
    };
    let mut state =
        NoosState::new(display, event_loop.handle(), event_loop.get_signal(), Backend::Udev(Box::new(data)));

    // ── Entrées ──
    event_loop
        .handle()
        .insert_source(LibinputInputBackend::new(libinput.clone()), |mut event, _, state| {
            match &mut event {
                InputEvent::DeviceAdded { device } => {
                    configure_device(device, &state.config);
                    if device.has_capability(smithay::reexports::input::DeviceCapability::Keyboard) {
                        // Nouveau clavier (ou reprise après changement de VT) : voyants à jour.
                        if let Some(kb) = state.seat.get_keyboard() {
                            device.led_update(led_flags(kb.led_state()));
                        }
                        if let Backend::Udev(data) = &mut state.backend {
                            data.keyboards.retain(|d| d.sysname() != device.sysname());
                            data.keyboards.push(device.clone());
                        }
                    }
                }
                InputEvent::DeviceRemoved { device } => {
                    if let Backend::Udev(data) = &mut state.backend {
                        data.keyboards.retain(|d| d.sysname() != device.sysname());
                    }
                }
                _ => {}
            }
            state.process_input_event(event);
        })
        .map_err(|e| anyhow::anyhow!("source libinput : {e}"))?;

    // ── Session (changement de VT) ──
    event_loop
        .handle()
        .insert_source(notifier, move |event, _, state| {
            let Backend::Udev(data) = &mut state.backend else { return };
            match event {
                SessionEvent::PauseSession => {
                    info!("session en pause (changement de VT)");
                    data.libinput.suspend();
                    if let Some(gpu) = data.gpu.as_mut() {
                        gpu.output_manager.pause();
                    }
                }
                SessionEvent::ActivateSession => {
                    info!("session réactivée");
                    if data.libinput.resume().is_err() {
                        error!("libinput : reprise impossible");
                    }
                    if let Some(gpu) = data.gpu.as_mut() {
                        if let Err(e) = gpu.output_manager.lock().activate(false) {
                            error!("réactivation DRM : {e}");
                        }
                        for s in gpu.surfaces.values_mut() {
                            s.render_scheduled = false;
                        }
                    }
                    state.loop_handle.insert_idle(|state| state.render_all());
                }
            }
        })
        .map_err(|e| anyhow::anyhow!("source session : {e}"))?;

    // ── GPU + écrans ──
    let udev = UdevBackend::new(&seat)?;
    let primary_node = primary_gpu.node_with_type(NodeType::Primary).and_then(|n| n.ok());
    let device = udev
        .device_list()
        .find(|(id, _)| primary_node.is_some_and(|n| n.dev_id() == *id) || *id == primary_gpu.dev_id())
        .map(|(id, path)| (id, path.to_path_buf()));
    match device {
        Some((id, path)) => {
            let node = DrmNode::from_dev_id(id)?;
            state.device_added(node, &path)?;
        }
        None => anyhow::bail!("périphérique DRM du GPU principal introuvable"),
    }

    event_loop
        .handle()
        .insert_source(udev, |event, _, state| match event {
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    state.device_changed(node);
                }
            }
            UdevEvent::Added { .. } | UdevEvent::Removed { .. } => {
                // Multi-GPU / GPU externes : pas encore géré.
            }
        })
        .map_err(|e| anyhow::anyhow!("source udev : {e}"))?;

    crate::startup(&mut state);

    event_loop.run(Some(Duration::from_millis(16)), &mut state, |state| {
        state.space.refresh();
        state.popups.cleanup();
        let _ = state.display_handle.flush_clients();
    })?;
    Ok(())
}

fn led_flags(leds: smithay::input::keyboard::LedState) -> smithay::reexports::input::Led {
    use smithay::reexports::input::Led;
    let mut flags = Led::empty();
    if leds.num == Some(true) {
        flags |= Led::NUMLOCK;
    }
    if leds.caps == Some(true) {
        flags |= Led::CAPSLOCK;
    }
    if leds.scroll == Some(true) {
        flags |= Led::SCROLLLOCK;
    }
    flags
}

fn configure_device(device: &mut smithay::reexports::input::Device, config: &NoosConfig) {
    if device.has_capability(DeviceCapability::Pointer) {
        let _ = device.config_accel_set_speed(config.input.pointer_accel);
        if device.config_tap_finger_count() > 0 {
            let _ = device.config_tap_set_enabled(config.input.tap_to_click);
        }
        if device.config_scroll_has_natural_scroll() {
            let _ = device.config_scroll_set_natural_scroll_enabled(config.input.natural_scroll);
        }
    }
}

fn parse_mode(spec: &str) -> Option<(u16, u16, Option<u32>)> {
    let (size, hz) = match spec.split_once('@') {
        Some((s, h)) => (s, h.trim().parse::<f64>().ok().map(|h| (h * 1000.0).round() as u32)),
        None => (spec, None),
    };
    let (w, h) = size.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?, hz))
}

/// Fréquence d'un mode DRM en mHz.
fn refresh_mhz(mode: &smithay::reexports::drm::control::Mode) -> u32 {
    WlMode::from(*mode).refresh as u32
}

impl NoosState {
    fn udev(&mut self) -> Option<&mut UdevData> {
        match &mut self.backend {
            Backend::Udev(d) => Some(d),
            _ => None,
        }
    }

    fn device_added(&mut self, node: DrmNode, path: &Path) -> anyhow::Result<()> {
        let handle: LoopHandle<'static, NoosState> = self.loop_handle.clone();
        let dh = self.display_handle.clone();
        let data = self.udev().unwrap();

        let fd = data
            .session
            .open(path, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK)
            .map_err(|e| anyhow::anyhow!("ouverture de {} : {e}", path.display()))?;
        let fd = DrmDeviceFd::new(DeviceFd::from(fd));
        let (drm, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;

        let token = handle
            .insert_source(notifier, move |event, metadata, state| match event {
                DrmEvent::VBlank(crtc) => state.frame_finish(crtc, metadata),
                DrmEvent::Error(e) => error!("DRM : {e:?}"),
            })
            .map_err(|e| anyhow::anyhow!("source DRM : {e}"))?;

        let egl_display = unsafe { EGLDisplay::new(gbm.clone()) }?;
        let egl_device = EGLDevice::device_for_display(&egl_display).ok();
        // Rendu logiciel (llvmpipe : machine virtuelle sans 3D, pilote absent) : pas de clôture
        // GPU, il faut attendre la fin du dessin avant l'affichage (voir render_surface).
        let software = egl_device.as_ref().is_some_and(|d| d.is_software());
        if software {
            warn!("rendu logiciel (pas d'accélération 3D) : synchronisation complète de chaque image");
        }
        let render_node = egl_device
            .and_then(|d| d.try_get_render_node().ok().flatten())
            .unwrap_or(node);
        let context = EGLContext::new(&egl_display)?;
        let renderer = unsafe { GlesRenderer::new(context) }?;

        let render_formats: FormatSet = renderer.egl_context().dmabuf_render_formats().iter().copied().collect();
        let allocator = GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT);
        let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into());
        let color_formats = if data.ten_bit { FORMATS_10BIT } else { FORMATS_8BIT };
        let output_manager = DrmOutputManager::new(
            drm,
            allocator,
            exporter,
            Some(gbm),
            color_formats.iter().copied(),
            render_formats,
        );

        // linux-dmabuf v4 avec feedback (préférence de scanout pour les jeux en plein écran).
        let dmabuf_formats = renderer.dmabuf_formats();
        let feedback = DmabufFeedbackBuilder::new(render_node.dev_id(), dmabuf_formats).build()?;
        let mut dmabuf_state = DmabufState::new();
        let global = dmabuf_state.create_global_with_default_feedback::<NoosState>(&dh, &feedback);
        let shm_formats = renderer.shm_formats();

        data.primary_gpu = render_node;
        data.gpu = Some(Gpu {
            node,
            software,
            renderer,
            output_manager,
            scanner: DrmScanner::new(),
            surfaces: HashMap::new(),
            _token: token,
        });
        self.dmabuf_state = Some((dmabuf_state, global));
        self.shm_state.update_formats(shm_formats);

        self.device_changed(node);
        Ok(())
    }

    fn device_changed(&mut self, node: DrmNode) {
        let events: Vec<DrmScanEvent> = {
            let Some(gpu) = self.udev().and_then(|d| d.gpu.as_mut()) else { return };
            if gpu.node != node {
                return;
            }
            match gpu.scanner.scan_connectors(gpu.output_manager.device()) {
                Ok(result) => result.into_iter().collect(),
                Err(e) => {
                    warn!("scan des connecteurs : {e}");
                    return;
                }
            }
        };
        for event in events {
            match event {
                DrmScanEvent::Connected { connector, crtc: Some(crtc) } => self.connector_connected(connector, crtc),
                DrmScanEvent::Disconnected { crtc: Some(crtc), .. } => self.connector_disconnected(crtc),
                _ => {}
            }
        }
        self.arrange_outputs();
        self.update_vrr();
    }

    fn connector_connected(&mut self, connector: connector::Info, crtc: crtc::Handle) {
        let name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
        let out_cfg = self.config.output(&name).cloned().unwrap_or_default();
        if out_cfg.enabled == Some(false) {
            info!(name, "écran désactivé dans la configuration");
            return;
        }
        let dh = self.display_handle.clone();
        let handle = self.loop_handle.clone();
        let x_offset: i32 = self
            .space
            .outputs()
            .map(|o| self.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0))
            .sum();
        let Some(gpu) = self.udev().and_then(|d| d.gpu.as_mut()) else { return };
        let drm = gpu.output_manager.device();

        // EDID : constructeur, modèle, capacités HDR.
        let info = display_info::for_connector(drm, connector.handle());
        let make = info.as_ref().and_then(|i| i.make()).unwrap_or_else(|| "Inconnu".into());
        let model = info.as_ref().and_then(|i| i.model()).unwrap_or_else(|| name.clone());
        let serial = info.as_ref().and_then(|i| i.serial()).unwrap_or_default();
        let edid_hdr = info.as_ref().map(|i| i.hdr_static_metadata().pq).unwrap_or(false);
        let driver_hdr = drm
            .get_properties(connector.handle())
            .ok()
            .map(|props| {
                props.into_iter().any(|(h, _)| {
                    drm.get_property(h).ok().is_some_and(|p| p.name().to_str() == Ok("HDR_OUTPUT_METADATA"))
                })
            })
            .unwrap_or(false);
        let hdr_capable = edid_hdr && driver_hdr;

        // Mode : celui de la config, sinon le mode préféré de l'écran.
        let modes = connector.modes();
        if modes.is_empty() {
            warn!(name, "aucun mode disponible");
            return;
        }
        let preferred = modes.iter().position(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED)).unwrap_or(0);
        let wanted = out_cfg.mode.as_deref().and_then(parse_mode).and_then(|(w, h, hz)| {
            modes
                .iter()
                .filter(|m| m.size() == (w, h))
                .min_by_key(|m| hz.map(|hz| (refresh_mhz(m) as i64 - hz as i64).abs()).unwrap_or(-(refresh_mhz(m) as i64)))
                .copied()
        });
        let drm_mode = wanted.unwrap_or(modes[preferred]);
        let wl_mode = WlMode::from(drm_mode);
        let mode_infos: Vec<ModeInfo> = modes
            .iter()
            .map(|m| ModeInfo {
                width: m.size().0 as i32,
                height: m.size().1 as i32,
                refresh: refresh_mhz(m) as i32,
                preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
            })
            .collect();

        let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (phys_w as i32, phys_h as i32).into(),
                subpixel: connector.subpixel().into(),
                make: make.clone(),
                model: model.clone(),
                serial_number: serial,
            },
        );
        let global = output.create_global::<NoosState>(&dh);
        let scale = out_cfg.scale.map(Scale::Fractional).unwrap_or(Scale::Integer(1));
        let transform = out_cfg.transform.as_deref().map(crate::wm::parse_transform).unwrap_or(Transform::Normal);
        output.set_preferred(wl_mode);
        output.change_current_state(Some(wl_mode), Some(transform), Some(scale), Some((x_offset, 0).into()));

        let planes = drm.planes(&crtc).ok();
        let drm_output = match gpu.output_manager.lock().initialize_output::<_, NoosElement>(
            crtc,
            drm_mode,
            &[connector.handle()],
            &output,
            planes,
            &mut gpu.renderer,
            &DrmOutputRenderElements::default(),
        ) {
            Ok(o) => o,
            Err(e) => {
                warn!(name, "initialisation de l'écran impossible : {e}");
                return;
            }
        };
        let vrr_support = drm_output
            .with_compositor(|c| c.vrr_supported(connector.handle()))
            .unwrap_or(VrrSupport::NotSupported);

        info!(
            name, make, model,
            mode = %format!("{}x{}@{:.2}", wl_mode.size.w, wl_mode.size.h, wl_mode.refresh as f64 / 1000.0),
            format = %drm_output.format(),
            vrr = ?vrr_support, hdr = hdr_capable,
            "écran connecté"
        );

        gpu.surfaces.insert(
            crtc,
            Surface {
                dh,
                output: output.clone(),
                global: Some(global),
                drm_output,
                vrr_support,
                vrr_active: false,
                hdr_capable,
                modes: mode_infos,
                drm_modes: modes.to_vec(),
                make,
                model,
                render_scheduled: true,
            },
        );
        self.space.map_output(&output, (x_offset, 0));
        if self.config.hdr_for(&name) {
            if hdr_capable {
                warn!(name, "HDR demandé : la sortie HDR (PQ/BT.2020) n'est pas encore implémentée, SDR utilisé");
            } else {
                warn!(name, "HDR demandé mais l'écran/pilote ne le supporte pas");
            }
        }
        handle.insert_idle(move |state| state.render_surface(crtc));
    }

    fn connector_disconnected(&mut self, crtc: crtc::Handle) {
        let Some(gpu) = self.udev().and_then(|d| d.gpu.as_mut()) else { return };
        if let Some(surface) = gpu.surfaces.remove(&crtc) {
            info!(name = surface.output.name(), "écran déconnecté");
            let output = surface.output.clone();
            drop(surface);
            self.fullscreen.remove(&output.name());
            self.space.unmap_output(&output);
        }
    }

    /// Range les écrans de gauche à droite et replace les fenêtres orphelines.
    pub(crate) fn arrange_outputs(&mut self) {
        let mut x = 0;
        for output in self.space.outputs().cloned().collect::<Vec<_>>() {
            let w = self.space.output_geometry(&output).map(|g| g.size.w).unwrap_or(0);
            self.space.map_output(&output, (x, 0));
            layer_map_for_output(&output).arrange();
            x += w;
        }
        let Some(first) = self.space.outputs().next().cloned() else { return };
        let area = self.usable_area(&first);
        let orphans: Vec<Window> = self
            .space
            .elements()
            .filter(|w| self.space.outputs_for_element(w).is_empty())
            .cloned()
            .collect();
        for w in orphans {
            self.space.map_element(w, area.loc, false);
        }
    }

    pub(crate) fn render_all(&mut self) {
        let crtcs: Vec<crtc::Handle> = self
            .udev()
            .and_then(|d| d.gpu.as_ref())
            .map(|g| g.surfaces.keys().copied().collect())
            .unwrap_or_default();
        for crtc in crtcs {
            self.render_surface(crtc);
        }
    }

    fn frame_finish(&mut self, crtc: crtc::Handle, metadata: &mut Option<DrmEventMetadata>) {
        let clock_now = self.clock.now();
        let Some(gpu) = self.udev().and_then(|d| d.gpu.as_mut()) else { return };
        let Some(surface) = gpu.surfaces.get_mut(&crtc) else { return };

        let frame_duration = surface
            .output
            .current_mode()
            .map(|m| Duration::from_secs_f64(1000.0 / m.refresh as f64))
            .unwrap_or(Duration::from_millis(16));
        let (clock, flags) = match metadata.as_ref().map(|m| m.time) {
            Some(DrmEventTime::Monotonic(tp)) if !tp.is_zero() => (
                tp.into(),
                wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwClock
                    | wp_presentation_feedback::Kind::HwCompletion,
            ),
            _ => (clock_now, wp_presentation_feedback::Kind::Vsync),
        };
        let seq = metadata.as_ref().map(|m| m.sequence).unwrap_or(0);

        match surface.drm_output.frame_submitted() {
            Ok(user_data) => {
                if let Some(mut feedback) = user_data.flatten() {
                    let refresh = if surface.vrr_active {
                        Refresh::variable(frame_duration)
                    } else {
                        Refresh::fixed(frame_duration)
                    };
                    feedback.presented(clock, refresh, seq as u64, flags);
                }
            }
            Err(e) => warn!("frame_submitted : {e:?}"),
        }

        // On redessine juste après le vblank : latence ≈ 1 frame.
        surface.render_scheduled = true;
        self.loop_handle.insert_idle(move |state| state.render_surface(crtc));
    }

    fn render_surface(&mut self, crtc: crtc::Handle) {
        // Écrans éteints : on n'envoie plus d'images (cela les rallumerait).
        if self.screens_off {
            return;
        }
        let Backend::Udev(mut data) = std::mem::replace(&mut self.backend, Backend::Detached) else {
            return;
        };
        let now = self.clock.now();
        let mut rendered = false;
        let mut output = None;
        let mut refresh = 60_000;

        if let Some(gpu) = data.gpu.as_mut() {
            if let Some(surface) = gpu.surfaces.get_mut(&crtc) {
                surface.render_scheduled = false;
                refresh = surface.output.current_mode().map(|m| m.refresh).unwrap_or(60_000);
                let out = surface.output.clone();

                // Image du curseur (thème XCursor), mise en cache par frame d'animation.
                let icon = match &self.cursor_status {
                    smithay::input::pointer::CursorImageStatus::Named(icon) => *icon,
                    _ => smithay::input::pointer::CursorIcon::Default,
                };
                let image = data.cursor.get_image(icon, 1, now.into());
                let buffer = match data.cursor_cache.iter().find(|(i, _)| i == &image) {
                    Some((_, b)) => b.clone(),
                    None => {
                        let b = MemoryRenderBuffer::from_slice(
                            &image.pixels_rgba,
                            Fourcc::Argb8888,
                            (image.width as i32, image.height as i32),
                            1,
                            Transform::Normal,
                            None,
                        );
                        data.cursor_cache.push((image.clone(), b.clone()));
                        b
                    }
                };

                let elements = render::output_elements(
                    self,
                    &mut gpu.renderer,
                    &out,
                    Some((&buffer, (image.xhot as i32, image.yhot as i32))),
                );
                let flags = if self.config.gaming.direct_scanout { FrameFlags::DEFAULT } else { FrameFlags::empty() };
                let clear = render::clear_color(self);
                match surface.drm_output.render_frame(&mut gpu.renderer, &elements, clear, flags) {
                    Ok(result) => {
                        rendered = !result.is_empty;
                        if rendered && gpu.software {
                            // llvmpipe dessine par tuiles dans des fils : sans clôture GPU, on
                            // attend la fin du dessin avant d'afficher (sinon tuiles noires ou
                            // restes de l'image précédente).
                            let _ = gpu.renderer.with_context(|gl| unsafe { gl.Finish() });
                        }
                        if rendered {
                            let feedback = self.take_presentation_feedback(&out, &result.states);
                            if let Err(e) = surface.drm_output.queue_frame(Some(feedback)) {
                                warn!("queue_frame : {e:?}");
                                rendered = false;
                            }
                        }
                    }
                    Err(e) => warn!("rendu : {e:?}"),
                }
                // Partage / capture d'écran en attente sur cet écran.
                self.process_captures(&mut gpu.renderer, &out, &elements, clear);
                if self.pending_screenshot.is_some() {
                    self.take_pending_screenshot(&mut gpu.renderer, &out, &elements, clear);
                }
                output = Some(out);
            }
        }
        self.backend = Backend::Udev(data);

        if let Some(output) = output {
            self.send_frames(&output);
        }
        if !rendered {
            // Rien n'a changé : on revérifie après une frame (aucun vblank n'arrivera).
            let delay = Duration::from_micros(1_000_000_000 / refresh.max(1) as u64);
            if let Some(s) = self.udev().and_then(|d| d.gpu.as_mut()).and_then(|g| g.surfaces.get_mut(&crtc)) {
                if s.render_scheduled {
                    return;
                }
                s.render_scheduled = true;
            }
            let _ = self.loop_handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
                state.render_surface(crtc);
                TimeoutAction::Drop
            });
        }
    }

    fn take_presentation_feedback(
        &self,
        output: &Output,
        states: &smithay::backend::renderer::element::RenderElementStates,
    ) -> OutputPresentationFeedback {
        use smithay::desktop::utils::{surface_presentation_feedback_flags_from_states, surface_primary_scanout_output};
        let mut feedback = OutputPresentationFeedback::new(output);
        for window in self.space.elements() {
            if self.space.outputs_for_element(window).contains(output) {
                window.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                    surface_presentation_feedback_flags_from_states(surface, None, states)
                });
            }
        }
        for layer in layer_map_for_output(output).layers() {
            layer.take_presentation_feedback(&mut feedback, surface_primary_scanout_output, |surface, _| {
                surface_presentation_feedback_flags_from_states(surface, None, states)
            });
        }
        feedback
    }
}

//! État global du compositeur.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    sync::Arc,
    time::Duration,
};

use noos_core::NoosConfig;
use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType, layer_map_for_output},
    input::{
        Seat, SeatState,
        keyboard::XkbConfig,
        pointer::{CursorImageStatus, PointerHandle},
    },
    output::Output,
    reexports::{
        calloop::{Interest, LoopHandle, LoopSignal, Mode, PostAction, generic::Generic},
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Clock, Logical, Monotonic, Point, Rectangle},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        dmabuf::{DmabufGlobal, DmabufState},
        fractional_scale::FractionalScaleManagerState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState, primary_selection::PrimarySelectionState,
            wlr_data_control::DataControlState,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, decoration::XdgDecorationState},
        },
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
    },
};
use tracing::{info, warn};

use crate::{backend::Backend, ipc::IpcServer};

/// Données attachées à chaque fenêtre (via `Window::user_data`).
#[derive(Debug, Default)]
pub struct WindowData {
    pub id: u64,
    /// La fenêtre a déjà été placée (centrée) après son premier buffer.
    pub placed: Cell<bool>,
    /// Géométrie à restaurer après maximisation / ancrage / plein écran.
    pub restore: RefCell<Option<Rectangle<i32, Logical>>>,
    pub maximized: Cell<bool>,
    pub fullscreen: Cell<bool>,
    /// Zone d'ancrage actuelle (moitié, quart, tiers… ou plein écran).
    pub zone: Cell<Option<crate::tiling::Zone>>,
    /// Animation de déplacement / redimensionnement en cours.
    pub anim: RefCell<Option<crate::tiling::Anim>>,
}

pub fn window_data(window: &Window) -> &WindowData {
    window.user_data().get::<WindowData>().expect("fenêtre sans WindowData")
}

pub struct NoosState {
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, NoosState>,
    pub loop_signal: LoopSignal,
    pub clock: Clock<Monotonic>,
    pub socket_name: OsString,
    pub backend: Backend,
    pub config: NoosConfig,

    // Bureau
    pub space: Space<Window>,
    pub popups: PopupManager,
    pub minimized: Vec<Window>,
    /// Fenêtre plein écran par nom d'écran.
    pub fullscreen: HashMap<String, Window>,
    pub next_window_id: u64,

    // États des protocoles Smithay
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub layer_shell_state: WlrLayerShellState,
    pub shm_state: ShmState,
    pub dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<NoosState>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub viewporter_state: ViewporterState,
    pub xdg_activation_state: XdgActivationState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub presentation_state: PresentationState,
    pub single_pixel_buffer_state: SinglePixelBufferState,

    // Entrées
    pub seat: Seat<NoosState>,
    pub pointer: PointerHandle<NoosState>,
    pub cursor_status: CursorImageStatus,
    pub dnd_icon: Option<WlSurface>,
    /// Super pressée sans autre touche : au relâchement on ouvre le menu Démarrer.
    pub super_alone: bool,
    /// Touches dont le relâchement ne doit pas être transmis (raccourcis consommés).
    pub suppressed_keys: Vec<u32>,
    /// Index courant pour Alt+Tab.
    pub alt_tab_index: Option<usize>,
    pub last_activity: std::time::Instant,
    pub screens_off: bool,
    pub suspend_requested: bool,
    /// Surfaces qui empêchent l'extinction (lecture vidéo…).
    pub idle_inhibitors: Vec<WlSurface>,
    /// Verrouillage de session (ext-session-lock-v1, voir `lock.rs`).
    pub session_lock_state: smithay::wayland::session_lock::SessionLockManagerState,
    pub lock: crate::lock::LockState,

    pub ipc: IpcServer,
    /// Capture d'écran demandée (chemin PNG), faite au prochain rendu.
    pub pending_screenshot: Option<std::path::PathBuf>,
    pub capture: crate::capture::CaptureState,
    pub blur: crate::blur::BlurState,
    /// Fenêtre en cours de déplacement (barre de dispositions, aperçu d'ancrage).
    pub snap_drag: Option<crate::tiling::SnapDrag>,
    /// Processus lancés par NOOS (commande, processus) : récupérés à leur fin (pas de
    /// zombies) ; le shell est relancé s'il s'arrête.
    pub children: std::cell::RefCell<Vec<(String, std::process::Child)>>,
    pub shell_restarts: Vec<std::time::Instant>,
    pub last_window_list: Vec<noos_core::ipc::WindowInfo>,
}

pub const SHELL_COMMAND: &str = "noos-shell";

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Client lancé dans un bac à sable (Flatpak, Snap) : pas d'accès direct aux
    /// protocoles sensibles (capture d'écran), il doit passer par les portails.
    pub sandboxed: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

impl NoosState {
    pub fn new(
        display: Display<NoosState>,
        loop_handle: LoopHandle<'static, NoosState>,
        loop_signal: LoopSignal,
        backend: Backend,
    ) -> Self {
        let dh = display.handle();
        let clock = Clock::new();
        let config = NoosConfig::load();

        // Socket Wayland pour les clients
        let source = ListeningSocketSource::new_auto().expect("impossible de créer le socket wayland");
        let socket_name = source.socket_name().to_os_string();
        loop_handle
            .insert_source(source, |stream, _, state| {
                let sandboxed = crate::capture::stream_is_sandboxed(&stream);
                if let Err(err) = state
                    .display_handle
                    .insert_client(stream, Arc::new(ClientState { sandboxed, ..Default::default() }))
                {
                    warn!("client wayland refusé : {err}");
                }
            })
            .expect("socket wayland");
        loop_handle
            .insert_source(Generic::new(display, Interest::READ, Mode::Level), |_, display, state| {
                // SAFETY : on ne détruit jamais le Display.
                unsafe {
                    display.get_mut().dispatch_clients(state).unwrap();
                }
                Ok(PostAction::Continue)
            })
            .expect("source display");
        info!(socket = ?socket_name, "socket wayland prêt");

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, clock.id() as u32);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);
        PointerConstraintsState::new::<Self>(&dh);
        smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<Self>(&dh);
        RelativePointerManagerState::new::<Self>(&dh);
        // Les applis demandent une forme de curseur ; le compositeur la dessine avec son thème :
        // même taille partout (GTK, Qt, Electron…).
        smithay::wayland::cursor_shape::CursorShapeManagerState::new::<Self>(&dh);
        let session_lock_state = smithay::wayland::session_lock::SessionLockManagerState::new::<Self, _>(&dh, |_| true);

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, backend.seat_name());
        let pointer = seat.add_pointer();
        let kb = keyboard_config(&config);
        let xkb = XkbConfig {
            layout: &kb.0,
            variant: &kb.1,
            options: if kb.2.is_empty() { None } else { Some(kb.2.clone()) },
            ..Default::default()
        };
        let delay = config.keyboard.repeat_delay.unwrap_or(300);
        let rate = config.keyboard.repeat_rate.unwrap_or(30);
        let keyboard = match seat.add_keyboard(xkb, delay, rate) {
            Ok(k) => k,
            Err(err) => {
                warn!("disposition clavier {kb:?} invalide ({err:?}), retour à « us »");
                seat.add_keyboard(XkbConfig::default(), delay, rate).expect("clavier")
            }
        };
        // Verr Num actif au démarrage (option [keyboard] numlock, activée par défaut). Les
        // voyants suivent : les claviers libinput lisent led_state() à leur ajout.
        if config.keyboard.numlock {
            let mut mods = keyboard.modifier_state();
            mods.num_lock = true;
            keyboard.set_modifier_state(mods);
        }

        let ipc = IpcServer::new(&loop_handle);
        let capture = crate::capture::CaptureState::new(&dh);

        Self {
            display_handle: dh,
            loop_handle,
            loop_signal,
            clock,
            socket_name,
            backend,
            config,
            space: Space::default(),
            popups: PopupManager::default(),
            minimized: Vec::new(),
            fullscreen: HashMap::new(),
            next_window_id: 1,
            compositor_state,
            xdg_shell_state,
            xdg_decoration_state,
            layer_shell_state,
            shm_state,
            dmabuf_state: None,
            output_manager_state,
            seat_state,
            data_device_state,
            primary_selection_state,
            data_control_state,
            viewporter_state,
            xdg_activation_state,
            fractional_scale_state,
            presentation_state,
            single_pixel_buffer_state,
            seat,
            pointer,
            cursor_status: CursorImageStatus::default_named(),
            dnd_icon: None,
            super_alone: false,
            suppressed_keys: Vec::new(),
            alt_tab_index: None,
            last_activity: std::time::Instant::now(),
            screens_off: false,
            suspend_requested: false,
            idle_inhibitors: Vec::new(),
            session_lock_state,
            lock: Default::default(),
            ipc,
            pending_screenshot: None,
            capture,
            blur: Default::default(),
            snap_drag: None,
            children: std::cell::RefCell::new(Vec::new()),
            shell_restarts: Vec::new(),
            last_window_list: Vec::new(),
        }
    }

    /// Surface (fenêtre, layer-shell ou popup) sous le pointeur, avec sa position globale.
    pub fn surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        // Verrouillée : rien d'autre que l'écran de verrouillage.
        if self.lock.locked {
            return self.lock_surface_under(pos);
        }
        let output = self.space.output_under(pos).next()?.clone();
        let output_geo = self.space.output_geometry(&output)?;

        // Plein écran : la fenêtre recouvre tout (sauf l'overlay).
        let layers = layer_map_for_output(&output);
        let layer_hit = |layer: smithay::wayland::shell::wlr_layer::Layer| {
            layers.layer_under(layer, pos - output_geo.loc.to_f64()).and_then(|l| {
                let l_geo = layers.layer_geometry(l)?;
                l.surface_under(pos - output_geo.loc.to_f64() - l_geo.loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + l_geo.loc + output_geo.loc).to_f64()))
            })
        };
        use smithay::wayland::shell::wlr_layer::Layer;

        if let Some(hit) = layer_hit(Layer::Overlay) {
            return Some(hit);
        }
        if let Some(window) = self.fullscreen.get(&output.name()) {
            let loc = self.space.element_location(window)?;
            return window
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + loc).to_f64()));
        }
        if let Some(hit) = layer_hit(Layer::Top) {
            return Some(hit);
        }
        if let Some((window, loc)) = self.space.element_under(pos) {
            if let Some((s, p)) = window.surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL) {
                return Some((s, (p + loc).to_f64()));
            }
        }
        layer_hit(Layer::Bottom).or_else(|| layer_hit(Layer::Background))
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .chain(self.minimized.iter())
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    pub fn window_by_id(&self, id: u64) -> Option<Window> {
        self.space
            .elements()
            .chain(self.minimized.iter())
            .find(|w| window_data(w).id == id)
            .cloned()
    }

    pub fn output_for_window(&self, window: &Window) -> Option<Output> {
        self.space
            .outputs_for_element(window)
            .into_iter()
            .next()
            .or_else(|| self.pointer_output())
    }

    pub fn pointer_output(&self) -> Option<Output> {
        let pos = self.pointer.current_location();
        self.space
            .output_under(pos)
            .next()
            .or_else(|| self.space.outputs().next())
            .cloned()
    }

    /// Zone utilisable d'un écran (hors barre des tâches et autres zones exclusives).
    pub fn usable_area(&self, output: &Output) -> Rectangle<i32, Logical> {
        let geo = self.space.output_geometry(output).unwrap_or_default();
        let map = layer_map_for_output(output);
        let zone = map.non_exclusive_zone();
        Rectangle::new(geo.loc + zone.loc, zone.size)
    }

    /// Lance une commande dans l'environnement de la session.
    pub fn spawn(&self, command: &str) {
        // Un seul shell à la fois (le chien de garde le relance déjà s'il s'arrête).
        if command.trim() == SHELL_COMMAND && self.shell_running() {
            info!("noos-shell tourne déjà : lancement ignoré");
            return;
        }
        info!(command, "lancement");
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd.env("WAYLAND_DISPLAY", &self.socket_name);
        cmd.env("NOOS_IPC_SOCKET", self.ipc.path());
        cmd.env_remove("DISPLAY");
        if let Some(d) = crate::XWAYLAND_DISPLAY.get() {
            cmd.env("DISPLAY", d);
        }
        match cmd.spawn() {
            Ok(child) => self.children.borrow_mut().push((command.trim().to_string(), child)),
            Err(err) => warn!(command, "échec du lancement : {err}"),
        }
    }

    fn shell_running(&self) -> bool {
        self.children
            .borrow_mut()
            .iter_mut()
            .any(|(cmd, child)| cmd == SHELL_COMMAND && matches!(child.try_wait(), Ok(None)))
    }

    /// Toutes les 2 s : récupère les processus terminés et relance le shell s'il s'est arrêté
    /// (au plus 5 fois par minute, pour ne pas boucler sur un plantage au démarrage).
    pub fn reap_children(&mut self) {
        let mut shell_died = false;
        self.children.borrow_mut().retain_mut(|(cmd, child)| match child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                if crate::KIOSK.get().is_some_and(|k| k.trim() == cmd) {
                    info!(%status, "application kiosque terminée : arrêt du compositeur");
                    self.loop_signal.stop();
                }
                if cmd == SHELL_COMMAND {
                    warn!(%status, "noos-shell s'est arrêté");
                    shell_died = true;
                }
                false
            }
            Err(_) => false,
        });
        if shell_died && std::env::var_os("NOOS_NO_SHELL").is_none() {
            let now = std::time::Instant::now();
            self.shell_restarts.retain(|t| now.duration_since(*t).as_secs() < 60);
            if self.shell_restarts.len() < 5 {
                self.shell_restarts.push(now);
                self.spawn(SHELL_COMMAND);
            } else {
                warn!("noos-shell plante en boucle : plus de relance automatique (Super+Maj+R ou IPC spawn)");
            }
        }
    }

    pub fn reload_config(&mut self) {
        self.config = NoosConfig::load();
        if crate::kiosk() {
            self.config.tiling.enabled = false;
        }
        // Disposition clavier modifiée (Paramètres, installateur) : appliquée à chaud.
        let kb = keyboard_config(&self.config);
        if let Some(keyboard) = self.seat.get_keyboard() {
            let xkb = XkbConfig {
                layout: &kb.0,
                variant: &kb.1,
                options: if kb.2.is_empty() { None } else { Some(kb.2.clone()) },
                ..Default::default()
            };
            if let Err(err) = keyboard.set_xkb_config(self, xkb) {
                warn!("disposition clavier {kb:?} invalide : {err:?}");
            }
        }
        info!("configuration rechargée");
        self.backend.apply_config(&self.config);
        self.update_vrr();
        self.ipc.broadcast(&noos_core::ipc::Event::ConfigReloaded);
    }

    /// Envoie les callbacks de frame aux clients après un rendu.
    pub fn send_frames(&self, output: &Output) {
        let time = self.clock.now();
        let throttle = Some(Duration::from_secs(1));
        for window in self.space.elements() {
            if self.space.outputs_for_element(window).contains(output) {
                window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
            }
        }
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
        if let CursorImageStatus::Surface(surface) = &self.cursor_status {
            smithay::desktop::utils::send_frames_surface_tree(surface, output, time, throttle, |_, _| {
                Some(output.clone())
            });
        }
        if let Some(icon) = &self.dnd_icon {
            smithay::desktop::utils::send_frames_surface_tree(icon, output, time, throttle, |_, _| {
                Some(output.clone())
            });
        }
        // Écran de verrouillage : sans ces rappels, son animation resterait figée.
        if let Some(lock) = self.lock_surface_for(output) {
            smithay::desktop::utils::send_frames_surface_tree(lock.wl_surface(), output, time, throttle, |_, _| {
                Some(output.clone())
            });
        }
    }
}

/// Disposition clavier : config NOOS, sinon `XKB_DEFAULT_*`, sinon `/etc/vconsole.conf`.
fn keyboard_config(config: &NoosConfig) -> (String, String, String) {
    let kb = &config.keyboard;
    if !kb.layout.is_empty() {
        return (kb.layout.clone(), kb.variant.clone(), kb.options.clone());
    }
    if let Ok(layout) = std::env::var("XKB_DEFAULT_LAYOUT") {
        return (
            layout,
            std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default(),
            std::env::var("XKB_DEFAULT_OPTIONS").unwrap_or_default(),
        );
    }
    let mut layout = String::from("us");
    let mut variant = String::new();
    // Dans un toolbox, le fichier de l'hôte est monté sous /run/host.
    for path in ["/etc/vconsole.conf", "/run/host/etc/vconsole.conf"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("XKBLAYOUT=") {
                    layout = v.trim_matches('"').to_string();
                } else if let Some(v) = line.strip_prefix("XKBVARIANT=") {
                    variant = v.trim_matches('"').to_string();
                }
            }
            break;
        }
    }
    (layout, variant, kb.options.clone())
}

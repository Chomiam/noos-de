//! Applis X11 : le compositeur lance lui-même `Xwayland` (paquet Debian `xwayland`) et joue
//! le rôle de gestionnaire de fenêtres X11 (`X11Wm` de Smithay), comme GNOME ou KDE. Les
//! fenêtres X11 entrent dans le même `Space<Window>` que les fenêtres Wayland : placement,
//! focus, réduction, maximisation, ancrage, plein écran et barre des tâches les traitent
//! pareil (les rares opérations propres à xdg-shell passent par les fonctions de ce module).
//!
//! Le focus clavier est un [`FocusTarget`] : surface Wayland, ou fenêtre X11 (qui demande en
//! plus `SetInputFocus` au serveur X).

use std::{borrow::Cow, os::unix::io::OwnedFd, process::Stdio};

use smithay::{
    backend::input::KeyState,
    desktop::Window,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{Focus, GrabStartData},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size},
    wayland::{
        compositor::CompositorHandler,
        seat::WaylandFocus,
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata, request_primary_client_selection,
                set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
    },
};
use tracing::{info, warn};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeEdge, ResizeSurfaceGrab},
    state::{NoosState, WindowData, window_data},
};

// ───────────────────────────── Focus clavier ─────────────────────────────

/// Cible du focus clavier : surface Wayland (fenêtre xdg, panneau, verrouillage…) ou fenêtre X11.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusTarget {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl FocusTarget {
    /// Surface Wayland associée (une fenêtre X11 l'obtient juste après sa création).
    pub fn surface(&self) -> Option<WlSurface> {
        match self {
            Self::Wayland(s) => Some(s.clone()),
            Self::X11(x) => x.wl_surface(),
        }
    }
}

impl From<WlSurface> for FocusTarget {
    fn from(s: WlSurface) -> Self {
        Self::Wayland(s)
    }
}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(s) => s.alive(),
            Self::X11(x) => x.alive(),
        }
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(s) => Some(Cow::Borrowed(s)),
            Self::X11(x) => x.wl_surface().map(Cow::Owned),
        }
    }
}

impl KeyboardTarget<NoosState> for FocusTarget {
    fn enter(&self, seat: &Seat<NoosState>, data: &mut NoosState, keys: Vec<KeysymHandle<'_>>, serial: Serial) {
        match self {
            Self::Wayland(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            Self::X11(x) => KeyboardTarget::enter(x, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<NoosState>, data: &mut NoosState, serial: Serial) {
        match self {
            Self::Wayland(s) => KeyboardTarget::leave(s, seat, data, serial),
            Self::X11(x) => KeyboardTarget::leave(x, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<NoosState>,
        data: &mut NoosState,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: smithay::backend::input::InputTime,
    ) {
        match self {
            Self::Wayland(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            Self::X11(x) => KeyboardTarget::key(x, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(&self, seat: &Seat<NoosState>, data: &mut NoosState, modifiers: ModifiersState, serial: Serial) {
        match self {
            Self::Wayland(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            Self::X11(x) => KeyboardTarget::modifiers(x, seat, data, modifiers, serial),
        }
    }
}

// ───────────────────────────── Fenêtres : Wayland ou X11 ─────────────────────────────

/// Fenêtre X11 « override-redirect » (menu, info-bulle, liste déroulante) : placée par
/// l'appli elle-même, jamais dans la barre des tâches ni focalisée.
pub fn is_override_redirect(window: &Window) -> bool {
    window.x11_surface().is_some_and(|x| x.is_override_redirect())
}

/// Cible du focus clavier pour une fenêtre.
pub fn focus_target(window: &Window) -> Option<FocusTarget> {
    if let Some(t) = window.toplevel() {
        Some(FocusTarget::Wayland(t.wl_surface().clone()))
    } else {
        window.x11_surface().map(|x| FocusTarget::X11(x.clone()))
    }
}

/// (titre, identifiant d'appli) : app_id xdg, ou classe WM_CLASS en X11.
pub fn title_and_app_id(window: &Window) -> (String, String) {
    if let Some(t) = window.toplevel() {
        return smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .map(|d| {
                    let d = d.lock().unwrap();
                    (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
                })
                .unwrap_or_default()
        });
    }
    match window.x11_surface() {
        Some(x) => {
            let class = x.class();
            (x.title(), if class.is_empty() { x.instance() } else { class })
        }
        None => Default::default(),
    }
}

/// Fenêtre X11 : nouvelle géométrie (position comprise : les menus d'une appli X11 se placent
/// d'après la position que le serveur X croit être celle de la fenêtre).
pub fn configure_x11(window: &Window, rect: Rectangle<i32, Logical>) {
    if let Some(x) = window.x11_surface() {
        if x.last_configure() != rect {
            if let Err(e) = x.configure(rect) {
                warn!("fenêtre X11 : configure : {e}");
            }
        }
    }
}

impl NoosState {
    /// Surface Wayland qui a le clavier (fenêtre X11 comprise).
    pub fn keyboard_focus_surface(&self) -> Option<WlSurface> {
        self.seat.get_keyboard()?.current_focus()?.surface()
    }

    pub fn window_for_x11(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .chain(self.minimized.iter())
            .chain(self.desktops.parked.iter().map(|(w, _)| w))
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }

    /// À chaque image : le serveur X doit connaître la position réelle des fenêtres X11
    /// (déplacements, animations, ancrage), sinon leurs menus s'ouvrent au mauvais endroit.
    pub fn sync_x11_geometry(&mut self) {
        if self.xwm.is_none() {
            return;
        }
        for w in self.space.elements() {
            let Some(x) = w.x11_surface() else { continue };
            if x.is_override_redirect() {
                continue;
            }
            let Some(loc) = self.space.element_location(w) else { continue };
            let size = x.last_configure().size;
            configure_x11(w, Rectangle::new(loc, size));
        }
    }

    /// Fenêtre (Wayland ou X11) retirée : listes, flou, glisser en cours.
    pub fn forget_window(&mut self, window: &Window) {
        self.minimized.retain(|w| w != window);
        self.desktops.parked.retain(|(w, _)| w != window);
        self.fullscreen.retain(|_, w| w != window);
        let id = window_data(window).id;
        self.space.unmap_elem(window);
        self.blur_forget_window(id);
        if self.snap_drag.as_ref().is_some_and(|d| &d.window == window) {
            self.snap_drag = None;
        }
        self.refresh_pointer_focus_soon();
    }

    // ───────────── Démarrage ─────────────

    /// Lance Xwayland sur `:display` (choisi par l'appelant pour exporter `DISPLAY` tout de
    /// suite : les clients X qui se connectent avant qu'il soit prêt attendent). Le
    /// gestionnaire de fenêtres X11 démarre quand Xwayland est prêt.
    pub fn start_xwayland(&mut self, display: u32) -> bool {
        let (xwayland, client) = match XWayland::spawn(
            &self.display_handle,
            display,
            std::iter::empty::<(String, String)>(),
            std::iter::empty::<String>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        ) {
            Ok(x) => x,
            Err(e) => {
                warn!("Xwayland introuvable ou en échec ({e}) : pas d'applis X11 (paquet xwayland)");
                return false;
            }
        };
        let dh = self.display_handle.clone();
        let ret = self.loop_handle.insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready { x11_socket, display_number } => {
                state.client_compositor_state(&client).set_client_scale(1.0);
                match X11Wm::start_wm(state.loop_handle.clone(), &dh, x11_socket, client.clone()) {
                    Ok(mut wm) => {
                        // Curseur des fenêtres X11 (fond du bureau X compris).
                        if let Some((pixels, size, hot)) = crate::cursor::default_cursor_rgba() {
                            if let Err(e) = wm.set_cursor(&pixels, size, hot) {
                                warn!("curseur X11 : {e}");
                            }
                        }
                        state.xwm = Some(wm);
                        info!(display = display_number, "Xwayland prêt (gestionnaire de fenêtres X11 intégré)");
                    }
                    Err(e) => warn!("gestionnaire de fenêtres X11 : {e}"),
                }
            }
            XWaylandEvent::Error => warn!("Xwayland s'est arrêté au démarrage"),
        });
        if let Err(e) = ret {
            warn!("Xwayland : source d'événements : {e}");
            return false;
        }
        true
    }
}

// ───────────────────────────── Gestionnaire de fenêtres X11 ─────────────────────────────

impl XWaylandShellHandler for NoosState {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl XwmHandler for NoosState {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().expect("gestionnaire X11 absent")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(e) = surface.set_mapped(true) {
            warn!("fenêtre X11 : map : {e}");
            return;
        }
        let window = Window::new_x11_window(surface.clone());
        let id = self.next_window_id;
        self.next_window_id += 1;
        window.user_data().insert_if_missing(|| WindowData { id, ..Default::default() });
        let data = window_data(&window);
        data.desktop.set(self.desktops.current);
        data.placed.set(true);

        // Centrée dans la zone utile de l'écran du pointeur, comme les fenêtres Wayland ;
        // jamais plus grande que cette zone.
        let mut size = surface.geometry().size;
        let loc = match self.pointer_output() {
            Some(output) => {
                let area = self.usable_area(&output);
                size = Size::from((size.w.clamp(1, area.size.w), size.h.clamp(1, area.size.h)));
                Point::from((area.loc.x + (area.size.w - size.w) / 2, area.loc.y + (area.size.h - size.h) / 2))
            }
            None => Point::from((0, 0)),
        };
        self.space.map_element(window.clone(), loc, false);
        configure_x11(&window, Rectangle::new(loc, size));
        let (title, class) = title_and_app_id(&window);
        info!(title, class, "fenêtre X11 affichée");
        self.focus_window(&window);
        self.notify_windows_changed();
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let loc = surface.geometry().loc;
        let window = Window::new_x11_window(surface);
        let id = self.next_window_id;
        self.next_window_id += 1;
        // `placed` reste faux : pas dans la barre des tâches.
        window.user_data().insert_if_missing(|| WindowData { id, ..Default::default() });
        self.space.map_element(window, loc, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11(&surface) {
            self.forget_window(&window);
            if !is_override_redirect(&window) {
                self.focus_top_window();
            }
            self.update_vrr();
            self.notify_windows_changed();
        }
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.window_for_x11(&surface) {
            self.forget_window(&window);
            self.focus_top_window();
            self.notify_windows_changed();
        }
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let window = self.window_for_x11(&surface);
        let mut geo = surface.last_configure();
        match &window {
            // Maximisée, ancrée ou plein écran : la taille est imposée.
            Some(win) if window_data(win).maximized.get() || window_data(win).fullscreen.get() => {}
            // Pas encore affichée : l'appli choisit sa taille (et sa position, recentrée au map).
            None => {
                if let Some(x) = x {
                    geo.loc.x = x;
                }
                if let Some(y) = y {
                    geo.loc.y = y;
                }
                if let Some(w) = w {
                    geo.size.w = w as i32;
                }
                if let Some(h) = h {
                    geo.size.h = h as i32;
                }
            }
            // Affichée : l'appli peut changer de taille, pas se déplacer.
            Some(_) => {
                if let Some(w) = w {
                    geo.size.w = w as i32;
                }
                if let Some(h) = h {
                    geo.size.h = h as i32;
                }
            }
        }
        let _ = surface.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        // Seules les fenêtres override-redirect se placent elles-mêmes.
        if !surface.is_override_redirect() {
            return;
        }
        let window = self.space.elements().find(|w| w.x11_surface() == Some(&surface)).cloned();
        if let Some(window) = window {
            self.space.map_element(window, geometry.loc, true);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            self.maximize(&w);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            if window_data(&w).maximized.get() {
                self.unmaximize(&w, None);
            }
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            if let Some(output) = self.output_for_window(&w) {
                self.set_fullscreen(&w, &output);
            }
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            if window_data(&w).fullscreen.get() {
                self.unset_fullscreen(&w);
            }
        }
    }

    fn minimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            self.minimize(&w);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(w) = self.window_for_x11(&surface) {
            self.activate_window(&w);
        }
    }

    fn active_window_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _timestamp: u32,
        _currently_active: Option<X11Surface>,
    ) {
        if let Some(w) = self.window_for_x11(&surface) {
            self.activate_window(&w);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, surface: X11Surface, _button: u32, edges: X11ResizeEdge) {
        let Some(start_data) = self.pointer.grab_start_data() else { return };
        let Some(window) = self.window_for_x11(&surface) else { return };
        if window_data(&window).fullscreen.get() {
            return;
        }
        let Some(loc) = self.space.element_location(&window) else { return };
        let rect = Rectangle::new(loc, window.geometry().size);
        let grab = ResizeSurfaceGrab::start(start_data, window, ResizeEdge::from_x11(edges), rect);
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, _button: u32) {
        let Some(start_data) = self.pointer.grab_start_data() else { return };
        let Some(window) = self.window_for_x11(&surface) else { return };
        if window_data(&window).fullscreen.get() {
            return;
        }
        if window_data(&window).maximized.get() {
            self.unmaximize(&window, Some(start_data.location));
        }
        window_data(&window).anim.borrow_mut().take();
        let Some(initial) = self.space.element_location(&window) else { return };
        let start_data = GrabStartData { focus: None, button: start_data.button, location: start_data.location };
        let grab = MoveSurfaceGrab { start_data, window, initial_window_location: initial };
        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    // ── Presse-papiers X11 ↔ Wayland ──

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        // Seule l'appli X11 active lit le presse-papiers.
        matches!(
            self.seat.get_keyboard().and_then(|k| k.current_focus()),
            Some(FocusTarget::X11(x)) if x.xwm_id() == Some(xwm)
        )
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let res = match selection {
            SelectionTarget::Clipboard => request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string()),
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string()),
        };
        if let Err(e) = res {
            warn!("presse-papiers Wayland → X11 : {e}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(&self.display_handle, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(&self.display_handle, &self.seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat);
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        warn!("Xwayland déconnecté : plus d'applis X11 jusqu'à la prochaine session");
        self.xwm = None;
    }
}

// (Dispatch du protocole xwayland-shell : `delegate_dispatch2!` de handlers.rs.)

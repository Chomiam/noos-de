//! Implémentation des protocoles Wayland (traits « Handler » de Smithay).

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::utils::on_commit_buffer_handler},
    delegate_dispatch2,
    desktop::{
        LayerSurface, PopupKind, Window, WindowSurfaceType, find_popup_root_surface,
        get_popup_toplevel_coords, layer_map_for_output,
    },
    input::{
        Seat, SeatHandler, SeatState,
        dnd::{DnDGrab, DndGrabHandler, DndTarget, GrabType, Source},
        pointer::{CursorImageStatus, Focus, PointerHandle},
    },
    output::Output,
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel,
        },
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Serial},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent, is_sync_subsurface,
            with_states,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        fractional_scale::{FractionalScaleHandler, with_fractional_scale},
        output::OutputHandler,
        pointer_constraints::{PointerConstraintsHandler, with_pointer_constraint},
        selection::{
            SelectionHandler,
            data_device::{DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus},
            primary_selection::{PrimarySelectionHandler, PrimarySelectionState, set_primary_focus},
            wlr_data_control::{DataControlHandler, DataControlState},
        },
        shell::{
            wlr_layer::{
                KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData,
                WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::{
                PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
                XdgToplevelSurfaceData, decoration::XdgDecorationHandler,
            },
        },
        shm::{ShmHandler, ShmState},
        xdg_activation::{XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData},
    },
};
use tracing::{debug, warn};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    state::{ClientState, NoosState, WindowData, window_data},
};

// ───────────────────────────── wl_compositor ─────────────────────────────

impl CompositorHandler for NoosState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend.early_import(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.on_commit();
                if &root == surface {
                    self.on_toplevel_commit(&window);
                }
            }
        }

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent() {
                let _ = popup.send_configure();
            }
        }

        self.layer_commit(surface);
        crate::grabs::handle_resize_commit(self, surface);
    }
}

impl NoosState {
    fn on_toplevel_commit(&mut self, window: &Window) {
        let toplevel = window.toplevel().unwrap();
        let initial_configure_sent = with_states(toplevel.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });
        if !initial_configure_sent {
            toplevel.send_configure();
            return;
        }

        // Premier buffer : on centre la fenêtre façon Windows 11.
        let data = window_data(window);
        let size = window.geometry().size;
        if !data.placed.get() && size.w > 0 && size.h > 0 {
            data.placed.set(true);
            // Kiosque : la fenêtre principale occupe tout l'écran.
            let parent = window.toplevel().and_then(|t| t.parent());
            if crate::kiosk() && parent.is_none() && !data.fullscreen.get() {
                if let Some(output) = self.pointer_output() {
                    self.set_fullscreen(window, &output);
                }
            } else if !data.maximized.get() && !data.fullscreen.get() {
                if let Some(output) = self.pointer_output() {
                    let area = self.usable_area(&output);
                    let loc = Point::from((
                        area.loc.x + (area.size.w - size.w).max(0) / 2,
                        area.loc.y + (area.size.h - size.h).max(0) / 2,
                    ));
                    self.space.map_element(window.clone(), loc, true);
                }
            }
            self.focus_window(window);
        }
        self.notify_windows_changed();
    }

    fn layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self
            .space
            .outputs()
            .find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
        else {
            return;
        };

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        let layer = {
            let mut map = layer_map_for_output(&output);
            map.arrange();
            map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL).cloned()
        };
        let Some(layer) = layer else { return };
        // Nouveau fond d'écran : le flou des panneaux est à recalculer.
        if layer.layer() == smithay::wayland::shell::wlr_layer::Layer::Background {
            self.blur.generation += 1;
        }

        if !initial_configure_sent {
            layer.layer_surface().send_configure();
            return;
        }

        // Les surfaces « exclusives » (menu Démarrer, centre de contrôle) prennent le clavier
        // à leur apparition — une seule fois, pour qu'un clic ailleurs puisse le leur reprendre.
        let exclusive = layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive;
        let first_map = {
            layer.user_data().insert_if_missing(|| std::cell::Cell::new(false));
            !layer.user_data().get::<std::cell::Cell<bool>>().unwrap().replace(true)
        };
        let keyboard = self.seat.get_keyboard().unwrap();
        if exclusive && first_map && keyboard.current_focus().as_ref() != Some(surface) {
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            keyboard.set_focus(self, Some(surface.clone()), serial);
        }
        // La zone exclusive a pu changer (barre des tâches) : on réajuste les fenêtres maximisées.
        self.relayout_maximized(&output);
    }
}

impl BufferHandler for NoosState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for NoosState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for NoosState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        if self.backend.import_dmabuf(&dmabuf) {
            let _ = notifier.successful::<NoosState>();
        } else {
            notifier.failed();
        }
    }
}

// ───────────────────────────── xdg-shell ─────────────────────────────

impl XdgShellHandler for NoosState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface.clone());
        let id = self.next_window_id;
        self.next_window_id += 1;
        window.user_data().insert_if_missing(|| WindowData { id, ..Default::default() });

        // Les applis GTK4/libadwaita dessinent leurs propres décorations (CSD).
        surface.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ClientSide);
            if let Some(output) = self.pointer_output() {
                state.bounds = Some(self.usable_area(&output).size);
            }
        });

        let loc = self
            .pointer_output()
            .map(|o| self.usable_area(&o).loc)
            .unwrap_or_default();
        self.space.map_element(window, loc, false);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            warn!("popup ignorée : {err}");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // Les grabs de popup ne sont pas encore gérés : les menus se ferment au clic extérieur
        // via le focus clavier.
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        let Some(seat) = Seat::<Self>::from_resource(&seat) else { return };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };

        // Glisser une fenêtre maximisée la restaure (comme sous Windows).
        if window_data(&window).maximized.get() {
            self.unmaximize(&window, Some(start_data.location));
        }
        // La fenêtre suit la souris : aucune animation résiduelle.
        window_data(&window).anim.borrow_mut().take();
        let Some(initial) = self.space.element_location(&window) else { return };
        let grab = MoveSurfaceGrab { start_data, window, initial_window_location: initial };
        let pointer = seat.get_pointer().unwrap();
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let Some(seat) = Seat::<Self>::from_resource(&seat) else { return };
        let Some(start_data) = check_grab(&seat, surface.wl_surface(), serial) else { return };
        let Some(window) = self.window_for_surface(surface.wl_surface()) else { return };
        let Some(loc) = self.space.element_location(&window) else { return };
        let size = window.geometry().size;

        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Resizing);
        });
        surface.send_pending_configure();

        let grab = ResizeSurfaceGrab::start(
            start_data,
            window,
            edges.into(),
            smithay::utils::Rectangle::new(loc, size),
        );
        seat.get_pointer().unwrap().set_grab(self, grab, serial, Focus::Clear);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.maximize(&window);
        } else {
            surface.send_configure();
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.unmaximize(&window, None);
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            surface.send_configure();
            return;
        };
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.output_for_window(&window));
        if let Some(output) = output {
            self.set_fullscreen(&window, &output);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.unset_fullscreen(&window);
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            self.minimize(&window);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.minimized.retain(|w| w.toplevel().is_some_and(|t| t != &surface));
        self.fullscreen.retain(|_, w| w.toplevel().is_some_and(|t| t != &surface));
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            let id = window_data(&window).id;
            self.space.unmap_elem(&window);
            self.blur_forget_window(id);
            if self.snap_drag.as_ref().is_some_and(|d| d.window == window) {
                self.snap_drag = None;
            }
            self.refresh_pointer_focus_soon();
        }
        self.focus_top_window();
        self.update_vrr();
        self.notify_windows_changed();
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {
        self.notify_windows_changed();
    }

    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.notify_windows_changed();
    }
}

fn check_grab(
    seat: &Seat<NoosState>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<smithay::input::pointer::GrabStartData<NoosState>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

impl NoosState {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else { return };

        // Popups de fenêtres
        if let Some(window) = self.window_for_surface(&root) {
            let Some(output) = self.output_for_window(&window) else { return };
            let Some(output_geo) = self.space.output_geometry(&output) else { return };
            let Some(window_geo) = self.space.element_geometry(&window) else { return };
            let mut target = output_geo;
            target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
            target.loc -= window_geo.loc;
            popup.with_pending_state(|state| {
                state.geometry = state.positioner.get_unconstrained_geometry(target);
            });
            return;
        }

        // Popups de surfaces layer-shell (menus du shell)
        for output in self.space.outputs() {
            let map = layer_map_for_output(output);
            if let Some(layer) = map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL) {
                let Some(layer_geo) = map.layer_geometry(layer) else { return };
                let output_geo = self.space.output_geometry(output).unwrap_or_default();
                let mut target = smithay::utils::Rectangle::from_size(output_geo.size);
                target.loc -= layer_geo.loc;
                target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
                popup.with_pending_state(|state| {
                    state.geometry = state.positioner.get_unconstrained_geometry(target);
                });
                return;
            }
        }
    }
}

impl XdgDecorationHandler for NoosState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ClientSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        // Pas encore de décorations côté serveur : on impose le CSD.
        toplevel.with_pending_state(|state| state.decoration_mode = Some(DecorationMode::ClientSide));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.request_mode(toplevel, DecorationMode::ClientSide);
    }
}

// ───────────────────────────── layer-shell ─────────────────────────────

impl WlrLayerShellHandler for NoosState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let Some(output) = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.pointer_output())
        else {
            warn!("surface layer-shell sans écran, ignorée");
            return;
        };
        debug!(?layer, namespace, output = output.name(), "nouvelle surface layer-shell");
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            warn!("map_layer : {err}");
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        self.unconstrain_popup(&popup);
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for output in outputs {
            let mut map = layer_map_for_output(&output);
            let layer = map.layers().find(|l| l.layer_surface() == &surface).cloned();
            if let Some(layer) = layer {
                map.unmap_layer(&layer);
                drop(map);
                self.relayout_maximized(&output);
            }
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        if keyboard.current_focus().as_ref() == Some(surface.wl_surface()) || keyboard.current_focus().is_none() {
            self.focus_top_window();
        }
    }
}

// ───────────────────────────── Seat / sélection ─────────────────────────────

impl SeatHandler for NoosState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }
}

impl smithay::input::tablet::TabletSeatHandler for NoosState {
    type ToolFocus = WlSurface;
}

impl SelectionHandler for NoosState {
    type SelectionUserData = ();
}

impl DataDeviceHandler for NoosState {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for NoosState {
    fn dropped(
        &mut self,
        _target: Option<DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        self.dnd_icon = None;
    }
}

impl WaylandDndGrabHandler for NoosState {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        self.dnd_icon = icon;
        match type_ {
            GrabType::Pointer => {
                let pointer = seat.get_pointer().unwrap();
                let Some(start_data) = pointer.grab_start_data() else {
                    source.cancel();
                    return;
                };
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                pointer.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => source.cancel(),
        }
    }
}

impl PrimarySelectionHandler for NoosState {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}

impl DataControlHandler for NoosState {
    fn data_control_state(&mut self) -> &mut DataControlState {
        &mut self.data_control_state
    }
}

impl OutputHandler for NoosState {}

// ───────────────────────────── Divers ─────────────────────────────

impl XdgActivationHandler for NoosState {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, _data: XdgActivationTokenData) -> bool {
        // Le shell lance les applis : on accepte tous les jetons (prototype).
        true
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed().as_secs() < 10 {
            if let Some(window) = self.window_for_surface(&surface) {
                self.activate_window(&window);
            }
        }
    }
}

impl FractionalScaleHandler for NoosState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let output = self
            .window_for_surface(&root)
            .and_then(|w| self.output_for_window(&w))
            .or_else(|| self.pointer_output());
        if let Some(output) = output {
            with_states(&surface, |states| {
                with_fractional_scale(states, |fs| {
                    fs.set_preferred_scale(output.current_scale().fractional_scale());
                });
            });
        }
    }
}

impl PointerConstraintsHandler for NoosState {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // Les jeux verrouillent la souris : on accorde la contrainte si la surface a le focus.
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |constraint| {
                if let Some(c) = constraint {
                    c.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
    }
}

impl smithay::wayland::idle_inhibit::IdleInhibitHandler for NoosState {
    fn inhibit(&mut self, surface: WlSurface) {
        if !self.idle_inhibitors.contains(&surface) {
            self.idle_inhibitors.push(surface);
        }
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle_inhibitors.retain(|s| s != &surface);
    }
}

delegate_dispatch2!(NoosState);

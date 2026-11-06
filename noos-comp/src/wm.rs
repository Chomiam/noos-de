//! Gestion des fenêtres « façon Windows 11 » : focus, maximisation, ancrage (Snap),
//! réduction dans la barre des tâches, plein écran.

use noos_core::{
    config::VrrPolicy,
    ipc::{Event, WindowInfo},
};
use smithay::{
    reexports::wayland_server::Resource,
    desktop::Window,
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::state::{NoosState, window_data};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    Left,
    Right,
    Up,
    Down,
}

pub fn parse_transform(s: &str) -> smithay::utils::Transform {
    use smithay::utils::Transform;
    match s {
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        _ => Transform::Normal,
    }
}

pub fn transform_name(t: smithay::utils::Transform) -> &'static str {
    use smithay::utils::Transform;
    match t {
        Transform::_90 => "90",
        Transform::_180 => "180",
        Transform::_270 => "270",
        _ => "normal",
    }
}

impl NoosState {
    /// Après un changement de géométrie d'écran : couches, fenêtres maximisées, clients.
    pub fn output_changed(&mut self, output: &Output) {
        smithay::desktop::layer_map_for_output(output).arrange();
        if let crate::backend::Backend::Udev(_) = self.backend {
            self.arrange_outputs();
        }
        self.relayout_maximized(output);
        let outputs = self.backend.output_infos(&self.space, &self.config);
        self.ipc.broadcast(&Event::Outputs { outputs });
    }

    pub fn focus_window(&mut self, window: &Window) {
        self.space.raise_element(window, true);
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        let surface = window.toplevel().map(|t| t.wl_surface().clone());
        if keyboard.current_focus() != surface {
            let (title, app_id) = window
                .toplevel()
                .map(|t| {
                    smithay::wayland::compositor::with_states(t.wl_surface(), |states| {
                        let d = states.data_map.get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>();
                        d.map(|d| {
                            let d = d.lock().unwrap();
                            (d.title.clone().unwrap_or_default(), d.app_id.clone().unwrap_or_default())
                        })
                        .unwrap_or_default()
                    })
                })
                .unwrap_or_default();
            tracing::info!(app_id, title, "focus clavier → fenêtre");
        }
        keyboard.set_focus(self, surface, serial);
        for w in self.space.elements() {
            if let Some(t) = w.toplevel() {
                t.send_pending_configure();
            }
        }
        self.notify_windows_changed();
    }

    /// Restaure si réduite, puis donne le focus.
    pub fn activate_window(&mut self, window: &Window) {
        if let Some(pos) = self.minimized.iter().position(|w| w == window) {
            let w = self.minimized.remove(pos);
            let restore = window_data(&w).restore.borrow_mut().take();
            let loc = restore.map(|r| r.loc).unwrap_or_else(|| {
                self.pointer_output().map(|o| self.usable_area(&o).loc).unwrap_or_default()
            });
            self.space.map_element(w, loc, true);
        }
        self.focus_window(window);
        // Réapparition : état « actif » renvoyé explicitement (certaines applis Electron/Chromium
        // l'attendent pour réaccepter les clics), et pointeur recalculé sous la souris.
        if let Some(t) = window.toplevel() {
            t.send_configure();
        }
        self.refresh_pointer_focus_soon();
    }

    pub fn focus_top_window(&mut self) {
        let top = self.space.elements().last().cloned();
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        match top {
            Some(w) => {
                self.space.raise_element(&w, true);
                keyboard.set_focus(self, w.toplevel().map(|t| t.wl_surface().clone()), serial);
                for e in self.space.elements().chain(self.minimized.iter()) {
                    if let Some(t) = e.toplevel() {
                        t.send_pending_configure();
                    }
                }
            }
            None => keyboard.set_focus(self, None, serial),
        }
        self.notify_windows_changed();
    }

    pub fn focused_window(&self) -> Option<Window> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        self.window_for_surface(&focus)
    }

    pub fn close_window(&mut self, window: &Window) {
        if let Some(t) = window.toplevel() {
            t.send_close();
        }
    }

    pub fn minimize(&mut self, window: &Window) {
        if self.minimized.contains(window) {
            return;
        }
        if let Some(loc) = self.space.element_location(window) {
            let mut restore = window_data(window).restore.borrow_mut();
            if restore.is_none() || !window_data(window).maximized.get() {
                *restore = Some(Rectangle::new(loc, window.geometry().size));
            }
        }
        if window_data(window).fullscreen.get() {
            self.unset_fullscreen(window);
        }
        self.space.unmap_elem(window);
        self.minimized.push(window.clone());
        self.focus_top_window();
        self.refresh_pointer_focus_soon();
    }

    /// Recalcule la surface sous le pointeur sans attendre un mouvement de souris : après une
    /// fenêtre réduite, restaurée, déplacée ou fermée sous un pointeur immobile, le prochain
    /// clic doit aller à la bonne fenêtre.
    pub fn refresh_pointer_focus_soon(&mut self) {
        self.loop_handle.insert_idle(|s| s.refresh_pointer_focus());
    }

    pub fn refresh_pointer_focus(&mut self) {
        let pointer = self.pointer.clone();
        if pointer.is_grabbed() {
            return;
        }
        let pos = pointer.current_location();
        let under = self.surface_under(pos);
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time: smithay::backend::input::InputTime::now(),
            },
        );
        pointer.frame(self);
    }

    pub fn show_desktop(&mut self) {
        let visible: Vec<Window> = self.space.elements().cloned().collect();
        if visible.is_empty() {
            // Deuxième appui : tout restaurer.
            for w in self.minimized.clone() {
                self.activate_window(&w);
            }
        } else {
            for w in visible {
                self.minimize(&w);
            }
        }
    }

    fn save_restore_geometry(&self, window: &Window) {
        let data = window_data(window);
        if data.maximized.get() || data.fullscreen.get() {
            return;
        }
        if let Some(loc) = self.space.element_location(window) {
            *data.restore.borrow_mut() = Some(Rectangle::new(loc, window.geometry().size));
        }
    }

    pub fn maximize(&mut self, window: &Window) {
        self.snap_zone(window, crate::tiling::Zone::Maximize);
    }

    /// `grab_point` : position du pointeur si la restauration vient d'un glisser.
    pub fn unmaximize(&mut self, window: &Window, grab_point: Option<Point<f64, Logical>>) {
        let data = window_data(window);
        let restore = data.restore.borrow_mut().take();
        data.maximized.set(false);
        data.zone.set(None);
        let from = self.window_rect(window);
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            for s in [
                xdg_toplevel::State::Maximized,
                xdg_toplevel::State::TiledLeft,
                xdg_toplevel::State::TiledRight,
                xdg_toplevel::State::TiledTop,
                xdg_toplevel::State::TiledBottom,
            ] {
                state.states.unset(s);
            }
            state.size = restore.map(|r| r.size);
        });
        toplevel.send_configure();
        if let Some(r) = restore {
            let loc = match grab_point {
                // Garder la fenêtre sous le curseur, centrée horizontalement (sans animation :
                // elle suit la souris).
                Some(p) => Point::from((p.x as i32 - r.size.w / 2, p.y as i32 - 16)),
                None => {
                    self.animate_window(window, from, r);
                    r.loc
                }
            };
            self.space.map_element(window.clone(), loc, true);
        }
        self.notify_windows_changed();
    }

    pub fn toggle_maximize(&mut self, window: &Window) {
        if window_data(window).maximized.get() {
            self.unmaximize(window, None);
        } else {
            self.maximize(window);
        }
    }

    /// Super + flèche, comme sous Windows : ← / → ancrent sur une moitié puis font défiler
    /// 1/2 → 2/3 → 1/3 ; ↑ / ↓ depuis une moitié donnent les quarts.
    pub fn snap(&mut self, window: &Window, side: Snap) {
        use crate::tiling::Zone::*;
        let current = window_data(window).zone.get();
        // Ancrage désactivé : seuls agrandir (↑) et restaurer / réduire (↓) restent.
        if !self.config.tiling.enabled {
            match side {
                Snap::Up => self.maximize(window),
                Snap::Down if window_data(window).maximized.get() => self.unmaximize(window, None),
                Snap::Down => self.minimize(window),
                Snap::Left | Snap::Right => {}
            }
            return;
        }
        let next = match (side, current) {
            (Snap::Left, Some(Left)) => LeftTwoThirds,
            (Snap::Left, Some(LeftTwoThirds)) => LeftThird,
            (Snap::Left, Some(TopRight)) => TopLeft,
            (Snap::Left, Some(BottomRight)) => BottomLeft,
            (Snap::Left, Some(Right | RightThird | RightTwoThirds)) => {
                self.unmaximize(window, None);
                return;
            }
            (Snap::Left, _) => Left,
            (Snap::Right, Some(Right)) => RightTwoThirds,
            (Snap::Right, Some(RightTwoThirds)) => RightThird,
            (Snap::Right, Some(TopLeft)) => TopRight,
            (Snap::Right, Some(BottomLeft)) => BottomRight,
            (Snap::Right, Some(Left | LeftThird | LeftTwoThirds)) => {
                self.unmaximize(window, None);
                return;
            }
            (Snap::Right, _) => Right,
            (Snap::Up, Some(Left | LeftThird | LeftTwoThirds)) => TopLeft,
            (Snap::Up, Some(Right | RightThird | RightTwoThirds)) => TopRight,
            (Snap::Up, Some(BottomLeft)) => Left,
            (Snap::Up, Some(BottomRight)) => Right,
            (Snap::Up, _) => Maximize,
            (Snap::Down, Some(Left | LeftThird | LeftTwoThirds)) => BottomLeft,
            (Snap::Down, Some(Right | RightThird | RightTwoThirds)) => BottomRight,
            (Snap::Down, Some(TopLeft)) => Left,
            (Snap::Down, Some(TopRight)) => Right,
            (Snap::Down, Some(_)) => {
                self.unmaximize(window, None);
                return;
            }
            (Snap::Down, None) => {
                self.minimize(window);
                return;
            }
        };
        self.snap_zone(window, next);
    }

    pub fn set_fullscreen(&mut self, window: &Window, output: &Output) {
        let Some(geo) = self.space.output_geometry(output) else { return };
        self.save_restore_geometry(window);
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.size = Some(geo.size);
            state.fullscreen_output = toplevel.wl_surface().client().and_then(|c| output.client_outputs(&c).next());
        });
        toplevel.send_configure();
        window_data(window).fullscreen.set(true);
        self.space.map_element(window.clone(), geo.loc, true);
        self.fullscreen.insert(output.name(), window.clone());
        self.focus_window(window);
        self.update_vrr();
    }

    pub fn unset_fullscreen(&mut self, window: &Window) {
        let data = window_data(window);
        data.fullscreen.set(false);
        self.fullscreen.retain(|_, w| w != window);
        let restore = if data.maximized.get() { None } else { data.restore.borrow_mut().take() };
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.fullscreen_output = None;
            state.size = restore.map(|r| r.size);
        });
        toplevel.send_configure();
        if let Some(r) = restore {
            self.space.map_element(window.clone(), r.loc, true);
        }
        if data.maximized.get() {
            self.maximize(window);
        }
        self.update_vrr();
        self.notify_windows_changed();
    }

    /// Recalcule la taille des fenêtres maximisées (la barre des tâches vient d'apparaître…).
    pub fn relayout_maximized(&mut self, output: &Output) {
        let area = self.usable_area(output);
        let windows: Vec<Window> = self
            .space
            .elements()
            .filter(|w| {
                let d = window_data(w);
                d.maximized.get() && !d.fullscreen.get()
            })
            .filter(|w| self.space.outputs_for_element(w).contains(output))
            .cloned()
            .collect();
        for window in windows {
            let toplevel = window.toplevel().unwrap();
            let zone = window_data(&window).zone.get().unwrap_or(crate::tiling::Zone::Maximize);
            let r = zone.rect(area);
            let (loc, size) = (r.loc, r.size);
            let changed = toplevel.with_pending_state(|s| {
                let changed = s.size != Some(size);
                s.size = Some(size);
                changed
            });
            if changed {
                toplevel.send_pending_configure();
            }
            if self.space.element_location(&window) != Some(loc) {
                self.space.map_element(window, loc, false);
            }
        }
    }

    /// Alt+Tab : passe à la fenêtre suivante dans l'ordre d'empilement.
    pub fn cycle_windows(&mut self) {
        let windows: Vec<Window> = self.space.elements().cloned().collect();
        if windows.len() < 2 {
            return;
        }
        // La plus basse dans la pile devient la plus haute.
        let next = windows[0].clone();
        self.focus_window(&next);
    }

    // ───────────────────────── Liste des fenêtres (IPC) ─────────────────────────

    pub fn window_list(&self) -> Vec<WindowInfo> {
        let focused = self.focused_window();
        let mut list: Vec<WindowInfo> = self
            .space
            .elements()
            .map(|w| (w, false))
            .chain(self.minimized.iter().map(|w| (w, true)))
            .filter(|(w, _)| window_data(w).placed.get())
            .map(|(w, minimized)| {
                let (title, app_id) = w
                    .toplevel()
                    .map(|t| {
                        with_states(t.wl_surface(), |states| {
                            let data = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
                            (data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default())
                        })
                    })
                    .unwrap_or_default();
                let d = window_data(w);
                WindowInfo {
                    id: d.id,
                    title,
                    app_id,
                    focused: focused.as_ref() == Some(w),
                    minimized,
                    maximized: d.maximized.get(),
                    fullscreen: d.fullscreen.get(),
                    geometry: None,
                }
            })
            .collect();
        // Ordre stable (ordre d'ouverture) pour que les icônes de la barre ne sautent pas.
        list.sort_by_key(|w| w.id);
        list
    }

    pub fn notify_windows_changed(&mut self) {
        let list = self.window_list();
        if list != self.last_window_list {
            self.last_window_list = list.clone();
            self.ipc.broadcast(&Event::Windows { windows: list });
        }
    }

    /// Applique la politique VRR à chaque écran selon la config et l'état plein écran.
    pub fn update_vrr(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for output in outputs {
            let name = output.name();
            let want = match self.config.vrr_for(&name) {
                VrrPolicy::Off => false,
                VrrPolicy::Always => true,
                VrrPolicy::Fullscreen => self.fullscreen.contains_key(&name),
            };
            self.backend.set_vrr(&output, want);
        }
        let outputs = self.backend.output_infos(&self.space, &self.config);
        self.ipc.broadcast(&Event::Outputs { outputs });
    }
}

//! Entrées clavier / souris et raccourcis globaux (façon Windows 11).
//!
//! | Raccourci                 | Action                                   |
//! |---------------------------|------------------------------------------|
//! | Super (seul)              | Menu Démarrer                            |
//! | Super+E                   | Explorateur de fichiers (noos-file)      |
//! | Super+I                   | Paramètres (noos-settings)               |
//! | Super+Entrée              | Terminal                                 |
//! | Super+D                   | Afficher le bureau                       |
//! | Super+↑ / ↓ / ← / →       | Maximiser / restaurer-réduire / ancrer   |
//! | Super+F / F11             | Plein écran                              |
//! | Super+Shift+R             | Recharger la configuration               |
//! | Alt+Tab                   | Fenêtre suivante                         |
//! | Alt+F4 / Super+Q          | Fermer la fenêtre                        |
//! | Super + glisser           | Déplacer une fenêtre                     |
//! | Ctrl+Alt+F1…F12           | Changer de terminal virtuel              |
//! | Ctrl+Alt+Retour arrière   | Quitter NOOS                             |

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent, KeyState,
        KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::{FilterResult, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::pointer_constraints::{PointerConstraint, with_pointer_constraint},
};

use crate::{grabs::MoveSurfaceGrab, state::NoosState, wm::Snap};

const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone)]
enum Action {
    Quit,
    VtSwitch(i32),
    Spawn(String),
    StartMenuKey,
    Close,
    Snap(Snap),
    Fullscreen,
    ShowDesktop,
    AltTab,
    ReloadConfig,
}

fn terminal_command() -> String {
    for term in ["noos-term", "ptyxis", "kgx", "gnome-terminal", "foot", "alacritty", "kitty", "konsole", "xterm"] {
        if which(term) {
            return term.to_string();
        }
    }
    "foot".into()
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}

fn shortcut(mods: &ModifiersState, sym: Keysym) -> Option<Action> {
    let raw = sym.raw();
    // XF86Switch_VT_1 … XF86Switch_VT_12
    if (0x1008FE01..=0x1008FE0C).contains(&raw) {
        return Some(Action::VtSwitch((raw - 0x1008FE01 + 1) as i32));
    }
    if mods.ctrl && mods.alt && sym == Keysym::BackSpace {
        return Some(Action::Quit);
    }
    if mods.alt && !mods.logo {
        return match sym {
            Keysym::Tab | Keysym::ISO_Left_Tab => Some(Action::AltTab),
            Keysym::F4 => Some(Action::Close),
            _ => None,
        };
    }
    if sym == Keysym::F11 && !mods.logo {
        return Some(Action::Fullscreen);
    }
    // Capture d'écran : Impr. écran, ou Super+Maj+S (comme Windows).
    if sym == Keysym::Print {
        return Some(Action::Spawn("noos-snip".into()));
    }
    if mods.logo && mods.shift && matches!(sym, Keysym::s | Keysym::S) {
        return Some(Action::Spawn("noos-snip".into()));
    }
    if !mods.logo {
        return None;
    }
    match sym {
        Keysym::e | Keysym::E => Some(Action::Spawn("noos-file".into())),
        Keysym::i | Keysym::I => Some(Action::Spawn("noos-settings".into())),
        Keysym::Return | Keysym::KP_Enter => Some(Action::Spawn(terminal_command())),
        Keysym::d | Keysym::D => Some(Action::ShowDesktop),
        Keysym::q | Keysym::Q => Some(Action::Close),
        Keysym::f | Keysym::F => Some(Action::Fullscreen),
        Keysym::r | Keysym::R if mods.shift => Some(Action::ReloadConfig),
        Keysym::Up => Some(Action::Snap(Snap::Up)),
        Keysym::Down => Some(Action::Snap(Snap::Down)),
        Keysym::Left => Some(Action::Snap(Snap::Left)),
        Keysym::Right => Some(Action::Snap(Snap::Right)),
        _ => None,
    }
}

impl NoosState {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        if !matches!(event, InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. }) {
            self.on_activity();
        }
        match event {
            InputEvent::Keyboard { event } => self.on_keyboard::<I>(event),
            InputEvent::PointerMotion { event } => self.on_pointer_motion::<I>(event),
            InputEvent::PointerMotionAbsolute { event } => self.on_pointer_motion_absolute::<I>(event),
            InputEvent::PointerButton { event } => self.on_pointer_button::<I>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<I>(event),
            _ => {}
        }
    }

    fn on_keyboard<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::<I>::time(&event);
        let keycode = event.key_code();
        let key_state = event.state();
        let keyboard = self.seat.get_keyboard().unwrap();

        let action = keyboard.input::<Option<Action>, _>(self, keycode, key_state, serial, time, |state, mods, handle| {
            let sym = handle.modified_sym();
            let is_super = matches!(handle.raw_latin_sym_or_raw_current_sym(), Some(Keysym::Super_L | Keysym::Super_R))
                || matches!(sym, Keysym::Super_L | Keysym::Super_R);

            if key_state == KeyState::Pressed {
                if is_super {
                    state.super_alone = true;
                    return FilterResult::Forward;
                }
                state.super_alone = false;
                // Kiosque : seul le changement de terminal virtuel reste disponible.
                let action = shortcut(mods, sym).filter(|a| !crate::kiosk() || matches!(a, Action::VtSwitch(_)));
                if let Some(action) = action {
                    state.suppressed_keys.push(keycode.raw());
                    return FilterResult::Intercept(Some(action));
                }
                FilterResult::Forward
            } else {
                if let Some(pos) = state.suppressed_keys.iter().position(|k| *k == keycode.raw()) {
                    state.suppressed_keys.remove(pos);
                    return FilterResult::Intercept(None);
                }
                if is_super && std::mem::take(&mut state.super_alone) && !crate::kiosk() {
                    return FilterResult::Intercept(Some(Action::StartMenuKey));
                }
                FilterResult::Forward
            }
        });

        if let Some(action) = action.flatten() {
            self.run_action(action);
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::Quit => {
                tracing::info!("Ctrl+Alt+Retour arrière : fin de session");
                self.loop_signal.stop();
            }
            Action::VtSwitch(vt) => self.backend.change_vt(vt),
            Action::Spawn(cmd) => self.spawn(&cmd),
            Action::StartMenuKey => {
                // Mode jeu : pas de menu Démarrer par-dessus un jeu en plein écran.
                let in_game = self.config.gaming.game_mode
                    && self.focused_window().is_some_and(|w| crate::state::window_data(&w).fullscreen.get());
                if !in_game {
                    self.ipc.broadcast(&noos_core::ipc::Event::ToggleStartMenu);
                }
            }
            Action::ShowDesktop => self.show_desktop(),
            Action::AltTab => self.cycle_windows(),
            Action::ReloadConfig => self.reload_config(),
            Action::Close => {
                if let Some(w) = self.focused_window() {
                    self.close_window(&w);
                }
            }
            Action::Snap(side) => {
                if let Some(w) = self.focused_window() {
                    self.snap(&w, side);
                }
            }
            Action::Fullscreen => {
                if let Some(w) = self.focused_window() {
                    if crate::state::window_data(&w).fullscreen.get() {
                        self.unset_fullscreen(&w);
                    } else if let Some(o) = self.output_for_window(&w) {
                        self.set_fullscreen(&w, &o);
                    }
                }
            }
        }
    }

    // ───────────────────────────── Pointeur ─────────────────────────────

    fn clamp_to_outputs(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.space.output_under(pos).next().is_some() {
            return pos;
        }
        // Ramène le point dans l'écran le plus proche.
        let mut best = pos;
        let mut best_d = f64::MAX;
        for o in self.space.outputs() {
            let g = self.space.output_geometry(o).unwrap_or_default().to_f64();
            let p = Point::from((
                pos.x.clamp(g.loc.x, g.loc.x + g.size.w - 1.0),
                pos.y.clamp(g.loc.y, g.loc.y + g.size.h - 1.0),
            ));
            let d = (p.x - pos.x).powi(2) + (p.y - pos.y).powi(2);
            if d < best_d {
                best_d = d;
                best = p;
            }
        }
        best
    }

    fn on_pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let pointer = self.pointer.clone();
        let mut pos = pointer.current_location();
        let under = self.surface_under(pos);

        // Souris verrouillée (jeux FPS) : on ne déplace pas le curseur.
        let mut locked = false;
        if let Some((surface, _)) = &under {
            with_pointer_constraint(surface, &pointer, |c| {
                if let Some(c) = c.filter(|c| c.is_active()) {
                    locked = matches!(&*c, PointerConstraint::Locked(_));
                }
            });
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta: event.delta(),
                delta_unaccel: event.delta_unaccel(),
                time: event.time(),
            },
        );

        if !locked {
            pos += event.delta();
            pos = self.clamp_to_outputs(pos);
            let under = self.surface_under(pos);
            pointer.motion(self, under, &MotionEvent { location: pos, serial, time: Event::<I>::time(&event) });
        }
        pointer.frame(self);
        self.backend.schedule_render();
    }

    fn on_pointer_motion_absolute<I: InputBackend>(&mut self, event: I::PointerMotionAbsoluteEvent) {
        let Some(output) = self.space.outputs().next().cloned() else { return };
        let geo = self.space.output_geometry(&output).unwrap_or_default();
        let pos = event.position_transformed(geo.size) + geo.loc.to_f64();
        let serial = SERIAL_COUNTER.next_serial();
        let under = self.surface_under(pos);
        let pointer = self.pointer.clone();
        pointer.motion(self, under, &MotionEvent { location: pos, serial, time: Event::<I>::time(&event) });
        pointer.frame(self);
    }

    fn on_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        let button = event.button_code();
        let state = event.state();

        if state == ButtonState::Pressed && !pointer.is_grabbed() {
            let pos = pointer.current_location();
            let mods = self.seat.get_keyboard().unwrap().modifier_state();
            self.super_alone = false;

            // Couches au-dessus des fenêtres (barre, menus, panneaux) : elles reçoivent le clic,
            // les fenêtres dessous ne doivent pas prendre le clavier (sinon le panneau se ferme).
            if let Some(layer) = self.upper_layer_under(pos) {
                if layer.can_receive_keyboard_focus() {
                    let keyboard = self.seat.get_keyboard().unwrap();
                    if keyboard.current_focus().as_ref() != Some(layer.wl_surface()) {
                        tracing::info!(namespace = layer.namespace(), "clic : clavier donné au panneau");
                        keyboard.set_focus(self, Some(layer.wl_surface().clone()), serial);
                    }
                }
            } else if let Some((window, _)) = self.space.element_under(pos).map(|(w, l)| (w.clone(), l)) {
                let is_fullscreen_elsewhere = self
                    .space
                    .output_under(pos)
                    .next()
                    .and_then(|o| self.fullscreen.get(&o.name()))
                    .is_some_and(|fw| fw != &window);
                if !is_fullscreen_elsewhere {
                    self.focus_window(&window);
                    // Super + clic gauche : déplacer la fenêtre depuis n'importe où.
                    if mods.logo && button == BTN_LEFT {
                        if let Some(initial) = self.space.element_location(&window) {
                            let start_data = GrabStartData { focus: None, button, location: pos };
                            let grab = MoveSurfaceGrab { start_data, window, initial_window_location: initial };
                            pointer.set_grab(self, grab, serial, Focus::Clear);
                            return;
                        }
                    }
                }
            } else {
                self.focus_layer_under(pos);
            }
        }

        pointer.button(self, &ButtonEvent { button, state, serial, time: Event::<I>::time(&event) });
        pointer.frame(self);
    }

    /// Surface layer-shell des couches Overlay / Top réellement sous le pointeur : affichée,
    /// et dont la zone cliquable contient le point (un panneau caché ou une zone transparente
    /// ne doit pas prendre le clavier à la fenêtre dessous).
    fn upper_layer_under(&self, pos: Point<f64, Logical>) -> Option<smithay::desktop::LayerSurface> {
        use smithay::wayland::shell::wlr_layer::Layer as L;
        let output = self.space.output_under(pos).next()?;
        let geo = self.space.output_geometry(output)?;
        let map = layer_map_for_output(output);
        let local = pos - geo.loc.to_f64();
        for layer in [L::Overlay, L::Top] {
            for l in map.layers_on(layer).rev() {
                let Some(lgeo) = map.layer_geometry(l) else { continue };
                if !lgeo.to_f64().contains(local) {
                    continue;
                }
                if l.surface_under(local - lgeo.loc.to_f64(), WindowSurfaceType::ALL).is_some() {
                    return Some(l.clone());
                }
            }
        }
        None
    }

    /// Clic sur une surface layer-shell (barre des tâches…) : lui donner le clavier si elle le demande.
    fn focus_layer_under(&mut self, pos: Point<f64, Logical>) {
        let Some(output) = self.space.output_under(pos).next().cloned() else { return };
        let geo = self.space.output_geometry(&output).unwrap_or_default();
        let surface = {
            let map = layer_map_for_output(&output);
            map.layer_under(smithay::wayland::shell::wlr_layer::Layer::Overlay, pos - geo.loc.to_f64())
                .or_else(|| map.layer_under(smithay::wayland::shell::wlr_layer::Layer::Top, pos - geo.loc.to_f64()))
                .or_else(|| map.layer_under(smithay::wayland::shell::wlr_layer::Layer::Bottom, pos - geo.loc.to_f64()))
                .or_else(|| map.layer_under(smithay::wayland::shell::wlr_layer::Layer::Background, pos - geo.loc.to_f64()))
                .filter(|l| l.can_receive_keyboard_focus())
                .map(|l| l.wl_surface().clone())
        };
        if let Some(surface) = surface {
            let serial = SERIAL_COUNTER.next_serial();
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(self, Some(surface), serial);
        } else if self.upper_layer_under(pos).is_some() {
            // Clic sur la barre des tâches (sans clavier) : la fenêtre active le reste,
            // comme sous Windows.
        } else {
            // Clic sur le bureau : plus aucune fenêtre active (ferme les menus du shell).
            let serial = SERIAL_COUNTER.next_serial();
            let keyboard = self.seat.get_keyboard().unwrap();
            keyboard.set_focus(self, None, serial);
        }
        let _ = WindowSurfaceType::ALL;
    }

    fn on_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let source = event.source();
        // Le défilement naturel est configuré dans libinput (backend DRM).
        let natural = 1.0;
        let h = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.0)
            * natural;
        let v = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.0)
            * natural;

        let mut frame = AxisFrame::new(Event::<I>::time(&event)).source(source);
        if h != 0.0 {
            frame = frame.value(Axis::Horizontal, h);
            if let Some(d) = event.amount_v120(Axis::Horizontal) {
                frame = frame.v120(Axis::Horizontal, (d * natural) as i32);
            }
        }
        if v != 0.0 {
            frame = frame.value(Axis::Vertical, v);
            if let Some(d) = event.amount_v120(Axis::Vertical) {
                frame = frame.v120(Axis::Vertical, (d * natural) as i32);
            }
        }
        if source == AxisSource::Finger {
            if event.amount(Axis::Horizontal) == Some(0.0) {
                frame = frame.stop(Axis::Horizontal);
            }
            if event.amount(Axis::Vertical) == Some(0.0) {
                frame = frame.stop(Axis::Vertical);
            }
        }
        let pointer = self.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }
}

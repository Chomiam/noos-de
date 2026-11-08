//! Réglages rapides (coin inférieur droit) : mode sombre, mode jeu, VRR, transparence,
//! volume, accès aux paramètres.

use std::{cell::Cell, process::Command, rc::Rc, time::Instant};

use adap::prelude::*;
use gtk::glib;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::config::{ColorScheme, VrrPolicy};

use crate::{Shell, apps};

pub struct QuickSettings {
    window: gtk::Window,
    dark: gtk::ToggleButton,
    game: gtk::ToggleButton,
    vrr: gtk::ToggleButton,
    glass: gtk::ToggleButton,
    volume: gtk::Scale,
    /// Évite les boucles quand on met à jour les boutons depuis la config.
    syncing: Cell<bool>,
    hidden_at: Cell<Option<Instant>>,
}

fn tile(label: &str, icon: &str) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::new();
    b.add_css_class("tile");
    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.append(&gtk::Image::from_icon_name(icon));
    v.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&v));
    b.set_hexpand(true);
    b
}

/// Volume de la sortie par défaut via PipeWire (`wpctl`), entre 0 et 1.
fn read_volume() -> Option<f64> {
    let out = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1)?.parse().ok()
}

impl QuickSettings {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-quicksettings"));
        window.set_layer(Layer::Top);
        crate::editmode::anchor_to_bar(&window, &shell.config());
        shell.flyouts.borrow_mut().push(window.downgrade());
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Right, 12);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.add_css_class("noos-flyout");
        root.add_css_class("quick");
        root.set_size_request(380, -1);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_column_homogeneous(true);
        let dark = tile("Mode sombre", "weather-clear-night-symbolic");
        let game = tile("Mode jeu", "input-gaming-symbolic");
        let vrr = tile("VRR auto", "video-display-symbolic");
        let glass = tile("Transparence", "view-paged-symbolic");
        grid.attach(&dark, 0, 0, 1, 1);
        grid.attach(&game, 1, 0, 1, 1);
        grid.attach(&vrr, 2, 0, 1, 1);
        grid.attach(&glass, 0, 1, 1, 1);
        root.append(&grid);

        let vol_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        vol_box.append(&gtk::Image::from_icon_name("audio-volume-high-symbolic"));
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        volume.set_hexpand(true);
        vol_box.append(&volume);
        root.append(&vol_box);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("footer");
        let info = gtk::Label::new(Some("NOOS"));
        info.add_css_class("dim-label");
        info.set_hexpand(true);
        info.set_xalign(0.0);
        footer.append(&info);
        let settings = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings.set_tooltip_text(Some("Tous les paramètres"));
        footer.append(&settings);
        root.append(&footer);
        window.set_child(Some(&root));

        let q = Rc::new(Self {
            window,
            dark,
            game,
            vrr,
            glass,
            volume,
            syncing: Cell::new(false),
            hidden_at: Cell::new(None),
        });

        {
            let (q2, s) = (q.clone(), shell.clone());
            q.dark.connect_toggled(move |b| {
                if q2.syncing.get() {
                    return;
                }
                let on = b.is_active();
                s.update_config(|c| c.appearance.color_scheme = if on { ColorScheme::Dark } else { ColorScheme::Light });
            });
        }
        {
            let (q2, s) = (q.clone(), shell.clone());
            q.game.connect_toggled(move |b| {
                if !q2.syncing.get() {
                    let on = b.is_active();
                    s.update_config(|c| c.gaming.game_mode = on);
                }
            });
        }
        {
            let (q2, s) = (q.clone(), shell.clone());
            q.vrr.connect_toggled(move |b| {
                if !q2.syncing.get() {
                    let on = b.is_active();
                    s.update_config(|c| c.gaming.vrr = if on { VrrPolicy::Fullscreen } else { VrrPolicy::Off });
                }
            });
        }
        {
            let (q2, s) = (q.clone(), shell.clone());
            q.glass.connect_toggled(move |b| {
                if !q2.syncing.get() {
                    let on = b.is_active();
                    s.update_config(|c| c.appearance.panel_opacity = if on { 0.72 } else { 1.0 });
                }
            });
        }
        {
            let q2 = q.clone();
            q.volume.connect_value_changed(move |s| {
                if !q2.syncing.get() {
                    let v = format!("{:.2}", s.value());
                    let _ = Command::new("wpctl").args(["set-volume", "@DEFAULT_AUDIO_SINK@", &v]).spawn();
                }
            });
        }
        {
            let q2 = q.clone();
            settings.connect_clicked(move |_| {
                q2.hide();
                apps::launch("org.noos.Settings");
            });
        }
        {
            let q2 = q.clone();
            crate::close_on_focus_loss(&q.window, move || q2.hide());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let q2 = q.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    q2.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            q.window.add_controller(keys);
        }
        {
            let q2 = q.clone();
            shell.theming.connect_changed(move |_| q2.refresh());
        }
        q.shell_ref(shell);
        q
    }

    fn shell_ref(&self, shell: &Rc<Shell>) {
        let cfg = shell.config();
        self.sync(&cfg);
    }

    fn sync(&self, cfg: &noos_core::NoosConfig) {
        self.syncing.set(true);
        self.dark.set_active(adap::StyleManager::default().is_dark() && cfg.appearance.color_scheme != ColorScheme::Light);
        self.game.set_active(cfg.gaming.game_mode);
        self.vrr.set_active(cfg.gaming.vrr != VrrPolicy::Off);
        self.glass.set_active(cfg.appearance.panel_opacity < 0.99);
        if let Some(v) = read_volume() {
            self.volume.set_value(v);
        }
        self.syncing.set(false);
    }

    pub fn refresh(&self) {
        self.sync(&noos_core::NoosConfig::load());
    }

    pub fn toggle(&self) {
        if let Some(t) = self.hidden_at.get() {
            if t.elapsed().as_millis() < 250 {
                return;
            }
        }
        if self.window.is_visible() {
            self.hide();
        } else {
            self.refresh();
            self.window.present();
        }
    }

    pub fn hide(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
            self.hidden_at.set(Some(Instant::now()));
        }
    }
}

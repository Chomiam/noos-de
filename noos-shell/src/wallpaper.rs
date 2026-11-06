//! Fond d'écran : une surface layer-shell « background » par écran, avec diaporama et
//! transitions animées (fondu, glissement, cube…) entre deux images.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::NoosConfig;

use crate::Shell;

pub struct Wallpaper {
    pub window: gtk::Window,
    stack: gtk::Stack,
    pictures: [gtk::Picture; 2],
    /// Image affichée (index dans `pictures`).
    front: Cell<usize>,
    shown: RefCell<Option<std::path::PathBuf>>,
}

/// Image choisie, si elle existe. Sinon : dégradé aux couleurs du thème (pas d'image fournie
/// par défaut ; une nouvelle installation prend l'image du jour de Bing).
pub fn wallpaper_path(cfg: &NoosConfig) -> Option<std::path::PathBuf> {
    let custom = std::path::PathBuf::from(&cfg.appearance.wallpaper);
    (!cfg.appearance.wallpaper.is_empty() && custom.exists()).then_some(custom)
}

/// Transitions du diaporama (identifiants de la configuration).
pub const TRANSITIONS: &[&str] = &["crossfade", "slide-left", "slide-up", "over-up", "rotate", "none"];

pub fn transition_type(id: &str) -> gtk::StackTransitionType {
    match id {
        "slide-left" => gtk::StackTransitionType::SlideLeft,
        "slide-up" => gtk::StackTransitionType::SlideUp,
        "over-up" => gtk::StackTransitionType::OverUp,
        "rotate" => gtk::StackTransitionType::RotateLeft,
        "none" => gtk::StackTransitionType::None,
        "random" => {
            let i = glib::random_int_range(0, (TRANSITIONS.len() - 1) as i32) as usize;
            transition_type(TRANSITIONS[i])
        }
        _ => gtk::StackTransitionType::Crossfade,
    }
}

impl Wallpaper {
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> Self {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-wallpaper"));
        window.set_layer(Layer::Background);
        window.set_monitor(Some(monitor));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }

        let stack = gtk::Stack::new();
        stack.set_transition_duration(1200);
        let pictures = [gtk::Picture::new(), gtk::Picture::new()];
        for (i, p) in pictures.iter().enumerate() {
            p.set_content_fit(gtk::ContentFit::Cover);
            p.set_can_shrink(true);
            stack.add_named(p, Some(&i.to_string()));
        }
        window.set_child(Some(&stack));

        let w = Self { window, stack, pictures, front: Cell::new(0), shown: RefCell::new(None) };
        w.apply_config(&shell.config());
        w.window.present();
        w
    }

    /// Fond fixe (hors diaporama).
    pub fn apply_config(&self, cfg: &NoosConfig) {
        if cfg.appearance.slideshow.enabled && !cfg.appearance.slideshow.images.is_empty() {
            return; // Le diaporama du shell s'en charge.
        }
        match wallpaper_path(cfg) {
            Some(path) => self.show(&path, gtk::StackTransitionType::Crossfade),
            None => {
                *self.shown.borrow_mut() = None;
                for p in &self.pictures {
                    p.set_paintable(gdk::Paintable::NONE);
                    p.add_css_class("no-wallpaper");
                }
            }
        }
    }

    /// Affiche `path`, avec une transition depuis l'image actuelle.
    pub fn show(&self, path: &std::path::Path, transition: gtk::StackTransitionType) {
        if self.shown.borrow().as_deref() == Some(path) {
            return;
        }
        let first = self.shown.borrow().is_none();
        *self.shown.borrow_mut() = Some(path.to_path_buf());
        let back = 1 - self.front.get();
        let pic = &self.pictures[back];
        pic.remove_css_class("no-wallpaper");
        pic.set_file(Some(&gio::File::for_path(path)));
        self.stack.set_transition_type(if first { gtk::StackTransitionType::None } else { transition });
        self.stack.set_visible_child(pic);
        self.front.set(back);
    }
}

/// Diaporama : une image toutes les N minutes, sur tous les écrans à la fois.
#[derive(Default)]
pub struct Slideshow {
    key: RefCell<String>,
    order: RefCell<Vec<String>>,
    index: Cell<usize>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Slideshow {
    /// (Re)démarre le diaporama si sa configuration a changé.
    pub fn sync(&self, shell: &Rc<Shell>, cfg: &NoosConfig) {
        let s = &cfg.appearance.slideshow;
        let active = s.enabled && !s.images.is_empty();
        let key = if active { format!("{:?}{}{}{}", s.images, s.interval_minutes, s.transition, s.shuffle) } else { String::new() };
        if *self.key.borrow() == key {
            // Inchangé : un écran ajouté reçoit quand même l'image en cours.
            if active {
                if let Some(path) = self.order.borrow().get(self.index.get()) {
                    shell.show_wallpaper(path, gtk::StackTransitionType::None);
                }
            }
            return;
        }
        *self.key.borrow_mut() = key;
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
        if !active {
            return;
        }
        let mut order: Vec<String> = s.images.iter().filter(|p| std::path::Path::new(p).exists()).cloned().collect();
        if s.shuffle {
            for i in (1..order.len()).rev() {
                let j = glib::random_int_range(0, i as i32 + 1) as usize;
                order.swap(i, j);
            }
        }
        if order.is_empty() {
            return;
        }
        *self.order.borrow_mut() = order;
        self.index.set(0);
        shell.show_wallpaper(&self.order.borrow()[0], gtk::StackTransitionType::None);
        let (shell_w, transition) = (Rc::downgrade(shell), s.transition.clone());
        let secs = (s.interval_minutes.max(1) * 60) as u64;
        let id = glib::timeout_add_local(Duration::from_secs(secs), move || {
            let Some(shell) = shell_w.upgrade() else { return glib::ControlFlow::Break };
            let next = {
                let show = &shell.slideshow;
                let order = show.order.borrow();
                let i = (show.index.get() + 1) % order.len().max(1);
                show.index.set(i);
                order.get(i).cloned()
            };
            if let Some(path) = next {
                shell.show_wallpaper(&path, transition_type(&transition));
            }
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(id);
    }
}

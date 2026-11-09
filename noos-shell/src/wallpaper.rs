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
    /// Widgets du bureau : surface à part (couche « bottom », juste au-dessus du fond). Dans la
    /// surface du fond, chaque rafraîchissement d'un widget (Ressources : toutes les 2 s) faisait
    /// recalculer au compositeur tout le flou Mica sur le processeur (≈ 170 ms, souris figée).
    pub desk: gtk::Window,
    fixed: gtk::Fixed,
    /// Mode édition : toute la surface des widgets reçoit les clics (glisser-déposer).
    editing: Rc<Cell<bool>>,
    /// Dernière zone de clic envoyée (rectangles des widgets).
    input: Rc<RefCell<Vec<(i32, i32, i32, i32)>>>,
    widgets: RefCell<Vec<crate::editmode::DeskWidget>>,
    output: String,
    monitor: gdk::Monitor,
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
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        window.set_child(Some(&overlay));

        // Widgets du bureau : même taille que le fond (positions en fraction de l'écran), fond
        // transparent, clics seulement sur les widgets (zone de saisie de la surface).
        let desk = gtk::Window::builder().application(&shell.app).decorated(false).build();
        desk.add_css_class("noos-layer");
        desk.init_layer_shell();
        desk.set_namespace(Some("noos-desktop-widgets"));
        desk.set_layer(Layer::Bottom);
        desk.set_monitor(Some(monitor));
        desk.set_keyboard_mode(KeyboardMode::None);
        desk.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            desk.set_anchor(edge, true);
        }
        let fixed = gtk::Fixed::new();
        fixed.set_hexpand(true);
        fixed.set_vexpand(true);
        desk.set_child(Some(&fixed));
        // Clic droit sur le bureau : mode édition et réglages.
        {
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let pop = gtk::Popover::new();
            pop.set_has_arrow(false);
            pop.set_child(Some(&menu));
            pop.set_parent(&overlay);
            let item = |label: &str, icon: &str, f: Box<dyn Fn()>| {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                c.append(&gtk::Image::from_icon_name(icon));
                c.append(&gtk::Label::new(Some(label)));
                b.set_child(Some(&c));
                let pop = pop.clone();
                b.connect_clicked(move |_| {
                    pop.popdown();
                    f();
                });
                menu.append(&b);
            };
            let sh = Rc::downgrade(shell);
            item("Modifier le bureau", "document-edit-symbolic", Box::new(move || {
                if let Some(s) = sh.upgrade() {
                    if let Some(e) = s.editmode.borrow().as_ref() {
                        e.set_active(true);
                    }
                }
            }));
            let sh = Rc::downgrade(shell);
            item("Vue des tâches", "window-restore-symbolic", Box::new(move || {
                if let Some(s) = sh.upgrade() {
                    if let Some(t) = s.taskview.borrow().as_ref() {
                        t.toggle();
                    }
                }
            }));
            item("Changer le fond d'écran", "preferences-desktop-wallpaper-symbolic", Box::new(|| crate::apps::spawn("noos-settings background")));
            item("Paramètres d'affichage", "video-display-symbolic", Box::new(|| crate::apps::spawn("noos-settings display")));
            let click = gtk::GestureClick::new();
            click.set_button(3);
            click.connect_pressed(move |_, _, x, y| {
                pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.popup();
            });
            overlay.add_controller(click);
        }

        let w = Self {
            window,
            desk,
            fixed,
            editing: Rc::new(Cell::new(false)),
            input: Rc::new(RefCell::new(vec![])),
            widgets: RefCell::new(vec![]),
            output: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            monitor: monitor.clone(),
            stack,
            pictures,
            front: Cell::new(0),
            shown: RefCell::new(None),
        };
        // Après chaque dessin (un widget a pu changer de taille) : zone de clic à jour.
        {
            let (fixed, editing, input) = (w.fixed.clone(), w.editing.clone(), w.input.clone());
            w.desk.connect_realize(move |desk| {
                let (desk2, fixed, editing, input) = (desk.clone(), fixed.clone(), editing.clone(), input.clone());
                if let Some(clock) = desk.frame_clock() {
                    clock.connect_after_paint(move |_| sync_input(&desk2, &fixed, editing.get(), &input));
                }
            });
        }
        w.apply_config(&shell.config());
        w.window.present();
        w
    }

    /// Zone de clic de la surface des widgets : leurs rectangles (le reste laisse passer les
    /// clics vers le fond d'écran), ou toute la surface en mode édition.
    pub fn sync_input(&self) {
        sync_input(&self.desk, &self.fixed, self.editing.get(), &self.input);
    }

    /// (Re)construit les widgets du bureau de cet écran.
    pub fn build_widgets(&self, shell: &Rc<Shell>, editing: bool) {
        for w in self.widgets.borrow_mut().drain(..) {
            self.fixed.remove(&w.frame);
            self.fixed.remove_controller(&w.drag);
        }
        let cfg = shell.config();
        let primary = shell.primary_output();
        let geo = self.monitor.geometry();
        let size = (geo.width(), geo.height());
        let mut has_note = false;
        for (i, w) in cfg.desktop_widgets.iter().enumerate() {
            let mine = if w.output.is_empty() { primary.as_ref().is_none_or(|p| *p == self.output) } else { w.output == self.output };
            if !mine {
                continue;
            }
            has_note |= w.kind == "note";
            let dw = crate::editmode::build_widget(shell, i, w, &self.fixed, size);
            dw.set_editing(editing);
            self.widgets.borrow_mut().push(dw);
        }
        // Une note se tape au clavier : la surface des widgets accepte alors le focus clavier.
        self.desk.set_keyboard_mode(if has_note || editing { KeyboardMode::OnDemand } else { KeyboardMode::None });
        self.editing.set(editing);
        // Pas de widget : surface masquée (rien à composer par-dessus le fond).
        let any = !self.widgets.borrow().is_empty();
        self.desk.set_visible(any);
        if any {
            self.desk.present();
        }
        self.input.borrow_mut().clear();
        self.sync_input();
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

/// Zone de clic de la surface des widgets (voir `Wallpaper::sync_input`).
fn sync_input(desk: &gtk::Window, fixed: &gtk::Fixed, editing: bool, input: &RefCell<Vec<(i32, i32, i32, i32)>>) {
    let Some(surface) = desk.surface() else { return };
    let rects: Vec<(i32, i32, i32, i32)> = if editing {
        vec![(0, 0, desk.width().max(1), desk.height().max(1))]
    } else {
        let mut v = vec![];
        let mut child = fixed.first_child();
        while let Some(c) = child {
            if let Some(b) = c.compute_bounds(desk) {
                v.push((b.x().floor() as i32 - 4, b.y().floor() as i32 - 4, b.width().ceil() as i32 + 8, b.height().ceil() as i32 + 8));
            }
            child = c.next_sibling();
        }
        v
    };
    if *input.borrow() == rects {
        return;
    }
    let region = gtk::cairo::Region::create();
    for (x, y, w, h) in &rects {
        let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(*x, *y, *w, *h));
    }
    surface.set_input_region(Some(&region));
    *input.borrow_mut() = rects;
    // La zone n'est transmise qu'avec le prochain dessin : en demander un (sans boucle : la
    // zone sera alors identique).
    desk.queue_draw();
}

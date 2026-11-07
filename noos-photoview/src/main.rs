//! noos-photoview — « Photos », la visionneuse d'images de NOOS (GTK4 + libAdapta).
//!
//! - affichage immersif : fond « ambilight » (l'image floutée derrière elle-même), barre de
//!   titre et commandes qui s'effacent après quelques secondes sans mouvement ;
//! - zoom fluide centré sur le pointeur (molette, pincement, + / -, double-clic 100 % ↔ ajusté),
//!   déplacement à la souris, rotation (R / Maj+R) et miroir (M) ;
//! - navigation dans le dossier (← / →, Début / Fin, boutons latéraux), bande de miniatures,
//!   fondu entre les images, diaporama (F5), plein écran (F11) ;
//! - panneau d'informations (I) : fichier, dimensions, EXIF (appareil, objectif, exposition,
//!   ouverture, ISO, focale, flash), position GPS avec lien vers la carte ;
//! - copier l'image (Ctrl+C), fond d'écran NOOS, ouvrir avec, afficher dans l'Explorateur,
//!   mettre à la corbeille (Suppr), ouvrir (Ctrl+O) ou glisser-déposer une image.

mod exif_info;
mod loader;
mod thumbs;

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "org.noos.PhotoView";
const ZOOM_MIN: f64 = 0.05;
const ZOOM_MAX: f64 = 20.0;
const HIDE_AFTER: Duration = Duration::from_millis(2600);
const SLIDE_EVERY: Duration = Duration::from_secs(4);

thread_local! {
    static VIEWER: RefCell<Option<Rc<Viewer>>> = const { RefCell::new(None) };
}

fn viewer() -> Option<Rc<Viewer>> {
    VIEWER.with(|v| v.borrow().clone())
}

struct Viewer {
    window: adap::ApplicationWindow,
    toolbar: adap::ToolbarView,
    toasts: adap::ToastOverlay,
    split: adap::OverlaySplitView,
    info_box: gtk::Box,
    stack: gtk::Stack,
    scroller: gtk::ScrolledWindow,
    picture: gtk::Picture,
    ambient: gtk::Picture,
    title: adap::WindowTitle,
    zoom_label: gtk::Button,
    controls: gtk::Revealer,
    side_buttons: Vec<gtk::Revealer>,
    strip: gtk::ListView,
    strip_model: gtk::StringList,
    strip_sel: gtk::SingleSelection,
    files: RefCell<Vec<PathBuf>>,
    index: Cell<usize>,
    texture: RefCell<Option<gdk::Texture>>,
    size: Cell<(i32, i32)>,
    /// `None` : ajusté à la fenêtre ; sinon facteur de zoom (1.0 = 100 %).
    zoom: Cell<Option<f64>>,
    rotation: Cell<u32>,
    flip: Cell<bool>,
    format: RefCell<String>,
    generation: Cell<u64>,
    pointer: Cell<(f64, f64)>,
    slideshow: RefCell<Option<glib::SourceId>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
}

fn osd_button(icon: &str, tip: &str, action: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.set_action_name(Some(action));
    b.add_css_class("flat");
    b.add_css_class("circular");
    b
}

fn human_size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

impl Viewer {
    fn new(app: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Photos")
            .default_width(1200)
            .default_height(800)
            .build();
        window.add_css_class("photoview");

        // ── Image : fond « ambilight » + image zoomable dans une zone défilante ──
        let ambient = gtk::Picture::new();
        ambient.set_content_fit(gtk::ContentFit::Cover);
        ambient.add_css_class("ambient");
        ambient.set_can_target(false);
        let picture = gtk::Picture::new();
        picture.set_halign(gtk::Align::Center);
        picture.set_valign(gtk::Align::Center);
        picture.set_content_fit(gtk::ContentFit::ScaleDown);
        picture.add_css_class("photo");
        let scroller = gtk::ScrolledWindow::builder()
            .child(&picture)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        scroller.add_css_class("viewer-area");
        let image_layer = gtk::Overlay::new();
        image_layer.set_child(Some(&ambient));
        image_layer.add_overlay(&scroller);

        // Boutons Précédent / Suivant sur les côtés.
        let mut side_buttons = Vec::new();
        for (icon, action, tip, align) in [
            ("go-previous-symbolic", "win.prev", "Image précédente (←)", gtk::Align::Start),
            ("go-next-symbolic", "win.next", "Image suivante (→)", gtk::Align::End),
        ] {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("circular");
            b.add_css_class("osd");
            b.add_css_class("side-nav");
            b.set_action_name(Some(action));
            b.set_tooltip_text(Some(tip));
            let r = gtk::Revealer::builder()
                .transition_type(gtk::RevealerTransitionType::Crossfade)
                .transition_duration(200)
                .child(&b)
                .halign(align)
                .valign(gtk::Align::Center)
                .margin_start(18)
                .margin_end(18)
                .reveal_child(true)
                .build();
            image_layer.add_overlay(&r);
            side_buttons.push(r);
        }

        // ── Commandes du bas : bande de miniatures + barre d'outils ──
        let strip_model = gtk::StringList::new(&[]);
        let strip_sel = gtk::SingleSelection::new(Some(strip_model.clone()));
        strip_sel.set_autoselect(false);
        let strip = gtk::ListView::new(Some(strip_sel.clone()), Some(strip_factory()));
        strip.set_orientation(gtk::Orientation::Horizontal);
        strip.add_css_class("strip");
        let strip_scroll = gtk::ScrolledWindow::builder()
            .child(&strip)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .min_content_height(76)
            .build();
        strip_scroll.set_halign(gtk::Align::Center);
        strip_scroll.set_propagate_natural_width(true);
        strip_scroll.set_max_content_width(900);

        let zoom_label = gtk::Button::with_label("100 %");
        zoom_label.add_css_class("flat");
        zoom_label.add_css_class("zoom-label");
        zoom_label.set_action_name(Some("win.zoom-fit"));
        zoom_label.set_tooltip_text(Some("Ajuster à la fenêtre (0)"));
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        bar.add_css_class("osd-bar");
        bar.set_halign(gtk::Align::Center);
        bar.append(&osd_button("zoom-out-symbolic", "Réduire (−)", "win.zoom-out"));
        bar.append(&zoom_label);
        bar.append(&osd_button("zoom-in-symbolic", "Agrandir (+)", "win.zoom-in"));
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bar.append(&osd_button("object-rotate-left-symbolic", "Pivoter à gauche (Maj+R)", "win.rotate-ccw"));
        bar.append(&osd_button("object-rotate-right-symbolic", "Pivoter à droite (R)", "win.rotate-cw"));
        bar.append(&osd_button("object-flip-horizontal-symbolic", "Miroir (M)", "win.flip"));
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        bar.append(&osd_button("media-playback-start-symbolic", "Diaporama (F5)", "win.slideshow"));
        bar.append(&osd_button("view-fullscreen-symbolic", "Plein écran (F11)", "win.fullscreen"));
        let bottom = gtk::Box::new(gtk::Orientation::Vertical, 8);
        bottom.add_css_class("bottom-osd");
        bottom.append(&strip_scroll);
        bottom.append(&bar);
        let controls = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(220)
            .child(&bottom)
            .valign(gtk::Align::End)
            .reveal_child(true)
            .build();
        image_layer.add_overlay(&controls);

        // ── Page vide (aucune image ouverte) ──
        let empty = adap::StatusPage::builder()
            .icon_name("image-x-generic-symbolic")
            .title("Photos")
            .description("Ouvrez une image, ou glissez-la ici. ← et → parcourent les images du dossier.")
            .build();
        let open_btn = gtk::Button::with_label("Ouvrir une image…");
        open_btn.add_css_class("pill");
        open_btn.add_css_class("suggested-action");
        open_btn.set_action_name(Some("win.open"));
        let pictures_btn = gtk::Button::with_label("Parcourir le dossier Images");
        pictures_btn.add_css_class("pill");
        pictures_btn.set_action_name(Some("win.open-pictures"));
        let empty_actions = gtk::Box::new(gtk::Orientation::Vertical, 10);
        empty_actions.set_halign(gtk::Align::Center);
        empty_actions.append(&open_btn);
        empty_actions.append(&pictures_btn);
        empty.set_child(Some(&empty_actions));

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&image_layer, Some("view"));

        // ── Panneau d'informations (à droite) ──
        let info_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
        info_box.add_css_class("info-panel");
        let info_scroll = gtk::ScrolledWindow::builder().child(&info_box).hscrollbar_policy(gtk::PolicyType::Never).build();
        let split = adap::OverlaySplitView::new();
        split.set_sidebar_position(gtk::PackType::End);
        split.set_sidebar(Some(&info_scroll));
        split.set_content(Some(&stack));
        split.set_show_sidebar(false);
        split.set_max_sidebar_width(340.0);
        split.set_min_sidebar_width(300.0);

        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&split));

        // ── Barre de titre ──
        let header = adap::HeaderBar::new();
        let title = adap::WindowTitle::new("Photos", "");
        header.set_title_widget(Some(&title));
        let open = gtk::Button::from_icon_name("document-open-symbolic");
        open.set_tooltip_text(Some("Ouvrir (Ctrl+O)"));
        open.set_action_name(Some("win.open"));
        header.pack_start(&open);
        let menu = gio::Menu::new();
        let s1 = gio::Menu::new();
        s1.append(Some("Ouvrir avec…"), Some("win.open-with"));
        s1.append(Some("Afficher dans l'Explorateur"), Some("win.show-in-folder"));
        menu.append_section(None, &s1);
        let s2 = gio::Menu::new();
        s2.append(Some("Copier l'image"), Some("win.copy"));
        s2.append(Some("Définir comme fond d'écran"), Some("win.wallpaper"));
        menu.append_section(None, &s2);
        let s3 = gio::Menu::new();
        s3.append(Some("Mettre à la corbeille"), Some("win.trash"));
        menu.append_section(None, &s3);
        let s4 = gio::Menu::new();
        s4.append(Some("Raccourcis clavier"), Some("win.shortcuts"));
        s4.append(Some("À propos de Photos"), Some("win.about"));
        menu.append_section(None, &s4);
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&menu));
        header.pack_end(&menu_btn);
        let info_btn = gtk::ToggleButton::new();
        info_btn.set_icon_name("dialog-information-symbolic");
        info_btn.set_tooltip_text(Some("Informations (I)"));
        info_btn.set_action_name(Some("win.info"));
        header.pack_end(&info_btn);

        let toolbar = adap::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        toolbar.set_extend_content_to_top_edge(true);
        toolbar.set_top_bar_style(adap::ToolbarStyle::Raised);
        window.set_content(Some(&toolbar));

        let v = Rc::new(Self {
            window,
            toolbar,
            toasts,
            split,
            info_box,
            stack,
            scroller,
            picture,
            ambient,
            title,
            zoom_label,
            controls,
            side_buttons,
            strip,
            strip_model,
            strip_sel,
            files: RefCell::new(vec![]),
            index: Cell::new(0),
            texture: RefCell::new(None),
            size: Cell::new((0, 0)),
            zoom: Cell::new(None),
            rotation: Cell::new(0),
            flip: Cell::new(false),
            format: RefCell::new(String::new()),
            generation: Cell::new(0),
            pointer: Cell::new((0.0, 0.0)),
            slideshow: RefCell::new(None),
            hide_timer: RefCell::new(None),
        });
        VIEWER.with(|c| *c.borrow_mut() = Some(v.clone()));
        v.connect_input();
        v.install_actions(app);
        v.update_actions();
        v
    }

    // ───────────────────────── Fichiers ─────────────────────────

    /// Ouvre une image (ou la première image d'un dossier) et charge les images voisines.
    fn open_path(self: &Rc<Self>, path: &Path) {
        let (dir, target) = if path.is_dir() { (path.to_path_buf(), None) } else { (path.parent().unwrap_or(Path::new("/")).to_path_buf(), Some(path)) };
        let mut files = loader::siblings(&dir, target.is_some_and(|t| t.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'))));
        let index = match target {
            Some(t) => match files.iter().position(|f| f == t) {
                Some(i) => i,
                None => {
                    // Fichier non reconnu par son extension : on l'ouvre quand même, seul.
                    files = vec![t.to_path_buf()];
                    0
                }
            },
            None => 0,
        };
        if files.is_empty() {
            self.toast("Aucune image dans ce dossier");
            return;
        }
        let names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.strip_model.splice(0, self.strip_model.n_items(), &refs);
        *self.files.borrow_mut() = files;
        self.stack.set_visible_child_name("view");
        self.show_index(index);
    }

    fn current(&self) -> Option<PathBuf> {
        self.files.borrow().get(self.index.get()).cloned()
    }

    fn show_index(self: &Rc<Self>, index: usize) {
        let Some(path) = self.files.borrow().get(index).cloned() else { return };
        let changed = index != self.index.get() || self.texture.borrow().is_none();
        self.index.set(index);
        if changed {
            self.rotation.set(0);
            self.flip.set(false);
        }
        if self.strip_sel.selected() != index as u32 {
            self.strip_sel.set_selected(index as u32);
        }
        self.strip.scroll_to(index as u32, gtk::ListScrollFlags::NONE, None);
        self.load(path, changed);
    }

    /// Décode l'image courante (avec rotation / miroir) puis l'affiche.
    fn load(self: &Rc<Self>, path: PathBuf, fade: bool) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.title.set_title(&name);
        self.window.set_title(Some(&format!("{name} — Photos")));
        let (rotation, flip) = (self.rotation.get(), self.flip.get());
        let v = self.clone();
        glib::spawn_future_local(async move {
            let res = loader::decode(path.clone(), rotation, flip).await;
            if v.generation.get() != generation {
                return; // une autre image a été demandée entre-temps
            }
            match res {
                Ok(d) => {
                    let texture = d.texture();
                    v.picture.set_paintable(Some(&texture));
                    *v.texture.borrow_mut() = Some(texture.clone());
                    v.size.set((d.width, d.height));
                    *v.format.borrow_mut() = d.format.clone();
                    v.fit();
                    v.update_subtitle();
                    v.update_ambient(&path, &texture);
                    if fade {
                        fade_in(&v.picture);
                    }
                    v.update_info();
                }
                Err(e) => {
                    v.picture.set_paintable(gdk::Paintable::NONE);
                    *v.texture.borrow_mut() = None;
                    v.title.set_subtitle("Image illisible");
                    v.toast(&format!("Impossible d'afficher {name} : {e}"));
                }
            }
            v.update_actions();
        });
    }

    /// Fond « ambilight » : la miniature (petite, donc floue à peu de frais) étirée derrière.
    fn update_ambient(self: &Rc<Self>, path: &Path, texture: &gdk::Texture) {
        self.ambient.set_paintable(Some(texture));
        let (v, path) = (self.clone(), path.to_path_buf());
        let generation = self.generation.get();
        glib::spawn_future_local(async move {
            if let Some(thumb) = thumbs::thumbnail(path).await {
                if v.generation.get() == generation && v.rotation.get() == 0 && !v.flip.get() {
                    if let Ok(t) = gdk::Texture::from_filename(thumb) {
                        v.ambient.set_paintable(Some(&t));
                    }
                }
            }
        });
    }

    fn update_subtitle(&self) {
        let n = self.files.borrow().len();
        let (w, h) = self.size.get();
        let mut parts = Vec::new();
        if n > 1 {
            parts.push(format!("{} / {n}", self.index.get() + 1));
        }
        if w > 0 {
            parts.push(format!("{w} × {h}"));
        }
        let fmt = self.format.borrow().clone();
        if !fmt.is_empty() {
            parts.push(fmt);
        }
        self.title.set_subtitle(&parts.join(" · "));
    }

    fn step(self: &Rc<Self>, delta: isize) {
        let n = self.files.borrow().len() as isize;
        if n == 0 {
            return;
        }
        let i = (self.index.get() as isize + delta).rem_euclid(n);
        self.show_index(i as usize);
    }

    // ───────────────────────── Zoom ─────────────────────────

    fn viewport(&self) -> (f64, f64) {
        (self.scroller.width().max(1) as f64, self.scroller.height().max(1) as f64)
    }

    /// Zoom réel actuellement affiché (en mode ajusté : sans agrandir les petites images).
    fn effective_zoom(&self) -> f64 {
        if let Some(z) = self.zoom.get() {
            return z;
        }
        let (w, h) = self.size.get();
        if w <= 0 || h <= 0 {
            return 1.0;
        }
        let (vw, vh) = self.viewport();
        (vw / w as f64).min(vh / h as f64).min(1.0)
    }

    fn fit(&self) {
        self.zoom.set(None);
        self.picture.set_size_request(-1, -1);
        self.picture.set_can_shrink(true);
        self.picture.set_content_fit(gtk::ContentFit::ScaleDown);
        self.scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        self.picture.remove_css_class("zoomed");
        self.update_zoom_label();
    }

    /// Zoom à `z`, en gardant fixe le point de l'image sous `anchor` (coordonnées de la zone).
    fn set_zoom(self: &Rc<Self>, z: f64, anchor: Option<(f64, f64)>) {
        let (w, h) = self.size.get();
        if w <= 0 || self.texture.borrow().is_none() {
            return;
        }
        let z = z.clamp(ZOOM_MIN, ZOOM_MAX);
        let old = self.effective_zoom();
        let (vw, vh) = self.viewport();
        let (ax, ay) = anchor.unwrap_or((vw / 2.0, vh / 2.0));
        // Point de l'image sous le pointeur, avant le changement.
        let (hadj, vadj) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        let off = |view: f64, size: f64| ((view - size) / 2.0).max(0.0);
        let (sx, sy) = if self.zoom.get().is_some() { (hadj.value(), vadj.value()) } else { (0.0, 0.0) };
        let ix = (sx + ax - off(vw, w as f64 * old)) / old;
        let iy = (sy + ay - off(vh, h as f64 * old)) / old;

        // Fit plus grand que le zoom demandé et image entière visible : rester ajusté.
        let fit_zoom = (vw / w as f64).min(vh / h as f64);
        if z <= fit_zoom.min(1.0) + 1e-3 && self.zoom.get().is_none() && z <= old {
            self.update_zoom_label();
            return;
        }
        self.zoom.set(Some(z));
        self.picture.set_can_shrink(false);
        self.picture.set_content_fit(gtk::ContentFit::Fill);
        self.picture.set_size_request((w as f64 * z).round() as i32, (h as f64 * z).round() as i32);
        self.picture.add_css_class("zoomed");
        self.scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        self.update_zoom_label();
        // Après la mise en page : recaler le défilement pour garder le point sous le pointeur.
        // La taille de la zone défilante n'est connue qu'après la mise en page : on attend que
        // ses bornes correspondent au nouveau zoom (quelques images au plus).
        let v = self.clone();
        let frames = Rc::new(Cell::new(0u32));
        let (zw, zh) = (w as f64 * z, h as f64 * z);
        self.scroller.add_tick_callback(move |_, _| {
            let (vw, vh) = v.viewport();
            let (hadj, vadj) = (v.scroller.hadjustment(), v.scroller.vadjustment());
            let ready = (hadj.upper() - zw.max(vw)).abs() < 2.0 && (vadj.upper() - zh.max(vh)).abs() < 2.0;
            frames.set(frames.get() + 1);
            if !ready && frames.get() < 10 {
                return glib::ControlFlow::Continue;
            }
            hadj.set_value(ix * z + off(vw, zw) - ax);
            vadj.set_value(iy * z + off(vh, zh) - ay);
            glib::ControlFlow::Break
        });
    }

    fn zoom_by(self: &Rc<Self>, factor: f64, anchor: Option<(f64, f64)>) {
        let z = self.effective_zoom() * factor;
        // Revenir en mode ajusté quand on dézoome sous la taille de la fenêtre.
        let (w, h) = self.size.get();
        let (vw, vh) = self.viewport();
        let fit = (vw / w.max(1) as f64).min(vh / h.max(1) as f64).min(1.0);
        if factor < 1.0 && z <= fit {
            self.fit();
        } else {
            self.set_zoom(z, anchor);
        }
    }

    fn update_zoom_label(&self) {
        let pct = (self.effective_zoom() * 100.0).round();
        self.zoom_label.set_label(&format!("{pct} %"));
    }

    // ───────────────────────── Commandes visibles / masquées ─────────────────────────

    fn set_chrome(&self, visible: bool) {
        self.toolbar.set_reveal_top_bars(visible);
        self.controls.set_reveal_child(visible && self.slideshow.borrow().is_none());
        let multi = self.files.borrow().len() > 1;
        for r in &self.side_buttons {
            r.set_reveal_child(visible && multi);
        }
        self.window.set_cursor_from_name(if visible { None } else { Some("none") });
    }

    /// Mouvement : on réaffiche les commandes, qui s'effacent ensuite après HIDE_AFTER.
    fn poke(self: &Rc<Self>) {
        self.set_chrome(true);
        if let Some(id) = self.hide_timer.borrow_mut().take() {
            id.remove();
        }
        if self.texture.borrow().is_none() || self.split.shows_sidebar() {
            return;
        }
        let v = self.clone();
        *self.hide_timer.borrow_mut() = Some(glib::timeout_add_local_once(HIDE_AFTER, move || {
            v.hide_timer.borrow_mut().take();
            v.set_chrome(false);
        }));
    }

    fn toggle_slideshow(self: &Rc<Self>) {
        if let Some(id) = self.slideshow.borrow_mut().take() {
            id.remove();
            self.toast("Diaporama arrêté");
            self.window.unfullscreen();
            self.poke();
            return;
        }
        if self.files.borrow().len() < 2 {
            self.toast("Le diaporama demande au moins deux images");
            return;
        }
        self.window.fullscreen();
        let v = self.clone();
        *self.slideshow.borrow_mut() = Some(glib::timeout_add_local(SLIDE_EVERY, move || {
            v.step(1);
            glib::ControlFlow::Continue
        }));
        self.set_chrome(false);
        self.toast("Diaporama — Échap pour arrêter");
    }

    // ───────────────────────── Entrées (souris, gestes) ─────────────────────────

    fn connect_input(self: &Rc<Self>) {
        let motion = gtk::EventControllerMotion::new();
        let v = self.clone();
        motion.connect_motion(move |_, x, y| {
            let (px, py) = v.pointer.get();
            if (px - x).abs() + (py - y).abs() > 2.0 {
                v.pointer.set((x, y));
                if v.slideshow.borrow().is_none() {
                    v.poke();
                }
            }
        });
        self.scroller.add_controller(motion);

        // Molette : zoom centré sur le pointeur.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let v = self.clone();
        scroll.connect_scroll(move |_, _, dy| {
            if v.texture.borrow().is_none() {
                return glib::Propagation::Proceed;
            }
            v.zoom_by(1.18f64.powf(-dy.clamp(-3.0, 3.0)), Some(v.pointer.get()));
            glib::Propagation::Stop
        });
        self.scroller.add_controller(scroll);

        // Double-clic : 100 % ↔ ajusté.
        let click = gtk::GestureClick::new();
        let v = self.clone();
        click.connect_pressed(move |_, n, x, y| {
            if n == 2 {
                if v.zoom.get().is_some() {
                    v.fit();
                } else {
                    v.set_zoom(1.0, Some((x, y)));
                }
            }
        });
        self.scroller.add_controller(click);

        // Glisser : déplacer l'image agrandie.
        let drag = gtk::GestureDrag::new();
        let start = Rc::new(Cell::new((0.0, 0.0)));
        {
            let (v, start) = (self.clone(), start.clone());
            drag.connect_drag_begin(move |_, _, _| {
                start.set((v.scroller.hadjustment().value(), v.scroller.vadjustment().value()));
                if v.zoom.get().is_some() {
                    v.scroller.set_cursor_from_name(Some("grabbing"));
                }
            });
        }
        {
            let v = self.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                if v.zoom.get().is_some() {
                    let (sx, sy) = start.get();
                    v.scroller.hadjustment().set_value(sx - dx);
                    v.scroller.vadjustment().set_value(sy - dy);
                }
            });
        }
        {
            let v = self.clone();
            drag.connect_drag_end(move |_, _, _| v.scroller.set_cursor_from_name(None));
        }
        self.scroller.add_controller(drag);

        // Pincement (pavé tactile, écran tactile).
        let pinch = gtk::GestureZoom::new();
        let base = Rc::new(Cell::new(1.0));
        {
            let (v, base) = (self.clone(), base.clone());
            pinch.connect_begin(move |_, _| base.set(v.effective_zoom()));
        }
        {
            let v = self.clone();
            pinch.connect_scale_changed(move |g, scale| {
                let anchor = g.bounding_box_center();
                v.set_zoom(base.get() * scale, anchor);
            });
        }
        self.scroller.add_controller(pinch);

        // Sélection dans la bande de miniatures.
        let v = self.clone();
        self.strip_sel.connect_selected_notify(move |s| {
            let i = s.selected() as usize;
            if s.selected() != gtk::INVALID_LIST_POSITION && i != v.index.get() {
                v.show_index(i);
            }
        });

        // Redimensionnement : l'étiquette de zoom suit le mode ajusté.
        let v = self.clone();
        self.scroller.connect_notify_local(Some("width"), move |_, _| v.update_zoom_label());

        // Déposer une image dans la fenêtre.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let v = self.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            match list.files().first().and_then(|f| f.path()) {
                Some(p) => {
                    v.open_path(&p);
                    true
                }
                None => false,
            }
        });
        self.window.add_controller(drop);

        // Panneau d'informations ouvert : les commandes restent visibles.
        let v = self.clone();
        self.split.connect_show_sidebar_notify(move |_| v.poke());
    }

    // ───────────────────────── Actions ─────────────────────────

    fn install_actions(self: &Rc<Self>, app: &adap::Application) {
        let add = |name: &str, accels: &[&str], f: Box<dyn Fn(&Rc<Viewer>)>| {
            let a = gio::SimpleAction::new(name, None);
            let v = self.clone();
            a.connect_activate(move |_, _| f(&v));
            self.window.add_action(&a);
            app.set_accels_for_action(&format!("win.{name}"), accels);
        };
        add("next", &["Right", "Page_Down", "space"], Box::new(|v| v.step(1)));
        add("prev", &["Left", "Page_Up", "BackSpace"], Box::new(|v| v.step(-1)));
        add("first", &["Home"], Box::new(|v| v.show_index(0)));
        add("last", &["End"], Box::new(|v| {
            let n = v.files.borrow().len();
            if n > 0 {
                v.show_index(n - 1);
            }
        }));
        add("zoom-in", &["plus", "equal", "KP_Add", "<Ctrl>plus", "<Ctrl>equal"], Box::new(|v| v.zoom_by(1.25, None)));
        add("zoom-out", &["minus", "KP_Subtract", "<Ctrl>minus"], Box::new(|v| v.zoom_by(0.8, None)));
        add("zoom-fit", &["0", "KP_0", "<Ctrl>0"], Box::new(|v| v.fit()));
        add("zoom-100", &["1", "KP_1"], Box::new(|v| v.set_zoom(1.0, None)));
        add("rotate-cw", &["r"], Box::new(|v| {
            v.rotation.set((v.rotation.get() + 1) % 4);
            if let Some(p) = v.current() {
                v.load(p, false);
            }
        }));
        add("rotate-ccw", &["<Shift>r"], Box::new(|v| {
            v.rotation.set((v.rotation.get() + 3) % 4);
            if let Some(p) = v.current() {
                v.load(p, false);
            }
        }));
        add("flip", &["m"], Box::new(|v| {
            v.flip.set(!v.flip.get());
            if let Some(p) = v.current() {
                v.load(p, false);
            }
        }));
        add("fullscreen", &["F11", "f"], Box::new(|v| {
            if v.window.is_fullscreen() {
                v.window.unfullscreen();
            } else {
                v.window.fullscreen();
            }
        }));
        add("slideshow", &["F5"], Box::new(|v| v.toggle_slideshow()));
        add("escape", &["Escape"], Box::new(|v| {
            if v.slideshow.borrow().is_some() {
                v.toggle_slideshow();
            } else if v.window.is_fullscreen() {
                v.window.unfullscreen();
            } else if v.split.shows_sidebar() {
                v.activate("info");
            }
        }));
        add("open", &["<Ctrl>o"], Box::new(|v| v.choose_file()));
        add("open-pictures", &[], Box::new(|v| {
            if let Some(p) = glib::user_special_dir(glib::UserDirectory::Pictures) {
                v.open_path(&p);
            }
        }));
        add("copy", &["<Ctrl>c"], Box::new(|v| {
            if let (Some(t), Some(d)) = (v.texture.borrow().clone(), gdk::Display::default()) {
                d.clipboard().set_texture(&t);
                v.toast("Image copiée");
            }
        }));
        add("trash", &["Delete"], Box::new(|v| v.trash_current()));
        add("wallpaper", &[], Box::new(|v| v.set_wallpaper()));
        add("open-with", &[], Box::new(|v| {
            let Some(p) = v.current() else { return };
            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(p)));
            launcher.set_always_ask(true);
            launcher.launch(Some(&v.window), gio::Cancellable::NONE, |_| {});
        }));
        add("show-in-folder", &[], Box::new(|v| {
            let Some(dir) = v.current().and_then(|p| p.parent().map(Path::to_path_buf)) else { return };
            if std::process::Command::new("noos-file").arg(&dir).spawn().is_err() {
                let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(dir).uri(), gio::AppLaunchContext::NONE);
            }
        }));
        add("shortcuts", &["<Ctrl>question"], Box::new(|v| v.show_shortcuts()));
        add("about", &[], Box::new(|v| {
            let about = adap::AboutDialog::builder()
                .application_name("Photos")
                .application_icon("multimedia-photo-viewer")
                .developer_name("NOOS")
                .version(env!("CARGO_PKG_VERSION"))
                .license_type(gtk::License::Gpl30)
                .website("https://github.com/Chomiam/noos-de")
                .comments("La visionneuse d'images de NOOS : zoom fluide, diaporama, informations EXIF.")
                .build();
            about.present(Some(&v.window));
        }));

        let info = gio::SimpleAction::new_stateful("info", None, &false.to_variant());
        let v = self.clone();
        info.connect_activate(move |a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            v.split.set_show_sidebar(on);
            v.update_info();
        });
        self.window.add_action(&info);
        app.set_accels_for_action("win.info", &["i", "<Alt>Return"]);
    }

    fn activate(&self, name: &str) {
        gtk::prelude::ActionGroupExt::activate_action(&self.window, name, None);
    }

    fn update_actions(&self) {
        let has = self.texture.borrow().is_some();
        let multi = self.files.borrow().len() > 1;
        for (name, on) in [
            ("next", multi),
            ("prev", multi),
            ("first", multi),
            ("last", multi),
            ("slideshow", multi),
            ("zoom-in", has),
            ("zoom-out", has),
            ("zoom-fit", has),
            ("zoom-100", has),
            ("rotate-cw", has),
            ("rotate-ccw", has),
            ("flip", has),
            ("copy", has),
            ("trash", has),
            ("wallpaper", has),
            ("open-with", has),
            ("show-in-folder", has),
        ] {
            if let Some(a) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                a.set_enabled(on);
            }
        }
        for r in &self.side_buttons {
            r.set_visible(multi);
        }
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    fn choose_file(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images"));
        filter.add_pixbuf_formats();
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Ouvrir une image").filters(&filters).modal(true).build();
        if let Some(dir) = self.current().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| glib::user_special_dir(glib::UserDirectory::Pictures)) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let v = self.clone();
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                v.open_path(&p);
            }
        });
    }

    fn trash_current(self: &Rc<Self>) {
        let Some(path) = self.current() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if let Err(e) = gio::File::for_path(&path).trash(gio::Cancellable::NONE) {
            self.toast(&format!("Mise à la corbeille impossible : {e}"));
            return;
        }
        let i = self.index.get();
        self.files.borrow_mut().remove(i);
        self.strip_model.remove(i as u32);
        let n = self.files.borrow().len();
        self.toast(&format!("« {name} » mise à la corbeille"));
        if n == 0 {
            *self.texture.borrow_mut() = None;
            self.picture.set_paintable(gdk::Paintable::NONE);
            self.stack.set_visible_child_name("empty");
            self.title.set_title("Photos");
            self.title.set_subtitle("");
            self.update_actions();
        } else {
            self.texture.borrow_mut().take();
            self.show_index(i.min(n - 1));
        }
    }

    fn set_wallpaper(&self) {
        let Some(path) = self.current() else { return };
        let mut cfg = noos_core::NoosConfig::load();
        cfg.appearance.wallpaper = path.to_string_lossy().to_string();
        cfg.appearance.bing_daily = false;
        cfg.appearance.slideshow.enabled = false;
        match cfg.save() {
            Ok(()) => {
                std::thread::spawn(|| {
                    let _ = noos_core::ipc::IpcClient::fire(&noos_core::ipc::Request::ReloadConfig);
                });
                self.toast("Fond d'écran modifié");
            }
            Err(e) => self.toast(&format!("Fond d'écran : {e}")),
        }
    }

    fn show_shortcuts(&self) {
        let rows = [
            ("← / →", "Image précédente / suivante"),
            ("Début / Fin", "Première / dernière image"),
            ("Molette, + / −", "Agrandir / réduire (centré sur le pointeur)"),
            ("Double-clic, 0 / 1", "Ajuster à la fenêtre / taille réelle"),
            ("R / Maj+R", "Pivoter à droite / à gauche"),
            ("M", "Miroir"),
            ("I", "Informations"),
            ("F11 ou F", "Plein écran"),
            ("F5", "Diaporama (Échap pour arrêter)"),
            ("Ctrl+C", "Copier l'image"),
            ("Suppr", "Mettre à la corbeille"),
            ("Ctrl+O", "Ouvrir une image"),
        ];
        let g = adap::PreferencesGroup::new();
        for (keys, what) in rows {
            let r = adap::ActionRow::new();
            r.set_title(what);
            let k = gtk::Label::new(Some(keys));
            k.add_css_class("keycap");
            r.add_suffix(&k);
            g.add(&r);
        }
        let page = adap::PreferencesPage::new();
        page.add(&g);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&page));
        let d = adap::Dialog::builder().title("Raccourcis clavier").content_width(460).content_height(620).child(&tv).build();
        d.present(Some(&self.window));
    }

    // ───────────────────────── Panneau d'informations ─────────────────────────

    fn update_info(self: &Rc<Self>) {
        if !self.split.shows_sidebar() {
            return;
        }
        while let Some(c) = self.info_box.first_child() {
            self.info_box.remove(&c);
        }
        let Some(path) = self.current() else { return };
        let row = |title: &str, value: &str| {
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title(title);
            r.set_subtitle(value);
            r.set_subtitle_selectable(true);
            r.add_css_class("property");
            r
        };
        let name = gtk::Label::new(Some(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        name.add_css_class("title-3");
        name.set_wrap(true);
        name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        name.set_xalign(0.0);
        self.info_box.append(&name);

        let file = adap::PreferencesGroup::new();
        file.set_title("Fichier");
        let (w, h) = self.size.get();
        if w > 0 {
            file.add(&row("Dimensions", &format!("{w} × {h} pixels ({:.1} Mpx)", (w as f64 * h as f64) / 1e6)));
        }
        if let Ok(meta) = std::fs::metadata(&path) {
            file.add(&row("Taille", &human_size(meta.len())));
            if let Some(t) = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()) {
                if let Some(d) = glib::DateTime::from_unix_local(t.as_secs() as i64).ok().and_then(|d| d.format("%-d %B %Y à %H:%M").ok()) {
                    file.add(&row("Modifié le", &d));
                }
            }
        }
        let fmt = self.format.borrow().clone();
        if !fmt.is_empty() {
            file.add(&row("Format", &fmt));
        }
        if let Some(dir) = path.parent() {
            file.add(&row("Dossier", &dir.to_string_lossy()));
        }
        self.info_box.append(&file);

        // EXIF, lu en arrière-plan.
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 18);
        self.info_box.append(&holder);
        let generation = self.generation.get();
        let v = self.clone();
        glib::spawn_future_local(async move {
            let Some(shot) = exif_info::read(path).await else { return };
            if v.generation.get() != generation || shot.is_empty() {
                return;
            }
            let g = adap::PreferencesGroup::new();
            g.set_title("Prise de vue");
            for (title, value, icon) in [
                ("Appareil", &shot.camera, "camera-photo-symbolic"),
                ("Objectif", &shot.lens, "camera-web-symbolic"),
                ("Date de prise de vue", &shot.date, "x-office-calendar-symbolic"),
                ("Exposition", &shot.exposure, "weather-clear-symbolic"),
                ("Ouverture", &shot.aperture, "camera-shutter-symbolic"),
                ("Sensibilité", &shot.iso, "display-brightness-symbolic"),
                ("Focale", &shot.focal, "zoom-in-symbolic"),
                ("Flash", &shot.flash, "weather-storm-symbolic"),
            ] {
                if let Some(value) = value {
                    let r = row(title, value);
                    r.add_prefix(&gtk::Image::from_icon_name(icon));
                    g.add(&r);
                }
            }
            holder.append(&g);
            if let Some((lat, lon)) = shot.gps {
                let gg = adap::PreferencesGroup::new();
                gg.set_title("Lieu");
                let r = row("Coordonnées GPS", &format!("{lat:.5}, {lon:.5}"));
                r.add_prefix(&gtk::Image::from_icon_name("mark-location-symbolic"));
                let map = gtk::Button::with_label("Voir sur la carte");
                map.set_valign(gtk::Align::Center);
                let uri = format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=15/{lat}/{lon}");
                let win = v.window.clone();
                map.connect_clicked(move |_| {
                    gtk::UriLauncher::new(&uri).launch(Some(&win), gio::Cancellable::NONE, |_| {});
                });
                r.add_suffix(&map);
                gg.add(&r);
                holder.append(&gg);
            }
        });
    }
}

/// Apparition en fondu d'une nouvelle image.
fn fade_in(widget: &impl IsA<gtk::Widget>) {
    let w = widget.clone().upcast::<gtk::Widget>();
    w.set_opacity(0.0);
    let target = adap::CallbackAnimationTarget::new(move |v| w.set_opacity(v));
    let anim = adap::TimedAnimation::new(widget, 0.0, 1.0, 220, target);
    anim.set_easing(adap::Easing::EaseOutCubic);
    anim.play();
}

fn strip_factory() -> gtk::SignalListItemFactory {
    let f = gtk::SignalListItemFactory::new();
    f.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let img = gtk::Image::new();
        img.set_pixel_size(60);
        img.add_css_class("strip-thumb");
        item.set_child(Some(&img));
    });
    f.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(path) = item.item().and_downcast::<gtk::StringObject>().map(|s| PathBuf::from(s.string().as_str())) else { return };
        let img = item.child().and_downcast::<gtk::Image>().unwrap();
        img.set_icon_name(Some("image-x-generic-symbolic"));
        img.set_tooltip_text(path.file_name().and_then(|n| n.to_str()));
        let key = path.to_string_lossy().to_string();
        unsafe { img.set_data("noos-path", key.clone()) };
        glib::spawn_future_local(async move {
            let Some(thumb) = thumbs::thumbnail(path).await else { return };
            if unsafe { img.data::<String>("noos-path").map(|p| p.as_ref() == &key) }.unwrap_or(false) {
                img.set_from_file(Some(&thumb));
            }
        });
    });
    f
}

const CSS: &str = r#"
window.photoview .viewer-area, window.photoview stack { background: none; }
window.photoview { background-color: #0b0b10; }
picture.ambient { filter: blur(42px) saturate(1.35) brightness(0.55); opacity: 0.9; }
picture.photo { box-shadow: 0 18px 60px rgba(0, 0, 0, 0.55); }
.side-nav { min-width: 46px; min-height: 46px; }
.bottom-osd { margin-bottom: 14px; }
.osd-bar, .strip {
  background-color: rgba(18, 18, 26, 0.72);
  border-radius: 99px;
  padding: 4px 8px;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.35), inset 0 0 0 1px rgba(255, 255, 255, 0.07);
  color: white;
}
.strip { border-radius: 14px; padding: 6px; }
.osd-bar > separator { margin: 6px 4px; background: rgba(255, 255, 255, 0.15); min-width: 1px; }
.osd-bar button { color: white; }
.zoom-label { min-width: 64px; font-feature-settings: "tnum"; }
.strip > row { padding: 3px; border-radius: 9px; background: none; }
.strip > row:selected { background-color: alpha(@accent_bg_color, 0.85); }
.strip > row:hover:not(:selected) { background-color: rgba(255, 255, 255, 0.12); }
image.strip-thumb { border-radius: 6px; }
.info-panel { margin: 60px 16px 18px 16px; }
.info-panel row.property { min-height: 44px; }
.keycap { font-family: monospace; padding: 2px 8px; border-radius: 6px; background-color: alpha(@window_fg_color, 0.1); }
"#;

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    });
    let ensure = |app: &adap::Application| -> Rc<Viewer> {
        if let Some(v) = viewer() {
            return v;
        }
        let v = Viewer::new(app);
        v.window.present();
        v
    };
    app.connect_activate(move |app| {
        let v = ensure(app);
        v.window.present();
        // Test visuel sans souris : NOOS_PHOTOVIEW_DEMO=<image>[;info][;zoom=2][;next][;slideshow]
        if let Ok(spec) = std::env::var("NOOS_PHOTOVIEW_DEMO") {
            demo(&v, &spec);
        }
    });
    app.connect_open(move |app, files, _| {
        let v = ensure(app);
        if let Some(p) = files.first().and_then(|f| f.path()) {
            v.open_path(&p);
        }
        v.window.present();
    });
    app.run()
}

fn demo(v: &Rc<Viewer>, spec: &str) {
    for (i, part) in spec.split(';').enumerate() {
        let v = v.clone();
        let part = part.to_string();
        glib::timeout_add_local_once(Duration::from_millis(400 + 900 * i as u64), move || match part.split_once('=') {
            Some(("zoom", z)) => v.set_zoom(z.parse().unwrap_or(2.0), None),
            _ if part == "info" => v.activate("info"),
            _ if part == "next" => v.step(1),
            _ if part == "hide" => v.set_chrome(false),
            _ if part == "slideshow" => v.toggle_slideshow(),
            _ if !part.is_empty() => v.open_path(Path::new(&part)),
            _ => {}
        });
    }
}

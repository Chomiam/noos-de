//! Personnalisation : arrière-plan, couleurs, thèmes, barre des tâches, polices.

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::config::{ColorScheme, TaskbarAlignment};
use noos_theme::{PALETTES, color::Rgb, palette};

use crate::ui::{App, background as in_background, card, combo, group, hero, link, page, spin, switch};

pub const ACCENTS: &[(&str, &str)] = &[
    ("Saphir", "#89b4fa"),
    ("Bleu Windows", "#0078d4"),
    ("Ciel", "#4cc2ff"),
    ("Turquoise", "#2ec2b3"),
    ("Vert", "#3a944a"),
    ("Lavande", "#cba6f7"),
    ("Violet", "#9141ac"),
    ("Rose", "#f38ba8"),
    ("Rouge", "#e62d42"),
    ("Orange", "#ed5b00"),
    ("Ambre", "#c88800"),
    ("Ardoise", "#6f8396"),
];

fn wallpaper_path(app: &App) -> Option<std::path::PathBuf> {
    let cfg = app.cfg();
    let custom = std::path::PathBuf::from(&cfg.appearance.wallpaper);
    (!cfg.appearance.wallpaper.is_empty() && custom.exists()).then_some(custom)
}

/// Format de l'écran (largeur / hauteur), pour des aperçus fidèles.
fn screen_ratio() -> f32 {
    gdk::Display::default()
        .and_then(|d| d.monitors().item(0).and_downcast::<gdk::Monitor>())
        .map(|m| m.geometry())
        .filter(|g| g.height() > 0)
        .map(|g| g.width() as f32 / g.height() as f32)
        .unwrap_or(16.0 / 9.0)
}

/// Miniature : l'image est recadrée au format de l'écran (centrée, comme le fond final) et
/// réduite à 384 px dans un thread ; la vignette prend toute la largeur de sa colonne et sa
/// hauteur suit (ni bandeau étiré, ni minuscule carré).
fn thumb(pic: &gtk::Picture) -> gtk::Picture {
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Cover);
    // Taille fixe au format de l'écran : la grille garde des vignettes fidèles.
    let w = 184;
    pic.set_size_request(w, (w as f32 / screen_ratio()).round() as i32);
    pic.set_halign(gtk::Align::Center);
    pic.set_valign(gtk::Align::Center);
    pic.clone()
}

fn load_thumb(pic: &gtk::Picture, path: std::path::PathBuf) {
    load_scaled(pic, path, 384, None);
}

/// Charge `path` recadré au format de l'écran et réduit à `width` px (dans un thread),
/// puis l'affiche dans `pic`. `done` est appelé une fois l'image affichée.
fn load_scaled(pic: &gtk::Picture, path: std::path::PathBuf, width: i32, done: Option<Box<dyn FnOnce()>>) {
    let ratio = screen_ratio() as f64;
    let pic = pic.clone();
    glib::spawn_future_local(async move {
        let data = in_background(move || -> Option<(Vec<u8>, i32, i32, usize, bool)> {
            use gtk::gdk_pixbuf::Pixbuf;
            let src = Pixbuf::from_file_at_scale(&path, width * 2, width * 2, true).ok()?;
            let (w, h) = (src.width() as f64, src.height() as f64);
            // Recadrage centré au format de l'écran.
            let (cw, ch) = if w / h > ratio { (h * ratio, h) } else { (w, w / ratio) };
            let cropped = src.new_subpixbuf(((w - cw) / 2.0) as i32, ((h - ch) / 2.0) as i32, cw as i32, ch as i32);
            let th = (width as f64 / ratio).round() as i32;
            let small = cropped.scale_simple(width, th, gtk::gdk_pixbuf::InterpType::Bilinear)?;
            Some((small.read_pixel_bytes().to_vec(), small.width(), small.height(), small.rowstride() as usize, small.has_alpha()))
        })
        .await;
        if let Some((bytes, w, h, stride, alpha)) = data {
            let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
            let tex = gdk::MemoryTexture::new(w, h, format, &glib::Bytes::from_owned(bytes), stride);
            pic.set_paintable(Some(&tex));
        }
        if let Some(done) = done {
            done();
        }
    });
}

/// Aperçu avant application : grande vue de l'image au format de l'écran (avec une barre des
/// tâches simulée), puis « Appliquer ». `fetch` obtient l'image en pleine résolution (cache).
fn preview_wallpaper(
    app: &Rc<App>,
    title: &str,
    details: &str,
    thumb: Option<std::path::PathBuf>,
    link: Option<String>,
    online: bool,
    fetch: Box<dyn FnOnce() -> Result<std::path::PathBuf, String> + Send + 'static>,
) {
    let ratio = screen_ratio();
    let dialog = adap::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(800);
    let view = adap::ToolbarView::new();
    view.add_top_bar(&adap::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_bottom(20);

    let pic = gtk::Picture::new();
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_size_request(740, (740.0 / ratio).round() as i32);
    pic.add_css_class("wallpaper-preview");
    pic.set_overflow(gtk::Overflow::Hidden);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&pic));
    overlay.set_halign(gtk::Align::Center);
    // Barre des tâches simulée, pour juger du rendu réel.
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("preview-taskbar");
    bar.set_valign(gtk::Align::End);
    overlay.add_overlay(&bar);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(32, 32);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    spinner.start();
    overlay.add_overlay(&spinner);
    body.append(&overlay);

    let info = gtk::Label::new(Some(details));
    info.add_css_class("dim-label");
    info.set_xalign(0.0);
    info.set_wrap(true);
    body.append(&info);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if let Some(url) = link {
        let source = gtk::LinkButton::with_label(&url, "Voir la source");
        actions.append(&source);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    let cancel = gtk::Button::with_label("Annuler");
    // Image en ligne : la garder seulement, ou la garder et l'appliquer.
    let download = gtk::Button::with_label("Télécharger");
    download.set_sensitive(false);
    download.set_visible(online);
    let apply = gtk::Button::with_label(if online { "Télécharger et appliquer" } else { "Appliquer" });
    apply.add_css_class("suggested-action");
    apply.set_sensitive(false);
    actions.append(&cancel);
    actions.append(&download);
    actions.append(&apply);
    body.append(&actions);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    if let Some(t) = thumb {
        load_scaled(&pic, t, 384, None);
    }
    let full: Rc<RefCell<Option<std::path::PathBuf>>> = Rc::new(RefCell::new(None));
    {
        let (pic, full, apply, download, spinner, info) =
            (pic.clone(), full.clone(), apply.clone(), download.clone(), spinner.clone(), info.clone());
        glib::spawn_future_local(async move {
            match in_background(fetch).await {
                Ok(path) => {
                    *full.borrow_mut() = Some(path.clone());
                    load_scaled(&pic, path, 1480, Some(Box::new(move || {
                        spinner.stop();
                        spinner.set_visible(false);
                        apply.set_sensitive(true);
                        download.set_sensitive(true);
                    })));
                }
                Err(e) => {
                    spinner.stop();
                    spinner.set_visible(false);
                    info.set_text(&format!("Téléchargement impossible : {e}"));
                }
            }
        });
    }
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let (app, d, full) = (app.clone(), dialog.clone(), full.clone());
        download.connect_clicked(move |_| {
            let Some(path) = full.borrow().clone() else { return };
            let kept = noos_core::paths::keep_wallpaper(&path);
            app.toast(&format!("Enregistré dans {}", kept.parent().map(|p| p.display().to_string()).unwrap_or_default()));
            d.close();
            let page = background(&app);
            app.nav.pop();
            app.nav.push(&page);
        });
    }
    {
        let (app, d) = (app.clone(), dialog.clone());
        apply.connect_clicked(move |_| {
            let Some(path) = full.borrow().clone() else { return };
            let kept = noos_core::paths::keep_wallpaper(&path).to_string_lossy().to_string();
            app.update(|c| {
                c.appearance.wallpaper = kept;
                c.appearance.bing_daily = false;
                c.appearance.slideshow.enabled = false;
            });
            app.toast("Fond d'écran appliqué");
            d.close();
            let page = background(&app);
            app.nav.pop();
            app.nav.push(&page);
        });
    }
    dialog.present(Some(&app.window));
}

fn wallpaper_preview(app: &App, w: i32, h: i32) -> gtk::Picture {
    let pic = match wallpaper_path(app) {
        Some(p) => gtk::Picture::for_filename(p),
        None => {
            let p = gtk::Picture::new();
            p.add_css_class("no-wallpaper");
            p
        }
    };
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_size_request(w, h);
    pic.set_can_shrink(true);
    pic.add_css_class("wallpaper-preview");
    pic.set_overflow(gtk::Overflow::Hidden);
    pic
}

/// Petite vignette de palette (fond de fenêtre + barre + accent).
fn palette_tile(app: &Rc<App>, idx: usize) -> gtk::Button {
    let pal = &PALETTES[idx];
    let dark = adap::StyleManager::default().is_dark();
    let v = if dark { pal.dark } else { pal.light };
    let b = gtk::Button::new();
    b.add_css_class("palette-tile");
    b.add_css_class(&format!("pal-{idx}"));
    if app.cfg().appearance.palette == pal.id {
        b.add_css_class("selected");
    }
    let css = gtk::CssProvider::new();
    let fg = Rgb::parse(v.window_bg).unwrap_or(Rgb::BLACK).readable_fg().hex();
    css.load_from_string(&format!(
        ".pal-{idx} {{ background-image: linear-gradient(180deg, {} 0%, {} 62%, {} 62%, {} 100%); color: {fg}; }}",
        v.headerbar_bg, v.window_bg, v.panel_bg, v.panel_bg
    ));
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
    inner.set_margin_top(10);
    inner.set_margin_bottom(8);
    inner.set_margin_start(10);
    inner.set_margin_end(10);
    let l = gtk::Label::new(Some(pal.label));
    l.add_css_class("caption-heading");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_max_width_chars(14);
    inner.append(&l);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.set_size_request(18, 6);
    dot.set_halign(gtk::Align::Start);
    dot.add_css_class(&format!("pal-dot-{idx}"));
    css.load_from_string(&format!(
        "{} .pal-dot-{idx} {{ background-color: {}; border-radius: 3px; }}",
        format!(
            ".pal-{idx} {{ background-image: linear-gradient(180deg, {} 0%, {} 62%, {} 62%, {} 100%); color: {fg}; }}",
            v.headerbar_bg, v.window_bg, v.panel_bg, v.panel_bg
        ),
        pal.accent
    ));
    inner.append(&dot);
    b.set_child(Some(&inner));
    let app = app.clone();
    b.connect_clicked(move |_| {
        app.update(|c| {
            c.appearance.palette = PALETTES[idx].id.to_string();
            c.appearance.accent = PALETTES[idx].accent.to_string();
        });
        let page = root(&app);
        app.nav.replace(&[page]);
    });
    b
}

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Personnalisation");
    let preview = wallpaper_preview(app, 260, 146);
    let cfg = app.cfg();
    let (h, _) = hero(
        &preview,
        "Sélectionnez un thème à appliquer",
        &format!(
            "Palette « {} » · mode {} · accent {}",
            palette::palette(&cfg.appearance.palette).label,
            match cfg.appearance.color_scheme {
                ColorScheme::Dark => "sombre",
                ColorScheme::Light => "clair",
                ColorScheme::System => "système",
            },
            cfg.appearance.accent
        ),
    );
    content.append(&h);
    let tiles = gtk::FlowBox::new();
    tiles.set_selection_mode(gtk::SelectionMode::None);
    tiles.set_max_children_per_line(5);
    tiles.set_column_spacing(10);
    tiles.set_row_spacing(10);
    for i in 0..PALETTES.len() {
        tiles.insert(&palette_tile(app, i), -1);
    }
    content.append(&tiles);

    let g = group("");
    g.add(&link(app, "preferences-desktop-wallpaper-symbolic", "Arrière-plan", "Image du bureau", background));
    g.add(&link(app, "applications-graphics-symbolic", "Couleurs", "Mode sombre ou clair, couleur d'accent, transparence", colors));
    g.add(&link(app, "preferences-desktop-theme-symbolic", "Thèmes", "Palette, coins, boutons de fenêtre, CSS personnalisé", themes));
    g.add(&link(app, "view-app-grid-symbolic", "Barre des tâches", "Alignement, hauteur, applications épinglées", taskbar));
    g.add(&link(app, "view-dual-symbolic", "Fenêtres", "Ancrage aux bords de l'écran, dispositions, animations", windows));
    g.add(&link(app, "changes-prevent-symbolic", "Écran de verrouillage", "Animations, écran de veille, verrouillage automatique", crate::lockscreen::root));
    g.add(&link(app, "system-lock-screen-symbolic", "Écran de connexion", "Fond, horloge, comptes et sessions de l'écran de connexion", crate::login::login_screen));
    g.add(&link(app, "preferences-desktop-font-symbolic", "Polices", "Police de l'interface", fonts));
    content.append(&g);
    p
}

// ───────────────────────────── Arrière-plan ─────────────────────────────

pub fn background(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Arrière-plan");
    let preview = wallpaper_preview(app, 420, 236);
    preview.set_halign(gtk::Align::Center);
    content.append(&preview);

    let g = group("Personnaliser votre arrière-plan");
    let current_text = wallpaper_path(app)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "Aucune image : dégradé aux couleurs du thème".into());
    let current = card("preferences-desktop-wallpaper-symbolic", "Image actuelle", &current_text);
    let pick = gtk::Button::with_label("Parcourir les photos");
    pick.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        pick.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_pixbuf_formats();
            filter.add_mime_type("image/svg+xml");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Choisir un fond d'écran").filters(&filters).build();
            let app2 = app.clone();
            dialog.open(Some(&app.window), gio::Cancellable::NONE, move |res| {
                if let Ok(Some(path)) = res.map(|f| f.path()) {
                    let s = path.to_string_lossy().to_string();
                    app2.update(|c| c.appearance.wallpaper = s);
                    let page = background(&app2);
                    app2.nav.pop();
                    app2.nav.push(&page);
                }
            });
        });
    }
    current.add_suffix(&pick);
    g.add(&current);
    let reset = card("edit-clear-symbolic", "Sans image", "Un simple dégradé aux couleurs du thème");
    let reset_b = gtk::Button::with_label("Retirer l'image");
    reset_b.set_valign(gtk::Align::Center);
    reset_b.set_sensitive(wallpaper_path(app).is_some());
    {
        let app = app.clone();
        reset_b.connect_clicked(move |_| {
            app.update(|c| {
                c.appearance.wallpaper.clear();
                c.appearance.bing_daily = false;
                c.appearance.slideshow.enabled = false;
            });
            let page = background(&app);
            app.nav.pop();
            app.nav.push(&page);
        });
    }
    reset.add_suffix(&reset_b);
    g.add(&reset);
    let show = app.cfg().appearance.slideshow.clone();
    if show.enabled && !show.images.is_empty() {
        let label = INTERVALS.iter().find(|(m, _)| *m == show.interval_minutes).map(|(_, l)| *l).unwrap_or("régulièrement");
        let running = card(
            "media-playlist-shuffle-symbolic",
            "Diaporama en cours",
            &format!("{} images · changement : {}", show.images.len(), label.to_lowercase()),
        );
        let stop = gtk::Button::with_label("Arrêter");
        stop.set_valign(gtk::Align::Center);
        let app2 = app.clone();
        stop.connect_clicked(move |_| {
            app2.update(|c| c.appearance.slideshow.enabled = false);
            let page = background(&app2);
            app2.nav.pop();
            app2.nav.push(&page);
        });
        running.add_suffix(&stop);
        g.add(&running);
    }
    content.append(&g);

    content.append(&bing_section(app));
    content.append(&wallhaven_section(app));

    content.append(&your_wallpapers(app));
    p
}

/// Durées proposées pour le diaporama (minutes, libellé).
const INTERVALS: &[(u32, &str)] = &[
    (1, "Toutes les minutes"),
    (5, "Toutes les 5 minutes"),
    (10, "Toutes les 10 minutes"),
    (15, "Toutes les 15 minutes"),
    (30, "Toutes les 30 minutes"),
    (60, "Toutes les heures"),
    (120, "Toutes les 2 heures"),
    (360, "Toutes les 6 heures"),
    (1440, "Chaque jour"),
];

/// Transitions (identifiant de la configuration, libellé).
const TRANSITIONS: &[(&str, &str)] = &[
    ("crossfade", "Fondu"),
    ("slide-left", "Glisser vers la gauche"),
    ("slide-up", "Glisser vers le haut"),
    ("over-up", "Recouvrir"),
    ("rotate", "Cube 3D"),
    ("random", "Aléatoire"),
    ("none", "Aucune"),
];

fn transition_type(id: &str) -> gtk::StackTransitionType {
    match id {
        "slide-left" => gtk::StackTransitionType::SlideLeft,
        "slide-up" => gtk::StackTransitionType::SlideUp,
        "over-up" => gtk::StackTransitionType::OverUp,
        "rotate" => gtk::StackTransitionType::RotateLeft,
        "none" => gtk::StackTransitionType::None,
        "random" => {
            let ids = ["crossfade", "slide-left", "slide-up", "over-up", "rotate"];
            transition_type(ids[glib::random_int_range(0, ids.len() as i32) as usize])
        }
        _ => gtk::StackTransitionType::Crossfade,
    }
}

/// Réglage du diaporama : durée, transition (avec aperçu animé), ordre aléatoire.
fn slideshow_dialog(app: &Rc<App>, images: Vec<std::path::PathBuf>) {
    if images.len() < 2 {
        return;
    }
    let ratio = screen_ratio();
    let current = app.cfg().appearance.slideshow.clone();
    let dialog = adap::Dialog::new();
    dialog.set_title("Diaporama");
    dialog.set_content_width(620);
    let view = adap::ToolbarView::new();
    view.add_top_bar(&adap::HeaderBar::new());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.set_margin_bottom(20);

    // Aperçu : les images sélectionnées défilent avec la transition choisie.
    let stack = gtk::Stack::new();
    stack.set_transition_duration(1200);
    stack.set_halign(gtk::Align::Center);
    let w = 560;
    let pics: Vec<gtk::Picture> = images
        .iter()
        .take(6)
        .enumerate()
        .map(|(i, path)| {
            let pic = gtk::Picture::new();
            pic.set_can_shrink(true);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(w, (w as f32 / ratio).round() as i32);
            pic.add_css_class("wallpaper-preview");
            pic.set_overflow(gtk::Overflow::Hidden);
            load_scaled(&pic, path.clone(), w, None);
            stack.add_named(&pic, Some(&i.to_string()));
            pic
        })
        .collect();
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&stack));
    overlay.set_halign(gtk::Align::Center);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bar.add_css_class("preview-taskbar");
    bar.set_valign(gtk::Align::End);
    overlay.add_overlay(&bar);
    body.append(&overlay);

    let group = adap::PreferencesGroup::new();
    group.set_description(Some(&format!("{} images sélectionnées", images.len())));
    let interval = adap::ComboRow::new();
    interval.set_title("Changer d'image");
    interval.set_model(Some(&gtk::StringList::new(&INTERVALS.iter().map(|(_, l)| *l).collect::<Vec<_>>())));
    interval.set_selected(INTERVALS.iter().position(|(m, _)| *m == current.interval_minutes).unwrap_or(4) as u32);
    group.add(&interval);
    let transition = adap::ComboRow::new();
    transition.set_title("Transition");
    transition.set_model(Some(&gtk::StringList::new(&TRANSITIONS.iter().map(|(_, l)| *l).collect::<Vec<_>>())));
    transition.set_selected(TRANSITIONS.iter().position(|(id, _)| *id == current.transition).unwrap_or(0) as u32);
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("flat");
    play.set_valign(gtk::Align::Center);
    play.set_tooltip_text(Some("Prévisualiser la transition"));
    transition.add_suffix(&play);
    group.add(&transition);
    let shuffle = adap::SwitchRow::new();
    shuffle.set_title("Ordre aléatoire");
    shuffle.set_active(current.shuffle);
    group.add(&shuffle);
    body.append(&group);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    let cancel = gtk::Button::with_label("Annuler");
    let start = gtk::Button::with_label("Lancer le diaporama");
    start.add_css_class("suggested-action");
    actions.append(&cancel);
    actions.append(&start);
    body.append(&actions);
    view.set_content(Some(&body));
    dialog.set_child(Some(&view));

    // Prévisualisation : image suivante avec la transition sélectionnée.
    let next: Rc<dyn Fn()> = {
        let (stack, transition) = (stack.clone(), transition.clone());
        let count = pics.len();
        let index = Rc::new(std::cell::Cell::new(0usize));
        Rc::new(move || {
            let id = TRANSITIONS.get(transition.selected() as usize).map(|t| t.0).unwrap_or("crossfade");
            stack.set_transition_type(transition_type(id));
            index.set((index.get() + 1) % count);
            stack.set_visible_child_name(&index.get().to_string());
        })
    };
    play.connect_clicked(move |_| next());
    {
        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
    }
    {
        let (app, d) = (app.clone(), dialog.clone());
        start.connect_clicked(move |_| {
            let minutes = INTERVALS.get(interval.selected() as usize).map(|i| i.0).unwrap_or(30);
            let id = TRANSITIONS.get(transition.selected() as usize).map(|t| t.0).unwrap_or("crossfade").to_string();
            let list: Vec<String> = images.iter().map(|p| p.to_string_lossy().to_string()).collect();
            let random = shuffle.is_active();
            app.update(|c| {
                c.appearance.slideshow = noos_core::config::Slideshow {
                    enabled: true,
                    images: list.clone(),
                    interval_minutes: minutes,
                    transition: id.clone(),
                    shuffle: random,
                };
                c.appearance.bing_daily = false;
                if let Some(first) = list.first() {
                    c.appearance.wallpaper = first.clone();
                }
            });
            app.toast(&format!("Diaporama lancé : {} images", list.len()));
            d.close();
            let page = background(&app);
            app.nav.pop();
            app.nav.push(&page);
        });
    }
    dialog.present(Some(&app.window));
}

/// « Vos fonds d'écran » (`~/Images/Wallpapers`) : aperçu au clic, et mode sélection pour
/// en mettre un ou plusieurs à la corbeille.
fn your_wallpapers(app: &Rc<App>) -> gtk::Widget {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let dir = noos_core::paths::wallpapers_dir();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| ["jpg", "jpeg", "png", "webp"].contains(&e.to_string_lossy().to_lowercase().as_str())))
                .collect()
        })
        .unwrap_or_default();
    // Les plus récentes d'abord.
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Vos fonds d'écran"));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    let where_ = gtk::Label::new(Some(&dir.to_string_lossy()));
    where_.add_css_class("dim-label");
    where_.add_css_class("caption");
    where_.set_xalign(0.0);
    let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
    titles.set_hexpand(true);
    titles.append(&title);
    titles.append(&where_);
    head.append(&titles);
    let select = gtk::ToggleButton::with_label("Sélectionner");
    select.add_css_class("flat");
    select.set_valign(gtk::Align::Center);
    select.set_sensitive(!files.is_empty());
    head.append(&select);
    v.append(&head);

    // Barre d'actions du mode sélection.
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.add_css_class("selection-bar");
    let count = gtk::Label::new(Some("Aucune image sélectionnée"));
    count.set_hexpand(true);
    count.set_xalign(0.0);
    let all = gtk::Button::with_label("Tout sélectionner");
    all.add_css_class("flat");
    let slideshow = gtk::Button::with_label("Diaporama");
    slideshow.add_css_class("suggested-action");
    slideshow.set_sensitive(false);
    slideshow.set_tooltip_text(Some("Sélectionnez au moins deux images"));
    let delete = gtk::Button::with_label("Mettre à la corbeille");
    delete.add_css_class("destructive-action");
    delete.set_sensitive(false);
    actions.append(&count);
    actions.append(&all);
    actions.append(&slideshow);
    actions.append(&delete);
    let revealer = gtk::Revealer::new();
    revealer.set_child(Some(&actions));
    v.append(&revealer);

    if files.is_empty() {
        let empty = gtk::Label::new(Some("Les fonds appliqués depuis Bing ou Wallhaven, et les images que vous placez dans ce dossier, apparaîtront ici."));
        empty.add_css_class("dim-label");
        empty.set_wrap(true);
        empty.set_xalign(0.0);
        v.append(&empty);
    }

    let gallery = gtk::FlowBox::new();
    gallery.set_selection_mode(gtk::SelectionMode::None);
    gallery.set_max_children_per_line(4);
    gallery.set_min_children_per_line(4);
    gallery.set_homogeneous(true);
    gallery.set_column_spacing(10);
    gallery.set_row_spacing(10);
    v.append(&gallery);

    let selected: Rc<RefCell<Vec<std::path::PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    let checks: Rc<RefCell<Vec<gtk::CheckButton>>> = Rc::new(RefCell::new(Vec::new()));
    let update_count = {
        let (selected, count, delete, slideshow) = (selected.clone(), count.clone(), delete.clone(), slideshow.clone());
        Rc::new(move || {
            let n = selected.borrow().len();
            slideshow.set_sensitive(n >= 2);
            count.set_text(&match n {
                0 => "Aucune image sélectionnée".to_string(),
                1 => "1 image sélectionnée".to_string(),
                n => format!("{n} images sélectionnées"),
            });
            delete.set_sensitive(n > 0);
        })
    };
    let current = app.cfg().appearance.wallpaper.clone();
    for path in files {
        let pic = gtk::Picture::new();
        pic.add_css_class("wallpaper-preview");
        pic.set_overflow(gtk::Overflow::Hidden);
        let b = gtk::Button::new();
        b.add_css_class("flat");
        b.set_child(Some(&thumb(&pic)));
        load_thumb(&pic, path.clone());
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        b.set_tooltip_text(Some(&name));
        if path.to_string_lossy() == current {
            b.add_css_class("current-wallpaper");
        }
        let check = gtk::CheckButton::new();
        check.add_css_class("selection-check");
        check.set_halign(gtk::Align::Start);
        check.set_valign(gtk::Align::Start);
        check.set_margin_start(10);
        check.set_margin_top(8);
        check.set_visible(false);
        checks.borrow_mut().push(check.clone());
        let tile = gtk::Overlay::new();
        tile.set_child(Some(&b));
        tile.add_overlay(&check);
        {
            let (selected, update, path) = (selected.clone(), update_count.clone(), path.clone());
            check.connect_toggled(move |c| {
                let mut sel = selected.borrow_mut();
                sel.retain(|p| p != &path);
                if c.is_active() {
                    sel.push(path.clone());
                }
                drop(sel);
                update();
            });
        }
        {
            let (app, select, check, path) = (app.clone(), select.clone(), check.clone(), path.clone());
            b.connect_clicked(move |_| {
                if select.is_active() {
                    check.set_active(!check.is_active());
                    return;
                }
                let size = gtk::gdk_pixbuf::Pixbuf::file_info(&path)
                    .map(|(_, w, h)| format!("{w} × {h} · "))
                    .unwrap_or_default();
                let p2 = path.clone();
                preview_wallpaper(
                    &app,
                    "Aperçu du fond d'écran",
                    &format!("{size}{}", path.display()),
                    None,
                    None,
                    false,
                    Box::new(move || Ok(p2)),
                );
            });
        }
        gallery.insert(&tile, -1);
    }

    {
        let (revealer, checks, selected, update) = (revealer.clone(), checks.clone(), selected.clone(), update_count.clone());
        select.connect_toggled(move |t| {
            let on = t.is_active();
            t.set_label(if on { "Terminé" } else { "Sélectionner" });
            revealer.set_reveal_child(on);
            for c in checks.borrow().iter() {
                c.set_visible(on);
                if !on {
                    c.set_active(false);
                }
            }
            if !on {
                selected.borrow_mut().clear();
            }
            update();
        });
    }
    {
        let checks = checks.clone();
        all.connect_clicked(move |_| {
            let every = checks.borrow().iter().all(|c| c.is_active());
            for c in checks.borrow().iter() {
                c.set_active(!every);
            }
        });
    }
    {
        let (app, selected) = (app.clone(), selected.clone());
        slideshow.connect_clicked(move |_| slideshow_dialog(&app, selected.borrow().clone()));
    }
    {
        let (app, selected) = (app.clone(), selected.clone());
        delete.connect_clicked(move |_| {
            let paths = selected.borrow().clone();
            let n = paths.len();
            let confirm = adap::AlertDialog::new(
                Some(&if n == 1 { "Mettre ce fond d'écran à la corbeille ?".to_string() } else { format!("Mettre ces {n} fonds d'écran à la corbeille ?") }),
                Some("Vous pourrez les récupérer depuis la corbeille de l'explorateur de fichiers."),
            );
            confirm.add_response("cancel", "Annuler");
            confirm.add_response("trash", "Mettre à la corbeille");
            confirm.set_response_appearance("trash", adap::ResponseAppearance::Destructive);
            confirm.set_close_response("cancel");
            let app2 = app.clone();
            confirm.choose(Some(&app.window), gio::Cancellable::NONE, move |r| {
                if r != "trash" {
                    return;
                }
                let current = app2.cfg().appearance.wallpaper.clone();
                let mut failed = 0;
                for p in &paths {
                    if gio::File::for_path(p).trash(gio::Cancellable::NONE).is_err() && std::fs::remove_file(p).is_err() {
                        failed += 1;
                    }
                }
                // Le fond actuel a été supprimé : retour au dégradé du thème.
                if paths.iter().any(|p| p.to_string_lossy() == current) {
                    app2.update(|c| c.appearance.wallpaper.clear());
                }
                app2.toast(&if failed > 0 {
                    format!("{failed} image(s) n'ont pas pu être supprimées")
                } else if paths.len() == 1 {
                    "Fond d'écran mis à la corbeille".to_string()
                } else {
                    format!("{} fonds d'écran mis à la corbeille", paths.len())
                });
                let page = background(&app2);
                app2.nav.pop();
                app2.nav.push(&page);
            });
        });
    }
    v.upcast()
}

/// Wallhaven : fonds au hasard adaptés à l'écran, ou recherche par mots-clés.
fn wallhaven_section(app: &Rc<App>) -> gtk::Widget {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let g = group("Fonds d'écran Wallhaven");
    g.set_description(Some("Des milliers de fonds d'écran communautaires, filtrés pour votre écran"));
    let shuffle = gtk::Button::from_icon_name("media-playlist-shuffle-symbolic");
    shuffle.add_css_class("flat");
    shuffle.set_tooltip_text(Some("D'autres au hasard"));
    g.set_header_suffix(Some(&shuffle));
    v.append(&g);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher : montagne, océan, ville la nuit, espace…"));
    v.append(&search);
    let status = gtk::Label::new(Some("Détection de l'écran…"));
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    status.set_wrap(true);
    v.append(&status);
    let gallery = gtk::FlowBox::new();
    gallery.set_selection_mode(gtk::SelectionMode::None);
    gallery.set_max_children_per_line(4);
    gallery.set_min_children_per_line(4);
    gallery.set_column_spacing(10);
    gallery.set_row_spacing(10);
    gallery.set_homogeneous(true);
    v.append(&gallery);

    // Taille de l'écran (le plus grand si plusieurs), orientation comprise.
    let screen: Rc<std::cell::Cell<(u32, u32)>> = Rc::new(std::cell::Cell::new((1920, 1080)));
    let load: Rc<dyn Fn()> = {
        let (app, gallery, status, search, screen) = (app.clone(), gallery.clone(), status.clone(), search.clone(), screen.clone());
        Rc::new(move || {
            gallery.remove_all();
            let (w, h) = screen.get();
            let query = search.text().trim().to_string();
            status.set_text(&format!("Recherche… (écran {w} × {h}, format {})", noos_core::wallhaven::ratio(w, h).replace('x', ":")));
            let (app, gallery, status) = (app.clone(), gallery.clone(), status.clone());
            glib::spawn_future_local(async move {
                let q = query.clone();
                let list = match in_background(move || noos_core::wallhaven::search(&q, w, h).map_err(|e| format!("{e:#}"))).await {
                    Ok(l) => l,
                    Err(e) => {
                        status.set_text(&e);
                        return;
                    }
                };
                let what = if query.is_empty() { "Sélection au hasard".to_string() } else { format!("« {query} »") };
                status.set_text(&if list.is_empty() {
                    format!("{what} : aucun résultat à au moins {w} × {h}")
                } else {
                    format!("{what} · au moins {w} × {h} · cliquez pour l'appliquer")
                });
                for wp in list.into_iter().take(24) {
                    let button = gtk::Button::new();
                    button.add_css_class("flat");
                    button.set_tooltip_text(Some(&format!("{} · {}", wp.resolution.replace('x', " × "), wp.page_url)));
                    let pic = gtk::Picture::new();
                    pic.set_can_shrink(true);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.add_css_class("wallpaper-preview");
                    pic.set_overflow(gtk::Overflow::Hidden);
                    button.set_child(Some(&thumb(&pic)));
                    gallery.insert(&button, -1);
                    {
                        let (wp, pic) = (wp.clone(), pic.clone());
                        glib::spawn_future_local(async move {
                            if let Ok(path) = in_background(move || wp.thumbnail().map_err(|e| e.to_string())).await {
                                load_thumb(&pic, path);
                            }
                        });
                    }
                    let app = app.clone();
                    button.connect_clicked(move |_| {
                        let thumb = noos_core::paths::cache_dir().join(format!("wallpapers/wallhaven/{}.jpg", wp.id));
                        let w2 = wp.clone();
                        preview_wallpaper(
                            &app,
                            "Aperçu du fond d'écran",
                            &format!("Wallhaven · {}", wp.resolution.replace('x', " × ")),
                            thumb.exists().then_some(thumb),
                            Some(wp.page_url.clone()),
                            true,
                            Box::new(move || w2.download_preview().map_err(|e| format!("{e:#}"))),
                        );
                    });
                }
            });
        })
    };
    {
        let load = load.clone();
        shuffle.connect_clicked(move |_| load());
    }
    {
        let (load, load2, search2) = (load.clone(), load.clone(), search.clone());
        search.connect_activate(move |_| load());
        // Échap dans le champ : retour à la sélection au hasard.
        search2.connect_stop_search(move |e| {
            e.set_text("");
            load2();
        });
    }
    {
        let (load, screen) = (load.clone(), screen.clone());
        glib::spawn_future_local(async move {
            if let Ok(noos_core::ipc::Event::Outputs { outputs }) = crate::ui::ipc(noos_core::ipc::Request::GetOutputs).await {
                if let Some(o) = outputs.iter().max_by_key(|o| o.width as i64 * o.height as i64) {
                    let (w, h) = (o.width.max(1) as u32, o.height.max(1) as u32);
                    let rotated = matches!(o.transform.as_str(), "90" | "270" | "flipped-90" | "flipped-270");
                    screen.set(if rotated { (h, w) } else { (w, h) });
                }
            }
            load();
        });
    }
    v.upcast()
}

/// Images du jour de Bing : galerie des 8 derniers jours + changement quotidien automatique.
fn bing_section(app: &Rc<App>) -> gtk::Widget {
    let cfg = app.cfg();
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);

    let g = group("Images du jour Bing");
    g.set_description(Some("Une nouvelle photo chaque jour, téléchargée en haute définition"));
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some("Actualiser"));
    g.set_header_suffix(Some(&refresh));
    let daily = switch(
        "preferences-desktop-wallpaper-symbolic",
        "Changer de fond d'écran chaque jour",
        "L'image du jour de Bing devient automatiquement votre fond d'écran",
        cfg.appearance.bing_daily,
    );
    {
        let app = app.clone();
        daily.connect_active_notify(move |r| {
            let on = r.is_active();
            app.update(|c| {
                c.appearance.bing_daily = on;
                if on {
                    c.appearance.slideshow.enabled = false;
                }
            });
            if on {
                // Tout de suite, sans attendre le lendemain.
                let app = app.clone();
                glib::spawn_future_local(async move {
                    match in_background(|| noos_core::bing::today().map_err(|e| format!("{e:#}"))).await {
                        Ok((path, img)) => {
                            let p = path.to_string_lossy().to_string();
                            app.update(|c| c.appearance.wallpaper = p);
                            app.toast(&format!("Fond d'écran : {}", img.title));
                        }
                        Err(e) => app.toast(&format!("Bing indisponible : {e}")),
                    }
                });
            }
        });
    }
    g.add(&daily);
    v.append(&g);

    let gallery = gtk::FlowBox::new();
    gallery.set_selection_mode(gtk::SelectionMode::None);
    gallery.set_max_children_per_line(4);
    gallery.set_min_children_per_line(4);
    gallery.set_column_spacing(10);
    gallery.set_row_spacing(10);
    gallery.set_homogeneous(true);
    let status = gtk::Label::new(Some("Chargement des images…"));
    status.add_css_class("dim-label");
    v.append(&status);
    v.append(&gallery);

    let load: Rc<dyn Fn()> = {
        let (app, gallery, status) = (app.clone(), gallery.clone(), status.clone());
        Rc::new(move || {
            gallery.remove_all();
            status.set_text("Chargement des images…");
            status.set_visible(true);
            let (app, gallery, status) = (app.clone(), gallery.clone(), status.clone());
            glib::spawn_future_local(async move {
                let images = match in_background(|| noos_core::bing::list(8).map_err(|e| format!("{e:#}"))).await {
                    Ok(i) => i,
                    Err(e) => {
                        status.set_text(&format!("Impossible de joindre Bing : {e}"));
                        return;
                    }
                };
                status.set_visible(false);
                for img in images {
                    let button = gtk::Button::new();
                    button.add_css_class("flat");
                    button.set_tooltip_text(Some(&format!("{}\n{}", img.title, img.copyright)));
                    let pic = gtk::Picture::new();
                    pic.set_can_shrink(true);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.add_css_class("wallpaper-preview");
                    pic.set_overflow(gtk::Overflow::Hidden);
                    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    col.append(&thumb(&pic));
                    let title = gtk::Label::new(Some(&img.title));
                    title.add_css_class("caption");
                    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    title.set_max_width_chars(20);
                    col.append(&title);
                    button.set_child(Some(&col));
                    gallery.insert(&button, -1);
                    {
                        let (img, pic) = (img.clone(), pic.clone());
                        glib::spawn_future_local(async move {
                            if let Ok(path) = in_background(move || img.thumbnail().map_err(|e| e.to_string())).await {
                                load_thumb(&pic, path);
                            }
                        });
                    }
                    let app = app.clone();
                    button.connect_clicked(move |_| {
                        let i2 = img.clone();
                        preview_wallpaper(
                            &app,
                            &img.title,
                            &format!("Bing · {}", img.copyright),
                            i2.thumbnail().ok(),
                            None,
                            true,
                            Box::new(move || i2.download_preview().map_err(|e| format!("{e:#}"))),
                        );
                    });
                }
            });
        })
    };
    {
        let load = load.clone();
        refresh.connect_clicked(move |_| load());
    }
    load();
    v.upcast()
}

// ───────────────────────────── Couleurs ─────────────────────────────

pub fn colors(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Couleurs");
    let cfg = app.cfg();
    let a = &cfg.appearance;

    let g = group("");
    let mode = combo(
        "weather-clear-night-symbolic",
        "Choisir votre mode",
        "Change les couleurs des fenêtres, de la barre des tâches et du menu Démarrer",
        &["Sombre", "Clair", "Suivre le système"].map(String::from),
        match a.color_scheme {
            ColorScheme::Dark => 0,
            ColorScheme::Light => 1,
            ColorScheme::System => 2,
        },
    );
    {
        let app = app.clone();
        mode.connect_selected_notify(move |r| {
            let s = match r.selected() {
                0 => ColorScheme::Dark,
                1 => ColorScheme::Light,
                _ => ColorScheme::System,
            };
            app.update(|c| c.appearance.color_scheme = s);
        });
    }
    g.add(&mode);
    let glass = switch(
        "view-paged-symbolic",
        "Effets de transparence",
        "Barre des tâches et menus translucides, fond d'écran flouté dessous (Mica)",
        a.panel_opacity < 0.99,
    );
    {
        let app = app.clone();
        glass.connect_active_notify(move |r| {
            let v = if r.is_active() { 0.72 } else { 1.0 };
            app.update(|c| c.appearance.panel_opacity = v);
        });
    }
    g.add(&glass);
    let opacity = spin("view-paged-symbolic", "Opacité de la barre des tâches", "En pourcentage", (30.0, 100.0, 1.0), (a.panel_opacity * 100.0).round());
    {
        let app = app.clone();
        opacity.connect_value_notify(move |r| {
            let v = r.value() / 100.0;
            app.update(|c| c.appearance.panel_opacity = v);
        });
    }
    g.add(&opacity);
    content.append(&g);

    let accent = group("Couleur d'accentuation");
    let custom = card("applications-graphics-symbolic", "Couleur personnalisée", "");
    let picker = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::builder().with_alpha(false).title("Couleur d'accent").build()));
    picker.set_rgba(&gdk::RGBA::parse(&a.accent).unwrap_or(gdk::RGBA::BLUE));
    picker.set_valign(gtk::Align::Center);
    custom.add_suffix(&picker);
    {
        let app = app.clone();
        picker.connect_rgba_notify(move |b| {
            let c = b.rgba();
            let hex = Rgb { r: c.red() as f64, g: c.green() as f64, b: c.blue() as f64 }.hex();
            app.update(|cfg| cfg.appearance.accent = hex);
        });
    }
    accent.add(&custom);
    let swatches = gtk::FlowBox::new();
    swatches.set_selection_mode(gtk::SelectionMode::None);
    swatches.set_max_children_per_line(12);
    swatches.set_margin_top(10);
    swatches.set_margin_bottom(10);
    swatches.set_margin_start(10);
    swatches.set_margin_end(10);
    let css = gtk::CssProvider::new();
    let mut rules = String::new();
    for (i, (name, hex)) in ACCENTS.iter().enumerate() {
        rules.push_str(&format!(".swatch-{i} {{ background: {hex}; }}\n"));
        let b = gtk::Button::new();
        b.add_css_class("swatch");
        b.add_css_class(&format!("swatch-{i}"));
        b.set_tooltip_text(Some(name));
        let (picker, hex) = (picker.clone(), hex.to_string());
        b.connect_clicked(move |_| picker.set_rgba(&gdk::RGBA::parse(&hex).unwrap()));
        swatches.insert(&b, -1);
    }
    css.load_from_string(&rules);
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&swatches));
    accent.add(&row);
    content.append(&accent);
    p
}

// ───────────────────────────── Thèmes ─────────────────────────────

pub fn themes(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Thèmes");
    let cfg = app.cfg();
    let a = &cfg.appearance;

    let g = group("Thème actuel");
    let labels: Vec<String> = PALETTES.iter().map(|p| p.label.to_string()).collect();
    let pal = combo(
        "preferences-desktop-theme-symbolic",
        "Palette",
        "Le thème est un vrai thème libAdapta : toutes les applis libAdapta de la session le suivent",
        &labels,
        PALETTES.iter().position(|p| p.id == a.palette).unwrap_or(0),
    );
    {
        let app = app.clone();
        pal.connect_selected_notify(move |r| {
            let p = &PALETTES[r.selected() as usize];
            app.update(|c| {
                c.appearance.palette = p.id.to_string();
                c.appearance.accent = p.accent.to_string();
            });
        });
    }
    g.add(&pal);
    let radius = spin("view-restore-symbolic", "Arrondi des coins", "Fenêtres, menus et cartes (px)", (0.0, 24.0, 1.0), a.corner_radius as f64);
    {
        let app = app.clone();
        radius.connect_value_notify(move |r| {
            let v = r.value() as u32;
            app.update(|c| c.appearance.corner_radius = v);
        });
    }
    g.add(&radius);
    content.append(&g);

    let w = group("Fenêtres");
    let styles = [
        ("mac", "macOS (pastilles colorées)"),
        ("windows", "Windows 11 (rectangulaires)"),
        ("theme", "Selon le thème GTK"),
    ];
    let buttons = combo(
        "window-close-symbolic",
        "Style des boutons de fenêtre",
        "Fermer, réduire, agrandir",
        &styles.iter().map(|s| s.1.to_string()).collect::<Vec<_>>(),
        styles.iter().position(|s| s.0 == a.button_style).unwrap_or(0),
    );
    {
        let app = app.clone();
        buttons.connect_selected_notify(move |r| {
            let v = styles[(r.selected() as usize).min(styles.len() - 1)].0.to_string();
            app.update(|c| c.appearance.button_style = v);
        });
    }
    w.add(&buttons);
    let layouts = [("icon:minimize,maximize,close", "À droite (Windows)"), ("close,maximize,minimize:icon", "À gauche (macOS)"), ("icon:close", "Fermer uniquement")];
    let lay = combo(
        "view-dual-symbolic",
        "Position des boutons",
        "",
        &layouts.iter().map(|l| l.1.to_string()).collect::<Vec<_>>(),
        layouts.iter().position(|l| l.0 == a.button_layout).unwrap_or(0),
    );
    {
        let app = app.clone();
        lay.connect_selected_notify(move |r| {
            let v = layouts[r.selected() as usize].0.to_string();
            app.update(|c| c.appearance.button_layout = v);
        });
    }
    w.add(&lay);
    content.append(&w);

    let adv = group("Avancé");
    let global = switch(
        "applications-system-symbolic",
        "Appliquer aussi aux applis GTK3 et GTK4",
        "Écrit les couleurs dans ~/.config/gtk-3.0 et gtk-4.0/gtk.css (également lus par vos autres sessions)",
        a.apply_to_all_apps,
    );
    {
        let app = app.clone();
        global.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.appearance.apply_to_all_apps = v);
        });
    }
    adv.add(&global);
    let css_row = adap::ExpanderRow::new();
    css_row.set_title("CSS personnalisé");
    css_row.set_subtitle("Ajouté à la fin du thème (sélecteurs GTK4 / libAdapta)");
    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&a.custom_css);
    let text = gtk::TextView::with_buffer(&buffer);
    text.set_monospace(true);
    text.set_top_margin(8);
    text.set_bottom_margin(8);
    text.set_left_margin(8);
    let apply = gtk::Button::with_label("Appliquer le CSS");
    apply.add_css_class("suggested-action");
    apply.set_halign(gtk::Align::End);
    apply.set_margin_top(6);
    apply.set_margin_bottom(6);
    apply.set_margin_end(6);
    {
        let (app, buffer) = (app.clone(), buffer.clone());
        apply.connect_clicked(move |_| {
            let s = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string();
            app.update(|c| c.appearance.custom_css = s);
            app.toast("CSS appliqué");
        });
    }
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bx.append(&gtk::ScrolledWindow::builder().child(&text).min_content_height(160).build());
    bx.append(&apply);
    css_row.add_row(&bx);
    adv.add(&css_row);
    let folder = card("folder-open-symbolic", "Ouvrir le dossier du thème", &noos_theme::theme_dir().to_string_lossy());
    folder.set_activatable(true);
    folder.connect_activated(|_| {
        let uri = gio::File::for_path(noos_theme::theme_dir()).uri();
        let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
    });
    adv.add(&folder);
    content.append(&adv);
    p
}

// ───────────────────────────── Barre des tâches ─────────────────────────────

pub fn taskbar(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Barre des tâches");
    let cfg = app.cfg();
    let t = &cfg.taskbar;

    let g = group("Comportements de la barre des tâches");
    let align = combo(
        "format-justify-center-symbolic",
        "Alignement de la barre des tâches",
        "",
        &["Centre", "Gauche"].map(String::from),
        if t.alignment == TaskbarAlignment::Left { 1 } else { 0 },
    );
    {
        let app = app.clone();
        align.connect_selected_notify(move |r| {
            let v = if r.selected() == 1 { TaskbarAlignment::Left } else { TaskbarAlignment::Center };
            app.update(|c| c.taskbar.alignment = v);
        });
    }
    g.add(&align);
    let sizes = [(40u32, "Petite (40 px)"), (48, "Normale (48 px)"), (56, "Grande (56 px)"), (64, "Très grande (64 px)")];
    let mut labels: Vec<String> = sizes.iter().map(|s| s.1.to_string()).collect();
    let mut idx = sizes.iter().position(|s| s.0 == t.height);
    if idx.is_none() {
        labels.push(format!("Personnalisée ({} px)", t.height));
        idx = Some(labels.len() - 1);
    }
    let size = combo("view-app-grid-symbolic", "Taille de la barre des tâches", "", &labels, idx.unwrap());
    {
        let app = app.clone();
        size.connect_selected_notify(move |r| {
            if let Some((h, _)) = sizes.get(r.selected() as usize) {
                let h = *h;
                app.update(|c| c.taskbar.height = h);
            }
        });
    }
    g.add(&size);
    let custom = spin("zoom-in-symbolic", "Hauteur personnalisée", "En pixels (28 à 96)", (28.0, 96.0, 2.0), t.height as f64);
    {
        let app = app.clone();
        custom.connect_value_notify(move |r| {
            let v = r.value() as u32;
            app.update(|c| c.taskbar.height = v);
        });
    }
    g.add(&custom);
    let search = switch("system-search-symbolic", "Rechercher", "Afficher la zone de recherche", t.show_search);
    {
        let app = app.clone();
        search.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.taskbar.show_search = v);
        });
    }
    g.add(&search);
    let vitals = switch("utilities-system-monitor-symbolic", "Ressources", "Widget à gauche de la barre : processeur, carte graphique, mémoire, disque, réseau", t.show_vitals);
    {
        let app = app.clone();
        vitals.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.taskbar.show_vitals = v);
        });
    }
    g.add(&vitals);
    content.append(&g);

    let multi = group("Plusieurs écrans");
    let all = switch("video-display-symbolic", "Barre des tâches sur tous les écrans", "Sinon, seulement sur l'écran principal", t.all_outputs);
    {
        let app = app.clone();
        all.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.taskbar.all_outputs = v);
        });
    }
    multi.add(&all);
    let own = switch("view-grid-symbolic", "Chaque barre montre les fenêtres de son écran", "Sinon, toutes les barres montrent toutes les fenêtres", t.windows_on_own_output);
    {
        let app = app.clone();
        own.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.taskbar.windows_on_own_output = v);
        });
    }
    multi.add(&own);
    content.append(&multi);

    let tray = group("Zone de notification");
    let expanded = switch(
        "view-app-grid-symbolic",
        "Afficher toutes les icônes",
        "Icônes des applications actives directement dans la barre, au lieu du menu « ^ »",
        t.tray_expanded,
    );
    {
        let app = app.clone();
        expanded.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.taskbar.tray_expanded = v);
        });
    }
    tray.add(&expanded);
    content.append(&tray);

    let pinned = group("Applications épinglées");
    pinned.set_description(Some("Identifiants .desktop (ex. firefox, org.gnome.Nautilus)"));
    let list_box = gtk::ListBox::new();
    list_box.add_css_class("boxed-list");
    list_box.set_selection_mode(gtk::SelectionMode::None);
    let refill: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let (app, list_box, refill2) = (app.clone(), list_box.clone(), refill.clone());
        *refill.borrow_mut() = Some(Box::new(move || {
            list_box.remove_all();
            for id in app.cfg().taskbar.pinned {
                let row = card("application-x-executable-symbolic", &id, "");
                let rm = gtk::Button::from_icon_name("user-trash-symbolic");
                rm.add_css_class("flat");
                rm.set_valign(gtk::Align::Center);
                rm.set_tooltip_text(Some("Désépingler"));
                let (app, refill3, id2) = (app.clone(), refill2.clone(), id.clone());
                rm.connect_clicked(move |_| {
                    app.update(|c| c.taskbar.pinned.retain(|p| p != &id2));
                    let r = refill3.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(f) = r.borrow().as_ref() {
                            f()
                        }
                    });
                });
                row.add_suffix(&rm);
                list_box.append(&row);
            }
            let add = adap::EntryRow::new();
            add.set_title("Épingler une application…");
            add.set_show_apply_button(true);
            let (app, refill3) = (app.clone(), refill2.clone());
            add.connect_apply(move |e| {
                let id = e.text().trim().trim_end_matches(".desktop").to_string();
                if !id.is_empty() {
                    app.update(|c| c.taskbar.pinned.push(id));
                    let r = refill3.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(f) = r.borrow().as_ref() {
                            f()
                        }
                    });
                }
            });
            list_box.append(&add);
        }));
    }
    if let Some(f) = refill.borrow().as_ref() {
        f();
    }
    pinned.add(&list_box);
    content.append(&pinned);
    p
}

// ───────────────────────────── Fenêtres ─────────────────────────────

/// Illustration : fond d'écran, une fenêtre ancrée à gauche et l'aperçu translucide à droite.
fn tiling_illustration(app: &Rc<App>) -> gtk::Overlay {
    let (w, h) = (260, 146);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&wallpaper_preview(app, w, h)));
    let (ar, ag, ab) = Rgb::parse(&app.cfg().appearance.accent).map(|c| (c.r, c.g, c.b)).unwrap_or((0.54, 0.71, 0.98));
    let area = gtk::DrawingArea::new();
    area.set_content_width(w);
    area.set_content_height(h);
    area.set_can_target(false);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let rounded = |x: f64, y: f64, rw: f64, rh: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + rw - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(x + rw - r, y + rh - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(x + r, y + rh - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
            cr.close_path();
        };
        let bar = 16.0;
        let usable = h - bar;
        let m = 5.0;
        // Fenêtre ancrée à gauche.
        rounded(m, m, w / 2.0 - 1.5 * m, usable - 2.0 * m, 5.0);
        cr.set_source_rgba(0.13, 0.13, 0.16, 0.92);
        let _ = cr.fill();
        rounded(m, m, w / 2.0 - 1.5 * m, 12.0, 5.0);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
        let _ = cr.fill();
        for (i, c) in [(0.0, (1.0, 0.37, 0.34)), (1.0, (0.16, 0.78, 0.25)), (2.0, (1.0, 0.74, 0.18))] {
            cr.set_source_rgb(c.0, c.1, c.2);
            cr.arc(w / 2.0 - m - 8.0 - i * 8.0, m + 6.0, 2.5, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
        }
        for i in 0..4 {
            rounded(m + 10.0, m + 22.0 + i as f64 * 12.0, (w / 2.0 - 40.0) * (1.0 - i as f64 * 0.15), 5.0, 2.5);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
            let _ = cr.fill();
        }
        // Aperçu de l'emplacement visé, à droite.
        let (x, y, rw, rh) = (w / 2.0 + 0.5 * m, m, w / 2.0 - 1.5 * m, usable - 2.0 * m);
        rounded(x, y, rw, rh, 6.0);
        cr.set_source_rgba(ar, ag, ab, 0.25);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(ar, ag, ab, 0.9);
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        // Barre des tâches.
        cr.rectangle(0.0, h - bar, w, bar);
        cr.set_source_rgba(0.08, 0.08, 0.1, 0.85);
        let _ = cr.fill();
    });
    overlay.add_overlay(&area);
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.add_css_class("tiling-illustration");
    overlay.set_valign(gtk::Align::Center);
    overlay
}

pub fn windows(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Fenêtres");
    let t = app.cfg().tiling;
    let (h, _) = hero(
        &tiling_illustration(app),
        "Ancrer les fenêtres",
        "Faites glisser une fenêtre contre un bord ou un coin de l'écran pour la disposer côte à côte avec les autres.",
    );
    content.append(&h);

    let main = group("Ancrage");
    let enabled = switch(
        "view-dual-symbolic",
        "Ancrer les fenêtres",
        "Glisser vers les bords de l'écran et Super + ← / → (Super + ↑ agrandit toujours)",
        t.enabled,
    );
    main.add(&enabled);
    content.append(&main);

    let options = group("Options d'ancrage");
    let edges = switch(
        "view-grid-symbolic",
        "Bords et coins interactifs",
        "Bord gauche ou droit : moitié de l'écran · coin : quart · bord haut : plein écran · Ctrl : tiers",
        t.edges,
    );
    let bar = switch(
        "view-app-grid-symbolic",
        "Barre de dispositions",
        "Proposer les dispositions en haut de l'écran pendant le déplacement d'une fenêtre",
        t.layout_bar,
    );
    let preview = switch(
        "view-reveal-symbolic",
        "Aperçu de l'emplacement",
        "Rectangle translucide à l'endroit où la fenêtre va se placer",
        t.preview,
    );
    for row in [&edges, &bar, &preview] {
        options.add(row);
        enabled.bind_property("active", row, "sensitive").sync_create().build();
    }
    content.append(&options);

    let anim = group("Animations");
    let animations = switch(
        "media-playback-start-symbolic",
        "Animer les fenêtres",
        "Transition fluide lors de l'ancrage, de l'agrandissement et de la restauration",
        t.animations,
    );
    anim.add(&animations);
    content.append(&anim);

    let keys = group("Raccourcis clavier");
    for (combo_keys, what) in [
        ("Super + ← / →", "Ancrer à gauche ou à droite, puis 2/3 et 1/3 de l'écran"),
        ("Super + ↑", "Agrandir, ou quart du haut depuis une moitié"),
        ("Super + ↓", "Restaurer, quart du bas, ou réduire"),
    ] {
        let row = card("input-keyboard-symbolic", what, "");
        let k = gtk::Label::new(Some(combo_keys));
        k.add_css_class("dim-label");
        k.add_css_class("monospace");
        row.add_suffix(&k);
        keys.add(&row);
    }
    content.append(&keys);

    type Setter = fn(&mut noos_core::config::Tiling, bool);
    let rows: [(&adap::SwitchRow, Setter); 5] = [
        (&enabled, |t, v| t.enabled = v),
        (&edges, |t, v| t.edges = v),
        (&bar, |t, v| t.layout_bar = v),
        (&preview, |t, v| t.preview = v),
        (&animations, |t, v| t.animations = v),
    ];
    for (row, set) in rows {
        let app = app.clone();
        row.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| set(&mut c.tiling, v));
        });
    }
    p
}

// ───────────────────────────── Polices ─────────────────────────────

pub fn fonts(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Polices");
    let cfg = app.cfg();
    let g = group("");
    let row = card("preferences-desktop-font-symbolic", "Police de l'interface", if cfg.appearance.font.is_empty() { "Police par défaut" } else { &cfg.appearance.font });
    let btn = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    btn.set_valign(gtk::Align::Center);
    if !cfg.appearance.font.is_empty() {
        btn.set_font_desc(&gtk::pango::FontDescription::from_string(&cfg.appearance.font));
    }
    {
        let (app, row) = (app.clone(), row.clone());
        btn.connect_font_desc_notify(move |b| {
            if let Some(desc) = b.font_desc() {
                let s = desc.to_string();
                row.set_subtitle(&s);
                app.update(|c| c.appearance.font = s);
            }
        });
    }
    row.add_suffix(&btn);
    g.add(&row);
    let reset = card("edit-undo-symbolic", "Police par défaut", "Revenir à la police du thème");
    let rb = gtk::Button::with_label("Rétablir");
    rb.set_valign(gtk::Align::Center);
    {
        let (app, row) = (app.clone(), row.clone());
        rb.connect_clicked(move |_| {
            row.set_subtitle("Police par défaut");
            app.update(|c| c.appearance.font.clear());
        });
    }
    reset.add_suffix(&rb);
    g.add(&reset);
    content.append(&g);
    let sample = gtk::Label::new(Some("Portez ce vieux whisky au juge blond qui fume. 0123456789"));
    sample.add_css_class("title-3");
    sample.set_wrap(true);
    sample.set_xalign(0.0);
    content.append(&sample);
    content.append(&ms_fonts(app));
    p
}

const MSFONTS: &str = "/usr/libexec/noos-msfonts";

/// Polices Microsoft d'origine : téléchargées à la demande (licence Microsoft à accepter).
fn ms_fonts(app: &Rc<App>) -> adap::PreferencesGroup {
    let g = group("Polices Microsoft");
    let families = pangocairo_fontmap().list_families().len();
    g.set_description(Some(&format!(
        "{families} familles de polices installées, dont des équivalentes aux polices Microsoft (Liberation pour Arial / Times New Roman / Courier New, Carlito pour Calibri, Caladea pour Cambria) : les documents gardent leur mise en page."
    )));
    let row = card("font-x-generic-symbolic", "Polices Microsoft d'origine", "Arial, Times New Roman, Courier New, Verdana, Georgia, Trebuchet MS, Comic Sans MS, Impact, Webdings");
    let btn = gtk::Button::new();
    btn.set_valign(gtk::Align::Center);
    row.add_suffix(&btn);
    g.add(&row);
    let state = Rc::new(std::cell::Cell::new(false));
    let refresh: Rc<dyn Fn()> = {
        let (btn, row, state) = (btn.clone(), row.clone(), state.clone());
        Rc::new(move || {
            let installed = std::process::Command::new(MSFONTS).arg("status").output().is_ok_and(|o| o.stdout.starts_with(b"installed"));
            btn.set_label(if installed { "Supprimer" } else { "Installer…" });
            btn.set_css_classes(if installed { &["destructive-action"] } else { &["suggested-action"] });
            row.set_subtitle(if installed {
                "Installées : Arial, Times New Roman, Courier New, Verdana, Georgia, Trebuchet MS, Comic Sans MS, Impact, Webdings"
            } else {
                "Arial, Times New Roman, Courier New, Verdana, Georgia, Trebuchet MS, Comic Sans MS, Impact, Webdings"
            });
            state.set(installed);
        })
    };
    refresh();
    {
        let (app, refresh) = (app.clone(), refresh.clone());
        btn.connect_clicked(move |b| {
            let installed = state.get();
            let run = {
                let (app, refresh, b) = (app.clone(), refresh.clone(), b.clone());
                move |action: &'static str| {
                    b.set_sensitive(false);
                    b.set_label(if action == "install" { "Téléchargement…" } else { "Suppression…" });
                    let task = crate::ui::background(move || {
                        std::process::Command::new("pkexec").args([MSFONTS, action]).output().map_err(|e| e.to_string()).and_then(|o| {
                            if o.status.success() { Ok(String::from_utf8_lossy(&o.stdout).trim().lines().last().unwrap_or("").to_string()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
                        })
                    });
                    let (app, refresh, b) = (app.clone(), refresh.clone(), b.clone());
                    glib::spawn_future_local(async move {
                        b.set_sensitive(true);
                        match task.await {
                            Ok(msg) => app.toast(&msg),
                            Err(e) if e.is_empty() => app.toast("Opération annulée"),
                            Err(e) => app.error("Polices Microsoft", &e),
                        }
                        refresh();
                    });
                }
            };
            if installed {
                run("remove");
                return;
            }
            let d = adap::AlertDialog::new(
                Some("Installer les polices Microsoft ?"),
                Some("Microsoft les distribue gratuitement (« Core fonts for the Web ») mais interdit de les inclure dans un système : elles sont téléchargées depuis leur archive officielle (environ 4 Mo).\n\nEn continuant, vous acceptez la <a href=\"https://corefonts.sourceforge.net/eula.htm\">licence d'utilisation de Microsoft</a> pour ces polices."),
            );
            d.set_body_use_markup(true);
            d.add_responses(&[("cancel", "Annuler"), ("ok", "Accepter et installer")]);
            d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
            d.set_default_response(Some("ok"));
            d.set_close_response("cancel");
            let run = Rc::new(run);
            d.connect_response(None, move |_, r| {
                if r == "ok" {
                    run("install");
                }
            });
            d.present(Some(&app.window));
        });
    }
    g
}

fn pangocairo_fontmap() -> gtk::pango::FontMap {
    gtk::Label::new(None).pango_context().font_map().expect("fontmap")
}

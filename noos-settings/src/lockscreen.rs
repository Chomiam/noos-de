//! Personnalisation › Écran de verrouillage : aperçu animé ; inactivité (verrouillage ou
//! simple écran de veille, assombrissement progressif, extinction de l'écran) ; animations
//! NOOS intégrées ; bibliothèque locale ; catalogue NOOS (hébergé dans le dépôt GitHub, sans
//! compte) ; import d'animations (fichier ou code collé, format Shadertoy ou GLSL Sandbox).

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gio, glib};
use noos_core::{
    config::IdleMode,
    ipc::{IpcClient, Request},
};
use noos_shader::{builtin, catalog, store, view::ShaderView};

use crate::ui::{App, background, card, combo, group, page, switch};

/// Taille des aperçus et vignettes.
const TILE_W: i32 = 220;
const TILE_H: i32 = 124;

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Écran de verrouillage");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    fill(app, &holder);
    p
}

fn reload(app: &Rc<App>, holder: &gtk::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    fill(app, holder);
}

fn apply_animation(app: &Rc<App>, holder: &gtk::Box, spec: &str, name: &str) {
    let spec = spec.to_string();
    app.update(|c| c.lock.animation = spec);
    app.toast(&format!("« {name} » sera affiché à l'écran de verrouillage"));
    reload(app, holder);
}

/// Petit aperçu animé (léger : peu d'images par seconde, basse résolution).
fn mini_view(src: Option<noos_shader::Source>) -> ShaderView {
    let v = ShaderView::new();
    v.set_fps(10);
    v.set_render_scale(0.35);
    v.set_source(src);
    v
}

fn minutes_label(m: u32) -> String {
    match m {
        0 => "Jamais".into(),
        60 => "1 heure".into(),
        m => format!("{m} min"),
    }
}

fn fill(app: &Rc<App>, holder: &gtk::Box) {
    let cfg = app.cfg();
    let current = noos_shader::resolve(&cfg.lock.animation);

    // ── Héros : aperçu animé ──
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    hero.add_css_class("win11-hero");
    let preview = gtk::Overlay::new();
    preview.add_css_class("ls-preview");
    preview.set_size_request(360, 210);
    preview.set_overflow(gtk::Overflow::Hidden);
    match &current {
        Some(src) => {
            let v = ShaderView::new();
            v.set_fps(30);
            v.set_render_scale(0.6);
            v.set_source(Some(src.clone()));
            preview.set_child(Some(v.widget()));
        }
        None => {
            let pic = gtk::Picture::new();
            pic.set_content_fit(gtk::ContentFit::Cover);
            if !cfg.appearance.wallpaper.is_empty() {
                pic.set_filename(Some(&cfg.appearance.wallpaper));
            }
            preview.set_child(Some(&pic));
        }
    }
    // Assombrissement de l'animation, tel qu'il sera appliqué.
    let dim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dim.add_css_class("ls-dim");
    dim.set_opacity(cfg.lock.animation_dim as f64);
    dim.set_can_target(false);
    preview.add_overlay(&dim);
    let clock = gtk::Label::new(Some(&glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()));
    clock.add_css_class("ls-clock");
    clock.set_valign(gtk::Align::Start);
    clock.set_margin_top(26);
    clock.set_can_target(false);
    clock.set_visible(cfg.lock.show_clock);
    preview.add_overlay(&clock);
    hero.append(&preview);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    let t = gtk::Label::new(Some(&current.as_ref().map(|s| s.name.clone()).unwrap_or_else(|| "Fond d'écran".into())));
    t.add_css_class("title-2");
    t.set_xalign(0.0);
    v.append(&t);
    let sub = gtk::Label::new(Some(&match &current {
        Some(s) if s.author == "NOOS" => "Animation NOOS".to_string(),
        Some(s) if s.author.is_empty() => "Animation importée".to_string(),
        Some(s) => format!("par {}", s.author),
        None => "Image fixe, sans animation".to_string(),
    }));
    sub.add_css_class("dim-label");
    sub.set_xalign(0.0);
    v.append(&sub);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_margin_top(10);
    let full = gtk::Button::with_label("Aperçu plein écran");
    full.add_css_class("pill");
    {
        let spec = cfg.lock.animation.clone();
        full.connect_clicked(move |_| {
            let _ = std::process::Command::new("noos-lock").args(["--preview", &spec]).spawn();
        });
    }
    let lock_now = gtk::Button::with_label("Verrouiller maintenant");
    lock_now.add_css_class("pill");
    lock_now.add_css_class("suggested-action");
    {
        let app = app.clone();
        lock_now.connect_clicked(move |_| {
            let task = background(|| IpcClient::request(&Request::Lock).map_err(|e| e.to_string()));
            let app = app.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = task.await {
                    app.error("Verrouillage", &e);
                }
            });
        });
    }
    actions.append(&full);
    actions.append(&lock_now);
    v.append(&actions);
    hero.append(&v);
    holder.append(&hero);

    // ── Inactivité ──
    let g = group("Quand je m'absente");
    g.set_description(Some("Super+L verrouille à tout moment, quel que soit ce réglage."));
    const MINUTES: [u32; 8] = [0, 1, 2, 5, 10, 15, 30, 60];
    let labels: Vec<String> = MINUTES.iter().map(|m| if *m == 0 { "Jamais".into() } else { format!("Après {}", minutes_label(*m)) }).collect();
    let idle = combo("preferences-system-time-symbolic", "Afficher l'écran de veille", "Délai sans clavier ni souris", &labels, MINUTES.iter().position(|m| *m == cfg.lock.idle_minutes).unwrap_or(3));
    {
        let app = app.clone();
        idle.connect_selected_notify(move |r| {
            let m = MINUTES[r.selected() as usize];
            app.update(|c| c.lock.idle_minutes = m);
        });
    }
    g.add(&idle);
    let mode = combo(
        "system-lock-screen-symbolic",
        "Ensuite",
        "",
        &["Verrouiller la session (mot de passe)".into(), "Écran de veille seulement, sans verrouiller".into()],
        if cfg.lock.mode == IdleMode::Screensaver { 1 } else { 0 },
    );
    let set_mode_subtitle = |r: &adap::ComboRow| {
        r.set_subtitle(if r.selected() == 1 {
            "Une touche ou un mouvement de souris ramène directement au bureau"
        } else {
            "Le mot de passe de votre compte est demandé pour revenir"
        });
    };
    set_mode_subtitle(&mode);
    {
        let app = app.clone();
        mode.connect_selected_notify(move |r| {
            set_mode_subtitle(r);
            let m = if r.selected() == 1 { IdleMode::Screensaver } else { IdleMode::Lock };
            app.update(|c| c.lock.mode = m);
        });
    }
    g.add(&mode);
    const DIM: [u32; 5] = [0, 5, 10, 20, 30];
    let dim_row = combo(
        "display-brightness-symbolic",
        "Assombrir l'écran juste avant",
        "L'écran s'assombrit progressivement ; bougez la souris pour l'annuler",
        &DIM.map(|s| if s == 0 { "Non".to_string() } else { format!("Pendant {s} s") }),
        DIM.iter().position(|s| *s == cfg.lock.dim_seconds).unwrap_or(2),
    );
    {
        let app = app.clone();
        dim_row.connect_selected_notify(move |r| {
            let s = DIM[r.selected() as usize];
            app.update(|c| c.lock.dim_seconds = s);
        });
    }
    g.add(&dim_row);
    const OFF: [u32; 9] = [0, 1, 2, 3, 5, 10, 15, 30, 60];
    let off = combo(
        "video-display-symbolic",
        "Éteindre l'écran après",
        "Inactivité totale, écran de veille compris (même réglage que Système › Alimentation)",
        &OFF.map(|m| if m == 0 { "Jamais".to_string() } else { minutes_label(m) }),
        OFF.iter().position(|m| *m == cfg.power.screen_off_minutes).unwrap_or(5),
    );
    {
        let app = app.clone();
        off.connect_selected_notify(move |r| {
            let m = OFF[r.selected() as usize];
            app.update(|c| c.power.screen_off_minutes = m);
        });
    }
    g.add(&off);
    let suspend = switch("system-suspend-symbolic", "Verrouiller avant la mise en veille", "Au réveil, l'écran de verrouillage est déjà là", cfg.lock.before_suspend);
    {
        let app = app.clone();
        suspend.connect_active_notify(move |s| {
            let v = s.is_active();
            app.update(|c| c.lock.before_suspend = v);
        });
    }
    g.add(&suspend);
    let clock_sw = switch("preferences-system-time-symbolic", "Afficher l'horloge", "", cfg.lock.show_clock);
    {
        let app = app.clone();
        clock_sw.connect_active_notify(move |s| {
            let v = s.is_active();
            app.update(|c| c.lock.show_clock = v);
        });
    }
    g.add(&clock_sw);
    holder.append(&g);

    // ── Performances et assombrissement de l'animation ──
    let g = group("Animation");
    let dim_anim = card("weather-clear-night-symbolic", "Assombrir l'animation", "Moins éblouissant dans le noir");
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 80.0, 5.0);
    scale.set_value((cfg.lock.animation_dim * 100.0).round() as f64);
    scale.set_size_request(200, -1);
    scale.set_valign(gtk::Align::Center);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.set_format_value_func(|_, v| format!("{v:.0} %"));
    {
        let (app, dim) = (app.clone(), dim.clone());
        scale.connect_value_changed(move |s| {
            let v = (s.value() / 100.0) as f32;
            dim.set_opacity(v as f64);
            app.update(|c| c.lock.animation_dim = v);
        });
    }
    dim_anim.add_suffix(&scale);
    g.add(&dim_anim);
    const FPS: [u32; 4] = [15, 24, 30, 60];
    let fps = combo("speedometer-symbolic", "Images par seconde", "Moins d'images = moins de consommation", &FPS.map(|f| format!("{f} images/s")), FPS.iter().position(|f| *f == cfg.lock.fps).unwrap_or(2));
    {
        let app = app.clone();
        fps.connect_selected_notify(move |r| {
            let f = FPS[r.selected() as usize];
            app.update(|c| c.lock.fps = f);
        });
    }
    g.add(&fps);
    const SCALES: [f32; 3] = [1.0, 0.75, 0.5];
    let quality = combo(
        "video-display-symbolic",
        "Qualité de rendu",
        "Certaines animations sont exigeantes pour la carte graphique",
        &["Maximale (pleine résolution)".into(), "Équilibrée (75 %)".into(), "Économe (50 %)".into()],
        SCALES.iter().position(|s| (s - cfg.lock.render_scale).abs() < 0.01).unwrap_or(1),
    );
    {
        let app = app.clone();
        quality.connect_selected_notify(move |r| {
            let s = SCALES[r.selected() as usize];
            app.update(|c| c.lock.render_scale = s);
        });
    }
    g.add(&quality);
    let still = card("preferences-desktop-wallpaper-symbolic", "Fond d'écran fixe", "Pas d'animation : l'image du bureau");
    if cfg.lock.animation.is_empty() {
        still.add_suffix(&check());
    } else {
        let b = gtk::Button::with_label("Utiliser");
        b.set_valign(gtk::Align::Center);
        let (app, holder) = (app.clone(), holder.clone());
        b.connect_clicked(move |_| apply_animation(&app, &holder, "", "Fond d'écran"));
        still.add_suffix(&b);
    }
    g.add(&still);
    holder.append(&g);

    // ── Animations NOOS ──
    holder.append(&section("Animations NOOS", "Intégrées à NOOS, toujours disponibles"));
    let flow = tiles_box();
    for b in builtin::BUILTINS {
        let spec = format!("builtin:{}", b.id);
        let v = mini_view(noos_shader::resolve(&spec));
        let t = tile(v.widget(), b.name, b.description, cfg.lock.animation == spec);
        let (app, holder, name) = (app.clone(), holder.clone(), b.name);
        t.connect_clicked(move |_| apply_animation(&app, &holder, &spec, name));
        flow.append(&t);
    }
    holder.append(&flow);

    // ── Bibliothèque ──
    let saved = store::list();
    holder.append(&section(
        "Ma bibliothèque",
        &if saved.is_empty() {
            "Les animations ajoutées depuis le catalogue ou importées apparaîtront ici".to_string()
        } else {
            format!("{} animation{}", saved.len(), plural(saved.len()))
        },
    ));
    if !saved.is_empty() {
        let flow = tiles_box();
        for s in saved {
            let spec = format!("library:{}", s.id);
            let v = mini_view(noos_shader::resolve(&spec));
            let desc = match s.origin.as_str() {
                "import" => "Importée".to_string(),
                "shadertoy" => format!("Shadertoy · par {}", s.author),
                _ => s.description.clone(),
            };
            let active = cfg.lock.animation == spec || cfg.lock.animation == format!("shadertoy:{}", s.id);
            let t = tile(v.widget(), &s.name, &desc, active);
            let (app, holder) = (app.clone(), holder.clone());
            t.connect_clicked(move |_| saved_dialog(&app, &holder, &s));
            flow.append(&t);
        }
        holder.append(&flow);
    }

    // ── Catalogue NOOS ──
    holder.append(&catalog_section(app, holder));

    // ── Import ──
    holder.append(&import_group(app, holder));
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

fn check() -> gtk::Image {
    let i = gtk::Image::from_icon_name("object-select-symbolic");
    i.add_css_class("accent");
    i
}

fn section(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("dim-label");
    s.set_xalign(0.0);
    s.set_wrap(true);
    b.append(&t);
    b.append(&s);
    b.set_margin_bottom(-12);
    b
}

fn tiles_box() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_min_children_per_line(2);
    f.set_max_children_per_line(4);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f
}

/// Tuile : aperçu animé, nom, description, coche si actif.
fn tile(visual: &impl IsA<gtk::Widget>, name: &str, desc: &str, active: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("ls-tile");
    if active {
        b.add_css_class("active");
    }
    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let frame = gtk::Overlay::new();
    frame.set_size_request(TILE_W, TILE_H);
    frame.set_overflow(gtk::Overflow::Hidden);
    frame.add_css_class("ls-thumb");
    frame.set_child(Some(visual));
    if active {
        let c = check();
        c.add_css_class("ls-badge");
        c.set_halign(gtk::Align::End);
        c.set_valign(gtk::Align::Start);
        c.set_margin_top(6);
        c.set_margin_end(6);
        frame.add_overlay(&c);
    }
    v.append(&frame);
    let n = gtk::Label::new(Some(name));
    n.add_css_class("ls-name");
    n.set_xalign(0.0);
    n.set_ellipsize(gtk::pango::EllipsizeMode::End);
    v.append(&n);
    let d = gtk::Label::new(Some(desc));
    d.add_css_class("dim-label");
    d.add_css_class("caption");
    d.set_xalign(0.0);
    d.set_ellipsize(gtk::pango::EllipsizeMode::End);
    v.append(&d);
    b.set_child(Some(&v));
    b
}

type Action = (&'static str, &'static str, Rc<dyn Fn()>);

/// Grand aperçu animé dans une fenêtre, avec actions.
fn preview_dialog(app: &Rc<App>, title: &str, subtitle: &str, src: noos_shader::Source, actions: &[Action]) {
    let d = adap::Dialog::new();
    d.set_title(title);
    d.set_content_width(760);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.set_margin_start(18);
    b.set_margin_end(18);
    b.set_margin_bottom(18);
    let view = ShaderView::new();
    view.set_fps(30);
    view.set_render_scale(0.75);
    let frame = gtk::Overlay::new();
    frame.set_size_request(-1, 400);
    frame.add_css_class("ls-thumb");
    frame.set_overflow(gtk::Overflow::Hidden);
    let err = gtk::Label::new(None);
    err.add_css_class("ls-error");
    err.set_wrap(true);
    err.set_visible(false);
    err.set_valign(gtk::Align::Center);
    {
        let err = err.clone();
        view.connect_error(move |e| {
            err.set_text(&format!("Cette animation ne fonctionne pas sur cette carte graphique :\n{}", e.lines().take(3).collect::<Vec<_>>().join("\n")));
            err.set_visible(true);
        });
    }
    view.set_source(Some(src));
    frame.set_child(Some(view.widget()));
    frame.add_overlay(&err);
    b.append(&frame);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("dim-label");
    s.set_wrap(true);
    s.set_xalign(0.0);
    b.append(&s);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    for (label, class, f) in actions {
        let btn = gtk::Button::with_label(label);
        btn.add_css_class("pill");
        if !class.is_empty() {
            btn.add_css_class(class);
        }
        let (f, d2) = (f.clone(), d.clone());
        btn.connect_clicked(move |_| {
            d2.close();
            f();
        });
        row.append(&btn);
    }
    b.append(&row);
    tv.set_content(Some(&b));
    d.set_child(Some(&tv));
    d.present(Some(&app.window));
}

fn saved_dialog(app: &Rc<App>, holder: &gtk::Box, s: &store::Saved) {
    let spec = format!("library:{}", s.id);
    let Some(src) = noos_shader::resolve(&spec) else { return };
    let apply: Rc<dyn Fn()> = {
        let (app, holder, spec, name) = (app.clone(), holder.clone(), spec.clone(), s.name.clone());
        Rc::new(move || apply_animation(&app, &holder, &spec, &name))
    };
    let delete: Rc<dyn Fn()> = {
        let (app, holder, id, name) = (app.clone(), holder.clone(), s.id.clone(), s.name.clone());
        Rc::new(move || {
            let (app2, holder2, id2, name2) = (app.clone(), holder.clone(), id.clone(), name.clone());
            app.confirm(&format!("Retirer « {name} » ?"), "L'animation sera supprimée de votre bibliothèque.", "Retirer", true, move || {
                if let Err(e) = store::delete(&id2) {
                    return app2.error("Suppression", &format!("{e:#}"));
                }
                let cur = app2.cfg().lock.animation;
                if cur == format!("library:{id2}") || cur == format!("shadertoy:{id2}") {
                    app2.update(|c| c.lock.animation = "builtin:aurora".into());
                }
                app2.toast(&format!("« {name2} » retirée"));
                reload(&app2, &holder2);
            });
        })
    };
    let mut info = vec![];
    if !s.author.is_empty() {
        info.push(format!("par {}", s.author));
    }
    info.push(match s.origin.as_str() {
        "catalogue" => "Catalogue NOOS".to_string(),
        "import" => "Importée".to_string(),
        "shadertoy" => "Shadertoy".to_string(),
        o => o.to_string(),
    });
    if !s.license.is_empty() {
        info.push(format!("licence {}", s.license));
    }
    let mut text = info.join(" · ");
    if !s.description.is_empty() {
        text = format!("{text}\n{}", s.description);
    }
    let mut actions: Vec<Action> = vec![("Retirer", "destructive-action", delete)];
    if !s.url.is_empty() {
        let url = s.url.clone();
        actions.push((
            "Page d'origine",
            "",
            Rc::new(move || {
                let _ = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>);
            }),
        ));
    }
    actions.push(("Appliquer", "suggested-action", apply));
    preview_dialog(app, &s.name, &text, src, &actions);
}

// ───────────────────────────── Catalogue NOOS ─────────────────────────────

fn catalog_section(app: &Rc<App>, holder: &gtk::Box) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
    v.append(&section(
        "Catalogue NOOS",
        "Animations créées pour NOOS, dont des clins d'œil aux écrans de veille de Windows 95 / 98. Sans compte, sous licence libre.",
    ));
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let spinner = gtk::Spinner::new();
    spinner.start();
    let label = gtk::Label::new(Some("Chargement du catalogue…"));
    label.add_css_class("dim-label");
    status.append(&spinner);
    status.append(&label);
    v.append(&status);
    let flow = tiles_box();
    v.append(&flow);

    let task = background(|| catalog::fetch().map_err(|e| format!("{e:#}")));
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let cat = match task.await {
            Ok(c) => c,
            Err(e) => {
                spinner.stop();
                spinner.set_visible(false);
                label.set_text(&format!("Catalogue indisponible : {e}"));
                return;
            }
        };
        status.set_visible(false);
        let current = app.cfg().lock.animation;
        for e in cat.shaders {
            let src = noos_shader::Source { name: e.name.clone(), author: e.author.clone(), image: e.code.clone(), common: String::new() };
            let view = mini_view(Some(src.clone()));
            let have = catalog::installed(&e.id);
            let desc = if have {
                "Dans ma bibliothèque".to_string()
            } else if !e.homage.is_empty() {
                format!("Hommage · {}", e.homage.trim_start_matches("Écran de veille "))
            } else {
                e.description.clone()
            };
            let t = tile(view.widget(), &e.name, &desc, current == catalog::spec(&e.id));
            let (app, holder) = (app.clone(), holder.clone());
            t.connect_clicked(move |_| catalog_dialog(&app, &holder, &e, src.clone()));
            flow.append(&t);
        }
    });
    v
}

fn catalog_dialog(app: &Rc<App>, holder: &gtk::Box, e: &catalog::Entry, src: noos_shader::Source) {
    let add = |apply: bool| -> Rc<dyn Fn()> {
        let (app, holder, e) = (app.clone(), holder.clone(), e.clone());
        Rc::new(move || match catalog::install(&e) {
            Ok(s) if apply => apply_animation(&app, &holder, &format!("library:{}", s.id), &s.name),
            Ok(s) => {
                app.toast(&format!("« {} » ajoutée à votre bibliothèque", s.name));
                reload(&app, &holder);
            }
            Err(err) => app.error("Ajout", &format!("{err:#}")),
        })
    };
    let mut text = e.description.clone();
    if !e.homage.is_empty() {
        text = format!("{text}\nClin d'œil : {}", e.homage);
    }
    text = format!("{text}\n{} · licence {}", if e.author.is_empty() { "NOOS" } else { &e.author }, e.license);
    let actions: Vec<Action> = if catalog::installed(&e.id) {
        vec![("Appliquer", "suggested-action", add(true))]
    } else {
        vec![("Ajouter à ma bibliothèque", "", add(false)), ("Ajouter et appliquer", "suggested-action", add(true))]
    };
    preview_dialog(app, &e.name, &text, src, &actions);
}

// ───────────────────────────── Import ─────────────────────────────

fn import_group(app: &Rc<App>, holder: &gtk::Box) -> adap::PreferencesGroup {
    let g = group("Importer une animation");
    g.set_description(Some(
        "Code GLSL d'une seule passe, sans textures ni son : format Shadertoy (fonction mainImage) ou GLSL Sandbox (main + gl_FragColor, converti automatiquement).",
    ));
    let file = card("document-open-symbolic", "Depuis un fichier", "Fichier .frag, .glsl ou .txt");
    file.set_activatable(true);
    file.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let (app, holder) = (app.clone(), holder.clone());
        file.connect_activated(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Animations GLSL"));
            for p in ["*.frag", "*.glsl", "*.fs", "*.txt"] {
                filter.add_pattern(p);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Importer une animation").filters(&filters).build();
            let (app2, holder2) = (app.clone(), holder.clone());
            dialog.open(Some(&app.window), gio::Cancellable::NONE, move |res| {
                let Ok(f) = res else { return };
                let Some(path) = f.path() else { return };
                let code = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => return app2.error("Import", &e.to_string()),
                };
                let name = path.file_stem().map(|s| s.to_string_lossy().replace(['-', '_'], " ")).unwrap_or_default();
                import_and_preview(&app2, &holder2, &name, &code, "");
            });
        });
    }
    g.add(&file);
    let paste = card("edit-paste-symbolic", "Coller du code", "Depuis un site de partage d'animations ou votre éditeur");
    paste.set_activatable(true);
    paste.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let (app, holder) = (app.clone(), holder.clone());
        paste.connect_activated(move |_| paste_dialog(&app, &holder));
    }
    g.add(&paste);
    g
}

fn import_and_preview(app: &Rc<App>, holder: &gtk::Box, name: &str, code: &str, url: &str) {
    match store::import(name, code, url) {
        Ok(s) => {
            app.toast(&format!("« {} » importée", s.name));
            reload(app, holder);
            saved_dialog(app, holder, &s);
        }
        Err(e) => app.error("Import", &format!("{e:#}")),
    }
}

fn paste_dialog(app: &Rc<App>, holder: &gtk::Box) {
    let d = adap::Dialog::new();
    d.set_title("Coller une animation");
    d.set_content_width(640);
    d.set_content_height(560);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.set_margin_start(18);
    b.set_margin_end(18);
    b.set_margin_bottom(18);
    let fields = adap::PreferencesGroup::new();
    let name = adap::EntryRow::new();
    name.set_title("Nom");
    let url = adap::EntryRow::new();
    url.set_title("Page d'origine (facultatif)");
    fields.add(&name);
    fields.add(&url);
    b.append(&fields);
    let text = gtk::TextView::new();
    text.set_monospace(true);
    text.set_wrap_mode(gtk::WrapMode::None);
    text.set_top_margin(8);
    text.set_left_margin(8);
    let sc = gtk::ScrolledWindow::builder().child(&text).vexpand(true).build();
    sc.add_css_class("card");
    b.append(&sc);
    let go = gtk::Button::with_label("Importer");
    go.add_css_class("pill");
    go.add_css_class("suggested-action");
    go.set_halign(gtk::Align::End);
    b.append(&go);
    {
        let (app, holder, d2) = (app.clone(), holder.clone(), d.clone());
        go.connect_clicked(move |_| {
            let buf = text.buffer();
            let code = buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string();
            d2.close();
            import_and_preview(&app, &holder, &name.text(), &code, url.text().trim());
        });
    }
    tv.set_content(Some(&b));
    d.set_child(Some(&tv));
    d.present(Some(&app.window));
}

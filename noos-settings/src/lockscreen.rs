//! Personnalisation › Écran de verrouillage : aperçu animé, verrouillage automatique,
//! animations NOOS intégrées, bibliothèque des animations Shadertoy téléchargées (appliquer,
//! supprimer) et parcours de l'API Shadertoy (recherche, tri, vignettes, téléchargement).

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};
use noos_core::ipc::{IpcClient, Request};
use noos_shader::{api, builtin, store, view::ShaderView};

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
    let clock = gtk::Label::new(Some(&glib::DateTime::now_local().ok().and_then(|d| d.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default()));
    clock.add_css_class("ls-clock");
    clock.set_valign(gtk::Align::Start);
    clock.set_margin_top(26);
    clock.set_can_target(false);
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
        Some(s) => format!("Shadertoy · par {}", s.author),
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

    // ── Verrouillage ──
    let g = group("Verrouillage");
    g.set_description(Some("Super+L verrouille à tout moment. L'écran de verrouillage demande le mot de passe de votre compte."));
    const MINUTES: [u32; 8] = [0, 1, 2, 5, 10, 15, 30, 60];
    let labels: Vec<String> = MINUTES.iter().map(|m| if *m == 0 { "Jamais".into() } else { format!("Après {m} min d'inactivité") }).collect();
    let idle = combo("preferences-system-time-symbolic", "Verrouiller automatiquement", "", &labels, MINUTES.iter().position(|m| *m == cfg.lock.idle_minutes).unwrap_or(3));
    {
        let app = app.clone();
        idle.connect_selected_notify(move |r| {
            let m = MINUTES[r.selected() as usize];
            app.update(|c| c.lock.idle_minutes = m);
        });
    }
    g.add(&idle);
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

    // ── Performances ──
    let g = group("Animation");
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
        "Les animations Shadertoy peuvent être exigeantes pour la carte graphique",
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
    holder.append(&section("Animations NOOS", "Créées pour NOOS, disponibles sans compte"));
    let flow = tiles_box();
    for b in builtin::BUILTINS {
        let spec = format!("builtin:{}", b.id);
        let v = ShaderView::new();
        v.set_fps(12);
        v.set_render_scale(0.5);
        v.set_source(noos_shader::resolve(&spec));
        let t = tile(v.widget(), b.name, b.description, cfg.lock.animation == spec);
        let (app, holder, name) = (app.clone(), holder.clone(), b.name);
        t.connect_clicked(move |_| apply_animation(&app, &holder, &spec, name));
        flow.append(&t);
    }
    holder.append(&flow);

    // ── Bibliothèque Shadertoy ──
    let saved = store::list();
    holder.append(&section(
        "Ma bibliothèque Shadertoy",
        &if saved.is_empty() { "Les animations téléchargées depuis Shadertoy apparaîtront ici".to_string() } else { format!("{} animation{} téléchargée{}", saved.len(), plural(saved.len()), plural(saved.len())) },
    ));
    if !saved.is_empty() {
        let flow = tiles_box();
        for s in saved {
            let spec = format!("shadertoy:{}", s.id);
            let pic = gtk::Picture::for_filename(store::thumb_path(&s.id));
            pic.set_content_fit(gtk::ContentFit::Cover);
            let t = tile(&pic, &s.name, &format!("par {}", s.author), cfg.lock.animation == spec);
            let (app, holder) = (app.clone(), holder.clone());
            t.connect_clicked(move |_| saved_dialog(&app, &holder, &s));
            flow.append(&t);
        }
        holder.append(&flow);
    }

    // ── Parcourir Shadertoy ──
    holder.append(&browser(app, holder));
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

/// Tuile : aperçu (animation ou vignette), nom, description, coche si actif.
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

/// Grand aperçu animé dans une fenêtre, avec actions.
fn preview_dialog(app: &Rc<App>, title: &str, subtitle: &str, src: noos_shader::Source, actions: &[(&str, &str, Rc<dyn Fn()>)]) {
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
    let spec = format!("shadertoy:{}", s.id);
    let Some(src) = noos_shader::resolve(&spec) else { return };
    let apply: Rc<dyn Fn()> = {
        let (app, holder, spec, name) = (app.clone(), holder.clone(), spec.clone(), s.name.clone());
        Rc::new(move || apply_animation(&app, &holder, &spec, &name))
    };
    let delete: Rc<dyn Fn()> = {
        let (app, holder, id, name, spec) = (app.clone(), holder.clone(), s.id.clone(), s.name.clone(), spec.clone());
        Rc::new(move || {
            if let Err(e) = store::delete(&id) {
                return app.error("Suppression", &format!("{e:#}"));
            }
            if app.cfg().lock.animation == spec {
                app.update(|c| c.lock.animation = "builtin:aurora".into());
            }
            app.toast(&format!("« {name} » supprimée"));
            reload(&app, &holder);
        })
    };
    let open: Rc<dyn Fn()> = {
        let url = s.url();
        Rc::new(move || {
            let _ = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>);
        })
    };
    preview_dialog(
        app,
        &s.name,
        &format!("par {} · {} vues · {} j'aime\n{}\nLicence : celle choisie par l'auteur sur Shadertoy (par défaut CC BY-NC-SA 3.0).", s.author, s.views, s.likes, s.description.lines().next().unwrap_or("")),
        src,
        &[("Supprimer", "destructive-action", delete), ("Voir sur Shadertoy", "", open), ("Appliquer", "suggested-action", apply)],
    );
}

// ───────────────────────────── API Shadertoy ─────────────────────────────

fn thumb_cache(id: &str) -> std::path::PathBuf {
    glib::user_cache_dir().join("noos").join("shadertoy").join(format!("{id}.jpg"))
}

fn browser(app: &Rc<App>, holder: &gtk::Box) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
    v.append(&section("Parcourir Shadertoy", "Des milliers d'animations GLSL partagées par la communauté (shadertoy.com)"));
    let Some(key) = api::key() else {
        let g = group("");
        g.set_description(Some(
            "Shadertoy demande une clé personnelle et gratuite : connectez-vous sur shadertoy.com, ouvrez « Your Apps », créez une application, puis collez sa clé ici. Elle reste sur ce PC.",
        ));
        let link = card("web-browser-symbolic", "Créer ma clé sur shadertoy.com", "shadertoy.com/myapps");
        link.set_activatable(true);
        link.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
        link.connect_activated(|_| {
            let _ = gio::AppInfo::launch_default_for_uri("https://www.shadertoy.com/myapps", None::<&gio::AppLaunchContext>);
        });
        g.add(&link);
        let entry = adap::EntryRow::new();
        entry.set_title("Clé de l'API Shadertoy");
        entry.set_show_apply_button(true);
        {
            let (app, holder) = (app.clone(), holder.clone());
            entry.connect_apply(move |e| {
                match api::set_key(&e.text()) {
                    Ok(()) => app.toast("Clé enregistrée"),
                    Err(err) => app.error("Clé Shadertoy", &format!("{err:#}")),
                }
                reload(&app, &holder);
            });
        }
        g.add(&entry);
        v.append(&g);
        return v;
    };

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher (ex. « ocean », « fractal », « clouds »)"));
    search.set_hexpand(true);
    let sorts = ["Populaires", "Les plus aimés", "Récents", "Tendances"];
    let sort = gtk::DropDown::from_strings(&sorts);
    let forget = gtk::Button::from_icon_name("dialog-password-symbolic");
    forget.set_tooltip_text(Some("Changer de clé Shadertoy"));
    forget.add_css_class("flat");
    {
        let (app, holder) = (app.clone(), holder.clone());
        forget.connect_clicked(move |_| {
            let _ = api::set_key("");
            reload(&app, &holder);
        });
    }
    bar.append(&search);
    bar.append(&sort);
    bar.append(&forget);
    v.append(&bar);
    let status = gtk::Label::new(Some("Recherchez ou choisissez un tri pour afficher des animations."));
    status.add_css_class("dim-label");
    status.set_xalign(0.0);
    v.append(&status);
    let flow = tiles_box();
    v.append(&flow);
    let more = gtk::Button::with_label("Plus de résultats");
    more.add_css_class("pill");
    more.set_halign(gtk::Align::Center);
    more.set_visible(false);
    v.append(&more);

    struct Query {
        text: String,
        sort: api::Sort,
        from: usize,
        generation: u32,
    }
    let q = Rc::new(RefCell::new(Query { text: String::new(), sort: api::Sort::Popular, from: 0, generation: 0 }));
    const PAGE: usize = 12;
    let load: Rc<dyn Fn(bool)> = {
        let (app, holder, flow, status, more, q, key) = (app.clone(), holder.clone(), flow.clone(), status.clone(), more.clone(), q.clone(), key.clone());
        Rc::new(move |append: bool| {
            if !append {
                flow.remove_all();
                q.borrow_mut().from = 0;
            }
            let (text, sort, from, generation) = {
                let mut qq = q.borrow_mut();
                qq.generation += 1;
                (qq.text.clone(), qq.sort, qq.from, qq.generation)
            };
            status.set_text("Recherche sur Shadertoy…");
            more.set_visible(false);
            let key2 = key.clone();
            let task = background(move || api::query(&key2, &text, sort, from, PAGE).map_err(|e| format!("{e:#}")));
            let (app, holder, flow, status, more, q, key) = (app.clone(), holder.clone(), flow.clone(), status.clone(), more.clone(), q.clone(), key.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                if q.borrow().generation != generation {
                    return;
                }
                let (total, ids) = match res {
                    Ok(r) => r,
                    Err(e) => {
                        status.set_text(&e);
                        return;
                    }
                };
                status.set_text(&format!("{total} animation{} · les incompatibles (textures, son, plusieurs passes) sont grisées", plural(total)));
                q.borrow_mut().from = from + ids.len();
                more.set_visible(from + ids.len() < total);
                for id in ids {
                    let placeholder = gtk::Spinner::new();
                    placeholder.start();
                    let t = tile(&placeholder, "…", "", false);
                    t.set_sensitive(false);
                    flow.append(&t);
                    let (key, id2) = (key.clone(), id.clone());
                    let task = background(move || {
                        let r = api::shader(&key, &id2);
                        let thumb = thumb_cache(&id2);
                        if !thumb.exists() {
                            let _ = api::download_thumb(&id2, &thumb);
                        }
                        r.map_err(|e| format!("{e:#}"))
                    });
                    let (app, holder) = (app.clone(), holder.clone());
                    glib::spawn_future_local(async move {
                        let Ok(remote) = task.await else {
                            t.set_visible(false);
                            return;
                        };
                        let pic = gtk::Picture::for_filename(thumb_cache(&id));
                        pic.set_content_fit(gtk::ContentFit::Cover);
                        let desc = if remote.compatible {
                            format!("par {} · ♥ {}", remote.saved.author, remote.saved.likes)
                        } else {
                            format!("Incompatible : {}", remote.reason)
                        };
                        let fresh = tile(&pic, &remote.saved.name, &desc, false);
                        // Remplacer le contenu de la tuile provisoire.
                        t.set_child(fresh.child().as_ref());
                        t.set_sensitive(true);
                        if !remote.compatible {
                            t.add_css_class("incompatible");
                        }
                        t.connect_clicked(move |_| remote_dialog(&app, &holder, &remote));
                    });
                }
            });
        })
    };
    {
        let (q, load) = (q.clone(), load.clone());
        search.connect_activate(move |s| {
            q.borrow_mut().text = s.text().to_string();
            load(false);
        });
    }
    {
        let (q, load) = (q.clone(), load.clone());
        sort.connect_selected_notify(move |d| {
            q.borrow_mut().sort = match d.selected() {
                1 => api::Sort::Love,
                2 => api::Sort::Newest,
                3 => api::Sort::Hot,
                _ => api::Sort::Popular,
            };
            load(false);
        });
    }
    {
        let load = load.clone();
        more.connect_clicked(move |_| load(true));
    }
    v
}

fn remote_dialog(app: &Rc<App>, holder: &gtk::Box, remote: &api::Remote) {
    let s = &remote.saved;
    if !remote.compatible {
        app.error(&s.name, &format!("Cette animation {} : l'écran de verrouillage ne peut pas l'afficher.", remote.reason));
        return;
    }
    let src = noos_shader::Source { name: s.name.clone(), author: s.author.clone(), image: s.image.clone(), common: s.common.clone() };
    let download = |apply: bool| -> Rc<dyn Fn()> {
        let (app, holder, remote) = (app.clone(), holder.clone(), remote.clone());
        Rc::new(move || {
            let r = remote.clone();
            let task = background(move || api::save(&r).map_err(|e| format!("{e:#}")));
            let (app, holder) = (app.clone(), holder.clone());
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(saved) if apply => apply_animation(&app, &holder, &format!("shadertoy:{}", saved.id), &saved.name),
                    Ok(saved) => {
                        app.toast(&format!("« {} » ajoutée à la bibliothèque", saved.name));
                        reload(&app, &holder);
                    }
                    Err(e) => app.error("Téléchargement", &e),
                }
            });
        })
    };
    let open: Rc<dyn Fn()> = {
        let url = s.url();
        Rc::new(move || {
            let _ = gio::AppInfo::launch_default_for_uri(&url, None::<&gio::AppLaunchContext>);
        })
    };
    preview_dialog(
        app,
        &s.name,
        &format!(
            "par {} · {} vues · {} j'aime{}\nLicence : celle choisie par l'auteur sur Shadertoy (par défaut CC BY-NC-SA 3.0).",
            s.author,
            s.views,
            s.likes,
            if s.tags.is_empty() { String::new() } else { format!(" · {}", s.tags.join(", ")) }
        ),
        src,
        &[("Voir sur Shadertoy", "", open), ("Télécharger", "", download(false)), ("Télécharger et appliquer", "suggested-action", download(true))],
    );
}

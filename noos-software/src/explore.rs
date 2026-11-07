//! Page Explorer : vitrine Flathub (bannière des applis de la semaine, catégories,
//! populaires, tendances, nouveautés) et fiche détaillée d'une application.

use std::{cell::Cell, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::glib;
use noos_pkg::{BackendKind, Package, Scope};
use noos_theme::color::Rgb;

use crate::{
    App,
    flathub::{self, AppDetails, AppSummary},
    ops::background,
};

/// Charge une image distante (cache local) dans un `gtk::Image`.
fn async_image(img: &gtk::Image, url: &str) {
    let url = url.to_string();
    let img = img.clone();
    let task = background(move || flathub::cached_image(&url));
    glib::spawn_future_local(async move {
        if let Some(path) = task.await {
            img.set_from_file(Some(&path));
        }
    });
}

fn async_picture(pic: &gtk::Picture, url: &str) {
    let url = url.to_string();
    let pic = pic.clone();
    let task = background(move || flathub::cached_image(&url));
    glib::spawn_future_local(async move {
        if let Some(path) = task.await {
            pic.set_filename(Some(&path));
        }
    });
}

fn section_header(title: &str) -> (gtk::Box, gtk::Button) {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = gtk::Label::new(Some(title));
    l.add_css_class("title-3");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    h.append(&l);
    let more = gtk::Button::with_label("Tout voir");
    more.add_css_class("flat");
    h.append(&more);
    (h, more)
}

fn flowbox(min: u32, max: u32) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_homogeneous(true);
    f.set_min_children_per_line(min);
    f.set_max_children_per_line(max);
    f.set_column_spacing(12);
    f.set_row_spacing(12);
    f.set_valign(gtk::Align::Start);
    f
}

impl App {
    pub(crate) fn is_flatpak_installed(&self, app_id: &str) -> bool {
        self.installed
            .borrow()
            .iter()
            .any(|(i, list, _)| self.kind(*i) == BackendKind::Flatpak && list.iter().any(|p| p.id == app_id))
    }

    /// Paquet dnf qui fournit le même logiciel qu'une appli Flathub (même identifiant AppStream).
    pub(crate) fn dnf_alternative(&self, app_id: &str) -> Option<Package> {
        let id = app_id.to_lowercase();
        let name = self.appstream.borrow().iter().find(|(_, v)| **v == id).map(|(k, _)| k.clone())?;
        self.backend_index(BackendKind::Dnf)?;
        let installed = self
            .installed
            .borrow()
            .iter()
            .any(|(i, list, _)| self.kind(*i) == BackendKind::Dnf && list.iter().any(|p| p.id == name));
        Some(Package {
            backend: Some(BackendKind::Dnf),
            id: name.clone(),
            name,
            origin: "fedora".into(),
            installed,
            ..Default::default()
        })
    }

    /// Installe une appli Flathub via le backend Flatpak (remote « flathub »).
    pub(crate) fn install_flathub(self: &Rc<Self>, app_id: &str, name: &str) {
        let Some(idx) = self.backends.iter().position(|b| b.kind() == BackendKind::Flatpak) else {
            self.toasts.add_toast(adap::Toast::new("Flatpak n'est pas installé sur ce système"));
            return;
        };
        let (id, n) = (app_id.to_string(), name.to_string());
        let app = self.clone();
        self.ops.enqueue(
            format!("Installation de {name}"),
            idx,
            move |b| {
                // Remote Flathub actif : utilisateur de préférence (pas de mot de passe).
                let repos = b.repos()?;
                let flathub = repos
                    .iter()
                    .filter(|r| r.file == "flathub" && r.enabled)
                    .min_by_key(|r| if r.scope == Some(Scope::User) { 0 } else { 1 })
                    .ok_or_else(|| anyhow::anyhow!("Flathub n'est pas configuré : ajoutez-le dans l'onglet Sources"))?;
                let pkg = Package {
                    backend: Some(BackendKind::Flatpak),
                    id,
                    name: n,
                    origin: "flathub".into(),
                    scope: flathub.scope,
                    ..Default::default()
                };
                Ok(b.install(&[pkg]))
            },
            move |_| app.after_change(),
        );
    }

    // ───────────────────────── Accueil ─────────────────────────

    pub(crate) fn build_home(self: &Rc<Self>) {
        let home = &self.explore_home;
        while let Some(c) = home.first_child() {
            home.remove(&c);
        }

        // ── Bannière ──
        let carousel = adap::Carousel::new();
        carousel.set_allow_scroll_wheel(false);
        carousel.set_spacing(16);
        carousel.set_height_request(300);
        let dots = adap::CarouselIndicatorDots::new();
        dots.set_carousel(Some(&carousel));
        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        placeholder.add_css_class("noos-banner");
        placeholder.set_hexpand(true);
        let sp = gtk::Spinner::new();
        sp.start();
        sp.set_vexpand(true);
        placeholder.append(&sp);
        carousel.append(&placeholder);
        home.append(&carousel);
        home.append(&dots);
        self.fill_banner(&carousel, &placeholder);

        // ── Catégories ──
        let cat_title = gtk::Label::new(Some("Catégories"));
        cat_title.add_css_class("title-3");
        cat_title.set_xalign(0.0);
        home.append(&cat_title);
        let cats = flowbox(2, 5);
        for (id, label, icon) in flathub::CATEGORIES {
            let b = gtk::Button::new();
            b.add_css_class("noos-category");
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            h.append(&gtk::Image::from_icon_name(icon));
            h.append(&gtk::Label::new(Some(label)));
            b.set_child(Some(&h));
            let app = self.clone();
            b.connect_clicked(move |_| app.show_collection(&format!("category/{id}"), label));
            cats.insert(&b, -1);
        }
        home.append(&cats);

        // ── Sections ──
        for (coll, title) in [
            ("popular", "Populaires sur Flathub"),
            ("trending", "Tendances"),
            ("recently-added", "Nouveautés"),
            ("recently-updated", "Mises à jour récentes"),
        ] {
            let (header, more) = section_header(title);
            home.append(&header);
            let grid = flowbox(2, 3);
            home.append(&grid);
            let app = self.clone();
            more.connect_clicked(move |_| app.show_collection(coll, title));
            self.fill_grid(&grid, coll.to_string(), 9);
        }

        let foot = gtk::Label::new(Some("Données fournies par Flathub (flathub.org)"));
        foot.add_css_class("dim-label");
        foot.add_css_class("caption");
        home.append(&foot);
    }

    fn fill_grid(self: &Rc<Self>, grid: &gtk::FlowBox, coll: String, count: usize) {
        let task = background(move || {
            if let Some(cat) = coll.strip_prefix("category/") {
                flathub::category(cat, count)
            } else {
                flathub::collection(&coll, count)
            }
        });
        let (app, grid) = (self.clone(), grid.clone());
        glib::spawn_future_local(async move {
            match task.await {
                Ok(apps) => {
                    for a in apps {
                        grid.insert(&app.app_tile(&a), -1);
                    }
                }
                Err(e) => {
                    let l = gtk::Label::new(Some(&format!("Indisponible : {e}")));
                    l.add_css_class("dim-label");
                    grid.insert(&l, -1);
                }
            }
        });
    }

    fn app_tile(self: &Rc<Self>, a: &AppSummary) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("noos-tile");
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let icon = gtk::Image::from_icon_name("application-x-executable");
        icon.set_pixel_size(56);
        async_image(&icon, &a.icon_url);
        h.append(&icon);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
        v.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(Some(&a.name));
        name.add_css_class("heading");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        v.append(&name);
        let summary = gtk::Label::new(Some(&a.summary));
        summary.add_css_class("dim-label");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_lines(2);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.set_max_width_chars(28);
        v.append(&summary);
        let meta = if self.is_flatpak_installed(&a.app_id) {
            "✓ Installé".to_string()
        } else {
            flathub::format_installs(a.installs_last_month)
        };
        if !meta.is_empty() {
            let m = gtk::Label::new(Some(&meta));
            m.add_css_class("caption");
            m.add_css_class(if meta.starts_with('✓') { "success" } else { "dim-label" });
            m.set_xalign(0.0);
            v.append(&m);
        }
        h.append(&v);
        b.set_child(Some(&h));
        let (app, id) = (self.clone(), a.app_id.clone());
        b.connect_clicked(move |_| app.show_details(&id));
        b
    }

    fn fill_banner(self: &Rc<Self>, carousel: &adap::Carousel, placeholder: &gtk::Box) {
        let task = background(|| {
            let mut ids = Vec::new();
            if let Ok(day) = flathub::app_of_the_day() {
                if !day.is_empty() {
                    ids.push((day, "Appli du jour"));
                }
            }
            for id in flathub::apps_of_the_week().unwrap_or_default() {
                if !ids.iter().any(|(x, _)| *x == id) {
                    ids.push((id, "Appli de la semaine"));
                }
            }
            if ids.is_empty() {
                // Hors ligne ou API modifiée : on se rabat sur les tendances.
                for a in flathub::collection("trending", 5).unwrap_or_default() {
                    ids.push((a.app_id, "Tendance"));
                }
            }
            ids.into_iter()
                .take(6)
                .filter_map(|(id, label)| flathub::details(&id).ok().map(|d| (d, label)))
                .collect::<Vec<_>>()
        });
        let (app, carousel, placeholder) = (self.clone(), carousel.clone(), placeholder.clone());
        glib::spawn_future_local(async move {
            let items = task.await;
            if items.is_empty() {
                carousel.set_visible(false);
                return;
            }
            carousel.remove(&placeholder);
            let mut css = String::new();
            for (i, (d, label)) in items.iter().enumerate() {
                css.push_str(&banner_css(i, d));
                carousel.append(&app.banner_page(i, d, label));
            }
            app.dynamic_css.load_from_string(&css);
            // Défilement automatique toutes les 7 s (pause pendant le survol).
            let hover = Rc::new(Cell::new(false));
            let motion = gtk::EventControllerMotion::new();
            {
                let h = hover.clone();
                motion.connect_enter(move |_, _, _| h.set(true));
            }
            {
                let h = hover.clone();
                motion.connect_leave(move |_| h.set(false));
            }
            carousel.add_controller(motion);
            let c = carousel.clone();
            glib::timeout_add_local(Duration::from_secs(7), move || {
                if c.root().is_none() {
                    return glib::ControlFlow::Break;
                }
                let n = c.n_pages();
                if n > 1 && !hover.get() && c.is_mapped() {
                    let next = (c.position().round() as u32 + 1) % n;
                    c.scroll_to(&c.nth_page(next), true);
                }
                glib::ControlFlow::Continue
            });
        });
    }

    fn banner_page(self: &Rc<Self>, i: usize, d: &AppDetails, label: &str) -> gtk::Widget {
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        card.add_css_class("noos-banner");
        card.add_css_class(&format!("noos-banner-{i}"));
        card.set_hexpand(true);

        let left = gtk::Box::new(gtk::Orientation::Vertical, 10);
        left.set_valign(gtk::Align::Center);
        left.set_hexpand(true);
        let tag = gtk::Label::new(Some(label));
        tag.add_css_class("caption-heading");
        tag.add_css_class("banner-tag");
        tag.set_xalign(0.0);
        left.append(&tag);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let icon = gtk::Image::from_icon_name("application-x-executable");
        icon.set_pixel_size(80);
        async_image(&icon, &d.summary.icon_url);
        head.append(&icon);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 4);
        names.set_valign(gtk::Align::Center);
        let title = gtk::Label::new(Some(&d.summary.name));
        title.add_css_class("title-1");
        title.set_xalign(0.0);
        title.set_wrap(true);
        names.append(&title);
        if !d.summary.developer.is_empty() {
            let dev = gtk::Label::new(Some(&d.summary.developer));
            dev.set_xalign(0.0);
            dev.add_css_class("banner-dim");
            names.append(&dev);
        }
        head.append(&names);
        left.append(&head);
        let summary = gtk::Label::new(Some(&d.summary.summary));
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.add_css_class("title-4");
        left.append(&summary);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let more = gtk::Button::with_label("En savoir plus");
        more.add_css_class("pill");
        more.add_css_class("banner-button");
        {
            let (app, id) = (self.clone(), d.summary.app_id.clone());
            more.connect_clicked(move |_| app.show_details(&id));
        }
        buttons.append(&more);
        if !self.is_flatpak_installed(&d.summary.app_id) {
            let inst = gtk::Button::with_label("Installer");
            inst.add_css_class("pill");
            inst.add_css_class("banner-button");
            let (app, id, name) = (self.clone(), d.summary.app_id.clone(), d.summary.name.clone());
            inst.connect_clicked(move |_| app.install_flathub(&id, &name));
            buttons.append(&inst);
        }
        left.append(&buttons);
        card.append(&left);

        if let Some(shot) = d.screenshots.first() {
            let pic = gtk::Picture::new();
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_size_request(420, 250);
            pic.set_valign(gtk::Align::Center);
            pic.add_css_class("banner-shot");
            pic.set_overflow(gtk::Overflow::Hidden);
            async_picture(&pic, shot);
            card.append(&pic);
        }
        card.upcast()
    }

    // ───────────────────────── Vue collection / catégorie ─────────────────────────

    pub(crate) fn show_collection(self: &Rc<Self>, coll: &str, title: &str) {
        let b = &self.collection_box;
        while let Some(c) = b.first_child() {
            b.remove(&c);
        }
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.add_css_class("circular");
        {
            let app = self.clone();
            back.connect_clicked(move |_| app.explore_stack.set_visible_child_name("home"));
        }
        head.append(&back);
        let t = gtk::Label::new(Some(title));
        t.add_css_class("title-2");
        head.append(&t);
        b.append(&head);
        let grid = flowbox(2, 3);
        b.append(&grid);
        self.fill_grid(&grid, coll.to_string(), 48);
        self.explore_stack.set_visible_child_name("collection");
    }

    // ───────────────────────── Fiche d'application ─────────────────────────

    pub(crate) fn show_details(self: &Rc<Self>, app_id: &str) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(18);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let sp = gtk::Spinner::new();
        sp.start();
        sp.set_size_request(32, 32);
        sp.set_vexpand(true);
        content.append(&sp);
        let scroll = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&scroll));
        let dialog = adap::Dialog::builder().content_width(820).content_height(700).child(&tv).build();
        dialog.present(Some(&self.window));

        let id = app_id.to_string();
        let task = background(move || flathub::details(&id));
        let app = self.clone();
        let dialog2 = dialog.clone();
        glib::spawn_future_local(async move {
            content.remove(&sp);
            let d = match task.await {
                Ok(d) => d,
                Err(e) => {
                    content.append(&crate::status_page("network-offline-symbolic", "Fiche indisponible", &format!("{e:#}")));
                    return;
                }
            };
            dialog2.set_title(&d.summary.name);

            // En-tête
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 18);
            let icon = gtk::Image::from_icon_name("application-x-executable");
            icon.set_pixel_size(96);
            async_image(&icon, &d.summary.icon_url);
            head.append(&icon);
            let names = gtk::Box::new(gtk::Orientation::Vertical, 4);
            names.set_valign(gtk::Align::Center);
            names.set_hexpand(true);
            let t = gtk::Label::new(Some(&d.summary.name));
            t.add_css_class("title-1");
            t.set_xalign(0.0);
            t.set_wrap(true);
            names.append(&t);
            let dev = gtk::Label::new(Some(&format!(
                "{}{}",
                d.summary.developer,
                if d.summary.verified { "  ✓ vérifié" } else { "" }
            )));
            dev.add_css_class("dim-label");
            dev.set_xalign(0.0);
            names.append(&dev);
            let sum = gtk::Label::new(Some(&d.summary.summary));
            sum.set_xalign(0.0);
            sum.set_wrap(true);
            names.append(&sum);
            head.append(&names);
            let installed = app.is_flatpak_installed(&d.summary.app_id);
            let alternative = app.dnf_alternative(&d.summary.app_id);
            let action: gtk::Widget = if let Some(dnf) = alternative.as_ref().filter(|p| p.installed) {
                // Déjà installé en paquet Fedora.
                let b = gtk::Button::with_label("Supprimer");
                b.add_css_class("destructive-action");
                b.set_tooltip_text(Some("Installé en paquet Fedora (dnf)"));
                let (app2, pkg, dlg) = (app.clone(), dnf.clone(), dialog2.clone());
                b.connect_clicked(move |_| {
                    dlg.close();
                    app2.remove(pkg.clone());
                });
                b.upcast()
            } else if let (false, Some(dnf)) = (installed, alternative.clone()) {
                // Disponible de deux façons : on laisse choisir.
                let flathub = Package {
                    backend: Some(BackendKind::Flatpak),
                    id: d.summary.app_id.clone(),
                    name: d.summary.name.clone(),
                    origin: "flathub".into(),
                    version: d.version.clone(),
                    ..Default::default()
                };
                let sw = noos_pkg::catalog::Software {
                    key: d.summary.app_id.to_lowercase(),
                    name: d.summary.name.clone(),
                    summary: d.summary.summary.clone(),
                    offers: vec![flathub, Package { name: d.summary.name.clone(), ..dnf }],
                };
                app.action_button(&sw)
            } else if installed {
                let b = gtk::Button::with_label("Supprimer");
                b.add_css_class("destructive-action");
                let (app2, aid, name, dlg) = (app.clone(), d.summary.app_id.clone(), d.summary.name.clone(), dialog2.clone());
                b.connect_clicked(move |_| {
                    dlg.close();
                    {
                        let pkg = app2
                            .installed
                            .borrow()
                            .iter()
                            .flat_map(|(_, l, _)| l.iter())
                            .find(|p| p.id == aid)
                            .cloned()
                            .unwrap_or(Package {
                                backend: Some(BackendKind::Flatpak),
                                id: aid.clone(),
                                name: name.clone(),
                                ..Default::default()
                            });
                        app2.remove(pkg);
                    }
                });
                b.upcast()
            } else {
                let b = gtk::Button::with_label("Installer");
                b.add_css_class("suggested-action");
                let (app2, aid, name, dlg) = (app.clone(), d.summary.app_id.clone(), d.summary.name.clone(), dialog2.clone());
                b.connect_clicked(move |_| {
                    dlg.close();
                    app2.install_flathub(&aid, &name);
                });
                b.upcast()
            };
            action.add_css_class("pill");
            action.set_valign(gtk::Align::Center);
            head.append(&action);
            content.append(&head);

            // Captures
            if !d.screenshots.is_empty() {
                let shots = adap::Carousel::new();
                shots.set_height_request(380);
                shots.set_spacing(12);
                for url in &d.screenshots {
                    let pic = gtk::Picture::new();
                    pic.set_content_fit(gtk::ContentFit::Contain);
                    pic.set_hexpand(true);
                    async_picture(&pic, url);
                    shots.append(&pic);
                }
                content.append(&shots);
                if d.screenshots.len() > 1 {
                    let dots = adap::CarouselIndicatorDots::new();
                    dots.set_carousel(Some(&shots));
                    content.append(&dots);
                }
            }

            // Description
            let desc = gtk::Label::new(Some(&d.description));
            desc.set_wrap(true);
            desc.set_xalign(0.0);
            desc.set_selectable(true);
            content.append(&desc);

            // Informations
            let info = adap::PreferencesGroup::new();
            info.set_title("Informations");
            for (title, value) in [
                ("Version", d.version.clone()),
                ("Licence", d.license.clone()),
                ("Identifiant", d.summary.app_id.clone()),
                ("Source", "Flathub (Flatpak)".to_string()),
            ] {
                if value.is_empty() {
                    continue;
                }
                let r = crate::row(title, &value);
                r.set_subtitle_selectable(true);
                info.add(&r);
            }
            if !d.homepage.is_empty() {
                let r = crate::row("Site web", &d.homepage);
                let link = gtk::LinkButton::with_label(&d.homepage, "Ouvrir");
                link.set_valign(gtk::Align::Center);
                r.add_suffix(&link);
                info.add(&r);
            }
            content.append(&info);
        });
    }
}

/// Dégradé aux couleurs de marque de l'appli (variante claire / sombre selon le thème).
fn banner_css(i: usize, d: &AppDetails) -> String {
    let dark = adap::StyleManager::default().is_dark();
    let brand = if dark { d.brand_dark.as_ref().or(d.brand_light.as_ref()) } else { d.brand_light.as_ref().or(d.brand_dark.as_ref()) };
    let base = brand.and_then(|c| Rgb::parse(c)).unwrap_or(if dark { Rgb { r: 0.25, g: 0.27, b: 0.40 } } else { Rgb { r: 0.80, g: 0.84, b: 0.95 } });
    let end = base.mix(if dark { Rgb::BLACK } else { Rgb::WHITE }, 0.35);
    let fg = base.mix(end, 0.5).readable_fg();
    let fg_hex = fg.hex();
    format!(
        ".noos-banner-{i} {{ background-image: linear-gradient(135deg, {}, {}); color: {fg_hex}; }}\n\
         .noos-banner-{i} .banner-dim, .noos-banner-{i} .banner-tag {{ color: alpha({fg_hex}, 0.75); }}\n\
         .noos-banner-{i} .banner-button {{ background-color: alpha({fg_hex}, 0.16); color: {fg_hex}; }}\n\
         .noos-banner-{i} .banner-button:hover {{ background-color: alpha({fg_hex}, 0.26); }}\n",
        base.hex(),
        end.hex()
    )
}


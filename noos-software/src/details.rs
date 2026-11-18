//! Fiche d'un logiciel trouvé par recherche (ou listé dans Installées) : descriptif, captures,
//! versions, tailles, dépôt, licence, dépendances et simulation de l'installation ou de la
//! suppression. Un onglet par façon de l'installer (Flatpak, paquet Debian…).
//! Données : `Backend::details` et `Backend::simulate` de `noos-pkg` (sans droits root).

use std::{cell::Cell, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_pkg::{
    BackendKind, Package,
    catalog::{self, Software},
    details::{Details, Effect},
};

use crate::{App, explore::{async_image, async_picture}, ops::background, row, source_text};

/// Section Debian (`graphics`, `contrib/games`…) en français.
fn section_label(section: &str) -> String {
    let (area, name) = match section.split_once('/') {
        Some((a, n)) => (Some(a), n),
        None => (None, section),
    };
    let fr = match name {
        "admin" => "Administration système",
        "cli-mono" | "java" | "javascript" | "perl" | "php" | "python" | "ruby" | "rust" | "golang" | "haskell" | "ocaml" | "lisp" => "Langages de programmation",
        "comm" => "Communication",
        "database" => "Bases de données",
        "debug" => "Débogage",
        "devel" => "Développement",
        "doc" => "Documentation",
        "editors" => "Éditeurs",
        "education" => "Éducation",
        "electronics" => "Électronique",
        "embedded" => "Systèmes embarqués",
        "fonts" => "Polices",
        "games" => "Jeux",
        "gnome" => "GNOME",
        "graphics" => "Graphisme",
        "hamradio" => "Radioamateur",
        "httpd" => "Serveurs web",
        "kde" => "KDE",
        "kernel" => "Noyau",
        "libs" | "oldlibs" => "Bibliothèques",
        "libdevel" => "Bibliothèques de développement",
        "localization" => "Traductions",
        "mail" => "Courrier électronique",
        "math" => "Mathématiques",
        "metapackages" => "Métapaquets",
        "misc" => "Divers",
        "net" => "Réseau",
        "news" => "Forums Usenet",
        "science" => "Sciences",
        "shells" => "Interpréteurs de commandes",
        "sound" => "Son",
        "tex" => "TeX",
        "text" => "Traitement de texte",
        "utils" => "Utilitaires",
        "vcs" => "Gestion de versions",
        "video" => "Vidéo",
        "web" => "Internet",
        "x11" => "Interface graphique (X11)",
        "xfce" => "Xfce",
        "" => return String::new(),
        other => other,
    };
    match area {
        Some(a) => format!("{fr} ({a})"),
        None => fr.to_string(),
    }
}

fn size(n: Option<u64>) -> String {
    n.filter(|n| *n > 0).map(|n| glib::format_size(n).to_string()).unwrap_or_default()
}

/// Liste courte : « a, b, c et 4 autres ».
fn short_list(names: &[String], max: usize) -> String {
    if names.len() <= max { names.join(", ") } else { format!("{} et {} autres", names[..max].join(", "), names.len() - max) }
}

/// Lanceur installé pour « Ouvrir ».
fn launcher(d: &Details) -> Option<gio::AppInfo> {
    let id = d.launchable.as_deref()?;
    let want = format!("{}.desktop", id.trim_end_matches(".desktop"));
    gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i.as_str() == want))
}

impl App {
    /// Ouvre la fiche d'un logiciel. `prefer` : onglet ouvert d'abord (sinon celui qui est
    /// installé, ou la première façon de l'installer).
    pub(crate) fn show_software(self: &Rc<Self>, sw: &Software, prefer: Option<BackendKind>) {
        let mut offers: Vec<Package> = sw.installed().cloned().collect();
        offers.extend(sw.offers.iter().filter(|o| !o.installed).cloned());
        if offers.is_empty() {
            return;
        }
        let start = prefer.and_then(|k| offers.iter().position(|o| o.backend == Some(k))).unwrap_or(0);

        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroll = gtk::ScrolledWindow::builder().child(&container).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let header = adap::HeaderBar::new();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&scroll));
        // Fond opaque : avec la transparence (Mica), la vitrine apparaissait à travers la fiche.
        tv.add_css_class("noos-sheet");
        let dialog = adap::Dialog::builder().title(&sw.name).content_width(860).content_height(760).child(&tv).build();
        let generation = Rc::new(Cell::new(0u64));

        if offers.len() > 1 {
            // Une façon de l'installer par onglet : comparer Flatpak et paquet Debian.
            let switch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            switch.add_css_class("linked");
            let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
            for o in &offers {
                let label = if o.installed { format!("✓ {}", source_text(o)) } else { source_text(o) };
                let b = gtk::ToggleButton::with_label(&label);
                b.set_tooltip_text(Some(crate::offer_explanation(o)));
                if let Some(first) = buttons.first() {
                    b.set_group(Some(first));
                }
                let (app, c, d, g, p) = (self.clone(), container.clone(), dialog.clone(), generation.clone(), o.clone());
                b.connect_toggled(move |b| {
                    if b.is_active() {
                        app.fill_package(&c, &d, &g, p.clone());
                    }
                });
                switch.append(&b);
                buttons.push(b);
            }
            header.set_title_widget(Some(&switch));
            let cmp = gtk::Button::with_label("Comparer");
            cmp.set_tooltip_text(Some("Comparer les façons d'installer ce logiciel"));
            let (app, sw2) = (self.clone(), sw.clone());
            cmp.connect_clicked(move |_| app.show_compare(&sw2));
            header.pack_start(&cmp);
            buttons[start].set_active(true);
        } else {
            self.fill_package(&container, &dialog, &generation, offers[0].clone());
        }
        dialog.present(Some(&self.window));

        // Captures d'écran (aperçu sans souris) : bas de la fiche.
        if let Some(px) = std::env::var("NOOS_SOFTWARE_SCROLL").ok().and_then(|v| v.parse::<f64>().ok()) {
            glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
                scroll.vadjustment().set_value(px);
            });
        }
    }

    fn fill_package(self: &Rc<Self>, container: &gtk::Box, dialog: &adap::Dialog, generation: &Rc<Cell<u64>>, pkg: Package) {
        crate::clear(container);
        container.append(&crate::loading("Lecture de la fiche…"));
        let gen_now = generation.get() + 1;
        generation.set(gen_now);
        let Some(idx) = pkg.backend.and_then(|k| self.backend_index(k)) else { return };

        let backends = self.backends.clone();
        let p = pkg.clone();
        let details = background(move || backends[idx].details(&p).map_err(|e| format!("{e:#}")));
        let backends = self.backends.clone();
        let p = pkg.clone();
        let effect = background(move || backends[idx].simulate(&p, !p.installed).map_err(|e| format!("{e:#}")));

        let (app, container, dialog, generation) = (self.clone(), container.clone(), dialog.clone(), generation.clone());
        glib::spawn_future_local(async move {
            let d = details.await;
            if generation.get() != gen_now {
                return;
            }
            crate::clear(&container);
            let d = match d {
                Ok(d) => d,
                Err(e) => {
                    container.append(&crate::status_page("dialog-warning-symbolic", "Fiche indisponible", &e));
                    return;
                }
            };
            let (sim_group, waiting) = app.render_details(&container, &dialog, &pkg, &d);
            let e = effect.await;
            if generation.get() == gen_now {
                sim_group.remove(&waiting);
                fill_effect(&sim_group, &pkg, e);
            }
        });
    }

    /// Construit la fiche ; renvoie le groupe « simulation » et sa ligne d'attente.
    fn render_details(self: &Rc<Self>, container: &gtk::Box, dialog: &adap::Dialog, pkg: &Package, d: &Details) -> (adap::PreferencesGroup, adap::ActionRow) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
        content.set_margin_top(18);
        content.set_margin_bottom(28);
        content.set_margin_start(28);
        content.set_margin_end(28);
        container.append(&content);
        let kind = pkg.backend.unwrap_or(BackendKind::Apt);
        dialog.set_title(&d.name);

        // ── En-tête : icône, nom, résumé, actions ──
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        head.append(&details_icon(pkg, d));
        let names = gtk::Box::new(gtk::Orientation::Vertical, 4);
        names.set_valign(gtk::Align::Center);
        names.set_hexpand(true);
        let t = gtk::Label::new(Some(&d.name));
        t.add_css_class("title-1");
        t.set_xalign(0.0);
        t.set_wrap(true);
        names.append(&t);
        if !d.summary.is_empty() {
            let s = gtk::Label::new(Some(&d.summary));
            s.set_xalign(0.0);
            s.set_wrap(true);
            names.append(&s);
        }
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.set_margin_top(4);
        line.append(&crate::badge(&if pkg.installed { format!("✓ {}", source_text(pkg)) } else { source_text(pkg) }, kind));
        // Mainteneur Debian : nom sans l'adresse.
        let maintainer = d.maintainer.split('<').next().unwrap_or("").trim().to_string();
        let byline = if d.developer.is_empty() { maintainer.clone() } else { d.developer.clone() };
        if !byline.is_empty() {
            let dev = gtk::Label::new(Some(&byline));
            dev.add_css_class("dim-label");
            dev.set_ellipsize(gtk::pango::EllipsizeMode::End);
            dev.set_xalign(0.0);
            line.append(&dev);
        }
        names.append(&line);
        head.append(&names);

        let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
        actions.set_valign(gtk::Align::Center);
        if pkg.installed {
            if let Some(info) = launcher(d) {
                let open = gtk::Button::with_label("Ouvrir");
                open.add_css_class("suggested-action");
                open.add_css_class("pill");
                let toasts = self.toasts.clone();
                open.connect_clicked(move |_| {
                    if let Err(e) = info.launch(&[], None::<&gio::AppLaunchContext>) {
                        toasts.add_toast(adap::Toast::new(&format!("Impossible d'ouvrir : {e}")));
                    }
                });
                actions.append(&open);
            }
            let rm = gtk::Button::with_label("Supprimer");
            rm.add_css_class("destructive-action");
            rm.add_css_class("pill");
            let (app, p, dlg) = (self.clone(), pkg.clone(), dialog.clone());
            rm.connect_clicked(move |_| {
                dlg.close();
                app.remove(p.clone());
            });
            actions.append(&rm);
        } else {
            let inst = gtk::Button::with_label("Installer");
            inst.add_css_class("suggested-action");
            inst.add_css_class("pill");
            inst.set_tooltip_text(Some(&source_text(pkg)));
            let (app, p, dlg) = (self.clone(), pkg.clone(), dialog.clone());
            inst.connect_clicked(move |_| {
                dlg.close();
                app.install(p.clone());
            });
            actions.append(&inst);
        }
        head.append(&actions);
        content.append(&head);

        // ── Captures d'écran ──
        if !d.screenshots.is_empty() {
            let shots = adap::Carousel::new();
            shots.set_height_request(360);
            shots.set_spacing(12);
            for s in &d.screenshots {
                let pic = gtk::Picture::new();
                pic.set_content_fit(gtk::ContentFit::Contain);
                pic.set_hexpand(true);
                pic.add_css_class("noos-shot");
                if !s.caption.is_empty() {
                    pic.set_tooltip_text(Some(&s.caption));
                }
                async_picture(&pic, &s.thumbnail);
                shots.append(&pic);
            }
            content.append(&shots);
            if d.screenshots.len() > 1 {
                let dots = adap::CarouselIndicatorDots::new();
                dots.set_carousel(Some(&shots));
                content.append(&dots);
            }
        }

        // ── Description ──
        let desc = gtk::Label::new(Some(if d.description.is_empty() { "Ce paquet n'a pas de description détaillée." } else { &d.description }));
        desc.set_wrap(true);
        desc.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        desc.set_xalign(0.0);
        desc.set_selectable(true);
        if d.description.is_empty() {
            desc.add_css_class("dim-label");
        }
        content.append(&desc);

        // ── Nouveautés (mise à jour disponible) ──
        let update = matches!((&d.installed_version, &d.available_version), (Some(i), Some(a)) if i != a && !a.is_empty());
        if update && let Some(idx) = self.backend_index(kind) {
            let news = adap::PreferencesGroup::new();
            news.set_title(&format!("Nouveautés de la version {}", d.available_version.clone().unwrap_or_default()));
            let wait = row("Lecture du journal des modifications…", "");
            let sp = gtk::Spinner::new();
            sp.start();
            wait.add_suffix(&sp);
            news.add(&wait);
            content.append(&news);
            let (backends, p) = (self.backends.clone(), pkg.clone());
            let task = background(move || backends[idx].changelog(&p).map_err(|e| format!("{e:#}")));
            glib::spawn_future_local(async move {
                let res = task.await;
                news.remove(&wait);
                match res {
                    Ok(entries) if !entries.is_empty() => {
                        for e in entries {
                            let r = row(&format!("Version {}", e.version), &e.text);
                            r.set_subtitle_lines(12);
                            r.set_subtitle_selectable(true);
                            if !e.date.is_empty() {
                                r.set_tooltip_text(Some(&e.date));
                            }
                            news.add(&r);
                        }
                    }
                    Ok(_) => news.add(&row("Pas de notes pour cette version", "")),
                    Err(e) => news.add(&row("Journal des modifications indisponible", &e)),
                }
            });
        }

        // ── Simulation ──
        let sim = adap::PreferencesGroup::new();
        sim.set_title(if pkg.installed { "Ce que la suppression retirera" } else { "Ce que l'installation ajoutera" });
        let waiting = row("Simulation en cours…", "Sans mot de passe, rien n'est modifié");
        let sp = gtk::Spinner::new();
        sp.start();
        waiting.add_suffix(&sp);
        sim.add(&waiting);
        content.append(&sim);

        // ── Données de l'application (installée) ──
        if pkg.installed {
            content.append(&self.data_group(pkg));
        }

        // ── Accès (Flatpak) ──
        if kind == BackendKind::Flatpak {
            content.append(&self.permissions_group(pkg));
        }

        // ── Avis (ODRS, si activé) ──
        if let Some(aid) = d.appstream_id.clone() {
            let ver = d.installed_version.clone().or(d.available_version.clone()).unwrap_or_default();
            content.append(&self.reviews_group(&aid, &ver));
        }

        // ── Informations ──
        let info = adap::PreferencesGroup::new();
        info.set_title("Informations");
        let rows: Vec<(&str, String)> = vec![
            ("Version installée", d.installed_version.clone().unwrap_or_default()),
            (if update { "Mise à jour disponible" } else if pkg.installed { "Version du dépôt" } else { "Version disponible" }, d.available_version.clone().filter(|a| Some(a) != d.installed_version.as_ref() || !pkg.installed).unwrap_or_default()),
            ("Taille du téléchargement", if pkg.installed && !update { String::new() } else { size(d.download_size) }),
            ("Taille une fois installé", size(d.installed_size)),
            ("Source", source_text(pkg)),
            ("Dépôt", d.repository.clone()),
            ("Catégorie", section_label(&d.section)),
            ("Licence", d.license.clone()),
            ("Développeur", d.developer.clone()),
            ("Mainteneur du paquet", maintainer),
            ("Paquet", d.id.clone()),
            ("Identifiant AppStream", d.appstream_id.clone().filter(|a| *a != d.id).unwrap_or_default()),
            ("Architecture", d.architecture.clone()),
            ("Runtime", d.runtime.clone()),
            ("Commandes", short_list(&d.commands, 8)),
        ];
        for (title, value) in rows {
            if value.is_empty() {
                continue;
            }
            let r = row(title, &value);
            r.set_subtitle_selectable(true);
            r.set_subtitle_lines(3);
            info.add(&r);
        }
        if !d.homepage.is_empty() {
            let r = row("Site web", &d.homepage);
            let link = gtk::LinkButton::with_label(&d.homepage, "Ouvrir");
            link.set_valign(gtk::Align::Center);
            r.add_suffix(&link);
            info.add(&r);
        }
        content.append(&info);

        // ── Dépendances ──
        if !(d.depends.is_empty() && d.recommends.is_empty() && d.suggests.is_empty()) {
            let deps = adap::PreferencesGroup::new();
            deps.set_title("Dépendances");
            deps.set_description(Some("Paquets nécessaires à son fonctionnement"));
            for (title, list, why) in [
                ("Dépend de", &d.depends, "Installés avec lui"),
                ("Recommande", &d.recommends, "Installés avec lui s'ils sont disponibles"),
                ("Suggère", &d.suggests, "Non installés : à ajouter vous-même si besoin"),
            ] {
                if list.is_empty() {
                    continue;
                }
                let e = adap::ExpanderRow::new();
                e.set_title(&format!("{title} ({})", list.len()));
                e.set_subtitle(&format!("{why} · {}", short_list(list, 4)));
                for dep in list.iter().take(200) {
                    let r = adap::ActionRow::new();
                    r.set_use_markup(false);
                    r.set_title(dep);
                    e.add_row(&r);
                }
                deps.add(&e);
            }
            content.append(&deps);
        }
        (sim, waiting)
    }

    /// Fiche ouverte au démarrage (`NOOS_SOFTWARE_DETAILS=gimp`, `apt:htop`,
    /// `flatpak:org.gimp.GIMP`, suffixe `@apt` / `@flatpak` pour l'onglet).
    pub(crate) fn open_details_spec(self: &Rc<Self>, spec: &str) {
        self.open_details_spec_with(spec, false);
    }

    /// Comme `open_details_spec` ; `uninstall` : propose aussitôt de supprimer la version installée
    /// (« Désinstaller » du menu Démarrer).
    pub(crate) fn open_details_spec_with(self: &Rc<Self>, spec: &str, uninstall: bool) {
        let (spec, prefer) = match spec.rsplit_once('@') {
            Some((s, k)) => (s.to_string(), BackendKind::parse(k)),
            None => (spec.to_string(), None),
        };
        let (only, id) = match spec.split_once(':') {
            Some((k, id)) if BackendKind::parse(k).is_some() => (BackendKind::parse(k), id.to_string()),
            _ => (None, spec.clone()),
        };
        let backends = self.backends.clone();
        let q = id.clone();
        let task = background(move || {
            let mut found: Vec<Package> = Vec::new();
            let mut appstream = std::collections::HashMap::new();
            for b in backends.iter().filter(|b| only.is_none_or(|k| b.kind() == k)) {
                found.extend(b.search(&q).unwrap_or_default());
                if b.kind() == BackendKind::Apt {
                    appstream = b.appstream_ids();
                    // Identifiant AppStream (`org.gimp.GIMP`, lien appstream://) → paquet Debian.
                    let lower = q.to_lowercase();
                    if let Some(pkg) = appstream.iter().find(|(_, v)| **v == lower.trim_end_matches(".desktop")).map(|(k, _)| k.clone()) {
                        found.extend(b.search(&pkg).unwrap_or_default().into_iter().filter(|p| p.id == pkg));
                    }
                }
            }
            (found, appstream)
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let (mut found, appstream) = task.await;
            if only.is_some() {
                found.retain(|p| p.id == id);
            }
            noos_pkg::rank(&id, &mut found);
            let software = catalog::merge(found, &appstream);
            let lower = id.to_lowercase();
            let pick = software
                .iter()
                .find(|s| s.offers.iter().any(|o| o.id == id) || s.key == lower.trim_end_matches(".desktop"))
                .or(software.first())
                .cloned();
            match pick {
                Some(sw) => {
                    app.show_software(&sw, prefer);
                    if let Some(p) = uninstall.then(|| sw.installed().next().cloned()).flatten() {
                        app.remove(p);
                    }
                }
                None => app.toasts.add_toast(adap::Toast::new(&format!("{id} : introuvable"))),
            }
        });
    }
}

fn stars_text(n: u32) -> String {
    (0..5).map(|i| if i < n { '★' } else { '☆' }).collect()
}

impl App {
    /// Notes et avis (odrs.gnome.org) : rien n'est demandé au service tant que l'utilisateur ne
    /// l'a pas activé (`[software] reviews`).
    fn reviews_group(self: &Rc<Self>, app_id: &str, version: &str) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        g.set_title("Avis des utilisateurs");
        // NOOS_SOFTWARE_REVIEWS=1 : aperçu, avis affichés sans toucher à la configuration.
        if !noos_core::NoosConfig::load().software.reviews && std::env::var_os("NOOS_SOFTWARE_REVIEWS").is_none() {
            let r = row("Afficher les notes et les avis", "Fournis par odrs.gnome.org (service de GNOME et KDE) : activer envoie l'identifiant de l'application et un identifiant anonyme de cet ordinateur.");
            r.set_subtitle_lines(3);
            let b = gtk::Button::with_label("Activer");
            b.set_valign(gtk::Align::Center);
            let (app, g2, aid, ver, r2) = (self.clone(), g.clone(), app_id.to_string(), version.to_string(), r.clone());
            b.connect_clicked(move |_| {
                let mut c = noos_core::NoosConfig::load();
                c.software.reviews = true;
                let _ = c.save();
                g2.remove(&r2);
                app.fill_reviews(&g2, &aid, &ver);
            });
            r.add_suffix(&b);
            g.add(&r);
        } else {
            self.fill_reviews(&g, app_id, version);
        }
        g
    }

    fn fill_reviews(self: &Rc<Self>, g: &adap::PreferencesGroup, app_id: &str, version: &str) {
        let wait = row("Chargement des avis…", "");
        g.add(&wait);
        let (aid, ver) = (app_id.to_string(), version.to_string());
        let task = background(move || (noos_pkg::odrs::ratings(&aid).ok(), noos_pkg::odrs::fetch(&aid, &ver, 8).map_err(|e| format!("{e:#}"))));
        let (app, g, aid, ver) = (self.clone(), g.clone(), app_id.to_string(), version.to_string());
        glib::spawn_future_local(async move {
            let (ratings, reviews) = task.await;
            g.remove(&wait);
            let (list, skey) = match reviews {
                Ok(x) => x,
                Err(e) => {
                    g.add(&row("Avis indisponibles", &e));
                    return;
                }
            };
            if let Some(avg) = ratings.as_ref().and_then(|r| r.average()) {
                let r = ratings.clone().unwrap_or_default();
                let bars: Vec<String> = r.stars().iter().enumerate().rev().map(|(i, c)| format!("{}★ {c}", i + 1)).collect();
                let head = row(&format!("{} {:.1} sur 5", stars_text(avg.round() as u32), avg).replace('.', ","), &format!("{} notes · {}", r.total, bars.join(" · ")));
                g.add(&head);
            } else {
                g.add(&row("Pas encore de note", "Soyez le premier à donner votre avis"));
            }
            for rv in &list {
                let who = rv.user_display.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "Anonyme".into());
                let date = glib::DateTime::from_unix_local(rv.date_created as i64).ok().and_then(|d| d.format("%e %B %Y").ok()).map(|s| s.trim().to_string()).unwrap_or_default();
                let mut meta = format!("{} · {who}", stars_text(rv.stars()));
                if let Some(v) = rv.version.as_deref().filter(|v| !v.is_empty()) {
                    meta.push_str(&format!(" · version {v}"));
                }
                if !date.is_empty() {
                    meta.push_str(&format!(" · {date}"));
                }
                let r = row(&rv.summary, &format!("{meta}\n{}", rv.description));
                r.set_subtitle_lines(8);
                r.set_subtitle_selectable(true);
                if let Some(k) = skey.clone() {
                    for (icon, action, tip) in [("emblem-ok-symbolic", "upvote", "Avis utile"), ("dialog-warning-symbolic", "report", "Signaler un avis abusif")] {
                        let b = gtk::Button::from_icon_name(icon);
                        b.add_css_class("flat");
                        b.set_valign(gtk::Align::Center);
                        b.set_tooltip_text(Some(tip));
                        let (k, aid, id, toasts) = (k.clone(), aid.clone(), rv.review_id, app.toasts.clone());
                        b.connect_clicked(move |b| {
                            b.set_sensitive(false);
                            let (k, aid) = (k.clone(), aid.clone());
                            let task = background(move || noos_pkg::odrs::vote(action, &aid, &k, id).is_ok());
                            let toasts = toasts.clone();
                            glib::spawn_future_local(async move {
                                toasts.add_toast(adap::Toast::new(if task.await { if action == "report" { "Avis signalé" } else { "Merci pour votre vote" } } else { "Envoi impossible" }));
                            });
                        });
                        r.add_suffix(&b);
                    }
                }
                g.add(&r);
            }
            if let Some(k) = skey {
                let b = gtk::Button::with_label("Écrire un avis");
                b.add_css_class("flat");
                let (app2, aid, ver) = (app.clone(), aid.clone(), ver.clone());
                b.connect_clicked(move |_| app2.write_review(&aid, &ver, &k));
                g.set_header_suffix(Some(&b));
            }
        });
    }

    /// Dialogue « Écrire un avis » : note, titre, texte, nom affiché ; publié sur odrs.gnome.org.
    fn write_review(self: &Rc<Self>, app_id: &str, version: &str, skey: &str) {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let stars = Rc::new(Cell::new(0u32));
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        line.set_halign(gtk::Align::Center);
        let buttons: Rc<std::cell::RefCell<Vec<gtk::Button>>> = Rc::default();
        for i in 1..=5u32 {
            let b = gtk::Button::with_label("☆");
            b.add_css_class("flat");
            b.add_css_class("title-2");
            let (stars, buttons2) = (stars.clone(), buttons.clone());
            b.connect_clicked(move |_| {
                stars.set(i);
                for (j, x) in buttons2.borrow().iter().enumerate() {
                    x.set_label(if (j as u32) < i { "★" } else { "☆" });
                }
            });
            line.append(&b);
            buttons.borrow_mut().push(b);
        }
        body.append(&line);
        let summary = gtk::Entry::new();
        summary.set_placeholder_text(Some("En une phrase"));
        body.append(&summary);
        let text = gtk::TextView::new();
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        text.set_height_request(120);
        text.add_css_class("card");
        body.append(&text);
        let name = gtk::Entry::new();
        name.set_placeholder_text(Some("Nom affiché (vide : Anonyme)"));
        name.set_text(&noos_core::NoosConfig::load().software.review_name);
        body.append(&name);
        let d = adap::AlertDialog::new(Some("Écrire un avis"), Some("Votre avis sera public sur odrs.gnome.org, visible dans GNOME Logiciels, KDE Discover et NOOS."));
        d.set_extra_child(Some(&body));
        d.add_responses(&[("cancel", "Annuler"), ("send", "Publier")]);
        d.set_response_appearance("send", adap::ResponseAppearance::Suggested);
        let (app, aid, ver, skey) = (self.clone(), app_id.to_string(), version.to_string(), skey.to_string());
        d.connect_response(None, move |_, r| {
            if r != "send" {
                return;
            }
            let (n, s, t) = (stars.get(), summary.text().to_string(), text.buffer().text(&text.buffer().start_iter(), &text.buffer().end_iter(), false).to_string());
            if n == 0 || s.trim().is_empty() {
                app.toasts.add_toast(adap::Toast::new("Choisissez une note et écrivez un titre"));
                return;
            }
            let who = name.text().to_string();
            let mut c = noos_core::NoosConfig::load();
            c.software.review_name = who.clone();
            let _ = c.save();
            let (aid, ver, skey) = (aid.clone(), ver.clone(), skey.clone());
            let task = background(move || noos_pkg::odrs::submit(&aid, &skey, &ver, n, &s, &t, &who).map_err(|e| format!("{e:#}")));
            let toasts = app.toasts.clone();
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(()) => toasts.add_toast(adap::Toast::new("Merci ! Votre avis est publié")),
                    Err(e) => toasts.add_toast(adap::Toast::new(&e)),
                }
            });
        });
        d.present(Some(&self.window));
    }

    /// Comparaison des façons d'installer un logiciel (apt, Flatpak, AppImage de AppImageHub).
    pub(crate) fn show_compare(self: &Rc<Self>, sw: &Software) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&crate::loading("Comparaison en cours…"));
        let scroll = gtk::ScrolledWindow::builder().child(&content).vexpand(true).build();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&scroll));
        tv.add_css_class("noos-sheet");
        let dialog = adap::Dialog::builder().title(&format!("Comparer — {}", sw.name)).content_width(860).content_height(620).child(&tv).build();
        dialog.present(Some(&self.window));

        let offers = sw.offers.clone();
        let backends = self.backends.clone();
        let names: Vec<String> = std::iter::once(sw.name.clone()).chain(sw.offers.iter().map(|o| o.id.rsplit('.').next().unwrap_or(&o.id).to_string())).collect();
        let task = background(move || {
            let rows: Vec<(Package, Option<Details>, usize)> = offers
                .into_iter()
                .map(|o| {
                    let b = o.backend.and_then(|k| backends.iter().find(|b| b.kind() == k));
                    let d = b.and_then(|b| b.details(&o).ok());
                    let wide = if o.backend == Some(BackendKind::Flatpak) {
                        noos_pkg::permissions::of_app(&o.id, o.installed, o.scope.unwrap_or(noos_pkg::Scope::System), if o.origin.is_empty() { "flathub" } else { &o.origin })
                            .map(|l| l.iter().filter(|p| p.level == 2 && p.enabled).count())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    (o, d, wide)
                })
                .collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            (rows, noos_pkg::appimagehub::find(&refs))
        });
        let (app, dlg) = (self.clone(), dialog.clone());
        glib::spawn_future_local(async move {
            let (rows, hub) = task.await;
            crate::clear(&content);
            let versions: Vec<(BackendKind, String)> = rows
                .iter()
                .map(|(o, d, _)| (o.backend.unwrap_or(BackendKind::Apt), d.as_ref().and_then(|d| d.available_version.clone().or(d.installed_version.clone())).unwrap_or_else(|| o.version.clone())))
                .collect();
            if let Some(best) = noos_pkg::catalog::recommend(&versions) {
                let others: Vec<String> = versions.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, (_, v))| v.clone()).collect();
                let why = if versions.iter().all(|(_, v)| noos_pkg::catalog::compare_versions(v, &versions[best].1).is_eq()) {
                    "même version : le paquet du système est mieux intégré".to_string()
                } else {
                    format!("version plus récente ({} contre {})", versions[best].1, others.join(", "))
                };
                let l = gtk::Label::new(Some(&format!("Conseillé : {} — {why}", source_text(&rows[best].0))));
                l.add_css_class("heading");
                l.set_wrap(true);
                l.set_xalign(0.0);
                content.append(&l);
            }
            let grid = gtk::Grid::new();
            grid.set_column_spacing(24);
            grid.set_row_spacing(10);
            grid.set_column_homogeneous(false);
            let cell = |t: &str, dim: bool| {
                let l = gtk::Label::new(Some(t));
                l.set_xalign(0.0);
                l.set_wrap(true);
                l.set_max_width_chars(28);
                if dim {
                    l.add_css_class("dim-label");
                }
                l
            };
            let labels = ["", "Version", "Téléchargement", "Taille installée", "Mises à jour", "Isolement", "Installé"];
            for (r, t) in labels.iter().enumerate() {
                grid.attach(&cell(t, true), 0, r as i32, 1, 1);
            }
            let mut col = 1;
            for (o, d, wide) in &rows {
                let kind = o.backend.unwrap_or(BackendKind::Apt);
                let badge = crate::badge(&source_text(o), kind);
                badge.set_halign(gtk::Align::Start);
                grid.attach(&badge, col, 0, 1, 1);
                let v = d.as_ref().and_then(|d| d.available_version.clone().or(d.installed_version.clone())).unwrap_or_else(|| o.version.clone());
                grid.attach(&cell(&v, false), col, 1, 1, 1);
                grid.attach(&cell(&size(d.as_ref().and_then(|d| d.download_size)), false), col, 2, 1, 1);
                grid.attach(&cell(&size(d.as_ref().and_then(|d| d.installed_size)), false), col, 3, 1, 1);
                grid.attach(&cell(if kind == BackendKind::Flatpak { "Par l'éditeur, indépendamment du système" } else { "Avec le système (Debian)" }, false), col, 4, 1, 1);
                let iso = if kind == BackendKind::Flatpak {
                    if *wide > 0 { format!("Bac à sable, {wide} accès large(s)") } else { "Bac à sable".to_string() }
                } else {
                    "Aucun : accès complet au système".to_string()
                };
                grid.attach(&cell(&iso, false), col, 5, 1, 1);
                grid.attach(&cell(if o.installed { "✓ Oui" } else { "—" }, false), col, 6, 1, 1);
                if !o.installed {
                    let b = gtk::Button::with_label("Installer");
                    b.add_css_class("suggested-action");
                    b.set_halign(gtk::Align::Start);
                    let (app2, p, dlg2) = (app.clone(), o.clone(), dlg.clone());
                    b.connect_clicked(move |_| {
                        dlg2.close();
                        app2.install(p.clone());
                    });
                    grid.attach(&b, col, 7, 1, 1);
                }
                col += 1;
            }
            if let Some(h) = &hub {
                let l = gtk::Label::new(Some("AppImage"));
                l.add_css_class("noos-badge");
                l.add_css_class("system");
                l.set_halign(gtk::Align::Start);
                grid.attach(&l, col, 0, 1, 1);
                grid.attach(&cell("Voir sur AppImageHub", true), col, 1, 1, 1);
                grid.attach(&cell("—", true), col, 2, 1, 1);
                grid.attach(&cell("—", true), col, 3, 1, 1);
                grid.attach(&cell("Selon l'AppImage (intégrées ou manuelles)", false), col, 4, 1, 1);
                grid.attach(&cell("Aucun : accès complet à vos fichiers", false), col, 5, 1, 1);
                grid.attach(&cell("—", false), col, 6, 1, 1);
                let link = gtk::LinkButton::with_label(h.download.as_deref().unwrap_or(&h.page), "Télécharger…");
                link.set_halign(gtk::Align::Start);
                grid.attach(&link, col, 7, 1, 1);
            }
            content.append(&grid);
            let note = gtk::Label::new(Some(
                "Flatpak : isolé, souvent plus récent, mises à jour indépendantes. Paquet Debian : intégré au système, \
                 plus léger quand ses bibliothèques sont déjà installées. AppImage : un seul fichier portable, sans isolement.",
            ));
            note.set_wrap(true);
            note.set_xalign(0.0);
            note.add_css_class("dim-label");
            note.add_css_class("caption");
            content.append(&note);
        });
    }

    /// Place occupée et données de l'utilisateur d'un logiciel installé.
    fn data_group(self: &Rc<Self>, pkg: &Package) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        g.set_title("Place occupée");
        let kind = pkg.backend.unwrap_or(BackendKind::Apt);
        let id = pkg.id.clone();
        let task = background(move || noos_pkg::space::sizes(&[(kind, id.clone())]).remove(&noos_pkg::space::key(kind, &id)).unwrap_or_default());
        let (app, g2) = (self.clone(), g.clone());
        glib::spawn_future_local(async move {
            let sz = task.await;
            g2.add(&row("Logiciel", &size(Some(sz.installed))));
            let dirs: Vec<String> = sz.data_dirs.iter().map(|d| d.display().to_string().replace(&glib::home_dir().display().to_string(), "~")).collect();
            let r = row("Vos données", &if dirs.is_empty() { "Aucun dossier de données trouvé".to_string() } else { format!("{} · {}", glib::format_size(sz.data), dirs.join(", ")) });
            r.set_subtitle_lines(3);
            if let Some(first) = sz.data_dirs.first().cloned() {
                let open = gtk::Button::from_icon_name("folder-open-symbolic");
                open.add_css_class("flat");
                open.set_valign(gtk::Align::Center);
                open.set_tooltip_text(Some("Ouvrir le dossier"));
                open.connect_clicked(move |_| {
                    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&first).uri(), None::<&gio::AppLaunchContext>);
                });
                r.add_suffix(&open);
                let clear = gtk::Button::with_label("Effacer…");
                clear.set_valign(gtk::Align::Center);
                let (app2, dirs2) = (app.clone(), sz.data_dirs.clone());
                clear.connect_clicked(move |b| {
                    let (dirs3, b2, toasts) = (dirs2.clone(), b.clone(), app2.toasts.clone());
                    app2.confirm(
                        "Effacer les données de l'application ?",
                        "Réglages, comptes et fichiers internes de l'application sont mis à la corbeille (récupérables). Elle repartira comme au premier lancement.",
                        "Mettre à la corbeille",
                        true,
                        move || {
                            let plan = noos_pkg::space::clear_data_plan(&dirs3);
                            let task = background(move || plan.run(&|_| {}).is_ok());
                            let (b3, toasts) = (b2.clone(), toasts.clone());
                            glib::spawn_future_local(async move {
                                if task.await {
                                    b3.set_sensitive(false);
                                    toasts.add_toast(adap::Toast::new("Données mises à la corbeille"));
                                }
                            });
                        },
                    );
                });
                r.add_suffix(&clear);
            }
            g2.add(&r);
        });
        g
    }

    /// Permissions d'une appli Flatpak : lecture seule avant l'installation, interrupteurs
    /// (`flatpak override --user`) ensuite.
    fn permissions_group(self: &Rc<Self>, pkg: &Package) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        g.set_title("Accès accordés");
        g.set_description(Some(if pkg.installed {
            "Ce que l'application peut faire hors de son bac à sable. Retirer un accès peut l'empêcher de fonctionner."
        } else {
            "Ce que l'application pourra faire hors de son bac à sable une fois installée."
        }));
        let wait = row("Lecture des permissions…", "");
        g.add(&wait);
        let (id, installed, scope, origin) = (pkg.id.clone(), pkg.installed, pkg.scope.unwrap_or(noos_pkg::Scope::System), if pkg.origin.is_empty() { "flathub".to_string() } else { pkg.origin.clone() });
        let task = background(move || {
            let list = noos_pkg::permissions::of_app(&id, installed, scope, &origin).map_err(|e| format!("{e:#}"));
            (list, installed && noos_pkg::permissions::has_overrides(&id))
        });
        let (app, g2, pkg) = (self.clone(), g.clone(), pkg.clone());
        glib::spawn_future_local(async move {
            let (list, overridden) = task.await;
            g2.remove(&wait);
            let list = match list {
                Ok(l) => l,
                Err(e) => {
                    g2.add(&row("Permissions indisponibles", &e));
                    return;
                }
            };
            if list.is_empty() {
                g2.add(&row("Aucun accès particulier", "L'application reste entièrement dans son bac à sable"));
            }
            for p in list {
                let r = row(&p.label, &p.detail);
                let icon = match p.level {
                    2 => "dialog-warning-symbolic",
                    1 => "dialog-information-symbolic",
                    _ => "emblem-ok-symbolic",
                };
                let img = gtk::Image::from_icon_name(icon);
                if p.level == 2 {
                    img.add_css_class("warning");
                }
                r.add_prefix(&img);
                if pkg.installed {
                    let sw = gtk::Switch::new();
                    sw.set_active(p.enabled);
                    sw.set_valign(gtk::Align::Center);
                    let (app, id) = (app.clone(), pkg.id.clone());
                    sw.connect_state_set(move |_, on| {
                        let plan = noos_pkg::permissions::override_plan(&id, &p, on);
                        let task = background(move || plan.run(&|_| {}).map_err(|e| format!("{e:#}")));
                        let toasts = app.toasts.clone();
                        glib::spawn_future_local(async move {
                            match task.await {
                                Ok(()) => toasts.add_toast(adap::Toast::new(if on { "Accès accordé (au prochain lancement)" } else { "Accès retiré (au prochain lancement)" })),
                                Err(e) => toasts.add_toast(adap::Toast::new(&e)),
                            }
                        });
                        glib::Propagation::Proceed
                    });
                    r.add_suffix(&sw);
                    r.set_activatable_widget(Some(&sw));
                }
                g2.add(&r);
            }
            if overridden {
                let b = gtk::Button::with_label("Rétablir");
                b.add_css_class("flat");
                b.set_tooltip_text(Some("Revenir aux permissions d'origine de l'application"));
                let (app, id) = (app.clone(), pkg.id.clone());
                b.connect_clicked(move |b| {
                    let plan = noos_pkg::permissions::reset_plan(&id);
                    let task = background(move || plan.run(&|_| {}).is_ok());
                    let (toasts, b) = (app.toasts.clone(), b.clone());
                    glib::spawn_future_local(async move {
                        if task.await {
                            b.set_visible(false);
                            toasts.add_toast(adap::Toast::new("Permissions d'origine rétablies (rouvrez la fiche)"));
                        }
                    });
                });
                g2.set_header_suffix(Some(&b));
            }
        });
        g
    }
}

/// Icône de la fiche : thème (AppStream `stock`), cache local, médias distants, sinon générique.
fn details_icon(pkg: &Package, d: &Details) -> gtk::Image {
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
    let img = if let Some(name) = d.icon_name.as_deref().filter(|n| theme.has_icon(n)) {
        gtk::Image::from_icon_name(name)
    } else if let Some(f) = &d.icon_file {
        gtk::Image::from_file(f)
    } else if let Some(url) = &d.icon_url {
        let img = crate::icon_for(pkg, 96);
        async_image(&img, url);
        img
    } else {
        crate::icon_for(pkg, 96)
    };
    img.set_pixel_size(96);
    img.set_valign(gtk::Align::Start);
    img
}

/// Remplit le groupe « simulation » : dépendances ajoutées, mises à jour, retraits.
fn fill_effect(group: &adap::PreferencesGroup, pkg: &Package, effect: Result<Effect, String>) {
    let flatpak = pkg.backend == Some(BackendKind::Flatpak);
    match effect {
        Err(e) => {
            let r = row(if pkg.installed { "Suppression impossible" } else { "Installation impossible" }, &e);
            r.set_subtitle_lines(6);
            r.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            group.add(&r);
        }
        Ok(e) => {
            if e.extra.is_empty() && e.upgraded.is_empty() && e.removed.is_empty() {
                let (title, sub) = match (pkg.installed, flatpak) {
                    (true, true) => ("Rien d'autre", "Les runtimes partagés restent installés ; les données dans ~/.var/app sont conservées"),
                    (true, false) => ("Rien d'autre", "Seul ce paquet sera supprimé"),
                    (false, true) => ("Rien d'autre à télécharger", "Le runtime nécessaire est déjà installé"),
                    (false, false) => ("Rien d'autre", "Toutes ses dépendances sont déjà installées"),
                };
                let r = row(title, sub);
                r.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
                group.add(&r);
            }
            for (list, title, icon, what) in [
                (&e.extra, if flatpak { "Runtime installé avec lui" } else { "Dépendances installées avec lui" }, "list-add-symbolic", ""),
                (&e.upgraded, "Paquets mis à jour au passage", "software-update-available-symbolic", ""),
                (&e.removed, if pkg.installed { "Paquets qui en dépendent, retirés aussi" } else { "Paquets retirés (conflit)" }, "dialog-warning-symbolic", "⚠ "),
            ] {
                if list.is_empty() {
                    continue;
                }
                let x = adap::ExpanderRow::new();
                x.set_title(&format!("{what}{title} : {}", list.len()));
                x.set_subtitle(&short_list(list, 5));
                x.add_prefix(&gtk::Image::from_icon_name(icon));
                for name in list.iter().take(300) {
                    let r = adap::ActionRow::new();
                    r.set_use_markup(false);
                    r.set_title(name);
                    x.add_row(&r);
                }
                group.add(&x);
            }
        }
    }
}

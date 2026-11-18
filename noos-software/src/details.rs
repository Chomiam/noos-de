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

        // ── Simulation ──
        let sim = adap::PreferencesGroup::new();
        sim.set_title(if pkg.installed { "Ce que la suppression retirera" } else { "Ce que l'installation ajoutera" });
        let waiting = row("Simulation en cours…", "Sans mot de passe, rien n'est modifié");
        let sp = gtk::Spinner::new();
        sp.start();
        waiting.add_suffix(&sp);
        sim.add(&waiting);
        content.append(&sim);

        // ── Informations ──
        let info = adap::PreferencesGroup::new();
        info.set_title("Informations");
        let update = matches!((&d.installed_version, &d.available_version), (Some(i), Some(a)) if i != a && !a.is_empty());
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

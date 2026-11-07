//! noos-software — « Logiciels » de NOOS.
//!
//! Applications Flatpak (Flathub, Fedora Flatpaks) et paquets Fedora (dnf). Quand un logiciel
//! existe des deux façons, on choisit au moment d'installer.
//! Toute la logique de paquets est dans la bibliothèque `noos-pkg` ; ce fichier n'est que
//! l'interface GTK4 + libAdapta.

mod explore;
mod flathub;
mod ops;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_pkg::{
    BackendKind, Package, Preset, Scope, SystemInfo, Update,
    catalog::{self, Software},
    sources::{self, Channel, Source, State},
};
use ops::{Backends, Ops, background};

struct App {
    window: adap::ApplicationWindow,
    backends: Backends,
    ops: Rc<Ops>,
    toasts: adap::ToastOverlay,
    sys: SystemInfo,

    search: gtk::SearchEntry,
    source: gtk::DropDown,
    results: gtk::ListBox,
    explore_stack: gtk::Stack,
    search_gen: Cell<u64>,
    found: RefCell<Vec<(usize, Package)>>,
    /// Paquet dnf → identifiant AppStream (rapprochement avec les Flatpak).
    appstream: RefCell<HashMap<String, String>>,
    explore_home: gtk::Box,
    collection_box: gtk::Box,
    dynamic_css: gtk::CssProvider,
    home_built: Cell<bool>,

    installed_box: gtk::Box,
    installed_filter: gtk::SearchEntry,
    installed: RefCell<Vec<(usize, Vec<Package>, Option<String>)>>,

    updates_box: gtk::Box,
    updates_page: adap::ViewStackPage,

    repos_box: gtk::Box,
}

// ───────────────────────────── Utilitaires d'interface ─────────────────────────────

fn page_container() -> (gtk::ScrolledWindow, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let clamp = adap::Clamp::builder().maximum_size(920).child(&content).build();
    (gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build(), content)
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn badge(text: &str, kind: BackendKind) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("noos-badge");
    l.add_css_class(match kind {
        BackendKind::Flatpak => "flatpak",
        BackendKind::Dnf => "system",
    });
    l.set_valign(gtk::Align::Center);
    l
}

/// Nom lisible d'un dépôt d'origine (« flathub » → « Flathub »).
fn origin_label(p: &Package) -> String {
    match (p.backend, p.origin.as_str()) {
        (Some(BackendKind::Flatpak), "flathub") => "Flathub".into(),
        (Some(BackendKind::Flatpak), "fedora") => "Fedora".into(),
        (_, o) if o.starts_with("rpmfusion") => "RPM Fusion".into(),
        (_, "fedora" | "updates" | "updates-testing") => "Fedora".into(),
        (_, o) if o.starts_with("copr:") => format!("COPR {}", o.rsplit(':').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/")),
        (_, "@System" | "") => String::new(),
        (_, o) => o.to_string(),
    }
}

/// Texte court de la source d'un paquet : « Flatpak · Flathub », « dnf · RPM Fusion ».
fn source_text(p: &Package) -> String {
    let base = match p.backend.unwrap_or(BackendKind::Flatpak) {
        BackendKind::Flatpak => "Flatpak",
        BackendKind::Dnf => "dnf",
    };
    let origin = origin_label(p);
    let scope = if p.scope == Some(Scope::User) { " · votre compte" } else { "" };
    if origin.is_empty() { format!("{base}{scope}") } else { format!("{base} · {origin}{scope}") }
}

/// Description d'une façon d'installer, pour le choix dnf / Flatpak.
fn offer_explanation(p: &Package) -> &'static str {
    match p.backend {
        Some(BackendKind::Flatpak) => "Application isolée (bac à sable), mise à jour par son éditeur",
        _ => "Paquet du système, intégré et mis à jour avec Fedora",
    }
}

/// Icône : thème d'icônes, puis données AppStream de Flatpak, sinon icône générique.
fn icon_for(p: &Package, size: i32) -> gtk::Image {
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
    let img = if theme.has_icon(&p.id) {
        gtk::Image::from_icon_name(&p.id)
    } else if let Some(path) = flatpak_appstream_icon(p) {
        gtk::Image::from_file(path)
    } else if p.backend != Some(BackendKind::Flatpak) && theme.has_icon(&p.name) {
        gtk::Image::from_icon_name(&p.name)
    } else if p.backend == Some(BackendKind::Flatpak) {
        gtk::Image::from_icon_name("application-x-executable")
    } else {
        gtk::Image::from_icon_name("package-x-generic")
    };
    img.set_pixel_size(size);
    img
}

fn flatpak_appstream_icon(p: &Package) -> Option<std::path::PathBuf> {
    if p.backend != Some(BackendKind::Flatpak) || p.origin.is_empty() {
        return None;
    }
    let arch = std::env::consts::ARCH;
    let user = glib::user_data_dir().join("flatpak");
    let system = std::path::PathBuf::from("/var/lib/flatpak");
    let roots: Vec<std::path::PathBuf> = match p.scope {
        Some(Scope::System) => vec![system, user],
        _ => vec![user, system],
    };
    for root in roots {
        for size in ["128x128", "64x64"] {
            let path = root.join("appstream").join(&p.origin).join(arch).join("active/icons").join(size).join(format!("{}.png", p.id));
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn status_page(icon: &str, title: &str, desc: &str) -> adap::StatusPage {
    adap::StatusPage::builder().icon_name(icon).title(title).description(desc).build()
}

fn loading(text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.set_valign(gtk::Align::Center);
    b.set_vexpand(true);
    let s = gtk::Spinner::new();
    s.set_size_request(32, 32);
    s.start();
    b.append(&s);
    let l = gtk::Label::new(Some(text));
    l.add_css_class("dim-label");
    b.append(&l);
    b
}

pub(crate) fn row(title: &str, subtitle: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.set_title_lines(1);
    r.set_subtitle_lines(2);
    r
}

impl App {
    fn kind(&self, idx: usize) -> BackendKind {
        self.backends[idx].kind()
    }

    fn confirm(self: &Rc<Self>, title: &str, body: &str, action: &str, destructive: bool, f: impl Fn() + 'static) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance(
            "ok",
            if destructive { adap::ResponseAppearance::Destructive } else { adap::ResponseAppearance::Suggested },
        );
        d.set_default_response(Some("cancel"));
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f();
            }
        });
        d.present(Some(&self.window));
    }

    // ───────────────────────── Installer / supprimer ─────────────────────────

    fn backend_index(&self, kind: BackendKind) -> Option<usize> {
        self.backends.iter().position(|b| b.kind() == kind)
    }

    pub(crate) fn install(self: &Rc<Self>, pkg: Package) {
        let Some(idx) = pkg.backend.and_then(|k| self.backend_index(k)) else {
            self.toasts.add_toast(adap::Toast::new("Gestionnaire de paquets indisponible"));
            return;
        };
        let app = self.clone();
        let p = pkg.clone();
        self.ops.enqueue(
            format!("Installation de {} ({})", pkg.name, source_text(&pkg)),
            idx,
            move |b| Ok(b.install(&[p])),
            move |_| app.after_change(),
        );
    }

    pub(crate) fn remove(self: &Rc<Self>, pkg: Package) {
        let Some(idx) = pkg.backend.and_then(|k| self.backend_index(k)) else { return };
        let app = self.clone();
        let name = pkg.name.clone();
        self.confirm(
            &format!("Supprimer {name} ?"),
            if pkg.backend == Some(BackendKind::Flatpak) {
                "L'application sera désinstallée. Ses données dans ~/.var/app sont conservées."
            } else {
                "Le paquet et les dépendances devenues inutiles seront supprimés."
            },
            "Supprimer",
            true,
            move || {
                let p = pkg.clone();
                let app2 = app.clone();
                app.ops.enqueue(format!("Suppression de {}", p.name.clone()), idx, move |b| Ok(b.remove(&[p])), move |_| app2.after_change());
            },
        );
    }

    fn after_change(self: &Rc<Self>) {
        if !self.search.text().trim().is_empty() {
            self.run_search();
        }
        self.load_installed();
        self.load_updates();
    }

    /// Bouton d'action d'un logiciel : « Supprimer » s'il est installé ; sinon « Installer »,
    /// avec un menu de choix (Flatpak Flathub, Flatpak Fedora, paquet dnf…) s'il y a plusieurs
    /// façons de l'installer.
    pub(crate) fn action_button(self: &Rc<Self>, sw: &Software) -> gtk::Widget {
        let installed: Vec<Package> = sw.installed().cloned().collect();
        if let Some(p) = installed.first() {
            let b = gtk::Button::with_label("Supprimer");
            b.add_css_class("destructive-action");
            b.add_css_class("pill");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&format!("Installé : {}", source_text(p))));
            let (app, pkg) = (self.clone(), p.clone());
            b.connect_clicked(move |_| app.remove(pkg.clone()));
            return b.upcast();
        }
        if sw.offers.len() == 1 {
            let b = gtk::Button::with_label("Installer");
            b.add_css_class("suggested-action");
            b.add_css_class("pill");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&source_text(&sw.offers[0])));
            let (app, pkg) = (self.clone(), sw.offers[0].clone());
            b.connect_clicked(move |_| app.install(pkg.clone()));
            return b.upcast();
        }
        let menu = gtk::MenuButton::new();
        let content = adap::ButtonContent::builder().label("Installer").icon_name("pan-down-symbolic").build();
        menu.set_child(Some(&content));
        menu.add_css_class("suggested-action");
        menu.add_css_class("pill");
        menu.set_valign(gtk::Align::Center);
        menu.set_tooltip_text(Some("Choisir comment installer ce logiciel"));
        let pop = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_margin_top(6);
        list.set_margin_bottom(6);
        list.set_margin_start(6);
        list.set_margin_end(6);
        let title = gtk::Label::new(Some("Installer depuis…"));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_margin_start(6);
        title.set_margin_bottom(4);
        list.append(&title);
        for offer in &sw.offers {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("noos-offer");
            let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&badge(&source_text(offer), offer.backend.unwrap_or(BackendKind::Flatpak)));
            if !offer.version.is_empty() {
                let v = gtk::Label::new(Some(&offer.version));
                v.add_css_class("dim-label");
                v.set_max_width_chars(24);
                v.set_ellipsize(gtk::pango::EllipsizeMode::End);
                line.append(&v);
            }
            col.append(&line);
            let why = gtk::Label::new(Some(offer_explanation(offer)));
            why.add_css_class("caption");
            why.add_css_class("dim-label");
            why.set_xalign(0.0);
            col.append(&why);
            b.set_child(Some(&col));
            let (app, pkg, pop2) = (self.clone(), offer.clone(), pop.clone());
            b.connect_clicked(move |_| {
                pop2.popdown();
                app.install(pkg.clone());
            });
            list.append(&b);
        }
        pop.set_child(Some(&list));
        menu.set_popover(Some(&pop));
        menu.upcast()
    }

    pub(crate) fn software_row(self: &Rc<Self>, sw: &Software) -> adap::ActionRow {
        let r = row(&sw.name, &sw.summary);
        let main = sw.installed().next().unwrap_or(&sw.offers[0]);
        r.add_prefix(&icon_for(main, 48));
        let info = gtk::Box::new(gtk::Orientation::Vertical, 4);
        info.set_valign(gtk::Align::Center);
        // Une pastille par source ; celle qui est installée d'abord.
        let mut shown: Vec<String> = Vec::new();
        for offer in sw.installed().chain(sw.offers.iter().filter(|o| !o.installed)) {
            let text = source_text(offer);
            if shown.contains(&text) {
                continue;
            }
            shown.push(text.clone());
            let b = badge(&if offer.installed { format!("✓ {text}") } else { text }, offer.backend.unwrap_or(BackendKind::Flatpak));
            b.set_halign(gtk::Align::End);
            info.append(&b);
        }
        if sw.offers.len() == 1 && !main.version.is_empty() {
            let v = gtk::Label::new(Some(&main.version));
            v.add_css_class("dim-label");
            v.add_css_class("caption");
            v.set_halign(gtk::Align::End);
            v.set_max_width_chars(22);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            info.append(&v);
        }
        r.add_suffix(&info);
        r.add_suffix(&self.action_button(sw));
        r
    }

    /// Ligne d'un seul paquet (pages Installées).
    fn package_row(self: &Rc<Self>, p: &Package) -> adap::ActionRow {
        self.software_row(&Software { key: p.id.clone(), name: p.name.clone(), summary: p.summary.clone(), offers: vec![p.clone()] })
    }

    // ───────────────────────── Explorer ─────────────────────────

    fn selected_backends(&self) -> Vec<usize> {
        match self.source.selected() {
            0 => (0..self.backends.len()).collect(),
            n => vec![(n - 1) as usize],
        }
    }

    fn run_search(self: &Rc<Self>) {
        let query = self.search.text().trim().to_string();
        let generation = self.search_gen.get() + 1;
        self.search_gen.set(generation);
        self.found.borrow_mut().clear();
        if query.chars().count() < 2 {
            self.explore_stack.set_visible_child_name("home");
            return;
        }
        self.explore_stack.set_visible_child_name("loading");
        let targets = self.selected_backends();
        let pending = Rc::new(Cell::new(targets.len()));
        for idx in targets {
            let backends = self.backends.clone();
            let q = query.clone();
            // dnf : on récupère aussi la table AppStream (une fois, mise en cache par le backend).
            let task = background(move || {
                let res = backends[idx].search(&q);
                let ids = (backends[idx].kind() == BackendKind::Dnf).then(|| backends[idx].appstream_ids());
                (res, ids)
            });
            let (app, pending, query) = (self.clone(), pending.clone(), query.clone());
            glib::spawn_future_local(async move {
                let (res, ids) = task.await;
                if app.search_gen.get() != generation {
                    return;
                }
                if let Some(ids) = ids {
                    *app.appstream.borrow_mut() = ids;
                }
                pending.set(pending.get() - 1);
                match res {
                    Ok(pkgs) => app.found.borrow_mut().extend(pkgs.into_iter().map(|p| (idx, p))),
                    Err(e) => app.toasts.add_toast(adap::Toast::new(&format!("{} : {e}", app.kind(idx).label()))),
                }
                // On attend les deux sources : sinon une ligne « dnf » se transformerait en
                // ligne « dnf + Flatpak » sous les yeux de l'utilisateur.
                if pending.get() == 0 {
                    app.render_results(&query);
                }
            });
        }
    }

    fn render_results(self: &Rc<Self>, query: &str) {
        let mut pkgs: Vec<Package> = self.found.borrow().iter().map(|(_, p)| p.clone()).collect();
        if pkgs.is_empty() {
            self.explore_stack.set_visible_child_name("none");
            return;
        }
        // Classement global : pertinence, puis Flatpak avant les paquets dnf à égalité.
        pkgs.sort_by_key(|p| p.backend != Some(BackendKind::Flatpak));
        noos_pkg::rank(query, &mut pkgs);
        let software = catalog::merge(pkgs, &self.appstream.borrow());
        self.results.remove_all();
        for sw in software.iter().take(120) {
            self.results.append(&self.software_row(sw));
        }
        self.explore_stack.set_visible_child_name("results");
    }

    // ───────────────────────── Installées ─────────────────────────

    fn load_installed(self: &Rc<Self>) {
        clear(&self.installed_box);
        self.installed_box.append(&loading("Chargement des logiciels installés…"));
        let backends = self.backends.clone();
        let task = background(move || {
            (0..backends.len())
                .map(|i| match backends[i].installed() {
                    Ok(list) => (i, list, None),
                    Err(e) => (i, vec![], Some(format!("{e:#}"))),
                })
                .collect::<Vec<_>>()
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            *app.installed.borrow_mut() = task.await;
            app.render_installed();
            // La vitrine a besoin de savoir ce qui est déjà installé.
            if !app.home_built.replace(true) {
                app.build_home();
            }
        });
    }

    fn render_installed(self: &Rc<Self>) {
        clear(&self.installed_box);
        self.installed_box.append(&self.installed_filter);
        let filter = self.installed_filter.text().to_lowercase();
        for (idx, list, err) in self.installed.borrow().iter() {
            let kind = self.kind(*idx);
            let shown: Vec<&Package> = list
                .iter()
                .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.id.to_lowercase().contains(&filter))
                .collect();
            let g = adap::PreferencesGroup::new();
            g.set_title(&format!("{} — {}", kind.label(), list.len()));
            g.set_description(Some(match kind {
                BackendKind::Flatpak => "Applications isolées (Flathub, Fedora Flatpaks)",
                BackendKind::Dnf => "Paquets installés explicitement (hors dépendances)",
            }));
            if let Some(e) = err {
                g.add(&row("Erreur", e));
            } else if shown.is_empty() {
                g.add(&row(if list.is_empty() { "Aucun" } else { "Aucun résultat pour ce filtre" }, ""));
            }
            for p in shown.iter().take(300) {
                g.add(&self.package_row(p));
            }
            if shown.len() > 300 {
                g.add(&row(&format!("… et {} autres (affinez le filtre)", shown.len() - 300), ""));
            }
            self.installed_box.append(&g);
        }
    }

    // ───────────────────────── Mises à jour ─────────────────────────

    fn load_updates(self: &Rc<Self>) {
        clear(&self.updates_box);
        self.updates_box.append(&loading("Recherche des mises à jour…"));
        let backends = self.backends.clone();
        let task = background(move || {
            (0..backends.len())
                .map(|i| (i, backends[i].updates().map_err(|e| format!("{e:#}"))))
                .collect::<Vec<_>>()
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let results = task.await;
            app.render_updates(results);
        });
    }

    fn render_updates(self: &Rc<Self>, results: Vec<(usize, Result<Vec<Update>, String>)>) {
        clear(&self.updates_box);
        let total: usize = results.iter().filter_map(|(_, r)| r.as_ref().ok()).map(|u| u.len()).sum();
        self.updates_page.set_badge_number(total as u32);
        self.updates_page.set_needs_attention(total > 0);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = gtk::Label::new(Some(&match total {
            0 => "Tout est à jour".to_string(),
            1 => "1 mise à jour disponible".to_string(),
            n => format!("{n} mises à jour disponibles"),
        }));
        title.add_css_class("title-2");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let refresh = gtk::Button::with_label("Actualiser les sources");
        refresh.set_tooltip_text(Some("Télécharge les métadonnées des dépôts (peut demander le mot de passe)"));
        {
            let app = self.clone();
            refresh.connect_clicked(move |_| app.refresh_sources());
        }
        header.append(&refresh);
        if total > 0 {
            let all = gtk::Button::with_label("Tout mettre à jour");
            all.add_css_class("suggested-action");
            let (app, idxs) = (self.clone(), results.iter().filter(|(_, r)| r.as_ref().is_ok_and(|u| !u.is_empty())).map(|(i, _)| *i).collect::<Vec<_>>());
            all.connect_clicked(move |_| {
                for i in &idxs {
                    app.upgrade(*i);
                }
            });
            header.append(&all);
        }
        self.updates_box.append(&header);

        if total == 0 && results.iter().all(|(_, r)| r.is_ok()) {
            self.updates_box.append(&status_page("emblem-ok-symbolic", "Votre système est à jour", "Aucune mise à jour pour l'instant."));
        }

        for (idx, res) in results {
            let kind = self.kind(idx);
            let g = adap::PreferencesGroup::new();
            g.set_title(kind.label());
            match res {
                Err(e) => g.add(&row("Impossible de vérifier les mises à jour", &e)),
                Ok(list) if list.is_empty() => continue,
                Ok(list) => {
                    g.set_description(Some(&format!("{} mise(s) à jour", list.len())));
                    let b = gtk::Button::with_label("Mettre à jour");
                    b.add_css_class("flat");
                    let app = self.clone();
                    b.connect_clicked(move |_| app.upgrade(idx));
                    g.set_header_suffix(Some(&b));
                    for u in list.iter().take(200) {
                        let sub = match (u.old_version.is_empty(), u.new_version.is_empty()) {
                            (false, false) if u.old_version != u.new_version => format!("{} → {}", u.old_version, u.new_version),
                            (_, false) => u.new_version.clone(),
                            _ => String::new(),
                        };
                        let sub = if u.package.summary.is_empty() { sub } else { format!("{sub}\n{}", u.package.summary) };
                        let r = row(&u.package.name, sub.trim());
                        r.add_prefix(&icon_for(&u.package, 32));
                        r.add_suffix(&badge(&source_text(&u.package), kind));
                        g.add(&r);
                    }
                    if list.len() > 200 {
                        g.add(&row(&format!("… et {} autres", list.len() - 200), ""));
                    }
                }
            }
            self.updates_box.append(&g);
        }
    }

    fn upgrade(self: &Rc<Self>, idx: usize) {
        let kind = self.kind(idx);
        let app = self.clone();
        self.ops.enqueue(format!("Mise à jour : {}", kind.label()), idx, |b| Ok(b.upgrade_all()), move |_| app.load_updates());
    }

    fn refresh_sources(self: &Rc<Self>) {
        let n = self.backends.len();
        for idx in 0..n {
            let app = self.clone();
            let last = idx + 1 == n;
            self.ops.enqueue(
                format!("Actualisation : {}", self.kind(idx).label()),
                idx,
                |b| Ok(b.refresh()),
                move |_| {
                    if last {
                        app.load_updates();
                    }
                },
            );
        }
    }

    // ───────────────────────── Sources ─────────────────────────

    fn load_repos(self: &Rc<Self>) {
        clear(&self.repos_box);
        self.repos_box.append(&loading("Lecture des sources…"));
        let backends = self.backends.clone();
        let version = self.sys.major();
        let task = background(move || {
            (0..backends.len())
                .map(|i| {
                    let b = &backends[i];
                    let sources = b.repos().map_err(|e| format!("{e:#}")).map(|repos| match b.kind() {
                        BackendKind::Dnf => sources::group_dnf(&repos, &version),
                        BackendKind::Flatpak => sources::group_flatpak(&repos),
                    });
                    (i, sources, b.presets())
                })
                .collect::<Vec<_>>()
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let res = task.await;
            app.render_repos(res);
        });
    }

    fn render_repos(self: &Rc<Self>, data: Vec<(usize, Result<Vec<Source>, String>, Vec<Preset>)>) {
        clear(&self.repos_box);
        let t = gtk::Label::new(Some("Sources de logiciels"));
        t.add_css_class("title-2");
        t.set_xalign(0.0);
        self.repos_box.append(&t);
        let intro = gtk::Label::new(Some(
            "Les sources actives sont celles dans lesquelles Logiciels cherche et installe. \
             Un logiciel disponible dans plusieurs sources vous laisse choisir au moment de l'installer.",
        ));
        intro.set_wrap(true);
        intro.set_xalign(0.0);
        intro.add_css_class("dim-label");
        self.repos_box.append(&intro);

        // Sources stables, par gestionnaire.
        let mut testing: Vec<(usize, Source)> = Vec::new();
        let mut extra: Vec<(usize, Preset)> = Vec::new();
        for (idx, res, presets) in data {
            let kind = self.kind(idx);
            let g = adap::PreferencesGroup::new();
            g.set_title(kind.label());
            g.set_description(Some(match kind {
                BackendKind::Dnf => "Paquets RPM installés avec le système (dnf)",
                BackendKind::Flatpak => "Applications isolées, indépendantes du système",
            }));
            match res {
                Err(e) => g.add(&row("Lecture impossible", &e)),
                Ok(list) => {
                    for src in list {
                        if src.channel == Channel::Testing {
                            testing.push((idx, src));
                        } else {
                            g.add(&self.source_row(idx, src));
                        }
                    }
                }
            }
            self.repos_box.append(&g);
            // Préréglages qui ne sont pas déjà des lignes « non installée » (COPR, URL…).
            extra.extend(
                presets
                    .into_iter()
                    .filter(|p| p.argument.is_some() || matches!(p.id, "flathub" | "flathub-beta" | "flathub-unfilter"))
                    .map(|p| (idx, p)),
            );
        }

        // Sources de test, à part.
        if !testing.is_empty() {
            let g = adap::PreferencesGroup::new();
            g.set_title("Sources de test");
            g.set_description(Some(
                "Versions en cours de validation : plus récentes, mais elles peuvent casser des logiciels \
                 ou le système. À n'activer que pour essayer un correctif précis.",
            ));
            for (idx, src) in testing {
                g.add(&self.source_row(idx, src));
            }
            self.repos_box.append(&g);
        }

        if !extra.is_empty() {
            let g = adap::PreferencesGroup::new();
            g.set_title("Ajouter une source");
            for (idx, p) in extra {
                let r = row(p.name, p.description);
                r.add_prefix(&gtk::Image::from_icon_name(match p.backend {
                    BackendKind::Flatpak => "application-x-addon-symbolic",
                    BackendKind::Dnf => "package-x-generic-symbolic",
                }));
                let b = gtk::Button::with_label(if p.argument.is_some() { "Ajouter…" } else { "Ajouter" });
                b.set_valign(gtk::Align::Center);
                let app = self.clone();
                b.connect_clicked(move |_| app.add_preset(idx, p.clone()));
                r.add_suffix(&b);
                g.add(&r);
            }
            self.repos_box.append(&g);
        }
    }

    fn source_row(self: &Rc<Self>, idx: usize, src: Source) -> adap::ActionRow {
        let r = row(&src.name, &src.description);
        r.set_subtitle_lines(3);
        let ids: Vec<&str> = src.repos.iter().map(|r| r.id.as_str()).collect();
        if !ids.is_empty() {
            r.set_tooltip_text(Some(&format!("Dépôts : {}", ids.join(", "))));
        }
        // État lisible d'un coup d'œil.
        let (text, class) = match src.state {
            State::Enabled => ("Active", "on"),
            State::Partial => ("Partielle", "partial"),
            State::Disabled => ("Inactive", "off"),
            State::Missing => ("Non installée", "off"),
        };
        let state = gtk::Label::new(Some(text));
        state.add_css_class("noos-state");
        state.add_css_class(class);
        state.set_valign(gtk::Align::Center);
        r.add_suffix(&state);

        if src.state == State::Missing {
            if let Some(preset) = src.preset {
                let b = gtk::Button::with_label("Ajouter");
                b.add_css_class("suggested-action");
                b.set_valign(gtk::Align::Center);
                let (app, name) = (self.clone(), src.name.clone());
                b.connect_clicked(move |_| {
                    let app2 = app.clone();
                    app.ops.enqueue(
                        format!("Ajout de {name}"),
                        idx,
                        move |b| b.add_preset(preset, None),
                        move |_| {
                            app2.load_repos();
                            app2.load_updates();
                        },
                    );
                });
                r.add_suffix(&b);
            }
            return r;
        }

        if !src.builtin && !src.essential {
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            del.set_tooltip_text(Some("Supprimer cette source"));
            let (app, s2) = (self.clone(), src.clone());
            del.connect_clicked(move |_| {
                let (app2, s3) = (app.clone(), s2.clone());
                app.confirm(
                    &format!("Supprimer « {} » ?", s2.name),
                    "Les logiciels déjà installés depuis cette source restent installés mais ne recevront plus de mises à jour.",
                    "Supprimer",
                    true,
                    move || {
                        let (app3, repo) = (app2.clone(), s3.repos[0].clone());
                        app2.ops.enqueue(format!("Suppression de {}", s3.name), idx, move |b| b.remove_repo(&repo), move |_| app3.load_repos());
                    },
                );
            });
            r.add_suffix(&del);
        }

        let sw = gtk::Switch::new();
        sw.set_active(src.enabled());
        sw.set_valign(gtk::Align::Center);
        if src.essential {
            sw.set_sensitive(false);
            sw.set_tooltip_text(Some("Indispensable au système : ne peut pas être désactivée"));
        }
        let app = self.clone();
        sw.connect_state_set(move |sw, on| {
            let apply = {
                let (app, src) = (app.clone(), src.clone());
                move || {
                    let (app2, repos) = (app.clone(), src.repos.clone());
                    let verb = if on { "Activation" } else { "Désactivation" };
                    app.ops.enqueue(format!("{verb} de {}", src.name), idx, move |b| b.set_repos_enabled(&repos, on), move |_| {
                        app2.load_repos();
                        app2.load_updates();
                    });
                }
            };
            if on && src.channel == Channel::Testing {
                // Activer une source de test mérite une confirmation.
                let sw2 = sw.clone();
                let d = adap::AlertDialog::new(
                    Some(&format!("Activer « {} » ?", src.name)),
                    Some("Les mises à jour de test n'ont pas fini d'être validées : elles peuvent casser des logiciels ou le démarrage. Désactivez cette source dès que vous n'en avez plus besoin."),
                );
                d.add_responses(&[("cancel", "Annuler"), ("ok", "Activer")]);
                d.set_response_appearance("ok", adap::ResponseAppearance::Destructive);
                d.set_default_response(Some("cancel"));
                d.connect_response(None, move |_, resp| {
                    if resp == "ok" {
                        apply();
                    } else {
                        sw2.set_active(false);
                    }
                });
                d.present(Some(&app.window));
            } else {
                apply();
            }
            glib::Propagation::Proceed
        });
        r.add_suffix(&sw);
        r
    }

    fn add_preset(self: &Rc<Self>, idx: usize, p: Preset) {
        let run = {
            let app = self.clone();
            let p = p.clone();
            move |arg: Option<String>| {
                let app2 = app.clone();
                let id = p.id;
                app.ops.enqueue(
                    format!("Ajout du dépôt {}", p.name),
                    idx,
                    move |b| b.add_preset(id, arg.as_deref()),
                    move |_| {
                        app2.load_repos();
                        app2.load_updates();
                    },
                );
            }
        };
        let Some(placeholder) = p.argument else {
            run(None);
            return;
        };
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some(placeholder));
        entry.set_activates_default(true);
        let d = adap::AlertDialog::new(Some(&format!("Ajouter : {}", p.name)), Some(p.description));
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", "Annuler"), ("add", "Ajouter")]);
        d.set_response_appearance("add", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("add"));
        let e2 = entry.clone();
        d.connect_response(None, move |_, r| {
            if r == "add" && !e2.text().trim().is_empty() {
                run(Some(e2.text().trim().to_string()));
            }
        });
        d.present(Some(&self.window));
        glib::idle_add_local_once(move || {
            entry.grab_focus();
        });
    }
}

// ───────────────────────────── Construction de la fenêtre ─────────────────────────────

const CSS: &str = r#"
.noos-badge { border-radius: 99px; padding: 2px 10px; font-size: 0.8em; font-weight: 600; }
.noos-badge.flatpak { background: alpha(@accent_bg_color, 0.18); color: @accent_color; }
.noos-badge.system { background: alpha(@success_bg_color, 0.18); color: @success_color; }
.noos-offer { padding: 8px 10px; border-radius: 10px; }
.noos-state { border-radius: 99px; padding: 2px 10px; font-size: 0.8em; font-weight: 600; }
.noos-state.on { background: alpha(@success_bg_color, 0.20); color: @success_color; }
.noos-state.partial { background: alpha(@warning_bg_color, 0.22); color: @warning_color; }
.noos-state.off { background: alpha(@window_fg_color, 0.08); color: alpha(@window_fg_color, 0.65); }
.noos-banner { border-radius: 18px; padding: 28px 32px; min-height: 250px; }
.noos-banner .title-1 { font-size: 2.2em; }
.banner-shot { border-radius: 12px; box-shadow: 0 6px 24px rgba(0,0,0,0.30); }
.noos-tile { padding: 12px 14px; border-radius: 12px; background-color: @card_bg_color; box-shadow: 0 0 0 1px alpha(@card_fg_color, 0.06); }
.noos-tile:hover { background-color: mix(@card_bg_color, @accent_bg_color, 0.08); }
.noos-category { padding: 14px 16px; border-radius: 12px; background-color: @card_bg_color; font-weight: 600; }
.noos-category:hover { background-color: mix(@card_bg_color, @accent_bg_color, 0.12); }
.noos-search { min-height: 44px; font-size: 1.1em; border-radius: 99px; }
"#;

fn build(gapp: &adap::Application) {
    let sys = SystemInfo::detect();
    let backends: Backends = Arc::new(noos_pkg::detect());

    let window = adap::ApplicationWindow::builder()
        .application(gapp)
        .title("Logiciels")
        .default_width(1060)
        .default_height(760)
        .build();
    let toasts = adap::ToastOverlay::new();
    let ops = Ops::new(backends.clone(), &toasts);
    ops.set_window(&window);

    let css = gtk::CssProvider::new();
    css.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    // ── Explorer ──
    let (explore_scroll, explore_content) = page_container();
    let search_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Applications, outils, jeux, paquets…"));
    search.add_css_class("noos-search");
    search.set_hexpand(true);
    let mut labels = vec!["Flatpak et dnf".to_string()];
    labels.extend(backends.iter().map(|b| b.kind().label().to_string()));
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let source = gtk::DropDown::from_strings(&refs);
    source.set_valign(gtk::Align::Center);
    search_line.append(&search);
    search_line.append(&source);
    explore_content.append(&search_line);

    let explore_stack = gtk::Stack::new();
    explore_stack.set_vhomogeneous(false);
    let explore_home = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let collection_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let home = explore_home.clone();
    explore_stack.add_named(&home, Some("home"));
    explore_stack.add_named(&loading("Recherche dans les applications Flatpak et les paquets Fedora…"), Some("loading"));
    let results = gtk::ListBox::new();
    results.add_css_class("boxed-list");
    results.set_selection_mode(gtk::SelectionMode::None);
    results.set_valign(gtk::Align::Start);
    explore_stack.add_named(&results, Some("results"));
    explore_stack.add_named(&status_page("edit-find-symbolic", "Aucun résultat", "Essayez un autre nom ou une autre source."), Some("none"));
    explore_stack.add_named(&collection_box, Some("collection"));
    explore_content.append(&explore_stack);

    // ── Installées ──
    let (installed_scroll, installed_box) = page_container();
    let installed_filter = gtk::SearchEntry::new();
    installed_filter.set_placeholder_text(Some("Filtrer les logiciels installés"));

    // ── Mises à jour ──
    let (updates_view, updates_box) = page_container();

    // ── Sources ──
    let (repos_scroll, repos_box) = page_container();

    let stack = adap::ViewStack::new();
    stack.add_titled_with_icon(&explore_scroll, Some("explore"), "Explorer", "system-search-symbolic");
    stack.add_titled_with_icon(&installed_scroll, Some("installed"), "Installées", "view-list-bullet-symbolic");
    let updates_page = stack.add_titled_with_icon(&updates_view, Some("updates"), "Mises à jour", "software-update-available-symbolic");
    stack.add_titled_with_icon(&repos_scroll, Some("repos"), "Sources", "network-server-symbolic");

    let header = adap::HeaderBar::new();
    let switcher = adap::ViewSwitcher::builder().stack(&stack).policy(adap::ViewSwitcherPolicy::Wide).build();
    header.set_title_widget(Some(&switcher));
    let log_btn = gtk::Button::from_icon_name("utilities-terminal-symbolic");
    log_btn.set_tooltip_text(Some("Journal des opérations"));
    header.pack_end(&log_btn);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&stack);
    content.append(&ops.bar);
    toasts.set_child(Some(&content));
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&toasts));
    window.set_content(Some(&tv));

    let app = Rc::new(App {
        window: window.clone(),
        backends,
        ops: ops.clone(),
        toasts,
        sys,
        search: search.clone(),
        source: source.clone(),
        results,
        explore_stack,
        search_gen: Cell::new(0),
        found: RefCell::new(vec![]),
        appstream: RefCell::new(HashMap::new()),
        explore_home,
        collection_box,
        dynamic_css: gtk::CssProvider::new(),
        home_built: Cell::new(false),
        installed_box,
        installed_filter: installed_filter.clone(),
        installed: RefCell::new(vec![]),
        updates_box,
        updates_page,
        repos_box,
    });

    // ── Signaux ──
    {
        let ops = ops.clone();
        log_btn.connect_clicked(move |_| ops.show_log());
    }
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &app.dynamic_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    {
        let app2 = app.clone();
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        search.connect_search_changed(move |_| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (app3, pending2) = (app2.clone(), pending.clone());
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(350), move || {
                pending2.borrow_mut().take();
                app3.run_search();
            }));
        });
    }
    {
        let app2 = app.clone();
        source.connect_selected_notify(move |_| app2.run_search());
    }
    {
        let app2 = app.clone();
        installed_filter.connect_search_changed(move |_| app2.render_installed());
    }
    // Chargement paresseux des pages.
    {
        let app2 = app.clone();
        let loaded = Rc::new(RefCell::new(std::collections::HashSet::<String>::new()));
        stack.connect_visible_child_name_notify(move |s| {
            let name = s.visible_child_name().map(|n| n.to_string()).unwrap_or_default();
            if !loaded.borrow_mut().insert(name.clone()) {
                return;
            }
            match name.as_str() {
                "repos" => app2.load_repos(),
                _ => {}
            }
        });
    }
    {
        // Ctrl+F : recherche ; Ctrl+L : journal.
        let keys = gtk::EventControllerKey::new();
        let (search2, stack2, ops2) = (search.clone(), stack.clone(), ops.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                if key == gdk::Key::f {
                    stack2.set_visible_child_name("explore");
                    search2.grab_focus();
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::l {
                    ops2.show_log();
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    if app.backends.is_empty() {
        app.toasts.add_toast(adap::Toast::new("Ni dnf ni Flatpak ne sont disponibles sur ce système"));
    }
    app.load_installed();
    app.load_updates();
    // Table AppStream dnf (≈ 1 s) : rapproche paquets Fedora et applis Flathub dès la vitrine.
    if let Some(idx) = app.backend_index(BackendKind::Dnf) {
        let backends = app.backends.clone();
        let task = background(move || backends[idx].appstream_ids());
        let app2 = app.clone();
        glib::spawn_future_local(async move {
            *app2.appstream.borrow_mut() = task.await;
        });
    }
    window.present();
    search.grab_focus();
    // Tests visuels (aperçu sans souris) : page d'ouverture et recherche initiale.
    if let Ok(page) = std::env::var("NOOS_SOFTWARE_PAGE") {
        stack.set_visible_child_name(&page);
    }
    if let Ok(q) = std::env::var("NOOS_SOFTWARE_SEARCH") {
        search.set_text(&q);
    }
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id("org.noos.Software")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
    });
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
        } else {
            build(app);
        }
    });
    app.connect_command_line(|app, _| {
        app.activate();
        glib::ExitCode::SUCCESS
    });
    app.run()
}

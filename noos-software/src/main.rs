//! noos-software — « Logiciels » de NOOS.
//!
//! Applications Flatpak (Flathub) et paquets Debian (apt). Quand un logiciel
//! existe des deux façons, on choisit au moment d'installer. Ouvre aussi les fichiers
//! `.AppImage` (installation dans ~/Applications + menu Démarrer, voir `appimage.rs`) et
//! `.deb` (installation par apt avec ses dépendances, voir `deb.rs`).
//! Toute la logique de paquets est dans la bibliothèque `noos-pkg` ; ce fichier n'est que
//! l'interface GTK4 + libAdapta.

mod appimage;
mod cleanup;
mod deb;
mod details;
mod explore;
mod flathub;
mod history;
mod mirrors;
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
    /// Paquet apt → identifiant AppStream (rapprochement avec les Flatpak).
    appstream: RefCell<HashMap<String, String>>,
    explore_home: gtk::Box,
    collection_box: gtk::Box,
    dynamic_css: gtk::CssProvider,
    home_built: Cell<bool>,

    installed_box: gtk::Box,
    installed_filter: gtk::SearchEntry,
    /// Tri de la page Installées : 0 nom, 1 place occupée.
    installed_sort: gtk::DropDown,
    /// Nouvelles versions des AppImage installées (chemin → version disponible).
    appimage_updates: RefCell<HashMap<std::path::PathBuf, noos_pkg::appimage_update::Available>>,
    /// Groupe « AppImage » de la page Mises à jour (rempli en arrière-plan).
    appimage_box: gtk::Box,
    /// Place occupée (clé `noos_pkg::space::key`), mesurée en arrière-plan.
    sizes: RefCell<HashMap<String, noos_pkg::space::Sizes>>,
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

pub(crate) fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

pub(crate) fn badge(text: &str, kind: BackendKind) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("noos-badge");
    l.add_css_class(match kind {
        BackendKind::Flatpak => "flatpak",
        BackendKind::Apt => "system",
    });
    l.set_valign(gtk::Align::Center);
    l
}

/// Nom lisible d'un dépôt d'origine (« flathub » → « Flathub »).
fn origin_label(p: &Package) -> String {
    match (p.backend, p.origin.as_str()) {
        (Some(BackendKind::Flatpak), "flathub") => "Flathub".into(),
        (_, "debian") => "Debian".into(),
        (_, "debian-experimental") => "Debian experimental".into(),
        (_, "noos") => "NOOS".into(),
        (_, "") => String::new(),
        (_, o) => o.to_string(),
    }
}

/// Texte court de la source d'un paquet : « Flatpak · Flathub », « apt · Debian ».
pub(crate) fn source_text(p: &Package) -> String {
    let base = match p.backend.unwrap_or(BackendKind::Flatpak) {
        BackendKind::Flatpak => "Flatpak",
        BackendKind::Apt => "apt",
    };
    let origin = origin_label(p);
    let scope = if p.scope == Some(Scope::User) { " · votre compte" } else { "" };
    if origin.is_empty() { format!("{base}{scope}") } else { format!("{base} · {origin}{scope}") }
}

/// Description d'une façon d'installer, pour le choix apt / Flatpak.
pub(crate) fn offer_explanation(p: &Package) -> &'static str {
    match p.backend {
        Some(BackendKind::Flatpak) => "Application isolée (bac à sable), mise à jour par son éditeur",
        _ => "Paquet du système, intégré et mis à jour avec Debian",
    }
}

/// Icône : thème d'icônes, puis données AppStream de Flatpak, sinon icône générique.
pub(crate) fn icon_for(p: &Package, size: i32) -> gtk::Image {
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

pub(crate) fn loading(text: &str) -> gtk::Box {
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
        self.ops.enqueue_install(format!("Installation de {} ({})", pkg.name, source_text(&pkg)), idx, vec![pkg.clone()], move |_| app.after_change());
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
                "Le paquet sera supprimé (la fiche dit si d'autres paquets en dépendent). Ses dépendances devenues inutiles se retirent ensuite depuis Installées › Faire de la place."
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
    /// avec un menu de choix (Flatpak Flathub, paquet Debian…) s'il y a plusieurs
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
        // Clic sur la ligne (hors bouton) : fiche détaillée.
        r.set_activatable(true);
        r.set_tooltip_text(Some("Afficher la fiche"));
        let (app, sw) = (self.clone(), sw.clone());
        r.connect_activated(move |_| app.show_software(&sw, None));
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
            // apt : on récupère aussi la table AppStream (une fois, mise en cache par le backend).
            let task = background(move || {
                let res = backends[idx].search(&q);
                let ids = (backends[idx].kind() == BackendKind::Apt).then(|| backends[idx].appstream_ids());
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
                // On attend les deux sources : sinon une ligne « apt » se transformerait en
                // ligne « apt + Flatpak » sous les yeux de l'utilisateur.
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
        // Classement global : pertinence, puis Flatpak avant les paquets apt à égalité.
        pkgs.sort_by_key(|p| p.backend != Some(BackendKind::Flatpak));
        noos_pkg::rank(query, &mut pkgs);
        let software = catalog::merge(pkgs, &self.appstream.borrow());
        self.results.remove_all();
        for entry in catalog::organize(query, software).iter().take(120) {
            self.results.append(&self.software_row(&entry.software));
            if !entry.related.is_empty() {
                // Aide, traductions, données… repliées sous le logiciel.
                let x = adap::ExpanderRow::new();
                x.set_title(&match entry.related.len() {
                    1 => "1 paquet associé".to_string(),
                    n => format!("{n} paquets associés"),
                });
                let names: Vec<&str> = entry.related.iter().map(|s| s.name.as_str()).collect();
                x.set_subtitle(&if names.len() > 4 { format!("{} et {} autres", names[..4].join(", "), names.len() - 4) } else { names.join(", ") });
                x.add_prefix(&gtk::Image::from_icon_name("view-more-symbolic"));
                for r in entry.related.iter().take(200) {
                    x.add_row(&self.software_row(r));
                }
                self.results.append(&x);
            }
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
            app.load_sizes();
            // La vitrine a besoin de savoir ce qui est déjà installé.
            if !app.home_built.replace(true) {
                app.build_home();
            }
        });
    }

    /// Place occupée par chaque logiciel installé (taille + données), puis nouvel affichage.
    fn load_sizes(self: &Rc<Self>) {
        let items: Vec<(BackendKind, String)> =
            self.installed.borrow().iter().flat_map(|(i, list, _)| list.iter().map(move |p| (*i, p.id.clone()))).map(|(i, id)| (self.kind(i), id)).collect();
        let task = background(move || noos_pkg::space::sizes(&items));
        let app = self.clone();
        glib::spawn_future_local(async move {
            *app.sizes.borrow_mut() = task.await;
            app.render_installed();
        });
    }

    fn size_of(&self, p: &Package) -> Option<noos_pkg::space::Sizes> {
        self.sizes.borrow().get(&noos_pkg::space::key(p.backend.unwrap_or(BackendKind::Apt), &p.id)).cloned()
    }

    fn render_installed(self: &Rc<Self>) {
        // Le filtre et le tri restent dans la page : on les détache avant de vider.
        for w in [self.installed_filter.clone().upcast::<gtk::Widget>(), self.installed_sort.clone().upcast()] {
            if let Some(parent) = w.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
                parent.remove(&w);
            }
        }
        clear(&self.installed_box);
        self.installed_box.append(&self.cleanup_group());
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.append(&self.installed_filter);
        line.append(&self.installed_sort);
        self.installed_box.append(&line);
        let by_size = self.installed_sort.selected() == 1;
        let filter = self.installed_filter.text().to_lowercase();
        // AppImage en tête : peu nombreuses, et c'est là qu'on cherche juste après une installation.
        self.installed_box.append(&self.appimage_group(&filter));
        for (idx, list, err) in self.installed.borrow().iter() {
            let kind = self.kind(*idx);
            let mut shown: Vec<&Package> = list
                .iter()
                .filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter) || p.id.to_lowercase().contains(&filter))
                .collect();
            if by_size {
                shown.sort_by_key(|p| std::cmp::Reverse(self.size_of(p).map(|s| s.installed + s.data).unwrap_or(0)));
            }
            let g = adap::PreferencesGroup::new();
            g.set_title(&format!("{} — {}", kind.label(), list.len()));
            g.set_description(Some(match kind {
                BackendKind::Flatpak => "Applications isolées (Flathub)",
                BackendKind::Apt => "Paquets installés explicitement (hors dépendances)",
            }));
            if let Some(e) = err {
                g.add(&row("Erreur", e));
            } else if shown.is_empty() {
                g.add(&row(if list.is_empty() { "Aucun" } else { "Aucun résultat pour ce filtre" }, ""));
            }
            for p in shown.iter().take(300) {
                let r = self.package_row(p);
                if let Some(sz) = self.size_of(p) {
                    let mut t = glib::format_size(sz.installed).to_string();
                    if sz.data > 0 {
                        t.push_str(&format!(" · données {}", glib::format_size(sz.data)));
                    }
                    r.set_subtitle(&if p.summary.is_empty() { t } else { format!("{}\n{t}", p.summary) });
                }
                g.add(&r);
            }
            if shown.len() > 300 {
                g.add(&row(&format!("… et {} autres (affinez le filtre)", shown.len() - 300), ""));
            }
            self.installed_box.append(&g);
        }
    }

    // ───────────────────────── Mises à jour ─────────────────────────

    fn load_updates(self: &Rc<Self>) {
        self.search_updates(false);
        self.check_appimage_updates();
    }

    /// Recherche des mises à jour (sans mot de passe) ; `force` : les métadonnées des dépôts
    /// sont retéléchargées. Le résultat est enregistré pour l'icône de la barre des tâches.
    fn search_updates(self: &Rc<Self>, force: bool) {
        clear(&self.updates_box);
        self.updates_box.append(&loading(if force { "Recherche des mises à jour auprès des dépôts…" } else { "Recherche des mises à jour…" }));
        let backends = self.backends.clone();
        if std::env::var_os("NOOS_SOFTWARE_UPDATES_DEMO").is_some() {
            // Aperçu : mises à jour fictives, rien n'est enregistré.
            let demo = demo_updates(&backends);
            let task = background(move || backends.iter().find(|b| b.kind() == BackendKind::Apt).map(|b| b.appstream_ids()));
            let app = self.clone();
            glib::spawn_future_local(async move {
                if let Some(ids) = task.await {
                    *app.appstream.borrow_mut() = ids;
                }
                app.render_updates(demo);
            });
            return;
        }
        let task = background(move || {
            // Table AppStream (applications / composants du système) : déjà en cache le plus souvent.
            let ids = backends.iter().find(|b| b.kind() == BackendKind::Apt).map(|b| b.appstream_ids());
            let res: Vec<(usize, Result<Vec<Update>, String>)> = if force {
                noos_pkg::check_updates(&backends, true).into_iter().enumerate().map(|(i, (_, r))| (i, r.map_err(|e| format!("{e:#}")))).collect()
            } else {
                (0..backends.len()).map(|i| (i, backends[i].updates().map_err(|e| format!("{e:#}")))).collect()
            };
            let rec: Vec<_> = res.iter().map(|(i, r)| (backends[*i].kind(), r.clone())).collect();
            noos_pkg::record_updates(&rec);
            (res, ids)
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let (results, ids) = task.await;
            if let Some(ids) = ids {
                *app.appstream.borrow_mut() = ids;
            }
            app.render_updates(results);
        });
    }

    fn render_updates(self: &Rc<Self>, results: Vec<(usize, Result<Vec<Update>, String>)>) {
        if let Some(p) = self.appimage_box.parent() {
            p.downcast::<gtk::Box>().unwrap().remove(&self.appimage_box);
        }
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
        title.set_xalign(0.0);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        titles.append(&title);
        let checked = gtk::Label::new(Some(&format!("Dernière vérification : {}", noos_core::updates::ago(noos_core::updates::load().checked_at))));
        checked.add_css_class("dim-label");
        checked.set_xalign(0.0);
        titles.append(&checked);
        header.append(&titles);
        let check = gtk::Button::new();
        let ch = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        ch.append(&gtk::Image::from_icon_name("view-refresh-symbolic"));
        ch.append(&gtk::Label::new(Some("Rechercher des mises à jour")));
        check.set_child(Some(&ch));
        check.set_tooltip_text(Some("Interroge les dépôts (Debian, NOOS, Flatpak) maintenant, sans mot de passe"));
        check.set_valign(gtk::Align::Center);
        {
            let app = self.clone();
            check.connect_clicked(move |_| app.search_updates(true));
        }
        header.append(&check);
        if total > 0 {
            let all = gtk::Button::with_label("Tout mettre à jour");
            all.add_css_class("suggested-action");
            all.set_valign(gtk::Align::Center);
            let (app, idxs) = (self.clone(), results.iter().filter(|(_, r)| r.as_ref().is_ok_and(|u| !u.is_empty())).map(|(i, _)| *i).collect::<Vec<_>>());
            all.connect_clicked(move |_| {
                for i in &idxs {
                    app.upgrade(*i);
                }
            });
            header.append(&all);
        }
        self.updates_box.append(&header);
        self.updates_box.append(&self.appimage_box);

        // Mises à jour apt « au redémarrage » (installées avant le démarrage, rien d'ouvert).
        let apt_total: usize = results.iter().filter(|(i, _)| self.kind(*i) == BackendKind::Apt).filter_map(|(_, r)| r.as_ref().ok()).map(|u| u.len()).sum();
        if noos_pkg::offline::pending() {
            let g = adap::PreferencesGroup::new();
            let r = row("Installation programmée au prochain redémarrage", "Les paquets sont téléchargés : ils s'installeront avant l'ouverture de la session.");
            r.add_prefix(&gtk::Image::from_icon_name("system-reboot-symbolic"));
            let cancel = gtk::Button::with_label("Annuler");
            cancel.set_valign(gtk::Align::Center);
            let app = self.clone();
            cancel.connect_clicked(move |_| {
                let app2 = app.clone();
                app.ops.enqueue("Annulation de l'installation au redémarrage", 0, |_| Ok(noos_pkg::offline::cancel_plan()), move |_| app2.load_updates());
            });
            r.add_suffix(&cancel);
            let reboot = gtk::Button::with_label("Redémarrer");
            reboot.add_css_class("suggested-action");
            reboot.set_valign(gtk::Align::Center);
            reboot.connect_clicked(|_| {
                let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
            });
            r.add_suffix(&reboot);
            g.add(&r);
            self.updates_box.append(&g);
        } else if apt_total > 0 {
            let g = adap::PreferencesGroup::new();
            let r = row(
                "Installer au redémarrage",
                "Recommandé pour les mises à jour du système : téléchargement maintenant, installation au prochain démarrage, quand rien n'est ouvert.",
            );
            r.add_prefix(&gtk::Image::from_icon_name("system-reboot-symbolic"));
            let b = gtk::Button::with_label("Programmer");
            b.set_valign(gtk::Align::Center);
            if let Some(idx) = self.backend_index(BackendKind::Apt) {
                let app = self.clone();
                b.connect_clicked(move |_| {
                    let app2 = app.clone();
                    app.ops.enqueue("Téléchargement des mises à jour (installation au redémarrage)", idx, |_| Ok(noos_pkg::offline::schedule_plan()), move |ok| {
                        app2.load_updates();
                        if ok {
                            app2.confirm(
                                "Redémarrer maintenant ?",
                                "Les mises à jour sont téléchargées. Elles s'installeront au prochain démarrage de l'ordinateur.",
                                "Redémarrer",
                                false,
                                || {
                                    let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
                                },
                            );
                        }
                    });
                });
            }
            r.add_suffix(&b);
            g.add(&r);
            self.updates_box.append(&g);
        }
        if let Some((code, when)) = noos_pkg::offline::last_result() {
            let recent = glib::DateTime::now_utc().map(|n| n.to_unix() - when < 3 * 86400).unwrap_or(false);
            if recent && code != 0 {
                let g = adap::PreferencesGroup::new();
                let r = row("La dernière installation au redémarrage a échoué", "Journal : /var/log/noos-offline-update.log. Relancez la mise à jour depuis cette page.");
                r.add_prefix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
                g.add(&r);
                self.updates_box.append(&g);
            }
        }

        if total == 0 && results.iter().all(|(_, r)| r.is_ok()) {
            self.updates_box.append(&status_page("emblem-ok-symbolic", "Votre système est à jour", "Aucune mise à jour pour l'instant."));
        }

        // Noyau, libc, pilotes, compositeur… : redémarrage à prévoir.
        let apt_names: Vec<String> = results
            .iter()
            .filter(|(i, _)| self.kind(*i) == BackendKind::Apt)
            .filter_map(|(_, r)| r.as_ref().ok())
            .flatten()
            .map(|u| u.package.id.clone())
            .collect();
        if noos_pkg::needs_reboot(&apt_names.iter().map(String::as_str).collect::<Vec<_>>()) {
            let g = adap::PreferencesGroup::new();
            let r = row("Un redémarrage sera nécessaire", "Le noyau, des pilotes ou des composants essentiels du système sont mis à jour : redémarrez une fois l'installation terminée.");
            r.add_prefix(&gtk::Image::from_icon_name("system-reboot-symbolic"));
            g.add(&r);
            self.updates_box.append(&g);
        }

        for (idx, res) in results {
            let kind = self.kind(idx);
            let list = match res {
                Err(e) => {
                    let g = adap::PreferencesGroup::new();
                    g.set_title(kind.label());
                    g.add(&row("Impossible de vérifier les mises à jour", &e));
                    self.updates_box.append(&g);
                    continue;
                }
                Ok(list) if list.is_empty() => continue,
                Ok(list) => list,
            };
            // Applications (AppStream, Flatpak) d'un côté, composants du système de l'autre.
            let (apps, system): (Vec<Update>, Vec<Update>) =
                list.into_iter().partition(|u| kind == BackendKind::Flatpak || self.appstream.borrow().contains_key(&u.package.id));
            let short = match kind {
                BackendKind::Flatpak => "Flatpak",
                BackendKind::Apt => "paquets Debian",
            };
            for (title, desc, items) in [
                (format!("Applications ({short})"), "Logiciels que vous utilisez", apps),
                (format!("Composants du système ({short})"), "Bibliothèques, pilotes, outils : à garder à jour pour la sécurité", system),
            ] {
                if items.is_empty() {
                    continue;
                }
                let g = adap::PreferencesGroup::new();
                g.set_title(&title);
                g.set_description(Some(&format!("{desc} · {} mise(s) à jour", items.len())));
                let checks: Rc<RefCell<Vec<(gtk::CheckButton, Package)>>> = Rc::new(RefCell::new(Vec::new()));
                let b = gtk::Button::with_label("Mettre à jour");
                b.add_css_class("flat");
                b.set_tooltip_text(Some("Met à jour les éléments cochés de ce groupe"));
                {
                    let (app, checks) = (self.clone(), checks.clone());
                    b.connect_clicked(move |_| {
                        let pkgs: Vec<Package> = checks.borrow().iter().filter(|(c, _)| c.is_active()).map(|(_, p)| p.clone()).collect();
                        app.upgrade_selected(idx, pkgs);
                    });
                }
                g.set_header_suffix(Some(&b));
                for u in items.iter().take(300) {
                    let sub = match (u.old_version.is_empty(), u.new_version.is_empty()) {
                        (false, false) if u.old_version != u.new_version => format!("{} → {}", u.old_version, u.new_version),
                        (_, false) => u.new_version.clone(),
                        _ => String::new(),
                    };
                    let sub = if u.package.summary.is_empty() { sub } else { format!("{sub}\n{}", u.package.summary) };
                    let r = row(&u.package.name, sub.trim());
                    let check = gtk::CheckButton::new();
                    check.set_active(true);
                    check.set_valign(gtk::Align::Center);
                    check.set_tooltip_text(Some("Inclure dans la mise à jour"));
                    r.add_prefix(&icon_for(&u.package, 32));
                    r.add_prefix(&check);
                    r.add_suffix(&badge(&source_text(&u.package), kind));
                    // Clic : fiche du logiciel (nouveautés de la version).
                    r.set_activatable(true);
                    let (app, p) = (self.clone(), u.package.clone());
                    r.connect_activated(move |_| {
                        app.show_software(&Software { key: p.id.clone(), name: p.name.clone(), summary: p.summary.clone(), offers: vec![p.clone()] }, None)
                    });
                    checks.borrow_mut().push((check, u.package.clone()));
                    g.add(&r);
                }
                if items.len() > 300 {
                    g.add(&row(&format!("… et {} autres", items.len() - 300), ""));
                }
                self.updates_box.append(&g);
            }
        }
    }

    /// Met à jour une sélection de paquets (rien de coché : rien à faire).
    fn upgrade_selected(self: &Rc<Self>, idx: usize, pkgs: Vec<Package>) {
        if pkgs.is_empty() {
            self.toasts.add_toast(adap::Toast::new("Cochez au moins une mise à jour"));
            return;
        }
        let app = self.clone();
        let title = match pkgs.len() {
            1 => format!("Mise à jour de {}", pkgs[0].name),
            n => format!("Mise à jour de {n} éléments ({})", self.kind(idx).label()),
        };
        self.ops.enqueue(title, idx, move |b| Ok(b.upgrade(&pkgs)), move |_| app.load_updates());
    }

    fn upgrade(self: &Rc<Self>, idx: usize) {
        let kind = self.kind(idx);
        let app = self.clone();
        self.ops.enqueue(format!("Mise à jour : {}", kind.label()), idx, |b| Ok(b.upgrade_all()), move |_| app.load_updates());
    }


    fn load_repos(self: &Rc<Self>) {
        clear(&self.repos_box);
        self.repos_box.append(&loading("Lecture des sources…"));
        let backends = self.backends.clone();
        let task = background(move || {
            (0..backends.len())
                .map(|i| {
                    let b = &backends[i];
                    let sources = b.repos().map_err(|e| format!("{e:#}")).map(|repos| match b.kind() {
                        BackendKind::Apt => sources::group_apt(&repos),
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
            let description = match kind {
                BackendKind::Apt => format!("Paquets .deb installés avec le système (Debian {}, apt)", self.sys.release_label()),
                BackendKind::Flatpak => "Applications isolées, indépendantes du système".to_string(),
            };
            g.set_description(Some(description.as_str()));
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
            if kind == BackendKind::Apt {
                self.repos_box.append(&self.mirror_group());
            }
            // Préréglages qui ne sont pas déjà des lignes « non installée » (dépôt, URL, i386…).
            extra.extend(
                presets
                    .into_iter()
                    .filter(|p| p.argument.is_some() || matches!(p.id, "flathub" | "flathub-beta" | "flathub-unfilter" | "i386"))
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
                    BackendKind::Apt => "package-x-generic-symbolic",
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
.noos-shot { border-radius: 10px; }
.noos-sheet { background-color: @dialog_bg_color; }
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
    let mut labels = vec!["Flatpak et apt".to_string()];
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
    explore_stack.add_named(&loading("Recherche dans les applications Flatpak et les paquets Debian…"), Some("loading"));
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
    installed_filter.set_hexpand(true);
    let installed_sort = gtk::DropDown::from_strings(&["Trier par nom", "Trier par place occupée"]);
    installed_sort.set_valign(gtk::Align::Center);

    // ── Mises à jour ──
    let (updates_view, updates_box) = page_container();

    // ── Sources ──
    let (repos_scroll, repos_box) = page_container();

    let stack = adap::ViewStack::new();
    STACK.with(|s| *s.borrow_mut() = Some(stack.clone()));
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
    let history_btn = gtk::Button::from_icon_name("document-open-recent-symbolic");
    history_btn.set_tooltip_text(Some("Historique des opérations"));
    header.pack_end(&history_btn);
    header.pack_end(&ops.queue_button);

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
        installed_sort: installed_sort.clone(),
        sizes: RefCell::new(HashMap::new()),
        appimage_updates: RefCell::new(HashMap::new()),
        appimage_box: gtk::Box::new(gtk::Orientation::Vertical, 0),
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
    {
        let app2 = app.clone();
        history_btn.connect_clicked(move |_| app2.show_history());
    }
    {
        // Fermer pendant une installation : la fenêtre se cache, le travail continue.
        let ops = ops.clone();
        window.connect_close_request(move |w| if ops.keep_running(w) { glib::Propagation::Stop } else { glib::Propagation::Proceed });
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
    {
        let app2 = app.clone();
        installed_sort.connect_selected_notify(move |_| app2.render_installed());
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
        app.toasts.add_toast(adap::Toast::new("Ni apt ni Flatpak ne sont disponibles sur ce système"));
    }
    app.load_installed();
    app.load_updates();
    // Table AppStream apt (DEP-11, ≈ 1 s) : rapproche paquets Debian et applis Flathub dès la vitrine.
    if let Some(idx) = app.backend_index(BackendKind::Apt) {
        let backends = app.backends.clone();
        let task = background(move || backends[idx].appstream_ids());
        let app2 = app.clone();
        glib::spawn_future_local(async move {
            *app2.appstream.borrow_mut() = task.await;
        });
    }
    APP.with(|a| *a.borrow_mut() = Some(app.clone()));
    window.present();
    search.grab_focus();
    // Tests visuels (aperçu sans souris) : page d'ouverture et recherche initiale.
    if let Ok(page) = std::env::var("NOOS_SOFTWARE_PAGE") {
        stack.set_visible_child_name(&page);
    }
    if let Ok(q) = std::env::var("NOOS_SOFTWARE_SEARCH") {
        search.set_text(&q);
    }
    if std::env::var_os("NOOS_SOFTWARE_QUEUE_DEMO").is_some() && !app.backends.is_empty() {
        // Aperçu : une opération fictive en cours et deux en attente, file ouverte.
        let long = if std::env::var("NOOS_SOFTWARE_QUEUE_DEMO").is_ok_and(|v| v == "close") { "4" } else { "60" };
        for (t, secs) in [("Installation de Éditeur d’image GIMP (apt · Debian)", long), ("Installation de Fluxer (Flatpak · Flathub)", "3"), ("Mise à jour de 3 éléments", "3")] {
            app.ops.enqueue(t, 0, move |_| Ok(noos_pkg::Plan::new().user(&["sleep", secs])), |_| {});
        }
        let ops = app.ops.clone();
        glib::timeout_add_local_once(Duration::from_millis(2500), move || ops.queue_button.popup());
        if std::env::var("NOOS_SOFTWARE_QUEUE_DEMO").is_ok_and(|v| v == "close") {
            // Fenêtre fermée pendant les opérations : elle doit se cacher, l'appli finir son travail.
            let w = window.clone();
            glib::timeout_add_local_once(Duration::from_millis(3000), move || w.close());
        }
    }
    if let (Some(px), None) = (std::env::var("NOOS_SOFTWARE_SCROLL").ok().and_then(|v| v.parse::<f64>().ok()), std::env::var_os("NOOS_SOFTWARE_DETAILS")) {
        // Aperçu : page Explorer défilée (packs, catégories Debian…).
        let sc = explore_scroll.clone();
        glib::timeout_add_local_once(Duration::from_millis(6000), move || sc.vadjustment().set_value(px));
    }
    if let Ok(c) = std::env::var("NOOS_SOFTWARE_COLLECTION") {
        // Aperçu : une collection ou une catégorie (`category/Graphics`).
        app.show_collection(&c, c.trim_start_matches("category/"));
    }
    if std::env::var_os("NOOS_SOFTWARE_MIRRORS").is_some() {
        // Aperçu : tableau des miroirs ouvert (NOOS_SOFTWARE_MIRRORS_TEST=1 : mesures lancées).
        stack.set_visible_child_name("repos");
        app.show_mirrors();
    }
    if std::env::var("NOOS_SOFTWARE_SORT").is_ok_and(|v| v == "size") {
        installed_sort.set_selected(1);
    }
    if std::env::var_os("NOOS_SOFTWARE_HISTORY").is_some() {
        app.show_history();
    }
    if let Ok(v) = std::env::var("NOOS_SOFTWARE_CLEANUP") {
        // Aperçu : « Faire de la place » ouvert (`demo` : éléments fictifs).
        stack.set_visible_child_name("installed");
        app.show_cleanup(v == "demo");
    }
    if let Ok(spec) = std::env::var("NOOS_SOFTWARE_DETAILS") {
        app.open_details_spec(&spec);
        if std::env::var_os("NOOS_SOFTWARE_COMPARE").is_some() {
            // Aperçu : comparaison ouverte par-dessus la fiche.
            let (app2, spec2) = (app.clone(), spec.clone());
            glib::timeout_add_local_once(Duration::from_millis(4000), move || {
                let backends = app2.backends.clone();
                let q = spec2.split('@').next().unwrap_or(&spec2).to_string();
                let task = background(move || {
                    let mut found = Vec::new();
                    let mut ids = HashMap::new();
                    for b in backends.iter() {
                        found.extend(b.search(&q).unwrap_or_default());
                        if b.kind() == BackendKind::Apt {
                            ids = b.appstream_ids();
                        }
                    }
                    catalog::merge(found, &ids)
                });
                glib::spawn_future_local(async move {
                    if let Some(sw) = task.await.into_iter().find(|s| s.offers.len() > 1) {
                        app2.show_compare(&sw);
                    }
                });
            });
        }
    }
}

/// Mises à jour fictives (`NOOS_SOFTWARE_UPDATES_DEMO=1`).
fn demo_updates(backends: &ops::Backends) -> Vec<(usize, Result<Vec<Update>, String>)> {
    let up = |kind: BackendKind, id: &str, name: &str, old: &str, new: &str, summary: &str| Update {
        old_version: old.into(),
        new_version: new.into(),
        package: Package {
            backend: Some(kind),
            id: id.into(),
            name: name.into(),
            version: new.into(),
            summary: summary.into(),
            origin: if kind == BackendKind::Flatpak { "flathub".into() } else { "debian".into() },
            installed: true,
            scope: (kind == BackendKind::Flatpak).then_some(Scope::System),
        },
    };
    backends
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let list = match b.kind() {
                BackendKind::Apt => vec![
                    up(BackendKind::Apt, "gimp", "Éditeur d’image GIMP", "3.2.6-1", "3.2.8-1+b1", "programme de manipulation d’image GIMP"),
                    up(BackendKind::Apt, "simple-scan", "Numériseur de documents", "49.1-1", "50.0-2", "utilitaire de numérisation d'images simple"),
                    up(BackendKind::Apt, "linux-image-amd64", "linux-image-amd64", "6.17.8-1", "6.17.9-1", "Linux pour PC 64 bits (métapaquet)"),
                    up(BackendKind::Apt, "libc6", "libc6", "2.42-3", "2.43-1", "Bibliothèque C GNU : bibliothèques partagées"),
                    up(BackendKind::Apt, "htop", "htop", "3.5.2-1", "3.5.3-1", "outil interactif de visualisation de processus"),
                ],
                BackendKind::Flatpak => vec![up(BackendKind::Flatpak, "app.fluxer.Fluxer", "Fluxer", "2026.1110", "2026.1114.183423", "A chat app that puts you first")],
            };
            (i, Ok(list))
        })
        .collect()
}

thread_local! {
    static STACK: std::cell::RefCell<Option<adap::ViewStack>> = const { std::cell::RefCell::new(None) };
    static APP: std::cell::RefCell<Option<Rc<App>>> = const { std::cell::RefCell::new(None) };
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
        // Fenêtre cachée pendant une opération en arrière-plan : on la remontre.
        if let Some(w) = app.active_window().or_else(|| app.windows().into_iter().next()) {
            w.set_visible(true);
            w.present();
        } else {
            build(app);
        }
    });
    app.connect_command_line(|app, cmd| {
        // Fichiers ouverts (Explorateur : `noos-software %F`). AppImage déjà installées :
        // lancées sans ouvrir de fenêtre ; sinon fenêtre « Installer ». Paquets .deb : fiche
        // du paquet puis installation par apt.
        // Options : `--details <apt:gimp|flatpak:id|gimp>`, `--app <lanceur.desktop>` (menu
        // Démarrer, barre des tâches), `--uninstall` (avec --app / --details), `--updates` ;
        // liens `appstream://<id>` et fichiers `.flatpakref` : fiche du logiciel.
        let args: Vec<String> = cmd.arguments().iter().skip(1).map(|a| a.to_string_lossy().to_string()).collect();
        let mut spec: Option<String> = None;
        let mut uninstall = false;
        let mut appimage_page = false;
        let mut rest: Vec<String> = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--details" => spec = it.next().cloned(),
                "--app" => match it.next().and_then(|d| noos_pkg::owner::owner_of_desktop(d)) {
                    Some(noos_pkg::owner::Owner::AppImage(_)) => appimage_page = true,
                    Some(o) => spec = o.spec(),
                    None => eprintln!("noos-software : paquet du lanceur introuvable"),
                },
                "--uninstall" => uninstall = true,
                _ if a.starts_with("appstream:") => {
                    spec = Some(a.trim_start_matches("appstream:").trim_start_matches("//").trim_end_matches('/').to_string());
                }
                _ if a.ends_with(".flatpakref") => {
                    let text = std::fs::read_to_string(a.trim_start_matches("file://")).unwrap_or_default();
                    spec = noos_pkg::owner::desktop_key(&text.replace("[Flatpak Ref]", "[Desktop Entry]"), "Name").map(|n| format!("flatpak:{n}"));
                }
                _ if a.starts_with("--") => {}
                _ => rest.push(a.clone()),
            }
        }
        let paths: Vec<std::path::PathBuf> = rest.iter().filter_map(|a| cmd.create_file_for_arg(a).path()).collect();
        let (files, debs): (Vec<_>, Vec<_>) =
            paths.into_iter().filter(|p| noos_pkg::appimage::is_appimage(p) || noos_pkg::deb::is_deb(p)).partition(|p| noos_pkg::appimage::is_appimage(p));
        if !files.is_empty() && debs.is_empty() && app.active_window().is_none() {
            let installed: Vec<std::path::PathBuf> = noos_pkg::appimage::installed().into_iter().map(|i| i.path).collect();
            let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if files.iter().all(|f| installed.contains(&canon(f))) {
                for f in &files {
                    if let Err(e) = noos_pkg::appimage::run(f) {
                        eprintln!("noos-software : {e:#}");
                    }
                }
                return glib::ExitCode::SUCCESS;
            }
        }
        app.activate();
        APP.with(|a| {
            if let Some(a) = a.borrow().as_ref() {
                for f in &files {
                    a.open_appimage(f.clone());
                }
                for f in &debs {
                    a.open_deb(f.clone());
                }
            }
        });
        if let Some(spec) = &spec {
            APP.with(|a| {
                if let Some(a) = a.borrow().as_ref() {
                    a.open_details_spec_with(spec, uninstall);
                }
            });
        }
        if appimage_page {
            STACK.with(|s| {
                if let Some(s) = s.borrow().as_ref() {
                    s.set_visible_child_name("installed");
                }
            });
        }
        // `noos-software --updates` : page Mises à jour (icône de la barre des tâches).
        if cmd.arguments().iter().any(|a| a.to_str() == Some("--updates")) {
            STACK.with(|s| {
                if let Some(s) = s.borrow().as_ref() {
                    s.set_visible_child_name("updates");
                }
            });
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}

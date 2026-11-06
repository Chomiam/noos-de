//! noos-software — « Logiciels » de NOOS.
//!
//! Une seule application pour toutes les sources : Flatpak (Flathub…), rpm-ostree (Fedora
//! Atomic), dnf (Fedora, RHEL…), apt (Debian, Ubuntu…), pacman (Arch…) et l'AUR.
//! Toute la logique de paquets est dans la bibliothèque `noos-pkg` ; ce fichier n'est que
//! l'interface GTK4 + libAdapta.

mod explore;
mod flathub;
mod ops;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_pkg::{BackendKind, Package, Preset, Repo, Scope, SystemInfo, Update};
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
    explore_home: gtk::Box,
    collection_box: gtk::Box,
    dynamic_css: gtk::CssProvider,
    home_built: Cell<bool>,

    installed_box: gtk::Box,
    installed_filter: gtk::SearchEntry,
    installed: RefCell<Vec<(usize, Vec<Package>, Option<String>)>>,

    updates_box: gtk::Box,
    updates_page: adap::ViewStackPage,
    banner: adap::Banner,

    repos_box: gtk::Box,
    show_debug: gtk::CheckButton,
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
        BackendKind::Aur => "aur",
        _ => "system",
    });
    l.set_valign(gtk::Align::Center);
    l
}

fn source_text(p: &Package) -> String {
    let kind = p.backend.unwrap_or(BackendKind::Flatpak);
    let base = match kind {
        BackendKind::Flatpak => "Flatpak".to_string(),
        BackendKind::Aur => "AUR".to_string(),
        k => k.id().to_string(),
    };
    let scope = match p.scope {
        Some(Scope::User) => " · utilisateur",
        Some(Scope::System) => " · système",
        None => "",
    };
    if p.origin.is_empty() { format!("{base}{scope}") } else { format!("{base} · {}{scope}", p.origin) }
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

    fn install(self: &Rc<Self>, idx: usize, pkg: Package) {
        let kind = self.kind(idx);
        let go = {
            let app = self.clone();
            let pkg = pkg.clone();
            move || {
                let p = pkg.clone();
                let app2 = app.clone();
                app.ops.enqueue(
                    format!("Installation de {}", pkg.name),
                    idx,
                    move |b| Ok(b.install(&[p])),
                    move |ok| {
                        if ok && kind == BackendKind::RpmOstree {
                            app2.toasts.add_toast(adap::Toast::new("Installé : redémarrez pour utiliser ce paquet"));
                        }
                        app2.after_change();
                    },
                );
            }
        };
        match kind {
            BackendKind::Aur => self.confirm(
                "Installer depuis l'AUR ?",
                "Les paquets de l'AUR sont proposés par la communauté et ne sont pas vérifiés. Leur script de construction s'exécute sur votre machine : n'installez que des paquets de confiance.",
                "Installer",
                false,
                go,
            ),
            BackendKind::RpmOstree => self.confirm(
                &format!("Superposer {} au système ?", pkg.name),
                "Sur un système atomique, les paquets RPM sont ajoutés à l'image et ne seront disponibles qu'après redémarrage. Préférez une application Flatpak quand elle existe.",
                "Superposer",
                false,
                go,
            ),
            _ => go(),
        }
    }

    fn remove(self: &Rc<Self>, idx: usize, pkg: Package) {
        let app = self.clone();
        let name = pkg.name.clone();
        self.confirm(
            &format!("Supprimer {name} ?"),
            if self.kind(idx) == BackendKind::Flatpak {
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

    fn package_row(self: &Rc<Self>, idx: usize, p: &Package) -> adap::ActionRow {
        let kind = self.kind(idx);
        let r = row(&p.name, &p.summary);
        r.add_prefix(&icon_for(p, 48));
        let info = gtk::Box::new(gtk::Orientation::Vertical, 4);
        info.set_valign(gtk::Align::Center);
        let b = badge(&source_text(p), kind);
        b.set_halign(gtk::Align::End);
        info.append(&b);
        if !p.version.is_empty() {
            let v = gtk::Label::new(Some(&p.version));
            v.add_css_class("dim-label");
            v.add_css_class("caption");
            v.set_halign(gtk::Align::End);
            v.set_max_width_chars(22);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            info.append(&v);
        }
        r.add_suffix(&info);
        let button = if p.installed {
            if kind == BackendKind::RpmOstree && !self.layered(p) {
                let l = gtk::Label::new(Some("Inclus"));
                l.add_css_class("dim-label");
                r.add_suffix(&l);
                return r;
            }
            let b = gtk::Button::with_label("Supprimer");
            b.add_css_class("destructive-action");
            let (app, pkg) = (self.clone(), p.clone());
            b.connect_clicked(move |_| app.remove(idx, pkg.clone()));
            b
        } else {
            let b = gtk::Button::with_label("Installer");
            b.add_css_class("suggested-action");
            let (app, pkg) = (self.clone(), p.clone());
            b.connect_clicked(move |_| app.install(idx, pkg.clone()));
            b
        };
        button.set_valign(gtk::Align::Center);
        button.add_css_class("pill");
        r.add_suffix(&button);
        r
    }

    fn layered(&self, p: &Package) -> bool {
        self.installed.borrow().iter().any(|(i, list, _)| self.kind(*i) == BackendKind::RpmOstree && list.iter().any(|x| x.id == p.id))
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
            let task = background(move || backends[idx].search(&q));
            let (app, pending, query) = (self.clone(), pending.clone(), query.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                if app.search_gen.get() != generation {
                    return;
                }
                pending.set(pending.get() - 1);
                match res {
                    Ok(pkgs) => app.found.borrow_mut().extend(pkgs.into_iter().map(|p| (idx, p))),
                    Err(e) => app.toasts.add_toast(adap::Toast::new(&format!("{} : {e}", app.kind(idx).label()))),
                }
                app.render_results(&query, pending.get() == 0);
            });
        }
    }

    fn render_results(self: &Rc<Self>, query: &str, finished: bool) {
        let mut found = self.found.borrow().clone();
        if found.is_empty() {
            if finished {
                self.explore_stack.set_visible_child_name("none");
            }
            return;
        }
        // Classement global : pertinence, puis Flatpak avant les paquets système à égalité.
        let mut pkgs: Vec<Package> = found.iter().map(|(_, p)| p.clone()).collect();
        noos_pkg::rank(query, &mut pkgs);
        found.sort_by_key(|(i, p)| {
            let pos = pkgs.iter().position(|x| x.id == p.id && x.backend == p.backend).unwrap_or(usize::MAX);
            (pos, self.kind(*i) != BackendKind::Flatpak)
        });
        self.results.remove_all();
        for (idx, p) in found.iter().take(120) {
            self.results.append(&self.package_row(*idx, p));
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
                BackendKind::RpmOstree => "Paquets superposés à l'image système",
                BackendKind::Flatpak => "Applications Flatpak",
                BackendKind::Aur => "Paquets hors dépôts (AUR)",
                _ => "Paquets installés explicitement (hors dépendances)",
            }));
            if let Some(e) = err {
                g.add(&row("Erreur", e));
            } else if shown.is_empty() {
                g.add(&row(if list.is_empty() { "Aucun" } else { "Aucun résultat pour ce filtre" }, ""));
            }
            for p in shown.iter().take(300) {
                g.add(&self.package_row(*idx, p));
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
                .map(|i| (i, backends[i].updates().map_err(|e| format!("{e:#}")), backends[i].status()))
                .collect::<Vec<_>>()
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let results = task.await;
            app.render_updates(results);
        });
    }

    fn render_updates(self: &Rc<Self>, results: Vec<(usize, Result<Vec<Update>, String>, noos_pkg::Status)>) {
        clear(&self.updates_box);
        let reboot = results.iter().find(|(_, _, s)| s.reboot_required);
        self.banner.set_revealed(reboot.is_some());
        if let Some((_, _, s)) = reboot {
            self.banner.set_title(&s.message);
        }
        let total: usize = results.iter().filter_map(|(_, r, _)| r.as_ref().ok()).map(|u| u.len()).sum();
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
            let (app, idxs) = (self.clone(), results.iter().filter(|(_, r, _)| r.as_ref().is_ok_and(|u| !u.is_empty())).map(|(i, _, _)| *i).collect::<Vec<_>>());
            all.connect_clicked(move |_| {
                for i in &idxs {
                    app.upgrade(*i);
                }
            });
            header.append(&all);
        }
        self.updates_box.append(&header);

        if total == 0 && results.iter().all(|(_, r, _)| r.is_ok()) {
            self.updates_box.append(&status_page("emblem-ok-symbolic", "Votre système est à jour", "Aucune mise à jour pour l'instant."));
        }

        for (idx, res, _) in results {
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
        self.ops.enqueue(
            format!("Mise à jour : {}", kind.label()),
            idx,
            |b| Ok(b.upgrade_all()),
            move |ok| {
                if ok && kind == BackendKind::RpmOstree {
                    app.toasts.add_toast(adap::Toast::new("Nouvelle image prête : redémarrez pour l'utiliser"));
                }
                app.load_updates();
            },
        );
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
        self.repos_box.append(&loading("Lecture des dépôts…"));
        let backends = self.backends.clone();
        let task = background(move || {
            (0..backends.len())
                .map(|i| (i, backends[i].repos().map_err(|e| format!("{e:#}")), backends[i].presets()))
                .collect::<Vec<_>>()
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let res = task.await;
            app.render_repos(res);
        });
    }

    fn render_repos(self: &Rc<Self>, data: Vec<(usize, Result<Vec<Repo>, String>, Vec<Preset>)>) {
        clear(&self.repos_box);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let t = gtk::Label::new(Some("Dépôts et sources"));
        t.add_css_class("title-2");
        t.set_hexpand(true);
        t.set_xalign(0.0);
        head.append(&t);
        head.append(&self.show_debug);
        self.repos_box.append(&head);
        let show_debug = self.show_debug.is_active();

        for (idx, repos, presets) in data {
            let kind = self.kind(idx);
            if kind == BackendKind::Aur && presets.is_empty() {
                continue;
            }
            let g = adap::PreferencesGroup::new();
            g.set_title(kind.label());
            match repos {
                Err(e) => g.add(&row("Lecture impossible", &e)),
                Ok(list) => {
                    let mut hidden = 0;
                    for r in list {
                        let technical = ["-debuginfo", "-source", "-debug", "(sources)"].iter().any(|s| r.id.ends_with(s) || r.name.contains(s));
                        if technical && !show_debug {
                            hidden += 1;
                            continue;
                        }
                        g.add(&self.repo_row(idx, r));
                    }
                    if hidden > 0 {
                        g.set_description(Some(&format!("{hidden} dépôt(s) de débogage / sources masqué(s)")));
                    }
                }
            }
            self.repos_box.append(&g);

            if !presets.is_empty() {
                let pg = adap::PreferencesGroup::new();
                pg.set_title(&format!("Ajouter un dépôt — {}", kind.label()));
                for p in presets {
                    let r = row(p.name, p.description);
                    let b = gtk::Button::with_label(if p.argument.is_some() { "Ajouter…" } else { "Ajouter" });
                    b.set_valign(gtk::Align::Center);
                    let app = self.clone();
                    b.connect_clicked(move |_| app.add_preset(idx, p.clone()));
                    r.add_suffix(&b);
                    pg.add(&r);
                }
                self.repos_box.append(&pg);
            }
        }
    }

    fn repo_row(self: &Rc<Self>, idx: usize, r: Repo) -> adap::ActionRow {
        let mut subtitle = r.url.clone();
        if r.builtin {
            subtitle = format!("{subtitle}\nFourni par la distribution");
        }
        let row = row(&r.name, subtitle.trim());
        row.set_subtitle_selectable(true);
        if !r.builtin && r.backend != BackendKind::Flatpak || (r.backend == BackendKind::Flatpak && !r.builtin) {
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            del.set_tooltip_text(Some("Supprimer ce dépôt"));
            let (app, repo) = (self.clone(), r.clone());
            del.connect_clicked(move |_| {
                let (app2, repo2) = (app.clone(), repo.clone());
                app.confirm(
                    &format!("Supprimer « {} » ?", repo.name),
                    "Les paquets déjà installés depuis ce dépôt restent installés mais ne recevront plus de mises à jour.",
                    "Supprimer",
                    true,
                    move || {
                        let (app3, repo3) = (app2.clone(), repo2.clone());
                        app2.ops.enqueue(format!("Suppression du dépôt {}", repo2.name), idx, move |b| b.remove_repo(&repo3), move |_| app3.load_repos());
                    },
                );
            });
            row.add_suffix(&del);
        }
        let sw = gtk::Switch::new();
        sw.set_active(r.enabled);
        sw.set_valign(gtk::Align::Center);
        let app = self.clone();
        sw.connect_state_set(move |_, on| {
            let (app2, repo) = (app.clone(), r.clone());
            let verb = if on { "Activation" } else { "Désactivation" };
            app.ops.enqueue(format!("{verb} de {}", r.name), idx, move |b| b.set_repo_enabled(&repo, on), move |_| app2.load_repos());
            glib::Propagation::Proceed
        });
        row.add_suffix(&sw);
        row
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
.noos-badge.aur { background: alpha(@warning_bg_color, 0.22); color: @warning_color; }
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
    let mut labels = vec!["Toutes les sources".to_string()];
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
    explore_stack.add_named(&loading("Recherche dans toutes les sources…"), Some("loading"));
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
    let (updates_scroll, updates_box) = page_container();
    let banner = adap::Banner::new("");
    banner.set_button_label(Some("Redémarrer"));
    banner.connect_button_clicked(|_| {
        let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
    });
    let updates_view = gtk::Box::new(gtk::Orientation::Vertical, 0);
    updates_view.append(&banner);
    updates_view.append(&updates_scroll);

    // ── Sources ──
    let (repos_scroll, repos_box) = page_container();
    let show_debug = gtk::CheckButton::with_label("Dépôts de débogage et sources");

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
        explore_home,
        collection_box,
        dynamic_css: gtk::CssProvider::new(),
        home_built: Cell::new(false),
        installed_box,
        installed_filter: installed_filter.clone(),
        installed: RefCell::new(vec![]),
        updates_box,
        updates_page,
        banner,
        repos_box,
        show_debug: show_debug.clone(),
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
    {
        let app2 = app.clone();
        show_debug.connect_toggled(move |_| app2.load_repos());
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
        app.toasts.add_toast(adap::Toast::new("Aucun gestionnaire de paquets reconnu sur ce système"));
    }
    app.load_installed();
    app.load_updates();
    let _ = &app.sys;
    window.present();
    search.grab_focus();
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

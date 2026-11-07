//! noos-save — « Sauvegarder et Restaurer » de NOOS (GTK4 + libAdapta).
//!
//! Instantanés Btrfs du système (snapper, config « root ») : création manuelle, instantanés
//! automatiques avant / après chaque opération dnf et réguliers, restauration au prochain
//! redémarrage, comparaison (paquets et fichiers), exploration, et sous-menu GRUB
//! « Restaurer depuis un instantané ». Les opérations privilégiées passent par
//! `noos-save-helper` (pkexec) ; voir `snapper.rs`.
//!
//! Démo sans snapper : `NOOS_SAVE_DEMO=1` (+ `NOOS_SAVE_DEMO_PENDING=1`, `NOOS_SAVE_DEMO_BOOT=6`,
//! `NOOS_SAVE_PAGE=settings`, `NOOS_SAVE_EXPAND=<n>`, `NOOS_SAVE_COMPARE=<n>`, `NOOS_SAVE_SCROLL=<px>`).

mod snapper;

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use snapper::{Change, Entry, Kind, Origin, Status};

const APP_ID: &str = "org.noos.Save";

const CSS: &str = "
.ns-hero { padding: 22px 24px; }
.ns-hero-icon { border-radius: 999px; min-width: 64px; min-height: 64px; }
.ns-hero-icon.ok { background: alpha(@success_color, 0.16); color: @success_color; }
.ns-hero-icon.warn { background: alpha(@warning_color, 0.16); color: @warning_color; }
.ns-hero-icon.temp { background: alpha(@accent_color, 0.16); color: @accent_color; }
.ns-hero-title { font-size: 1.45em; font-weight: 800; }
.ns-stat { padding: 10px 16px; border-radius: 12px; background: alpha(currentColor, 0.055); min-width: 118px; }
.ns-stat-value { font-size: 1.35em; font-weight: 800; }
.ns-stat-label { font-size: 0.85em; opacity: 0.7; }
.ns-usage trough, .ns-usage progress { min-height: 6px; border-radius: 3px; }
.ns-icon { border-radius: 999px; min-width: 34px; min-height: 34px; }
.ns-badge { border-radius: 999px; padding: 2px 10px; font-size: 0.78em; font-weight: 700; }
.ns-manual { background: alpha(@accent_color, 0.17); color: @accent_color; }
.ns-dnf { background: alpha(@warning_color, 0.17); color: @warning_color; }
.ns-timeline { background: alpha(#c061cb, 0.2); color: #dc8add; }
.ns-keep { color: #f6d32d; }
.ns-grub { background: alpha(currentColor, 0.08); }
.ns-legend { font-size: 0.85em; opacity: 0.75; }
.ns-diff-add { color: @success_color; font-weight: 800; }
.ns-diff-del { color: @error_color; font-weight: 800; }
.ns-diff-up { color: @accent_color; font-weight: 800; }
.ns-mono { font-family: monospace; font-size: 0.9em; }
";

/// Exécute `f` dans un fil, résultat récupéré sur la boucle GTK.
fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("fil de travail") }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Manual,
    Auto,
}

struct App {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    banner: adap::Banner,
    boot_banner: adap::Banner,
    views: adap::ViewStack,
    scroll: gtk::ScrolledWindow,
    content: gtk::Box,
    list: gtk::Box,
    settings: gtk::Box,
    create: gtk::Button,
    busy: gtk::Spinner,
    status: RefCell<Status>,
    filter: RefCell<Filter>,
    search: RefCell<String>,
}

fn row(title: &str, subtitle: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.set_subtitle(subtitle);
    r
}

fn info_row(title: &str, value: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.add_css_class("property");
    r.set_subtitle(value);
    r.set_subtitle_selectable(true);
    r
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    for c in classes {
        l.add_css_class(c);
    }
    l
}

fn parse_date(date: &str) -> Option<glib::DateTime> {
    // snapper donne l'heure locale, sans fuseau.
    glib::DateTime::from_iso8601(&date.replace(' ', "T"), Some(&glib::TimeZone::local())).ok()
}

fn fmt(date: &str, f: &str) -> String {
    parse_date(date).and_then(|d| d.format(f).ok()).map(|s| s.to_string()).unwrap_or_else(|| date.to_string())
}

/// Jours écoulés depuis `date` (0 = aujourd'hui), en jours calendaires.
fn days_ago(date: &str) -> Option<i32> {
    let d = parse_date(date)?;
    let now = glib::DateTime::now_local().ok()?;
    let day = |x: &glib::DateTime| glib::DateTime::from_local(x.year(), x.month(), x.day_of_month(), 0, 0, 0.0).ok();
    let diff = day(&now)?.difference(&day(&d)?);
    Some((diff.as_seconds() as f64 / 86_400.0).round() as i32)
}

fn day_title(date: &str) -> String {
    match days_ago(date) {
        Some(0) => "Aujourd'hui".into(),
        Some(1) => "Hier".into(),
        Some(n) if (2..7).contains(&n) => capitalize(&fmt(date, "%A %-d %B")),
        _ => capitalize(&fmt(date, "%A %-d %B %Y")),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// « il y a 3 h », « il y a 2 jours »…
fn relative(date: &str) -> String {
    let (Some(d), Ok(now)) = (parse_date(date), glib::DateTime::now_local()) else { return date.to_string() };
    let s = now.difference(&d).as_seconds();
    match s {
        i64::MIN..60 => "à l'instant".into(),
        60..3600 => format!("il y a {} min", s / 60),
        3600..86_400 => format!("il y a {} h", s / 3600),
        86_400..172_800 => "hier".into(),
        _ => format!("il y a {} jours", s / 86_400),
    }
}

fn human_size(b: u64) -> String {
    let g = b as f64 / 1e9;
    if g >= 100.0 { format!("{g:.0} Go") } else { format!("{g:.1} Go").replace('.', ",") }
}

/// Titre lisible d'une description dnf (« dnf install -y git » → « Installation : git »).
fn nice_description(e: &Entry) -> String {
    let d = e.main.description.trim();
    match e.main.origin() {
        Origin::Timeline => return "Instantané régulier".into(),
        Origin::Dnf => {
            let words: Vec<&str> = d.split_whitespace().filter(|w| !w.starts_with('-')).collect();
            let words: Vec<&str> = match words.first() {
                Some(w) if w.ends_with("dnf") || w.ends_with("dnf5") || w.ends_with("noos-pkg") => words[1..].to_vec(),
                _ => words,
            };
            let (verb, rest) = match words.split_first() {
                Some((v, r)) => (*v, r.join(" ")),
                None => ("", String::new()),
            };
            let what = match verb {
                "install" | "in" => "Installation",
                "remove" | "rm" | "erase" => "Suppression",
                "upgrade" | "update" | "up" | "distro-sync" | "system-upgrade" | "offline" => "Mise à jour",
                "reinstall" => "Réinstallation",
                "downgrade" => "Retour en arrière",
                "swap" => "Remplacement",
                "group" => "Groupe",
                "" => return format!("Opération dnf n° {}", e.main.number),
                v => return format!("dnf {v} {rest}").trim().to_string(),
            };
            return if rest.is_empty() { format!("{what} du système") } else { format!("{what} : {rest}") };
        }
        Origin::Manual => {}
    }
    if d.is_empty() { format!("Instantané n° {}", e.main.number) } else { d.to_string() }
}

fn origin_style(o: Origin) -> (&'static str, &'static str, &'static str) {
    match o {
        Origin::Manual => ("ns-manual", "Manuel", "avatar-default-symbolic"),
        Origin::Dnf => ("ns-dnf", "Auto · dnf", "system-software-install-symbolic"),
        Origin::Timeline => ("ns-timeline", "Auto · régulier", "alarm-symbolic"),
    }
}

fn badge(o: Origin) -> gtk::Label {
    let (class, text, _) = origin_style(o);
    let l = label(text, &["ns-badge", class]);
    l.set_valign(gtk::Align::Center);
    l
}

fn round_icon(o: Origin) -> gtk::Image {
    let (class, _, icon) = origin_style(o);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class("ns-icon");
    i.add_css_class(class);
    i.set_valign(gtk::Align::Center);
    i
}

/// Boîte qui passe à la ligne (FlowBox sans sélection).
fn wrap(spacing: u32) -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_column_spacing(spacing);
    f.set_row_spacing(spacing);
    f.set_max_children_per_line(12);
    f.set_homogeneous(false);
    f.set_valign(gtk::Align::Start);
    f
}

fn stat(value: &str, caption: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.add_css_class("ns-stat");
    let v = label(value, &["ns-stat-value"]);
    v.set_xalign(0.0);
    let c = label(caption, &["ns-stat-label"]);
    c.set_xalign(0.0);
    b.append(&v);
    b.append(&c);
    b
}

impl App {
    fn new(gapp: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(gapp)
            .title("Sauvegarder et Restaurer")
            .default_width(900)
            .default_height(780)
            .build();
        let views = adap::ViewStack::new();
        let header = adap::HeaderBar::new();
        let switcher = adap::ViewSwitcher::builder().stack(&views).policy(adap::ViewSwitcherPolicy::Wide).build();
        header.set_title_widget(Some(&switcher));
        let create = gtk::Button::new();
        create.set_child(Some(&adap::ButtonContent::builder().label("Créer").icon_name("list-add-symbolic").build()));
        create.add_css_class("suggested-action");
        create.set_tooltip_text(Some("Créer un instantané du système maintenant"));
        header.pack_start(&create);
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Actualiser"));
        header.pack_end(&refresh);
        let busy = gtk::Spinner::new();
        header.pack_end(&busy);

        let banner = adap::Banner::new("");
        banner.set_button_label(Some("Annuler la restauration"));
        let boot_banner = adap::Banner::new("");
        boot_banner.set_button_label(Some("Garder cet état…"));

        let page = |child: &gtk::Box| {
            child.set_margin_top(24);
            child.set_margin_bottom(32);
            child.set_margin_start(12);
            child.set_margin_end(12);
            let clamp = adap::Clamp::builder().maximum_size(820).tightening_threshold(600).child(child).build();
            gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()
        };
        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 22);
        let settings = gtk::Box::new(gtk::Orientation::Vertical, 24);
        let scroll = page(&content);
        views.add_titled_with_icon(&scroll, Some("snapshots"), "Instantanés", "document-revert-symbolic");
        views.add_titled_with_icon(&page(&settings), Some("settings"), "Réglages", "emblem-system-symbolic");

        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&boot_banner);
        body.append(&banner);
        body.append(&views);
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&body));
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&toasts));
        window.set_content(Some(&tv));

        let app = Rc::new(Self {
            window,
            toasts,
            banner,
            boot_banner,
            views,
            scroll,
            content,
            list,
            settings,
            create: create.clone(),
            busy,
            status: RefCell::new(Status::default()),
            filter: RefCell::new(Filter::All),
            search: RefCell::new(String::new()),
        });
        {
            let app2 = app.clone();
            create.connect_clicked(move |_| app2.ask_create());
        }
        {
            let app2 = app.clone();
            refresh.connect_clicked(move |_| app2.reload());
        }
        {
            let app2 = app.clone();
            app.banner.connect_button_clicked(move |_| {
                app2.confirm(
                    "Annuler la restauration ?",
                    "Le système redémarrera normalement, dans son état actuel.",
                    "Annuler la restauration",
                    false,
                    {
                        let app3 = app2.clone();
                        move || app3.run(&["cancel"], "Restauration annulée")
                    },
                );
            });
        }
        {
            let app2 = app.clone();
            app.boot_banner.connect_button_clicked(move |_| {
                let Some(n) = app2.status.borrow().boot_snapshot else { return };
                let app3 = app2.clone();
                app2.confirm(
                    &format!("Garder définitivement l'instantané n° {n} ?"),
                    "Le système redémarrera dans cet état, de façon permanente. L'état d'avant est conservé : vous pourrez le supprimer une fois que tout fonctionne.\n\nVos dossiers personnels (/home) ne changent pas.",
                    "Restaurer au redémarrage",
                    true,
                    move || app3.run(&["rollback", &n.to_string()], "Restauration prévue : redémarrez pour l'appliquer"),
                );
            });
        }
        app
    }

    fn reload(self: &Rc<Self>) {
        self.busy.start();
        let task = background(snapper::read);
        let app = self.clone();
        glib::spawn_future_local(async move {
            let st = task.await;
            app.busy.stop();
            *app.status.borrow_mut() = st;
            app.render();
        });
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    /// Lance `noos-save-helper <args>` puis recharge.
    fn run(self: &Rc<Self>, args: &[&str], done: &str) {
        self.busy.start();
        self.window.set_sensitive(false);
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let task = background(move || snapper::helper(&args.iter().map(String::as_str).collect::<Vec<_>>()));
        let (app, done) = (self.clone(), done.to_string());
        glib::spawn_future_local(async move {
            let res = task.await;
            app.window.set_sensitive(true);
            match res {
                Ok(_) => app.toast(&done),
                Err(e) => {
                    let d = adap::AlertDialog::new(Some("L'opération a échoué"), Some(&format!("{e:#}")));
                    d.add_response("ok", "Fermer");
                    d.present(Some(&app.window));
                }
            }
            app.reload();
        });
    }

    fn confirm(self: &Rc<Self>, title: &str, body: &str, action: &str, destructive: bool, f: impl Fn() + 'static) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance("ok", if destructive { adap::ResponseAppearance::Destructive } else { adap::ResponseAppearance::Suggested });
        d.set_default_response(Some("cancel"));
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f();
            }
        });
        d.present(Some(&self.window));
    }

    /// Demande un texte (création, renommage) puis appelle `f`.
    fn ask_text(self: &Rc<Self>, title: &str, body: &str, initial: &str, action: &str, f: impl Fn(String) + 'static) {
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Ex. : avant d'installer les pilotes NVIDIA"));
        entry.set_text(initial);
        entry.set_activates_default(true);
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let e2 = entry.clone();
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f(e2.text().trim().to_string());
            }
        });
        d.present(Some(&self.window));
        glib::idle_add_local_once(move || {
            entry.grab_focus();
        });
    }

    fn ask_create(self: &Rc<Self>) {
        let app = self.clone();
        self.ask_text(
            "Créer un instantané",
            "Une photo de tout le système (hors dossiers personnels) que vous pourrez restaurer ou démarrer depuis le menu de démarrage. Les instantanés manuels ne sont jamais supprimés automatiquement.",
            "",
            "Créer",
            move |text| {
                let desc = if text.is_empty() { "Instantané manuel".to_string() } else { text };
                app.run(&["create", &desc], "Instantané créé");
            },
        );
    }

    fn render(self: &Rc<Self>) {
        let st = self.status.borrow().clone();
        clear(&self.content);
        self.create.set_sensitive(st.configured && st.pending.is_none() && st.boot_snapshot.is_none());

        match &st.pending {
            Some(p) => {
                let n = p.get("SNAPSHOT").map(String::as_str).unwrap_or("?");
                self.banner.set_title(&format!("L'instantané n° {n} sera restauré au prochain redémarrage"));
                self.banner.set_revealed(true);
            }
            None => self.banner.set_revealed(false),
        }
        match st.boot_snapshot {
            Some(n) if st.pending.is_none() => {
                self.boot_banner.set_title(&format!(
                    "Session temporaire sur l'instantané n° {n} : les changements seront oubliés au redémarrage"
                ));
                self.boot_banner.set_revealed(true);
            }
            _ => self.boot_banner.set_revealed(false),
        }

        if !st.btrfs {
            self.content.append(
                &adap::StatusPage::builder()
                    .icon_name("drive-harddisk-symbolic")
                    .title("Instantanés indisponibles")
                    .description("Le système n'est pas installé sur un système de fichiers Btrfs. L'installateur de NOOS utilise Btrfs : réinstallez pour profiter des instantanés.")
                    .vexpand(true)
                    .build(),
            );
            clear(&self.settings);
            return;
        }
        if !st.configured {
            let page = adap::StatusPage::builder()
                .icon_name("document-revert-symbolic")
                .title("Protégez votre système")
                .description("Les instantanés gardent une copie du système avant chaque installation ou mise à jour. En cas de problème, vous revenez en arrière en un redémarrage, ou directement depuis le menu de démarrage. Vos dossiers personnels ne sont pas concernés.")
                .vexpand(true)
                .build();
            let b = gtk::Button::with_label("Activer les instantanés");
            b.add_css_class("pill");
            b.add_css_class("suggested-action");
            b.set_halign(gtk::Align::Center);
            let app = self.clone();
            b.connect_clicked(move |_| app.run(&["setup"], "Instantanés activés"));
            page.set_child(Some(&b));
            self.content.append(&page);
            clear(&self.settings);
            return;
        }

        self.content.append(&self.hero(&st));

        if let Some(r) = &st.restored {
            self.content.append(&self.restored_group(&st, r));
        }

        // Barre de filtres : origine + recherche.
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let entries = st.list();
        let manual = entries.iter().filter(|e| e.main.origin() == Origin::Manual).count();
        let toggles = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggles.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        for (f, text) in [
            (Filter::All, format!("Tous  {}", entries.len())),
            (Filter::Manual, format!("Manuels  {manual}")),
            (Filter::Auto, format!("Automatiques  {}", entries.len() - manual)),
        ] {
            let b = gtk::ToggleButton::with_label(&text);
            if let Some(g) = &first {
                b.set_group(Some(g));
            } else {
                first = Some(b.clone());
            }
            b.set_active(*self.filter.borrow() == f);
            let app = self.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    *app.filter.borrow_mut() = f;
                    app.render_list();
                }
            });
            toggles.append(&b);
        }
        bar.append(&toggles);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher un instantané"));
        search.set_hexpand(true);
        search.set_text(&self.search.borrow());
        {
            let app = self.clone();
            search.connect_search_changed(move |s| {
                *app.search.borrow_mut() = s.text().to_lowercase();
                app.render_list();
            });
        }
        bar.append(&search);
        self.content.append(&bar);

        // Légende.
        let legend = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for o in [Origin::Manual, Origin::Dnf, Origin::Timeline] {
            legend.append(&badge(o));
        }
        let star = gtk::Image::from_icon_name("starred-symbolic");
        star.add_css_class("ns-keep");
        star.set_margin_start(6);
        legend.append(&star);
        legend.append(&label("à conserver", &["ns-legend"]));
        let boot = gtk::Image::from_icon_name("system-reboot-symbolic");
        boot.set_margin_start(6);
        boot.set_opacity(0.7);
        legend.append(&boot);
        legend.append(&label("dans le menu de démarrage", &["ns-legend"]));
        legend.set_margin_top(-12);
        self.content.append(&legend);

        self.content.append(&self.list);
        self.render_list();

        clear(&self.settings);
        self.fill_settings(&st);
    }

    /// Carte d'état : protection, nombre d'instantanés, dernier, disque, menu de démarrage.
    fn hero(self: &Rc<Self>, st: &Status) -> gtk::Box {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 18);
        card.add_css_class("card");
        card.add_css_class("ns-hero");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        let entries = st.list();
        let auto = st.dnf_snapshots || st.get("TIMELINE_CREATE") == "yes";
        let (icon, class, title, sub) = if let Some(n) = st.boot_snapshot {
            (
                "document-revert-symbolic",
                "temp",
                format!("Démarré sur l'instantané n° {n}"),
                "Lecture seule avec une couche temporaire : testez, puis gardez cet état ou redémarrez pour revenir au système actuel.".to_string(),
            )
        } else if st.pending.is_some() {
            ("system-reboot-symbolic", "temp", "Restauration prévue".into(), "Redémarrez pour revenir à l'état choisi.".into())
        } else if auto && !entries.is_empty() {
            (
                "security-high-symbolic",
                "ok",
                "Système protégé".into(),
                format!(
                    "Instantané automatique {}. En cas de problème : Démarrage › « Restaurer depuis un instantané ».",
                    match (st.dnf_snapshots, st.get("TIMELINE_CREATE") == "yes") {
                        (true, true) => "avant et après chaque opération dnf, et toutes les heures",
                        (true, false) => "avant et après chaque opération dnf",
                        _ => "toutes les heures",
                    }
                ),
            )
        } else if auto {
            ("security-medium-symbolic", "ok", "Protection active".into(), "Le premier instantané sera créé à la prochaine opération dnf. Vous pouvez aussi en créer un maintenant.".into())
        } else {
            (
                "security-low-symbolic",
                "warn",
                "Protection manuelle uniquement".into(),
                "Aucun instantané automatique : activez-les dans Réglages pour pouvoir revenir en arrière après une mise à jour.".into(),
            )
        };
        let img = gtk::Image::from_icon_name(icon);
        img.set_pixel_size(32);
        img.add_css_class("ns-hero-icon");
        img.add_css_class(class);
        img.set_valign(gtk::Align::Center);
        top.append(&img);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
        texts.set_valign(gtk::Align::Center);
        let t = label(&title, &["ns-hero-title"]);
        t.set_xalign(0.0);
        let s = label(&sub, &["dim-label"]);
        s.set_xalign(0.0);
        s.set_wrap(true);
        texts.append(&t);
        texts.append(&s);
        top.append(&texts);
        card.append(&top);

        let stats = wrap(10);
        stats.set_homogeneous(true);
        stats.set_min_children_per_line(2);
        stats.set_max_children_per_line(3);
        let manual = entries.iter().filter(|e| e.main.origin() == Origin::Manual).count();
        stats.append(&stat(&entries.len().to_string(), "instantanés"));
        stats.append(&stat(&manual.to_string(), "manuels"));
        stats.append(&stat(&(entries.len() - manual).to_string(), "automatiques"));
        let last = entries
            .iter()
            .map(|e| e.post.as_ref().unwrap_or(&e.main))
            .max_by_key(|s| s.number)
            .map(|s| relative(&s.date))
            .unwrap_or_else(|| "—".into());
        stats.append(&stat(&last, "dernier instantané"));
        let grub = if st.grub_snapshots {
            let n: usize = entries.iter().map(|e| e.numbers().len()).sum::<usize>().min(st.grub_max as usize);
            format!("{n} entrée{}", if n > 1 { "s" } else { "" })
        } else {
            "désactivé".into()
        };
        stats.append(&stat(&grub, "menu de démarrage"));
        if let Some((size, used)) = st.disk {
            let b = stat(&format!("{} libres", human_size(size.saturating_sub(used))), &format!("sur {}", human_size(size)));
            let bar = gtk::LevelBar::for_interval(0.0, 1.0);
            bar.set_value(used as f64 / size.max(1) as f64);
            bar.add_css_class("ns-usage");
            bar.set_margin_top(4);
            b.append(&bar);
            stats.append(&b);
        }
        card.append(&stats);
        card
    }

    fn restored_group(self: &Rc<Self>, st: &Status, r: &std::collections::HashMap<String, String>) -> adap::PreferencesGroup {
        let old = r.get("OLD").cloned().unwrap_or_default();
        let g = adap::PreferencesGroup::new();
        g.set_title("Restauration effectuée");
        let r2 = row(
            &format!("Système restauré depuis l'instantané n° {}", r.get("SNAPSHOT").map(String::as_str).unwrap_or("?")),
            &format!("L'ancien état est conservé ({old}) au cas où. Supprimez-le quand tout fonctionne pour libérer de la place."),
        );
        r2.set_subtitle_lines(3);
        r2.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
        if st.pending.is_none() {
            let del = gtk::Button::with_label("Supprimer l'ancien état");
            del.set_valign(gtk::Align::Center);
            del.add_css_class("destructive-action");
            let app = self.clone();
            del.connect_clicked(move |_| {
                let (app2, old2) = (app.clone(), old.clone());
                app.confirm(
                    "Supprimer l'ancien état du système ?",
                    "Il ne sera plus possible de revenir à l'état d'avant la restauration.",
                    "Supprimer",
                    true,
                    move || app2.run(&["delete-old", &old2], "Ancien état supprimé"),
                );
            });
            r2.add_suffix(&del);
        }
        g.add(&r2);
        g
    }

    /// Liste filtrée, groupée par jour.
    fn render_list(self: &Rc<Self>) {
        clear(&self.list);
        let st = self.status.borrow().clone();
        let filter = *self.filter.borrow();
        let search = self.search.borrow().clone();
        let entries = match &st.entries {
            Err(e) => {
                let g = adap::PreferencesGroup::new();
                g.add(&row("Lecture des instantanés impossible", e));
                self.list.append(&g);
                return;
            }
            Ok(l) => l,
        };
        let shown: Vec<&Entry> = entries
            .iter()
            .filter(|e| match filter {
                Filter::All => true,
                Filter::Manual => e.main.origin() == Origin::Manual,
                Filter::Auto => e.main.origin().automatic(),
            })
            .filter(|e| {
                search.is_empty()
                    || nice_description(e).to_lowercase().contains(&search)
                    || e.main.description.to_lowercase().contains(&search)
                    || e.numbers().iter().any(|n| n.to_string() == search)
            })
            .collect();
        if shown.is_empty() {
            let empty = adap::StatusPage::builder()
                .icon_name(if entries.is_empty() { "document-revert-symbolic" } else { "edit-find-symbolic" })
                .title(if entries.is_empty() { "Aucun instantané" } else { "Aucun résultat" })
                .description(if entries.is_empty() {
                    "Il en sera créé un avant chaque opération dnf. Vous pouvez aussi cliquer sur « Créer »."
                } else {
                    "Aucun instantané ne correspond à ce filtre."
                })
                .build();
            empty.add_css_class("compact");
            self.list.append(&empty);
            return;
        }
        let expand: Option<u32> = std::env::var("NOOS_SAVE_EXPAND").ok().and_then(|v| v.parse().ok());
        let mut current: Option<(String, adap::PreferencesGroup)> = None;
        for e in shown {
            let day = day_title(&e.main.date);
            if current.as_ref().is_none_or(|(d, _)| *d != day) {
                let g = adap::PreferencesGroup::new();
                g.set_title(&day);
                self.list.append(&g);
                current = Some((day, g));
            }
            let r = self.entry_row(e, &st);
            if expand.is_some_and(|n| e.numbers().contains(&n)) {
                r.set_expanded(true);
            }
            current.as_ref().unwrap().1.add(&r);
        }
    }

    fn entry_row(self: &Rc<Self>, e: &Entry, st: &Status) -> adap::ExpanderRow {
        let origin = e.main.origin();
        let busy = st.pending.is_some();
        let r = adap::ExpanderRow::new();
        r.set_use_markup(false);
        r.set_title(&nice_description(e));
        r.set_title_lines(1);
        let numbers = e.numbers().iter().map(|n| format!("n° {n}")).collect::<Vec<_>>().join(" → ");
        let time = match &e.post {
            Some(p) => format!("{} → {}", fmt(&e.main.date, "%H:%M"), fmt(&p.date, "%H:%M")),
            None => fmt(&e.main.date, "%H:%M"),
        };
        let kind = match (origin, &e.post) {
            (Origin::Dnf, Some(_)) => " · avant / après",
            (Origin::Dnf, None) if e.main.kind == Kind::Pre => " · avant (opération interrompue ?)",
            _ => "",
        };
        r.set_subtitle(&format!("{time} · {numbers}{kind}"));
        r.add_prefix(&round_icon(origin));

        if st.boot_snapshot.is_some_and(|n| e.numbers().contains(&n)) {
            let l = label("En cours", &["ns-badge", "ns-grub"]);
            l.set_valign(gtk::Align::Center);
            r.add_suffix(&l);
        }
        if e.main.important() {
            let star = gtk::Image::from_icon_name("starred-symbolic");
            star.add_css_class("ns-keep");
            star.set_tooltip_text(Some("À conserver : jamais supprimé automatiquement"));
            r.add_suffix(&star);
        }
        if st.in_grub(e.main.number) {
            let boot = gtk::Image::from_icon_name("system-reboot-symbolic");
            boot.set_opacity(0.7);
            boot.set_tooltip_text(Some("Disponible au démarrage : « Restaurer depuis un instantané »"));
            r.add_suffix(&boot);
        }
        r.add_suffix(&badge(origin));

        // Détails (date complète, origine, nettoyage, menu de démarrage) et actions.
        let full = |s: &snapper::Snapshot| capitalize(&fmt(&s.date, "%A %-d %B %Y à %H:%M:%S"));
        match &e.post {
            Some(p) => {
                r.add_row(&info_row("Avant l'opération", &format!("{} — n° {}", full(&e.main), e.main.number)));
                let dur = parse_date(&p.date)
                    .zip(parse_date(&e.main.date))
                    .map(|(a, b)| a.difference(&b).as_seconds())
                    .map(|s| if s < 60 { format!(" (opération de {s} s)") } else { format!(" (opération de {} min {} s)", s / 60, s % 60) })
                    .unwrap_or_default();
                r.add_row(&info_row("Après l'opération", &format!("{} — n° {}{dur}", full(p), p.number)));
            }
            None => r.add_row(&info_row("Date", &format!("{} — n° {}", full(&e.main), e.main.number))),
        }
        let source = match origin {
            Origin::Manual => "Créé manuellement".to_string(),
            Origin::Dnf => format!("Automatique, par dnf : {}", if e.main.description.is_empty() { "dnf" } else { &e.main.description }),
            Origin::Timeline => "Automatique, instantané régulier (toutes les heures)".to_string(),
        };
        r.add_row(&info_row("Origine", &source));
        let cleanup = if e.main.important() {
            "À conserver : jamais supprimé automatiquement".to_string()
        } else {
            match origin {
                Origin::Manual => "Jamais supprimé automatiquement".to_string(),
                Origin::Dnf => format!("Supprimé automatiquement au-delà des {} opérations dnf les plus récentes", st.number("NUMBER_LIMIT", 10)),
                Origin::Timeline => "Supprimé automatiquement selon les limites des instantanés réguliers".to_string(),
            }
        };
        r.add_row(&info_row("Nettoyage", &cleanup));
        r.add_row(&info_row(
            "Menu de démarrage",
            &if st.in_grub(e.main.number) {
                "Oui : Démarrage › « Restaurer depuis un instantané » (essai sans rien modifier)".to_string()
            } else if !st.grub_snapshots {
                "Non : sous-menu désactivé dans Réglages".to_string()
            } else {
                format!("Non : seuls les {} plus récents y figurent", st.grub_max)
            },
        ));
        let system = info_row("Système", "Lecture…");
        r.add_row(&system);
        {
            // Noyau et nombre de paquets : lus au premier dépliage.
            let loaded = std::cell::Cell::new(false);
            let n = e.post.as_ref().map(|p| p.number).unwrap_or(e.main.number);
            let load = move |sys: &adap::ActionRow| {
                if loaded.replace(true) {
                    return;
                }
                let task = background(move || snapper::details(n));
                let sys = sys.clone();
                glib::spawn_future_local(async move {
                    let d = task.await;
                    sys.set_subtitle(&format!(
                        "Noyau {} · {} paquets",
                        d.kernel.as_deref().unwrap_or("inconnu"),
                        d.packages.map(|p| p.to_string()).unwrap_or_else(|| "?".into())
                    ));
                });
            };
            if r.is_expanded() {
                load(&system);
            }
            r.connect_expanded_notify(move |r| {
                if r.is_expanded() {
                    load(&system);
                }
            });
        }

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        actions.set_margin_top(12);
        actions.set_margin_bottom(12);
        actions.set_margin_start(12);
        actions.set_margin_end(12);
        let button = |text: &str, icon: &str, tip: &str| {
            let b = gtk::Button::new();
            b.set_child(Some(&adap::ButtonContent::builder().label(text).icon_name(icon).build()));
            b.set_tooltip_text(Some(tip));
            actions.append(&b);
            b
        };
        // Actions secondaires : icônes à droite (infobulle = libellé).
        let small = |icon: &str, tip: &str| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("flat");
            b.set_tooltip_text(Some(tip));
            tools.append(&b);
            b
        };
        let in_snapshot_boot = st.boot_snapshot.is_some();

        let restore = button(
            "Restaurer…",
            "document-revert-symbolic",
            if e.post.is_some() { "Revenir à l'état du système juste avant cette opération" } else { "Revenir à l'état du système à ce moment" },
        );
        restore.add_css_class("suggested-action");
        restore.set_sensitive(!busy);
        {
            let (app, n, title, date) = (self.clone(), e.main.number, nice_description(e), fmt(&e.main.date, "%-d %B %Y à %H:%M"));
            let before = e.post.is_some();
            restore.connect_clicked(move |_| {
                let app2 = app.clone();
                app.confirm(
                    &format!("Restaurer « {title} » ?"),
                    &format!(
                        "Au prochain redémarrage, le système reviendra à son état du {date}{}.\n\n\
                         Les logiciels installés et les réglages système depuis seront annulés ; vos dossiers personnels (/home) ne changent pas. \
                         L'état actuel est conservé et vous pourrez annuler tant que vous n'avez pas redémarré.\n\n\
                         Astuce : pour l'essayer d'abord sans rien changer, démarrez-le depuis le menu de démarrage.",
                        if before { ", juste avant cette opération" } else { "" }
                    ),
                    "Restaurer au redémarrage",
                    true,
                    move || app2.run(&["rollback", &n.to_string()], "Restauration prévue : redémarrez pour l'appliquer"),
                );
            });
        }

        let compare = button(
            "Comparer",
            "view-dual-symbolic",
            if e.post.is_some() { "Ce que cette opération a changé" } else { "Ce qui a changé depuis cet instantané" },
        );
        {
            let (app, e2) = (self.clone(), e.clone());
            compare.connect_clicked(move |_| app.compare(&e2));
        }

        let browse = button("Parcourir", "folder-open-symbolic", "Ouvrir les fichiers de l'instantané (lecture seule) pour en récupérer un");
        {
            let (app, n) = (self.clone(), e.post.as_ref().map(|p| p.number).unwrap_or(e.main.number));
            browse.connect_clicked(move |_| {
                let uri = gio::File::for_path(snapper::snapshot_root(n)).uri();
                if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>) {
                    app.toast(&format!("Ouverture impossible : {err}"));
                }
            });
        }

        let rename = small("document-edit-symbolic", "Renommer");
        rename.set_sensitive(!busy && !in_snapshot_boot);
        {
            let (app, n, current) = (self.clone(), e.main.number, if origin == Origin::Manual { e.main.description.clone() } else { nice_description(e) });
            rename.connect_clicked(move |_| {
                let app2 = app.clone();
                app.ask_text("Renommer l'instantané", "Une description pour le retrouver facilement.", &current, "Renommer", move |text| {
                    if !text.is_empty() {
                        app2.run(&["describe", &n.to_string(), &text], "Instantané renommé");
                    }
                });
            });
        }

        let important = e.main.important();
        let keep = small(
            if important { "starred-symbolic" } else { "non-starred-symbolic" },
            if important {
                "À conserver — cliquer pour rétablir le nettoyage automatique"
            } else {
                "Conserver : ne jamais supprimer automatiquement"
            },
        );
        if important {
            keep.add_css_class("ns-keep");
        }
        keep.set_sensitive(!busy && !in_snapshot_boot);
        {
            let (app, nums) = (self.clone(), e.numbers());
            keep.connect_clicked(move |_| {
                let app2 = app.clone();
                let v = if important { "no" } else { "yes" };
                // Une paire avant / après se conserve en entier.
                let nums = nums.clone();
                app.busy.start();
                app.window.set_sensitive(false);
                let task = background(move || nums.iter().try_for_each(|n| snapper::helper(&["keep", &n.to_string(), v]).map(|_| ())));
                glib::spawn_future_local(async move {
                    let res = task.await;
                    app2.window.set_sensitive(true);
                    match res {
                        Ok(()) => app2.toast(if important { "Nettoyage automatique rétabli" } else { "Instantané conservé" }),
                        Err(e) => app2.toast(&format!("{e:#}")),
                    }
                    app2.reload();
                });
            });
        }

        let del = small("user-trash-symbolic", "Supprimer cet instantané");
        del.add_css_class("error");
        del.set_sensitive(!busy && !st.boot_snapshot.is_some_and(|n| e.numbers().contains(&n)));
        {
            let (app, numbers, title) = (self.clone(), e.numbers(), nice_description(e));
            del.connect_clicked(move |_| {
                let (app2, numbers) = (app.clone(), numbers.clone());
                app.confirm(
                    &format!("Supprimer « {title} » ?"),
                    "Cet instantané ne pourra plus être restauré ni démarré.",
                    "Supprimer",
                    true,
                    move || {
                        let mut args = vec!["delete".to_string()];
                        args.extend(numbers.iter().map(u32::to_string));
                        app2.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "Instantané supprimé");
                    },
                );
            });
        }
        let holder = gtk::ListBoxRow::new();
        holder.set_activatable(false);
        tools.set_hexpand(true);
        tools.set_halign(gtk::Align::End);
        actions.append(&tools);
        holder.set_child(Some(&actions));
        r.add_row(&holder);
        r
    }

    /// Fenêtre « Comparer » : paquets (rpm) et fichiers (snapper status).
    fn compare(self: &Rc<Self>, e: &Entry) {
        let (from, to, title) = match &e.post {
            Some(p) => (e.main.number, Some(p.number), format!("Changements de « {} »", nice_description(e))),
            None => (e.main.number, None, format!("Depuis l'instantané n° {}", e.main.number)),
        };
        let dialog = adap::Dialog::new();
        dialog.set_title(&title);
        dialog.set_content_width(720);
        dialog.set_content_height(640);
        let stack = adap::ViewStack::new();
        let header = adap::HeaderBar::new();
        header.set_title_widget(Some(&adap::ViewSwitcher::builder().stack(&stack).policy(adap::ViewSwitcherPolicy::Wide).build()));
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        let sub = label(
            &match to {
                Some(t) => format!("De l'instantané n° {from} (avant) à n° {t} (après)"),
                None => format!("De l'instantané n° {from} ({}) au système actuel", fmt(&e.main.date, "%-d %B à %H:%M")),
            },
            &["dim-label"],
        );
        sub.set_margin_top(6);
        sub.set_margin_bottom(6);
        tv.add_top_bar(&sub);

        let loading = |text: &str| {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
            b.set_valign(gtk::Align::Center);
            b.set_vexpand(true);
            let sp = gtk::Spinner::new();
            sp.set_size_request(32, 32);
            sp.start();
            b.append(&sp);
            b.append(&label(text, &["dim-label"]));
            b
        };
        let pk_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pk_page.append(&loading("Comparaison des paquets…"));
        let fi_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        fi_page.append(&loading("Comparaison des fichiers (peut prendre un moment)…"));
        let pk = stack.add_titled_with_icon(&pk_page, Some("packages"), "Paquets", "package-x-generic-symbolic");
        let fi = stack.add_titled_with_icon(&fi_page, Some("files"), "Fichiers", "text-x-generic-symbolic");
        tv.set_content(Some(&stack));
        dialog.set_child(Some(&tv));
        dialog.present(Some(&self.window));

        let scrolled = |child: &gtk::Widget| {
            let s = gtk::ScrolledWindow::builder().child(child).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
            s
        };
        let list_box = || {
            let l = gtk::ListBox::new();
            l.add_css_class("boxed-list");
            l.set_selection_mode(gtk::SelectionMode::None);
            l.set_margin_top(12);
            l.set_margin_bottom(18);
            l.set_margin_start(18);
            l.set_margin_end(18);
            l.set_valign(gtk::Align::Start);
            l
        };
        let empty = |text: &str| {
            adap::StatusPage::builder().icon_name("object-select-symbolic").title("Aucune différence").description(text).vexpand(true).build()
        };

        // Paquets.
        {
            let task = background(move || snapper::package_diff(from, to));
            let (pk_page, pk) = (pk_page.clone(), pk.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                clear(&pk_page);
                match res {
                    Err(e) => pk_page.append(&adap::StatusPage::builder().icon_name("dialog-warning-symbolic").title("Lecture impossible").description(format!("{e:#}")).vexpand(true).build()),
                    Ok(diff) if diff.is_empty() => pk_page.append(&empty("Les mêmes paquets, dans les mêmes versions.")),
                    Ok(diff) => {
                        pk.set_badge_number(diff.len() as u32);
                        let count = |c: Change| diff.iter().filter(|d| d.0 == c).count();
                        let summary = wrap(10);
                        summary.set_margin_top(12);
                        summary.set_margin_start(18);
                        summary.set_margin_end(18);
                        for (c, cap) in [(Change::Added, "ajoutés"), (Change::Removed, "supprimés"), (Change::Upgraded, "mis à jour"), (Change::Downgraded, "rétrogradés")] {
                            if count(c.clone()) > 0 {
                                summary.append(&stat(&count(c).to_string(), cap));
                            }
                        }
                        let l = list_box();
                        for (c, name, version) in diff.iter().take(2000) {
                            let r = row(name, version);
                            let (sign, class) = match c {
                                Change::Added => ("+", "ns-diff-add"),
                                Change::Removed => ("−", "ns-diff-del"),
                                Change::Upgraded => ("↑", "ns-diff-up"),
                                Change::Downgraded => ("↓", "ns-diff-del"),
                            };
                            let s = label(sign, &[class]);
                            s.set_width_chars(2);
                            r.add_prefix(&s);
                            l.append(&r);
                        }
                        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        b.append(&summary);
                        b.append(&l);
                        pk_page.append(&scrolled(b.upcast_ref()));
                    }
                }
            });
        }
        // Fichiers.
        {
            let task = background(move || snapper::file_changes(from, to.unwrap_or(0)));
            let (fi_page, fi) = (fi_page.clone(), fi.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                clear(&fi_page);
                match res {
                    Err(e) => fi_page.append(&adap::StatusPage::builder().icon_name("dialog-warning-symbolic").title("Lecture impossible").description(format!("{e:#}")).vexpand(true).build()),
                    Ok(files) if files.is_empty() => fi_page.append(&empty("Aucun fichier système modifié.")),
                    Ok(files) => {
                        fi.set_badge_number(files.len() as u32);
                        let search = gtk::SearchEntry::new();
                        search.set_placeholder_text(Some(&format!("Filtrer les {} fichiers", files.len())));
                        search.set_margin_top(12);
                        search.set_margin_start(18);
                        search.set_margin_end(18);
                        let l = list_box();
                        let files = Rc::new(files);
                        let fill = {
                            let (l, files) = (l.clone(), files.clone());
                            move |q: &str| {
                                l.remove_all();
                                let q = q.to_lowercase();
                                let mut n = 0;
                                for (flag, path) in files.iter().filter(|(_, p)| q.is_empty() || p.to_lowercase().contains(&q)) {
                                    n += 1;
                                    if n > 1500 {
                                        l.append(&row("…", "Liste tronquée : affinez le filtre"));
                                        break;
                                    }
                                    let r = adap::ActionRow::new();
                                    r.set_use_markup(false);
                                    r.set_title(path);
                                    r.set_title_lines(1);
                                    r.add_css_class("ns-mono");
                                    let (sign, class, tip) = match flag {
                                        '+' => ("+", "ns-diff-add", "créé"),
                                        '-' => ("−", "ns-diff-del", "supprimé"),
                                        _ => ("~", "ns-diff-up", "modifié"),
                                    };
                                    let s = label(sign, &[class]);
                                    s.set_width_chars(2);
                                    s.set_tooltip_text(Some(tip));
                                    r.add_prefix(&s);
                                    l.append(&r);
                                }
                            }
                        };
                        fill("");
                        search.connect_search_changed(move |s| fill(&s.text()));
                        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        b.append(&search);
                        b.append(&l);
                        fi_page.append(&scrolled(b.upcast_ref()));
                    }
                }
            });
        }
    }

    fn fill_settings(self: &Rc<Self>, st: &Status) {
        let ro = st.boot_snapshot.is_some();

        // Automatiques.
        let g = adap::PreferencesGroup::new();
        g.set_title("Instantanés automatiques");
        g.set_description(Some("Marqués « Auto » dans la liste. Les plus anciens sont supprimés automatiquement, sauf ceux à conserver."));
        let dnf = adap::SwitchRow::new();
        dnf.set_title("Avant et après chaque opération dnf");
        dnf.set_subtitle("Installation, suppression ou mise à jour de paquets (Logiciels, terminal, mises à jour…)");
        dnf.set_active(st.dnf_snapshots);
        {
            let app = self.clone();
            dnf.connect_active_notify(move |r| {
                let on = r.is_active();
                app.run(&["dnf", if on { "on" } else { "off" }], if on { "Instantanés dnf activés" } else { "Instantanés dnf désactivés" });
            });
        }
        g.add(&dnf);
        let keep = adap::SpinRow::with_range(2.0, 50.0, 1.0);
        keep.set_title("Opérations dnf conservées");
        keep.set_subtitle("Chaque opération = une paire avant / après");
        keep.set_value(f64::from(st.number("NUMBER_LIMIT", 10)));
        self.on_spin(&keep, "NUMBER_LIMIT");
        g.add(&keep);

        let timeline = adap::ExpanderRow::new();
        timeline.set_title("Instantanés réguliers");
        timeline.set_subtitle("Un instantané toutes les heures, avec un nombre limité gardé par période");
        timeline.set_show_enable_switch(true);
        timeline.set_enable_expansion(st.get("TIMELINE_CREATE") == "yes");
        {
            let app = self.clone();
            timeline.connect_enable_expansion_notify(move |r| {
                let on = r.enables_expansion();
                app.run(&["set", "TIMELINE_CREATE", if on { "yes" } else { "no" }], if on { "Instantanés réguliers activés" } else { "Instantanés réguliers désactivés" });
            });
        }
        for (key, title, default) in [
            ("TIMELINE_LIMIT_HOURLY", "Horaires conservés", 5),
            ("TIMELINE_LIMIT_DAILY", "Quotidiens conservés", 7),
            ("TIMELINE_LIMIT_WEEKLY", "Hebdomadaires conservés", 2),
            ("TIMELINE_LIMIT_MONTHLY", "Mensuels conservés", 1),
        ] {
            let s = adap::SpinRow::with_range(0.0, 30.0, 1.0);
            s.set_title(title);
            s.set_value(f64::from(st.number(key, default)));
            self.on_spin(&s, key);
            timeline.add_row(&s);
        }
        g.add(&timeline);
        g.set_sensitive(!ro);
        self.settings.append(&g);

        // Menu de démarrage.
        let g = adap::PreferencesGroup::new();
        g.set_title("Menu de démarrage");
        g.set_description(Some(
            "Au démarrage, « Restaurer depuis un instantané » liste les instantanés avec leur date, heure et nom. \
             L'instantané choisi démarre en lecture seule avec une couche temporaire : rien n'est modifié, et vous pouvez ensuite le garder définitivement depuis cette application.",
        ));
        let grub = adap::SwitchRow::new();
        grub.set_title("Restaurer depuis un instantané");
        grub.set_subtitle("Sous-menu dans le menu de démarrage (GRUB)");
        grub.set_active(st.grub_snapshots);
        {
            let app = self.clone();
            grub.connect_active_notify(move |r| {
                let on = r.is_active();
                app.run(&["grub", if on { "on" } else { "off" }], if on { "Sous-menu de démarrage activé" } else { "Sous-menu de démarrage désactivé" });
            });
        }
        g.add(&grub);
        let max = adap::SpinRow::with_range(1.0, 100.0, 1.0);
        max.set_title("Instantanés dans le menu");
        max.set_subtitle("Les plus récents, tous types confondus");
        max.set_value(f64::from(st.grub_max));
        {
            let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
            let app = self.clone();
            max.connect_value_notify(move |s| {
                if let Some(id) = pending.borrow_mut().take() {
                    id.remove();
                }
                let (app2, value, pending2) = (app.clone(), (s.value() as u32).to_string(), pending.clone());
                *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                    pending2.borrow_mut().take();
                    app2.run(&["grub-max", &value], "Menu de démarrage mis à jour");
                }));
            });
        }
        g.add(&max);
        let refresh = row("Mettre à jour le menu maintenant", "Normalement automatique à chaque nouvel instantané");
        refresh.add_prefix(&gtk::Image::from_icon_name("view-refresh-symbolic"));
        refresh.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        refresh.set_activatable(true);
        {
            let app = self.clone();
            refresh.connect_activated(move |_| app.run(&["grub", "refresh"], "Menu de démarrage mis à jour"));
        }
        g.add(&refresh);
        g.set_sensitive(!ro);
        self.settings.append(&g);

        // Informations.
        let g = adap::PreferencesGroup::new();
        g.set_title("Informations");
        g.add(&info_row("Ce qui est sauvegardé", "Tout le système (/), dont /etc et les logiciels installés. Vos dossiers personnels (/home) et /boot ne sont pas concernés."));
        g.add(&info_row("Système de fichiers", &format!("Btrfs · {}", if st.device.is_empty() { "?" } else { &st.device })));
        if let Some((size, used)) = st.disk {
            g.add(&info_row(
                "Espace disque",
                &format!("{} utilisés sur {} ({} %) — les instantanés partagent les fichiers inchangés", human_size(used), human_size(size), used * 100 / size.max(1)),
            ));
        }
        g.add(&info_row("Emplacement", "/.snapshots (configuration snapper « root »)"));
        self.settings.append(&g);
    }

    /// Enregistre une valeur numérique une seconde après le dernier changement.
    fn on_spin(self: &Rc<Self>, s: &adap::SpinRow, key: &'static str) {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let app = self.clone();
        s.connect_value_notify(move |s| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (app2, value, pending2) = (app.clone(), (s.value() as u32).to_string(), pending.clone());
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                pending2.borrow_mut().take();
                app2.run(&["set", key, &value], "Réglage enregistré");
            }));
        });
    }
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::empty()).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    app.connect_activate(|gapp| {
        if let Some(w) = gapp.active_window() {
            w.present();
            return;
        }
        let a = App::new(gapp);
        if std::env::var("NOOS_SAVE_PAGE").is_ok_and(|p| p == "settings") {
            a.views.set_visible_child_name("settings");
        }
        a.window.present();
        a.reload();
        if let Some(y) = std::env::var("NOOS_SAVE_SCROLL").ok().and_then(|v| v.parse::<f64>().ok()) {
            let a2 = a.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || a2.scroll.vadjustment().set_value(y));
        }
        if let Some(n) = std::env::var("NOOS_SAVE_COMPARE").ok().and_then(|v| v.parse::<u32>().ok()) {
            let a2 = a.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                let e = a2.status.borrow().list().iter().find(|e| e.numbers().contains(&n)).cloned();
                if let Some(e) = e {
                    a2.compare(&e);
                }
            });
        }
        let keep = RefCell::new(Some(a.clone()));
        a.window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
    });
    app.run()
}

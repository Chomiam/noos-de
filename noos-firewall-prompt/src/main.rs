//! noos-firewall-prompt — agent de session du pare-feu NOOS.
//!
//! Lancé à l'ouverture de la session (`/etc/xdg/autostart/org.noos.FirewallPrompt.desktop`),
//! il reste connecté au démon (`/run/noos-firewall/control.sock`) et affiche une fenêtre
//! « Alerte de sécurité » pour chaque connexion en attente : programme, icône, hôte, port,
//! protocole ; Autoriser / Bloquer, une fois ou toujours. Après une décision « toujours », la
//! fenêtre propose de révoquer la règle créée.
//!
//! Démonstration sans démon : `NOOS_FIREWALL_DEMO=1` (sortie), `in` (entrée), `done` (écran
//! « règle créée » après une décision simulée).

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_firewall::{
    model::{Action, Decision, Direction, Duration as Dur, Program, Prompt, Scope},
    proto::{self, DecideResult, Event, Request},
};

const APP_ID: &str = "org.noos.FirewallPrompt";

const CSS: &str = r#"
.fw-band { background-color: @accent_bg_color; color: @accent_fg_color; padding: 10px 18px; font-weight: 600; }
.fw-band.fw-in { background-color: #c42b1c; color: white; }
.fw-body { padding: 18px 24px 8px 24px; }
.fw-title { font-size: 18px; font-weight: 700; }
.fw-app { font-size: 15px; font-weight: 700; }
.fw-path { font-size: 12px; }
.fw-key { color: alpha(@window_fg_color, 0.65); }
.fw-value { font-weight: 600; }
.fw-details { background-color: alpha(@window_fg_color, 0.04); border-radius: 10px; padding: 12px 14px; }
.fw-buttons { padding: 12px 24px 18px 24px; background-color: alpha(@window_fg_color, 0.04); }
.fw-buttons button { min-height: 36px; min-width: 120px; }
.fw-countdown { font-size: 12px; }
.fw-done-icon { color: @success_color; }
.fw-done-icon.fw-blocked { color: @error_color; }
"#;

/// Exécution bloquante hors du fil de l'interface.
fn bg<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("tâche interrompue") }
}

/// Nom et icône lisibles d'un programme : fichier .desktop (Flatpak, exécutable, nom court).
fn app_identity(p: &Program) -> (String, gio::Icon) {
    let fallback = || -> gio::Icon { gio::ThemedIcon::new("application-x-executable").upcast() };
    if let Some(id) = &p.flatpak
        && let Some(info) = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop"))
    {
        return (info.name().to_string(), shown_icon(info.icon()));
    }
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let exe_base = base(&p.exe);
    for info in gio::AppInfo::all() {
        // Pas executable() : il plante sur une entrée sans Exec (pointeur nul non vérifié).
        let Some(cmd) = info.commandline() else {
            continue;
        };
        let cmd = cmd.to_string_lossy().to_string();
        let e = cmd.split_whitespace().next().unwrap_or("").trim_matches('"').to_string();
        if e.is_empty() {
            continue;
        }
        let resolved = glib::find_program_in_path(&*e).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| e.to_string());
        if resolved == p.exe || (!exe_base.is_empty() && base(&e) == exe_base) || (!p.comm.is_empty() && base(&e) == p.comm) {
            return (info.name().to_string(), shown_icon(info.icon()));
        }
    }
    let name = if !p.comm.is_empty() {
        p.comm.clone()
    } else if !exe_base.is_empty() {
        exe_base
    } else {
        "Programme inconnu".into()
    };
    // Nom de processus en minuscules (« steam ») : majuscule initiale, comme un nom d'appli.
    let mut c = name.chars();
    let name = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(name);
    (name, fallback())
}

/// Icône affichable. Une icône nommée absente du thème, ou un fichier que GTK 4 ne lit plus
/// (XPM : `/usr/share/pixmaps/python3.14.xpm`), laisserait un vide : on cherche alors le même
/// nom dans le thème (`python3.14` → `python3` → `python`), sinon l'icône générique.
fn shown_icon(icon: Option<gio::Icon>) -> gio::Icon {
    let fallback = || -> gio::Icon { gio::ThemedIcon::new("application-x-executable").upcast() };
    let Some(display) = gtk::gdk::Display::default() else { return icon.unwrap_or_else(fallback) };
    let theme = gtk::IconTheme::for_display(&display);
    // Le thème cherche aussi dans /usr/share/pixmaps : un nom peut « exister » et ne mener
    // qu'à un XPM illisible. On regarde le fichier réellement trouvé.
    let usable = |name: &str| {
        theme.has_icon(name)
            && theme
                .lookup_icon(name, &[], 48, 1, gtk::TextDirection::Ltr, gtk::IconLookupFlags::empty())
                .file()
                .and_then(|f| f.path())
                .is_none_or(|p| !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xpm")))
    };
    let Some(icon) = icon else { return fallback() };
    if let Some(f) = icon.downcast_ref::<gio::FileIcon>() {
        let path = f.file().path().unwrap_or_default();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if path.exists() && matches!(ext.as_str(), "png" | "svg" | "svgz" | "jpg" | "jpeg" | "webp") {
            return icon;
        }
        let mut name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        while !name.is_empty() {
            if usable(&name) {
                return gio::ThemedIcon::new(&name).upcast();
            }
            // python3.14 → python3 → python
            let cut = name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches(['.', '-', '_']).to_string();
            if cut == name {
                break;
            }
            name = cut;
        }
        return fallback();
    }
    if let Some(t) = icon.downcast_ref::<gio::ThemedIcon>() {
        return match t.names().iter().find(|n| usable(n)) {
            Some(n) => gio::ThemedIcon::new(n).upcast(),
            None => fallback(),
        };
    }
    icon
}

/// Nom du service d'un port (`/etc/services`), ex. 443/tcp → https.
fn service_name(port: u16, proto: &str) -> Option<String> {
    let text = std::fs::read_to_string("/etc/services").ok()?;
    let want = format!("{port}/{proto}");
    text.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let name = f.next()?;
        (f.next()? == want).then(|| name.to_string())
    })
}

struct Ui {
    app: adap::Application,
    window: RefCell<Option<adap::ApplicationWindow>>,
    queue: RefCell<VecDeque<Prompt>>,
    /// Demande affichée (et son minuteur).
    current: RefCell<Option<u64>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Ui {
    fn window(self: &Rc<Self>) -> adap::ApplicationWindow {
        if let Some(w) = self.window.borrow().as_ref() {
            return w.clone();
        }
        let w = adap::ApplicationWindow::builder().application(&self.app).title("Pare-feu NOOS").default_width(560).resizable(false).build();
        w.add_css_class("fw-prompt");
        let ui = self.clone();
        w.connect_close_request(move |w| {
            // Fermer = laisser la décision par défaut s'appliquer (comme « Plus tard »).
            ui.stop_timer();
            if let Some(id) = ui.current.borrow_mut().take() {
                ui.queue.borrow_mut().retain(|p| p.id != id);
            }
            *ui.window.borrow_mut() = None;
            let ui2 = ui.clone();
            glib::idle_add_local_once(move || ui2.show_next());
            w.set_visible(false);
            glib::Propagation::Proceed
        });
        *self.window.borrow_mut() = Some(w.clone());
        w
    }

    fn stop_timer(&self) {
        if let Some(t) = self.timer.borrow_mut().take() {
            t.remove();
        }
    }

    fn on_event(self: &Rc<Self>, ev: Event) {
        match ev {
            Event::Prompt(p) => {
                let mut q = self.queue.borrow_mut();
                if !q.iter().any(|x| x.id == p.id) {
                    q.push_back(p);
                }
                drop(q);
                if self.current.borrow().is_none() {
                    self.show_next();
                } else {
                    self.update_counter();
                }
            }
            Event::Done { id } => {
                self.queue.borrow_mut().retain(|p| p.id != id);
                if *self.current.borrow() == Some(id) {
                    self.stop_timer();
                    *self.current.borrow_mut() = None;
                    self.show_next();
                } else {
                    self.update_counter();
                }
            }
        }
    }

    fn update_counter(self: &Rc<Self>) {
        if let Some(w) = self.window.borrow().as_ref()
            && let Some(l) = find_named::<gtk::Label>(w.upcast_ref(), "counter")
        {
            let n = self.queue.borrow().len();
            l.set_label(&if n > 1 { format!("{n} demandes en attente") } else { String::new() });
        }
    }

    fn show_next(self: &Rc<Self>) {
        let next = self.queue.borrow().front().cloned();
        match next {
            Some(p) => self.show_prompt(p),
            None => {
                if let Some(w) = self.window.borrow_mut().take() {
                    w.destroy();
                }
            }
        }
    }

    fn show_prompt(self: &Rc<Self>, p: Prompt) {
        self.stop_timer();
        *self.current.borrow_mut() = Some(p.id);
        let w = self.window();
        let incoming = p.conn.direction == Direction::In;
        let (name, icon) = app_identity(&p.program);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let band = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        band.add_css_class("fw-band");
        if incoming {
            band.add_css_class("fw-in");
        }
        band.append(&gtk::Image::from_icon_name("security-high-symbolic"));
        let bl = gtk::Label::new(Some("Alerte de sécurité — Pare-feu NOOS"));
        bl.set_hexpand(true);
        bl.set_xalign(0.0);
        band.append(&bl);
        let counter = gtk::Label::new(None);
        counter.set_widget_name("counter");
        band.append(&counter);
        root.append(&band);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.add_css_class("fw-body");
        let title = gtk::Label::new(Some(&if incoming { format!("{name} veut accepter des connexions entrantes") } else { format!("{name} veut se connecter au réseau") }));
        title.add_css_class("fw-title");
        title.set_wrap(true);
        title.set_xalign(0.0);
        body.append(&title);

        // Programme : icône, nom, chemin.
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let img = gtk::Image::from_gicon(&icon);
        img.set_pixel_size(56);
        head.append(&img);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_valign(gtk::Align::Center);
        let n = gtk::Label::new(Some(&name));
        n.add_css_class("fw-app");
        n.set_xalign(0.0);
        names.append(&n);
        let path =
            p.program.flatpak.as_ref().map(|id| format!("Flatpak · {id}")).unwrap_or_else(|| if p.program.exe.is_empty() { p.program.key.clone() } else { p.program.exe.clone() });
        let pl = gtk::Label::new(Some(&path));
        pl.add_css_class("fw-path");
        pl.add_css_class("dim-label");
        pl.set_xalign(0.0);
        pl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        pl.set_max_width_chars(48);
        pl.set_tooltip_text(Some(&p.program.cmdline));
        names.append(&pl);
        head.append(&names);
        body.append(&head);

        // Détails de la connexion.
        let grid = gtk::Grid::builder().column_spacing(16).row_spacing(6).build();
        grid.add_css_class("fw-details");
        let proto_s = p.conn.proto.label();
        let port = p.conn.port();
        let svc = service_name(port, if p.conn.ip_proto == 17 { "udp" } else { "tcp" });
        let host_value = gtk::Label::new(Some(&p.conn.remote));
        let rows: Vec<(&str, gtk::Label)> = vec![
            (if incoming { "Depuis" } else { "Destination" }, host_value.clone()),
            (if incoming { "Port local" } else { "Port" }, gtk::Label::new(Some(&format!("{port} ({proto_s}){}", svc.map(|s| format!(" · {s}")).unwrap_or_default())))),
            ("Sens", gtk::Label::new(Some(if incoming { "Entrant" } else { "Sortant" }))),
            ("Interface", gtk::Label::new(Some(if p.conn.interface.is_empty() { "—" } else { &p.conn.interface }))),
            (
                "Compte",
                gtk::Label::new(Some(&format!("{} · processus {}", if p.program.user.is_empty() { p.program.uid.to_string() } else { p.program.user.clone() }, p.program.pid))),
            ),
        ];
        for (i, (k, v)) in rows.into_iter().enumerate() {
            let kl = gtk::Label::new(Some(k));
            kl.add_css_class("fw-key");
            kl.set_xalign(0.0);
            v.add_css_class("fw-value");
            v.set_xalign(0.0);
            v.set_selectable(true);
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.set_hexpand(true);
            grid.attach(&kl, 0, i as i32, 1, 1);
            grid.attach(&v, 1, i as i32, 1, 1);
        }
        body.append(&grid);
        // Nom d'hôte (résolution inverse, en arrière-plan).
        if let Some(addr) = gio::InetAddress::from_string(&p.conn.remote) {
            let (hv, ip) = (host_value.clone(), p.conn.remote.clone());
            glib::spawn_future_local(async move {
                if let Ok(name) = gio::Resolver::default().lookup_by_address_future(&addr).await
                    && name != ip
                {
                    hv.set_label(&format!("{name} ({ip})"));
                }
            });
        }

        // Retenir le choix et sa portée.
        let prefs = adap::PreferencesGroup::new();
        let remember = adap::SwitchRow::builder().title("Retenir ce choix").subtitle("Sinon, la décision ne vaut que pour cette connexion").active(true).build();
        prefs.add(&remember);
        let mut scopes: Vec<(Scope, String)> = vec![
            (Scope::Program, format!("{name}, toutes les connexions{}", if incoming { " entrantes" } else { "" })),
            (Scope::ProgramPort, format!("{name}, port {port} seulement")),
            (Scope::ProgramDestination, format!("{name}, {} port {port} seulement", p.conn.remote)),
        ];
        if incoming {
            scopes.push((Scope::Port, format!("Le port {port} ({proto_s}) pour tous les programmes")));
        }
        let labels: Vec<&str> = scopes.iter().map(|(_, s)| s.as_str()).collect();
        let scope_row = adap::ComboRow::builder().title("S'applique à").model(&gtk::StringList::new(&labels)).build();
        remember.bind_property("active", &scope_row, "sensitive").sync_create().build();
        prefs.add(&scope_row);
        body.append(&prefs);
        root.append(&body);

        // Boutons + compte à rebours.
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.add_css_class("fw-buttons");
        let countdown = gtk::Label::new(None);
        countdown.add_css_class("fw-countdown");
        countdown.add_css_class("dim-label");
        countdown.set_hexpand(true);
        countdown.set_xalign(0.0);
        countdown.set_wrap(true);
        buttons.append(&countdown);
        let block = gtk::Button::with_label("Bloquer");
        let allow = gtk::Button::new();
        let allow_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        allow_box.set_halign(gtk::Align::Center);
        if p.needs_admin {
            // Bouclier : autoriser demande le mot de passe administrateur (polkit).
            allow_box.append(&gtk::Image::from_icon_name("security-high-symbolic"));
            allow.set_tooltip_text(Some("Demande les droits administrateur"));
        }
        allow_box.append(&gtk::Label::new(Some("Autoriser")));
        allow.set_child(Some(&allow_box));
        allow.add_css_class("suggested-action");
        block.add_css_class(if incoming { "destructive-action" } else { "flat" });
        buttons.append(&block);
        buttons.append(&allow);
        root.append(&buttons);

        w.set_content(Some(&root));
        self.update_counter();

        let left = Rc::new(std::cell::Cell::new(p.deadline_secs));
        let default_label = if p.default_action == Action::Allow { "autoriser" } else { "bloquer" };
        let tick = {
            let (countdown, left) = (countdown.clone(), left.clone());
            move || countdown.set_label(&format!("Sans réponse dans {} s : {default_label} pour cette fois.", left.get()))
        };
        tick();
        let tick2 = tick.clone();
        let left2 = left.clone();
        *self.timer.borrow_mut() = Some(glib::timeout_add_seconds_local(1, move || {
            left2.set(left2.get().saturating_sub(1));
            tick2();
            glib::ControlFlow::Continue
        }));

        let decide = {
            let (ui, remember, scope_row, block, allow) = (self.clone(), remember.clone(), scope_row.clone(), block.clone(), allow.clone());
            let scopes: Vec<Scope> = scopes.iter().map(|(s, _)| *s).collect();
            let (p, name) = (p.clone(), name.clone());
            Rc::new(move |action: Action| {
                let d = Decision {
                    action,
                    duration: if remember.is_active() { Dur::Always } else { Dur::Once },
                    scope: scopes.get(scope_row.selected() as usize).copied().unwrap_or_default(),
                    all_users: false,
                };
                block.set_sensitive(false);
                allow.set_sensitive(false);
                let (ui, p, name, block, allow) = (ui.clone(), p.clone(), name.clone(), block.clone(), allow.clone());
                glib::spawn_future_local(async move {
                    let id = p.id;
                    match bg(move || proto::decide(id, d)).await {
                        Ok(res) => ui.after_decision(&p, &name, d, res),
                        Err(e) => {
                            block.set_sensitive(true);
                            allow.set_sensitive(true);
                            ui.toast(&format!("{e:#}"));
                        }
                    }
                });
            })
        };
        let d1 = decide.clone();
        block.connect_clicked(move |_| d1(Action::Block));
        let d2 = decide.clone();
        allow.connect_clicked(move |_| d2(Action::Allow));
        allow.grab_focus();

        if std::env::var("NOOS_FIREWALL_DEMO").as_deref() == Ok("done") {
            let d3 = decide.clone();
            glib::timeout_add_local_once(Duration::from_millis(800), move || d3(Action::Allow));
        }
        w.present();
    }

    fn toast(&self, text: &str) {
        let d = adap::AlertDialog::new(Some("Décision non appliquée"), Some(text));
        d.add_response("ok", "OK");
        if let Some(w) = self.window.borrow().as_ref() {
            d.present(Some(w));
        }
    }

    /// Décision prise : écran de confirmation avec « Révoquer » si une règle a été créée.
    fn after_decision(self: &Rc<Self>, p: &Prompt, name: &str, d: Decision, res: DecideResult) {
        self.stop_timer();
        self.queue.borrow_mut().retain(|x| x.id != p.id);
        *self.current.borrow_mut() = None;
        if res.app_rule.is_none() && res.port_rule.is_none() {
            self.show_next();
            return;
        }
        let w = self.window();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("fw-body");
        body.set_margin_top(12);
        let allowed = d.action == Action::Allow;
        let icon = gtk::Image::from_icon_name(if allowed { "emblem-ok-symbolic" } else { "action-unavailable-symbolic" });
        icon.set_pixel_size(48);
        icon.add_css_class("fw-done-icon");
        if !allowed {
            icon.add_css_class("fw-blocked");
        }
        body.append(&icon);
        let t = gtk::Label::new(Some(&format!("{name} est {}", if allowed { "autorisé" } else { "bloqué" })));
        t.add_css_class("fw-title");
        body.append(&t);
        let what = match (res.port_rule, d.scope) {
            (Some(_), _) => format!("Le port {} ({}) est ouvert pour tous les programmes.", p.conn.port(), p.conn.proto.label()),
            (_, Scope::ProgramPort) => format!("Règle permanente pour le port {}.", p.conn.port()),
            (_, Scope::ProgramDestination) => format!("Règle permanente pour {} port {}.", p.conn.remote, p.conn.port()),
            _ => "Règle permanente pour toutes ses connexions.".into(),
        };
        let s = gtk::Label::new(Some(&format!("{what} Vous pouvez la révoquer maintenant ou plus tard dans Paramètres › Confidentialité et sécurité › Pare-feu.")));
        s.set_wrap(true);
        s.set_justify(gtk::Justification::Center);
        s.add_css_class("dim-label");
        body.append(&s);
        root.append(&body);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.add_css_class("fw-buttons");
        buttons.set_halign(gtk::Align::Fill);
        let manage = gtk::Button::with_label("Gérer le pare-feu");
        manage.add_css_class("flat");
        manage.set_hexpand(true);
        manage.set_halign(gtk::Align::Start);
        let revoke = gtk::Button::with_label("Révoquer");
        revoke.add_css_class("destructive-action");
        let close = gtk::Button::with_label("Fermer");
        close.add_css_class("suggested-action");
        buttons.append(&manage);
        buttons.append(&revoke);
        buttons.append(&close);
        root.append(&buttons);
        w.set_content(Some(&root));
        w.present();

        let ui = self.clone();
        let next = Rc::new(move || {
            ui.stop_timer();
            ui.show_next();
        });
        let n1 = next.clone();
        close.connect_clicked(move |_| n1());
        let n2 = next.clone();
        manage.connect_clicked(move |_| {
            let _ =
                gio::AppInfo::create_from_commandline("noos-settings firewall", None, gio::AppInfoCreateFlags::NONE).and_then(|a| a.launch(&[], None::<&gio::AppLaunchContext>));
            n2();
        });
        let (n3, rev_btn) = (next.clone(), revoke.clone());
        let ui2 = self.clone();
        revoke.connect_clicked(move |_| {
            rev_btn.set_sensitive(false);
            let req = match (res.app_rule, res.port_rule) {
                (Some(id), _) => Request::DeleteAppRule { id },
                (_, Some(id)) => Request::DeletePortRule { id },
                _ => return,
            };
            let (n3, ui2, rev_btn) = (n3.clone(), ui2.clone(), rev_btn.clone());
            glib::spawn_future_local(async move {
                match bg(move || proto::call(&req)).await {
                    Ok(_) => n3(),
                    Err(e) => {
                        rev_btn.set_sensitive(true);
                        ui2.toast(&format!("{e:#}"));
                    }
                }
            });
        });
        // Fermeture automatique si rien d'autre ne se passe.
        let n4 = next.clone();
        *self.timer.borrow_mut() = Some(glib::timeout_add_seconds_local_once(if proto::demo() { 600 } else { 20 }, move || n4()));
    }
}

fn find_named<T: IsA<gtk::Widget>>(w: &gtk::Widget, name: &str) -> Option<T> {
    if w.widget_name() == name {
        return w.clone().downcast().ok();
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        if let Some(f) = find_named(&ch, name) {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE).build();
    let started = Rc::new(std::cell::Cell::new(false));
    app.connect_command_line(move |app, _| {
        if started.replace(true) {
            return glib::ExitCode::SUCCESS;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&gdk::Display::default().expect("écran"), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        // Agent permanent : l'application vit sans fenêtre.
        std::mem::forget(app.hold());
        let ui = Rc::new(Ui { app: app.clone(), window: RefCell::new(None), queue: RefCell::new(VecDeque::new()), current: RefCell::new(None), timer: RefCell::new(None) });
        let (tx, rx) = async_channel::unbounded::<Event>();
        std::thread::spawn(move || {
            let mut warned = false;
            loop {
                let tx2 = tx.clone();
                let r = proto::agent(move |ev| {
                    let _ = tx2.send_blocking(ev);
                });
                // Démon redémarré (mise à jour du paquet) : reconnexion rapide, message une seule fois.
                match r {
                    Err(e) if !warned => {
                        eprintln!("noos-firewall-prompt : {e:#} — nouvelles tentatives toutes les 2 s");
                        warned = true;
                    }
                    Err(_) => {}
                    Ok(()) => warned = false,
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                ui.on_event(ev);
            }
        });
        glib::ExitCode::SUCCESS
    });
    app.run()
}

//! Confidentialité et sécurité › Pare-feu : pare-feu applicatif de NOOS (`noos-firewalld`,
//! nftables + NFQUEUE). État, mode, règles par application, ports, demandes en attente, journal
//! des connexions bloquées. Tout passe par le socket du démon ; les modifications qui touchent
//! le système demandent le mot de passe administrateur (polkit, fenêtre du shell).
//!
//! `NOOS_FIREWALL_DEMO=1` : données fictives (aperçu, captures).

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gio, glib};
use noos_core::paths;
use noos_firewall::{
    model::*,
    proto::{self, Request},
};

use crate::ui::{App, background, card, combo, group, link, page, spin, switch};

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Confidentialité et sécurité");
    let g = group("Sécurité");
    g.add(&link(app, "security-high-symbolic", "Pare-feu et protection du réseau", "Applications autorisées, ports ouverts, connexions bloquées", firewall_page));
    g.add(&link(app, "software-update-available-symbolic", "Mises à jour", "Les mises à jour de sécurité sont gérées par Logiciels", crate::other::updates));
    content.append(&g);
    p
}

pub fn firewall_page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Confidentialité et sécurité"], "Pare-feu et protection du réseau");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    load(app, &holder);
    p
}

/// Tout ce que la page affiche, lu d'un coup hors du fil de l'interface.
struct Snapshot {
    status: Status,
    rules: Rules,
    pending: Vec<Prompt>,
    log: Vec<LogEntry>,
}

fn fetch() -> Result<Snapshot, String> {
    let e = |e: anyhow::Error| format!("{e:#}");
    Ok(Snapshot { status: proto::status().map_err(e)?, rules: proto::rules().map_err(e)?, pending: proto::pending().map_err(e)?, log: proto::log(40).map_err(e)? })
}

fn clear(holder: &gtk::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
}

fn load(app: &Rc<App>, holder: &gtk::Box) {
    let task = background(fetch);
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let snap = task.await;
        clear(&holder);
        match snap {
            Ok(s) => render(&app, &holder, s),
            Err(e) => render_down(&app, &holder, &e),
        }
    });
}

/// Requête de modification, puis rechargement de la page (succès ou échec).
fn send(app: &Rc<App>, holder: &gtk::Box, title: &str, req: Request) {
    let task = background(move || proto::call(&req).map_err(|e| format!("{e:#}")));
    let (app, holder, title) = (app.clone(), holder.clone(), title.to_string());
    glib::spawn_future_local(async move {
        if let Err(e) = task.await {
            app.error(&title, &e);
        }
        load(&app, &holder);
    });
}

fn shield_hero(file: &str, state: &str, title: &str, subtitle: &str) -> (gtk::Box, gtk::Box, gtk::Box) {
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    hero.add_css_class("win11-hero");
    hero.add_css_class("fw-hero");
    let img = gtk::Image::from_file(paths::data_dir().join("icons").join(file));
    img.set_pixel_size(112);
    hero.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("title-1");
    t.add_css_class(state);
    t.set_xalign(0.0);
    t.set_wrap(true);
    v.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.set_xalign(0.0);
    s.set_wrap(true);
    v.append(&s);
    hero.append(&v);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_valign(gtk::Align::Center);
    hero.append(&actions);
    (hero, v, actions)
}

/// Le démon ne répond pas : installé mais arrêté, ou absent.
fn render_down(app: &Rc<App>, holder: &gtk::Box, err: &str) {
    let installed = ["/usr/lib/systemd/system/noos-firewall.service", "/etc/systemd/system/noos-firewall.service"].iter().any(|p| std::path::Path::new(p).exists());
    let (hero, v, actions) = shield_hero(
        "shield-bad.svg",
        "fw-state-bad",
        if installed { "Le pare-feu est arrêté" } else { "Le pare-feu NOOS n'est pas installé" },
        if installed {
            "Le service noos-firewall ne fonctionne pas : les connexions entrantes restent bloquées, mais les applications ne sont plus contrôlées."
        } else {
            "Installez ou mettez à jour le paquet noos pour protéger cet appareil."
        },
    );
    let d = gtk::Label::new(Some(err));
    d.add_css_class("fw-backend");
    d.set_xalign(0.0);
    d.set_selectable(true);
    v.append(&d);
    if installed {
        let b = gtk::Button::with_label("Démarrer le pare-feu");
        b.add_css_class("suggested-action");
        b.add_css_class("pill");
        let (app2, holder2) = (app.clone(), holder.clone());
        b.connect_clicked(move |_| {
            let (app3, holder3) = (app2.clone(), holder2.clone());
            app2.run_plan("Démarrage du pare-feu", Ok(noos_pkg::Plan::new().root(&["systemctl", "enable", "--now", "noos-firewall.service"])), move |_| {
                // Le socket apparaît une fraction de seconde après le démarrage.
                glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || load(&app3, &holder3));
            });
        });
        actions.append(&b);
    }
    holder.append(&hero);
}

fn render(app: &Rc<App>, holder: &gtk::Box, s: Snapshot) {
    let st = &s.status;
    let cfg = st.config.clone();
    let admin = st.is_admin;

    // ── Héros ──
    let (file, class, title, subtitle) = if !cfg.enabled {
        ("shield-bad.svg", "fw-state-bad", "Le pare-feu est désactivé", "Votre appareil accepte toutes les connexions. Activez le pare-feu.".to_string())
    } else if st.nft_error.is_some() {
        ("shield-warn.svg", "fw-state-warn", "Le pare-feu n'a pas pu s'appliquer", "La table nftables a été refusée ; détails ci-dessous.".to_string())
    } else if cfg.mode == Mode::AllowAll {
        ("shield-warn.svg", "fw-state-warn", "Le pare-feu laisse tout passer", "Mode « Tout autoriser » : seules les règles de blocage s'appliquent.".to_string())
    } else if cfg.shields_up {
        ("shield-ok.svg", "fw-state-ok", "Toutes les connexions entrantes sont bloquées", "Même celles des applications autorisées.".to_string())
    } else {
        ("shield-ok.svg", "fw-state-ok", "Le pare-feu est activé", format!("{} — les connexions entrantes non autorisées sont bloquées.", mode_short(cfg.mode)))
    };
    let (hero, v, actions) = shield_hero(file, class, title, &subtitle);
    let stats = gtk::Label::new(Some(&format!(
        "{} connexions bloquées · {} règles d'applications · {} règles de ports{}",
        st.blocked_total,
        st.app_rules,
        st.port_rules,
        if st.agents == 0 { " · aucune fenêtre d'autorisation connectée" } else { "" }
    )));
    stats.add_css_class("fw-backend");
    stats.set_xalign(0.0);
    stats.set_wrap(true);
    v.append(&stats);
    if let Some(e) = &st.nft_error {
        let d = gtk::Label::new(Some(e));
        d.add_css_class("caption");
        d.add_css_class("dim-label");
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_selectable(true);
        v.append(&d);
    }
    let toggle = gtk::Button::with_label(if cfg.enabled { "Désactiver" } else { "Activer" });
    toggle.add_css_class(if cfg.enabled { "destructive-action" } else { "suggested-action" });
    toggle.add_css_class("pill");
    toggle.set_sensitive(admin);
    {
        let (app2, holder2, cfg2) = (app.clone(), holder.clone(), cfg.clone());
        toggle.connect_clicked(move |_| {
            let mut c = cfg2.clone();
            c.enabled = !c.enabled;
            if cfg2.enabled {
                let (app3, holder3) = (app2.clone(), holder2.clone());
                app2.confirm("Désactiver le pare-feu ?", "Votre appareil acceptera toutes les connexions, entrantes comme sortantes.", "Désactiver", true, move || {
                    send(&app3, &holder3, "Désactivation du pare-feu", Request::SetConfig(c.clone()))
                });
            } else {
                send(&app2, &holder2, "Activation du pare-feu", Request::SetConfig(c));
            }
        });
    }
    actions.append(&toggle);
    holder.append(&hero);

    // ── Autres pare-feu actifs ──
    if !st.conflicts.is_empty() {
        let names = st.conflicts.join(" et ");
        let b = adap::Banner::new(&format!("{names} est aussi actif : il filtre en plus de NOOS et peut bloquer des ports ouverts ici."));
        b.set_button_label(Some("Le désactiver"));
        b.set_revealed(true);
        let (app2, holder2, list) = (app.clone(), holder.clone(), st.conflicts.clone());
        b.connect_button_clicked(move |_| {
            let mut plan = noos_pkg::Plan::new();
            for n in &list {
                plan = plan.root_owned(vec!["systemctl".into(), "disable".into(), "--now".into(), format!("{n}.service")]);
                if n == "ufw" {
                    plan = plan.root(&["sed", "-i", "s/^ENABLED=yes/ENABLED=no/", "/etc/ufw/ufw.conf"]);
                }
            }
            let (app3, holder3) = (app2.clone(), holder2.clone());
            app2.run_plan("Désactivation de l'autre pare-feu", Ok(plan), move |_| load(&app3, &holder3));
        });
        holder.append(&b);
    }

    if !cfg.enabled {
        return;
    }

    // ── Demandes en attente ──
    if !s.pending.is_empty() {
        let g = group("Autorisations en attente");
        g.set_description(Some("Connexions retenues en attendant votre décision"));
        for p in &s.pending {
            let (name, icon) = app_identity(&p.program);
            let r = adap::ActionRow::builder().title(&name).subtitle(format!("{} · décision par défaut dans {} s", conn_text(&p.conn), p.deadline_secs)).use_markup(false).build();
            r.add_prefix(&app_icon(&icon));
            for (label, action, css) in [("Bloquer", Action::Block, "flat"), ("Autoriser", Action::Allow, "suggested-action")] {
                let b = gtk::Button::with_label(label);
                b.add_css_class(css);
                b.set_valign(gtk::Align::Center);
                let (app2, holder2, id) = (app.clone(), holder.clone(), p.id);
                b.connect_clicked(move |_| {
                    let d = Decision { action, duration: Duration::Always, scope: Scope::Program, all_users: false };
                    send(&app2, &holder2, "Décision", Request::Decide { id, decision: d });
                });
                r.add_suffix(&b);
            }
            g.add(&r);
        }
        holder.append(&g);
    }

    // ── Comportement ──
    let g = group("Comportement");
    let modes: Vec<String> = Mode::ALL.iter().map(|m| m.label().to_string()).collect();
    let mode_row = combo(
        "dialog-question-symbolic",
        "Nouvelles connexions",
        "Que faire quand aucune règle ne couvre une connexion",
        &modes,
        Mode::ALL.iter().position(|m| *m == cfg.mode).unwrap_or(0),
    );
    let shields =
        switch("security-high-symbolic", "Bloquer toutes les connexions entrantes", "Y compris celles des applications et des ports autorisés (réseau public)", cfg.shields_up);
    let actions_l = vec!["Bloquer".to_string(), "Autoriser".to_string()];
    let default_row = combo(
        "alarm-symbolic",
        "Sans réponse",
        "Décision appliquée pour cette fois quand la fenêtre d'autorisation reste sans réponse",
        &actions_l,
        if cfg.default_action == Action::Allow { 1 } else { 0 },
    );
    let timeout = spin("", "Délai de réponse (secondes)", "", (5.0, 600.0, 5.0), cfg.timeout_secs as f64);
    let no_agent = combo(
        "",
        "Sorties sans fenêtre d'autorisation",
        "Avant l'ouverture de la session ou si la fenêtre n'est pas lancée",
        &actions_l,
        if cfg.no_agent_action == Action::Allow { 1 } else { 0 },
    );
    let system =
        switch("emblem-system-symbolic", "Autoriser les services du système", "NetworkManager, apt, horloge… (comptes système) se connectent sans demander", cfg.allow_system);
    let ping = switch("network-transmit-receive-symbolic", "Répondre au ping", "Les autres appareils peuvent vérifier que celui-ci est en ligne", cfg.allow_ping);
    let lan = switch(
        "network-workgroup-symbolic",
        "Découverte du réseau local",
        "Imprimantes, partages et appareils proches (mDNS, SSDP), réseau local seulement",
        cfg.allow_lan_discovery,
    );
    let trusted =
        adap::EntryRow::builder().title("Interfaces de confiance (machines virtuelles, conteneurs)").text(cfg.trusted_interfaces.join(", ")).show_apply_button(true).build();
    let logb = switch("document-open-recent-symbolic", "Journal des connexions bloquées", "", cfg.log_blocked);
    for w in [
        mode_row.upcast_ref::<gtk::Widget>(),
        shields.upcast_ref(),
        default_row.upcast_ref(),
        timeout.upcast_ref(),
        no_agent.upcast_ref(),
        system.upcast_ref(),
        ping.upcast_ref(),
        lan.upcast_ref(),
        trusted.upcast_ref(),
        logb.upcast_ref(),
    ] {
        w.set_sensitive(admin);
        g.add(w);
    }
    if !admin {
        g.set_description(Some("Réservé aux administrateurs (groupe sudo)"));
    }
    let apply = {
        let (app, holder, cfg) = (app.clone(), holder.clone(), cfg.clone());
        let (mode_row, shields, default_row, timeout, no_agent, system, ping, lan, trusted, logb) =
            (mode_row.clone(), shields.clone(), default_row.clone(), timeout.clone(), no_agent.clone(), system.clone(), ping.clone(), lan.clone(), trusted.clone(), logb.clone());
        Rc::new(move || {
            let c = Config {
                mode: Mode::ALL[mode_row.selected() as usize % Mode::ALL.len()],
                shields_up: shields.is_active(),
                default_action: if default_row.selected() == 1 { Action::Allow } else { Action::Block },
                timeout_secs: timeout.value() as u32,
                no_agent_action: if no_agent.selected() == 1 { Action::Allow } else { Action::Block },
                allow_system: system.is_active(),
                allow_ping: ping.is_active(),
                allow_lan_discovery: lan.is_active(),
                trusted_interfaces: trusted.text().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                log_blocked: logb.is_active(),
                ..cfg.clone()
            };
            if c != cfg {
                send(&app, &holder, "Réglages du pare-feu", Request::SetConfig(c));
            }
        })
    };
    {
        let a = apply.clone();
        mode_row.connect_selected_notify(move |_| a());
        for c in [&default_row, &no_agent] {
            let a = apply.clone();
            c.connect_selected_notify(move |_| a());
        }
        for sw in [&shields, &system, &ping, &lan, &logb] {
            let a = apply.clone();
            sw.connect_active_notify(move |_| a());
        }
        // Spin : appliqué quand on quitte le champ, pas à chaque cran.
        let a = apply.clone();
        let fc = gtk::EventControllerFocus::new();
        fc.connect_leave(move |_| a());
        timeout.add_controller(fc);
        let a = apply.clone();
        trusted.connect_apply(move |_| a());
    }
    holder.append(&g);

    // ── Applications ──
    let g = group("Applications");
    g.set_description(Some("Règles par programme : autorisé ou bloqué, en sortie ou en entrée"));
    let add = gtk::Button::with_label("Ajouter une règle…");
    add.add_css_class("flat");
    {
        let (app2, holder2) = (app.clone(), holder.clone());
        add.connect_clicked(move |_| app_rule_dialog(&app2, &holder2, None, admin));
    }
    g.set_header_suffix(Some(&add));
    let mut apps = s.rules.apps.clone();
    apps.sort_by_key(|r| (r.action != Action::Block, std::cmp::Reverse(r.last_hit)));
    if apps.is_empty() {
        g.add(&card("application-x-executable-symbolic", "Aucune règle", "Les règles apparaissent ici quand vous répondez à une demande du pare-feu"));
    }
    for r in &apps {
        g.add(&app_row(app, holder, r, st.uid, admin));
    }
    holder.append(&g);

    // ── Ports ──
    let g = group("Ports");
    g.set_description(Some("Règles nftables : ouvrir un port à un serveur, un jeu, un partage… (plages et listes acceptées)"));
    let add = gtk::Button::with_label("Ouvrir un port…");
    add.add_css_class("flat");
    add.set_sensitive(admin);
    {
        let (app2, holder2) = (app.clone(), holder.clone());
        add.connect_clicked(move |_| port_rule_dialog(&app2, &holder2, None));
    }
    g.set_header_suffix(Some(&add));
    if s.rules.ports.is_empty() {
        g.add(&card("network-wired-symbolic", "Aucun port ouvert", "Les connexions entrantes passent par les règles d'applications"));
    }
    for r in &s.rules.ports {
        g.add(&port_row(app, holder, r, admin));
    }
    holder.append(&g);

    // ── Journal ──
    let g = group("Connexions bloquées");
    g.set_description(Some("Les plus récentes d'abord ; les répétitions sont regroupées"));
    if admin && !s.log.is_empty() {
        let b = gtk::Button::with_label("Effacer");
        b.add_css_class("flat");
        let (app2, holder2) = (app.clone(), holder.clone());
        b.connect_clicked(move |_| send(&app2, &holder2, "Journal", Request::ClearLog));
        g.set_header_suffix(Some(&b));
    }
    if s.log.is_empty() {
        g.add(&card("emblem-ok-symbolic", "Aucune connexion bloquée", ""));
    }
    for e in &s.log {
        g.add(&log_row(app, holder, e, st.uid));
    }
    holder.append(&g);

    // Captures : NOOS_FIREWALL_DIALOG=port|app ouvre le dialogue d'ajout (une seule fois).
    if let Ok(which) = std::env::var("NOOS_FIREWALL_DIALOG") {
        // SAFETY : fil principal, avant tout autre fil qui lirait l'environnement.
        unsafe { std::env::remove_var("NOOS_FIREWALL_DIALOG") };
        match which.as_str() {
            "port" => port_rule_dialog(app, holder, None),
            "app" => app_rule_dialog(app, holder, None, admin),
            _ => {}
        }
    }
}

fn mode_short(m: Mode) -> &'static str {
    match m {
        Mode::Interactive => "Mode interactif",
        Mode::AllowOutgoing => "Sorties autorisées",
        Mode::Silent => "Mode silencieux",
        Mode::AllowAll => "Tout autoriser",
    }
}

fn conn_text(c: &Conn) -> String {
    let arrow = if c.direction == Direction::In { "←" } else { "→" };
    format!("{arrow} {}  port {} {}", c.remote, c.port(), c.proto.label())
}

fn ago(t: u64) -> String {
    let d = now().saturating_sub(t);
    match d {
        0..60 => "à l'instant".into(),
        60..3600 => format!("il y a {} min", d / 60),
        3600..86_400 => format!("il y a {} h", d / 3600),
        _ => format!("il y a {} j", d / 86_400),
    }
}

/// Nom et icône lisibles : fichier .desktop (Flatpak, exécutable, nom court).
fn app_identity(p: &Program) -> (String, gio::Icon) {
    key_identity(&p.key, &p.comm)
}

fn key_identity(key: &str, hint: &str) -> (String, gio::Icon) {
    let fallback = || -> gio::Icon { gio::ThemedIcon::new("application-x-executable").upcast() };
    if let Some(id) = key.strip_prefix("flatpak:") {
        if let Some(info) = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop")) {
            return (info.name().to_string(), info.icon().unwrap_or_else(fallback));
        }
        let short = id.rsplit('.').next().unwrap_or(id);
        return (if hint.is_empty() { short.to_string() } else { hint.to_string() }, gio::ThemedIcon::new(id).upcast());
    }
    if key.ends_with('*') {
        return (if hint.is_empty() { key.to_string() } else { hint.to_string() }, gio::ThemedIcon::new("folder-symbolic").upcast());
    }
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let kb = base(key);
    for info in gio::AppInfo::all() {
        // Pas executable() : il plante sur une entrée sans Exec (pointeur nul non vérifié).
        let Some(cmd) = info.commandline() else {
            continue;
        };
        let cmd = cmd.to_string_lossy().to_string();
        let e = cmd.split_whitespace().next().unwrap_or("").trim_matches('"').to_string();
        if !e.is_empty() && (base(&e) == kb || (!hint.is_empty() && base(&e) == hint)) {
            return (info.name().to_string(), info.icon().unwrap_or_else(fallback));
        }
    }
    let name = if hint.is_empty() { kb } else { hint.to_string() };
    // Nom de processus en minuscules (« steam ») : majuscule initiale, comme un nom d'appli.
    let mut c = name.chars();
    let name = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or(name);
    (name, fallback())
}

fn app_icon(icon: &gio::Icon) -> gtk::Image {
    let i = gtk::Image::from_gicon(icon);
    i.set_pixel_size(32);
    i
}

fn app_row(app: &Rc<App>, holder: &gtk::Box, r: &AppRule, uid: u32, admin: bool) -> adap::ActionRow {
    let (name, icon) = key_identity(&r.app, &r.label);
    let mut details = vec![r.action.label().to_string(), r.direction.label().to_lowercase()];
    if let Some(p) = &r.port {
        details.push(format!("port {p}{}", if r.proto != Proto::Any { format!(" {}", r.proto.label()) } else { String::new() }));
    }
    if let Some(a) = &r.remote {
        details.push(format!("vers {a}"));
    }
    details.push(match r.user {
        None => "tous les comptes".into(),
        Some(u) if u == uid => "ce compte".into(),
        Some(u) => format!("compte {u}"),
    });
    if r.hits > 0 {
        details.push(format!("{} connexions, dernière {}", r.hits, ago(r.last_hit)));
    }
    let row = adap::ActionRow::builder().title(&name).subtitle(details.join(" · ")).use_markup(false).build();
    row.set_tooltip_text(Some(&r.app));
    row.add_prefix(&app_icon(&icon));
    let badge = gtk::Image::from_icon_name(if r.action == Action::Allow { "emblem-ok-symbolic" } else { "action-unavailable-symbolic" });
    badge.add_css_class(if r.action == Action::Allow { "fw-state-ok" } else { "fw-state-bad" });
    row.add_prefix(&badge);
    let editable = admin || (r.user == Some(uid) && r.direction == Direction::Out);
    let sw = gtk::Switch::builder().active(r.enabled).valign(gtk::Align::Center).tooltip_text("Règle active").sensitive(editable).build();
    {
        let (app2, holder2, r2) = (app.clone(), holder.clone(), r.clone());
        sw.connect_state_set(move |_, on| {
            send(&app2, &holder2, "Règle d'application", Request::SaveAppRule(AppRule { enabled: on, ..r2.clone() }));
            glib::Propagation::Proceed
        });
    }
    row.add_suffix(&sw);
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class("flat");
    edit.set_valign(gtk::Align::Center);
    edit.set_tooltip_text(Some("Modifier"));
    edit.set_sensitive(editable);
    {
        let (app2, holder2, r2) = (app.clone(), holder.clone(), r.clone());
        edit.connect_clicked(move |_| app_rule_dialog(&app2, &holder2, Some(r2.clone()), admin));
    }
    row.add_suffix(&edit);
    let rm = gtk::Button::from_icon_name("user-trash-symbolic");
    rm.add_css_class("flat");
    rm.set_valign(gtk::Align::Center);
    rm.set_tooltip_text(Some("Révoquer et supprimer"));
    rm.set_sensitive(editable);
    {
        let (app2, holder2, id, name2) = (app.clone(), holder.clone(), r.id, name.clone());
        rm.connect_clicked(move |_| {
            let (app3, holder3) = (app2.clone(), holder2.clone());
            app2.confirm(
                &format!("Supprimer la règle de « {name2} » ?"),
                "Le pare-feu demandera à nouveau à la prochaine connexion de ce programme.",
                "Supprimer",
                true,
                move || send(&app3, &holder3, "Suppression de la règle", Request::DeleteAppRule { id }),
            );
        });
    }
    row.add_suffix(&rm);
    row
}

fn port_title(r: &PortRule) -> String {
    let p = r.ports.replace(',', ", ");
    format!("{} {p} ({})", if r.ports.contains([',', '-']) { "Ports" } else { "Port" }, r.proto.label())
}

fn port_row(app: &Rc<App>, holder: &gtk::Box, r: &PortRule, admin: bool) -> adap::ActionRow {
    let mut details = vec![r.action.label().to_string(), r.direction.label().to_lowercase(), r.zone.label().to_lowercase()];
    if let Some(i) = &r.interface {
        details.push(format!("interface {i}"));
    }
    if let Some(a) = &r.remote {
        details.push(format!("{} {a}", if r.direction == Direction::Out { "vers" } else { "depuis" }));
    }
    let title = if r.note.is_empty() { port_title(r) } else { format!("{} — {}", port_title(r), r.note) };
    let row = card(if r.action == Action::Allow { "network-wired-symbolic" } else { "action-unavailable-symbolic" }, &title, &details.join(" · "));
    let sw = gtk::Switch::builder().active(r.enabled).valign(gtk::Align::Center).sensitive(admin).tooltip_text("Règle active").build();
    {
        let (app2, holder2, r2) = (app.clone(), holder.clone(), r.clone());
        sw.connect_state_set(move |_, on| {
            send(&app2, &holder2, "Règle de port", Request::SavePortRule(PortRule { enabled: on, ..r2.clone() }));
            glib::Propagation::Proceed
        });
    }
    row.add_suffix(&sw);
    let edit = gtk::Button::from_icon_name("document-edit-symbolic");
    edit.add_css_class("flat");
    edit.set_valign(gtk::Align::Center);
    edit.set_sensitive(admin);
    edit.set_tooltip_text(Some("Modifier"));
    {
        let (app2, holder2, r2) = (app.clone(), holder.clone(), r.clone());
        edit.connect_clicked(move |_| port_rule_dialog(&app2, &holder2, Some(r2.clone())));
    }
    row.add_suffix(&edit);
    let rm = gtk::Button::from_icon_name("user-trash-symbolic");
    rm.add_css_class("flat");
    rm.set_valign(gtk::Align::Center);
    rm.set_sensitive(admin);
    rm.set_tooltip_text(Some(if r.action == Action::Allow { "Fermer ce port" } else { "Supprimer" }));
    {
        let (app2, holder2, id, t) = (app.clone(), holder.clone(), r.id, port_title(r));
        rm.connect_clicked(move |_| {
            let (app3, holder3) = (app2.clone(), holder2.clone());
            app2.confirm(&format!("Supprimer la règle « {t} » ?"), "La règle nftables est retirée immédiatement.", "Supprimer", true, move || {
                send(&app3, &holder3, "Suppression de la règle", Request::DeletePortRule { id })
            });
        });
    }
    row.add_suffix(&rm);
    row
}

fn log_row(app: &Rc<App>, holder: &gtk::Box, e: &LogEntry, uid: u32) -> adap::ActionRow {
    let known = !matches!(e.program.key.as_str(), "" | "aucun" | "inconnu");
    let (name, icon) = if known {
        app_identity(&e.program)
    } else {
        (if e.program.comm.is_empty() { "Aucun programme".into() } else { e.program.comm.clone() }, gio::ThemedIcon::new("network-error-symbolic").upcast())
    };
    let count = if e.count > 1 { format!(" · ×{}", e.count) } else { String::new() };
    let row = adap::ActionRow::builder().title(&name).subtitle(format!("{} · {}{count} · {}", conn_text(&e.conn), e.reason, ago(e.time))).use_markup(false).build();
    row.add_prefix(&app_icon(&icon));
    if known {
        let b = gtk::Button::with_label("Autoriser");
        b.add_css_class("flat");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some("Créer une règle qui autorise ce programme"));
        let (app2, holder2, e2) = (app.clone(), holder.clone(), e.clone());
        b.connect_clicked(move |_| {
            let own = e2.program.uid == uid && e2.conn.direction == Direction::Out;
            let r = AppRule {
                enabled: true,
                app: e2.program.key.clone(),
                label: e2.program.comm.clone(),
                direction: e2.conn.direction,
                action: Action::Allow,
                user: own.then_some(uid),
                ..Default::default()
            };
            send(&app2, &holder2, "Nouvelle règle", Request::SaveAppRule(r));
        });
        row.add_suffix(&b);
    }
    row
}

// ───────────────────────── Dialogues ─────────────────────────

fn boxed() -> gtk::ListBox {
    let l = gtk::ListBox::new();
    l.add_css_class("boxed-list");
    l.set_selection_mode(gtk::SelectionMode::None);
    l
}

fn mini_combo(title: &str, items: &[&str], selected: usize) -> adap::ComboRow {
    let r = adap::ComboRow::builder().title(title).model(&gtk::StringList::new(items)).build();
    r.set_selected(selected as u32);
    r
}

const DIRS: [Direction; 3] = [Direction::Out, Direction::In, Direction::Both];
const PROTOS: [Proto; 3] = [Proto::Any, Proto::Tcp, Proto::Udp];
const ACTIONS: [Action; 2] = [Action::Allow, Action::Block];

fn idx<T: PartialEq>(list: &[T], v: &T) -> usize {
    list.iter().position(|x| x == v).unwrap_or(0)
}

fn app_rule_dialog(app: &Rc<App>, holder: &gtk::Box, rule: Option<AppRule>, admin: bool) {
    let r = rule.clone().unwrap_or(AppRule { enabled: true, action: Action::Allow, ..Default::default() });
    let list = boxed();
    let prog = adap::EntryRow::builder().title("Programme (chemin, flatpak:ID ou dossier/*)").text(&r.app).build();
    let browse = gtk::Button::from_icon_name("document-open-symbolic");
    browse.add_css_class("flat");
    browse.set_valign(gtk::Align::Center);
    browse.set_tooltip_text(Some("Choisir un programme"));
    prog.add_suffix(&browse);
    list.append(&prog);
    let action = mini_combo("Action", &["Autoriser", "Bloquer"], idx(&ACTIONS, &r.action));
    let dir = mini_combo("Sens", &["Sortant", "Entrant", "Entrant et sortant"], idx(&DIRS, &r.direction));
    let proto_ = mini_combo("Protocole", &["TCP et UDP", "TCP", "UDP"], idx(&PROTOS, &r.proto));
    let port = adap::EntryRow::builder().title("Port (facultatif : 443, 6000-6010, 80,443)").text(r.port.clone().unwrap_or_default()).build();
    let remote = adap::EntryRow::builder().title("Adresse distante (facultative : IP ou réseau CIDR)").text(r.remote.clone().unwrap_or_default()).build();
    let all = adap::SwitchRow::builder().title("Tous les comptes").subtitle("Sinon : seulement le vôtre").active(r.user.is_none() && rule.is_some()).sensitive(admin).build();
    for w in [action.upcast_ref::<gtk::Widget>(), dir.upcast_ref(), proto_.upcast_ref(), port.upcast_ref(), remote.upcast_ref(), all.upcast_ref()] {
        list.append(w);
    }
    {
        let (prog, window) = (prog.clone(), app.window.clone());
        browse.connect_clicked(move |_| {
            let fd = gtk::FileDialog::builder().title("Choisir un programme").initial_folder(&gio::File::for_path("/usr/bin")).build();
            let prog = prog.clone();
            fd.open(Some(&window), None::<&gio::Cancellable>, move |res| {
                if let Ok(f) = res
                    && let Some(p) = f.path()
                {
                    prog.set_text(&p.to_string_lossy());
                }
            });
        });
    }
    let d = adap::AlertDialog::new(Some(if rule.is_some() { "Modifier la règle" } else { "Nouvelle règle d'application" }), None);
    d.set_extra_child(Some(&list));
    d.add_responses(&[("cancel", "Annuler"), ("save", "Enregistrer")]);
    d.set_response_appearance("save", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("save"));
    let (app2, holder2) = (app.clone(), holder.clone());
    d.connect_response(None, move |_, resp| {
        if resp != "save" {
            return;
        }
        let opt = |e: &adap::EntryRow| Some(e.text().trim().to_string()).filter(|s| !s.is_empty());
        let direction = DIRS[dir.selected() as usize % 3];
        let new = AppRule {
            app: prog.text().trim().to_string(),
            action: ACTIONS[action.selected() as usize % 2],
            direction,
            proto: PROTOS[proto_.selected() as usize % 3],
            port: opt(&port),
            remote: opt(&remote),
            user: if all.is_active() { None } else { Some(proto::my_uid()) },
            ..r.clone()
        };
        send(&app2, &holder2, "Règle d'application", Request::SaveAppRule(new));
    });
    d.present(Some(&app.window));
}

fn interfaces() -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir("/sys/class/net").map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != "lo").collect()).unwrap_or_default();
    v.sort();
    v
}

fn port_rule_dialog(app: &Rc<App>, holder: &gtk::Box, rule: Option<PortRule>) {
    let r = rule.clone().unwrap_or(PortRule { enabled: true, direction: Direction::In, action: Action::Allow, proto: Proto::Tcp, ..Default::default() });
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let list = boxed();
    let ports = adap::EntryRow::builder().title("Port, plage ou liste (25565, 6000-6010, 80,443)").text(&r.ports).build();
    ports.set_input_purpose(gtk::InputPurpose::Number);
    let proto_ = mini_combo("Protocole", &["TCP et UDP", "TCP", "UDP"], idx(&PROTOS, &r.proto));
    let dir = mini_combo("Sens", &["Sortant", "Entrant", "Entrant et sortant"], idx(&DIRS, &r.direction));
    let action = mini_combo("Action", &["Autoriser (ouvrir)", "Bloquer"], idx(&ACTIONS, &r.action));
    let zone = mini_combo("Zone", &["Tous les réseaux", "Réseau local seulement"], if r.zone == Zone::Lan { 1 } else { 0 });
    let mut ifs = vec!["Toutes les interfaces".to_string()];
    ifs.extend(interfaces());
    if let Some(i) = &r.interface
        && !ifs.contains(i)
    {
        ifs.push(i.clone());
    }
    let refs: Vec<&str> = ifs.iter().map(String::as_str).collect();
    let iface = mini_combo("Interface", &refs, r.interface.as_ref().and_then(|i| ifs.iter().position(|x| x == i)).unwrap_or(0));
    let remote = adap::EntryRow::builder().title("Adresse distante (facultative : IP ou réseau CIDR)").text(r.remote.clone().unwrap_or_default()).build();
    let note = adap::EntryRow::builder().title("Description (facultative)").text(&r.note).build();
    for w in [
        ports.upcast_ref::<gtk::Widget>(),
        proto_.upcast_ref(),
        dir.upcast_ref(),
        action.upcast_ref(),
        zone.upcast_ref(),
        iface.upcast_ref(),
        remote.upcast_ref(),
        note.upcast_ref(),
    ] {
        list.append(w);
    }
    if rule.is_none() {
        let presets = gtk::FlowBox::new();
        presets.set_selection_mode(gtk::SelectionMode::None);
        presets.set_max_children_per_line(3);
        for (label, p, pr, lan) in [
            ("Minecraft", "25565", 0u32, false),
            ("Serveur web", "8080", 1, false),
            ("SSH", "22", 1, true),
            ("Steam (réseau local)", "27015-27030,27036", 0, true),
            ("KDE Connect", "1714-1764", 0, true),
            ("Sunshine (jeu à distance)", "47984-48010", 0, true),
        ] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("pill");
            let (ports, proto_, zone, note) = (ports.clone(), proto_.clone(), zone.clone(), note.clone());
            b.connect_clicked(move |_| {
                ports.set_text(p);
                proto_.set_selected(pr);
                zone.set_selected(lan as u32);
                note.set_text(label);
            });
            presets.insert(&b, -1);
        }
        bx.append(&presets);
    }
    // Préréglages au-dessus : sous la liste, ils sortaient de la hauteur du dialogue.
    bx.append(&list);
    let d = adap::AlertDialog::new(
        Some(if rule.is_some() { "Modifier la règle de port" } else { "Ouvrir un port" }),
        Some("La règle nftables s'applique immédiatement et reste au redémarrage."),
    );
    d.set_extra_child(Some(&bx));
    d.add_responses(&[("cancel", "Annuler"), ("save", "Enregistrer")]);
    d.set_response_appearance("save", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("save"));
    let (app2, holder2) = (app.clone(), holder.clone());
    d.connect_response(None, move |_, resp| {
        if resp != "save" {
            return;
        }
        let ports_s = match validate_ports(&ports.text()) {
            Ok(p) => p,
            Err(e) => return app2.error("Règle de port", &format!("{e:#}")),
        };
        let opt = |e: &adap::EntryRow| Some(e.text().trim().to_string()).filter(|s| !s.is_empty());
        let new = PortRule {
            ports: ports_s,
            proto: PROTOS[proto_.selected() as usize % 3],
            direction: DIRS[dir.selected() as usize % 3],
            action: ACTIONS[action.selected() as usize % 2],
            zone: if zone.selected() == 1 { Zone::Lan } else { Zone::Any },
            interface: (iface.selected() > 0).then(|| ifs[iface.selected() as usize].clone()),
            remote: opt(&remote),
            note: note.text().trim().to_string(),
            ..r.clone()
        };
        send(&app2, &holder2, "Règle de port", Request::SavePortRule(new));
    });
    d.present(Some(&app.window));
}

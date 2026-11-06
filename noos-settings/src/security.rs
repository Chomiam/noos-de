//! Confidentialité et sécurité : pare-feu (firewalld / ufw / nftables / iptables).

use std::rc::Rc;

use adap::prelude::*;
use gtk::glib;
use noos_core::paths;
use noos_pkg::firewall::{self, FwBackend, FwStatus, PortRule, Proto};

use crate::ui::{App, background, card, group, link, page};

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Confidentialité et sécurité");
    let g = group("Sécurité");
    g.add(&link(app, "security-high-symbolic", "Pare-feu et protection du réseau", "État du pare-feu, ports ouverts", firewall_page));
    g.add(&link(app, "software-update-available-symbolic", "Mises à jour", "Les mises à jour de sécurité sont gérées par Logiciels", crate::other::updates));
    content.append(&g);
    p
}

fn shield(status: &FwStatus) -> (&'static str, &'static str, String, &'static str) {
    match (status.backend, status.active) {
        (FwBackend::None, _) => ("shield-bad.svg", "fw-state-bad", "Aucun pare-feu n'est installé".into(), "Votre appareil n'est pas protégé contre les connexions entrantes."),
        (_, false) => ("shield-bad.svg", "fw-state-bad", "Le pare-feu est désactivé".into(), "Votre appareil peut être vulnérable. Activez le pare-feu."),
        (b, true) if !b.manageable() => ("shield-warn.svg", "fw-state-warn", "Le pare-feu est actif".into(), "Ses règles sont gérées manuellement : NOOS les affiche sans les modifier."),
        _ => ("shield-ok.svg", "fw-state-ok", "Le pare-feu est activé".into(), "Les connexions entrantes non autorisées sont bloquées."),
    }
}

pub fn firewall_page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Confidentialité et sécurité"], "Pare-feu et protection du réseau");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    load(app, &holder);
    p
}

fn load(app: &Rc<App>, holder: &gtk::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let sp = gtk::Spinner::new();
    sp.start();
    sp.set_size_request(32, 32);
    holder.append(&sp);
    let task = background(firewall::status);
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let st = task.await;
        holder.remove(&sp);
        render(&app, &holder, st);
    });
}

fn render(app: &Rc<App>, holder: &gtk::Box, st: FwStatus) {
    let (file, state_class, title, subtitle) = shield(&st);

    // ── Héros : grand bouclier ──
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 28);
    hero.add_css_class("win11-hero");
    hero.add_css_class("fw-hero");
    let img = gtk::Image::from_file(paths::data_dir().join("icons").join(file));
    img.set_pixel_size(128);
    hero.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    let t = gtk::Label::new(Some(&title));
    t.add_css_class("title-1");
    t.add_css_class(state_class);
    t.set_xalign(0.0);
    t.set_wrap(true);
    v.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.set_xalign(0.0);
    s.set_wrap(true);
    v.append(&s);
    // Backend affiché discrètement.
    let mut backend_txt = format!("Backend : {}", st.backend.label());
    if let Some(z) = &st.zone {
        backend_txt.push_str(&format!(" · zone {z}"));
    }
    if st.backend != FwBackend::None {
        backend_txt.push_str(if st.enabled_at_boot { " · démarre avec le système" } else { " · ne démarre pas automatiquement" });
    }
    let b = gtk::Label::new(Some(&backend_txt));
    b.add_css_class("fw-backend");
    b.set_xalign(0.0);
    b.set_selectable(true);
    v.append(&b);
    if !st.detail.is_empty() && st.backend != FwBackend::None {
        let d = gtk::Label::new(Some(&st.detail));
        d.add_css_class("dim-label");
        d.add_css_class("caption");
        d.set_xalign(0.0);
        v.append(&d);
    }
    hero.append(&v);

    let action = if st.backend == FwBackend::None {
        let btn = gtk::Button::with_label("Installer un pare-feu");
        btn.add_css_class("suggested-action");
        let (app2, holder2) = (app.clone(), holder.clone());
        btn.connect_clicked(move |_| {
            let sys = noos_pkg::SystemInfo::detect();
            match firewall::install_plan(&sys) {
                Some((name, plan)) => {
                    let (app3, holder3) = (app2.clone(), holder2.clone());
                    app2.confirm(
                        &format!("Installer {name} ?"),
                        &format!("{name} sera installé puis activé : les connexions entrantes seront bloquées, sauf celles que vous autorisez."),
                        "Installer",
                        false,
                        move || {
                            let (app4, holder4) = (app3.clone(), holder3.clone());
                            app3.run_plan(&format!("Installation de {name}"), Ok(plan.clone()), move |_| load(&app4, &holder4));
                        },
                    );
                }
                None => app2.toast("Distribution non reconnue : installez firewalld ou ufw"),
            }
        });
        Some(btn)
    } else if st.backend.manageable() {
        let btn = gtk::Button::with_label(if st.active { "Désactiver" } else { "Activer" });
        btn.add_css_class(if st.active { "destructive-action" } else { "suggested-action" });
        let (app2, holder2, backend, active) = (app.clone(), holder.clone(), st.backend, st.active);
        btn.connect_clicked(move |_| {
            let (app3, holder3) = (app2.clone(), holder2.clone());
            let go = move || {
                let (app4, holder4) = (app3.clone(), holder3.clone());
                app3.run_plan(
                    if active { "Désactivation du pare-feu" } else { "Activation du pare-feu" },
                    firewall::set_active(backend, !active),
                    move |_| load(&app4, &holder4),
                );
            };
            if active {
                app2.confirm("Désactiver le pare-feu ?", "Votre appareil acceptera toutes les connexions entrantes.", "Désactiver", true, go);
            } else {
                go();
            }
        });
        Some(btn)
    } else {
        None
    };
    if let Some(btn) = action {
        btn.add_css_class("pill");
        btn.set_valign(gtk::Align::Center);
        hero.append(&btn);
    }
    holder.append(&hero);

    if st.backend == FwBackend::None {
        return;
    }

    // ── Ports ouverts ──
    let ports = group("Ports ouverts");
    ports.set_description(Some(if st.backend == FwBackend::Firewalld {
        "Les ouvertures s'appliquent immédiatement et sont conservées au redémarrage"
    } else {
        "Connexions entrantes autorisées"
    }));
    if st.backend.manageable() {
        let add = gtk::Button::with_label("Ouvrir un port…");
        add.add_css_class("flat");
        let (app2, holder2, st2) = (app.clone(), holder.clone(), st.clone());
        add.connect_clicked(move |_| open_dialog(&app2, &holder2, &st2));
        add.set_sensitive(st.active);
        ports.set_header_suffix(Some(&add));
    }
    if !st.rules_known {
        let r = card("dialog-password-symbolic", "Règles non lues", "Leur lecture demande les droits administrateur");
        let b = gtk::Button::with_label("Afficher les règles");
        b.set_valign(gtk::Align::Center);
        let (app2, holder2, st2) = (app.clone(), holder.clone(), st.clone());
        b.connect_clicked(move |_| read_privileged(&app2, &holder2, st2.clone()));
        r.add_suffix(&b);
        ports.add(&r);
    } else if st.ports.is_empty() {
        ports.add(&card("network-wired-symbolic", "Aucun port ouvert", "Toutes les connexions entrantes sont bloquées, sauf les services ci-dessous"));
    }
    for rule in &st.ports {
        ports.add(&port_row(app, holder, &st, rule));
    }
    holder.append(&ports);

    // ── Services ──
    if !st.services.is_empty() {
        let sv = group("Services autorisés");
        for name in &st.services {
            let r = card("network-server-symbolic", name, service_hint(name));
            if st.backend.manageable() {
                let rm = gtk::Button::from_icon_name("user-trash-symbolic");
                rm.add_css_class("flat");
                rm.set_valign(gtk::Align::Center);
                rm.set_tooltip_text(Some("Ne plus autoriser"));
                let (app2, holder2, backend, zone, name2) = (app.clone(), holder.clone(), st.backend, st.zone.clone(), name.clone());
                rm.connect_clicked(move |_| {
                    let (app3, holder3, zone3, name3) = (app2.clone(), holder2.clone(), zone.clone(), name2.clone());
                    app2.confirm(
                        &format!("Bloquer « {name2} » ?"),
                        "Les connexions entrantes vers ce service seront refusées.",
                        "Bloquer",
                        true,
                        move || {
                            let (app4, holder4) = (app3.clone(), holder3.clone());
                            app3.run_plan("Retrait d'un service", firewall::remove_service(backend, &name3, zone3.as_deref()), move |_| load(&app4, &holder4));
                        },
                    );
                });
                r.add_suffix(&rm);
            }
            sv.add(&r);
        }
        holder.append(&sv);
    }
}

fn service_hint(name: &str) -> &'static str {
    match name {
        "ssh" | "OpenSSH" => "Connexion à distance (port 22)",
        "dhcpv6-client" => "Configuration réseau IPv6",
        "mdns" => "Découverte des appareils sur le réseau local",
        "samba-client" | "samba" => "Partage de fichiers Windows",
        "kdeconnect" => "KDE Connect / GSConnect",
        "http" => "Serveur web (80)",
        "https" => "Serveur web sécurisé (443)",
        "ipp-client" | "ipp" => "Impression réseau",
        _ => "",
    }
}

fn port_row(app: &Rc<App>, holder: &gtk::Box, st: &FwStatus, rule: &PortRule) -> adap::ActionRow {
    let hint = match rule.port.as_str() {
        "22" => "SSH",
        "80" | "8080" => "Web",
        "443" => "Web sécurisé",
        "25565" => "Minecraft",
        "27015" | "27036" => "Steam",
        "1025-65535" => "Plage large ouverte par la distribution",
        _ => "",
    };
    let r = card("network-wired-symbolic", &rule.label(), hint);
    if st.backend.manageable() {
        let rm = gtk::Button::from_icon_name("user-trash-symbolic");
        rm.add_css_class("flat");
        rm.set_valign(gtk::Align::Center);
        rm.set_tooltip_text(Some("Fermer ce port"));
        let (app2, holder2, backend, zone, rule2) = (app.clone(), holder.clone(), st.backend, st.zone.clone(), rule.clone());
        rm.connect_clicked(move |_| {
            let (app3, holder3, zone3, rule3) = (app2.clone(), holder2.clone(), zone.clone(), rule2.clone());
            app2.confirm(&format!("Fermer le port {} ?", rule2.label()), "Les connexions entrantes sur ce port seront bloquées.", "Fermer", true, move || {
                let (app4, holder4) = (app3.clone(), holder3.clone());
                app3.run_plan("Fermeture du port", firewall::close_port(backend, &rule3, zone3.as_deref()), move |_| load(&app4, &holder4));
            });
        });
        r.add_suffix(&rm);
    }
    r
}

fn open_dialog(app: &Rc<App>, holder: &gtk::Box, st: &FwStatus) {
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Port (ex. 25565) ou plage (6000-6010)"));
    entry.set_activates_default(true);
    entry.set_input_purpose(gtk::InputPurpose::Digits);
    bx.append(&entry);
    let proto = gtk::DropDown::from_strings(&["TCP", "UDP", "TCP et UDP"]);
    bx.append(&proto);
    let presets = gtk::FlowBox::new();
    presets.set_selection_mode(gtk::SelectionMode::None);
    presets.set_max_children_per_line(4);
    for (label, port, p) in [("Minecraft", "25565", 2u32), ("Serveur web", "8080", 0), ("Steam (jeu local)", "27015-27030", 2), ("KDE Connect", "1714-1764", 2)] {
        let b = gtk::Button::with_label(label);
        b.add_css_class("pill");
        let (entry, proto) = (entry.clone(), proto.clone());
        b.connect_clicked(move |_| {
            entry.set_text(port);
            proto.set_selected(p);
        });
        presets.insert(&b, -1);
    }
    bx.append(&presets);

    let d = adap::AlertDialog::new(
        Some("Ouvrir un port"),
        Some(&format!(
            "Autorise les connexions entrantes vers ce port{}.",
            st.zone.as_ref().map(|z| format!(" (zone {z})")).unwrap_or_default()
        )),
    );
    d.set_extra_child(Some(&bx));
    d.add_responses(&[("cancel", "Annuler"), ("open", "Ouvrir")]);
    d.set_response_appearance("open", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("open"));
    let (app2, holder2, backend, zone) = (app.clone(), holder.clone(), st.backend, st.zone.clone());
    d.connect_response(None, move |_, r| {
        if r != "open" {
            return;
        }
        let p = match proto.selected() {
            1 => Proto::Udp,
            2 => Proto::Both,
            _ => Proto::Tcp,
        };
        let port = entry.text().to_string();
        let (app3, holder3) = (app2.clone(), holder2.clone());
        app2.run_plan(&format!("Ouverture du port {port}"), firewall::open_port(backend, &port, p, zone.as_deref()), move |_| load(&app3, &holder3));
    });
    d.present(Some(&app.window));
}

fn read_privileged(app: &Rc<App>, holder: &gtk::Box, st: FwStatus) {
    let backend = st.backend;
    let task = background(move || firewall::read_rules_privileged(backend).map_err(|e| format!("{e:#}")));
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        match task.await {
            Ok((ports, services, raw)) => {
                if backend.manageable() {
                    let mut st2 = st.clone();
                    st2.ports = ports;
                    st2.services = services;
                    st2.rules_known = true;
                    while let Some(c) = holder.first_child() {
                        holder.remove(&c);
                    }
                    render(&app, &holder, st2);
                } else {
                    // nftables / iptables : affichage brut en lecture seule.
                    let view = gtk::TextView::new();
                    view.buffer().set_text(&raw);
                    view.set_editable(false);
                    view.set_monospace(true);
                    view.set_top_margin(12);
                    view.set_left_margin(12);
                    let tv = adap::ToolbarView::new();
                    tv.add_top_bar(&adap::HeaderBar::new());
                    tv.set_content(Some(&gtk::ScrolledWindow::builder().child(&view).vexpand(true).build()));
                    let d = adap::Dialog::builder().title(format!("Règles {}", backend.label())).content_width(760).content_height(520).child(&tv).build();
                    d.present(Some(&app.window));
                }
            }
            Err(e) => app.error("Lecture des règles", &e),
        }
    });
}

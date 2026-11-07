//! Réseau › DNS : fournisseurs proposés (pays, juridiction, journalisation, filtrage,
//! confidentialité), mesure du temps de réponse, DNS over TLS, serveurs personnalisés.

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::glib;

use super::{badge, clear, info, label, nm, spinner};
use crate::ui::{self, App, background, group};
use nm::{Connection, DnsChoice, Provider, Tls};

struct State {
    targets: Vec<Connection>,
    primary: Option<Connection>,
    dns: Option<nm::DnsState>,
}

fn read() -> State {
    let all = nm::connections();
    let primary = nm::primary();
    let dns = primary.as_ref().map(nm::dns_state);
    State { targets: all.into_iter().filter(Connection::is_lan).collect(), primary, dns }
}

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = ui::page(app, &["Réseau et Internet"], "DNS");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    load(app, &holder);
    p
}

fn load(app: &Rc<App>, holder: &gtk::Box) {
    clear(holder);
    holder.append(&spinner());
    let task = background(read);
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let st = task.await;
        clear(&holder);
        render(&app, &holder, Rc::new(st));
    });
}

fn privacy_badge(level: u8) -> gtk::Label {
    match level {
        3 => badge("Confidentialité excellente", "net-ok"),
        2 => badge("Confidentialité bonne", "net-mid"),
        _ => badge("Confidentialité moyenne", "net-bad"),
    }
}

fn render(app: &Rc<App>, holder: &gtk::Box, st: Rc<State>) {
    let Some(primary) = st.primary.clone() else {
        holder.append(
            &adap::StatusPage::builder()
                .icon_name("network-offline-symbolic")
                .title("Aucune connexion active")
                .description("Connectez-vous à un réseau pour choisir son DNS.")
                .build(),
        );
        return;
    };
    let dns = st.dns.clone().unwrap_or(nm::DnsState { choice: DnsChoice::Auto, tls: Tls::Off, in_use: vec![] });

    // ── Héros : DNS actuel ──
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    h.add_css_class("win11-hero");
    let current = match &dns.choice {
        DnsChoice::Provider(id) => nm::provider(id),
        _ => None,
    };
    let flag = label(current.map(|p| p.flag).unwrap_or("🌐"), &["net-flag-big"]);
    flag.set_valign(gtk::Align::Center);
    h.append(&flag);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.set_hexpand(true);
    v.set_valign(gtk::Align::Center);
    v.append(&label("Serveur DNS actuel", &["dim-label", "caption"]));
    v.append(&label(
        &match &dns.choice {
            DnsChoice::Auto => "Automatique (box / opérateur)".to_string(),
            DnsChoice::Provider(_) => current.map(|p| p.name.to_string()).unwrap_or_default(),
            DnsChoice::Custom(l) => format!("Personnalisé : {}", l.join(", ")),
        },
        &["title-2"],
    ));
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let encrypted = !matches!(dns.choice, DnsChoice::Auto) && dns.tls != Tls::Off;
    line.append(&if encrypted {
        badge(if dns.tls == Tls::Strict { "🔒 Requêtes chiffrées (DNS over TLS)" } else { "🔒 Chiffrées si possible" }, "net-ok")
    } else {
        badge("Requêtes en clair : visibles par votre opérateur", "net-bad")
    });
    if let Some(p) = current {
        line.append(&privacy_badge(p.privacy));
    }
    line.set_margin_top(4);
    v.append(&line);
    if !dns.in_use.is_empty() {
        let s = label(&format!("Serveurs utilisés : {}", dns.in_use.join(" · ")), &["dim-label", "caption"]);
        s.set_wrap(true);
        s.set_margin_top(4);
        v.append(&s);
    }
    h.append(&v);
    holder.append(&h);

    // ── Réglages : chiffrement et portée ──
    let g = group("Réglages");
    let tls = crate::ui::combo(
        "channel-secure-symbolic",
        "Chiffrement des requêtes (DNS over TLS)",
        "Empêche votre opérateur et le réseau local (Wi-Fi public…) de voir les sites que vous visitez",
        &["Obligatoire (recommandé)".into(), "Si possible".into(), "Désactivé".into()],
        match dns.tls {
            Tls::Strict => 0,
            Tls::Opportunistic => 1,
            Tls::Off => if matches!(dns.choice, DnsChoice::Auto) { 0 } else { 2 },
        },
    );
    g.add(&tls);
    let all = crate::ui::switch(
        "network-workgroup-symbolic",
        "Pour tous les réseaux",
        &if st.targets.len() > 1 {
            format!("Appliquer aussi aux {} réseaux enregistrés (Wi-Fi, filaires), pas seulement à « {} »", st.targets.len(), primary.name)
        } else {
            "Appliquer aussi aux réseaux Wi-Fi et filaires enregistrés plus tard".to_string()
        },
        true,
    );
    g.add(&all);
    holder.append(&g);

    let tls_mode = {
        let tls = tls.clone();
        move || match tls.selected() {
            0 => Tls::Strict,
            1 => Tls::Opportunistic,
            _ => Tls::Off,
        }
    };
    // Applique un choix à la connexion principale (ou à toutes), puis recharge.
    let apply: Rc<dyn Fn(DnsChoice)> = {
        let (app, holder, st, all, tls_mode) = (app.clone(), holder.clone(), st.clone(), all.clone(), tls_mode.clone());
        Rc::new(move |choice: DnsChoice| {
            let mut mode = tls_mode();
            if let DnsChoice::Provider(id) = &choice {
                let p = nm::provider(id).expect("fournisseur");
                if p.tls.is_empty() {
                    mode = Tls::Off;
                } else if p.tls_only && mode == Tls::Off {
                    mode = Tls::Strict;
                }
            }
            let targets: Vec<Connection> = if all.is_active() { st.targets.clone() } else { st.primary.iter().cloned().collect() };
            let task = background(move || -> Result<(), String> {
                let mut errors = Vec::new();
                for c in &targets {
                    if let Err(e) = nm::set_dns(c, &choice, mode) {
                        errors.push(format!("{} : {e:#}", c.name));
                    }
                }
                if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
            });
            let (app, holder) = (app.clone(), holder.clone());
            holder.set_sensitive(false);
            glib::spawn_future_local(async move {
                let res = task.await;
                holder.set_sensitive(true);
                match res {
                    Ok(()) => app.toast("DNS appliqué"),
                    Err(e) => app.error("DNS", &e),
                }
                load(&app, &holder);
            });
        })
    };
    {
        let (apply, choice) = (apply.clone(), dns.choice.clone());
        tls.connect_selected_notify(move |_| {
            if !matches!(choice, DnsChoice::Auto) {
                apply(choice.clone());
            }
        });
    }

    // ── Fournisseurs ──
    let g = group("Fournisseurs");
    g.set_description(Some(
        "L'appréciation de confidentialité est celle de NOOS, d'après les engagements publiés par chaque service (journalisation, juridiction, audits).",
    ));
    let measure = gtk::Button::with_label("Mesurer la vitesse");
    measure.set_valign(gtk::Align::Center);
    g.set_header_suffix(Some(&measure));
    let mut speed_labels: Vec<(&'static Provider, gtk::Label)> = Vec::new();

    // Automatique.
    let auto = adap::ActionRow::new();
    auto.set_title("Automatique");
    auto.set_subtitle("Les serveurs de votre box ou de votre opérateur (souvent ni chiffrés, ni sans journaux)");
    auto.add_prefix(&label("🏠", &["net-flag"]));
    if matches!(dns.choice, DnsChoice::Auto) {
        auto.add_suffix(&current_mark());
    } else {
        let b = gtk::Button::with_label("Utiliser");
        b.set_valign(gtk::Align::Center);
        let apply = apply.clone();
        b.connect_clicked(move |_| apply(DnsChoice::Auto));
        auto.add_suffix(&b);
    }
    g.add(&auto);

    for p in nm::PROVIDERS {
        let r = adap::ExpanderRow::new();
        r.set_title(p.name);
        r.set_subtitle(&format!("{} · {}", p.country, p.company));
        r.add_prefix(&label(p.flag, &["net-flag"]));
        let speed = label("", &["net-speed"]);
        speed.set_valign(gtk::Align::Center);
        r.add_suffix(&speed);
        speed_labels.push((p, speed));
        if let Some(b) = p.blocks {
            r.add_suffix(&badge(&format!("Bloque : {b}"), "net-filter"));
        }
        if p.no_logs {
            r.add_suffix(&badge("Sans journaux", "net-ok"));
        }
        let is_current = matches!(&dns.choice, DnsChoice::Provider(id) if *id == p.id);
        // Tests (captures) : `NOOS_NET_EXPAND=quad9` déplie ce fournisseur.
        if std::env::var("NOOS_NET_EXPAND").is_ok_and(|v| v.split(',').any(|x| x == p.id)) {
            r.set_expanded(true);
        }
        if is_current {
            r.add_suffix(&current_mark());
        }

        r.add_row(&info("Pays et société", &format!("{} {} — {}", p.flag, p.country, p.company)));
        r.add_row(&info("Juridiction", p.jurisdiction));
        r.add_row(&info("Journalisation", p.logging));
        r.add_row(&blocking_row(p.id));
        let b = nm::blocking(p.id);
        if !b.extra.is_empty() {
            r.add_row(&info("Précisions", b.extra));
        }
        if !b.lists.is_empty() && b.lists != "Aucune" {
            r.add_row(&info("Listes utilisées", b.lists));
        }
        if !b.variants.is_empty() {
            r.add_row(&info(
                "Autres versions du même service",
                &b.variants.iter().map(|(a, d)| format!("{a} — {d}")).collect::<Vec<_>>().join("\n"),
            ));
        }
        if !b.report.is_empty() {
            r.add_row(&info("Site bloqué à tort ?", b.report));
        }
        let mut addrs = p.v4.join(", ");
        if !p.v6.is_empty() {
            addrs.push_str(&format!("\n{}", p.v6.join(", ")));
        }
        r.add_row(&info("Adresses", &addrs));
        r.add_row(&info(
            "Chiffrement",
            &if p.tls.is_empty() {
                "DNS over TLS non proposé : requêtes en clair".to_string()
            } else if p.tls_only {
                format!("Uniquement chiffré (DNS over TLS : {}) — le chiffrement sera activé", p.tls)
            } else {
                format!("DNS over TLS : {}", p.tls)
            },
        ));
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        actions.set_margin_top(10);
        actions.set_margin_bottom(10);
        actions.set_margin_start(12);
        actions.set_margin_end(12);
        actions.append(&privacy_badge(p.privacy));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        let use_btn = gtk::Button::with_label(if is_current { "Utilisé actuellement" } else { "Utiliser ce DNS" });
        use_btn.add_css_class("suggested-action");
        use_btn.set_sensitive(!is_current);
        {
            let apply = apply.clone();
            let id = p.id;
            use_btn.connect_clicked(move |_| apply(DnsChoice::Provider(id)));
        }
        actions.append(&use_btn);
        let holder_row = gtk::ListBoxRow::new();
        holder_row.set_activatable(false);
        holder_row.set_child(Some(&actions));
        r.add_row(&holder_row);
        g.add(&r);
    }

    // Personnalisé.
    let custom = adap::ExpanderRow::new();
    custom.set_title("Personnalisé");
    custom.set_subtitle("Vos propres serveurs (Pi-hole, AdGuard Home, serveur d'entreprise…)");
    custom.add_prefix(&label("⚙️", &["net-flag"]));
    if let DnsChoice::Custom(l) = &dns.choice {
        custom.add_suffix(&current_mark());
        custom.set_subtitle(&l.join(", "));
    }
    let entry = adap::EntryRow::new();
    entry.set_title("Adresses séparées par des virgules (ajoutez #nom pour le chiffrement)");
    if let DnsChoice::Custom(l) = &dns.choice {
        entry.set_text(&l.join(", "));
    }
    entry.set_show_apply_button(true);
    {
        let (apply, app) = (apply.clone(), app.clone());
        entry.connect_apply(move |e| {
            let list: Vec<String> = e.text().split([',', ' ']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if list.is_empty() || list.iter().any(|a| a.split('#').next().unwrap_or("").parse::<std::net::IpAddr>().is_err()) {
                app.toast("Adresse IP invalide");
                return;
            }
            apply(DnsChoice::Custom(list));
        });
    }
    custom.add_row(&entry);
    g.add(&custom);
    holder.append(&g);

    // Mesure : chaque fournisseur en parallèle, le plus rapide mis en avant.
    let labels = Rc::new(speed_labels);
    measure.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label("Mesure…");
        let results: Rc<RefCell<Vec<(usize, Option<u128>)>>> = Rc::new(RefCell::new(Vec::new()));
        for (i, (p, l)) in labels.iter().enumerate() {
            l.set_text("…");
            l.remove_css_class("net-fastest");
            let (ip, tls_only, has_tls) = (p.v4[0].to_string(), p.tls_only, !p.tls.is_empty());
            let task = background(move || nm::latency(&ip, tls_only, has_tls).map(|d| d.as_millis()));
            let (l, results, labels, b) = (l.clone(), results.clone(), labels.clone(), b.clone());
            glib::spawn_future_local(async move {
                let ms = task.await;
                l.set_text(&ms.map(|m| format!("{m} ms")).unwrap_or_else(|| "injoignable".into()));
                results.borrow_mut().push((i, ms));
                if results.borrow().len() == labels.len() {
                    if let Some((best, _)) = results.borrow().iter().filter_map(|(i, m)| m.map(|m| (*i, m))).min_by_key(|(_, m)| *m) {
                        labels[best].1.add_css_class("net-fastest");
                        labels[best].1.set_text(&format!("⚡ {}", labels[best].1.text()));
                    }
                    b.set_sensitive(true);
                    b.set_label("Mesurer à nouveau");
                }
            });
        }
    });
    if std::env::var_os("NOOS_NET_MEASURE").is_some() {
        glib::idle_add_local_once(move || measure.emit_clicked());
    }
}

fn current_mark() -> gtk::Image {
    let i = gtk::Image::from_icon_name("object-select-symbolic");
    i.add_css_class("net-ok");
    i.set_tooltip_text(Some("Utilisé actuellement"));
    i
}

/// Tableau « Ce qui est bloqué » : une pastille par catégorie (bloqué / non bloqué).
fn blocking_row(id: &str) -> gtk::ListBoxRow {
    let b = nm::blocking(id);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
    v.set_margin_top(10);
    v.set_margin_bottom(10);
    v.set_margin_start(12);
    v.set_margin_end(12);
    let t = label("Ce qui est bloqué", &["dim-label", "caption"]);
    v.append(&t);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(3);
    flow.set_column_spacing(6);
    flow.set_row_spacing(6);
    for ((name, desc), on) in nm::BLOCK_CATEGORIES.iter().zip(b.categories) {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        h.add_css_class("net-block");
        h.add_css_class(if on { "on" } else { "off" });
        let i = gtk::Image::from_icon_name(if on { "object-select-symbolic" } else { "window-close-symbolic" });
        h.append(&i);
        let l = label(name, &[]);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        h.append(&l);
        h.set_tooltip_text(Some(&format!("{} — {desc}", if on { "Bloqué" } else { "Non bloqué" })));
        flow.append(&h);
    }
    v.append(&flow);
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&v));
    row
}

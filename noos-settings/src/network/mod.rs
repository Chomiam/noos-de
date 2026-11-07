//! Réseau et Internet : état de la connexion avec débit en direct, Wi-Fi, propriétés et
//! adresse IP, DNS (fournisseurs, confidentialité, DNS over TLS), VPN WireGuard, point
//! d'accès mobile et diagnostic. Tout passe par NetworkManager (`nm.rs`).

pub mod dns;
pub mod nm;
pub mod vpn;
pub mod wifi;

use std::{cell::RefCell, collections::VecDeque, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::glib;

use crate::ui::{App, background, card, group, link, page};
use nm::{Connection, DnsChoice, Tls};

/// Tout ce qu'affiche la page d'accueil, lu en arrière-plan.
#[derive(Clone)]
struct Overview {
    primary: Option<Connection>,
    devices: Vec<nm::Device>,
    info: std::collections::HashMap<String, String>,
    dns: Option<nm::DnsState>,
    vpns: Vec<Connection>,
    speed: Option<u32>,
}

fn overview() -> Overview {
    let primary = nm::primary();
    let info = primary.as_ref().map(|c| nm::device_info(&c.device)).unwrap_or_default();
    let dns = primary.as_ref().map(nm::dns_state);
    let speed = primary.as_ref().and_then(|c| nm::link_speed(&c.device));
    Overview {
        devices: nm::devices(),
        vpns: nm::connections().into_iter().filter(Connection::is_wireguard).collect(),
        primary,
        info,
        dns,
        speed,
    }
}

pub fn dns_label(st: &nm::DnsState) -> String {
    let name = match &st.choice {
        DnsChoice::Auto => "Automatique (box / opérateur)".to_string(),
        DnsChoice::Provider(id) => nm::provider(id).map(|p| format!("{} {}", p.flag, p.name)).unwrap_or_default(),
        DnsChoice::Custom(l) => format!("Personnalisé ({})", l.join(", ")),
    };
    let tls = match (st.tls, &st.choice) {
        (_, DnsChoice::Auto) => "",
        (Tls::Strict, _) => " · chiffré",
        (Tls::Opportunistic, _) => " · chiffré si possible",
        (Tls::Off, _) => " · non chiffré",
    };
    format!("{name}{tls}")
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

pub fn spinner() -> gtk::Spinner {
    let sp = gtk::Spinner::new();
    sp.set_size_request(32, 32);
    sp.set_halign(gtk::Align::Center);
    sp.set_margin_top(24);
    sp.start();
    sp
}

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

pub fn badge(text: &str, class: &str) -> gtk::Label {
    let l = label(text, &["net-badge", class]);
    l.set_valign(gtk::Align::Center);
    l
}

/// Rangée d'information (titre discret, valeur sélectionnable).
pub fn info(title: &str, value: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.set_subtitle(if value.is_empty() { "—" } else { value });
    r.add_css_class("property");
    r.set_subtitle_selectable(true);
    r
}

// ───────────────────────────── Accueil ─────────────────────────────

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Réseau et Internet");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    holder.append(&spinner());
    content.append(&holder);
    let load = {
        let (app, holder) = (app.clone(), holder.clone());
        move || {
            let task = background(overview);
            let (app, holder) = (app.clone(), holder.clone());
            glib::spawn_future_local(async move {
                let ov = task.await;
                clear(&holder);
                render_root(&app, &holder, &ov);
            });
        }
    };
    load();
    // Retour sur la page (après DNS, VPN, Wi-Fi…) : l'état est relu.
    let first = std::cell::Cell::new(true);
    p.connect_showing(move |_| {
        if !first.replace(false) {
            load();
        }
    });
    p
}

fn render_root(app: &Rc<App>, holder: &gtk::Box, ov: &Overview) {
    holder.append(&hero(ov));

    let g = group("");
    let wifi_dev = ov.devices.iter().find(|d| d.kind == "wifi");
    if let Some(d) = wifi_dev {
        let sub = if d.state == "connected" { format!("Connecté à {}", d.connection) } else { "Réseaux sans fil disponibles, réseaux enregistrés".into() };
        g.add(&link(app, "network-wireless-symbolic", "Wi-Fi", &sub, wifi::page));
    }
    if let Some(c) = &ov.primary {
        let ip = nm::indexed(&ov.info, "IP4.ADDRESS").first().cloned().unwrap_or_default();
        g.add(&link(
            app,
            if c.is_wifi() { "network-wireless-symbolic" } else { "network-wired-symbolic" },
            "Propriétés de la connexion",
            &format!("{} · {}", c.name, if ip.is_empty() { "pas d'adresse IP".into() } else { ip }),
            properties,
        ));
    }
    let dns_sub = ov.dns.as_ref().map(dns_label).unwrap_or_else(|| "Serveurs DNS, chiffrement, confidentialité".into());
    g.add(&link(app, "network-server-symbolic", "DNS", &dns_sub, dns::page));
    let active_vpn = ov.vpns.iter().find(|v| v.active);
    let vpn_sub = match (active_vpn, ov.vpns.len()) {
        (Some(v), _) => format!("Connecté : {}", v.name),
        (None, 0) => "Importer une configuration WireGuard (.conf)".into(),
        (None, n) => format!("{n} profil{} WireGuard, aucun actif", if n > 1 { "s" } else { "" }),
    };
    g.add(&link(app, "network-vpn-symbolic", "VPN", &vpn_sub, vpn::page));
    if wifi_dev.is_some() {
        g.add(&link(app, "network-wireless-hotspot-symbolic", "Point d'accès mobile", "Partager la connexion de ce PC en Wi-Fi", hotspot));
    }
    g.add(&link(app, "system-search-symbolic", "Diagnostic", "Trouver pourquoi Internet ne fonctionne pas", diagnostics));
    holder.append(&g);

    let g = group("Cartes réseau");
    for d in &ov.devices {
        let (icon, kind) = match d.kind.as_str() {
            "wifi" => ("network-wireless-symbolic", "Wi-Fi"),
            "ethernet" => ("network-wired-symbolic", "Ethernet"),
            "wireguard" => ("network-vpn-symbolic", "WireGuard"),
            "bridge" => ("network-workgroup-symbolic", "Pont"),
            "bt" => ("bluetooth-symbolic", "Bluetooth"),
            _ => ("network-wired-symbolic", d.kind.as_str()),
        };
        let state = match d.state.as_str() {
            "connected" => format!("Connecté · {}", d.connection),
            "disconnected" => "Déconnecté".into(),
            "unavailable" => "Indisponible (câble débranché ou désactivé)".into(),
            "unmanaged" => "Non géré par NOOS".into(),
            s if s.starts_with("connecting") => "Connexion…".into(),
            s => s.to_string(),
        };
        let r = card(icon, &format!("{kind} — {}", d.name), &state);
        if d.state == "connected" {
            r.add_suffix(&badge("Actif", "net-ok"));
        }
        g.add(&r);
    }
    holder.append(&g);
}

/// Carte d'état : connexion, adresses, DNS, et débit en direct (graphique).
fn hero(ov: &Overview) -> gtk::Box {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    h.add_css_class("win11-hero");
    h.add_css_class("net-hero");
    let (icon, class, title) = match &ov.primary {
        Some(c) if c.is_wifi() => ("network-wireless-signal-excellent-symbolic", "net-ok", "Connecté en Wi-Fi"),
        Some(_) => ("network-wired-symbolic", "net-ok", "Connecté"),
        None => ("network-offline-symbolic", "net-bad", "Hors ligne"),
    };
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(40);
    img.add_css_class("net-hero-icon");
    img.add_css_class(class);
    img.set_valign(gtk::Align::Center);
    h.append(&img);

    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    v.append(&label(title, &["title-2"]));
    match &ov.primary {
        Some(c) => {
            let speed = ov.speed.map(|s| if s >= 1000 { format!(" · {} Gbit/s", s / 1000) } else { format!(" · {s} Mbit/s") }).unwrap_or_default();
            v.append(&label(&format!("{} · {}{speed}", c.name, c.device), &["dim-label"]));
            let ip4 = nm::indexed(&ov.info, "IP4.ADDRESS").first().map(|a| a.split('/').next().unwrap_or(a).to_string());
            let ip6 = !nm::indexed(&ov.info, "IP6.ADDRESS").is_empty();
            let line = format!("IPv4 {}{}", ip4.unwrap_or_else(|| "—".into()), if ip6 { " · IPv6 ✓" } else { "" });
            v.append(&label(&line, &["dim-label", "caption"]));
            if let Some(d) = &ov.dns {
                v.append(&label(&format!("DNS : {}", dns_label(d)), &["dim-label", "caption"]));
            }
            if let Some(vpn) = ov.vpns.iter().find(|v| v.active) {
                let b = badge(&format!("VPN : {}", vpn.name), "net-vpn");
                b.set_halign(gtk::Align::Start);
                b.set_margin_top(4);
                v.append(&b);
            }
        }
        None => v.append(&label("Aucune connexion active. Branchez un câble ou choisissez un réseau Wi-Fi.", &["dim-label"])),
    }
    h.append(&v);
    if let Some(c) = &ov.primary {
        h.append(&traffic(&c.device));
    }
    h
}

/// Graphique du débit (60 dernières secondes), mis à jour chaque seconde.
fn traffic(dev: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.set_valign(gtk::Align::Center);
    let rates = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let down = label("↓ —", &["net-down", "caption-heading"]);
    let up = label("↑ —", &["net-up", "caption-heading"]);
    rates.append(&down);
    rates.append(&up);
    let area = gtk::DrawingArea::new();
    area.set_content_width(240);
    area.set_content_height(70);
    let hist: Rc<RefCell<VecDeque<(f64, f64)>>> = Rc::new(RefCell::new(VecDeque::from(vec![(0.0, 0.0); 60])));
    {
        let hist = hist.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let data = hist.borrow();
            let max = data.iter().map(|(a, b)| a.max(*b)).fold(1024.0_f64, f64::max) * 1.15;
            let fg = area.color();
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.08);
            for i in 1..4 {
                let y = h * i as f64 / 4.0;
                cr.move_to(0.0, y);
                cr.line_to(w, y);
            }
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            let n = data.len().max(2) as f64 - 1.0;
            for (idx, (r, g, bl)) in [(0, (0.30, 0.76, 1.0)), (1, (1.0, 0.56, 0.69))] {
                cr.move_to(0.0, h);
                for (i, s) in data.iter().enumerate() {
                    let v = if idx == 0 { s.0 } else { s.1 };
                    cr.line_to(w * i as f64 / n, h - (v / max) * h);
                }
                cr.line_to(w, h);
                cr.close_path();
                cr.set_source_rgba(r, g, bl, 0.22);
                let _ = cr.fill_preserve();
                cr.new_path();
                for (i, s) in data.iter().enumerate() {
                    let v = if idx == 0 { s.0 } else { s.1 };
                    let (x, y) = (w * i as f64 / n, h - (v / max) * h);
                    if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
                }
                cr.set_source_rgba(r, g, bl, 0.95);
                cr.set_line_width(1.6);
                let _ = cr.stroke();
            }
        });
    }
    area.add_css_class("net-graph");
    b.append(&rates);
    b.append(&area);
    let total = label("", &["dim-label", "caption"]);
    b.append(&total);

    let dev = dev.to_string();
    let last = RefCell::new(nm::counters(&dev));
    let weak = area.downgrade();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let Some(area) = weak.upgrade() else { return glib::ControlFlow::Break };
        let now = nm::counters(&dev);
        if let (Some((rx, tx)), Some((prx, ptx))) = (now, *last.borrow()) {
            let (d, u) = (rx.saturating_sub(prx) as f64, tx.saturating_sub(ptx) as f64);
            let mut hst = hist.borrow_mut();
            hst.pop_front();
            hst.push_back((d, u));
            down.set_text(&format!("↓ {}", nm::human_rate(d)));
            up.set_text(&format!("↑ {}", nm::human_rate(u)));
            total.set_text(&format!("Depuis le démarrage : ↓ {} · ↑ {}", nm::human_bytes(rx), nm::human_bytes(tx)));
            drop(hst);
            area.queue_draw();
        }
        *last.borrow_mut() = now;
        glib::ControlFlow::Continue
    });
    b
}

// ───────────────────────────── Propriétés ─────────────────────────────

pub fn properties(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Réseau et Internet"], "Propriétés de la connexion");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    holder.append(&spinner());
    content.append(&holder);
    let task = background(|| {
        let c = nm::primary();
        let info = c.as_ref().map(|c| nm::device_info(&c.device)).unwrap_or_default();
        let f = c.as_ref().map(|c| nm::con_fields(&c.uuid, "ipv4.method,ipv4.addresses,ipv4.gateway,connection.autoconnect")).unwrap_or_default();
        let speed = c.as_ref().and_then(|c| nm::link_speed(&c.device));
        (c, info, f, speed)
    });
    let app = app.clone();
    glib::spawn_future_local(async move {
        let (c, dinfo, f, speed) = task.await;
        clear(&holder);
        let Some(c) = c else {
            holder.append(&adap::StatusPage::builder().icon_name("network-offline-symbolic").title("Aucune connexion active").build());
            return;
        };
        let g = group(&c.name);
        g.add(&info_row_pair("Type", if c.is_wifi() { "Wi-Fi" } else { "Ethernet" }, &c.device));
        g.add(&info("Adresse matérielle (MAC)", dinfo.get("GENERAL.HWADDR").map(String::as_str).unwrap_or("")));
        if let Some(s) = speed {
            g.add(&info("Vitesse de la liaison", &if s >= 1000 { format!("{} Gbit/s", s as f64 / 1000.0) } else { format!("{s} Mbit/s") }));
        }
        g.add(&info("Adresses IPv4", &nm::indexed(&dinfo, "IP4.ADDRESS").join("\n")));
        g.add(&info("Passerelle IPv4", dinfo.get("IP4.GATEWAY").map(String::as_str).unwrap_or("")));
        g.add(&info("Adresses IPv6", &nm::indexed(&dinfo, "IP6.ADDRESS").join("\n")));
        let mut dns = nm::indexed(&dinfo, "IP4.DNS");
        dns.extend(nm::indexed(&dinfo, "IP6.DNS"));
        g.add(&info("Serveurs DNS utilisés", &dns.join("\n")));
        g.add(&info("MTU", dinfo.get("GENERAL.MTU").map(String::as_str).unwrap_or("")));
        holder.append(&g);

        // Configuration IPv4 : automatique (DHCP) ou manuelle.
        let g = group("Adresse IPv4");
        g.set_description(Some("Automatique convient presque toujours. Une adresse fixe sert pour un serveur, une imprimante partagée ou une redirection de port."));
        let manual = f.get("ipv4.method").is_some_and(|m| m == "manual");
        let mode = crate::ui::combo("preferences-system-network-symbolic", "Attribution", "", &["Automatique (DHCP)".into(), "Manuelle (adresse fixe)".into()], usize::from(manual));
        g.add(&mode);
        let current = nm::indexed(&dinfo, "IP4.ADDRESS").first().cloned().unwrap_or_default();
        let addr = adap::EntryRow::new();
        addr.set_title("Adresse / préfixe (ex. 192.168.1.50/24)");
        addr.set_text(f.get("ipv4.addresses").filter(|s| !s.is_empty() && *s != "--").map(String::as_str).unwrap_or(&current));
        let gw = adap::EntryRow::new();
        gw.set_title("Passerelle (ex. 192.168.1.1)");
        gw.set_text(f.get("ipv4.gateway").filter(|s| !s.is_empty() && *s != "--").map(String::as_str).or(dinfo.get("IP4.GATEWAY").map(String::as_str)).unwrap_or(""));
        addr.set_visible(manual);
        gw.set_visible(manual);
        g.add(&addr);
        g.add(&gw);
        let apply = gtk::Button::with_label("Appliquer");
        apply.add_css_class("suggested-action");
        apply.set_valign(gtk::Align::Center);
        g.set_header_suffix(Some(&apply));
        {
            let (addr, gw) = (addr.clone(), gw.clone());
            mode.connect_selected_notify(move |m| {
                addr.set_visible(m.selected() == 1);
                gw.set_visible(m.selected() == 1);
            });
        }
        {
            let (app, c) = (app.clone(), c.clone());
            apply.connect_clicked(move |_| {
                let manual = mode.selected() == 1;
                let (a, g) = (addr.text().trim().to_string(), gw.text().trim().to_string());
                if manual && (!a.contains('/') || a.parse::<std::net::Ipv4Addr>().is_ok()) {
                    app.toast("Indiquez l'adresse avec son préfixe, par exemple 192.168.1.50/24");
                    return;
                }
                let c = c.clone();
                let task = background(move || {
                    let s: Vec<(&str, &str)> = if manual {
                        vec![("ipv4.method", "manual"), ("ipv4.addresses", a.as_str()), ("ipv4.gateway", g.as_str())]
                    } else {
                        vec![("ipv4.method", "auto"), ("ipv4.addresses", ""), ("ipv4.gateway", "")]
                    };
                    nm::modify(&c.uuid, &s).and_then(|_| nm::nmcli(&["connection", "up", &c.uuid]).map(|_| ()))
                });
                let app = app.clone();
                glib::spawn_future_local(async move {
                    match task.await {
                        Ok(()) => app.toast("Adresse IPv4 appliquée"),
                        Err(e) => app.error("Adresse IPv4", &format!("{e:#}")),
                    }
                });
            });
        }
        holder.append(&g);
    });
    p
}

fn info_row_pair(title: &str, a: &str, b: &str) -> adap::ActionRow {
    info(title, &format!("{a} · {b}"))
}

// ───────────────────────────── Point d'accès ─────────────────────────────

pub fn hotspot(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Réseau et Internet"], "Point d'accès mobile");
    let g = group("Partager la connexion");
    g.set_description(Some("Ce PC crée un réseau Wi-Fi : téléphones et autres appareils s'y connectent et utilisent sa connexion (Ethernet de préférence : la carte Wi-Fi ne peut pas être à la fois connectée et point d'accès)."));
    let host = glib::host_name().to_string();
    let ssid = adap::EntryRow::new();
    ssid.set_title("Nom du réseau");
    ssid.set_text(&format!("NOOS-{host}"));
    let pass = adap::PasswordEntryRow::new();
    pass.set_title("Mot de passe (8 caractères minimum)");
    pass.set_text(&random_password());
    let band = crate::ui::combo("", "Bande", "5 GHz : plus rapide ; 2,4 GHz : plus loin et compatible avec tout", &["Automatique".into(), "5 GHz".into(), "2,4 GHz".into()], 0);
    let state = adap::SwitchRow::new();
    state.set_title("Point d'accès");
    state.set_subtitle("Désactivé");
    g.add(&state);
    g.add(&ssid);
    g.add(&pass);
    g.add(&band);
    content.append(&g);
    let active = nm::connections().iter().any(|c| c.name == "Hotspot" && c.active);
    state.set_active(active);
    state.set_subtitle(if active { "Actif : les appareils peuvent se connecter" } else { "Désactivé" });
    let guard = Rc::new(std::cell::Cell::new(false));
    let app = app.clone();
    state.connect_active_notify(move |s| {
        if guard.get() {
            return;
        }
        let on = s.is_active();
        let (ssid, pw, band) = (ssid.text().to_string(), pass.text().to_string(), band.selected());
        if on && pw.chars().count() < 8 {
            app.toast("Le mot de passe doit faire au moins 8 caractères");
            guard.set(true);
            s.set_active(false);
            guard.set(false);
            return;
        }
        s.set_sensitive(false);
        let task = background(move || -> anyhow::Result<()> {
            if on {
                let dev = nm::wifi_device().ok_or_else(|| anyhow::anyhow!("aucune carte Wi-Fi"))?;
                let mut args = vec!["device", "wifi", "hotspot", "ifname", dev.as_str(), "con-name", "Hotspot", "ssid", ssid.as_str(), "password", pw.as_str()];
                match band {
                    1 => args.extend(["band", "a"]),
                    2 => args.extend(["band", "bg"]),
                    _ => {}
                }
                nm::nmcli(&args).map(|_| ())
            } else {
                nm::nmcli(&["connection", "down", "Hotspot"]).map(|_| ())
            }
        });
        let (s, app, guard) = (s.clone(), app.clone(), guard.clone());
        glib::spawn_future_local(async move {
            let res = task.await;
            s.set_sensitive(true);
            match res {
                Ok(()) => s.set_subtitle(if on { "Actif : les appareils peuvent se connecter" } else { "Désactivé" }),
                Err(e) => {
                    app.error("Point d'accès mobile", &format!("{e:#}"));
                    guard.set(true);
                    s.set_active(!on);
                    guard.set(false);
                }
            }
        });
    });
    p
}

fn random_password() -> String {
    // Lettres et chiffres sans ambiguïté (pas de 0/O, 1/l).
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut seed = std::fs::read("/dev/urandom").ok().map(|_| ()).map_or(0u64, |_| 0);
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let mut b = [0u8; 8];
        let _ = std::io::Read::read_exact(&mut f, &mut b);
        seed = u64::from_le_bytes(b);
    }
    (0..12)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            CHARS[(seed % CHARS.len() as u64) as usize] as char
        })
        .collect()
}

// ───────────────────────────── Diagnostic ─────────────────────────────

struct Check {
    ok: bool,
    title: &'static str,
    detail: String,
    advice: &'static str,
}

fn diag_steps() -> Vec<Box<dyn FnOnce() -> Check + Send>> {
    vec![
        Box::new(|| {
            let c = nm::primary();
            Check {
                ok: c.is_some(),
                title: "Carte réseau connectée",
                detail: c.map(|c| format!("{} ({})", c.name, c.device)).unwrap_or_else(|| "aucune connexion active".into()),
                advice: "Branchez le câble Ethernet ou connectez-vous à un réseau Wi-Fi.",
            }
        }),
        Box::new(|| {
            let ip = nm::primary().map(|c| nm::device_info(&c.device)).and_then(|i| nm::indexed(&i, "IP4.ADDRESS").first().cloned());
            let ok = ip.as_ref().is_some_and(|a| !a.starts_with("169.254."));
            Check {
                ok,
                title: "Adresse IP obtenue",
                detail: ip.unwrap_or_else(|| "aucune".into()),
                advice: "Le routeur (box) n'a pas attribué d'adresse : redémarrez la box, ou vérifiez l'adresse IP manuelle dans Propriétés.",
            }
        }),
        Box::new(|| {
            let gw = nm::primary().map(|c| nm::device_info(&c.device)).and_then(|i| i.get("IP4.GATEWAY").cloned()).filter(|g| !g.is_empty() && g != "--");
            let ping = gw.as_ref().and_then(|g| ping(g));
            Check {
                ok: ping.is_some(),
                title: "Routeur (box) joignable",
                detail: match (&gw, ping) {
                    (Some(g), Some(ms)) => format!("{g} répond en {ms:.1} ms"),
                    (Some(g), None) => format!("{g} ne répond pas"),
                    _ => "pas de passerelle".into(),
                },
                advice: "La box ne répond pas : vérifiez qu'elle est allumée et que le câble ou le Wi-Fi fonctionne.",
            }
        }),
        Box::new(|| {
            let t = std::time::Instant::now();
            let ok = nm::demo() || std::net::ToSocketAddrs::to_socket_addrs(&("fedoraproject.org", 443)).is_ok_and(|mut a| a.next().is_some());
            Check {
                ok,
                title: "Résolution des noms (DNS)",
                detail: if ok { format!("fedoraproject.org résolu en {} ms", t.elapsed().as_millis().max(if nm::demo() { 18 } else { 0 })) } else { "échec".into() },
                advice: "Les noms de sites ne sont pas traduits en adresses : essayez un autre DNS (Réseau › DNS), ou « Automatique ».",
            }
        }),
        Box::new(|| {
            let state = if nm::demo() { "full".to_string() } else { nm::nmcli(&["networking", "connectivity", "check"]).map(|s| s.trim().to_string()).unwrap_or_default() };
            Check {
                ok: state == "full",
                title: "Accès à Internet",
                detail: match state.as_str() {
                    "full" => "complet".into(),
                    "portal" => "portail captif : ouvrez le navigateur pour vous identifier".into(),
                    "limited" => "limité : réseau local seulement".into(),
                    "none" => "aucun".into(),
                    s => s.to_string(),
                },
                advice: "Le réseau local fonctionne mais pas Internet : problème probable de la box ou de l'opérateur.",
            }
        }),
    ]
}

fn ping(host: &str) -> Option<f64> {
    if nm::demo() {
        return Some(0.8);
    }
    let out = std::process::Command::new("ping").args(["-c", "2", "-W", "1", "-q", host]).env("LC_ALL", "C").output().ok()?;
    if !out.status.success() {
        return None;
    }
    // rtt min/avg/max/mdev = 0.412/0.501/0.590/0.089 ms
    String::from_utf8_lossy(&out.stdout).lines().find(|l| l.contains("min/avg"))?.split('=').nth(1)?.split('/').nth(1)?.trim().parse().ok()
}

pub fn diagnostics(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Réseau et Internet"], "Diagnostic");
    let g = group("Vérifications");
    g.set_description(Some("Chaque étape dépend de la précédente : la première en échec indique où se trouve le problème."));
    let again = gtk::Button::with_label("Relancer");
    again.set_valign(gtk::Align::Center);
    g.set_header_suffix(Some(&again));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    g.add(&list);
    content.append(&g);
    let run = Rc::new({
        let list = list.clone();
        move || {
            list.remove_all();
            let rows: Vec<(adap::ActionRow, gtk::Spinner)> = (0..diag_steps().len())
                .map(|_| {
                    let r = adap::ActionRow::new();
                    r.set_title("En attente…");
                    let sp = gtk::Spinner::new();
                    r.add_prefix(&sp);
                    list.append(&r);
                    (r, sp)
                })
                .collect();
            let list = list.clone();
            glib::spawn_future_local(async move {
                let mut failed = false;
                for (step, (row, sp)) in diag_steps().into_iter().zip(rows) {
                    sp.start();
                    let c = background(step).await;
                    sp.stop();
                    row.remove(&sp);
                    row.set_title(c.title);
                    let ok = c.ok && !failed;
                    let icon = gtk::Image::from_icon_name(if c.ok { "emblem-ok-symbolic" } else { "dialog-error-symbolic" });
                    icon.add_css_class(if c.ok { "net-ok" } else { "net-bad" });
                    row.add_prefix(&icon);
                    row.set_subtitle(&if c.ok { c.detail } else { format!("{} — {}", c.detail, c.advice) });
                    row.set_subtitle_lines(3);
                    if !c.ok {
                        failed = true;
                    }
                    let _ = ok;
                }
                let r = adap::ActionRow::new();
                r.set_title(if failed { "Un problème a été trouvé" } else { "Tout fonctionne" });
                r.set_subtitle(if failed { "Suivez le conseil de la première étape en échec." } else { "Si un site ne s'ouvre pas, le problème vient sans doute du site lui-même." });
                r.add_prefix(&gtk::Image::from_icon_name(if failed { "dialog-warning-symbolic" } else { "face-smile-symbolic" }));
                list.append(&r);
            });
        }
    });
    run();
    again.connect_clicked(move |_| run());
    let _ = app;
    p
}

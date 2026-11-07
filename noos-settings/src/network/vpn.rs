//! Réseau › VPN : profils WireGuard (NetworkManager) — import d'un fichier .conf (bouton ou
//! glisser-déposer) avec aperçu, connexion / déconnexion, détails, démarrage automatique,
//! suppression, et vérification de l'adresse IP publique.

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gdk, gio, glib};

use super::{badge, clear, info, label, nm, spinner};
use crate::ui::{self, App, background, group};
use nm::Connection;

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = ui::page(app, &["Réseau et Internet"], "VPN");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    load(app, &holder);

    // Glisser-déposer d'un fichier .conf n'importe où sur la page.
    let drop = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
    {
        let (app, holder) = (app.clone(), holder.clone());
        drop.connect_drop(move |_, value, _, _| {
            if let Ok(f) = value.get::<gio::File>() {
                if let Some(path) = f.path() {
                    preview_import(&app, &holder, path);
                    return true;
                }
            }
            false
        });
    }
    content.add_controller(drop);
    p
}

fn load(app: &Rc<App>, holder: &gtk::Box) {
    clear(holder);
    holder.append(&spinner());
    let task = background(|| {
        nm::connections()
            .into_iter()
            .filter(Connection::is_wireguard)
            .map(|c| {
                let f = nm::con_fields(&c.uuid, "connection.autoconnect,ipv4.addresses,ipv4.dns,wireguard.peers,wireguard.listen-port");
                (c, f)
            })
            .collect::<Vec<_>>()
    });
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let list = task.await;
        clear(&holder);
        render(&app, &holder, list);
    });
}

/// Pair WireGuard affiché : « clé endpoint=… allowed-ips=… » → (serveur, trafic, clé).
fn peer_summary(peers: &str) -> (String, String, String) {
    let first = peers.split(',').next().unwrap_or("").trim();
    let mut it = first.split_whitespace();
    let key = it.next().unwrap_or("").to_string();
    let mut endpoint = String::new();
    let mut allowed = String::new();
    for kv in it {
        if let Some(v) = kv.strip_prefix("endpoint=") {
            endpoint = v.to_string();
        } else if let Some(v) = kv.strip_prefix("allowed-ips=") {
            allowed = v.replace(';', ", ");
        }
    }
    let traffic = if allowed.contains("0.0.0.0/0") || allowed.contains("::/0") {
        "Tout le trafic passe par le VPN".to_string()
    } else if allowed.is_empty() {
        "—".into()
    } else {
        format!("Seulement : {allowed}")
    };
    (endpoint, traffic, key)
}

fn render(app: &Rc<App>, holder: &gtk::Box, list: Vec<(Connection, std::collections::HashMap<String, String>)>) {
    let active = list.iter().find(|(c, _)| c.active).map(|(c, _)| c.clone());

    // ── Héros : état + IP publique ──
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    h.add_css_class("win11-hero");
    let img = gtk::Image::from_icon_name(if active.is_some() { "network-vpn-symbolic" } else { "network-vpn-disconnected-symbolic" });
    img.set_pixel_size(40);
    img.add_css_class("net-hero-icon");
    img.add_css_class(if active.is_some() { "net-ok" } else { "net-idle" });
    img.set_valign(gtk::Align::Center);
    h.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.set_hexpand(true);
    v.set_valign(gtk::Align::Center);
    v.append(&label(&active.as_ref().map(|c| format!("Protégé par « {} »", c.name)).unwrap_or_else(|| "Aucun VPN actif".into()), &["title-2"]));
    v.append(&label(
        if active.is_some() {
            "Votre trafic passe par un tunnel WireGuard chiffré."
        } else {
            "Votre opérateur et les sites visités voient votre adresse IP réelle."
        },
        &["dim-label"],
    ));
    let ip_line = label("", &["caption"]);
    ip_line.set_wrap(true);
    v.append(&ip_line);
    h.append(&v);
    let check = gtk::Button::with_label("Vérifier mon IP publique");
    check.set_valign(gtk::Align::Center);
    check.set_tooltip_text(Some("Interroge ipinfo.io pour voir l'adresse et le pays vus par les sites web"));
    {
        let ip_line = ip_line.clone();
        check.connect_clicked(move |b| {
            b.set_sensitive(false);
            ip_line.set_text("Interrogation d'ipinfo.io…");
            let task = background(nm::public_ip);
            let (b, ip_line) = (b.clone(), ip_line.clone());
            glib::spawn_future_local(async move {
                b.set_sensitive(true);
                match task.await {
                    Ok(ip) => ip_line.set_text(&format!(
                        "Adresse publique : {} — {}{} · {}",
                        ip.ip,
                        if ip.city.is_empty() { String::new() } else { format!("{}, ", ip.city) },
                        nm::country_name(&ip.country),
                        ip.org
                    )),
                    Err(e) => ip_line.set_text(&format!("Vérification impossible : {e:#}")),
                }
            });
        });
    }
    h.append(&check);
    holder.append(&h);

    // ── Profils ──
    let g = group("Profils WireGuard");
    let import = gtk::Button::new();
    import.set_child(Some(&adap::ButtonContent::builder().label("Importer un fichier .conf…").icon_name("document-open-symbolic").build()));
    import.add_css_class("suggested-action");
    import.set_valign(gtk::Align::Center);
    {
        let (app, holder) = (app.clone(), holder.clone());
        import.connect_clicked(move |_| choose_file(&app, &holder));
    }
    g.set_header_suffix(Some(&import));
    if list.is_empty() {
        g.set_description(Some(
            "Aucun profil. Téléchargez un fichier de configuration WireGuard depuis votre fournisseur (Mullvad, Proton VPN, IVPN, AirVPN…) ou votre propre serveur, puis importez-le — ou glissez-le sur cette page.",
        ));
        let empty = adap::StatusPage::builder()
            .icon_name("network-vpn-symbolic")
            .title("Pas encore de VPN")
            .description("Glissez un fichier .conf ici")
            .build();
        empty.add_css_class("compact");
        holder.append(&g);
        holder.append(&empty);
        return;
    }
    g.set_description(Some("Un seul VPN à la fois : en activer un coupe les autres. Glissez un fichier .conf sur la page pour en ajouter."));
    for (c, f) in list {
        let (endpoint, traffic, key) = peer_summary(f.get("wireguard.peers").map(String::as_str).unwrap_or(""));
        let r = adap::ExpanderRow::new();
        r.set_title(&c.name);
        r.set_subtitle(&if c.active { format!("Connecté · {endpoint}") } else { format!("Serveur {endpoint}") });
        r.add_prefix(&gtk::Image::from_icon_name("network-vpn-symbolic"));
        if c.active {
            r.add_suffix(&badge("Actif", "net-ok"));
        }
        let sw = gtk::Switch::new();
        sw.set_active(c.active);
        sw.set_valign(gtk::Align::Center);
        sw.set_tooltip_text(Some(if c.active { "Se déconnecter" } else { "Se connecter" }));
        {
            let (app, holder, uuid, others) = (app.clone(), holder.clone(), c.uuid.clone(), active.clone());
            sw.connect_state_set(move |sw, on| {
                sw.set_sensitive(false);
                let (uuid, others) = (uuid.clone(), others.clone());
                let task = background(move || -> anyhow::Result<()> {
                    if on {
                        if let Some(o) = others.filter(|o| o.uuid != uuid) {
                            let _ = nm::nmcli(&["connection", "down", &o.uuid]);
                        }
                        nm::nmcli(&["connection", "up", &uuid]).map(|_| ())
                    } else {
                        nm::nmcli(&["connection", "down", &uuid]).map(|_| ())
                    }
                });
                let (app, holder) = (app.clone(), holder.clone());
                glib::spawn_future_local(async move {
                    match task.await {
                        Ok(()) => app.toast(if on { "VPN connecté" } else { "VPN déconnecté" }),
                        Err(e) => app.error("VPN", &format!("{e:#}")),
                    }
                    load(&app, &holder);
                });
                glib::Propagation::Proceed
            });
        }
        r.add_suffix(&sw);

        r.add_row(&info("Serveur (endpoint)", &endpoint));
        r.add_row(&info("Trafic", &traffic));
        r.add_row(&info("Adresse dans le tunnel", f.get("ipv4.addresses").map(String::as_str).unwrap_or("")));
        r.add_row(&info("DNS du VPN", f.get("ipv4.dns").map(String::as_str).filter(|s| *s != "--").unwrap_or("ceux du réseau (attention aux fuites DNS)")));
        r.add_row(&info("Clé publique du serveur", &key));
        let auto = adap::SwitchRow::new();
        auto.set_title("Se connecter automatiquement");
        auto.set_subtitle("Dès qu'une connexion à Internet est disponible");
        auto.set_active(f.get("connection.autoconnect").is_some_and(|v| v == "yes"));
        {
            let (app, uuid) = (app.clone(), c.uuid.clone());
            auto.connect_active_notify(move |s| {
                let (uuid, on) = (uuid.clone(), s.is_active());
                let task = background(move || nm::modify(&uuid, &[("connection.autoconnect", if on { "yes" } else { "no" })]));
                let app = app.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = task.await {
                        app.error("VPN", &format!("{e:#}"));
                    }
                });
            });
        }
        r.add_row(&auto);
        let del_row = adap::ActionRow::new();
        del_row.set_title("Supprimer ce profil");
        del_row.set_subtitle("La configuration et sa clé privée seront effacées de ce PC");
        let del = gtk::Button::with_label("Supprimer");
        del.add_css_class("destructive-action");
        del.set_valign(gtk::Align::Center);
        {
            let (app, holder, uuid, name) = (app.clone(), holder.clone(), c.uuid.clone(), c.name.clone());
            del.connect_clicked(move |_| {
                let (app2, holder, uuid) = (app.clone(), holder.clone(), uuid.clone());
                app.confirm(&format!("Supprimer « {name} » ?"), "Il faudra réimporter le fichier .conf pour l'utiliser à nouveau.", "Supprimer", true, move || {
                    let uuid = uuid.clone();
                    let task = background(move || nm::nmcli(&["connection", "delete", &uuid]));
                    let (app, holder) = (app2.clone(), holder.clone());
                    glib::spawn_future_local(async move {
                        match task.await {
                            Ok(_) => app.toast("Profil supprimé"),
                            Err(e) => app.error("VPN", &format!("{e:#}")),
                        }
                        load(&app, &holder);
                    });
                });
            });
        }
        del_row.add_suffix(&del);
        r.add_row(&del_row);
        g.add(&r);
    }
    holder.append(&g);
}

fn choose_file(app: &Rc<App>, holder: &gtk::Box) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Configuration WireGuard (.conf)"));
    filter.add_pattern("*.conf");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder().title("Importer un profil WireGuard").filters(&filters).modal(true).build();
    let (app, holder) = (app.clone(), holder.clone());
    dialog.open(Some(&app.window.clone()), gio::Cancellable::NONE, move |res| {
        if let Ok(f) = res {
            if let Some(path) = f.path() {
                preview_import(&app, &holder, path);
            }
        }
    });
}

/// Aperçu du fichier (serveur, adresses, DNS, trafic) avec le nom du profil, puis import.
pub fn preview_import(app: &Rc<App>, holder: &gtk::Box, path: std::path::PathBuf) {
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return app.error("Import WireGuard", &e.to_string()),
    };
    let conf = match nm::parse_wg(&text) {
        Ok(c) => c,
        Err(e) => return app.error("Import WireGuard", &format!("Fichier non reconnu : {e:#}")),
    };
    let stem = path.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_else(|| "VPN".into());
    let body = format!(
        "Serveur : {}\nAdresse : {}\nDNS : {}\nTrafic : {}{}",
        conf.endpoint,
        conf.addresses.join(", "),
        if conf.dns.is_empty() { "ceux du réseau".into() } else { conf.dns.join(", ") },
        if conf.full_tunnel() { "tout passe par le VPN".to_string() } else { format!("seulement {}", conf.allowed.join(", ")) },
        if conf.peers > 1 { format!("\n{} serveurs (pairs) dans ce fichier", conf.peers) } else { String::new() },
    );
    let entry = gtk::Entry::new();
    entry.set_text(&stem);
    entry.set_activates_default(true);
    let d = adap::AlertDialog::new(Some("Importer ce profil WireGuard ?"), Some(&body));
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", "Annuler"), ("import", "Importer"), ("connect", "Importer et se connecter")]);
    d.set_response_appearance("connect", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("connect"));
    let (app2, holder) = (app.clone(), holder.clone());
    d.connect_response(None, move |_, r| {
        if r == "cancel" {
            return;
        }
        let connect = r == "connect";
        let name = entry.text().trim().to_string();
        let name = if name.is_empty() { "VPN".to_string() } else { name };
        let path = path.clone();
        let task = background(move || -> anyhow::Result<()> {
            let uuid = nm::import_wg(&path, &name)?;
            if connect {
                // Un seul VPN à la fois.
                for c in nm::connections().into_iter().filter(|c| c.is_wireguard() && c.active && c.uuid != uuid) {
                    let _ = nm::nmcli(&["connection", "down", &c.uuid]);
                }
                nm::nmcli(&["connection", "up", &uuid])?;
            }
            Ok(())
        });
        let (app, holder) = (app2.clone(), holder.clone());
        glib::spawn_future_local(async move {
            match task.await {
                Ok(()) => app.toast(if connect { "Profil importé et connecté" } else { "Profil importé" }),
                Err(e) => app.error("Import WireGuard", &format!("{e:#}")),
            }
            load(&app, &holder);
        });
    });
    d.present(Some(&app.window));
}

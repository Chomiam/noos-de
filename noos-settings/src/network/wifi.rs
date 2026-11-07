//! Réseau › Wi-Fi : activation, réseaux disponibles (signal, sécurité, bande), connexion avec
//! mot de passe, réseaux enregistrés (mot de passe, adresse MAC aléatoire, connexion
//! automatique, oubli).

use std::rc::Rc;

use adap::prelude::*;
use gtk::glib;

use super::{badge, clear, nm, spinner};
use crate::ui::{self, App, background, group};

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = ui::page(app, &["Réseau et Internet"], "Wi-Fi");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    load(app, &holder, false);
    p
}

type Saved = Vec<(nm::Connection, std::collections::HashMap<String, String>)>;

fn load(app: &Rc<App>, holder: &gtk::Box, rescan: bool) {
    clear(holder);
    holder.append(&spinner());
    let task = background(move || {
        let dev = nm::wifi_device();
        let on = nm::wifi_enabled();
        let aps = if dev.is_some() && on { nm::wifi_scan(rescan) } else { vec![] };
        let saved: Saved = nm::connections()
            .into_iter()
            .filter(nm::Connection::is_wifi)
            .filter(|c| c.name != "Hotspot")
            .map(|c| {
                let f = nm::con_fields(&c.uuid, "802-11-wireless.cloned-mac-address,connection.autoconnect");
                (c, f)
            })
            .collect();
        (dev, on, aps, saved)
    });
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        let (dev, on, aps, saved) = task.await;
        clear(&holder);
        if dev.is_none() {
            holder.append(
                &adap::StatusPage::builder()
                    .icon_name("network-wireless-disabled-symbolic")
                    .title("Aucune carte Wi-Fi")
                    .description("Ce PC n'a pas de carte Wi-Fi reconnue (ou elle est désactivée dans le BIOS).")
                    .build(),
            );
            return;
        }
        render(&app, &holder, on, aps, saved);
    });
}

fn render(app: &Rc<App>, holder: &gtk::Box, on: bool, aps: Vec<nm::Ap>, saved: Saved) {
    let g = group("");
    let radio = crate::ui::switch("network-wireless-symbolic", "Wi-Fi", if on { "Activé" } else { "Désactivé (mode avion Wi-Fi)" }, on);
    {
        let (app, holder) = (app.clone(), holder.clone());
        radio.connect_active_notify(move |s| {
            let on = s.is_active();
            let task = background(move || nm::nmcli(&["radio", "wifi", if on { "on" } else { "off" }]));
            let (app, holder) = (app.clone(), holder.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = task.await {
                    app.error("Wi-Fi", &format!("{e:#}"));
                }
                // Laisser la carte se réveiller avant la recherche.
                glib::timeout_future(std::time::Duration::from_millis(if on { 2500 } else { 0 })).await;
                load(&app, &holder, on);
            });
        });
    }
    g.add(&radio);
    holder.append(&g);
    if !on {
        return;
    }

    let g = group("Réseaux disponibles");
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some("Rechercher à nouveau"));
    {
        let (app, holder) = (app.clone(), holder.clone());
        refresh.connect_clicked(move |_| load(&app, &holder, true));
    }
    g.set_header_suffix(Some(&refresh));
    if aps.is_empty() {
        g.set_description(Some("Aucun réseau trouvé."));
    }
    for ap in aps {
        let r = adap::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&ap.ssid);
        let sec = if ap.security.is_empty() { "Ouvert (non sécurisé)".to_string() } else { format!("Sécurisé ({})", ap.security.trim()) };
        r.set_subtitle(&format!("{}{sec} · {} · signal {} %", if ap.in_use { "Connecté · " } else { "" }, ap.band, ap.signal));
        r.add_prefix(&gtk::Image::from_icon_name(nm::signal_icon(ap.signal)));
        if ap.security.is_empty() {
            r.add_suffix(&badge("Ouvert", "net-bad"));
        } else {
            r.add_suffix(&gtk::Image::from_icon_name("channel-secure-symbolic"));
        }
        if ap.in_use {
            r.add_suffix(&badge("Connecté", "net-ok"));
        } else {
            r.set_activatable(true);
            let known = saved.iter().find(|(c, _)| c.name == ap.ssid).map(|(c, _)| c.uuid.clone());
            let (app, holder, ssid, secured) = (app.clone(), holder.clone(), ap.ssid.clone(), !ap.security.is_empty());
            r.connect_activated(move |_| {
                if known.is_none() && secured {
                    ask_password(&app, &holder, &ssid);
                } else {
                    connect(&app, &holder, ssid.clone(), None, known.clone());
                }
            });
        }
        g.add(&r);
    }
    holder.append(&g);

    if saved.is_empty() {
        return;
    }
    let g = group("Réseaux enregistrés");
    for (c, f) in saved {
        let r = adap::ExpanderRow::new();
        r.set_title(&c.name);
        r.set_subtitle(if c.active { "Connecté" } else { "Enregistré" });
        r.add_prefix(&gtk::Image::from_icon_name("network-wireless-symbolic"));

        let pw = adap::ActionRow::new();
        pw.set_title("Mot de passe");
        pw.set_subtitle("••••••••");
        let show = gtk::Button::from_icon_name("view-reveal-symbolic");
        show.add_css_class("flat");
        show.set_valign(gtk::Align::Center);
        show.set_tooltip_text(Some("Afficher le mot de passe (pour le donner à un invité)"));
        {
            let (pw, uuid) = (pw.clone(), c.uuid.clone());
            show.connect_clicked(move |_| {
                let uuid = uuid.clone();
                let task = background(move || nm::wifi_password(&uuid));
                let pw = pw.clone();
                glib::spawn_future_local(async move {
                    pw.set_subtitle(&task.await.unwrap_or_else(|| "aucun (réseau ouvert) ou inaccessible".into()));
                    pw.set_subtitle_selectable(true);
                });
            });
        }
        pw.add_suffix(&show);
        r.add_row(&pw);

        let mac = adap::SwitchRow::new();
        mac.set_title("Adresse matérielle aléatoire");
        mac.set_subtitle("Empêche le suivi de ce PC d'un réseau à l'autre (hotspots publics, magasins)");
        mac.set_active(f.get("802-11-wireless.cloned-mac-address").is_some_and(|v| v == "random" || v == "stable"));
        setting(app, &mac, &c.uuid, "802-11-wireless.cloned-mac-address", "stable", "permanent");
        r.add_row(&mac);

        let auto = adap::SwitchRow::new();
        auto.set_title("Se connecter automatiquement");
        auto.set_active(f.get("connection.autoconnect").is_none_or(|v| v == "yes"));
        setting(app, &auto, &c.uuid, "connection.autoconnect", "yes", "no");
        r.add_row(&auto);

        let forget = adap::ActionRow::new();
        forget.set_title("Oublier ce réseau");
        let b = gtk::Button::with_label("Oublier");
        b.add_css_class("destructive-action");
        b.set_valign(gtk::Align::Center);
        {
            let (app, holder, uuid, name) = (app.clone(), holder.clone(), c.uuid.clone(), c.name.clone());
            b.connect_clicked(move |_| {
                let (app2, holder, uuid) = (app.clone(), holder.clone(), uuid.clone());
                app.confirm(&format!("Oublier « {name} » ?"), "Le mot de passe sera effacé de ce PC.", "Oublier", true, move || {
                    let uuid = uuid.clone();
                    let task = background(move || nm::nmcli(&["connection", "delete", &uuid]));
                    let (app, holder) = (app2.clone(), holder.clone());
                    glib::spawn_future_local(async move {
                        if let Err(e) = task.await {
                            app.error("Wi-Fi", &format!("{e:#}"));
                        }
                        load(&app, &holder, false);
                    });
                });
            });
        }
        forget.add_suffix(&b);
        r.add_row(&forget);
        g.add(&r);
    }
    holder.append(&g);
}

/// Interrupteur lié à un réglage de connexion (valeurs pour activé / désactivé).
fn setting(app: &Rc<App>, row: &adap::SwitchRow, uuid: &str, key: &'static str, on: &'static str, off: &'static str) {
    let (app, uuid) = (app.clone(), uuid.to_string());
    row.connect_active_notify(move |s| {
        let (uuid, v) = (uuid.clone(), if s.is_active() { on } else { off });
        let task = background(move || nm::modify(&uuid, &[(key, v)]));
        let app = app.clone();
        glib::spawn_future_local(async move {
            match task.await {
                Ok(()) => app.toast("Réglage enregistré (appliqué à la prochaine connexion)"),
                Err(e) => app.error("Wi-Fi", &format!("{e:#}")),
            }
        });
    });
}

fn ask_password(app: &Rc<App>, holder: &gtk::Box, ssid: &str) {
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_activates_default(true);
    let d = adap::AlertDialog::new(Some(&format!("Se connecter à « {ssid} »")), Some("Saisissez la clé de sécurité du réseau."));
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Se connecter")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let (app2, holder, ssid, e2) = (app.clone(), holder.clone(), ssid.to_string(), entry.clone());
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            connect(&app2, &holder, ssid.clone(), Some(e2.text().to_string()), None);
        }
    });
    d.present(Some(&app.window));
    glib::idle_add_local_once(move || {
        entry.grab_focus();
    });
}

fn connect(app: &Rc<App>, holder: &gtk::Box, ssid: String, password: Option<String>, known: Option<String>) {
    app.toast(&format!("Connexion à « {ssid} »…"));
    let task = background(move || match (known, password) {
        (Some(uuid), _) => nm::nmcli(&["connection", "up", &uuid]),
        (None, Some(pw)) => nm::nmcli(&["device", "wifi", "connect", &ssid, "password", &pw]),
        (None, None) => nm::nmcli(&["device", "wifi", "connect", &ssid]),
    });
    let (app, holder) = (app.clone(), holder.clone());
    glib::spawn_future_local(async move {
        match task.await {
            Ok(_) => app.toast("Connecté"),
            Err(e) => app.error("Wi-Fi", &format!("{e:#}")),
        }
        load(&app, &holder, false);
    });
}

//! Actions intégrées du bureau (son, luminosité, Paramètres, applications, session, réseau,
//! mises à jour). Mêmes briques que les applications (`noos_actions::Registry`) : l'IA n'agit
//! qu'à travers ces actions déclarées, jamais par une commande shell inventée par le modèle.

use std::{process::Command, rc::Rc};

use gtk::{gio, prelude::*};
use noos_actions::{ActionInfo, Kind, Outcome, Registry, Risk};
use noos_core::ipc::{IpcClient, Request};
use serde_json::Value;

/// Pages de Paramètres ouvrables (`noos-settings <page>`, liste `PAGES` de noos-settings).
pub const SETTINGS_PAGES: &[&str] = &[
    "network", "wifi", "dns", "vpn", "ip", "hotspot", "network-diagnostics", "display", "power", "notifications", "about",
    "devices", "personalization", "background", "colors", "themes", "taskbar", "windows", "login", "fonts", "lockscreen", "apps",
    "default-apps", "ai", "gaming", "privacy", "firewall", "updates",
];

/// Lance une commande avec l'environnement de la session graphique (par le compositeur :
/// le démon, activé par D-Bus, n'a pas forcément `WAYLAND_DISPLAY`).
fn spawn(command: &str) -> Result<(), String> {
    match IpcClient::fire(&Request::Spawn { command: command.into() }) {
        Ok(_) => Ok(()),
        Err(_) => Command::new("sh").args(["-c", command]).spawn().map(|_| ()).map_err(|e| format!("lancement impossible : {e}")),
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).output().map_err(|e| format!("{cmd} : {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn int(p: &Value, k: &str) -> i64 {
    p.get(k).and_then(Value::as_i64).unwrap_or(0)
}

fn on(p: &Value, k: &str) -> bool {
    matches!(p.get(k).and_then(Value::as_str), Some("on" | "oui" | "true"))
}

/// Volume actuel de la sortie par défaut (0–150).
fn volume() -> Option<i64> {
    let out = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).ok()?;
    let v: f64 = out.split_whitespace().nth(1)?.parse().ok()?;
    Some((v * 100.0).round() as i64)
}

/// Rétroéclairage : (périphérique, valeur, maximum).
fn backlight() -> Option<(String, u64, u64)> {
    let rd = std::fs::read_dir("/sys/class/backlight").ok()?;
    let dir = rd.flatten().max_by_key(|e| std::fs::read_to_string(e.path().join("max_brightness")).ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0))?;
    let read = |f: &str| std::fs::read_to_string(dir.path().join(f)).ok()?.trim().parse::<u64>().ok();
    Some((dir.file_name().to_string_lossy().into_owned(), read("brightness")?, read("max_brightness")?))
}

/// Luminosité en % via logind (sans droits root).
fn set_brightness(pct: i64) -> Outcome {
    let (dev, _, max) = backlight().ok_or("Aucun écran à luminosité réglable (écran externe ?).")?;
    let value = ((pct.clamp(1, 100) as u64) * max / 100).max(1);
    run(
        "busctl",
        &["call", "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "SetBrightness", "ssu", "backlight", &dev, &value.to_string()],
    )?;
    Ok(format!("Luminosité réglée à {} %.", pct.clamp(1, 100)))
}

/// Alias français → identifiant d'application.
const APP_ALIASES: &[(&str, &str)] = &[
    ("terminal", "org.noos.Terminal"),
    ("console", "org.noos.Terminal"),
    ("explorateur", "org.noos.Files"),
    ("explorateur de fichiers", "org.noos.Files"),
    ("fichiers", "org.noos.Files"),
    ("mes fichiers", "org.noos.Files"),
    ("parametres", "org.noos.Settings"),
    ("reglages", "org.noos.Settings"),
    ("logiciels", "org.noos.Software"),
    ("boutique", "org.noos.Software"),
    ("magasin", "org.noos.Software"),
    ("capture", "org.noos.Snip"),
    ("outil capture", "org.noos.Snip"),
    ("photos", "org.noos.PhotoView"),
    ("visionneuse", "org.noos.PhotoView"),
    ("archives", "org.noos.Archive"),
    ("editeur", "org.noos.TextEditor"),
    ("editeur de texte", "org.noos.TextEditor"),
    ("bloc notes", "org.noos.TextEditor"),
    ("sauvegarde", "org.noos.Save"),
    ("sauvegarder", "org.noos.Save"),
    ("navigateur", "firefox"),
    ("internet", "firefox"),
];

/// Trouve l'application la plus proche d'un nom (« firefox », « le terminal », « steam »).
pub fn find_app(name: &str) -> Option<gio::AppInfo> {
    let want = crate::rules::normalize(name);
    let want = want.trim_start_matches("l ").trim();
    if let Some((_, id)) = APP_ALIASES.iter().find(|(a, _)| *a == want) {
        let id = id.to_lowercase();
        if let Some(a) = gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i.to_lowercase().trim_end_matches(".desktop") == id)) {
            return Some(a);
        }
    }
    let mut best: Option<(u32, gio::AppInfo)> = None;
    for a in gio::AppInfo::all().into_iter().filter(|a| a.should_show()) {
        let name = crate::rules::normalize(&a.display_name());
        let id = a.id().map(|s| s.to_lowercase()).unwrap_or_default();
        let exe = a.executable().file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let score = if name == want || exe == want {
            100
        } else if name.starts_with(want) {
            80
        } else if name.split(' ').any(|w| w == want) || id.contains(want) {
            60
        } else if name.contains(want) {
            40
        } else {
            0
        };
        if score > best.as_ref().map(|b| b.0).unwrap_or(0) {
            best = Some((score, a));
        }
    }
    best.map(|b| b.1)
}

fn notify(summary: &str, body: &str) {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    let hints = gtk::glib::VariantDict::new(None).end();
    let args = ("NOOS", 0u32, "starred-symbolic", summary, body, Vec::<String>::new(), hints, -1i32).to_variant();
    let _ = conn.call_sync(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
    );
}

/// Registre des actions du bureau.
pub fn system() -> Rc<Registry> {
    let r = Registry::new("System", "Bureau NOOS");

    // ── Son ──
    r.add(
        ActionInfo::new("system.volume.set", "Régler le volume", "Règle le volume de la sortie audio (0 à 150 %)", Risk::Change).param("pourcentage", Kind::Integer, "volume en %", true),
        |p| {
            let v = int(p, "pourcentage").clamp(0, 150);
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])?;
            run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", v as f64 / 100.0)])?;
            Ok(format!("Volume réglé à {v} %."))
        },
    );
    r.add(
        ActionInfo::new("system.volume.change", "Monter ou baisser le son", "Change le volume d'un écart en points (+10, -10…)", Risk::Change).param("delta", Kind::Integer, "écart en points de %", true),
        |p| {
            let v = (volume().unwrap_or(50) + int(p, "delta")).clamp(0, 100);
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])?;
            run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{:.2}", v as f64 / 100.0)])?;
            Ok(format!("Volume à {v} %."))
        },
    );
    r.add(
        ActionInfo::new("system.mute", "Couper ou remettre le son", "Coupe (on) ou rétablit (off) le son", Risk::Change).param("etat", Kind::String, "on = couper, off = rétablir", true).choices(&["on", "off"]),
        |p| {
            let m = on(p, "etat");
            run("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", if m { "1" } else { "0" }])?;
            Ok(if m { "Son coupé." } else { "Son rétabli." }.into())
        },
    );

    // ── Luminosité ──
    r.add(
        ActionInfo::new("system.brightness.set", "Régler la luminosité", "Règle la luminosité de l'écran intégré (1 à 100 %)", Risk::Change).param("pourcentage", Kind::Integer, "luminosité en %", true),
        |p| set_brightness(int(p, "pourcentage")),
    );
    r.add(
        ActionInfo::new("system.brightness.change", "Augmenter ou baisser la luminosité", "Change la luminosité d'un écart en points", Risk::Change).param("delta", Kind::Integer, "écart en points de %", true),
        |p| {
            let (_, cur, max) = backlight().ok_or("Aucun écran à luminosité réglable.")?;
            set_brightness((cur * 100 / max.max(1)) as i64 + int(p, "delta"))
        },
    );

    // ── Apparence ──
    r.add(
        ActionInfo::new("appearance.scheme", "Mode sombre ou clair", "Passe le bureau en mode sombre ou clair", Risk::Change).param("mode", Kind::String, "sombre ou clair", true).choices(&["sombre", "clair"]),
        |p| {
            let dark = p.get("mode").and_then(Value::as_str) == Some("sombre");
            let mut cfg = noos_core::NoosConfig::load();
            cfg.appearance.color_scheme = if dark { noos_core::config::ColorScheme::Dark } else { noos_core::config::ColorScheme::Light };
            cfg.save().map_err(|e| format!("{e}"))?;
            noos_theme::install_theme(&cfg.appearance).map_err(|e| format!("{e:#}"))?;
            let _ = IpcClient::fire(&Request::ReloadConfig);
            Ok(if dark { "Mode sombre activé." } else { "Mode clair activé." }.into())
        },
    );

    // ── Ouvrir ──
    r.add(
        ActionInfo::new("settings.open", "Ouvrir les Paramètres", "Ouvre l'application Paramètres, éventuellement sur une page", Risk::Read)
            .param("page", Kind::String, "page à afficher", false)
            .choices(SETTINGS_PAGES),
        |p| {
            let page = p.get("page").and_then(Value::as_str).unwrap_or("");
            spawn(&format!("noos-settings {page}"))?;
            Ok("J'ouvre les Paramètres.".into())
        },
    );
    r.add(
        ActionInfo::new("app.open", "Ouvrir une application", "Lance une application installée d'après son nom", Risk::Read).param("nom", Kind::String, "nom de l'application", true),
        |p| {
            let name = p.get("nom").and_then(Value::as_str).unwrap_or("");
            let app = find_app(name).ok_or_else(|| format!("Je ne trouve pas d'application « {name} »."))?;
            let cmd = app.commandline().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
            // Codes de champ des .desktop (%U, %f…) : sans objet ici.
            let cmd: String = cmd.split_whitespace().filter(|w| !w.starts_with('%')).collect::<Vec<_>>().join(" ");
            spawn(&cmd)?;
            Ok(format!("J'ouvre {}.", app.display_name()))
        },
    );
    r.add(ActionInfo::new("system.screenshot", "Capture d'écran", "Ouvre l'outil de capture d'écran", Risk::Read), |_| {
        spawn("noos-snip --app")?;
        Ok("Outil de capture ouvert.".into())
    });

    // ── Réseau ──
    r.add(
        ActionInfo::new("network.wifi", "Activer ou couper le Wi-Fi", "Active (on) ou désactive (off) le Wi-Fi", Risk::Change).param("etat", Kind::String, "on ou off", true).choices(&["on", "off"]),
        |p| {
            let o = on(p, "etat");
            run("nmcli", &["radio", "wifi", if o { "on" } else { "off" }])?;
            Ok(if o { "Wi-Fi activé." } else { "Wi-Fi désactivé." }.into())
        },
    );

    // ── Système et session (confirmation) ──
    r.add(ActionInfo::new("system.update", "Mettre à jour le système", "Installe les mises à jour du système et des applications", Risk::System), |_| {
        // Long (minutes) : en arrière-plan, notification à la fin. Élévation par polkit.
        std::thread::spawn(|| {
            notify("Mise à jour en cours", "Les mises à jour du système s'installent…");
            match Command::new("noos-pkg").arg("upgrade").output() {
                Ok(o) if o.status.success() => notify("Système à jour", "Toutes les mises à jour sont installées."),
                Ok(o) => notify("Échec de la mise à jour", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("erreur inconnue")),
                Err(e) => notify("Échec de la mise à jour", &e.to_string()),
            }
        });
        Ok("Mise à jour lancée : je vous préviens quand c'est fini.".into())
    });
    r.add(ActionInfo::new("session.lock", "Verrouiller", "Verrouille la session", Risk::Read), |_| {
        IpcClient::fire(&Request::Lock).map_err(|e| e.to_string())?;
        Ok("Session verrouillée.".into())
    });
    r.add(ActionInfo::new("session.suspend", "Mettre en veille", "Met l'ordinateur en veille", Risk::System), |_| {
        IpcClient::fire(&Request::Suspend).map_err(|e| e.to_string())?;
        Ok("Mise en veille.".into())
    });
    r.add(ActionInfo::new("session.reboot", "Redémarrer", "Redémarre l'ordinateur", Risk::System), |_| {
        run("systemctl", &["reboot"]).map(|_| "Redémarrage…".into())
    });
    r.add(ActionInfo::new("session.poweroff", "Éteindre", "Éteint l'ordinateur", Risk::System), |_| {
        run("systemctl", &["poweroff"]).map(|_| "Extinction…".into())
    });
    r.add(ActionInfo::new("session.logout", "Se déconnecter", "Ferme la session", Risk::System), |_| {
        IpcClient::fire(&Request::Quit).map_err(|e| e.to_string())?;
        Ok("Déconnexion…".into())
    });

    // ── Informations ──
    r.add(ActionInfo::new("info.time", "Heure", "Donne l'heure", Risk::Read), |_| {
        let now = gtk::glib::DateTime::now_local().map_err(|e| e.to_string())?;
        Ok(format!("Il est {}.", now.format("%H h %M").map_err(|e| e.to_string())?))
    });
    r.add(ActionInfo::new("info.date", "Date", "Donne la date du jour", Risk::Read), |_| {
        const DAYS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
        const MONTHS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
        let d = gtk::glib::DateTime::now_local().map_err(|e| e.to_string())?;
        Ok(format!("Nous sommes le {} {} {} {}.", DAYS[(d.day_of_week() - 1) as usize], d.day_of_month(), MONTHS[(d.month() - 1) as usize], d.year()))
    });
    r
}

#[cfg(test)]
mod tests {
    #[test]
    fn rules_match_registry() {
        // Toute action produite par les règles existe et accepte ses paramètres.
        let reg = super::system();
        let ids: Vec<_> = reg.list().actions;
        for t in ["monte le son à 90", "coupe le son", "ouvre les paramètres réseau", "mets à jour", "verrouille", "passe en mode sombre", "désactive le wifi", "baisse la luminosité", "quelle heure"] {
            let i = crate::rules::parse(t).unwrap_or_else(|| panic!("{t}"));
            let a = ids.iter().find(|a| a.id == i.action).unwrap_or_else(|| panic!("{}", i.action));
            a.check(&i.params).unwrap();
        }
    }
}

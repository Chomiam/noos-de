//! Intégration à la session systemd --user (vraie session uniquement, jamais en imbriqué).
//!
//! `xdg-desktop-portal.service` exige `graphical-session.target` (Requisite=) : sans lui,
//! aucun portail ne démarre (pas de sélecteur de fichiers pour les Flatpak, pas de thème
//! ni d'icônes dans les Flatpak, pas d'ouverture de liens entre applis, pas de partage
//! d'écran). Comme sway/niri, NOOS fournit une cible `noos-session.target` liée à
//! `graphical-session.target`.
//!
//! Tout est écrit dans `$XDG_RUNTIME_DIR` (tmpfs, effacé à la déconnexion) : rien n'est
//! installé de façon permanente et la session GNOME n'est pas touchée.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use tracing::{info, warn};

const TARGET: &str = "noos-session.target";
const WLR_BUS: &str = "org.freedesktop.impl.portal.desktop.wlr";
/// Applications GApplication de NOOS lancées par activation D-Bus (fichier .desktop
/// avec `DBusActivatable=true`).
const DBUS_APPS: &[(&str, &str)] = &[("org.noos.Settings", "noos-settings")];

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Binaire xdg-desktop-portal-wlr : fourni avec NOOS (préfixe), sinon celui du système.
fn xdpw_binary() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(prefix) = std::env::var("NOOS_PREFIX") {
        candidates.push(Path::new(&prefix).join("libexec/xdg-desktop-portal-wlr"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(Path::new(&home).join(".local/opt/noos/libexec/xdg-desktop-portal-wlr"));
    }
    candidates.push("/usr/libexec/xdg-desktop-portal-wlr".into());
    candidates.into_iter().find(|p| p.is_file())
}

fn write(path: &Path, content: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(path, content) {
        warn!("écriture de {} impossible : {e}", path.display());
    }
}

fn systemctl(args: &[&str]) -> bool {
    Command::new("systemctl").arg("--user").args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// Prépare les portails et démarre la cible de session. `vars` : variables à exporter
/// vers systemd --user et l'activation D-Bus.
pub fn start(vars: &mut Vec<&'static str>) {
    let run = runtime_dir();

    // ── Portails : dossier des définitions (= système + portail wlr de NOOS) ──
    let portals = run.join("noos/portals");
    let _ = std::fs::remove_dir_all(&portals);
    let _ = std::fs::create_dir_all(&portals);
    if let Ok(entries) = std::fs::read_dir("/usr/share/xdg-desktop-portal/portals") {
        for e in entries.flatten() {
            let _ = std::os::unix::fs::symlink(e.path(), portals.join(e.file_name()));
        }
    }
    let xdpw = xdpw_binary();
    if let Some(bin) = &xdpw {
        let _ = std::fs::remove_file(portals.join("wlr.portal"));
        write(
            &portals.join("wlr.portal"),
            &format!(
                "[portal]\nDBusName={WLR_BUS}\nInterfaces=org.freedesktop.impl.portal.ScreenCast;org.freedesktop.impl.portal.Screenshot;\nUseIn=NOOS;wlroots;\n"
            ),
        );
        // Configuration : un seul écran → partagé directement ; plusieurs → le premier
        // (le sélecteur NOOS viendra plus tard).
        let conf = run.join("noos/xdpw.conf");
        write(&conf, "[screencast]\nmax_fps=60\nchooser_type=none\n");
        // Unité systemd + activation D-Bus (dossiers d'exécution, pris en compte par systemd/dbus).
        write(
            &run.join("systemd/user/noos-portal-wlr.service"),
            &format!(
                "[Unit]\nDescription=Portail de partage d'écran (xdg-desktop-portal-wlr) pour NOOS\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n[Service]\nType=dbus\nBusName={WLR_BUS}\nExecStart={} -c {}\nRestart=on-failure\n",
                bin.display(),
                conf.display()
            ),
        );
        write(
            &run.join(format!("dbus-1/services/{WLR_BUS}.service")),
            &format!(
                "[D-BUS Service]\nName={WLR_BUS}\nExec={}\nSystemdService=noos-portal-wlr.service\n",
                bin.display()
            ),
        );
    } else {
        warn!("xdg-desktop-portal-wlr introuvable : pas de partage d'écran (relancez scripts/install-user.sh)");
    }
    // ── Applications NOOS activables par D-Bus ──
    // La barre des tâches les ouvre alors par un simple appel D-Bus à l'instance déjà en
    // mémoire (Paramètres est préchargé), sans lancer de nouveau processus.
    for (app_id, bin) in DBUS_APPS {
        let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(bin)));
        if let Some(exe) = exe.filter(|e| e.is_file()) {
            let mut env = String::new();
            for var in ["LD_LIBRARY_PATH", "NOOS_DATA_DIR", "XDG_DATA_DIRS", "PATH"] {
                if let Ok(v) = std::env::var(var) {
                    env.push_str(&format!(" {var}={v}"));
                }
            }
            write(
                &run.join(format!("dbus-1/services/{app_id}.service")),
                &format!("[D-BUS Service]\nName={app_id}\nExec=/usr/bin/env{env} {} --gapplication-service\n", exe.display()),
            );
        }
    }

    // Choix des portails (aussi installé dans ~/.config/xdg-desktop-portal par install-user.sh :
    // selon sa version, xdg-desktop-portal lit sa configuration ici ou là).
    let mut conf = String::from("[preferred]\ndefault=gtk\n");
    if xdpw.is_some() {
        conf.push_str("org.freedesktop.impl.portal.ScreenCast=wlr\n");
    }
    write(&portals.join("noos-portals.conf"), &conf);
    // SAFETY : appelé depuis startup(), avant tout autre thread.
    unsafe { std::env::set_var("XDG_DESKTOP_PORTAL_DIR", &portals) };
    vars.push("XDG_DESKTOP_PORTAL_DIR");
    vars.push("XDG_SESSION_TYPE");

    // ── Cible de session ──
    // Service de secrets (mots de passe des applis, porte-clés) : GNOME le démarre
    // explicitement (oo7 n'est pas activable par D-Bus) ; sans lui, Chrome, l'app Claude…
    // ne retrouvent plus leurs secrets. Démarré tôt, il reçoit le mot de passe de connexion
    // (module PAM) et déverrouille le trousseau « Login » automatiquement.
    let secrets = if Path::new("/usr/lib/systemd/user/oo7-daemon.service").exists() {
        "Wants=oo7-daemon.service\n"
    } else if Path::new("/usr/lib/systemd/user/gnome-keyring-daemon.service").exists() {
        "Wants=gnome-keyring-daemon.service\n"
    } else {
        ""
    };
    write(
        &run.join(format!("systemd/user/{TARGET}")),
        &format!("[Unit]\nDescription=Session de bureau NOOS\nDocumentation=man:systemd.special(7)\nBindsTo=graphical-session.target\nWants=graphical-session-pre.target\nAfter=graphical-session-pre.target\nWants=xdg-desktop-autostart.target\nBefore=xdg-desktop-autostart.target\n{secrets}"),
    );

    import_environment(vars);
    // dbus-broker ne relit ses dossiers de services que sur demande.
    let _ = Command::new("busctl")
        .args(["--user", "call", "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "ReloadConfig"])
        .status();
    systemctl(&["daemon-reload"]);
    // Un essai raté plus tôt (portail démarré sans cible) laisse l'unité en échec.
    systemctl(&["reset-failed", "xdg-desktop-portal.service", "xdg-desktop-portal-gtk.service"]);
    // Bloquant : la cible doit être active avant que le shell n'interroge les portails
    // (l'autostart XDG est ordonné après, il ne retarde pas ce démarrage).
    if systemctl(&["start", TARGET]) {
        info!(portail_wlr = xdpw.is_some(), "session systemd démarrée ({TARGET})");
    } else {
        warn!("impossible de démarrer {TARGET} : portails XDG indisponibles");
    }
}

fn import_environment(vars: &[&str]) {
    let _ = Command::new("systemctl").args(["--user", "import-environment"]).args(vars).status();
    let mut busctl = Command::new("busctl");
    busctl.args([
        "--user",
        "call",
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "UpdateActivationEnvironment",
        "a{ss}",
    ]);
    let pairs: Vec<(String, String)> =
        vars.iter().filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v))).collect();
    busctl.arg(pairs.len().to_string());
    for (k, v) in pairs {
        busctl.arg(k).arg(v);
    }
    let _ = busctl.status();
}

/// À la fermeture de NOOS : arrête les services liés à la session graphique et retire
/// nos variables de l'environnement systemd (pour la prochaine session, GNOME compris).
pub fn stop() {
    info!("arrêt de la session systemd");
    systemctl(&["stop", TARGET]);
    systemctl(&[
        "unset-environment",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "NOOS_IPC_SOCKET",
        "ADAP_DEBUG_THEME_NAME",
        "XDG_DESKTOP_PORTAL_DIR",
    ]);
    let run = runtime_dir();
    let _ = std::fs::remove_file(run.join(format!("systemd/user/{TARGET}")));
    let _ = std::fs::remove_file(run.join("systemd/user/noos-portal-wlr.service"));
    let _ = std::fs::remove_file(run.join(format!("dbus-1/services/{WLR_BUS}.service")));
    for (app_id, _) in DBUS_APPS {
        let _ = std::fs::remove_file(run.join(format!("dbus-1/services/{app_id}.service")));
    }
    let _ = std::fs::remove_dir_all(run.join("noos/portals"));
}

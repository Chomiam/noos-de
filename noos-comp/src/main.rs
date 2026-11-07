//! noos-comp — compositeur Wayland de NOOS.
//!
//! ```text
//! noos-comp            # auto : winit si lancé dans une session graphique, sinon DRM
//! noos-comp --winit    # dans une fenêtre (développement)
//! noos-comp --drm      # session réelle (depuis GDM ou un TTY)
//! noos-comp --no-shell # sans lancer noos-shell
//! ```

mod backend;
mod blur;
mod capture;
mod cursor;
mod grabs;
mod handlers;
mod idle;
mod lock;
mod input;
mod ipc;
mod render;
mod screenshot;
mod session;
mod state;
mod tiling;
mod wm;

use std::{path::Path, sync::OnceLock};

use tracing::{info, warn};

use crate::state::NoosState;

/// `DISPLAY` de xwayland-satellite (si lancé), transmis aux clients.
pub static XWAYLAND_DISPLAY: OnceLock<String> = OnceLock::new();

/// Mode kiosque (`--kiosk "<commande>"`) : une seule application plein écran, sans barre
/// des tâches ni raccourcis (écran de connexion noos-login, installateur). Le compositeur
/// s'arrête quand l'application se termine.
pub static KIOSK: OnceLock<String> = OnceLock::new();

pub fn kiosk() -> bool {
    KIOSK.get().is_some()
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Winit,
    Drm,
}

fn main() -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,smithay=warn"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = if args.iter().any(|a| a == "--winit") {
        Mode::Winit
    } else if args.iter().any(|a| a == "--drm" || a == "--udev" || a == "--tty") {
        Mode::Drm
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some() {
        Mode::Winit
    } else {
        Mode::Drm
    };
    if args.iter().any(|a| a == "--no-shell") {
        // SAFETY : encore mono-thread à ce stade.
        unsafe { std::env::set_var("NOOS_NO_SHELL", "1") };
    }
    if let Some(i) = args.iter().position(|a| a == "--kiosk") {
        let Some(cmd) = args.get(i + 1) else {
            anyhow::bail!("--kiosk attend une commande");
        };
        let _ = KIOSK.set(cmd.clone());
        // SAFETY : encore mono-thread à ce stade.
        unsafe { std::env::set_var("NOOS_NO_SHELL", "1") };
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("noos-comp [--winit | --drm] [--no-shell] [--kiosk \"<commande>\"]");
        return Ok(());
    }

    // Même thème et même taille de curseur pour le compositeur et pour les applis qui
    // dessinent encore le leur (X11, anciennes applis) : réglages GNOME de l'utilisateur.
    for (var, key, default) in [("XCURSOR_THEME", "cursor-theme", "Adwaita"), ("XCURSOR_SIZE", "cursor-size", "24")] {
        if std::env::var_os(var).is_none() {
            let value = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", key])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string());
            // SAFETY : encore mono-thread à ce stade.
            unsafe { std::env::set_var(var, value) };
        }
    }

    info!(?mode, "démarrage de NOOS");
    match mode {
        Mode::Winit => backend::winit::run(),
        Mode::Drm => {
            let result = backend::udev::run();
            if !kiosk() {
                session::stop();
            }
            result
        }
    }
}

/// Appelé une fois le backend prêt : environnement de session, Xwayland, shell.
pub fn startup(state: &mut NoosState) {
    let nested = matches!(state.backend, backend::Backend::Winit(_));

    // SAFETY : la boucle d'événements n'a pas encore démarré, aucun autre thread ne lit l'env.
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);
        std::env::set_var("NOOS_IPC_SOCKET", state.ipc.path());
        std::env::set_var("XDG_CURRENT_DESKTOP", "NOOS");
        std::env::set_var("XDG_SESSION_DESKTOP", "noos");
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("MOZ_ENABLE_WAYLAND", "1");
        std::env::set_var("QT_QPA_PLATFORM", "wayland;xcb");
        std::env::set_var("ELECTRON_OZONE_PLATFORM_HINT", "auto");
        // Thème libAdapta de NOOS pour toutes les applis libAdapta de la session.
        if std::env::var_os("ADAP_DEBUG_THEME_NAME").is_none() {
            std::env::set_var("ADAP_DEBUG_THEME_NAME", "NOOS");
        }
        std::env::remove_var("DISPLAY");
        // Aperçu dans une fenêtre : les clients ne doivent pas se croire seuls sur le bus
        // de session (qui est celui de la session hôte).
        if nested {
            std::env::set_var("NOOS_NESTED", "1");
        } else {
            std::env::remove_var("NOOS_NESTED");
        }
    }

    if let Some(cmd) = KIOSK.get() {
        // Pas de session de bureau : ni Xwayland, ni portails, ni mise en veille de l'écran.
        state.config.tiling.enabled = false;
        // Pas d'extinction ni de mise en veille (installation en cours) ; seule la fin de
        // l'application est surveillée.
        state.start_reaper();
        state.spawn(cmd);
        info!(socket = ?state.socket_name, command = cmd, "NOOS prêt (kiosque)");
        return;
    }

    start_xwayland(state);
    state.start_idle_timer();

    if !nested {
        // Rend WAYLAND_DISPLAY & co visibles des services D-Bus/systemd (portails, applis activées).
        // Jamais en mode imbriqué : on écraserait l'environnement de la session hôte.
        let mut vars = vec![
            "WAYLAND_DISPLAY",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "NOOS_IPC_SOCKET",
            "ADAP_DEBUG_THEME_NAME",
            "XCURSOR_THEME",
            "XCURSOR_SIZE",
        ];
        if XWAYLAND_DISPLAY.get().is_some() {
            vars.push("DISPLAY");
        }
        session::start(&mut vars);
    }

    if std::env::var_os("NOOS_NO_SHELL").is_none() {
        state.spawn("noos-shell");
    }
    if let Ok(extra) = std::env::var("NOOS_AUTOSTART") {
        for cmd in extra.split(';').filter(|c| !c.trim().is_empty()) {
            state.spawn(cmd);
        }
    }
    info!(socket = ?state.socket_name, "NOOS prêt");
}

/// Xwayland via xwayland-satellite (applis X11, Steam, beaucoup de jeux).
fn start_xwayland(state: &NoosState) {
    let found = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("xwayland-satellite").is_file()))
        .unwrap_or(false);
    if !found {
        warn!("xwayland-satellite introuvable : pas de support X11");
        return;
    }
    let display = (1..64)
        .find(|n| !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists() && !Path::new(&format!("/tmp/.X{n}-lock")).exists())
        .unwrap_or(42);
    let name = format!(":{display}");
    match std::process::Command::new("xwayland-satellite")
        .arg(&name)
        .env("WAYLAND_DISPLAY", &state.socket_name)
        .spawn()
    {
        Ok(_) => {
            info!(display = name, "xwayland-satellite lancé");
            let _ = XWAYLAND_DISPLAY.set(name.clone());
            unsafe { std::env::set_var("DISPLAY", &name) };
        }
        Err(e) => warn!("xwayland-satellite : {e}"),
    }
}

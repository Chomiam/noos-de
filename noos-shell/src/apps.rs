//! Applications installées (fichiers .desktop) et lancement.

use gtk::{gdk, gio, glib, prelude::*};
use noos_core::ipc::{IpcClient, Request};

/// Applications NOOS : commande de secours si le .desktop n'est pas encore installé.
const BUILTINS: &[(&str, &str, &str, &str)] = &[
    ("org.noos.Files", "Explorateur de fichiers", "system-file-manager", "noos-file"),
    ("org.noos.Settings", "Paramètres", "preferences-system", "noos-settings"),
    ("org.noos.Software", "Logiciels", "system-software-install", "noos-software"),
    ("org.noos.Terminal", "Terminal", "utilities-terminal", "noos-term"),
    ("org.noos.Snip", "Outil Capture d'écran", "applets-screenshooter", "noos-snip --app"),
];

pub fn find(id: &str) -> Option<gio_unix::DesktopAppInfo> {
    let id = id.trim_end_matches(".desktop");
    if let Some(app) = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop")) {
        return Some(app);
    }
    // Recherche plus large : identifiant en minuscules ou StartupWMClass.
    let lower = id.to_lowercase();
    gio::AppInfo::all().into_iter().filter_map(|a| a.downcast::<gio_unix::DesktopAppInfo>().ok()).find(|a| {
        let app_id = a.id().map(|s| s.to_lowercase()).unwrap_or_default();
        app_id.trim_end_matches(".desktop") == lower
            || a.startup_wm_class().is_some_and(|c| c.to_lowercase() == lower)
    })
}

pub fn display_name(id: &str) -> String {
    if let Some(app) = find(id) {
        return app.display_name().to_string();
    }
    BUILTINS
        .iter()
        .find(|b| b.0 == id)
        .map(|b| b.1.to_string())
        .unwrap_or_else(|| id.to_string())
}

pub fn icon(id: &str) -> gio::Icon {
    if let Some(icon) = find(id).and_then(|a| a.icon()) {
        return icon;
    }
    if let Some(b) = BUILTINS.iter().find(|b| b.0 == id) {
        return gio::ThemedIcon::new(b.2).upcast();
    }
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
    for name in [id.to_string(), id.to_lowercase(), id.rsplit('.').next().unwrap_or(id).to_lowercase()] {
        if theme.has_icon(&name) {
            return gio::ThemedIcon::new(&name).upcast();
        }
    }
    gio::ThemedIcon::new("application-x-executable").upcast()
}

/// Nom / icône / commande d'une appli NOOS intégrée.
pub fn builtin(id: &str) -> Option<(&'static str, &'static str)> {
    BUILTINS.iter().find(|b| b.0 == id).map(|b| (b.1, b.2))
}

pub fn exists(id: &str) -> bool {
    find(id).is_some() || BUILTINS.iter().any(|b| b.0 == id)
}

/// Une fenêtre (app_id Wayland) appartient-elle à cette appli épinglée ?
pub fn matches(pinned: &str, app_id: &str) -> bool {
    if pinned.eq_ignore_ascii_case(app_id) {
        return true;
    }
    find(pinned).is_some_and(|a| {
        a.startup_wm_class().is_some_and(|c| c.eq_ignore_ascii_case(app_id))
            || a.id().is_some_and(|i| i.trim_end_matches(".desktop").eq_ignore_ascii_case(app_id))
            || a.executable().file_name().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(app_id))
    })
}

pub fn launch_info(app: &impl IsA<gio::AppInfo>) {
    eprintln!("noos-shell : lancement de {} ({})", app.as_ref().display_name(), app.as_ref().id().unwrap_or_default());
    if let Some(id) = app.as_ref().id() {
        let id = id.trim_end_matches(".desktop").to_string();
        if RESIDENT.contains(&id.as_str()) && activate_running(&id) {
            return;
        }
    }
    let ctx = gdk::Display::default().map(|d| d.app_launch_context());
    if let Err(e) = app.launch(&[], ctx.as_ref()) {
        eprintln!("noos-shell : lancement impossible : {e}");
    }
}

/// Applications NOOS qui restent en mémoire (une instance unique) : si elles tournent déjà,
/// on les active par D-Bus (ouverture instantanée), sinon on lance le programme.
const RESIDENT: &[&str] = &["org.noos.Settings", "org.noos.Terminal"];

fn activate_running(app_id: &str) -> bool {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return false };
    let owned = conn
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(app_id,).to_variant()),
            Some(glib::VariantTy::new("(b)").unwrap()),
            gio::DBusCallFlags::NONE,
            500,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|v| v.get::<(bool,)>())
        .is_some_and(|(b,)| b);
    if !owned {
        return false;
    }
    let path = format!("/{}", app_id.replace('.', "/"));
    let platform: std::collections::HashMap<String, glib::Variant> = std::collections::HashMap::new();
    conn.call_sync(
        Some(app_id),
        &path,
        "org.freedesktop.Application",
        "Activate",
        Some(&(platform,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .is_ok()
}

pub fn launch(id: &str) {
    eprintln!("noos-shell : lancement de {id}");
    if RESIDENT.contains(&id) && activate_running(id) {
        return;
    }
    if let Some(app) = find(id) {
        launch_info(&app);
    } else if let Some(b) = BUILTINS.iter().find(|b| b.0 == id) {
        spawn(b.3);
    } else {
        spawn(id);
    }
}

/// Lance une commande via le compositeur (environnement de session garanti).
pub fn spawn(command: &str) {
    request(Request::Spawn { command: command.to_string() });
}

/// Envoie une requête IPC sans bloquer l'interface.
pub fn request(req: Request) {
    std::thread::spawn(move || {
        if let Err(e) = IpcClient::fire(&req) {
            eprintln!("noos-shell : IPC {req:?} : {e}");
        }
    });
}

/// Toutes les applications visibles, triées par nom.
pub fn all() -> Vec<gio::AppInfo> {
    let mut apps: Vec<gio::AppInfo> = gio::AppInfo::all().into_iter().filter(|a| a.should_show()).collect();
    apps.sort_by_key(|a| a.display_name().to_lowercase());
    apps
}

/// Recherche simple : nom, description, mots-clés.
pub fn search(query: &str) -> Vec<gio::AppInfo> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all();
    }
    let mut scored: Vec<(u8, gio::AppInfo)> = all()
        .into_iter()
        .filter_map(|a| {
            let name = a.display_name().to_lowercase();
            let score = if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else {
                let desc = a.description().map(|d| d.to_lowercase()).unwrap_or_default();
                let keywords = a
                    .downcast_ref::<gio_unix::DesktopAppInfo>()
                    .map(|d| d.keywords().iter().map(|k| k.to_lowercase()).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                if desc.contains(&q) || keywords.contains(&q) { 2 } else { return None }
            };
            Some((score, a))
        })
        .collect();
    scored.sort_by_key(|(s, a)| (*s, a.display_name().to_lowercase()));
    scored.into_iter().map(|(_, a)| a).collect()
}

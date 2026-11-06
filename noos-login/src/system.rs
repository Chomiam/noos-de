//! Comptes, sessions disponibles, mémoire du dernier choix et alimentation.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

// ───────────────────────────── Comptes ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub name: String,
    pub full_name: String,
    pub home: PathBuf,
    pub avatar: Option<PathBuf>,
}

impl User {
    pub fn display_name(&self) -> &str {
        if self.full_name.is_empty() { &self.name } else { &self.full_name }
    }
}

/// Comptes « humains » de `/etc/passwd` (UID 1000 – 59999, shell de connexion valide).
pub fn users() -> Vec<User> {
    let text = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    parse_passwd(&text)
}

pub fn parse_passwd(text: &str) -> Vec<User> {
    let mut users: Vec<User> = text
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let uid: u32 = f[2].parse().ok()?;
            let shell = f[6];
            if !(1000..60000).contains(&uid) || shell.ends_with("nologin") || shell.ends_with("false") {
                return None;
            }
            let name = f[0].to_string();
            let avatar = ["/var/lib/AccountsService/icons"]
                .iter()
                .map(|d| Path::new(d).join(&name))
                .find(|p| p.is_file());
            Some(User {
                full_name: f[4].split(',').next().unwrap_or("").to_string(),
                home: PathBuf::from(f[5]),
                avatar,
                name,
            })
        })
        .collect();
    users.sort_by(|a, b| a.display_name().to_lowercase().cmp(&b.display_name().to_lowercase()));
    users
}

// ───────────────────────────── Sessions ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// Nom du fichier .desktop sans extension (ex. `noos`, `gnome`, `plasma`).
    pub id: String,
    pub name: String,
    pub comment: String,
    pub exec: String,
    pub desktop_names: String,
    pub wayland: bool,
}

impl Session {
    /// Commande et environnement pour `start_session`.
    pub fn command(&self) -> (Vec<String>, Vec<String>) {
        let kind = if self.wayland { "wayland" } else { "x11" };
        let mut env = vec![
            format!("XDG_SESSION_TYPE={kind}"),
            format!("XDG_SESSION_DESKTOP={}", self.id),
        ];
        if !self.desktop_names.is_empty() {
            env.push(format!("XDG_CURRENT_DESKTOP={}", self.desktop_names.replace(';', ":").trim_end_matches(':')));
        }
        // greetd assemble la commande en une seule ligne et la lance lui-même avec
        // `sh -c "… . /etc/profile; exec <commande>"` : on lui passe la ligne Exec telle
        // quelle (surtout pas un `sh -c` imbriqué, qui ne garderait que « exec »).
        let line = if self.wayland { self.exec.clone() } else { format!("startx /usr/bin/env {}", self.exec) };
        (vec![line], env)
    }

    /// Nom d'icône pour la carte de session.
    pub fn icon(&self) -> &'static str {
        let id = self.id.to_lowercase();
        if id.contains("noos") {
            "computer-symbolic"
        } else if id.contains("gnome") {
            "view-app-grid-symbolic"
        } else if id.contains("plasma") || id.contains("kde") {
            "applications-graphics-symbolic"
        } else if !self.wayland {
            "video-display-symbolic"
        } else {
            "preferences-desktop-display-symbolic"
        }
    }
}

const WAYLAND_DIRS: &[&str] = &["/usr/share/wayland-sessions", "/usr/local/share/wayland-sessions"];
const X11_DIRS: &[&str] = &["/usr/share/xsessions", "/usr/local/share/xsessions"];

pub fn sessions() -> Vec<Session> {
    let mut out: BTreeMap<String, Session> = BTreeMap::new();
    let startx = Path::new("/usr/bin/startx").exists();
    // NOOS_LOGIN_SESSION_DIRS : dossiers supplémentaires de sessions Wayland (tests).
    let extra: Vec<String> = std::env::var("NOOS_LOGIN_SESSION_DIRS")
        .map(|v| v.split(':').filter(|d| !d.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    let wayland_dirs: Vec<&str> = WAYLAND_DIRS.iter().copied().chain(extra.iter().map(|s| s.as_str())).collect();
    for (dirs, wayland) in [(wayland_dirs.as_slice(), true), (X11_DIRS, false)] {
        if !wayland && !startx {
            continue;
        }
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) != Some("desktop") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                let id = path.file_stem().unwrap().to_string_lossy().to_string();
                if let Some(s) = parse_desktop(&id, &text, wayland) {
                    // Une session Wayland l'emporte sur son homonyme X11.
                    out.entry(s.name.clone()).or_insert(s);
                }
            }
        }
    }
    let mut v: Vec<Session> = out.into_values().collect();
    // NOOS en premier, puis ordre alphabétique.
    v.sort_by_key(|s| (!s.id.contains("noos"), !s.wayland, s.name.to_lowercase()));
    v
}

pub fn parse_desktop(id: &str, text: &str, wayland: bool) -> Option<Session> {
    let mut in_entry = false;
    let mut f: BTreeMap<String, String> = BTreeMap::new();
    let lang = std::env::var("LANG").unwrap_or_default();
    let short = lang.split(['_', '.']).next().unwrap_or("").to_string();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            f.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if f.get("Hidden").is_some_and(|v| v == "true") || f.get("NoDisplay").is_some_and(|v| v == "true") {
        return None;
    }
    if let Some(try_exec) = f.get("TryExec") {
        if !exists_in_path(try_exec) {
            return None;
        }
    }
    let localized = |key: &str| {
        f.get(&format!("{key}[{short}]")).or_else(|| f.get(key)).cloned().unwrap_or_default()
    };
    let exec = f.get("Exec")?.clone();
    Some(Session {
        id: id.to_string(),
        name: localized("Name"),
        comment: localized("Comment"),
        exec,
        desktop_names: f.get("DesktopNames").cloned().unwrap_or_default(),
        wayland,
    })
}

fn exists_in_path(bin: &str) -> bool {
    if bin.starts_with('/') {
        return Path::new(bin).exists();
    }
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(true)
}

// ───────────────────────── Mémoire du greeter ─────────────────────────

/// Dernier compte, dernière session par compte et réglages d'accessibilité.
/// `/var/lib/noos-login/state.toml` (dossier de l'utilisateur `greeter`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub last_user: String,
    pub sessions: BTreeMap<String, String>,
    pub large_text: bool,
    pub high_contrast: bool,
    pub osk: bool,
}

fn state_path() -> PathBuf {
    std::env::var_os("NOOS_LOGIN_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/noos-login/state.toml"))
}

impl State {
    pub fn load() -> Self {
        std::fs::read_to_string(state_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = state_path();
        if let Ok(text) = toml::to_string_pretty(self) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

// ───────────────────────────── Divers ─────────────────────────────

/// Disposition clavier du système (`/etc/vconsole.conf`, écrit par l'installateur).
pub fn keyboard_layout() -> String {
    std::fs::read_to_string("/etc/vconsole.conf")
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("XKBLAYOUT=").map(|v| v.trim_matches('"').split(',').next().unwrap_or("").to_string()))
        })
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "us".into())
}

pub fn power(action: &str) {
    let _ = std::process::Command::new("systemctl").arg(action).spawn();
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_and_desktop() {
        let p = "root:x:0:0:root:/root:/bin/bash\naxel:x:1000:1000:Axel Valens,,,:/home/axel:/bin/bash\nsvc:x:1001:1001::/:/sbin/nologin\n";
        let u = parse_passwd(p);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].display_name(), "Axel Valens");
        let d = "[Desktop Entry]\nName=NOOS\nComment=Bureau\nExec=/usr/bin/noos-session\nDesktopNames=NOOS\n[Other]\nName=x\n";
        let s = parse_desktop("noos", d, true).unwrap();
        assert_eq!(s.name, "NOOS");
        let (cmd, env) = s.command();
        assert_eq!(cmd, vec!["/usr/bin/noos-session".to_string()]);
        assert!(env.contains(&"XDG_CURRENT_DESKTOP=NOOS".to_string()));
    }
}

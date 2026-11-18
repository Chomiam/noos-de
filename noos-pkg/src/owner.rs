//! À quel paquet appartient une application ? Sert au menu Démarrer et à la barre des tâches
//! (« Désinstaller », « Fiche dans Logiciels ») : lanceur `.desktop` → paquet apt
//! (`dpkg-query -S`), application Flatpak (clé `X-Flatpak`) ou AppImage (`X-NOOS-AppImage`).

use std::path::{Path, PathBuf};

use crate::{BackendKind, exec::capture_codes};

#[derive(Debug, Clone, PartialEq)]
pub enum Owner {
    Package(BackendKind, String),
    AppImage(PathBuf),
}

impl Owner {
    /// Désignation comprise par `noos-software --details` (`apt:gimp`, `flatpak:org.gimp.GIMP`).
    pub fn spec(&self) -> Option<String> {
        match self {
            Owner::Package(k, id) => Some(format!("{}:{id}", k.id())),
            Owner::AppImage(_) => None,
        }
    }
}

/// Dossiers `applications` (XDG_DATA_HOME puis XDG_DATA_DIRS).
fn application_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".local/share"));
    let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    std::iter::once(home).chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from)).map(|d| d.join("applications")).collect()
}

/// Fichier d'un lanceur (`gimp.desktop` ou `gimp`).
pub fn desktop_file(desktop_id: &str) -> Option<PathBuf> {
    let name = format!("{}.desktop", desktop_id.trim_end_matches(".desktop"));
    application_dirs().into_iter().map(|d| d.join(&name)).find(|p| p.is_file())
}

/// Valeur d'une clé de la section `[Desktop Entry]`.
pub fn desktop_key(text: &str, key: &str) -> Option<String> {
    let mut in_main = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_main = t == "[Desktop Entry]";
        } else if in_main {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// Sortie de `dpkg-query -S <fichier>` : « gimp: /usr/share/applications/gimp.desktop »
/// (ou « a, b: chemin » si plusieurs paquets le déclarent).
pub fn parse_dpkg_search(out: &str) -> Option<String> {
    let line = out.lines().find(|l| l.contains(": /"))?;
    let pkgs = line.split(": /").next()?;
    let first = pkgs.split(',').next()?.trim();
    // « paquet:amd64 » → « paquet ».
    Some(first.split(':').next().unwrap_or(first).to_string()).filter(|s| !s.is_empty() && !s.starts_with("diversion"))
}

/// Paquet propriétaire d'un lanceur.
pub fn owner_of_desktop(desktop_id: &str) -> Option<Owner> {
    let path = desktop_file(desktop_id)?;
    owner_of_file(&path)
}

pub fn owner_of_file(path: &Path) -> Option<Owner> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if let Some(app) = desktop_key(&text, "X-NOOS-AppImage") {
        return Some(Owner::AppImage(PathBuf::from(app)));
    }
    if let Some(id) = desktop_key(&text, "X-Flatpak") {
        return Some(Owner::Package(BackendKind::Flatpak, id));
    }
    // Lanceurs exportés par Flatpak : lien vers …/flatpak/app/<id>/…
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if real.to_string_lossy().contains("/flatpak/") {
        let id = path.file_stem()?.to_string_lossy().to_string();
        return Some(Owner::Package(BackendKind::Flatpak, id));
    }
    for p in [real.as_path(), path] {
        let out = capture_codes(&["dpkg-query", "-S", &p.to_string_lossy()], &[0, 1]).unwrap_or_default();
        if let Some(pkg) = parse_dpkg_search(&out) {
            return Some(Owner::Package(BackendKind::Apt, pkg));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpkg_search() {
        assert_eq!(parse_dpkg_search("gimp: /usr/share/applications/gimp.desktop\n").as_deref(), Some("gimp"));
        assert_eq!(parse_dpkg_search("libfoo1:amd64, libfoo1:i386: /usr/share/applications/x.desktop").as_deref(), Some("libfoo1"));
        assert_eq!(parse_dpkg_search("dpkg-query: no path found matching pattern /x"), None);
    }

    #[test]
    fn cles_desktop() {
        let t = "[Desktop Entry]\nName=GIMP\nX-Flatpak=org.gimp.GIMP\n[Desktop Action new]\nX-Flatpak=autre\n";
        assert_eq!(desktop_key(t, "X-Flatpak").as_deref(), Some("org.gimp.GIMP"));
        assert_eq!(desktop_key(t, "Exec"), None);
        assert_eq!(Owner::Package(BackendKind::Apt, "gimp".into()).spec().as_deref(), Some("apt:gimp"));
    }
}

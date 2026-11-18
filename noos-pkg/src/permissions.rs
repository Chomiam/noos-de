//! Permissions des applications Flatpak (à la manière de Flatseal) : lecture des métadonnées
//! (`flatpak remote-info --show-metadata` avant l'installation, `flatpak info --show-metadata`
//! et `--show-permissions` ensuite) et modifications par `flatpak override --user`.

use crate::{Plan, Scope, exec::capture_codes};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Filesystem,
    Device,
    Socket,
    Shared,
}

/// Niveau d'attention : 0 anodin, 1 à connaître, 2 accès large.
pub type Level = u8;

#[derive(Debug, Clone, PartialEq)]
pub struct Permission {
    pub category: Category,
    /// Valeur Flatpak (`network`, `x11`, `home`, `xdg-download:ro`…).
    pub value: String,
    pub label: String,
    pub detail: String,
    pub level: Level,
    /// Active après les modifications de l'utilisateur.
    pub enabled: bool,
}

/// Section `[Context]` d'un fichier de métadonnées : clé → valeurs.
pub fn parse_context(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut in_ctx = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_ctx = t == "[Context]";
        } else if in_ctx {
            if let Some((k, v)) = t.split_once('=') {
                out.push((k.trim().to_string(), v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()));
            }
        }
    }
    out
}

fn path_of(v: &str) -> &str {
    v.split(':').next().unwrap_or(v)
}

/// Description lisible d'une permission.
pub fn describe(category: Category, value: &str) -> (String, String, Level) {
    let s = |a: &str, b: &str, l: Level| (a.to_string(), b.to_string(), l);
    match category {
        Category::Shared => match value {
            "network" => s("Réseau et Internet", "Peut se connecter à Internet et au réseau local", 1),
            "ipc" => s("Mémoire partagée", "Nécessaire à l'affichage X11, sans risque particulier", 0),
            o => s(o, "Ressource partagée avec le système", 1),
        },
        Category::Socket => match value {
            "wayland" => s("Affichage (Wayland)", "Fenêtres isolées les unes des autres", 0),
            "fallback-x11" => s("Affichage X11 en secours", "Seulement si Wayland n'est pas disponible", 0),
            "x11" => s("Affichage X11", "Peut voir les autres fenêtres X11 et lire leur clavier", 2),
            "pulseaudio" => s("Son et microphone", "Peut jouer du son et enregistrer le micro", 1),
            "system-bus" => s("Bus système complet", "Peut parler à tous les services du système", 2),
            "session-bus" => s("Bus de session complet", "Peut parler à toutes les applications de la session", 2),
            "ssh-auth" => s("Clés SSH", "Peut utiliser vos clés SSH (agent)", 2),
            "cups" => s("Imprimantes", "Peut imprimer directement", 0),
            "pcsc" => s("Cartes à puce", "Lecteurs de cartes (carte d'identité, clés de sécurité)", 1),
            "gpg-agent" => s("Clés GPG", "Peut utiliser vos clés GPG (agent)", 2),
            "inherit-wayland-socket" => s("Affichage (Wayland hérité)", "", 0),
            o => s(o, "Accès à un service du système", 1),
        },
        Category::Device => match value {
            "dri" => s("Accélération graphique", "Utilise la carte graphique (jeux, vidéo)", 0),
            "all" => s("Tous les périphériques", "Webcam, manettes, clés USB, micro…", 2),
            "kvm" => s("Virtualisation (KVM)", "Peut lancer des machines virtuelles", 1),
            "shm" => s("Mémoire partagée (/dev/shm)", "", 0),
            "input" => s("Périphériques d'entrée", "Manettes, volants, tablettes graphiques", 1),
            "usb" => s("Périphériques USB", "Accès direct aux périphériques USB", 2),
            o => s(o, "Accès à un périphérique", 1),
        },
        Category::Filesystem => {
            let ro = value.ends_with(":ro");
            let path = path_of(value);
            let detail = if path.starts_with("xdg-run/") {
                "Communication avec un service de la session"
            } else if ro {
                "Lecture seule"
            } else {
                "Lecture et écriture"
            };
            let (label, level) = match path {
                "host" => ("Tous vos fichiers et ceux du système".to_string(), 2),
                "host-os" => ("Fichiers du système d'exploitation".to_string(), 1),
                "host-etc" => ("Configuration du système (/etc)".to_string(), 1),
                "home" | "~" => ("Tout votre dossier personnel".to_string(), 2),
                "xdg-download" => ("Dossier Téléchargements".to_string(), 0),
                "xdg-documents" => ("Dossier Documents".to_string(), 1),
                "xdg-pictures" => ("Dossier Images".to_string(), 1),
                "xdg-music" => ("Dossier Musique".to_string(), 1),
                "xdg-videos" => ("Dossier Vidéos".to_string(), 1),
                "xdg-desktop" => ("Bureau".to_string(), 1),
                p if p.starts_with("xdg-run/") => (format!("Service de session « {} »", &p[8..]), 0),
                p if p.starts_with("xdg-config/") => (format!("Réglages « {} »", &p[11..]), 0),
                p if p.starts_with("xdg-data/") || p.starts_with("xdg-cache/") => (format!("Données « {} »", p.split_once('/').map(|x| x.1).unwrap_or(p)), 0),
                p if p.starts_with("xdg-download/") => (format!("Téléchargements/{}", &p[13..]), 0),
                "/tmp" => ("Fichiers temporaires (/tmp)".to_string(), 0),
                p => (format!("Dossier {p}"), if p.starts_with("~/") || p.starts_with('/') { 1 } else { 0 }),
            };
            let level = if ro && level > 0 { level - 1 } else { level };
            (label, detail.into(), level)
        }
    }
}

fn category_of(key: &str) -> Option<Category> {
    Some(match key {
        "shared" => Category::Shared,
        "sockets" => Category::Socket,
        "devices" => Category::Device,
        "filesystems" => Category::Filesystem,
        _ => return None,
    })
}

/// Permissions déclarées (`declared`, métadonnées d'origine) et effectives (`effective`, après
/// les modifications de l'utilisateur ; `None` : identiques).
pub fn list(declared: &str, effective: Option<&str>) -> Vec<Permission> {
    let decl = parse_context(declared);
    let eff = effective.map(parse_context).unwrap_or_else(|| decl.clone());
    let values = |ctx: &[(String, Vec<String>)], key: &str| -> Vec<String> {
        ctx.iter().filter(|(k, _)| k == key).flat_map(|(_, v)| v.clone()).filter(|v| !v.starts_with('!')).collect()
    };
    let mut out: Vec<Permission> = Vec::new();
    for key in ["filesystems", "devices", "sockets", "shared"] {
        let cat = category_of(key).unwrap();
        let (d, e) = (values(&decl, key), values(&eff, key));
        let mut all = d.clone();
        all.extend(e.iter().filter(|v| !d.iter().any(|x| path_of(x) == path_of(v))).cloned());
        for v in all {
            let enabled = e.iter().any(|x| path_of(x) == path_of(&v));
            let shown = e.iter().find(|x| path_of(x) == path_of(&v)).cloned().unwrap_or(v);
            let (label, detail, level) = describe(cat, &shown);
            out.push(Permission { category: cat, value: shown, label, detail, level, enabled });
        }
    }
    out.sort_by_key(|p| (std::cmp::Reverse(p.level), p.category));
    out
}

/// Option de `flatpak override` pour activer / retirer une permission.
pub fn override_flag(p: &Permission, enable: bool) -> String {
    let v = if p.category == Category::Filesystem && !enable { path_of(&p.value).to_string() } else { p.value.clone() };
    let name = match (p.category, enable) {
        (Category::Shared, true) => "share",
        (Category::Shared, false) => "unshare",
        (Category::Socket, true) => "socket",
        (Category::Socket, false) => "nosocket",
        (Category::Device, true) => "device",
        (Category::Device, false) => "nodevice",
        (Category::Filesystem, true) => "filesystem",
        (Category::Filesystem, false) => "nofilesystem",
    };
    format!("--{name}={v}")
}

pub fn override_plan(app_id: &str, p: &Permission, enable: bool) -> Plan {
    Plan::new().user_owned(vec!["flatpak".into(), "override".into(), "--user".into(), override_flag(p, enable), app_id.into()])
}

pub fn reset_plan(app_id: &str) -> Plan {
    Plan::new().user(&["flatpak", "override", "--user", "--reset", app_id])
}

/// Permissions d'une appli : installée (`flatpak info`) ou dans un dépôt (`remote-info`).
pub fn of_app(app_id: &str, installed: bool, scope: Scope, remote: &str) -> anyhow::Result<Vec<Permission>> {
    if installed {
        let declared = capture_codes(&["flatpak", "info", scope.flag(), "--show-metadata", app_id], &[0])?;
        let effective = capture_codes(&["flatpak", "info", scope.flag(), "--show-permissions", app_id], &[0]).ok();
        Ok(list(&declared, effective.as_deref()))
    } else {
        let declared = capture_codes(&["flatpak", "remote-info", scope.flag(), "--show-metadata", remote, app_id], &[0])?;
        Ok(list(&declared, None))
    }
}

/// L'utilisateur a-t-il modifié les permissions de l'appli ?
pub fn has_overrides(app_id: &str) -> bool {
    capture_codes(&["flatpak", "override", "--user", "--show", app_id], &[0]).is_ok_and(|o| o.lines().any(|l| l.contains('=')))
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = "[Application]\nname=org.gimp.GIMP\n\n[Context]\nshared=ipc;network;\nsockets=fallback-x11;wayland;\ndevices=all;\nfilesystems=xdg-config/GIMP:create;/tmp;host;xdg-download:ro;\n\n[Session Bus Policy]\norg.gtk.vfs.*=talk\n";

    #[test]
    fn lecture_et_niveaux() {
        let p = list(META, None);
        assert_eq!(p.len(), 9);
        // Les accès larges d'abord.
        assert_eq!(p[0].label, "Tous vos fichiers et ceux du système");
        assert_eq!(p[0].level, 2);
        assert!(p.iter().all(|x| x.enabled));
        let dl = p.iter().find(|x| x.value == "xdg-download:ro").unwrap();
        assert_eq!((dl.label.as_str(), dl.detail.as_str()), ("Dossier Téléchargements", "Lecture seule"));
        assert!(p.iter().any(|x| x.label == "Réglages « GIMP »"));
    }

    #[test]
    fn modifications() {
        let eff = "[Context]\nshared=ipc;\nsockets=fallback-x11;wayland;\ndevices=all;\nfilesystems=xdg-config/GIMP:create;/tmp;xdg-download:ro;xdg-music;\n";
        let p = list(META, Some(eff));
        assert!(!p.iter().find(|x| x.value == "network").unwrap().enabled);
        assert!(!p.iter().find(|x| x.value == "host").unwrap().enabled);
        // Ajoutée par l'utilisateur.
        assert!(p.iter().find(|x| x.value == "xdg-music").unwrap().enabled);
        let host = p.iter().find(|x| x.value == "host").unwrap();
        assert_eq!(override_flag(host, false), "--nofilesystem=host");
        let net = p.iter().find(|x| x.value == "network").unwrap();
        assert_eq!(override_flag(net, true), "--share=network");
        let dl = p.iter().find(|x| x.value == "xdg-download:ro").unwrap();
        assert_eq!(override_flag(dl, false), "--nofilesystem=xdg-download");
        assert!(override_plan("org.gimp.GIMP", net, false).describe().join(" ").contains("flatpak override --user --unshare=network org.gimp.GIMP"));
    }
}

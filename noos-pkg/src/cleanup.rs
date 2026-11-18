//! Nettoyage : ce qui prend de la place sans servir, mesuré sans droits root, retiré en une
//! seule élévation.
//!
//! | Élément                      | Mesure                                   | Action                          |
//! |------------------------------|------------------------------------------|---------------------------------|
//! | dépendances devenues inutiles | `apt-get -s autoremove`                  | `apt-get autoremove` (pkexec)   |
//! | paquets téléchargés           | `/var/cache/apt/archives/*.deb`          | `apt-get clean` (pkexec)        |
//! | anciens noyaux                | `dpkg-query` (garde le courant et le plus récent) | `apt-get remove` (pkexec) |
//! | runtimes Flatpak inutilisés   | `flatpak uninstall --unused` (réponse « n ») | `flatpak uninstall --unused`  |
//! | données d'applis Flatpak supprimées | `~/.var/app/<id>` sans appli installée | corbeille (`gio trash`)      |

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{Plan, Scope, backends::apt::apt_root, exec::capture_codes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    AptOrphans,
    AptCache,
    OldKernels,
    FlatpakUnused,
    FlatpakData,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub title: String,
    pub description: String,
    /// Place récupérée (octets, estimation).
    pub size: u64,
    /// Ce qui sera retiré (noms de paquets, dossiers).
    pub entries: Vec<String>,
    pub plan: Plan,
    /// Récupérable ensuite (corbeille) ?
    pub reversible: bool,
}

/// Taille d'un dossier (fichiers réguliers, sans suivre les liens).
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    std::fs::read_dir(path).map(|rd| rd.filter_map(Result::ok).map(|e| dir_size(&e.path())).sum()).unwrap_or(0)
}

/// Taille installée (octets) de paquets, d'après dpkg (`Installed-Size` en Kio).
fn installed_size(pkgs: &[String]) -> u64 {
    if pkgs.is_empty() {
        return 0;
    }
    let mut argv = vec!["dpkg-query", "-W", "-f=${Installed-Size}\n"];
    argv.extend(pkgs.iter().map(String::as_str));
    capture_codes(&argv, &[0, 1]).unwrap_or_default().lines().filter_map(|l| l.trim().parse::<u64>().ok()).sum::<u64>() * 1024
}

/// Paquets que `apt-get -s autoremove` retirerait.
pub fn parse_autoremove(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|r| r.split_whitespace().next())
        .map(|n| n.split(':').next().unwrap_or(n).to_string())
        .collect()
}

/// Tableau de `flatpak uninstall --unused` avant confirmation : « 1.  org.x.Platform  23.08  r ».
pub fn parse_flatpak_unused(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let n = w.next()?;
            (n.ends_with('.') && n.trim_end_matches('.').parse::<u32>().is_ok()).then_some(())?;
            let id = w.next()?;
            let branch = w.next().unwrap_or("");
            Some(if branch.is_empty() { id.to_string() } else { format!("{id}//{branch}") })
        })
        .collect()
}

/// Version d'un paquet noyau (`linux-image-6.17.8+deb14-amd64` → `6.17.8+deb14-amd64`).
fn kernel_version(pkg: &str) -> Option<&str> {
    pkg.strip_prefix("linux-image-").filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// Comparaison de versions de noyau « 6.17.8+deb14-amd64 » (champs numériques d'abord).
fn kernel_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

/// Noyaux à retirer : tous sauf celui qui tourne et le plus récent.
pub fn old_kernels(installed: &[String], running: &str) -> Vec<String> {
    let mut kernels: Vec<&String> = installed.iter().filter(|p| kernel_version(p).is_some()).collect();
    kernels.sort_by_key(|p| kernel_key(kernel_version(p).unwrap_or("")));
    let newest = kernels.last().map(|s| s.to_string());
    kernels
        .into_iter()
        .filter(|p| kernel_version(p) != Some(running) && Some(p.to_string()) != newest)
        .cloned()
        .collect()
}

fn flatpak_unused(scope: Scope) -> Vec<String> {
    let out = std::process::Command::new("flatpak")
        .args(["uninstall", "--unused", scope.flag()])
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            if let Some(mut i) = c.stdin.take() {
                let _ = i.write_all(b"n\n");
            }
            c.wait_with_output()
        });
    out.map(|o| parse_flatpak_unused(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

fn flatpak_root(scope: Scope) -> PathBuf {
    match scope {
        Scope::User => noos_core::paths::home().join(".local/share/flatpak"),
        Scope::System => "/var/lib/flatpak".into(),
    }
}

/// Analyse (lente : quelques secondes). N'inclut que ce qui libère de la place.
pub fn scan() -> Vec<Item> {
    let mut items = Vec::new();
    let have = |b: &str| crate::exec::have(b);

    if have("apt-get") {
        let out = capture_codes(&["apt-get", "-s", "-o", "Debug::NoLocking=1", "autoremove"], &[0]).unwrap_or_default();
        let orphans = parse_autoremove(&out);
        if !orphans.is_empty() {
            items.push(Item {
                kind: Kind::AptOrphans,
                title: "Dépendances devenues inutiles".into(),
                description: "Paquets installés pour un logiciel supprimé depuis".into(),
                size: installed_size(&orphans),
                plan: Plan::new().root_owned(apt_root(&["autoremove"])),
                entries: orphans,
                reversible: false,
            });
        }
        let debs: Vec<PathBuf> = std::fs::read_dir("/var/cache/apt/archives")
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "deb")).collect())
            .unwrap_or_default();
        let size: u64 = debs.iter().filter_map(|p| p.metadata().ok()).map(|m| m.len()).sum();
        if size > 0 {
            items.push(Item {
                kind: Kind::AptCache,
                title: "Paquets déjà installés, gardés en cache".into(),
                description: format!("{} fichiers .deb téléchargés (retéléchargés si besoin)", debs.len()),
                size,
                entries: vec![],
                plan: Plan::new().root(&["apt-get", "clean"]),
                reversible: false,
            });
        }
        let running = capture_codes(&["uname", "-r"], &[0]).unwrap_or_default().trim().to_string();
        let installed: Vec<String> = capture_codes(&["dpkg-query", "-W", "-f=${binary:Package} ${db:Status-Abbrev}\n", "linux-image-*"], &[0, 1])
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(" ii"))
            .filter_map(|l| l.split_whitespace().next().map(|n| n.split(':').next().unwrap_or(n).to_string()))
            .collect();
        let old = old_kernels(&installed, &running);
        if !old.is_empty() {
            // Modules et en-têtes de la même version partent avec eux.
            let versions: Vec<&str> = old.iter().filter_map(|p| kernel_version(p)).collect();
            let all: Vec<String> = capture_codes(&["dpkg-query", "-W", "-f=${binary:Package} ${db:Status-Abbrev}\n"], &[0])
                .unwrap_or_default()
                .lines()
                .filter(|l| l.contains(" ii"))
                .filter_map(|l| l.split_whitespace().next().map(|n| n.split(':').next().unwrap_or(n).to_string()))
                .filter(|n| versions.iter().any(|v| n.ends_with(v)) && (n.starts_with("linux-")))
                .collect();
            let mut argv = apt_root(&["remove"]);
            argv.extend(all.iter().cloned());
            items.push(Item {
                kind: Kind::OldKernels,
                title: "Anciens noyaux Linux".into(),
                description: format!("Le noyau en cours ({running}) et le plus récent sont gardés"),
                size: installed_size(&all) + versions.iter().map(|v| dir_size(&Path::new("/lib/modules").join(v))).sum::<u64>(),
                entries: all,
                plan: Plan::new().root_owned(argv),
                reversible: false,
            });
        }
    }

    if have("flatpak") {
        for scope in [Scope::System, Scope::User] {
            let unused = flatpak_unused(scope);
            if unused.is_empty() {
                continue;
            }
            let root = flatpak_root(scope);
            let size = unused
                .iter()
                .map(|r| {
                    let mut parts = r.split("//");
                    let id = parts.next().unwrap_or("");
                    dir_size(&root.join("runtime").join(id))
                })
                .sum();
            items.push(Item {
                kind: Kind::FlatpakUnused,
                title: format!("Runtimes Flatpak inutilisés ({})", if scope == Scope::User { "votre compte" } else { "système" }),
                description: "Bibliothèques partagées dont plus aucune application n'a besoin".into(),
                size,
                entries: unused,
                plan: Plan::new().user(&["flatpak", "uninstall", "--unused", "--noninteractive", "-y", scope.flag()]),
                reversible: false,
            });
        }
        let installed: HashSet<String> = capture_codes(&["flatpak", "list", "--app", "--columns=application"], &[0])
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect();
        let var = noos_core::paths::home().join(".var/app");
        let leftovers: Vec<PathBuf> = std::fs::read_dir(&var)
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<PathBuf>>())
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| !installed.contains(&*n.to_string_lossy())))
            .collect();
        if !leftovers.is_empty() {
            let mut plan = Plan::new();
            for p in &leftovers {
                plan = plan.user_owned(vec!["gio".into(), "trash".into(), p.to_string_lossy().into_owned()]);
            }
            items.push(Item {
                kind: Kind::FlatpakData,
                title: "Données d'applications Flatpak supprimées".into(),
                description: "Réglages et fichiers internes restés dans ~/.var/app (mis à la corbeille)".into(),
                size: leftovers.iter().map(|p| dir_size(p)).sum(),
                entries: leftovers.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect(),
                plan,
                reversible: true,
            });
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoremove() {
        let out = "NOTE: This is only a simulation!\nRemv libfoo1 [1.0-1]\nRemv libbar2:i386 [2.0]\nInst x\n";
        assert_eq!(parse_autoremove(out), ["libfoo1", "libbar2"]);
    }

    #[test]
    fn flatpak_inutilises() {
        let out = "\n        ID                                      Branch    Op\n 1.     org.freedesktop.Platform.GL.default     23.08     r\n 2.     org.gnome.Platform                      45        r\n\nProceed with these changes to the system installation? [Y/n]: n\n";
        assert_eq!(parse_flatpak_unused(out), ["org.freedesktop.Platform.GL.default//23.08", "org.gnome.Platform//45"]);
        assert!(parse_flatpak_unused("Nothing unused to uninstall\n").is_empty());
    }

    #[test]
    fn noyaux() {
        let installed: Vec<String> =
            ["linux-image-amd64", "linux-image-6.16.3+deb14-amd64", "linux-image-6.17.10+deb14-amd64", "linux-image-6.17.9+deb14-amd64"].iter().map(|s| s.to_string()).collect();
        // En cours : 6.17.9 ; le plus récent : 6.17.10 ; à retirer : 6.16.3 (jamais le métapaquet).
        assert_eq!(old_kernels(&installed, "6.17.9+deb14-amd64"), ["linux-image-6.16.3+deb14-amd64"]);
        assert!(old_kernels(&installed[..2], "6.16.3+deb14-amd64").is_empty());
    }
}

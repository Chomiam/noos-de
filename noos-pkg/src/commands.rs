//! « commande introuvable » : quel paquet fournit une commande ? Index construit depuis le champ
//! `Commands` des listes apt (`/var/lib/apt/lists/*_Packages`), gardé dans
//! `~/.cache/noos/commands.tsv` tant que les listes ne changent pas, complété par les paquets
//! qui portent le nom de la commande. Utilisé par les gestionnaires de fish et bash
//! (`noos-pkg provides --hint <commande>`).

use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};

use crate::exec::capture_codes;

/// Couples (commande, paquet) d'un fichier `Packages`.
pub fn parse_packages(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pkg = String::new();
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Package: ") {
            pkg = p.trim().to_string();
        } else if let Some(c) = line.strip_prefix("Commands: ") {
            out.extend(c.split(',').map(|c| c.trim()).filter(|c| !c.is_empty()).map(|c| (c.to_string(), pkg.clone())));
        }
    }
    out
}

fn lists() -> Vec<PathBuf> {
    let arch = crate::backends::apt::native_arch();
    let mut v: Vec<PathBuf> = std::fs::read_dir("/var/lib/apt/lists")
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.to_string_lossy().contains(&format!("binary-{arch}_Packages"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn cache() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".cache"));
    base.join("noos/commands.tsv")
}

fn newest(paths: &[PathBuf]) -> Option<std::time::SystemTime> {
    paths.iter().filter_map(|p| p.metadata().ok()?.modified().ok()).max()
}

fn read_list(p: &Path) -> String {
    let s = p.to_string_lossy();
    if s.ends_with(".lz4") {
        capture_codes(&["lz4", "-dc", &s], &[0]).unwrap_or_default()
    } else if s.ends_with(".gz") {
        capture_codes(&["gzip", "-dc", &s], &[0]).unwrap_or_default()
    } else {
        std::fs::read_to_string(p).unwrap_or_default()
    }
}

/// Index commande → paquets (reconstruit si les listes apt sont plus récentes).
pub fn index() -> BTreeMap<String, Vec<String>> {
    let files = lists();
    let file = cache();
    let stale = match (file.metadata().and_then(|m| m.modified()).ok(), newest(&files)) {
        (Some(c), Some(l)) => l > c,
        (None, _) => true,
        _ => false,
    };
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if stale {
        for f in &files {
            for (c, p) in parse_packages(&read_list(f)) {
                let e = map.entry(c).or_default();
                if !e.contains(&p) {
                    e.push(p);
                }
            }
        }
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::File::create(&file) {
            for (c, ps) in &map {
                let _ = writeln!(f, "{c}\t{}", ps.join(","));
            }
        }
    } else {
        for l in std::fs::read_to_string(&file).unwrap_or_default().lines() {
            if let Some((c, ps)) = l.split_once('\t') {
                map.insert(c.into(), ps.split(',').map(String::from).collect());
            }
        }
    }
    map
}

/// Paquets qui fournissent `command` (index, puis paquet du même nom s'il existe).
pub fn providers(command: &str) -> Vec<String> {
    if command.is_empty() || command.contains('/') {
        return vec![];
    }
    let mut out = index().get(command).cloned().unwrap_or_default();
    if !out.iter().any(|p| p == command) {
        let has = capture_codes(&["apt-cache", "policy", command], &[0]).is_ok_and(|o| o.contains("Candidate:") && !o.contains("Candidate: (none)"));
        if has {
            out.push(command.to_string());
        }
    }
    // Le paquet du même nom d'abord (vlc avant vlc-bin).
    out.sort_by_key(|p| p != command);
    out
}

/// Message du terminal (français).
pub fn hint(command: &str, providers: &[String]) -> String {
    match providers {
        [] => format!("« {command} » : commande introuvable."),
        [one] => format!(
            "« {command} » : commande introuvable. Elle est fournie par le paquet « {one} ».\n  Installer : sudo apt install {one}   (ou dans Logiciels : noos-software --details apt:{one})"
        ),
        many => format!(
            "« {command} » : commande introuvable. Elle est fournie par l'un de ces paquets :\n{}\n  Installer : sudo apt install <paquet>",
            many.iter().map(|p| format!("  • {p}")).collect::<Vec<_>>().join("\n")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_et_message() {
        let t = "Package: 7zip\nVersion: 1\nCommands: 7z, 7za, 7zr, p7zip\n\nPackage: gimp\nCommands: gimp, gimp-3\n\nPackage: p7zip-full\nCommands: 7z\n";
        let v = parse_packages(t);
        assert_eq!(v.len(), 7);
        assert!(v.contains(&("7z".into(), "p7zip-full".into())));
        assert!(hint("gimp", &["gimp".into()]).contains("sudo apt install gimp"));
        assert!(hint("7z", &["7zip".into(), "p7zip-full".into()]).contains("• p7zip-full"));
        assert_eq!(providers("/bin/x"), Vec::<String>::new());
    }
}

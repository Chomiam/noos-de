//! Packs « Sélection NOOS » : plusieurs logiciels installés en un clic (Bureautique, Création,
//! Jeux…). La liste vient de `noos-pkg/data/packs.json` sur GitHub (mise à jour sans nouvelle
//! version du paquet, cache d'un jour dans `~/.cache/noos-software/packs.json`), avec la copie
//! intégrée au programme en repli.

use std::{path::PathBuf, time::Duration};

use serde::Deserialize;

use crate::{BackendKind, Package};

pub const URL: &str = "https://raw.githubusercontent.com/Chomiam/noos-de/main/noos-pkg/data/packs.json";
const BUILTIN: &str = include_str!("../data/packs.json");

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Item {
    pub backend: String,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub icon: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Deserialize)]
struct File {
    packs: Vec<Pack>,
}

pub fn parse(json: &str) -> anyhow::Result<Vec<Pack>> {
    Ok(serde_json::from_str::<File>(json)?.packs)
}

pub fn builtin() -> Vec<Pack> {
    parse(BUILTIN).unwrap_or_default()
}

fn cache_file() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".cache"));
    base.join("noos-software/packs.json")
}

/// Packs à jour (GitHub, cache d'un jour), sinon la copie intégrée.
pub fn load() -> Vec<Pack> {
    let file = cache_file();
    let fresh = file.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(24 * 3600)));
    if !fresh {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("part");
        let ok = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "15", "-o"])
            .arg(&tmp)
            .arg(URL)
            .status()
            .is_ok_and(|s| s.success());
        if ok && std::fs::read_to_string(&tmp).ok().is_some_and(|t| parse(&t).is_ok()) {
            let _ = std::fs::rename(&tmp, &file);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    std::fs::read_to_string(&file).ok().and_then(|t| parse(&t).ok()).filter(|p| !p.is_empty()).unwrap_or_else(builtin)
}

impl Item {
    pub fn package(&self) -> Option<Package> {
        let backend = BackendKind::parse(&self.backend)?;
        Some(Package {
            backend: Some(backend),
            id: self.id.clone(),
            name: self.name.clone(),
            origin: if backend == BackendKind::Flatpak { "flathub".into() } else { String::new() },
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_integres() {
        let p = builtin();
        assert!(p.len() >= 5);
        let jeux = p.iter().find(|p| p.id == "jeux").unwrap();
        assert!(jeux.items.iter().any(|i| i.id == "steam-installer"));
        assert!(p.iter().flat_map(|p| &p.items).all(|i| i.package().is_some()), "backend inconnu dans packs.json");
    }
}

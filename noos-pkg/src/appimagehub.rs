//! AppImageHub (appimage.github.io, catalogue public des AppImage) : sert à la comparaison
//! apt / Flatpak / AppImage de la fiche. Le flux `feed.json` (~3 Mo) est gardé un jour dans
//! `~/.cache/noos-software/appimagehub.json`.

use std::{path::PathBuf, time::Duration};

use serde::Deserialize;

pub const FEED: &str = "https://appimage.github.io/feed.json";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Page de l'appli sur AppImageHub.
    pub page: String,
    /// Page de téléchargement (versions GitHub, site de l'éditeur), si connue.
    pub download: Option<String>,
    pub license: String,
}

#[derive(Deserialize)]
struct Feed {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    name: String,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    links: Option<Vec<Link>>,
}

#[derive(Deserialize)]
struct Link {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    url: String,
}

/// « GNU Image Manipulation Program » / « gimp » → forme comparable (minuscules, lettres et chiffres).
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Cherche une appli par l'un de ses noms dans le flux.
pub fn find_in(feed: &str, names: &[&str]) -> Option<Entry> {
    let feed: Feed = serde_json::from_str(feed).ok()?;
    let wanted: Vec<String> = names.iter().map(|n| norm(n)).filter(|n| n.len() >= 2).collect();
    let item = feed.items.into_iter().find(|i| wanted.contains(&norm(&i.name)))?;
    let download = item.links.unwrap_or_default().into_iter().find_map(|l| match l.kind.as_str() {
        "GitHub" if !l.url.is_empty() => Some(format!("https://github.com/{}/releases", l.url.trim_matches('/'))),
        "Download" if l.url.starts_with("http") => Some(l.url),
        _ => None,
    });
    Some(Entry {
        page: format!("https://appimage.github.io/{}/", item.name.replace(' ', "_")),
        name: item.name,
        download,
        license: item.license.unwrap_or_default(),
    })
}

fn cache_file() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".cache"));
    base.join("noos-software/appimagehub.json")
}

/// Cherche dans le flux (téléchargé au plus une fois par jour).
pub fn find(names: &[&str]) -> Option<Entry> {
    let file = cache_file();
    let fresh = file.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(24 * 3600)));
    if !fresh {
        let _ = std::fs::create_dir_all(file.parent()?);
        let tmp = file.with_extension("part");
        let ok = std::process::Command::new("curl").args(["-fsSL", "--max-time", "30", "-o"]).arg(&tmp).arg(FEED).status().is_ok_and(|s| s.success());
        if ok {
            let _ = std::fs::rename(&tmp, &file);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    find_in(&std::fs::read_to_string(&file).ok()?, names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recherche() {
        let feed = r#"{"items":[{"name":"GIMP","license":"GPL-3.0+","links":null},{"name":"Kdenlive","links":[{"type":"GitHub","url":"KDE/kdenlive"},{"type":"Download","url":"https://kdenlive.org/download"}]},{"name":"Bad"}]}"#;
        let g = find_in(feed, &["Éditeur d’image GIMP", "gimp"]).unwrap();
        assert_eq!((g.page.as_str(), g.download.as_deref(), g.license.as_str()), ("https://appimage.github.io/GIMP/", None, "GPL-3.0+"));
        let k = find_in(feed, &["kdenlive"]).unwrap();
        assert_eq!(k.download.as_deref(), Some("https://github.com/KDE/kdenlive/releases"));
        assert!(find_in(feed, &["inkscape"]).is_none());
        assert_eq!(norm("Éditeur d’image"), "éditeurdimage");
    }
}

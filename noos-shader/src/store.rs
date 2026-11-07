//! Bibliothèque locale : `~/.local/share/noos/shaders/<id>/shader.json` (+ `thumb.jpg`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Saved {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub likes: u64,
    pub views: u64,
    /// Code de la passe « Image » et de l'onglet « Common ».
    pub image: String,
    pub common: String,
    /// Date de téléchargement (secondes depuis 1970).
    pub saved_at: u64,
}

impl Saved {
    pub fn url(&self) -> String {
        format!("https://www.shadertoy.com/view/{}", self.id)
    }
}

pub fn dir() -> PathBuf {
    noos_core::paths::xdg_data_home().join("noos").join("shaders")
}

fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 16 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn thumb_path(id: &str) -> PathBuf {
    dir().join(id).join("thumb.jpg")
}

pub fn get(id: &str) -> Option<Saved> {
    if !valid(id) {
        return None;
    }
    let text = std::fs::read_to_string(dir().join(id).join("shader.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Animations téléchargées, les plus récentes d'abord.
pub fn list() -> Vec<Saved> {
    let mut v: Vec<Saved> = std::fs::read_dir(dir())
        .map(|rd| rd.flatten().filter_map(|e| get(&e.file_name().to_string_lossy())).collect())
        .unwrap_or_default();
    v.sort_by_key(|s| std::cmp::Reverse(s.saved_at));
    v
}

pub fn save(s: &Saved) -> anyhow::Result<()> {
    anyhow::ensure!(valid(&s.id), "identifiant invalide");
    let d = dir().join(&s.id);
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join("shader.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

pub fn delete(id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid(id), "identifiant invalide");
    std::fs::remove_dir_all(dir().join(id))?;
    Ok(())
}

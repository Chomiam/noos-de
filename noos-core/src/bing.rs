//! Images du jour de Bing comme fonds d'écran (liste, miniatures, téléchargement UHD).
//!
//! API publique : `https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n=8&mkt=fr-FR`
//! (les 8 derniers jours). Téléchargements avec `curl`, rangés dans
//! `~/.local/share/noos/wallpapers/bing/` ; on garde les 30 plus récents.

use std::{path::PathBuf, process::Command};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BingImage {
    /// AAAAMMJJ
    pub date: String,
    /// Ex. `/th?id=OHR.AlpineBraies_FR-FR8651963330`
    pub urlbase: String,
    pub title: String,
    pub copyright: String,
}

#[derive(Deserialize)]
struct Archive {
    images: Vec<RawImage>,
}

#[derive(Deserialize)]
struct RawImage {
    startdate: String,
    urlbase: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    copyright: String,
}

/// Images en pleine définition : `~/Images/Wallpapers/bing-AAAAMMJJ_….jpg`.
pub fn dir() -> PathBuf {
    paths::wallpapers_dir()
}

fn thumbs_dir() -> PathBuf {
    paths::cache_dir().join("wallpapers/bing")
}

/// Marché Bing d'après la langue du système (fr_FR.UTF-8 → fr-FR).
fn market() -> String {
    let lang = std::env::var("LANG").unwrap_or_default();
    let base = lang.split('.').next().unwrap_or("").replace('_', "-");
    if base.len() == 5 { base } else { "fr-FR".into() }
}

fn curl(url: &str, out: Option<&PathBuf>) -> anyhow::Result<Vec<u8>> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "30", "-A", "NOOS"]);
    if let Some(p) = out {
        cmd.arg("-o").arg(p);
    }
    let output = cmd.arg(url).output().context("curl introuvable")?;
    if !output.status.success() {
        bail!("téléchargement impossible : {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

/// Les `n` (≤ 8) dernières images du jour.
pub fn list(n: u32) -> anyhow::Result<Vec<BingImage>> {
    let url = format!("https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n={}&mkt={}", n.min(8), market());
    let body = curl(&url, None)?;
    let archive: Archive = serde_json::from_slice(&body).context("réponse Bing illisible")?;
    Ok(archive
        .images
        .into_iter()
        .map(|i| BingImage { date: i.startdate, urlbase: i.urlbase, title: i.title, copyright: i.copyright })
        .collect())
}

impl BingImage {
    fn stem(&self) -> String {
        let name = self.urlbase.rsplit('=').next().unwrap_or("bing").replace(['/', '?', '&'], "_");
        format!("bing-{}_{}", self.date, name)
    }

    /// Miniature 400×240 (mise en cache).
    pub fn thumbnail(&self) -> anyhow::Result<PathBuf> {
        let path = thumbs_dir().join(format!("{}.jpg", self.stem()));
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let tmp = path.with_extension("part");
            curl(&format!("https://www.bing.com{}_400x240.jpg", self.urlbase), Some(&tmp))?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(path)
    }

    /// Image en pleine définition (UHD, sinon 1920×1080), dans `~/Images/Wallpapers`.
    pub fn download(&self) -> anyhow::Result<PathBuf> {
        let p = self.download_to(&dir())?;
        prune();
        Ok(p)
    }

    /// Téléchargement pour un aperçu (cache) : rien n'arrive dans `~/Images/Wallpapers`
    /// tant que l'image n'est pas appliquée (voir [`paths::keep_wallpaper`]).
    pub fn download_preview(&self) -> anyhow::Result<PathBuf> {
        let existing = dir().join(format!("{}.jpg", self.stem()));
        if existing.exists() {
            return Ok(existing);
        }
        self.download_to(&paths::preview_dir())
    }

    fn download_to(&self, target: &std::path::Path) -> anyhow::Result<PathBuf> {
        let path = target.join(format!("{}.jpg", self.stem()));
        if path.exists() {
            return Ok(path);
        }
        std::fs::create_dir_all(target)?;
        let tmp = path.with_extension("part");
        if curl(&format!("https://www.bing.com{}_UHD.jpg", self.urlbase), Some(&tmp)).is_err() {
            curl(&format!("https://www.bing.com{}_1920x1080.jpg", self.urlbase), Some(&tmp))?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }
}

/// Garde les 30 images les plus récentes (les noms commencent par la date).
fn prune() {
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    // Seulement nos images Bing : le dossier peut contenir d'autres fonds de l'utilisateur.
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jpg") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("bing-")))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(30);
    for f in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(f);
    }
}

/// Image du jour : téléchargée si besoin. Renvoie (chemin, image).
pub fn today() -> anyhow::Result<(PathBuf, BingImage)> {
    let img = list(1)?.into_iter().next().context("aucune image du jour")?;
    Ok((img.download()?, img))
}

/// Fichier de fond d'écran provenant de Bing ?
pub fn is_bing(path: &str) -> bool {
    let p = std::path::Path::new(path);
    p.starts_with(dir()) && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("bing-"))
}

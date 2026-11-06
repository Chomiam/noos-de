//! Fonds d'écran Wallhaven (wallhaven.cc) : API publique sans clé (45 requêtes / minute),
//! contenu « tout public » et catégorie « général » uniquement, filtré selon l'écran
//! (résolution minimale et format).

use std::{path::PathBuf, process::Command};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::paths;

const API: &str = "https://wallhaven.cc/api/v1/search";

#[derive(Debug, Clone, PartialEq)]
pub struct Wallpaper {
    pub id: String,
    pub resolution: String,
    pub page_url: String,
    pub full_url: String,
    pub thumb_url: String,
}

#[derive(Deserialize)]
struct Response {
    data: Vec<Raw>,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    url: String,
    path: String,
    resolution: String,
    thumbs: Thumbs,
}

#[derive(Deserialize)]
struct Thumbs {
    small: String,
}

/// Images en pleine résolution : `~/Images/Wallpapers/wallhaven-<id>.jpg`.
pub fn dir() -> PathBuf {
    paths::wallpapers_dir()
}

fn thumbs_dir() -> PathBuf {
    paths::cache_dir().join("wallpapers/wallhaven")
}

/// Format le plus proche de l'écran, dans les formats connus de Wallhaven.
pub fn ratio(width: u32, height: u32) -> &'static str {
    const RATIOS: &[(&str, f64)] = &[
        ("16x9", 16.0 / 9.0),
        ("16x10", 1.6),
        ("21x9", 21.0 / 9.0),
        ("32x9", 32.0 / 9.0),
        ("4x3", 4.0 / 3.0),
        ("5x4", 1.25),
        ("1x1", 1.0),
        ("9x16", 9.0 / 16.0),
        ("10x16", 10.0 / 16.0),
    ];
    let r = width as f64 / height.max(1) as f64;
    RATIOS.iter().min_by(|a, b| (a.1 - r).abs().total_cmp(&(b.1 - r).abs())).map(|x| x.0).unwrap_or("16x9")
}

/// Wallhaven indexe ses images avec des mots-clés anglais : traduction des mots courants.
pub fn translate(query: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("montagne", "mountain"), ("montagnes", "mountains"), ("mer", "sea"), ("océan", "ocean"),
        ("forêt", "forest"), ("foret", "forest"), ("ville", "city"), ("nuit", "night"), ("espace", "space"),
        ("plage", "beach"), ("lac", "lake"), ("neige", "snow"), ("désert", "desert"), ("aurore", "aurora"),
        ("voiture", "car"), ("voitures", "cars"), ("abstrait", "abstract"), ("minimaliste", "minimalist"),
        ("chat", "cat"), ("chats", "cats"), ("chien", "dog"), ("fleur", "flower"), ("fleurs", "flowers"),
        ("ciel", "sky"), ("nuages", "clouds"), ("nuage", "clouds"), ("cascade", "waterfall"),
        ("automne", "autumn"), ("hiver", "winter"), ("printemps", "spring"), ("été", "summer"),
        ("paysage", "landscape"), ("paysages", "landscape"), ("pluie", "rain"), ("galaxie", "galaxy"),
        ("étoiles", "stars"), ("étoile", "stars"), ("japon", "japan"), ("rivière", "river"), ("arbre", "tree"),
        ("arbres", "trees"), ("route", "road"), ("pont", "bridge"), ("château", "castle"), ("île", "island"),
        ("volcan", "volcano"), ("lune", "moon"), ("soleil", "sun"), ("planète", "planet"), ("jeu", "video games"),
        ("jeux", "video games"), ("animaux", "animals"), ("oiseau", "bird"), ("sombre", "dark"), ("coloré", "colorful"),
        ("noir", "black"), ("bleu", "blue"), ("vert", "green"), ("rouge", "red"), ("violet", "purple"), ("orange", "orange"),
        ("brume", "fog"), ("brouillard", "fog"), ("glace", "ice"), ("champ", "field"), ("fleuve", "river"),
    ];
    let lower = query.trim().to_lowercase();
    let lower = lower.replace("coucher de soleil", "sunset").replace("lever de soleil", "sunrise").replace("jeu vidéo", "video games");
    lower
        .split_whitespace()
        .map(|w| WORDS.iter().find(|(fr, _)| *fr == w).map(|(_, en)| en.to_string()).unwrap_or_else(|| w.to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn encode(q: &str) -> String {
    q.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn parse(json: &[u8]) -> anyhow::Result<Vec<Wallpaper>> {
    let r: Response = serde_json::from_slice(json).context("réponse Wallhaven illisible")?;
    Ok(r.data
        .into_iter()
        .map(|w| Wallpaper { id: w.id, resolution: w.resolution, page_url: w.url, full_url: w.path, thumb_url: w.thumbs.small })
        .collect())
}

/// Recherche (mots-clés vides = sélection au hasard) adaptée à l'écran `width` × `height`.
pub fn search(query: &str, width: u32, height: u32) -> anyhow::Result<Vec<Wallpaper>> {
    let q = translate(query);
    let sorting = if q.is_empty() { "random" } else { "relevance" };
    let url = format!(
        "{API}?categories=100&purity=100&sorting={sorting}&atleast={width}x{height}&ratios={}&q={}",
        ratio(width, height),
        encode(&q)
    );
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", "-A", "NOOS"])
        .arg(&url)
        .output()
        .context("curl introuvable")?;
    if !out.status.success() {
        bail!("Wallhaven injoignable : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse(&out.stdout)
}

fn curl_to(url: &str, dest: &std::path::Path) -> anyhow::Result<()> {
    let tmp = dest.with_extension("part");
    let ok = Command::new("curl").args(["-fsSL", "--max-time", "120", "-o"]).arg(&tmp).arg(url).status()?.success();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        bail!("téléchargement impossible");
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

impl Wallpaper {
    pub fn thumbnail(&self) -> anyhow::Result<PathBuf> {
        let path = thumbs_dir().join(format!("{}.jpg", self.id));
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            curl_to(&self.thumb_url, &path)?;
        }
        Ok(path)
    }

    fn file_name(&self) -> String {
        let ext = self.full_url.rsplit('.').next().filter(|e| e.len() <= 4).unwrap_or("jpg");
        format!("wallhaven-{}.{ext}", self.id)
    }

    /// Image en pleine résolution (au moins celle de l'écran), dans `~/Images/Wallpapers`.
    pub fn download(&self) -> anyhow::Result<PathBuf> {
        self.download_to(&dir())
    }

    /// Téléchargement pour un aperçu (cache), sans encombrer `~/Images/Wallpapers`.
    pub fn download_preview(&self) -> anyhow::Result<PathBuf> {
        let existing = dir().join(self.file_name());
        if existing.exists() {
            return Ok(existing);
        }
        self.download_to(&paths::preview_dir())
    }

    fn download_to(&self, target: &std::path::Path) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(target)?;
        let path = target.join(self.file_name());
        if !path.exists() {
            curl_to(&self.full_url, &path)?;
        }
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_ratio_and_translation() {
        assert_eq!(ratio(2560, 1440), "16x9");
        assert_eq!(ratio(1920, 1200), "16x10");
        assert_eq!(ratio(3440, 1440), "21x9");
        assert_eq!(ratio(1080, 1920), "9x16");
        assert_eq!(translate("Montagne coucher de soleil"), "mountain sunset");
        assert_eq!(translate("cyberpunk ville nuit"), "cyberpunk city night");
    }

    #[test]
    fn parses_response() {
        let json = br#"{"data":[{"id":"jxwyrw","url":"https://wallhaven.cc/w/jxwyrw","path":"https://w.wallhaven.cc/full/jx/wallhaven-jxwyrw.jpg","resolution":"3840x2160","thumbs":{"large":"l","original":"o","small":"https://th.wallhaven.cc/small/jx/jxwyrw.jpg"}}],"meta":{}}"#;
        let w = &parse(json).unwrap()[0];
        assert_eq!(w.resolution, "3840x2160");
        assert!(w.full_url.ends_with(".jpg"));
    }
}

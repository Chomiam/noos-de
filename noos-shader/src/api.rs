//! API Shadertoy (https://www.shadertoy.com/howto#q2) : chaque utilisateur crée sa propre clé
//! (« Your Apps » sur shadertoy.com) ; elle est rangée dans `~/.config/noos/shadertoy-key`
//! (lisible par lui seul). Seuls les shaders publiés « Public + API » sont accessibles.
//!
//! Requêtes HTTP par `curl` (déjà présent sur NOOS).

use std::{path::Path, process::Command};

use anyhow::{Context, bail};
use serde_json::Value;

use crate::store::Saved;

const BASE: &str = "https://www.shadertoy.com/api/v1";

fn key_path() -> std::path::PathBuf {
    noos_core::paths::config_dir().join("shadertoy-key")
}

pub fn key() -> Option<String> {
    std::fs::read_to_string(key_path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn set_key(key: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let p = key_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    if key.trim().is_empty() {
        let _ = std::fs::remove_file(&p);
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&p)?;
    std::io::Write::write_all(&mut f, key.trim().as_bytes())?;
    Ok(())
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn get_json(url: &str) -> anyhow::Result<Value> {
    let out = Command::new("curl").args(["-sfL", "--max-time", "15", "-A", "NOOS", url]).output().context("curl introuvable")?;
    if !out.status.success() {
        bail!("Shadertoy ne répond pas (connexion à Internet ?)");
    }
    let v: Value = serde_json::from_slice(&out.stdout).context("réponse de Shadertoy illisible")?;
    if let Some(e) = v.get("Error").and_then(Value::as_str) {
        bail!("Shadertoy : {e}");
    }
    Ok(v)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Popular,
    Love,
    Newest,
    Hot,
}

impl Sort {
    fn param(self) -> &'static str {
        match self {
            Sort::Popular => "popular",
            Sort::Love => "love",
            Sort::Newest => "newest",
            Sort::Hot => "hot",
        }
    }
}

/// Recherche (texte vide = tous) : nombre total et identifiants de la page.
pub fn query(key: &str, text: &str, sort: Sort, from: usize, num: usize) -> anyhow::Result<(usize, Vec<String>)> {
    let path = if text.trim().is_empty() { "shaders".to_string() } else { format!("shaders/query/{}", encode(text.trim())) };
    let v = get_json(&format!("{BASE}/{path}?sort={}&from={from}&num={num}&key={}", sort.param(), encode(key)))?;
    let total = v.get("Shaders").and_then(Value::as_u64).unwrap_or(0) as usize;
    let ids = v.get("Results").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    Ok((total, ids))
}

/// Shader distant et compatibilité : une seule passe « Image » (plus éventuellement « Common »),
/// sans textures, sons ni tampons (non pris en charge par l'écran de verrouillage).
#[derive(Debug, Clone)]
pub struct Remote {
    pub saved: Saved,
    pub compatible: bool,
    pub reason: String,
}

pub fn shader(key: &str, id: &str) -> anyhow::Result<Remote> {
    let v = get_json(&format!("{BASE}/shaders/{}?key={}", encode(id), encode(key)))?;
    let s = v.get("Shader").context("shader absent de la réponse")?;
    let info = s.get("info").cloned().unwrap_or_default();
    let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let num = |k: &str| info.get(k).and_then(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0);
    let passes = s.get("renderpass").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut image = String::new();
    let mut common = String::new();
    let mut reason = String::new();
    for p in &passes {
        let kind = p.get("type").and_then(Value::as_str).unwrap_or("");
        let code = p.get("code").and_then(Value::as_str).unwrap_or("").to_string();
        let inputs = p.get("inputs").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
        match kind {
            "image" if inputs == 0 => image = code,
            "image" => reason = "utilise des textures ou des sons".into(),
            "common" => common = code,
            "buffer" => reason = "utilise plusieurs passes (tampons)".into(),
            "sound" => reason = "produit du son".into(),
            "cubemap" => reason = "utilise un cubemap".into(),
            other => reason = format!("passe « {other} » non prise en charge"),
        }
    }
    if image.is_empty() && reason.is_empty() {
        reason = "pas de passe Image".into();
    }
    let tags = info.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect()).unwrap_or_default();
    Ok(Remote {
        compatible: reason.is_empty(),
        reason,
        saved: Saved {
            id: text("id"),
            name: text("name"),
            author: text("username"),
            description: text("description"),
            tags,
            likes: num("likes"),
            views: num("viewed"),
            image,
            common,
            saved_at: 0,
        },
    })
}

/// Vignette officielle (`/media/shaders/<id>.jpg`).
pub fn download_thumb(id: &str, dest: &Path) -> anyhow::Result<()> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = dest.with_extension("part");
    let ok = Command::new("curl")
        .args(["-sfL", "--max-time", "15", "-A", "NOOS", "-o"])
        .arg(&tmp)
        .arg(format!("https://www.shadertoy.com/media/shaders/{}.jpg", encode(id)))
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        bail!("vignette indisponible");
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Enregistre un shader distant dans la bibliothèque (avec sa vignette).
pub fn save(remote: &Remote) -> anyhow::Result<Saved> {
    anyhow::ensure!(remote.compatible, "shader incompatible : {}", remote.reason);
    let mut s = remote.saved.clone();
    s.saved_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    crate::store::save(&s)?;
    let _ = download_thumb(&s.id, &crate::store::thumb_path(&s.id));
    Ok(s)
}

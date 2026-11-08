//! Bibliothèque locale : `~/.local/share/noos/shaders/<id>/shader.json`.
//! Animations téléchargées depuis le catalogue NOOS ou importées (fichier, code collé).

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
    /// Licence déclarée (catalogue) ; vide pour une animation importée.
    pub license: String,
    /// Provenance : `catalogue`, `import`, ou `shadertoy` (anciennes versions de NOOS).
    pub origin: String,
    /// Page d'origine, si connue.
    pub url: String,
    /// Code de la passe « Image » (format `mainImage`) et de l'onglet « Common ».
    pub image: String,
    pub common: String,
    /// Date d'ajout (secondes depuis 1970).
    pub saved_at: u64,
}

pub fn dir() -> PathBuf {
    noos_core::paths::xdg_data_home().join("noos").join("shaders")
}

pub fn valid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn get(id: &str) -> Option<Saved> {
    if !valid(id) {
        return None;
    }
    let text = std::fs::read_to_string(dir().join(id).join("shader.json")).ok()?;
    let mut s: Saved = serde_json::from_str(&text).ok()?;
    if s.origin.is_empty() && s.url.is_empty() {
        // Animation téléchargée par l'ancienne intégration Shadertoy.
        s.origin = "shadertoy".into();
        s.url = format!("https://www.shadertoy.com/view/{}", s.id);
    }
    Some(s)
}

/// Animations de la bibliothèque, les plus récentes d'abord.
pub fn list() -> Vec<Saved> {
    let mut v: Vec<Saved> = std::fs::read_dir(dir())
        .map(|rd| rd.flatten().filter_map(|e| get(&e.file_name().to_string_lossy())).collect())
        .unwrap_or_default();
    v.sort_by_key(|s| std::cmp::Reverse(s.saved_at));
    v
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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

/// « Mon Shader génial ! » → `mon-shader-genial`.
fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        };
        if !(c == '-' && s.ends_with('-')) {
            s.push(c);
        }
    }
    let s = s.trim_matches('-').chars().take(40).collect::<String>();
    if s.is_empty() { "animation".into() } else { s }
}

/// Importe du code (Shadertoy `mainImage` ou GLSL Sandbox, converti) sous un nom.
pub fn import(name: &str, code: &str, url: &str) -> anyhow::Result<Saved> {
    anyhow::ensure!(crate::looks_valid(code), "ce code ne contient ni « mainImage » ni « void main » avec « gl_FragColor »");
    let base = format!("import-{}", slug(name));
    let mut id = base.clone();
    let mut n = 2;
    while dir().join(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    let s = Saved {
        id,
        name: if name.trim().is_empty() { "Animation importée".into() } else { name.trim().into() },
        author: String::new(),
        origin: "import".into(),
        url: url.into(),
        image: crate::adapt(code),
        saved_at: now(),
        ..Default::default()
    };
    save(&s)?;
    Ok(s)
}

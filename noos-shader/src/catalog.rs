//! Catalogue d'animations NOOS : un index JSON (code compris) hébergé dans le dépôt GitHub
//! de NOOS (`data/shaders/catalog/index.json`), téléchargé par `curl` — ni compte ni clé.
//! Toutes les animations du catalogue sont sous licence libre (redistribuables).
//! `NOOS_SHADER_CATALOG` : autre adresse ou fichier local (tests).

use std::process::Command;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::store::{self, Saved};

pub const URL: &str = "https://raw.githubusercontent.com/Chomiam/noos-de/main/data/shaders/catalog/index.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub tags: Vec<String>,
    pub license: String,
    /// Clin d'œil (ex. « Écran de veille de Windows 95 »).
    pub homage: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Catalog {
    pub version: u32,
    pub shaders: Vec<Entry>,
}

/// Copie livrée avec NOOS (paquet : `/opt/noos/share/noos/shaders/catalog/index.json`) :
/// catalogue hors ligne quand GitHub est injoignable.
fn bundled() -> Option<Catalog> {
    [noos_core::paths::data_dir(), std::path::PathBuf::from("/opt/noos/share/noos")]
        .iter()
        .map(|d| d.join("shaders/catalog/index.json"))
        .find_map(|p| serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok())
}

/// Télécharge le catalogue (quelques dizaines de Ko) ; sinon la copie livrée avec NOOS.
pub fn fetch() -> anyhow::Result<Catalog> {
    match fetch_remote() {
        Ok(c) => Ok(c),
        Err(e) => bundled().ok_or(e),
    }
}

fn fetch_remote() -> anyhow::Result<Catalog> {
    let src = std::env::var("NOOS_SHADER_CATALOG").unwrap_or_else(|_| URL.into());
    let text = if src.starts_with("http") {
        let out = Command::new("curl").args(["-sfL", "-m", "20", &src]).output().context("curl")?;
        if !out.status.success() {
            bail!("catalogue injoignable (connexion Internet ?)");
        }
        String::from_utf8(out.stdout)?
    } else {
        std::fs::read_to_string(&src).with_context(|| src.clone())?
    };
    let c: Catalog = serde_json::from_str(&text).context("catalogue illisible")?;
    Ok(c)
}

pub fn spec(id: &str) -> String {
    format!("library:cat-{id}")
}

pub fn installed(id: &str) -> bool {
    store::get(&format!("cat-{id}")).is_some()
}

/// Ajoute une animation du catalogue à la bibliothèque.
pub fn install(e: &Entry) -> anyhow::Result<Saved> {
    let s = Saved {
        id: format!("cat-{}", e.id),
        name: e.name.clone(),
        author: e.author.clone(),
        description: e.description.clone(),
        tags: e.tags.clone(),
        license: e.license.clone(),
        origin: "catalogue".into(),
        url: "https://github.com/Chomiam/noos-de/tree/main/data/shaders/catalog".into(),
        image: e.code.clone(),
        common: String::new(),
        saved_at: store::now(),
    };
    store::save(&s)?;
    Ok(s)
}

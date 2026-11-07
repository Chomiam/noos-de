//! Réglages de l'éditeur (`~/.config/noos/texteditor.toml`) : apparence, édition, fichiers
//! récents et session (onglets et dossier rouverts au démarrage).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Police (description Pango, ex. « Monospace 11 »).
    pub font: String,
    /// Thème de coloration ; « noos » suit le mode clair / sombre.
    pub scheme: String,
    pub wrap: bool,
    pub minimap: bool,
    pub whitespace: bool,
    pub highlight_line: bool,
    pub line_numbers: bool,
    pub tab_width: u32,
    pub spaces: bool,
    /// Marge droite (colonne) ; 0 = masquée.
    pub right_margin: u32,
    pub autosave: bool,
    pub completion: bool,
    pub restore_session: bool,
    pub sidebar: bool,
    pub recent: Vec<String>,
    pub session: Vec<String>,
    pub folder: Option<String>,
    pub width: i32,
    pub height: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: "Monospace 11".into(),
            scheme: "noos".into(),
            wrap: false,
            minimap: true,
            whitespace: false,
            highlight_line: true,
            line_numbers: true,
            tab_width: 4,
            spaces: true,
            right_margin: 100,
            autosave: false,
            completion: true,
            restore_session: true,
            sidebar: true,
            recent: vec![],
            session: vec![],
            folder: None,
            width: 1100,
            height: 760,
        }
    }
}

fn path() -> PathBuf {
    noos_core::paths::config_dir().join("texteditor.toml")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }

    pub fn add_recent(&mut self, file: &str) {
        self.recent.retain(|f| f != file);
        self.recent.insert(0, file.to_string());
        self.recent.truncate(12);
    }
}

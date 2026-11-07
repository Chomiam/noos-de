//! Réglages du terminal (`~/.config/noos/terminal.toml`) et alias de la barre latérale.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TermConfig {
    /// Famille de police (« Monospace » = police à chasse fixe du système).
    pub font_family: String,
    pub font_size: f64,
    /// Identifiant du thème (voir `themes.rs`) ; « noos » suit le mode clair/sombre.
    pub theme: String,
    /// Opacité du fond (0.5 à 1).
    pub opacity: f64,
    /// Copier automatiquement le texte sélectionné.
    pub copy_on_select: bool,
    /// Barre latérale des alias visible.
    pub sidebar: bool,
    pub scrollback_lines: i64,
    pub cursor_blink: bool,
    #[serde(rename = "alias")]
    pub aliases: Vec<Alias>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Alias {
    pub name: String,
    pub command: String,
    /// Ouvrir dans un nouvel onglet (sinon : exécuter dans l'onglet actif).
    pub new_tab: bool,
}

impl Default for TermConfig {
    fn default() -> Self {
        Self {
            font_family: "Monospace".into(),
            font_size: 11.0,
            theme: "noos".into(),
            opacity: 1.0,
            copy_on_select: false,
            sidebar: false,
            scrollback_lines: 10_000,
            cursor_blink: true,
            aliases: vec![
                Alias { name: "Claude".into(), command: "claude".into(), new_tab: true },
                Alias { name: "Mettre à jour le système".into(), command: "NOOS_PKG_ELEVATE=sudo noos-pkg upgrade".into(), new_tab: true },
            ],
        }
    }
}

fn path() -> PathBuf {
    noos_core::paths::config_dir().join("terminal.toml")
}

impl TermConfig {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| toml::from_str(&s).map_err(|e| eprintln!("noos-term : terminal.toml invalide : {e}")).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&p, s) {
                    eprintln!("noos-term : écriture de {} : {e}", p.display());
                }
            }
            Err(e) => eprintln!("noos-term : {e}"),
        }
    }
}

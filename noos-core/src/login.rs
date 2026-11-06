//! Configuration de l'écran de connexion noos-login : `/etc/noos/login.toml`.
//!
//! Fichier système (l'écran de connexion tourne sous l'utilisateur `greeter`, avant toute
//! session) : lu par noos-login, écrit par Paramètres → Personnalisation → Écran de
//! connexion. L'installateur crée `/etc/noos` avec le groupe `wheel` en écriture : les
//! administrateurs le modifient sans mot de passe ; sinon Paramètres passe par pkexec.

use std::{fs, path::{Path, PathBuf}};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::config::ColorScheme;

pub const DIR: &str = "/etc/noos";
pub const FILE: &str = "/etc/noos/login.toml";
/// Image de fond copiée à côté de la configuration (`login-background.<ext>`).
pub const BACKGROUND_STEM: &str = "login-background";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LoginConfig {
    /// Image de fond (chemin absolu, lisible par l'utilisateur `greeter`). Vide = dégradé NOOS.
    pub background: String,
    /// Flouter le fond derrière le formulaire de connexion (comme Windows 11).
    pub blur: bool,
    /// Assombrissement du fond (0.0 – 0.8).
    pub dim: f64,
    /// Écran de verrouillage d'abord (grande horloge), le formulaire au clic ou à une touche.
    pub lock_screen: bool,
    /// Horloge sur 24 h.
    pub clock_24h: bool,
    /// Liste des comptes en bas à gauche (sinon : saisie du nom d'utilisateur).
    pub show_user_list: bool,
    pub color_scheme: ColorScheme,
    /// Palette NOOS (voir `noos-theme`).
    pub palette: String,
    /// Couleur d'accent `#rrggbb`.
    pub accent: String,
    /// Session proposée par défaut (identifiant du fichier .desktop, ex. `noos`).
    pub default_session: String,
}

impl Default for LoginConfig {
    fn default() -> Self {
        Self {
            background: String::new(),
            blur: true,
            dim: 0.25,
            lock_screen: true,
            clock_24h: true,
            show_user_list: true,
            color_scheme: ColorScheme::Dark,
            palette: "nuit-saphir".into(),
            accent: "#89b4fa".into(),
            default_session: "noos".into(),
        }
    }
}

impl LoginConfig {
    pub fn load() -> Self {
        Self::load_from(Path::new(FILE)).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lecture de {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("ouverture de {}", path.display())),
        }
    }

    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Écriture directe (atomique) dans `dir`, si ce dossier est accessible en écriture.
    pub fn save_in(&self, dir: &Path) -> anyhow::Result<PathBuf> {
        let path = dir.join("login.toml");
        let tmp = dir.join(".login.toml.tmp");
        fs::write(&tmp, self.to_toml()?).with_context(|| format!("écriture dans {}", dir.display()))?;
        fs::rename(&tmp, &path)?;
        Ok(path)
    }
}

/// Le dossier `/etc/noos` est-il modifiable par l'utilisateur courant ?
pub fn dir_writable() -> bool {
    let probe = Path::new(DIR).join(format!(".probe-{}", std::process::id()));
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial() {
        let c = LoginConfig { accent: "#ff0000".into(), blur: false, ..Default::default() };
        let back: LoginConfig = toml::from_str(&c.to_toml().unwrap()).unwrap();
        assert_eq!(c, back);
        let partial: LoginConfig = toml::from_str("clock_24h = false").unwrap();
        assert!(!partial.clock_24h && partial.blur);
    }
}

//! Vérification des mises à jour en arrière-plan : état partagé entre le service
//! (`noos-pkg check`, lancé par le timer utilisateur `noos-update-check.timer`), l'icône de la
//! barre des tâches (noos-shell), Logiciels et Paramètres › Mises à jour.
//! Fichier : `~/.local/state/noos/updates.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Item {
    pub name: String,
    /// `dnf`, `flatpak`…
    pub backend: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    /// Dernière vérification (secondes depuis 1970).
    pub checked_at: u64,
    pub total: usize,
    /// Nombre de mises à jour par source (« Fedora », « Flatpak »…).
    pub by_backend: Vec<(String, usize)>,
    pub items: Vec<Item>,
    /// Erreurs éventuelles (source injoignable…).
    pub errors: Vec<String>,
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| crate::paths::home().join(".local/state"));
    base.join("noos").join("updates.json")
}

pub fn load() -> State {
    std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(s: &State) -> std::io::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    // Écriture atomique : l'icône de la barre surveille ce fichier.
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).unwrap_or_default())?;
    std::fs::rename(tmp, p)
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// « il y a 5 min », « il y a 3 h », « hier », « il y a 4 jours », « jamais ».
pub fn ago(t: u64) -> String {
    if t == 0 {
        return "jamais".into();
    }
    let d = now().saturating_sub(t);
    match d {
        0..=59 => "à l'instant".into(),
        60..=3599 => format!("il y a {} min", d / 60),
        3600..=86_399 => format!("il y a {} h", d / 3600),
        86_400..=172_799 => "hier".into(),
        _ => format!("il y a {} jours", d / 86_400),
    }
}

/// Intervalles proposés (heures) et leur libellé.
pub const INTERVALS: &[(u32, &str)] = &[
    (2, "Toutes les 2 heures"),
    (4, "Toutes les 4 heures"),
    (6, "Toutes les 6 heures"),
    (12, "Deux fois par jour"),
    (24, "Une fois par jour"),
    (72, "Tous les 3 jours"),
    (168, "Une fois par semaine"),
];

#[cfg(test)]
mod tests {
    #[test]
    fn ago() {
        assert_eq!(super::ago(0), "jamais");
        assert_eq!(super::ago(super::now() - 7200), "il y a 2 h");
    }
}

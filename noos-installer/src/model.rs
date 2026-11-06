//! Choix de l'utilisateur, partagés par toutes les pages.

use crate::disk::Disk;

#[derive(Debug, Clone)]
pub struct Choices {
    /// Locale du système installé (ex. `fr_FR.UTF-8`).
    pub locale: String,
    /// Fuseau horaire IANA (ex. `Europe/Paris`).
    pub timezone: String,
    pub kb_layout: String,
    pub kb_variant: String,
    pub kb_label: String,
    pub full_name: String,
    pub username: String,
    pub password: String,
    pub hostname: String,
    pub strong_password: bool,
    pub disk: Option<Disk>,
    pub swap: bool,
    pub swap_gib: u32,
    pub rpmfusion: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            locale: "fr_FR.UTF-8".into(),
            timezone: "Europe/Paris".into(),
            kb_layout: "fr".into(),
            kb_variant: String::new(),
            kb_label: "Français".into(),
            full_name: String::new(),
            username: String::new(),
            password: String::new(),
            hostname: String::new(),
            strong_password: true,
            disk: None,
            swap: true,
            swap_gib: crate::disk::default_swap_gib(),
            rpmfusion: true,
        }
    }
}

pub const LOCALES: &[(&str, &str, &str, &str)] = &[
    // (locale, nom, disposition par défaut, fuseau par défaut)
    ("fr_FR.UTF-8", "Français (France)", "fr", "Europe/Paris"),
    ("fr_BE.UTF-8", "Français (Belgique)", "be", "Europe/Brussels"),
    ("fr_CH.UTF-8", "Français (Suisse)", "ch:fr", "Europe/Zurich"),
    ("fr_CA.UTF-8", "Français (Canada)", "ca", "America/Toronto"),
    ("en_US.UTF-8", "English (United States)", "us", "America/New_York"),
    ("en_GB.UTF-8", "English (United Kingdom)", "gb", "Europe/London"),
    ("de_DE.UTF-8", "Deutsch", "de", "Europe/Berlin"),
    ("es_ES.UTF-8", "Español", "es", "Europe/Madrid"),
    ("it_IT.UTF-8", "Italiano", "it", "Europe/Rome"),
    ("pt_BR.UTF-8", "Português (Brasil)", "br", "America/Sao_Paulo"),
    ("nl_NL.UTF-8", "Nederlands", "nl", "Europe/Amsterdam"),
];

/// Paquet de langue glibc pour une locale (`fr_FR.UTF-8` → `glibc-langpack-fr`).
pub fn langpack(locale: &str) -> String {
    format!("glibc-langpack-{}", locale.split(['_', '.']).next().unwrap_or("en"))
}

//! noos-pkg — paquets de NOOS (basé sur Fedora) : **dnf** et **Flatpak**.
//!
//! | Backend   | Contenu                                         | Élévation                 |
//! |-----------|-------------------------------------------------|---------------------------|
//! | `dnf`     | paquets RPM (Fedora, RPM Fusion, COPR…)         | pkexec                    |
//! | `flatpak` | applications Flatpak (Flathub, Fedora Flatpaks) | aucune (`--user`) / polkit|
//!
//! Chaque backend sait : rechercher, lister les paquets installés explicitement, lister et
//! appliquer les mises à jour, installer/supprimer, et gérer ses dépôts. Les dépôts bruts
//! sont regroupés en **sources** lisibles (voir [`sources`]) et les résultats des deux
//! backends en **logiciels** proposant une ou plusieurs installations (voir [`catalog`]).

pub mod backends;
pub mod catalog;
pub mod exec;
pub mod firewall;
pub mod sources;
pub mod system;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use exec::{Log, Plan};
pub use system::SystemInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Flatpak,
    Dnf,
}

impl BackendKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Flatpak => "flatpak",
            Self::Dnf => "dnf",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Applications Flatpak",
            Self::Dnf => "Paquets Fedora (dnf)",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Flatpak, Self::Dnf]
            .into_iter()
            .find(|k| k.id() == s)
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Installation Flatpak : utilisateur (sans mot de passe) ou système.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    User,
    System,
}

impl Scope {
    pub fn flag(self) -> &'static str {
        match self {
            Scope::User => "--user",
            Scope::System => "--system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Package {
    pub backend: Option<BackendKind>,
    /// Identifiant passé à install/remove (nom du paquet ou ID d'appli Flatpak).
    pub id: String,
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Dépôt / remote d'origine.
    pub origin: String,
    pub installed: bool,
    pub scope: Option<Scope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Update {
    pub package: Package,
    pub old_version: String,
    pub new_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Repo {
    pub backend: BackendKind,
    /// Identifiant interne utilisé pour activer / désactiver / supprimer.
    pub id: String,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub scope: Option<Scope>,
    /// Fichier .repo (dnf) ou nom du remote (Flatpak).
    pub file: String,
    /// Dépôt fourni par la distribution (supprimable uniquement à vos risques).
    pub builtin: bool,
}

/// Dépôt additionnel proposé pour une distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: &'static str,
    pub backend: BackendKind,
    pub name: &'static str,
    pub description: &'static str,
    /// Le préréglage attend un argument (COPR « propriétaire/projet », URL…).
    pub argument: Option<&'static str>,
}

pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>>;
    /// Paquets installés explicitement (pas les dépendances).
    fn installed(&self) -> anyhow::Result<Vec<Package>>;
    fn updates(&self) -> anyhow::Result<Vec<Update>>;

    /// Rafraîchit les métadonnées des dépôts.
    fn refresh(&self) -> Plan;
    fn install(&self, packages: &[Package]) -> Plan;
    fn remove(&self, packages: &[Package]) -> Plan;
    fn upgrade_all(&self) -> Plan;

    fn repos(&self) -> anyhow::Result<Vec<Repo>>;
    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan>;
    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan>;
    fn presets(&self) -> Vec<Preset>;
    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan>;
    /// Active / désactive plusieurs dépôts en une fois (une source = plusieurs dépôts).
    fn set_repos_enabled(&self, repos: &[Repo], enabled: bool) -> anyhow::Result<Plan> {
        let mut plan = Plan::new();
        for r in repos {
            plan = plan.extend(self.set_repo_enabled(r, enabled)?);
        }
        Ok(plan)
    }

    /// Identifiants AppStream des paquets (nom du paquet → identifiant en minuscules) : sert à
    /// reconnaître le même logiciel en paquet dnf et en Flatpak.
    fn appstream_ids(&self) -> std::collections::HashMap<String, String> {
        Default::default()
    }
}

/// Backends disponibles sur cette machine, dans l'ordre d'affichage.
pub fn detect() -> Vec<Box<dyn Backend>> {
    let sys = SystemInfo::detect();
    let mut list: Vec<Box<dyn Backend>> = Vec::new();
    if exec::have("dnf") || exec::have("dnf5") {
        list.push(Box::new(backends::dnf::Dnf::new(sys.clone())));
    }
    if exec::have("flatpak") {
        list.push(Box::new(backends::flatpak::Flatpak::new(sys)));
    }
    list
}

/// Classe les résultats : correspondance exacte, puis préfixe, puis le reste.
pub fn rank(query: &str, packages: &mut [Package]) {
    let q = query.to_lowercase();
    packages.sort_by_key(|p| {
        let id = p.id.to_lowercase();
        let name = p.name.to_lowercase();
        let last = id.rsplit('.').next().unwrap_or(&id).to_string();
        let score = if id == q || name == q || last == q {
            0
        } else if name.starts_with(&q) || last.starts_with(&q) || id.starts_with(&q) {
            1
        } else if name.contains(&q) || id.contains(&q) {
            2
        } else {
            3
        };
        (score, name.len(), name)
    });
}

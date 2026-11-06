//! noos-pkg — une seule interface pour tous les gestionnaires de paquets.
//!
//! | Backend      | Distributions                               | Élévation                 |
//! |--------------|---------------------------------------------|---------------------------|
//! | `flatpak`    | toutes                                      | aucune (`--user`) / polkit|
//! | `rpm-ostree` | Fedora Atomic (Silverblue, Kinoite, Bazzite…)| polkit (démon rpm-ostreed)|
//! | `dnf`        | Fedora, RHEL, Alma, Rocky, CentOS Stream    | pkexec                    |
//! | `apt`        | Debian, Ubuntu, Mint, Pop!_OS…              | pkexec                    |
//! | `pacman`     | Arch, EndeavourOS, Manjaro, CachyOS…        | pkexec                    |
//! | `aur`        | Arch + paru/yay                             | l'assistant (via pkexec)  |
//!
//! Chaque backend sait : rechercher, lister les paquets installés explicitement, lister et
//! appliquer les mises à jour, installer/supprimer, et gérer ses dépôts additionnels
//! (Flathub, RPM Fusion, COPR, PPA, contrib/non-free, multilib, Chaotic-AUR…).

pub mod backends;
pub mod exec;
pub mod firewall;
pub mod system;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use exec::{Log, Plan};
pub use system::SystemInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Flatpak,
    RpmOstree,
    Dnf,
    Apt,
    Pacman,
    Aur,
}

impl BackendKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Flatpak => "flatpak",
            Self::RpmOstree => "rpm-ostree",
            Self::Dnf => "dnf",
            Self::Apt => "apt",
            Self::Pacman => "pacman",
            Self::Aur => "aur",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Flatpak",
            Self::RpmOstree => "Système (rpm-ostree)",
            Self::Dnf => "Système (dnf)",
            Self::Apt => "Système (apt)",
            Self::Pacman => "Système (pacman)",
            Self::Aur => "AUR",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Flatpak, Self::RpmOstree, Self::Dnf, Self::Apt, Self::Pacman, Self::Aur]
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
    /// Fichier de configuration (dnf/apt/pacman).
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
    /// Le préréglage attend un argument (COPR « user/projet », PPA « user/ppa », URL…).
    pub argument: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Status {
    /// Un redémarrage est nécessaire pour appliquer des changements (rpm-ostree).
    pub reboot_required: bool,
    pub message: String,
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

    fn status(&self) -> Status {
        Status::default()
    }
}

/// Backends disponibles sur cette machine, dans l'ordre d'affichage.
pub fn detect() -> Vec<Box<dyn Backend>> {
    let sys = SystemInfo::detect();
    let mut list: Vec<Box<dyn Backend>> = Vec::new();
    if sys.atomic && exec::have("rpm-ostree") {
        list.push(Box::new(backends::rpmostree::RpmOstree::new(sys.clone())));
    } else if exec::have("dnf") || exec::have("dnf5") {
        list.push(Box::new(backends::dnf::Dnf::new(sys.clone())));
    } else if exec::have("apt-get") {
        list.push(Box::new(backends::apt::Apt::new(sys.clone())));
    } else if exec::have("pacman") {
        list.push(Box::new(backends::pacman::Pacman::new(sys.clone())));
        if let Some(aur) = backends::aur::Aur::detect() {
            list.push(Box::new(aur));
        }
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

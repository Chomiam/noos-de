//! noos-pkg — paquets de NOOS (basé sur Debian sid) : **apt** et **Flatpak**.
//!
//! | Backend   | Contenu                                              | Élévation                 |
//! |-----------|------------------------------------------------------|---------------------------|
//! | `apt`     | paquets .deb (Debian sid, contrib / non-free, NOOS…) | pkexec                    |
//! | `flatpak` | applications Flatpak (Flathub)                       | aucune (`--user`) / polkit|
//!
//! Chaque backend sait : rechercher, lister les paquets installés explicitement, lister et
//! appliquer les mises à jour, installer/supprimer, et gérer ses dépôts. Les dépôts bruts
//! sont regroupés en **sources** lisibles (voir [`sources`]) et les résultats des deux
//! backends en **logiciels** proposant une ou plusieurs installations (voir [`catalog`]).

pub mod appimage;
pub mod appimagehub;
pub mod backends;
pub mod catalog;
pub mod cleanup;
pub mod deb;
pub mod details;
pub mod exec;
pub mod hardware;
pub mod history;
pub mod odrs;
pub mod owner;
pub mod packs;
pub mod permissions;
pub mod sources;
pub mod space;
pub mod system;

use std::fmt;

use serde::{Deserialize, Serialize};

pub use exec::{Log, Plan};
pub use system::SystemInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Flatpak,
    Apt,
}

impl BackendKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::Flatpak => "flatpak",
            Self::Apt => "apt",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Flatpak => "Applications Flatpak",
            Self::Apt => "Paquets Debian (apt)",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Flatpak, Self::Apt]
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
    /// Fichier de sources APT ou nom du remote (Flatpak).
    pub file: String,
    /// Dépôt fourni par la distribution (supprimable uniquement à vos risques).
    pub builtin: bool,
    /// Suites APT (`sid`, `experimental`…) ; vide pour Flatpak.
    #[serde(default)]
    pub suites: Vec<String>,
    /// Composants APT (`main`, `contrib`, `non-free`…) ; vide pour Flatpak.
    #[serde(default)]
    pub components: Vec<String>,
}

/// Dépôt additionnel proposé pour une distribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: &'static str,
    pub backend: BackendKind,
    pub name: &'static str,
    pub description: &'static str,
    /// Le préréglage attend un argument (adresse d'un dépôt, URL…).
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
    /// reconnaître le même logiciel en paquet Debian et en Flatpak.
    fn appstream_ids(&self) -> std::collections::HashMap<String, String> {
        Default::default()
    }

    /// Fiche détaillée d'un paquet (descriptif, versions, tailles, dépôt, dépendances,
    /// AppStream). Lecture seule, sans droits root.
    fn details(&self, package: &Package) -> anyhow::Result<details::Details> {
        anyhow::bail!("pas de fiche détaillée pour {}", package.id)
    }

    /// Applications d'une catégorie AppStream (`Graphics`, `Game`…) : vitrine pour les paquets
    /// Debian.
    fn apps_in_category(&self, category: &str) -> Vec<Package> {
        let _ = category;
        vec![]
    }

    /// Met à jour seulement ces paquets (sélection de la page Mises à jour).
    fn upgrade(&self, packages: &[Package]) -> Plan {
        let _ = packages;
        self.upgrade_all()
    }

    /// Nouveautés d'une version (journal des modifications), de la plus récente à la version
    /// installée exclue. Peut demander le réseau.
    fn changelog(&self, package: &Package) -> anyhow::Result<Vec<details::ChangelogEntry>> {
        anyhow::bail!("pas de journal des modifications pour {}", package.id)
    }

    /// Simule l'installation (`install`) ou la suppression d'un paquet : ce qui sera ajouté,
    /// mis à jour ou retiré en plus de lui.
    fn simulate(&self, _package: &Package, _install: bool) -> anyhow::Result<details::Effect> {
        Ok(Default::default())
    }
}

/// Backends disponibles sur cette machine, dans l'ordre d'affichage.
pub fn detect() -> Vec<Box<dyn Backend>> {
    let sys = SystemInfo::detect();
    let mut list: Vec<Box<dyn Backend>> = Vec::new();
    if exec::have("apt-get") && exec::have("dpkg-query") {
        list.push(Box::new(backends::apt::Apt::new(sys.clone())));
    }
    if exec::have("flatpak") {
        list.push(Box::new(backends::flatpak::Flatpak::new(sys)));
    }
    list
}

/// Recherche des mises à jour **sans droits root** : apt télécharge les listes de paquets dans
/// le cache de l'utilisateur (`~/.cache/noos/apt`, `force` : même si elles ont moins de 6 h),
/// Flatpak interroge ses dépôts directement. Utilisé par le service `noos-update-check` et par
/// Logiciels.
pub fn check_updates(backends: &[Box<dyn Backend>], force: bool) -> Vec<(BackendKind, anyhow::Result<Vec<Update>>)> {
    backends
        .iter()
        .map(|b| {
            if b.kind() == BackendKind::Apt {
                backends::apt::refresh_user_lists(force);
            }
            (b.kind(), b.updates())
        })
        .collect()
}

/// Installe les mises à jour Flatpak trouvées (service `noos-update-check`, réglage
/// `[updates] auto_flatpak`) ; renvoie leur nombre si l'opération a réussi.
pub fn auto_update_flatpak(backends: &[Box<dyn Backend>], found: &[(BackendKind, Result<Vec<Update>, String>)]) -> Option<usize> {
    let ups = found.iter().find(|(k, _)| *k == BackendKind::Flatpak)?.1.as_ref().ok()?;
    if ups.is_empty() {
        return None;
    }
    let b = backends.iter().find(|b| b.kind() == BackendKind::Flatpak)?;
    let pkgs: Vec<Package> = ups.iter().map(|u| u.package.clone()).collect();
    b.upgrade(&pkgs).run(&|l| eprintln!("{l}")).ok()?;
    Some(ups.len())
}

/// Notification de bureau (sans effet hors session graphique).
pub fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("gdbus")
        .args([
            "call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path", "/org/freedesktop/Notifications",
            "--method", "org.freedesktop.Notifications.Notify", "Logiciels", "0", "system-software-install", title, body, "[]", "{}", "8000",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Mises à jour apt « au redémarrage » (systemd.offline-updates) : téléchargement maintenant,
/// installation au démarrage suivant par `noos-offline-update.service`.
pub mod offline {
    use crate::Plan;

    pub const MARK: &str = "/var/lib/noos/offline-update";

    /// Télécharge les paquets et programme l'installation (une élévation).
    pub fn schedule_plan() -> Plan {
        Plan::new()
            .root_owned(crate::backends::apt::apt_root(&["update"]))
            .root_owned(crate::backends::apt::apt_root(&["--download-only", "full-upgrade"]))
            .root(&["mkdir", "-p", "/var/lib/noos"])
            .root(&["touch", MARK])
            .root(&["ln", "-sfn", MARK, "/system-update"])
    }

    pub fn cancel_plan() -> Plan {
        Plan::new().root(&["rm", "-f", "/system-update", MARK])
    }

    /// Installation programmée au prochain redémarrage ?
    pub fn pending() -> bool {
        std::fs::read_link("/system-update").is_ok_and(|l| l == std::path::Path::new(MARK))
    }

    /// Dernière installation hors ligne : (code de sortie d'apt, date en secondes).
    pub fn last_result() -> Option<(i32, i64)> {
        let t = std::fs::read_to_string(format!("{MARK}.result")).ok()?;
        let mut it = t.split_whitespace();
        Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn programmation() {
            let d = super::schedule_plan().describe().join("\n");
            assert!(d.contains("--download-only full-upgrade"), "{d}");
            assert!(d.contains("ln -sfn /var/lib/noos/offline-update /system-update"), "{d}");
        }
    }
}

/// Enregistre le résultat d'une recherche (`~/.local/state/noos/updates.json`) : l'icône de
/// la barre des tâches et Paramètres › Mises à jour le lisent.
pub fn record_updates(results: &[(BackendKind, Result<Vec<Update>, String>)]) -> noos_core::updates::State {
    let mut st = noos_core::updates::State { checked_at: noos_core::updates::now(), ..Default::default() };
    for (kind, r) in results {
        match r {
            Ok(list) => {
                st.total += list.len();
                if !list.is_empty() {
                    st.by_backend.push((kind.label().to_string(), list.len()));
                }
                st.items.extend(list.iter().take(500).map(|u| noos_core::updates::Item {
                    name: u.package.name.clone(),
                    backend: kind.id().to_string(),
                    old: u.old_version.clone(),
                    new: u.new_version.clone(),
                }));
            }
            Err(e) => st.errors.push(format!("{} : {e}", kind.label())),
        }
    }
    let _ = noos_core::updates::save(&st);
    st
}

/// Une mise à jour de ces paquets demande-t-elle un redémarrage (noyau, libc, systemd,
/// pilotes, compositeur NOOS…) ?
pub fn needs_reboot(names: &[&str]) -> bool {
    names.iter().any(|n| {
        n.starts_with("linux-image")
            || n.starts_with("linux-modules")
            || n.starts_with("nvidia-kernel")
            || n.starts_with("nvidia-driver")
            || n.starts_with("firmware-")
            || matches!(*n, "libc6" | "systemd" | "libsystemd0" | "dbus" | "dbus-daemon" | "noos" | "mesa-vulkan-drivers" | "libgl1-mesa-dri" | "intel-microcode" | "amd64-microcode")
    })
}

/// Pertinence d'un nom pour une requête : 0 exact, 1 préfixe, 2 contient, 3 ailleurs
/// (description, mots-clés).
pub fn match_score(query: &str, id: &str, name: &str) -> u8 {
    let q = query.trim().to_lowercase();
    let id = id.to_lowercase();
    let name = name.to_lowercase();
    let last = id.rsplit('.').next().unwrap_or(&id).to_string();
    if id == q || name == q || last == q {
        0
    } else if name.starts_with(&q) || last.starts_with(&q) || id.starts_with(&q) {
        1
    } else if name.contains(&q) || id.contains(&q) {
        2
    } else {
        3
    }
}

/// Classe les résultats : correspondance exacte, puis préfixe, puis le reste. Les résultats
/// trouvés ailleurs que dans le nom gardent leur ordre (déjà trié par la source).
pub fn rank(query: &str, packages: &mut [Package]) {
    packages.sort_by_key(|p| {
        let score = match_score(query, &p.id, &p.name);
        let name = p.name.to_lowercase();
        if score < 3 { (score, name.len(), name) } else { (3, 0, String::new()) }
    });
}

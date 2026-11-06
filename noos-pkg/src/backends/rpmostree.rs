//! rpm-ostree : Fedora Atomic (Silverblue, Kinoite, Sway Atomic, Bazzite, Aurora…).
//!
//! Particularités :
//! - les paquets RPM sont **superposés** à l'image et ne s'appliquent qu'au redémarrage ;
//! - les mises à jour remplacent l'image entière (un seul élément « Système ») ;
//! - pas de pkexec : le démon rpm-ostreed demande lui-même l'autorisation via polkit.

use super::{dnf, yumrepo};
use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, Status, SystemInfo, Update,
    exec::{capture, capture_codes},
};

pub struct RpmOstree {
    sys: SystemInfo,
}

impl RpmOstree {
    pub fn new(sys: SystemInfo) -> Self {
        Self { sys }
    }

    fn status_json() -> anyhow::Result<serde_json::Value> {
        Ok(serde_json::from_str(&capture(&["rpm-ostree", "status", "--json"])?)?)
    }
}

/// Analyse la sortie de `rpm-ostree search` (« nom : résumé » sous des titres « ===== »).
pub fn parse_search(out: &str) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    out.lines()
        .filter(|l| !l.starts_with("====="))
        .filter_map(|l| l.split_once(" : "))
        .map(|(n, s)| (n.trim().to_string(), s.trim().to_string()))
        .filter(|(n, _)| !n.is_empty() && seen.insert(n.clone()))
        .collect()
}

impl Backend for RpmOstree {
    fn kind(&self) -> BackendKind {
        BackendKind::RpmOstree
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        let out = capture_codes(&["rpm-ostree", "search", q], &[0, 1])?;
        let installed: std::collections::HashSet<String> =
            capture(&["rpm", "-qa", "--qf", "%{NAME}\n"]).unwrap_or_default().lines().map(String::from).collect();
        let mut pkgs: Vec<Package> = parse_search(&out)
            .into_iter()
            .map(|(name, summary)| Package {
                backend: Some(BackendKind::RpmOstree),
                installed: installed.contains(&name),
                id: name.clone(),
                name,
                summary,
                origin: "rpm-ostree".into(),
                ..Default::default()
            })
            .collect();
        crate::rank(q, &mut pkgs);
        Ok(pkgs)
    }

    /// Paquets superposés (demandés), y compris ceux en attente de redémarrage.
    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        let status = Self::status_json()?;
        let mut pkgs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for d in status["deployments"].as_array().into_iter().flatten().take(1) {
            for key in ["requested-packages", "requested-local-packages"] {
                for name in d[key].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
                    if seen.insert(name.to_string()) {
                        pkgs.push(Package {
                            backend: Some(BackendKind::RpmOstree),
                            id: name.into(),
                            name: name.into(),
                            summary: "Paquet superposé à l'image système".into(),
                            installed: true,
                            ..Default::default()
                        });
                    }
                }
            }
        }
        Ok(pkgs)
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        // Code 77 = pas de mise à jour.
        let out = capture_codes(&["rpm-ostree", "upgrade", "--check"], &[0, 77])?;
        if !out.contains("AvailableUpdate") {
            return Ok(vec![]);
        }
        let field = |name: &str| {
            out.lines()
                .find_map(|l| l.trim().strip_prefix(&format!("{name}:")))
                .map(|v| v.trim().to_string())
                .unwrap_or_default()
        };
        let current = Self::status_json()
            .ok()
            .and_then(|s| {
                s["deployments"]
                    .as_array()?
                    .iter()
                    .find(|d| d["booted"].as_bool() == Some(true))?["version"]
                    .as_str()
                    .map(String::from)
            })
            .unwrap_or_default();
        let diff = field("Diff");
        Ok(vec![Update {
            package: Package {
                backend: Some(BackendKind::RpmOstree),
                id: "system".into(),
                name: format!("Image système {}", self.sys.name),
                summary: if diff.is_empty() { "Nouvelle image système".into() } else { diff },
                installed: true,
                ..Default::default()
            },
            old_version: current,
            new_version: field("Version"),
        }])
    }

    fn refresh(&self) -> Plan {
        Plan::new().user(&["rpm-ostree", "refresh-md"])
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> =
            ["rpm-ostree", "install", "-y", "--idempotent", "--allow-inactive"].map(String::from).to_vec();
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().user_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> = ["rpm-ostree", "uninstall", "-y"].map(String::from).to_vec();
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().user_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        Plan::new().user(&["rpm-ostree", "upgrade"])
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        yumrepo::list(BackendKind::RpmOstree)
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        dnf::toggle_plan(repo, enabled)
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        dnf::remove_plan(repo, |owner| Plan::new().user_owned(vec!["rpm-ostree".into(), "uninstall".into(), "-y".into(), owner]))
    }

    fn presets(&self) -> Vec<Preset> {
        dnf::rpm_presets(&self.sys, BackendKind::RpmOstree)
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        match preset {
            "rpmfusion-free" | "rpmfusion-nonfree" => {
                let mut argv: Vec<String> = ["rpm-ostree", "install", "-y", "--idempotent"].map(String::from).to_vec();
                argv.extend(yumrepo::rpmfusion_urls(&self.sys, preset == "rpmfusion-nonfree"));
                Ok(Plan::new().user_owned(argv))
            }
            _ => dnf::add_repo_file_plan(&self.sys, preset, argument),
        }
    }

    fn status(&self) -> Status {
        let Ok(status) = Self::status_json() else { return Status::default() };
        let pending = status["deployments"]
            .as_array()
            .and_then(|d| d.first())
            .is_some_and(|d| d["booted"].as_bool() == Some(false));
        Status {
            reboot_required: pending,
            message: if pending {
                "Des changements du système seront appliqués au prochain redémarrage.".into()
            } else {
                String::new()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_output() {
        let out = "\n===== Name Matched =====\nhtop : Interactive process viewer\npcp-htop : PCP viewer\n\n===== Summary Matched =====\nhtop : Interactive process viewer\n";
        let r = parse_search(out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], ("htop".into(), "Interactive process viewer".into()));
    }
}

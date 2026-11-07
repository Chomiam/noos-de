//! Flatpak — installations système (Flathub et Fedora Flatpaks, actifs par défaut sur NOOS)
//! et utilisateur (`--user`, sans mot de passe).

use std::collections::{HashMap, HashSet};

use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, Scope, SystemInfo, Update,
    exec::{capture, capture_codes},
};

pub struct Flatpak {
    _sys: SystemInfo,
}

const FLATHUB: &str = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const FLATHUB_BETA: &str = "https://flathub.org/beta-repo/flathub-beta.flatpakrepo";

fn scope_of(s: &str) -> Scope {
    if s.contains("user") { Scope::User } else { Scope::System }
}

fn cols(line: &str) -> Vec<&str> {
    line.split('\t').collect()
}

impl Flatpak {
    pub fn new(sys: SystemInfo) -> Self {
        Self { _sys: sys }
    }

    /// (nom du remote, installation, options) de tous les remotes.
    fn remotes() -> Vec<(String, Scope, String, String, String)> {
        capture(&["flatpak", "remotes", "--show-disabled", "--columns=name,title,url,options"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let c = cols(l);
                (c.len() >= 4).then(|| (c[0].to_string(), scope_of(c[3]), c[1].to_string(), c[2].to_string(), c[3].to_string()))
            })
            .collect()
    }

    fn installed_ids() -> HashSet<String> {
        capture(&["flatpak", "list", "--app", "--columns=application"])
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .collect()
    }

    fn by_scope(packages: &[Package]) -> Vec<(Scope, Option<String>, Vec<String>)> {
        let mut groups: Vec<(Scope, Option<String>, Vec<String>)> = Vec::new();
        for p in packages {
            let scope = p.scope.unwrap_or(Scope::User);
            let origin = (!p.origin.is_empty()).then(|| p.origin.clone());
            match groups.iter_mut().find(|(s, o, _)| *s == scope && *o == origin) {
                Some(g) => g.2.push(p.id.clone()),
                None => groups.push((scope, origin, vec![p.id.clone()])),
            }
        }
        groups
    }
}

impl Backend for Flatpak {
    fn kind(&self) -> BackendKind {
        BackendKind::Flatpak
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        let out = capture(&["flatpak", "search", "--columns=application,name,version,branch,remotes,description", q])?;
        let remotes = Self::remotes();
        let installed = Self::installed_ids();
        let mut seen = HashSet::new();
        let mut pkgs: Vec<Package> = Vec::new();
        for l in out.lines() {
            let c = cols(l);
            if c.len() < 6 {
                continue;
            }
            // Une proposition par remote actif (Flathub, Fedora Flatpaks…) : l'utilisateur choisit.
            for r in c[4].split(',').map(str::trim) {
                for (name, scope, ..) in remotes.iter().filter(|x| x.0 == r && !x.4.contains("disabled")) {
                    if !seen.insert((c[0].to_string(), name.clone(), *scope)) {
                        continue;
                    }
                    pkgs.push(Package {
                        backend: Some(BackendKind::Flatpak),
                        id: c[0].into(),
                        name: c[1].into(),
                        version: c[2].into(),
                        summary: c[5].into(),
                        origin: name.clone(),
                        scope: Some(*scope),
                        installed: installed.contains(c[0]),
                    });
                }
            }
        }
        crate::rank(q, &mut pkgs);
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        let out = capture(&["flatpak", "list", "--app", "--columns=application,name,version,origin,installation,description"])?;
        let mut pkgs: Vec<Package> = out
            .lines()
            .filter_map(|l| {
                let c = cols(l);
                (c.len() >= 5).then(|| Package {
                    backend: Some(BackendKind::Flatpak),
                    id: c[0].into(),
                    name: c[1].into(),
                    version: c[2].into(),
                    origin: c[3].into(),
                    scope: Some(scope_of(c[4])),
                    summary: c.get(5).copied().unwrap_or("").into(),
                    installed: true,
                })
            })
            .collect();
        pkgs.sort_by_key(|p| p.name.to_lowercase());
        Ok(pkgs)
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        let current: HashMap<String, String> = capture(&["flatpak", "list", "--columns=application,version,installation"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let c = cols(l);
                (c.len() >= 3).then(|| (format!("{}@{}", c[0], c[2]), c[1].to_string()))
            })
            .collect();
        let mut updates = Vec::new();
        for scope in [Scope::User, Scope::System] {
            // Une installation sans remote renvoie une erreur : on l'ignore.
            let Ok(out) = capture_codes(
                &["flatpak", "remote-ls", "--updates", scope.flag(), "--columns=application,name,version,origin"],
                &[0],
            ) else {
                continue;
            };
            let scope_name = if scope == Scope::User { "user" } else { "system" };
            for l in out.lines() {
                let c = cols(l);
                if c.len() < 4 {
                    continue;
                }
                updates.push(Update {
                    old_version: current.get(&format!("{}@{scope_name}", c[0])).cloned().unwrap_or_default(),
                    new_version: c[2].into(),
                    package: Package {
                        backend: Some(BackendKind::Flatpak),
                        id: c[0].into(),
                        name: if c[1].is_empty() { c[0].into() } else { c[1].into() },
                        origin: c[3].into(),
                        scope: Some(scope),
                        installed: true,
                        ..Default::default()
                    },
                });
            }
        }
        Ok(updates)
    }

    fn refresh(&self) -> Plan {
        let remotes = Self::remotes();
        let mut plan = Plan::new();
        for scope in [Scope::User, Scope::System] {
            if remotes.iter().any(|r| r.1 == scope && !r.4.contains("disabled")) {
                plan = plan.user(&["flatpak", "update", "--appstream", scope.flag()]);
            }
        }
        plan
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut plan = Plan::new();
        for (scope, origin, ids) in Self::by_scope(packages) {
            let mut argv: Vec<String> = vec!["flatpak".into(), "install".into(), "--noninteractive".into(), "-y".into(), scope.flag().into()];
            if let Some(o) = origin {
                argv.push(o);
            }
            argv.extend(ids);
            plan = plan.user_owned(argv);
        }
        plan
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut plan = Plan::new();
        for (scope, _, ids) in Self::by_scope(packages) {
            let mut argv: Vec<String> = vec!["flatpak".into(), "uninstall".into(), "--noninteractive".into(), "-y".into(), scope.flag().into()];
            argv.extend(ids);
            plan = plan.user_owned(argv);
        }
        plan
    }

    fn upgrade_all(&self) -> Plan {
        let mut plan = Plan::new().user(&["flatpak", "update", "--noninteractive", "-y", "--user"]);
        if Self::remotes().iter().any(|r| r.1 == Scope::System) {
            plan = plan.user(&["flatpak", "update", "--noninteractive", "-y", "--system"]);
        }
        plan
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        Ok(Self::remotes()
            .into_iter()
            // Les « -origin » sont des remotes techniques créés pour des .flatpak isolés.
            .filter(|(_, _, _, _, opts)| !opts.contains("no-enumerate"))
            .map(|(name, scope, title, url, opts)| Repo {
                backend: BackendKind::Flatpak,
                id: format!("{}:{name}", if scope == Scope::User { "user" } else { "system" }),
                name: if title.is_empty() || title == "-" {
                    name.clone()
                } else if opts.contains("filtered") {
                    format!("{title} (filtré par la distribution)")
                } else {
                    title
                },
                url,
                enabled: !opts.contains("disabled"),
                scope: Some(scope),
                file: name.clone(),
                builtin: opts.contains("oci") && scope == Scope::System,
            })
            .collect())
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        let scope = repo.scope.unwrap_or(Scope::User);
        Ok(Plan::new().user(&["flatpak", "remote-modify", scope.flag(), if enabled { "--enable" } else { "--disable" }, &repo.file]))
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        let scope = repo.scope.unwrap_or(Scope::User);
        Ok(Plan::new().user(&["flatpak", "remote-delete", scope.flag(), &repo.file]))
    }

    fn presets(&self) -> Vec<Preset> {
        let mut p = vec![
            Preset {
                id: "flathub-system",
                backend: BackendKind::Flatpak,
                name: "Flathub",
                description: "Le principal magasin d'applications Flatpak, pour tous les comptes de la machine",
                argument: None,
            },
            Preset {
                id: "flathub",
                backend: BackendKind::Flatpak,
                name: "Flathub (votre compte uniquement)",
                description: "Installations sans mot de passe, visibles seulement par vous",
                argument: None,
            },
            Preset {
                id: "flathub-beta",
                backend: BackendKind::Flatpak,
                name: "Flathub — tests (bêta)",
                description: "Versions bêta des applications Flathub",
                argument: None,
            },
            Preset {
                id: "flatpakrepo",
                backend: BackendKind::Flatpak,
                name: "Dépôt .flatpakrepo",
                description: "Nom et URL d'un fichier .flatpakrepo",
                argument: Some("nom https://…/depot.flatpakrepo"),
            },
        ];
        if Self::remotes().iter().any(|r| r.0 == "flathub" && r.1 == Scope::System && r.4.contains("filtered")) {
            p.insert(0, Preset {
                id: "flathub-unfilter",
                backend: BackendKind::Flatpak,
                name: "Flathub complet (retirer le filtre Fedora)",
                description: "Fedora limite Flathub système à une sélection : ce réglage donne accès à tout le catalogue",
                argument: None,
            });
        }
        p
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        let add = |scope: Scope, name: &str, url: &str| {
            Plan::new().user(&["flatpak", "remote-add", "--if-not-exists", scope.flag(), name, url])
        };
        match preset {
            "flathub" => Ok(add(Scope::User, "flathub", FLATHUB)),
            "flathub-system" => Ok(add(Scope::System, "flathub", FLATHUB)),
            "flathub-beta" => Ok(add(Scope::User, "flathub-beta", FLATHUB_BETA)),
            "flathub-unfilter" => Ok(Plan::new().user(&["flatpak", "remote-modify", "--system", "--no-filter", "flathub"])),
            "flatpakrepo" => {
                let arg = argument.unwrap_or("").trim();
                let (name, url) = arg
                    .split_once(char::is_whitespace)
                    .map(|(n, u)| (n.trim(), u.trim()))
                    .filter(|(n, u)| !n.is_empty() && u.starts_with("https://"))
                    .ok_or_else(|| anyhow::anyhow!("format attendu : nom https://…/depot.flatpakrepo"))?;
                if !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
                    anyhow::bail!("nom de dépôt invalide : {name}");
                }
                Ok(add(Scope::User, name, url))
            }
            _ => anyhow::bail!("préréglage inconnu : {preset}"),
        }
    }
}

//! dnf5 (Fedora 45).

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use super::yumrepo;
use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, SystemInfo, Update,
    exec::capture,
};

/// Séparateur de champs improbable dans un résumé de paquet.
const SEP: &str = "\u{1f}";

pub struct Dnf {
    sys: SystemInfo,
    bin: &'static str,
    appstream: OnceLock<HashMap<String, String>>,
}

impl Dnf {
    pub fn new(sys: SystemInfo) -> Self {
        let bin = if crate::exec::have("dnf") { "dnf" } else { "dnf5" };
        Self { sys, bin, appstream: OnceLock::new() }
    }

    fn query_format() -> String {
        // Vraie fin de ligne : dnf5 ne l'ajoute pas, dnf4 en ajoute une de plus (ignorée).
        format!("%{{name}}{SEP}%{{evr}}{SEP}%{{arch}}{SEP}%{{repoid}}{SEP}%{{summary}}\n")
    }

    fn repoquery(&self, extra: &[&str]) -> anyhow::Result<Vec<Package>> {
        let qf = format!("--queryformat={}", Self::query_format());
        let mut argv = vec![self.bin, "repoquery", "--quiet", qf.as_str()];
        argv.extend_from_slice(extra);
        let out = capture(&argv)?;
        let arch = native_arch();
        let mut seen = HashSet::new();
        Ok(out
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(SEP).collect();
                if f.len() < 5 || (f[2] != arch && f[2] != "noarch") || !seen.insert(f[0].to_string()) {
                    return None;
                }
                Some(Package {
                    backend: Some(BackendKind::Dnf),
                    id: f[0].into(),
                    name: f[0].into(),
                    version: f[1].into(),
                    origin: f[3].into(),
                    summary: f[4].into(),
                    ..Default::default()
                })
            })
            .collect())
    }

    fn installed_versions() -> HashMap<String, String> {
        capture(&["rpm", "-qa", "--qf", "%{NAME}\t%{VERSION}-%{RELEASE}\n"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split_once('\t').map(|(n, v)| (n.to_string(), v.to_string())))
            .collect()
    }
}

pub fn native_arch() -> String {
    capture(&["uname", "-m"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "x86_64".into())
}

/// Dépôts additionnels proposés.
pub fn rpm_presets() -> Vec<Preset> {
    let backend = BackendKind::Dnf;
    vec![
        Preset {
            id: "rpmfusion-free",
            backend,
            name: "RPM Fusion — libre",
            description: "Codecs et logiciels libres absents de Fedora (ffmpeg complet, VLC…)",
            argument: None,
        },
        Preset {
            id: "rpmfusion-nonfree",
            backend,
            name: "RPM Fusion — non libre",
            description: "Pilotes NVIDIA, Steam, firmwares et codecs propriétaires (ajoute aussi la partie libre)",
            argument: None,
        },
        Preset {
            id: "copr",
            backend,
            name: "COPR",
            description: "Dépôt communautaire Fedora (propriétaire/projet)",
            argument: Some("propriétaire/projet"),
        },
        Preset {
            id: "repo-url",
            backend,
            name: "Fichier .repo",
            description: "Ajoute un dépôt depuis l'URL d'un fichier .repo",
            argument: Some("https://…/fichier.repo"),
        },
    ]
}

/// Lit `dnf repoquery --whatprovides 'metainfo()'` : nom du paquet → identifiant AppStream
/// (minuscules, sans `.desktop` / `.appdata.xml` / `.metainfo.xml`).
pub fn parse_metainfo(out: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut current: Option<String> = None;
    for line in out.lines() {
        let line = line.trim();
        let provide = match line.split_once(SEP) {
            Some((name, rest)) => {
                current = Some(name.to_string());
                rest
            }
            None => line,
        };
        let Some(name) = current.as_ref() else { continue };
        let Some(id) = provide.strip_prefix("metainfo(").and_then(|p| p.strip_suffix(')')) else { continue };
        let id = id.trim_end_matches(".xml").trim_end_matches(".appdata").trim_end_matches(".metainfo");
        if id.is_empty() || map.contains_key(name) {
            continue;
        }
        map.insert(name.clone(), id.trim_end_matches(".desktop").to_lowercase());
    }
    map
}

/// Plan d'ajout d'un .repo (COPR ou URL) : téléchargement en root.
pub fn add_repo_file_plan(sys: &SystemInfo, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
    let arg = argument.map(str::trim).filter(|a| !a.is_empty());
    let (url, path) = match preset {
        "copr" => yumrepo::copr(sys, arg.ok_or_else(|| anyhow::anyhow!("indiquez propriétaire/projet"))?)?,
        "repo-url" => {
            let url = arg.ok_or_else(|| anyhow::anyhow!("indiquez l'URL du fichier .repo"))?;
            if !url.starts_with("https://") {
                anyhow::bail!("seules les URL https:// sont acceptées");
            }
            (url.to_string(), yumrepo::repo_path_for_url(url)?)
        }
        _ => anyhow::bail!("préréglage inconnu : {preset}"),
    };
    Ok(Plan::new().root_owned(vec!["curl".into(), "-fsSL".into(), "-o".into(), path, url]))
}

/// Activation / désactivation par réécriture des fichiers .repo (ou de la surcharge dnf5).
pub fn toggle_plan(repos: &[Repo], enabled: bool) -> anyhow::Result<Plan> {
    let mut plan = Plan::new();
    for (path, content) in yumrepo::toggle_many(repos, enabled)? {
        plan = plan.write(&path, &content);
    }
    Ok(plan)
}

/// Suppression : paquet propriétaire (ex. rpmfusion-free-release), section ou fichier.
pub fn remove_plan(repo: &Repo, remove_pkg: impl Fn(String) -> Plan) -> anyhow::Result<Plan> {
    if let Some(owner) = yumrepo::owner_package(&repo.file) {
        if repo.builtin || owner.starts_with("fedora-") || owner.ends_with("-repos") {
            anyhow::bail!("dépôt fourni par la distribution ({owner}) : désactivez-le plutôt que de le supprimer");
        }
        return Ok(remove_pkg(owner));
    }
    if repo.file.starts_with(yumrepo::VENDOR_DIR) {
        anyhow::bail!("fichier de la distribution (lecture seule) : désactivez ce dépôt");
    }
    let text = std::fs::read_to_string(&repo.file)?;
    Ok(match yumrepo::remove_section(&text, &repo.id) {
        Some(rest) => Plan::new().write(&repo.file, &rest),
        None => Plan::new().remove(&repo.file),
    })
}

impl Backend for Dnf {
    fn kind(&self) -> BackendKind {
        BackendKind::Dnf
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim().to_lowercase();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        let pattern = format!("*{q}*");
        let mut pkgs = self.repoquery(&["--latest-limit=1", &pattern])?;
        let installed = Self::installed_versions();
        for p in &mut pkgs {
            p.installed = installed.contains_key(&p.id);
        }
        crate::rank(&q, &mut pkgs);
        pkgs.truncate(150);
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        let versions = Self::installed_versions();
        let mut pkgs = self.repoquery(&["--userinstalled"])?;
        for p in &mut pkgs {
            p.installed = true;
            if let Some(v) = versions.get(&p.id) {
                p.version = v.clone();
            }
        }
        pkgs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(pkgs)
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        let versions = Self::installed_versions();
        let pkgs = self.repoquery(&["--upgrades", "--latest-limit=1"])?;
        Ok(pkgs
            .into_iter()
            .map(|p| Update {
                old_version: versions.get(&p.id).cloned().unwrap_or_default(),
                new_version: p.version.clone(),
                package: Package { installed: true, ..p },
            })
            .collect())
    }

    fn refresh(&self) -> Plan {
        Plan::new().root(&[self.bin, "makecache", "--refresh"])
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv = vec![self.bin.to_string(), "install".into(), "-y".into()];
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv = vec![self.bin.to_string(), "remove".into(), "-y".into()];
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        Plan::new().root(&[self.bin, "upgrade", "-y", "--refresh"])
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        yumrepo::list(BackendKind::Dnf)
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        toggle_plan(std::slice::from_ref(repo), enabled)
    }

    fn set_repos_enabled(&self, repos: &[Repo], enabled: bool) -> anyhow::Result<Plan> {
        toggle_plan(repos, enabled)
    }

    fn appstream_ids(&self) -> HashMap<String, String> {
        self.appstream
            .get_or_init(|| {
                let qf = format!("--queryformat=%{{name}}{SEP}%{{provides}}\n");
                capture(&[self.bin, "repoquery", "--quiet", "--latest-limit=1", "--whatprovides", "metainfo()", &qf])
                    .map(|out| parse_metainfo(&out))
                    .unwrap_or_default()
            })
            .clone()
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        remove_plan(repo, |owner| Plan::new().root_owned(vec![self.bin.into(), "remove".into(), "-y".into(), owner]))
    }

    fn presets(&self) -> Vec<Preset> {
        rpm_presets()
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        let install = |urls: Vec<String>| {
            let mut argv = vec![self.bin.to_string(), "install".into(), "-y".into()];
            argv.extend(urls);
            Plan::new().root_owned(argv)
        };
        match preset {
            "rpmfusion-free" => Ok(install(vec![yumrepo::rpmfusion_url(&self.sys, false)])),
            "rpmfusion-nonfree" => Ok(install(vec![
                yumrepo::rpmfusion_url(&self.sys, false),
                yumrepo::rpmfusion_url(&self.sys, true),
            ])),
            _ => add_repo_file_plan(&self.sys, preset, argument),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metainfo() {
        let out = format!(
            "gimp{SEP}application()\napplication(gimp.desktop)\nmetainfo()\nmetainfo(org.gimp.GIMP.appdata.xml)\n\
             vlc{SEP}metainfo(org.videolan.vlc.appdata.xml)\n\
             gimp-data-extras{SEP}metainfo()\ngimp-data-extras = 2.0.4\n\
             foo{SEP}metainfo(org.example.Foo.metainfo.xml)\n"
        );
        let m = parse_metainfo(&out);
        assert_eq!(m["gimp"], "org.gimp.gimp");
        assert_eq!(m["vlc"], "org.videolan.vlc");
        assert_eq!(m["foo"], "org.example.foo");
        assert!(!m.contains_key("gimp-data-extras"));
    }
}

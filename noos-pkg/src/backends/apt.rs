//! apt / dpkg (Debian sid).
//!
//! | Opération            | Commande                                                         |
//! |----------------------|------------------------------------------------------------------|
//! | recherche            | `apt-cache search --names-only` + `apt-cache policy`             |
//! | installés            | `apt-mark showmanual` + `dpkg-query` (hors priorités required / important) |
//! | mises à jour         | `apt-get -s full-upgrade` (simulation, sans droits)               |
//! | vérification (timer) | `apt-get update` dans le cache de l'utilisateur, sans root        |
//! | installer / retirer  | `apt-get install|remove -y` (pkexec)                             |
//! | AppStream            | fichiers DEP-11 de `/var/lib/apt/lists` (paquet `appstream`)     |

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use super::aptsources;
use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, SystemInfo, Update,
    exec::{capture, capture_codes},
};

pub struct Apt {
    _sys: SystemInfo,
    appstream: OnceLock<HashMap<String, String>>,
}

/// Options communes des commandes apt en root : pas de question, configuration locale conservée.
const NONINTERACTIVE: &[&str] = &[
    "env",
    "DEBIAN_FRONTEND=noninteractive",
    "apt-get",
    "-y",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

pub(crate) fn apt_root(args: &[&str]) -> Vec<String> {
    NONINTERACTIVE.iter().chain(args).map(|s| s.to_string()).collect()
}

/// Dossier des listes de paquets de l'utilisateur (vérification sans droits root).
pub fn user_lists_dir() -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".cache"));
    cache.join("noos/apt")
}

/// Date de la liste la plus récente d'un dossier de listes apt (fichiers *Release).
fn lists_mtime(dir: &Path) -> Option<std::time::SystemTime> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().ends_with("Release"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()
}

/// Options apt pointant sur les listes de l'utilisateur si elles sont plus récentes que
/// celles du système (sinon : aucune option, listes du système).
pub(crate) fn freshest_lists_opts() -> Vec<String> {
    let user = user_lists_dir();
    let sys = lists_mtime(Path::new("/var/lib/apt/lists"));
    match (lists_mtime(&user.join("lists")), sys) {
        (Some(u), Some(s)) if u > s => user_opts(&user),
        (Some(_), None) => user_opts(&user),
        _ => vec![],
    }
}

fn user_opts(base: &Path) -> Vec<String> {
    vec![
        "-o".into(),
        format!("Dir::State::Lists={}", base.join("lists").display()),
        "-o".into(),
        format!("Dir::Cache={}", base.join("cache").display()),
        "-o".into(),
        "Debug::NoLocking=1".into(),
    ]
}

/// `apt-get update` sans root, dans le cache de l'utilisateur. Sans `force`, seulement si les
/// listes ont plus de 6 heures.
pub fn refresh_user_lists(force: bool) {
    let base = user_lists_dir();
    if !force {
        if let Some(t) = lists_mtime(&base.join("lists")) {
            if t.elapsed().is_ok_and(|d| d.as_secs() < 6 * 3600) {
                return;
            }
        }
    }
    let _ = std::fs::create_dir_all(base.join("lists/partial"));
    let _ = std::fs::create_dir_all(base.join("cache/archives/partial"));
    let opts = user_opts(&base);
    let mut argv: Vec<&str> = vec!["apt-get", "-qq"];
    argv.extend(opts.iter().map(String::as_str));
    argv.push("update");
    let _ = capture_codes(&argv, &[0, 100]);
}

pub fn native_arch() -> String {
    capture(&["dpkg", "--print-architecture"]).map(|s| s.trim().to_string()).unwrap_or_else(|_| "amd64".into())
}

/// Nom court d'origine d'après une ligne de `apt-cache policy` ou l'étiquette d'une simulation.
pub fn origin_of(source: &str) -> String {
    let s = source.to_lowercase();
    if s.contains("chomiam.github.io/noos-de") || s.starts_with("noos") {
        "noos".into()
    } else if s.contains("experimental") {
        "debian-experimental".into()
    } else if s.contains("debian") {
        "debian".into()
    } else {
        // « 500 https://dl.example.com/linux stable/main amd64 Packages » → dl.example.com
        source
            .split_whitespace()
            .find(|w| w.contains("://"))
            .and_then(|u| u.split("://").nth(1))
            .and_then(|r| r.split('/').next())
            .unwrap_or(source)
            .to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Policy {
    pub installed: Option<String>,
    pub candidate: Option<String>,
    pub origin: String,
}

/// Lit la sortie de `apt-cache policy <paquets…>` (en `LC_ALL=C`).
pub fn parse_policy(out: &str) -> HashMap<String, Policy> {
    let mut map = HashMap::new();
    let mut name: Option<String> = None;
    let mut cur = Policy::default();
    let mut in_candidate = false;
    let flush = |name: &mut Option<String>, cur: &mut Policy, map: &mut HashMap<String, Policy>| {
        if let Some(n) = name.take() {
            map.insert(n, std::mem::take(cur));
        }
    };
    for line in out.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            flush(&mut name, &mut cur, &mut map);
            let n = line.trim_end_matches(':');
            // « paquet:i386 » : architecture étrangère, gardée telle quelle.
            name = Some(n.to_string());
            in_candidate = false;
            continue;
        }
        let t = line.trim();
        if let Some(v) = t.strip_prefix("Installed:") {
            let v = v.trim();
            cur.installed = (v != "(none)" && !v.is_empty()).then(|| v.to_string());
        } else if let Some(v) = t.strip_prefix("Candidate:") {
            let v = v.trim();
            cur.candidate = (v != "(none)" && !v.is_empty()).then(|| v.to_string());
        } else if t == "Version table:" {
            continue;
        } else {
            // Ligne de version : « *** 3.0.4-1 500 » ou « 3.0.4-1 500 » ; ligne de source :
            // « 500 http://deb.debian.org/debian sid/main amd64 Packages ».
            let words: Vec<&str> = t.trim_start_matches("***").split_whitespace().collect();
            if words.len() == 2 && words[1].parse::<i32>().is_ok() {
                in_candidate = cur.candidate.as_deref() == Some(words[0]);
            } else if in_candidate && cur.origin.is_empty() && words.len() >= 2 && !t.contains("/var/lib/dpkg/status") {
                cur.origin = origin_of(t);
            }
        }
    }
    flush(&mut name, &mut cur, &mut map);
    map
}

/// Lit `apt-cache search` : « nom - résumé ».
pub fn parse_search(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.split_once(" - ").map(|(n, s)| (n.trim().to_string(), s.trim().to_string())))
        .filter(|(n, _)| !n.is_empty() && !n.contains(' '))
        .collect()
}

/// Lit `apt-get -s full-upgrade` : « Inst nom [ancienne] (nouvelle Origine:suite [arch]) ».
pub fn parse_simulation(out: &str) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Inst ")?;
            let (name, rest) = rest.split_once(' ')?;
            let rest = rest.trim_start();
            // Nouveau paquet (dépendance) : pas de [ancienne version] → ignoré.
            let old = rest.strip_prefix('[')?.split_once(']')?.0.to_string();
            let paren = rest.split_once('(')?.1;
            let mut words = paren.split_whitespace();
            let new = words.next()?.to_string();
            let origin = words.next().map(origin_of).unwrap_or_default();
            Some(Update {
                old_version: old,
                new_version: new.clone(),
                package: Package {
                    backend: Some(BackendKind::Apt),
                    id: name.to_string(),
                    name: name.to_string(),
                    version: new,
                    origin,
                    installed: true,
                    ..Default::default()
                },
            })
        })
        .collect()
}

/// Paquets installés : nom → (version, priorité, résumé).
fn dpkg_installed() -> HashMap<String, (String, String, String)> {
    capture(&["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${db:Status-Abbrev}\t${Priority}\t${binary:Summary}\n"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 5 && f[2].starts_with("ii")).then(|| {
                // « paquet:amd64 » (architecture native) → « paquet ».
                let arch = format!(":{}", native_arch_cached());
                let name = f[0].strip_suffix(&arch).unwrap_or(f[0]).to_string();
                (name, (f[1].to_string(), f[3].to_string(), f[4].to_string()))
            })
        })
        .collect()
}

fn native_arch_cached() -> &'static str {
    static ARCH: OnceLock<String> = OnceLock::new();
    ARCH.get_or_init(native_arch)
}

/// Fichiers DEP-11 (AppStream de Debian) : `Package:` → `ID:` (minuscules, sans `.desktop`).
pub fn parse_dep11(text: &str, map: &mut HashMap<String, String>) {
    let (mut id, mut pkg): (Option<String>, Option<String>) = (None, None);
    let commit = |id: &mut Option<String>, pkg: &mut Option<String>, map: &mut HashMap<String, String>| {
        if let (Some(i), Some(p)) = (id.take(), pkg.take()) {
            map.entry(p).or_insert_with(|| i.trim_end_matches(".desktop").to_lowercase());
        }
    };
    for line in text.lines() {
        if line.starts_with("---") {
            commit(&mut id, &mut pkg, map);
        } else if let Some(v) = line.strip_prefix("ID: ") {
            id = Some(v.trim().trim_matches('\'').trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("Package: ") {
            pkg = Some(v.trim().to_string());
        }
    }
    commit(&mut id, &mut pkg, map);
}

fn read_maybe_compressed(path: &Path) -> Option<String> {
    let p = path.to_string_lossy();
    let tool = if p.ends_with(".gz") {
        "gzip"
    } else if p.ends_with(".xz") {
        "xz"
    } else if p.ends_with(".zst") {
        "zstd"
    } else if p.ends_with(".lz4") {
        "lz4"
    } else {
        return std::fs::read_to_string(path).ok();
    };
    capture(&[tool, "-dc", &p]).ok()
}

impl Apt {
    pub fn new(sys: SystemInfo) -> Self {
        Self { _sys: sys, appstream: OnceLock::new() }
    }

    fn policy(names: &[String], extra: &[String]) -> HashMap<String, Policy> {
        if names.is_empty() {
            return HashMap::new();
        }
        let mut argv: Vec<&str> = vec!["apt-cache"];
        argv.extend(extra.iter().map(String::as_str));
        argv.push("policy");
        argv.extend(names.iter().map(String::as_str));
        capture(&argv).map(|o| parse_policy(&o)).unwrap_or_default()
    }
}

/// Préréglages proposés (dépôts et réglages de Debian).
pub fn apt_presets() -> Vec<Preset> {
    let backend = BackendKind::Apt;
    vec![
        Preset {
            id: "debian-nonfree",
            backend,
            name: "Debian — contrib et non-free",
            description: "Pilotes NVIDIA, firmwares (Wi-Fi, cartes graphiques), Steam et logiciels non libres",
            argument: None,
        },
        Preset {
            id: "debian-experimental",
            backend,
            name: "Debian experimental",
            description: "Versions en préparation, installées seulement à la demande",
            argument: None,
        },
        Preset {
            id: "noos",
            backend,
            name: "NOOS",
            description: "Bureau NOOS et ses applications (dépôt APT sur GitHub Pages)",
            argument: None,
        },
        Preset {
            id: "google-chrome",
            backend,
            name: "Google Chrome",
            description: "Dépôt officiel de Google : le navigateur Chrome et ses mises à jour",
            argument: None,
        },
        Preset {
            id: "nvidia-cuda",
            backend,
            name: "NVIDIA — pilotes et CUDA",
            description: "Dépôt officiel de NVIDIA (Debian 13) : pilotes plus récents que ceux de Debian, CUDA",
            argument: None,
        },
        Preset {
            id: "i386",
            backend,
            name: "Applications 32 bits (i386)",
            description: "Architecture supplémentaire nécessaire à Steam et à Wine",
            argument: None,
        },
        Preset {
            id: "apt-repo",
            backend,
            name: "Dépôt APT",
            description: "Adresse, suite et composants d'un dépôt, avec l'adresse de sa clé de signature",
            argument: Some("https://… suite [composants…] clé=https://…/cle.asc"),
        },
        Preset {
            id: "sources-url",
            backend,
            name: "Fichier .sources",
            description: "Ajoute un dépôt depuis l'URL d'un fichier .sources (deb822)",
            argument: Some("https://…/fichier.sources"),
        },
    ]
}

/// Argument de « Dépôt APT » : (URI, suite, composants, URL de la clé).
pub fn parse_repo_argument(arg: &str) -> anyhow::Result<(String, String, Vec<String>, Option<String>)> {
    let mut words = arg.split_whitespace();
    let uri = words.next().filter(|u| u.starts_with("https://") || u.starts_with("http://")).ok_or_else(|| {
        anyhow::anyhow!("format attendu : https://dépôt suite [composants…] clé=https://…/cle.asc")
    })?;
    let suite = words.next().ok_or_else(|| anyhow::anyhow!("suite manquante (ex. stable, sid, ./)"))?;
    let mut comps = Vec::new();
    let mut key = None;
    for w in words {
        if let Some(k) = w.strip_prefix("clé=").or_else(|| w.strip_prefix("cle=")).or_else(|| w.strip_prefix("key=")) {
            if !k.starts_with("https://") {
                anyhow::bail!("la clé doit être téléchargée en https://");
            }
            key = Some(k.to_string());
        } else {
            if !w.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
                anyhow::bail!("composant invalide : {w}");
            }
            comps.push(w.to_string());
        }
    }
    if !suite.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c)) {
        anyhow::bail!("suite invalide : {suite}");
    }
    Ok((uri.trim_end_matches('/').to_string(), suite.to_string(), comps, key))
}

/// Plan d'ajout d'un préréglage.
pub fn add_preset_plan(preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
    let arg = argument.map(str::trim).filter(|a| !a.is_empty());
    let update = || apt_root(&["update"]);
    Ok(match preset {
        "debian-nonfree" => {
            let mut plan = Plan::new();
            let mut touched = false;
            for path in [aptsources::DEBIAN_SOURCES, aptsources::SOURCES_LIST] {
                let Ok(text) = std::fs::read_to_string(path) else { continue };
                let new = aptsources::add_components(path, &text, aptsources::NONFREE_COMPONENTS);
                if new != text {
                    plan = plan.write(path, &new);
                }
                touched |= aptsources::parse_file(path, &text).iter().any(|e| e.uris.iter().any(|u| aptsources::is_debian_uri(u)));
            }
            if !touched {
                anyhow::bail!("aucun dépôt Debian trouvé dans {} ni {}", aptsources::DEBIAN_SOURCES, aptsources::SOURCES_LIST);
            }
            plan.root_owned(update())
        }
        "debian-experimental" => Plan::new()
            .write(&format!("{}/debian-experimental.sources", aptsources::SOURCES_DIR), &aptsources::experimental_sources())
            .root_owned(update()),
        "noos" => Plan::new().write(aptsources::NOOS_SOURCES, &aptsources::noos_sources()).root_owned(update()),
        "i386" => Plan::new().root(&["dpkg", "--add-architecture", "i386"]).root_owned(update()),
        "google-chrome" => Plan::new()
            .root(&["install", "-d", "-m", "0755", "/etc/apt/trusted.gpg.d"])
            .root(&["curl", "-fsSL", "-o", aptsources::CHROME_KEYRING, aptsources::CHROME_KEY_URL])
            .write(aptsources::CHROME_LIST, &aptsources::chrome_list())
            .root_owned(update()),
        "nvidia-cuda" => Plan::new()
            .root(&["install", "-d", "-m", "0755", aptsources::KEYRINGS_DIR])
            .root(&["curl", "-fsSL", "-o", aptsources::NVIDIA_KEYRING, aptsources::NVIDIA_KEY_URL])
            .write(aptsources::NVIDIA_SOURCES, &aptsources::nvidia_sources())
            .root_owned(update()),
        "apt-repo" => {
            let (uri, suite, comps, key) = parse_repo_argument(arg.ok_or_else(|| anyhow::anyhow!("indiquez l'adresse du dépôt"))?)?;
            let name = aptsources::file_name_for(&uri);
            let mut plan = Plan::new();
            let signed_by = match key {
                Some(k) => {
                    let ext = if k.ends_with(".gpg") { "gpg" } else { "asc" };
                    let path = format!("{}/{name}.{ext}", aptsources::KEYRINGS_DIR);
                    plan = plan
                        .root(&["install", "-d", "-m", "0755", aptsources::KEYRINGS_DIR])
                        .root_owned(vec!["curl".into(), "-fsSL".into(), "-o".into(), path.clone(), k]);
                    path
                }
                None => String::new(),
            };
            plan.write(
                &format!("{}/{name}.sources", aptsources::SOURCES_DIR),
                &aptsources::deb822(&uri, &suite, &comps.join(" "), &signed_by),
            )
            .root_owned(update())
        }
        "sources-url" => {
            let url = arg.ok_or_else(|| anyhow::anyhow!("indiquez l'URL du fichier .sources"))?;
            if !url.starts_with("https://") {
                anyhow::bail!("seules les URL https:// sont acceptées");
            }
            let file = url.rsplit('/').next().unwrap_or("");
            let file: String = file.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect();
            if !(file.ends_with(".sources") || file.ends_with(".list")) || file.len() < 7 {
                anyhow::bail!("l'URL doit pointer vers un fichier .sources ou .list");
            }
            Plan::new()
                .root_owned(vec!["curl".into(), "-fsSL".into(), "-o".into(), format!("{}/{file}", aptsources::SOURCES_DIR), url.to_string()])
                .root_owned(update())
        }
        _ => anyhow::bail!("préréglage inconnu : {preset}"),
    })
}

impl Backend for Apt {
    fn kind(&self) -> BackendKind {
        BackendKind::Apt
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim().to_lowercase();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        // apt-cache search prend une expression régulière : on échappe la saisie.
        let pattern: String = q.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c.to_string() } else { format!("\\{c}") }).collect();
        let out = capture(&["apt-cache", "search", "--names-only", &pattern])?;
        let mut pkgs: Vec<Package> = parse_search(&out)
            .into_iter()
            .map(|(name, summary)| Package {
                backend: Some(BackendKind::Apt),
                id: name.clone(),
                name,
                summary,
                ..Default::default()
            })
            .collect();
        crate::rank(&q, &mut pkgs);
        pkgs.truncate(150);
        let names: Vec<String> = pkgs.iter().map(|p| p.id.clone()).collect();
        let policy = Self::policy(&names, &[]);
        for p in &mut pkgs {
            if let Some(pol) = policy.get(&p.id) {
                p.installed = pol.installed.is_some();
                p.version = pol.candidate.clone().or_else(|| pol.installed.clone()).unwrap_or_default();
                p.origin = pol.origin.clone();
            }
        }
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        let manual: HashSet<String> = capture(&["apt-mark", "showmanual"])?.lines().map(|l| l.trim().to_string()).collect();
        let arch = format!(":{}", native_arch_cached());
        let mut pkgs: Vec<Package> = dpkg_installed()
            .into_iter()
            .filter(|(name, (_, prio, _))| {
                (manual.contains(name) || manual.contains(&format!("{name}{arch}"))) && !matches!(prio.as_str(), "required" | "important")
            })
            .map(|(name, (version, _, summary))| Package {
                backend: Some(BackendKind::Apt),
                id: name.clone(),
                name,
                version,
                summary,
                installed: true,
                ..Default::default()
            })
            .collect();
        pkgs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(pkgs)
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        let opts = freshest_lists_opts();
        let mut argv: Vec<&str> = vec!["apt-get", "-s", "-o", "Debug::NoLocking=1"];
        argv.extend(opts.iter().map(String::as_str));
        argv.push("full-upgrade");
        let out = capture(&argv)?;
        let mut ups = parse_simulation(&out);
        let installed = dpkg_installed();
        for u in &mut ups {
            if let Some((_, _, summary)) = installed.get(&u.package.id) {
                u.package.summary = summary.clone();
            }
        }
        Ok(ups)
    }

    fn refresh(&self) -> Plan {
        Plan::new().root(&["apt-get", "update"])
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv = apt_root(&["install"]);
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv = apt_root(&["remove"]);
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        Plan::new().root_owned(apt_root(&["update"])).root_owned(apt_root(&["full-upgrade"]))
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        aptsources::list()
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        self.set_repos_enabled(std::slice::from_ref(repo), enabled)
    }

    fn set_repos_enabled(&self, repos: &[Repo], enabled: bool) -> anyhow::Result<Plan> {
        let mut plan = Plan::new();
        for (path, content) in aptsources::toggle_many(repos, enabled)? {
            plan = plan.write(&path, &content);
        }
        Ok(plan.root(&["apt-get", "update"]))
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        if repo.builtin {
            anyhow::bail!("dépôt fourni par la distribution ou par NOOS : désactivez-le plutôt que de le supprimer");
        }
        let idx = repo
            .id
            .rsplit(':')
            .next()
            .and_then(|n| n.trim_end_matches("-source").parse().ok())
            .ok_or_else(|| anyhow::anyhow!("identifiant de dépôt invalide : {}", repo.id))?;
        let text = std::fs::read_to_string(&repo.file)?;
        let plan = match aptsources::remove_entry(&repo.file, &text, idx) {
            Some(rest) => Plan::new().write(&repo.file, &rest),
            None => Plan::new().remove(&repo.file),
        };
        Ok(plan.root(&["apt-get", "update"]))
    }

    fn presets(&self) -> Vec<Preset> {
        apt_presets()
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        add_preset_plan(preset, argument)
    }

    fn appstream_ids(&self) -> HashMap<String, String> {
        self.appstream
            .get_or_init(|| {
                let mut map = HashMap::new();
                let arch = native_arch_cached();
                let Ok(rd) = std::fs::read_dir("/var/lib/apt/lists") else { return map };
                let mut files: Vec<PathBuf> = rd
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        n.contains("_dep11_Components-") && n.contains(&format!("-{arch}.yml"))
                    })
                    .collect();
                files.sort();
                for f in files {
                    if let Some(text) = read_maybe_compressed(&f) {
                        parse_dep11(&text, &mut map);
                    }
                }
                map
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        let out = "gimp:\n  Installed: (none)\n  Candidate: 3.0.4-1\n  Version table:\n     3.0.4-1 500\n        500 http://deb.debian.org/debian sid/main amd64 Packages\n\
                   noos:\n  Installed: 0.1.0~git1-1\n  Candidate: 0.1.0~git2-1\n  Version table:\n     0.1.0~git2-1 500\n        500 https://chomiam.github.io/noos-de sid/main amd64 Packages\n *** 0.1.0~git1-1 100\n        100 /var/lib/dpkg/status\n\
                   vlc:\n  Installed: 3.0.21-1\n  Candidate: 3.0.21-1\n  Version table:\n *** 3.0.21-1 500\n        500 http://deb.debian.org/debian sid/main amd64 Packages\n        100 /var/lib/dpkg/status\n";
        let p = parse_policy(out);
        assert_eq!(p["gimp"], Policy { installed: None, candidate: Some("3.0.4-1".into()), origin: "debian".into() });
        assert_eq!(p["noos"].origin, "noos");
        assert_eq!(p["noos"].installed.as_deref(), Some("0.1.0~git1-1"));
        assert_eq!(p["vlc"].origin, "debian");
    }

    #[test]
    fn search_and_simulation() {
        let s = parse_search("gimp - GNU Image Manipulation Program\ngimp-data - Data files for GIMP\n");
        assert_eq!(s[0], ("gimp".into(), "GNU Image Manipulation Program".into()));
        let sim = "NOTE: This is only a simulation!\nInst libfoo1 [1.0-1] (1.1-1 Debian:unstable [amd64])\nInst libnew2 (2.0-1 Debian:unstable [amd64])\nInst noos [0.1.0~git1-1] (0.1.0~git2-1 NOOS:sid [amd64])\nConf libfoo1 (1.1-1 Debian:unstable [amd64])\n";
        let u = parse_simulation(sim);
        assert_eq!(u.len(), 2);
        assert_eq!((u[0].old_version.as_str(), u[0].new_version.as_str()), ("1.0-1", "1.1-1"));
        assert_eq!(u[0].package.origin, "debian");
        assert_eq!(u[1].package.origin, "noos");
    }

    #[test]
    fn dep11() {
        let y = "---\nFile: DEP-11\nVersion: '1.0'\n---\nType: desktop-application\nID: org.gimp.GIMP\nPackage: gimp\nName:\n  C: GIMP\n---\nType: desktop-application\nID: vlc.desktop\nPackage: vlc\n";
        let mut m = HashMap::new();
        parse_dep11(y, &mut m);
        assert_eq!(m["gimp"], "org.gimp.gimp");
        assert_eq!(m["vlc"], "vlc");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn repo_argument() {
        let (u, s, c, k) = parse_repo_argument("https://repo.example.org/apt/ stable main extra clé=https://repo.example.org/key.asc").unwrap();
        assert_eq!((u.as_str(), s.as_str()), ("https://repo.example.org/apt", "stable"));
        assert_eq!(c, ["main", "extra"]);
        assert_eq!(k.as_deref(), Some("https://repo.example.org/key.asc"));
        assert!(parse_repo_argument("ftp://x stable").is_err());
        assert!(parse_repo_argument("https://x stable main clé=http://x/k").is_err());
        assert!(parse_repo_argument("https://x st;able").is_err());
    }

    #[test]
    fn origins() {
        assert_eq!(origin_of("Debian:unstable"), "debian");
        assert_eq!(origin_of("500 https://dl.example.com/linux stable/main amd64 Packages"), "dl.example.com");
        assert_eq!(origin_of("1 http://deb.debian.org/debian experimental/main amd64 Packages"), "debian-experimental");
    }
}

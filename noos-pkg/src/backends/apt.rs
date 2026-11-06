//! apt — Debian, Ubuntu et dérivées (Mint, Pop!_OS, Zorin, elementary…).
//!
//! Sources gérées dans les deux formats :
//! - une ligne : `/etc/apt/sources.list`, `sources.list.d/*.list` ;
//! - deb822 : `sources.list.d/*.sources` (par défaut depuis Debian 13 et Ubuntu 24.04).

use std::collections::{HashMap, HashSet};

use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, SystemInfo, Update,
    exec::{capture, have},
};

const SOURCES: &str = "/etc/apt/sources.list";
const SOURCES_D: &str = "/etc/apt/sources.list.d";
const OFFICIAL_HOSTS: &[&str] =
    &["deb.debian.org", "security.debian.org", "archive.ubuntu.com", "security.ubuntu.com", "ports.ubuntu.com", "packages.linuxmint.com"];

pub struct Apt {
    sys: SystemInfo,
}

impl Apt {
    pub fn new(sys: SystemInfo) -> Self {
        Self { sys }
    }

    fn apt_env() -> Vec<String> {
        vec!["env".into(), "DEBIAN_FRONTEND=noninteractive".into(), "apt-get".into()]
    }

    fn installed_map() -> HashMap<String, (String, String)> {
        capture(&["dpkg-query", "-W", "-f=${db:Status-Abbrev}\t${binary:Package}\t${Version}\t${binary:Summary}\n"])
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (c.len() >= 4 && c[0].starts_with("ii")).then(|| {
                    let name = c[1].split(':').next().unwrap_or(c[1]).to_string();
                    (name, (c[2].to_string(), c[3].to_string()))
                })
            })
            .collect()
    }
}

/// `apt-cache policy` → nom → (installé, candidat, origine).
pub fn parse_policy(out: &str) -> HashMap<String, (Option<String>, String, String)> {
    let mut map = HashMap::new();
    let mut name = String::new();
    let (mut inst, mut cand, mut origin) = (None, String::new(), String::new());
    let mut flush = |name: &str, inst: &mut Option<String>, cand: &mut String, origin: &mut String| {
        if !name.is_empty() {
            map.insert(name.to_string(), (inst.take(), std::mem::take(cand), std::mem::take(origin)));
        }
    };
    for line in out.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            flush(&name, &mut inst, &mut cand, &mut origin);
            name = line.trim_end_matches(':').to_string();
        } else if let Some(v) = line.trim().strip_prefix("Installed:") {
            let v = v.trim();
            inst = (v != "(none)").then(|| v.to_string());
        } else if let Some(v) = line.trim().strip_prefix("Candidate:") {
            cand = v.trim().to_string();
        } else if origin.is_empty() && line.contains("://") {
            // « 500 http://deb.debian.org/debian trixie/main amd64 Packages »
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                origin = parts[2].to_string();
            }
        }
    }
    flush(&name, &mut inst, &mut cand, &mut origin);
    map
}

/// `apt list --upgradable` → (nom, nouvelle version, ancienne version, suite).
pub fn parse_upgradable(out: &str) -> Vec<(String, String, String, String)> {
    out.lines()
        .filter(|l| l.contains("[upgradable from:"))
        .filter_map(|l| {
            let (head, rest) = l.split_once(' ')?;
            let (name, suite) = head.split_once('/')?;
            let new = rest.split_whitespace().next()?;
            let old = l.split("[upgradable from:").nth(1)?.trim().trim_end_matches(']').trim();
            Some((name.into(), new.into(), old.into(), suite.into()))
        })
        .collect()
}

// ───────────────────────────── Sources ─────────────────────────────

fn is_official(uri: &str) -> bool {
    OFFICIAL_HOSTS.iter().any(|h| uri.contains(h))
}

/// Entrées d'un fichier au format une ligne. Identifiant : `fichier:numéro de ligne`.
pub fn parse_one_line(path: &str, text: &str) -> Vec<Repo> {
    text.lines()
        .enumerate()
        .filter_map(|(i, raw)| {
            let line = raw.trim();
            let (enabled, body) = match line.strip_prefix('#') {
                Some(rest) => (false, rest.trim()),
                None => (true, line),
            };
            if !(body.starts_with("deb ") || body.starts_with("deb-src ")) {
                return None;
            }
            let mut tokens: Vec<&str> = body.split_whitespace().collect();
            let kind = tokens.remove(0);
            if tokens.first().is_some_and(|t| t.starts_with('[')) {
                while !tokens.is_empty() && !tokens[0].ends_with(']') {
                    tokens.remove(0);
                }
                if !tokens.is_empty() {
                    tokens.remove(0);
                }
            }
            let uri = tokens.first().copied().unwrap_or("").to_string();
            Some(Repo {
                backend: BackendKind::Apt,
                id: format!("{path}:{i}"),
                name: format!("{} {}", if kind == "deb-src" { "(sources)" } else { "" }, tokens.join(" ")).trim().to_string(),
                builtin: is_official(&uri),
                url: uri,
                enabled,
                scope: None,
                file: path.into(),
            })
        })
        .collect()
}

/// Strophes deb822. Identifiant : `fichier#index`.
pub fn parse_deb822(path: &str, text: &str) -> Vec<Repo> {
    stanzas(text)
        .into_iter()
        .enumerate()
        .filter_map(|(i, stanza)| {
            let field = |k: &str| {
                stanza.iter().find_map(|l| {
                    let (key, v) = l.split_once(':')?;
                    key.trim().eq_ignore_ascii_case(k).then(|| v.trim().to_string())
                })
            };
            let uris = field("URIs")?;
            let enabled = !field("Enabled").is_some_and(|v| v.eq_ignore_ascii_case("no"));
            let types = field("Types").unwrap_or_default();
            Some(Repo {
                backend: BackendKind::Apt,
                id: format!("{path}#{i}"),
                name: format!(
                    "{}{} {}",
                    if types.contains("deb-src") && !types.contains("deb ") && types.trim() == "deb-src" { "(sources) " } else { "" },
                    field("Suites").unwrap_or_default(),
                    field("Components").unwrap_or_default()
                )
                .trim()
                .to_string(),
                builtin: is_official(&uris),
                url: uris,
                enabled,
                scope: None,
                file: path.into(),
            })
        })
        .collect()
}

/// Découpe un fichier deb822 en strophes (lignes non vides consécutives, hors commentaires).
fn stanzas(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if !line.trim_start().starts_with('#') {
            cur.push(line.to_string());
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Réécrit la strophe `index` d'un fichier deb822 via `f` (None = supprimer la strophe).
pub fn edit_deb822(text: &str, index: usize, f: impl Fn(&[String]) -> Option<Vec<String>>) -> String {
    // On reconstruit en conservant les commentaires des strophes intactes.
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut cur = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                blocks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line.to_string());
        }
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    let mut stanza_idx = 0;
    let mut out: Vec<Vec<String>> = Vec::new();
    for block in blocks {
        let is_stanza = block.iter().any(|l| !l.trim_start().starts_with('#'));
        if is_stanza {
            if stanza_idx == index {
                if let Some(new) = f(&block) {
                    out.push(new);
                }
            } else {
                out.push(block);
            }
            stanza_idx += 1;
        } else {
            out.push(block);
        }
    }
    out.iter().map(|b| b.join("\n")).collect::<Vec<_>>().join("\n\n") + "\n"
}

pub fn set_field(stanza: &[String], key: &str, value: &str) -> Vec<String> {
    let mut found = false;
    let mut out: Vec<String> = stanza
        .iter()
        .map(|l| match l.split_once(':') {
            Some((k, _)) if k.trim().eq_ignore_ascii_case(key) => {
                found = true;
                format!("{key}: {value}")
            }
            _ => l.clone(),
        })
        .collect();
    if !found {
        out.push(format!("{key}: {value}"));
    }
    out
}

fn list_files() -> Vec<String> {
    let mut files = vec![];
    if std::path::Path::new(SOURCES).exists() {
        files.push(SOURCES.to_string());
    }
    if let Ok(rd) = std::fs::read_dir(SOURCES_D) {
        let mut v: Vec<String> = rd
            .filter_map(Result::ok)
            .map(|e| e.path().to_string_lossy().to_string())
            .filter(|p| p.ends_with(".list") || p.ends_with(".sources"))
            .collect();
        v.sort();
        files.extend(v);
    }
    files
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().trim_matches('-').to_string()
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
        let out = capture(&["apt-cache", "search", "--names-only", &q])?;
        let mut pkgs: Vec<Package> = out
            .lines()
            .filter_map(|l| l.split_once(" - "))
            .map(|(n, s)| Package {
                backend: Some(BackendKind::Apt),
                id: n.trim().into(),
                name: n.trim().into(),
                summary: s.trim().into(),
                ..Default::default()
            })
            .collect();
        crate::rank(&q, &mut pkgs);
        pkgs.truncate(80);
        // Versions et état d'installation en un seul appel.
        let names: Vec<&str> = pkgs.iter().map(|p| p.id.as_str()).collect();
        if !names.is_empty() {
            let mut argv = vec!["apt-cache", "policy"];
            argv.extend(&names);
            let policy = parse_policy(&capture(&argv).unwrap_or_default());
            for p in &mut pkgs {
                if let Some((inst, cand, origin)) = policy.get(&p.id) {
                    p.installed = inst.is_some();
                    p.version = inst.clone().unwrap_or_else(|| cand.clone());
                    p.origin = origin.clone();
                }
            }
        }
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        let manual: HashSet<String> = capture(&["apt-mark", "showmanual"])?
            .lines()
            .map(|l| l.split(':').next().unwrap_or(l).trim().to_string())
            .collect();
        let mut pkgs: Vec<Package> = Self::installed_map()
            .into_iter()
            .filter(|(n, _)| manual.contains(n))
            .map(|(n, (v, s))| Package {
                backend: Some(BackendKind::Apt),
                id: n.clone(),
                name: n,
                version: v,
                summary: s,
                installed: true,
                ..Default::default()
            })
            .collect();
        pkgs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(pkgs)
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        let out = capture(&["apt", "list", "--upgradable"])?;
        Ok(parse_upgradable(&out)
            .into_iter()
            .map(|(name, new, old, suite)| Update {
                package: Package {
                    backend: Some(BackendKind::Apt),
                    id: name.clone(),
                    name,
                    origin: suite,
                    installed: true,
                    ..Default::default()
                },
                old_version: old,
                new_version: new,
            })
            .collect())
    }

    fn refresh(&self) -> Plan {
        Plan::new().root(&["apt-get", "update"])
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv = Self::apt_env();
        argv.extend(["install".into(), "-y".into()]);
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv = Self::apt_env();
        argv.extend(["remove".into(), "-y".into()]);
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        let mut argv = Self::apt_env();
        argv.extend(["full-upgrade".into(), "-y".into()]);
        Plan::new().root(&["apt-get", "update"]).root_owned(argv)
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        let mut repos = Vec::new();
        for f in list_files() {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            repos.extend(if f.ends_with(".sources") { parse_deb822(&f, &text) } else { parse_one_line(&f, &text) });
        }
        Ok(repos)
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        let text = std::fs::read_to_string(&repo.file)?;
        let new = if let Some((_, idx)) = repo.id.rsplit_once('#').filter(|_| repo.file.ends_with(".sources")) {
            let idx: usize = idx.parse()?;
            edit_deb822(&text, idx, |s| Some(set_field(s, "Enabled", if enabled { "yes" } else { "no" })))
        } else {
            let line_no: usize = repo.id.rsplit_once(':').map(|(_, n)| n).unwrap_or("").parse()?;
            text.lines()
                .enumerate()
                .map(|(i, l)| {
                    if i != line_no {
                        return l.to_string();
                    }
                    let body = l.trim_start().trim_start_matches('#').trim_start();
                    if enabled { body.to_string() } else { format!("# {body}") }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        Ok(Plan::new().write(&repo.file, &new).root(&["apt-get", "update"]))
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        let text = std::fs::read_to_string(&repo.file)?;
        let siblings = self.repos()?.into_iter().filter(|r| r.file == repo.file).count();
        let plan = if siblings <= 1 && repo.file != SOURCES {
            Plan::new().remove(&repo.file)
        } else if repo.file.ends_with(".sources") {
            let idx: usize = repo.id.rsplit_once('#').map(|(_, n)| n).unwrap_or("").parse()?;
            Plan::new().write(&repo.file, &edit_deb822(&text, idx, |_| None))
        } else {
            let line_no: usize = repo.id.rsplit_once(':').map(|(_, n)| n).unwrap_or("").parse()?;
            let rest: Vec<&str> = text.lines().enumerate().filter(|(i, _)| *i != line_no).map(|(_, l)| l).collect();
            Plan::new().write(&repo.file, &(rest.join("\n") + "\n"))
        };
        Ok(plan.root(&["apt-get", "update"]))
    }

    fn presets(&self) -> Vec<Preset> {
        let mut p = Vec::new();
        if self.sys.is_ubuntu() {
            p.push(Preset {
                id: "ppa",
                backend: BackendKind::Apt,
                name: "PPA Launchpad",
                description: "Dépôt personnel Ubuntu (ex. mozillateam/ppa)",
                argument: Some("utilisateur/ppa"),
            });
            p.push(Preset {
                id: "universe",
                backend: BackendKind::Apt,
                name: "Universe et Multiverse",
                description: "Logiciels maintenus par la communauté et logiciels non libres",
                argument: None,
            });
        } else if self.sys.is("debian") {
            p.push(Preset {
                id: "contrib-nonfree",
                backend: BackendKind::Apt,
                name: "contrib, non-free et non-free-firmware",
                description: "Pilotes, firmwares et logiciels non libres (NVIDIA, Steam…)",
                argument: None,
            });
            if !self.sys.version_codename.is_empty() {
                p.push(Preset {
                    id: "backports",
                    backend: BackendKind::Apt,
                    name: "Debian Backports",
                    description: "Versions plus récentes recompilées pour la version stable",
                    argument: None,
                });
            }
        }
        p.push(Preset {
            id: "custom",
            backend: BackendKind::Apt,
            name: "Dépôt APT personnalisé",
            description: "URI, suite, composants et clé de signature",
            argument: Some("https://depot suite [composants] [cle=https://…/cle.asc]"),
        });
        p
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        let mut plan = Plan::new();
        let ensure_sp = |plan: Plan| {
            if have("add-apt-repository") {
                plan
            } else {
                let mut argv = Self::apt_env();
                argv.extend(["install".into(), "-y".into(), "software-properties-common".into()]);
                plan.root(&["apt-get", "update"]).root_owned(argv)
            }
        };
        match preset {
            "ppa" => {
                let arg = argument.unwrap_or("").trim().trim_start_matches("ppa:");
                if arg.split('/').count() != 2 || arg.contains(' ') {
                    anyhow::bail!("format attendu : utilisateur/ppa");
                }
                plan = ensure_sp(plan);
                Ok(plan.root_owned(vec!["add-apt-repository".into(), "-y".into(), format!("ppa:{arg}")]))
            }
            "universe" => {
                plan = ensure_sp(plan);
                Ok(plan
                    .root(&["add-apt-repository", "-y", "-n", "universe"])
                    .root(&["add-apt-repository", "-y", "-n", "multiverse"])
                    .root(&["apt-get", "update"]))
            }
            "contrib-nonfree" => {
                let mut wrote = false;
                for f in list_files() {
                    let text = std::fs::read_to_string(&f).unwrap_or_default();
                    let new = if f.ends_with(".sources") {
                        let mut t = text.clone();
                        for (i, r) in parse_deb822(&f, &text).iter().enumerate() {
                            if r.url.contains("debian.org") {
                                t = edit_deb822(&t, i, |s| {
                                    let comps = s
                                        .iter()
                                        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == "Components").map(|(_, v)| v.trim().to_string()))
                                        .unwrap_or_else(|| "main".into());
                                    Some(set_field(s, "Components", &add_components(&comps)))
                                });
                            }
                        }
                        t
                    } else {
                        text.lines()
                            .map(|l| {
                                let t = l.trim_start();
                                if (t.starts_with("deb ") || t.starts_with("deb-src ")) && t.contains("debian.org") {
                                    let parts: Vec<&str> = t.split_whitespace().collect();
                                    // deb [opts] uri suite comps…
                                    let comp_start = parts.iter().position(|p| p.contains("://")).map(|i| i + 2).unwrap_or(parts.len());
                                    let comps = parts[comp_start.min(parts.len())..].join(" ");
                                    format!("{} {}", parts[..comp_start.min(parts.len())].join(" "), add_components(&comps))
                                } else {
                                    l.to_string()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                            + "\n"
                    };
                    if new != text {
                        plan = plan.write(&f, &new);
                        wrote = true;
                    }
                }
                if !wrote {
                    anyhow::bail!("contrib et non-free sont déjà activés");
                }
                Ok(plan.root(&["apt-get", "update"]))
            }
            "backports" => {
                let code = &self.sys.version_codename;
                let content = format!(
                    "Types: deb\nURIs: http://deb.debian.org/debian\nSuites: {code}-backports\nComponents: main contrib non-free non-free-firmware\nSigned-By: /usr/share/keyrings/debian-archive-keyring.gpg\n"
                );
                Ok(plan.write(&format!("{SOURCES_D}/debian-backports.sources"), &content).root(&["apt-get", "update"]))
            }
            "custom" => {
                let arg = argument.unwrap_or("").trim();
                let mut key = None;
                let mut tokens = Vec::new();
                for t in arg.split_whitespace() {
                    match t.strip_prefix("cle=").or_else(|| t.strip_prefix("key=")) {
                        Some(k) => key = Some(k.to_string()),
                        None => tokens.push(t),
                    }
                }
                if tokens.len() < 2 || !tokens[0].starts_with("http") {
                    anyhow::bail!("format attendu : https://depot suite [composants] [cle=https://…/cle.asc]");
                }
                let uri = tokens[0];
                let suite = tokens[1];
                let comps = if tokens.len() > 2 { tokens[2..].join(" ") } else { "main".into() };
                let host = uri.split("://").nth(1).unwrap_or(uri).split('/').next().unwrap_or("depot");
                let name = format!("noos-{}", sanitize(host));
                let mut content = format!("Types: deb\nURIs: {uri}\nSuites: {suite}\n");
                if !suite.ends_with('/') {
                    content.push_str(&format!("Components: {comps}\n"));
                }
                if let Some(k) = key {
                    if !k.starts_with("https://") {
                        anyhow::bail!("la clé doit être téléchargée en https://");
                    }
                    let key_path = format!("/etc/apt/keyrings/{name}.asc");
                    plan = plan.root(&["mkdir", "-p", "/etc/apt/keyrings"]).root_owned(vec![
                        "curl".into(),
                        "-fsSL".into(),
                        "-o".into(),
                        key_path.clone(),
                        k,
                    ]);
                    content.push_str(&format!("Signed-By: {key_path}\n"));
                }
                Ok(plan.write(&format!("{SOURCES_D}/{name}.sources"), &content).root(&["apt-get", "update"]))
            }
            _ => anyhow::bail!("préréglage inconnu : {preset}"),
        }
    }
}

fn add_components(comps: &str) -> String {
    let mut list: Vec<String> = comps.split_whitespace().map(String::from).collect();
    for c in ["contrib", "non-free", "non-free-firmware"] {
        if !list.iter().any(|x| x == c) {
            list.push(c.into());
        }
    }
    list.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy() {
        let out = "htop:\n  Installed: (none)\n  Candidate: 3.4.1-5\n  Version table:\n     3.4.1-5 500\n        500 http://deb.debian.org/debian trixie/main amd64 Packages\nvim:\n  Installed: 2:9.1\n  Candidate: 2:9.1\n";
        let p = parse_policy(out);
        assert_eq!(p["htop"], (None, "3.4.1-5".into(), "trixie/main".into()));
        assert_eq!(p["vim"].0.as_deref(), Some("2:9.1"));
    }

    #[test]
    fn upgradable() {
        let out = "Listing...\nlibc6/noble-updates 2.39-0ubuntu8.4 amd64 [upgradable from: 2.39-0ubuntu8.3]\n";
        let u = parse_upgradable(out);
        assert_eq!(u[0], ("libc6".into(), "2.39-0ubuntu8.4".into(), "2.39-0ubuntu8.3".into(), "noble-updates".into()));
    }

    #[test]
    fn sources_formats() {
        let one = "deb http://deb.debian.org/debian trixie main\n# deb [arch=amd64] https://repo.example.com stable main\n# commentaire\n";
        let r = parse_one_line("/etc/apt/sources.list", one);
        assert_eq!(r.len(), 2);
        assert!(r[0].enabled && r[0].builtin);
        assert!(!r[1].enabled && !r[1].builtin);
        assert_eq!(r[1].url, "https://repo.example.com");

        let deb822 = "# Debian\nTypes: deb\nURIs: http://deb.debian.org/debian\nSuites: trixie trixie-updates\nComponents: main\n\nTypes: deb\nURIs: https://x.example\nSuites: stable\nComponents: main\nEnabled: no\n";
        let r = parse_deb822("/etc/apt/sources.list.d/debian.sources", deb822);
        assert_eq!(r.len(), 2);
        assert!(r[0].enabled && !r[1].enabled);
        let t = edit_deb822(deb822, 1, |s| Some(set_field(s, "Enabled", "yes")));
        assert!(parse_deb822("x.sources", &t)[1].enabled);
        assert!(t.starts_with("# Debian"));
        let t = edit_deb822(deb822, 0, |_| None);
        assert_eq!(parse_deb822("x.sources", &t).len(), 1);
    }

    #[test]
    fn components() {
        assert_eq!(add_components("main"), "main contrib non-free non-free-firmware");
        assert_eq!(add_components("main contrib"), "main contrib non-free non-free-firmware");
    }
}

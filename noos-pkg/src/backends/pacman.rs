//! pacman — Arch Linux et dérivées (EndeavourOS, Manjaro, CachyOS, Garuda…).
//!
//! Règle d'or d'Arch : jamais de `pacman -Sy` seul (mise à jour partielle). Les installations
//! se font donc avec `-Syu --needed`, et la liste des mises à jour utilise `checkupdates`
//! (pacman-contrib) qui synchronise une copie temporaire de la base.

use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, SystemInfo, Update,
    exec::{capture_codes, have},
};

pub const CONF: &str = "/etc/pacman.conf";
const OFFICIAL: &[&str] =
    &["core", "extra", "multilib", "core-testing", "extra-testing", "multilib-testing", "community", "testing", "gnome-unstable", "kde-unstable"];
const CHAOTIC_KEY: &str = "3056513887B78AEB";

pub struct Pacman {
    _sys: SystemInfo,
}

impl Pacman {
    pub fn new(sys: SystemInfo) -> Self {
        Self { _sys: sys }
    }
}

/// Sortie `pacman -Ss` / `paru -Ss` : « repo/nom version [installé] » + ligne de description.
pub fn parse_ss(out: &str, backend: BackendKind) -> Vec<Package> {
    let mut pkgs = Vec::new();
    let mut lines = out.lines().peekable();
    while let Some(head) = lines.next() {
        if head.starts_with(' ') || head.is_empty() {
            continue;
        }
        let mut parts = head.split_whitespace();
        let Some((repo, name)) = parts.next().and_then(|r| r.split_once('/')) else { continue };
        let version = parts.next().unwrap_or("").to_string();
        let installed = head.contains("[installed") || head.contains("[Installed") || head.contains("[installé");
        let summary = match lines.peek() {
            Some(l) if l.starts_with(' ') => lines.next().unwrap().trim().to_string(),
            _ => String::new(),
        };
        pkgs.push(Package {
            backend: Some(backend),
            id: name.into(),
            name: name.into(),
            version,
            summary,
            origin: repo.into(),
            installed,
            scope: None,
        });
    }
    pkgs
}

/// Blocs `pacman -Qi` (« Clé : valeur »).
pub fn parse_qi(out: &str) -> Vec<Package> {
    out.split("\n\n")
        .filter_map(|block| {
            let get = |k: &str| {
                block.lines().find_map(|l| {
                    let (key, v) = l.split_once(':')?;
                    (key.trim() == k).then(|| v.trim().to_string())
                })
            };
            let name = get("Name")?;
            Some(Package {
                backend: Some(BackendKind::Pacman),
                id: name.clone(),
                name,
                version: get("Version").unwrap_or_default(),
                summary: get("Description").unwrap_or_default(),
                installed: true,
                ..Default::default()
            })
        })
        .collect()
}

/// « nom ancienne -> nouvelle » (checkupdates, paru -Qua).
pub fn parse_arrow_updates(out: &str, backend: BackendKind) -> Vec<Update> {
    out.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() >= 4 && p[2] == "->").then(|| Update {
                package: Package {
                    backend: Some(backend),
                    id: p[0].into(),
                    name: p[0].into(),
                    installed: true,
                    ..Default::default()
                },
                old_version: p[1].into(),
                new_version: p[3].into(),
            })
        })
        .collect()
}

/// Sections de pacman.conf, y compris commentées (`#[multilib]` = désactivée).
pub fn parse_conf(text: &str) -> Vec<Repo> {
    let mut repos: Vec<Repo> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let (enabled, body) = match t.strip_prefix('#') {
            Some(rest) => (false, rest.trim()),
            None => (true, t),
        };
        if let Some(name) = body.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
            // Les sections commentées ne comptent que pour les dépôts officiels
            // (pacman.conf contient des exemples « #[custom] », « #[repo-name] »).
            if name == "options" || name.contains(' ') || (!enabled && !OFFICIAL.contains(&name)) {
                continue;
            }
            repos.push(Repo {
                backend: BackendKind::Pacman,
                id: name.into(),
                name: name.into(),
                url: String::new(),
                enabled,
                scope: None,
                file: CONF.into(),
                builtin: OFFICIAL.contains(&name),
            });
        } else if let Some(last) = repos.last_mut() {
            if last.url.is_empty() && enabled == last.enabled {
                if let Some((k, v)) = body.split_once('=') {
                    if matches!(k.trim(), "Server" | "Include") {
                        last.url = v.trim().to_string();
                    }
                }
            }
        }
    }
    repos
}

/// (Dé)commente la section `[name]` et ses lignes jusqu'à la section suivante.
pub fn toggle_section(text: &str, name: &str, enabled: bool) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        let uncommented = t.trim_start_matches('#').trim();
        let is_header = uncommented.starts_with('[') && uncommented.ends_with(']');
        if is_header {
            inside = uncommented == format!("[{name}]");
        }
        if inside && !uncommented.is_empty() && (is_header || uncommented.contains('=')) {
            out.push(if enabled { uncommented.to_string() } else { format!("#{uncommented}") });
        } else {
            out.push(line.to_string());
        }
    }
    out.join("\n") + "\n"
}

pub fn remove_section(text: &str, name: &str) -> String {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let u = line.trim().trim_start_matches('#').trim();
        if u.starts_with('[') && u.ends_with(']') {
            inside = u == format!("[{name}]");
        }
        if !inside {
            out.push(line);
        }
    }
    out.join("\n") + "\n"
}

/// Trousseau pacman initialisé ? (clé maître locale nécessaire pour `pacman-key --lsign-key`)
fn keyring_plan() -> Plan {
    let ready = std::fs::read_dir("/etc/pacman.d/gnupg/private-keys-v1.d").is_ok_and(|mut d| d.next().is_some());
    if ready {
        Plan::new()
    } else {
        Plan::new().root(&["pacman-key", "--init"]).root(&["pacman-key", "--populate"])
    }
}

impl Backend for Pacman {
    fn kind(&self) -> BackendKind {
        BackendKind::Pacman
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim().to_lowercase();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        // Code 1 = aucun résultat.
        let out = capture_codes(&["pacman", "-Ss", &q], &[0, 1])?;
        let mut pkgs = parse_ss(&out, BackendKind::Pacman);
        crate::rank(&q, &mut pkgs);
        pkgs.truncate(150);
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        // -Qie : installés explicitement ; -n : seulement ceux des dépôts (l'AUR est à part).
        Ok(parse_qi(&capture_codes(&["pacman", "-Qien"], &[0, 1])?))
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        if !have("checkupdates") || !have("fakeroot") {
            anyhow::bail!("installez pacman-contrib et fakeroot pour lister les mises à jour sans risque de mise à jour partielle");
        }
        // Code 2 = aucune mise à jour.
        Ok(parse_arrow_updates(&capture_codes(&["checkupdates"], &[0, 2])?, BackendKind::Pacman))
    }

    fn refresh(&self) -> Plan {
        // Pas de « pacman -Sy » isolé (mise à jour partielle) : checkupdates suffit.
        Plan::new()
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> = ["pacman", "-Syu", "--needed", "--noconfirm"].map(String::from).to_vec();
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> = ["pacman", "-Rns", "--noconfirm"].map(String::from).to_vec();
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        Plan::new().root(&["pacman", "-Syu", "--noconfirm"])
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        Ok(parse_conf(&std::fs::read_to_string(CONF)?))
    }

    fn set_repo_enabled(&self, repo: &Repo, enabled: bool) -> anyhow::Result<Plan> {
        let text = std::fs::read_to_string(CONF)?;
        let mut plan = Plan::new().write(CONF, &toggle_section(&text, &repo.id, enabled));
        if enabled {
            plan = plan.root(&["pacman", "-Syu", "--noconfirm"]);
        }
        Ok(plan)
    }

    fn remove_repo(&self, repo: &Repo) -> anyhow::Result<Plan> {
        let text = std::fs::read_to_string(CONF)?;
        Ok(Plan::new().write(CONF, &remove_section(&text, &repo.id)))
    }

    fn presets(&self) -> Vec<Preset> {
        let mut p = Vec::new();
        let conf = std::fs::read_to_string(CONF).unwrap_or_default();
        if !parse_conf(&conf).iter().any(|r| r.id == "multilib" && r.enabled) {
            p.push(Preset {
                id: "multilib",
                backend: BackendKind::Pacman,
                name: "multilib",
                description: "Bibliothèques 32 bits (Steam, Wine, Proton)",
                argument: None,
            });
        }
        p.push(Preset {
            id: "chaotic-aur",
            backend: BackendKind::Pacman,
            name: "Chaotic-AUR",
            description: "Paquets AUR précompilés (binaires, mis à jour automatiquement)",
            argument: None,
        });
        if !have("checkupdates") {
            p.push(Preset {
                id: "pacman-contrib",
                backend: BackendKind::Pacman,
                name: "Outils pacman-contrib",
                description: "Nécessaire pour vérifier les mises à jour sans risque (checkupdates)",
                argument: None,
            });
        }
        p.push(Preset {
            id: "custom",
            backend: BackendKind::Pacman,
            name: "Dépôt pacman personnalisé",
            description: "Nom, serveur et empreinte de clé facultative",
            argument: Some("nom https://serveur/$repo/$arch [cle=EMPREINTE]"),
        });
        p
    }

    fn add_preset(&self, preset: &str, argument: Option<&str>) -> anyhow::Result<Plan> {
        let text = std::fs::read_to_string(CONF)?;
        let repos = parse_conf(&text);
        match preset {
            "multilib" => match repos.iter().find(|r| r.id == "multilib") {
                Some(r) if r.enabled => anyhow::bail!("multilib est déjà activé"),
                Some(r) => self.set_repo_enabled(r, true),
                // Absente (ex. image Docker) : on l'ajoute.
                None => {
                    let conf = format!("{}\n\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n", text.trim_end());
                    Ok(Plan::new().write(CONF, &conf).root(&["pacman", "-Syu", "--noconfirm"]))
                }
            },
            // checkupdates a besoin de fakeroot (fourni par base-devel, souvent absent).
            "pacman-contrib" => Ok(Plan::new().root(&["pacman", "-Syu", "--needed", "--noconfirm", "pacman-contrib", "fakeroot"])),
            "chaotic-aur" => {
                if repos.iter().any(|r| r.id == "chaotic-aur") {
                    anyhow::bail!("Chaotic-AUR est déjà configuré");
                }
                let conf = format!("{}\n[chaotic-aur]\nInclude = /etc/pacman.d/chaotic-mirrorlist\n", text.trim_end());
                Ok(keyring_plan()
                    .root(&["pacman-key", "--recv-key", CHAOTIC_KEY, "--keyserver", "keyserver.ubuntu.com"])
                    .root(&["pacman-key", "--lsign-key", CHAOTIC_KEY])
                    .root(&[
                        "pacman",
                        "-U",
                        "--noconfirm",
                        "https://cdn-mirror.chaotic.cx/chaotic-aur/chaotic-keyring.pkg.tar.zst",
                        "https://cdn-mirror.chaotic.cx/chaotic-aur/chaotic-mirrorlist.pkg.tar.zst",
                    ])
                    .write(CONF, &conf)
                    .root(&["pacman", "-Syu", "--noconfirm"]))
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
                let [name, server] = tokens.as_slice() else {
                    anyhow::bail!("format attendu : nom https://serveur/$repo/$arch [cle=EMPREINTE]");
                };
                if !name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) || !server.starts_with("https://") {
                    anyhow::bail!("nom invalide ou serveur non https");
                }
                if repos.iter().any(|r| r.id == *name) {
                    anyhow::bail!("le dépôt {name} existe déjà");
                }
                let mut plan = Plan::new();
                if let Some(k) = &key {
                    if !k.chars().all(|c| c.is_ascii_hexdigit()) {
                        anyhow::bail!("empreinte de clé invalide");
                    }
                    plan = keyring_plan()
                        .root(&["pacman-key", "--recv-key", k, "--keyserver", "keyserver.ubuntu.com"])
                        .root(&["pacman-key", "--lsign-key", k]);
                }
                let conf = format!("{}\n[{name}]\nServer = {server}\n", text.trim_end());
                Ok(plan.write(CONF, &conf).root(&["pacman", "-Syu", "--noconfirm"]))
            }
            _ => anyhow::bail!("préréglage inconnu : {preset}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF_SAMPLE: &str = "[options]\nHoldPkg = pacman\n#       [repo-name]\n\n[core]\nInclude = /etc/pacman.d/mirrorlist\n\n[extra]\nInclude = /etc/pacman.d/mirrorlist\n\n#[multilib]\n#Include = /etc/pacman.d/mirrorlist\n\n#[custom]\n#Server = file:///home/custompkgs\n";

    #[test]
    fn conf() {
        let r = parse_conf(CONF_SAMPLE);
        assert_eq!(r.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["core", "extra", "multilib"]);
        assert!(!r[2].enabled);
        assert_eq!(r[2].url, "/etc/pacman.d/mirrorlist");
        let on = toggle_section(CONF_SAMPLE, "multilib", true);
        assert!(on.contains("\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n"));
        assert!(parse_conf(&on)[2].enabled);
        let off = toggle_section(&on, "multilib", false);
        assert!(!parse_conf(&off)[2].enabled);
        assert!(!remove_section(CONF_SAMPLE, "extra").contains("[extra]"));
    }

    #[test]
    fn ss() {
        let out = "extra/htop 3.4.1-1 [installed]\n    Interactive process viewer\nextra/btop 1.4.0-1\n    Resource monitor\n";
        let p = parse_ss(out, BackendKind::Pacman);
        assert_eq!(p.len(), 2);
        assert!(p[0].installed && !p[1].installed);
        assert_eq!(p[1].summary, "Resource monitor");
        assert_eq!(p[0].origin, "extra");
    }

    #[test]
    fn arrows() {
        let u = parse_arrow_updates("linux 6.10.1-1 -> 6.10.2-1\n", BackendKind::Pacman);
        assert_eq!(u[0].new_version, "6.10.2-1");
    }
}

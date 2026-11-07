//! Fichiers `/etc/yum.repos.d/*.repo` (format INI) et URL des dépôts tiers courants
//! (RPM Fusion, COPR).

use crate::{BackendKind, Repo, SystemInfo};

pub const REPO_DIR: &str = "/etc/yum.repos.d";
/// Dépôts fournis par la distribution (Fedora ≥ 42, lecture seule).
pub const VENDOR_DIR: &str = "/usr/share/dnf5/repos.d";
/// Surcharges (enabled=…) écrites par dnf5 config-manager et GNOME Logiciels.
pub const OVERRIDE_DIR: &str = "/etc/dnf/repos.override.d";
pub const OVERRIDE_FILE: &str = "/etc/dnf/repos.override.d/99-config_manager.repo";

/// Fichiers fournis par Fedora elle-même.
const BUILTIN_PREFIXES: &[&str] = &["fedora"];

pub fn parse_file(backend: BackendKind, path: &str, text: &str) -> Vec<Repo> {
    let file_name = std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let builtin = BUILTIN_PREFIXES.iter().any(|p| file_name.starts_with(p));
    let mut repos = Vec::new();
    let mut current: Option<Repo> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some(r) = current.take() {
                repos.push(r);
            }
            current = Some(Repo {
                backend,
                id: section.to_string(),
                name: section.to_string(),
                url: String::new(),
                enabled: true,
                scope: None,
                file: path.to_string(),
                builtin,
            });
            continue;
        }
        let Some(r) = current.as_mut() else { continue };
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "name" => r.name = v.to_string(),
            "enabled" => r.enabled = !matches!(v, "0" | "false" | "no" | "False"),
            "baseurl" | "metalink" | "mirrorlist" if r.url.is_empty() => {
                r.url = v.split_whitespace().next().unwrap_or("").to_string()
            }
            _ => {}
        }
    }
    if let Some(r) = current {
        repos.push(r);
    }
    repos
}

fn repo_files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "repo")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Tous les dépôts : /etc/yum.repos.d (prioritaire, même nom de fichier) + dépôts de la
/// distribution, puis application des surcharges de /etc/dnf/repos.override.d.
pub fn list(backend: BackendKind) -> anyhow::Result<Vec<Repo>> {
    let mut repos = Vec::new();
    let etc = repo_files(REPO_DIR);
    let etc_names: Vec<_> = etc.iter().filter_map(|p| p.file_name().map(|f| f.to_owned())).collect();
    for path in etc {
        repos.extend(parse_file(backend, &path.to_string_lossy(), &std::fs::read_to_string(&path).unwrap_or_default()));
    }
    for path in repo_files(VENDOR_DIR) {
        if path.file_name().is_some_and(|f| etc_names.iter().any(|e| e == f)) {
            continue;
        }
        // builtin selon le nom de fichier : rpmfusion-*.repo (paquet tiers) reste supprimable.
        repos.extend(parse_file(backend, &path.to_string_lossy(), &std::fs::read_to_string(&path).unwrap_or_default()));
    }
    for path in repo_files(OVERRIDE_DIR) {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        for o in parse_file(backend, &path.to_string_lossy(), &text) {
            if !text.contains(&format!("[{}]", o.id)) || !section_has_key(&text, &o.id, "enabled") {
                continue;
            }
            if let Some(r) = repos.iter_mut().find(|r| r.id == o.id) {
                r.enabled = o.enabled;
            }
        }
    }
    Ok(repos)
}

fn section_has_key(text: &str, id: &str, key: &str) -> bool {
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            inside = t == format!("[{id}]");
        } else if inside && t.split('=').next().map(str::trim) == Some(key) {
            return true;
        }
    }
    false
}

/// Contenus à écrire pour activer / désactiver des dépôts (chemin → contenu complet) :
/// - fichier de /etc/yum.repos.d sans surcharge existante → modification du fichier ;
/// - sinon → surcharge dans 99-config_manager.repo (comme `dnf config-manager setopt`).
///
/// Plusieurs dépôts d'un même fichier sont appliqués à la suite sur le même texte.
pub fn toggle_many(repos: &[Repo], enabled: bool) -> anyhow::Result<Vec<(String, String)>> {
    let mut files: Vec<(String, String)> = Vec::new();
    let read = |files: &Vec<(String, String)>, path: &str| -> String {
        files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| std::fs::read_to_string(path).unwrap_or_default())
    };
    let put = |files: &mut Vec<(String, String)>, path: &str, text: String| match files.iter_mut().find(|(p, _)| p == path) {
        Some(e) => e.1 = text,
        None => files.push((path.to_string(), text)),
    };
    for repo in repos {
        let override_text = read(&files, OVERRIDE_FILE);
        let overridden = section_has_key(&override_text, &repo.id, "enabled");
        if repo.file.starts_with(REPO_DIR) && !overridden {
            let text = read(&files, &repo.file);
            if text.is_empty() {
                anyhow::bail!("lecture de {} impossible", repo.file);
            }
            put(&mut files, &repo.file, set_enabled(&text, &repo.id, enabled));
            continue;
        }
        let base = if override_text.trim().is_empty() {
            "# Generated by libdnf.\n# Do not modify this file manually.\n".to_string()
        } else {
            override_text
        };
        let new = if base.contains(&format!("[{}]", repo.id)) {
            set_enabled(&base, &repo.id, enabled)
        } else {
            let base = if base.ends_with('\n') { base } else { format!("{base}\n") };
            format!("{base}[{}]\nenabled={}\n", repo.id, if enabled { 1 } else { 0 })
        };
        put(&mut files, OVERRIDE_FILE, new);
    }
    Ok(files)
}

/// Réécrit `enabled=` dans la section `[id]` (ajoute la ligne si absente).
pub fn set_enabled(text: &str, id: &str, enabled: bool) -> String {
    let value = if enabled { "1" } else { "0" };
    let mut out = Vec::new();
    let mut in_section = false;
    let mut done = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_section && !done {
                out.push(format!("enabled={value}"));
                done = true;
            }
            in_section = trimmed == format!("[{id}]");
        } else if in_section && !done {
            let key = trimmed.split('=').next().unwrap_or("").trim();
            if key == "enabled" {
                out.push(format!("enabled={value}"));
                done = true;
                continue;
            }
        }
        out.push(line.to_string());
    }
    if in_section && !done {
        out.push(format!("enabled={value}"));
    }
    out.join("\n") + "\n"
}

/// Retire la section `[id]`. Renvoie `None` si le fichier devient vide.
pub fn remove_section(text: &str, id: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            skipping = trimmed == format!("[{id}]");
        }
        if !skipping {
            out.push(line);
        }
    }
    let has_section = out.iter().any(|l| l.trim().starts_with('['));
    has_section.then(|| out.join("\n") + "\n")
}

/// Paquet RPM propriétaire d'un fichier (ex. `rpmfusion-free-release`), s'il y en a un.
pub fn owner_package(path: &str) -> Option<String> {
    let out = crate::exec::capture(&["rpm", "-qf", "--qf", "%{NAME}", path]).ok()?;
    let name = out.trim();
    (!name.is_empty() && !name.contains(' ')).then(|| name.to_string())
}

/// Paquet de configuration RPM Fusion (`free` ou `nonfree`).
pub fn rpmfusion_url(sys: &SystemInfo, nonfree: bool) -> String {
    let (kind, ver) = (if nonfree { "nonfree" } else { "free" }, sys.major());
    format!("https://mirrors.rpmfusion.org/{kind}/fedora/rpmfusion-{kind}-release-{ver}.noarch.rpm")
}

/// `owner/projet` (ou `@groupe/projet`) → (URL du .repo, chemin local).
pub fn copr(sys: &SystemInfo, spec: &str) -> anyhow::Result<(String, String)> {
    let spec = spec.trim().trim_start_matches("copr:");
    let (owner, project) = spec
        .split_once('/')
        .filter(|(o, p)| !o.is_empty() && !p.is_empty() && !p.contains('/'))
        .ok_or_else(|| anyhow::anyhow!("format attendu : propriétaire/projet (ex. atim/starship)"))?;
    let chroot = format!("fedora-{}", sys.major());
    let (url_owner, file_owner) = match owner.strip_prefix('@') {
        Some(group) => (format!("g/{group}"), format!("group_{group}")),
        None => (owner.to_string(), owner.to_string()),
    };
    let url = format!(
        "https://copr.fedorainfracloud.org/coprs/{url_owner}/{project}/repo/{chroot}/{file_owner}-{project}-{chroot}.repo"
    );
    let path = format!("{REPO_DIR}/_copr:copr.fedorainfracloud.org:{file_owner}:{project}.repo");
    Ok((url, path))
}

/// Nom de fichier local pour un .repo téléchargé depuis une URL.
pub fn repo_path_for_url(url: &str) -> anyhow::Result<String> {
    let name = url.rsplit('/').next().unwrap_or("");
    let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).collect();
    if !name.ends_with(".repo") || name.len() < 6 {
        anyhow::bail!("l'URL doit pointer vers un fichier .repo");
    }
    Ok(format!("{REPO_DIR}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[updates]\nname=Fedora $releasever - Updates\nmetalink=https://mirrors.fedoraproject.org/metalink?repo=updates\nenabled=1\n\n[updates-debuginfo]\nname=Debug\nenabled=0\n";

    #[test]
    fn parse() {
        let r = parse_file(BackendKind::Dnf, "/etc/yum.repos.d/fedora-updates.repo", SAMPLE);
        assert_eq!(r.len(), 2);
        assert!(r[0].enabled && r[0].builtin);
        assert!(!r[1].enabled);
        assert!(r[0].url.starts_with("https://"));
    }

    #[test]
    fn toggle() {
        let t = set_enabled(SAMPLE, "updates-debuginfo", true);
        let r = parse_file(BackendKind::Dnf, "x.repo", &t);
        assert!(r[1].enabled);
        let t = set_enabled("[a]\nname=A\n[b]\nname=B\n", "a", false);
        assert!(t.contains("[a]\nname=A\nenabled=0\n[b]"));
    }

    #[test]
    fn rpmfusion() {
        let sys = SystemInfo::parse("ID=fedora\nVERSION_ID=45\n");
        assert_eq!(rpmfusion_url(&sys, true), "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-45.noarch.rpm");
    }

    #[test]
    fn remove() {
        assert!(remove_section(SAMPLE, "updates").unwrap().contains("[updates-debuginfo]"));
        assert!(remove_section("[x]\na=b\n", "x").is_none());
    }

    #[test]
    fn override_section_detection() {
        let t = "# Generated by libdnf.\n[google-chrome]\nenabled=1\n[x]\npriority=5\n";
        assert!(section_has_key(t, "google-chrome", "enabled"));
        assert!(!section_has_key(t, "x", "enabled"));
    }

    #[test]
    fn copr_urls() {
        let sys = SystemInfo::parse("ID=fedora\nVERSION_ID=45\n");
        let (url, path) = copr(&sys, "atim/starship").unwrap();
        assert_eq!(url, "https://copr.fedorainfracloud.org/coprs/atim/starship/repo/fedora-45/atim-starship-fedora-45.repo");
        assert!(path.ends_with("_copr:copr.fedorainfracloud.org:atim:starship.repo"));
        let (url, _) = copr(&sys, "@cosmic/nightly").unwrap();
        assert!(url.contains("/coprs/g/cosmic/nightly/repo/fedora-45/group_cosmic-nightly-fedora-45.repo"));
        assert!(copr(&sys, "pasbon").is_err());
    }
}

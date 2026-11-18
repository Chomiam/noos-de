//! Fichiers `.deb` téléchargés : lecture (`dpkg-deb -f`), simulation sans droits
//! (`apt-get -s install <fichier>`, dépendances résolues depuis les dépôts) et paquet à
//! installer par le backend apt (`apt-get install <chemin>` : apt accepte un chemin de
//! fichier à la place d'un nom, d'où le chemin absolu dans `Package::id`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{BackendKind, Package, Plan, exec::capture_codes};

/// Types MIME (shared-mime-info) des paquets Debian.
pub const MIME_TYPES: [&str; 2] = ["application/vnd.debian.binary-package", "application/x-deb"];

#[derive(Debug, Clone, Default)]
pub struct DebInfo {
    pub path: PathBuf,
    pub package: String,
    pub version: String,
    pub architecture: String,
    /// Première ligne de la description.
    pub summary: String,
    /// Suite de la description (paragraphes, « . » = ligne vide).
    pub description: String,
    pub maintainer: String,
    pub homepage: Option<String>,
    /// Taille une fois installé, en octets.
    pub installed_size: u64,
    /// Version déjà installée, s'il y en a une.
    pub installed_version: Option<String>,
}

/// Résultat de la simulation `apt-get -s install` (dépendances ajoutées, mises à jour, retraits).
pub type Simulation = crate::details::Effect;

/// Le fichier ressemble-t-il à un paquet Debian (archive ar « debian-binary ») ?
pub fn is_deb(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 21];
    path.is_file()
        && std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok()
        && &head[..8] == b"!<arch>\n"
        && &head[8..21] == b"debian-binary"
}

/// Champs de contrôle « Clé: valeur » (lignes de continuation commençant par une espace).
fn parse_control(text: &str) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = fields.last_mut() {
                v.push('\n');
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            fields.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    fields
}

/// Description Debian : 1re ligne = résumé ; ensuite « . » seul = paragraphe vide.
fn split_description(raw: &str) -> (String, String) {
    let mut lines = raw.lines();
    let summary = lines.next().unwrap_or("").trim().to_string();
    let body: Vec<&str> = lines.map(|l| if l.trim() == "." { "" } else { l.trim() }).collect();
    (summary, body.join("\n").trim().to_string())
}

pub fn inspect(path: &Path) -> Result<DebInfo> {
    let path = path.canonicalize().with_context(|| format!("{} introuvable", path.display()))?;
    if !is_deb(&path) {
        bail!("{} n'est pas un paquet Debian", path.display());
    }
    let p = path.to_string_lossy();
    let out = capture_codes(&["dpkg-deb", "-f", &p], &[0]).context("lecture du paquet")?;
    let fields = parse_control(&out);
    let get = |k: &str| fields.iter().find(|(f, _)| f.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let package = get("Package").filter(|s| !s.is_empty()).context("champ Package absent")?;
    let (summary, description) = split_description(&get("Description").unwrap_or_default());
    let installed_version = capture_codes(&["dpkg-query", "-W", "-f", "${db:Status-Abbrev}|${Version}", &package], &[0, 1])
        .ok()
        .and_then(|o| {
            let (status, v) = o.split_once('|')?;
            status.starts_with("ii").then(|| v.trim().to_string())
        });
    Ok(DebInfo {
        version: get("Version").unwrap_or_default(),
        architecture: get("Architecture").unwrap_or_default(),
        summary,
        description,
        maintainer: get("Maintainer").unwrap_or_default(),
        homepage: get("Homepage").filter(|h| !h.is_empty()),
        installed_size: get("Installed-Size").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) * 1024,
        installed_version,
        package,
        path,
    })
}

/// Simule l'installation sans droits root : dépendances à ajouter, ou erreur lisible
/// (dépendances introuvables, mauvaise architecture…).
pub fn simulate(info: &DebInfo) -> Result<Simulation> {
    let opts = crate::backends::apt::freshest_lists_opts();
    let path = info.path.to_string_lossy();
    let mut argv: Vec<&str> = vec!["apt-get", "-s", "-o", "Debug::NoLocking=1"];
    argv.extend(opts.iter().map(String::as_str));
    argv.extend(["install", &path]);
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .output()
        .context("apt-get introuvable")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        // apt explique le problème sur stdout (« The following packages have unmet
        // dependencies ») puis résume sur stderr (« E: … »).
        bail!("{}", crate::details::apt_refusal(&stdout, &stderr));
    }
    Ok(crate::details::parse_apt_simulation(&stdout, &info.package))
}

/// Plan d'installation (pkexec) : `apt-get install <chemin>`, avec `--reinstall` si la même
/// version est déjà installée (sinon apt ne fait rien).
pub fn install_plan(info: &DebInfo) -> Plan {
    let path = info.path.to_string_lossy();
    let reinstall = info.installed_version.as_deref() == Some(info.version.as_str());
    let args: Vec<&str> = if reinstall { vec!["install", "--reinstall", &path] } else { vec!["install", &path] };
    Plan::new().root_owned(crate::backends::apt::apt_root(&args))
}

/// Paquet équivalent (affichage, journal) : `id` = chemin du fichier.
pub fn package(info: &DebInfo) -> Package {
    Package {
        backend: Some(BackendKind::Apt),
        id: info.path.to_string_lossy().into_owned(),
        name: info.package.clone(),
        version: info.version.clone(),
        summary: info.summary.clone(),
        origin: "Fichier .deb".into(),
        installed: false,
        scope: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn champs_et_description() {
        let f = parse_control(
            "Package: exemple\nVersion: 1.2-3\nDescription: Outil d'exemple\n Première ligne.\n .\n Second paragraphe.\nArchitecture: amd64\n",
        );
        assert_eq!(f[0], ("Package".into(), "exemple".into()));
        let desc = &f.iter().find(|(k, _)| k == "Description").unwrap().1;
        let (s, d) = split_description(desc);
        assert_eq!(s, "Outil d'exemple");
        assert_eq!(d, "Première ligne.\n\nSecond paragraphe.");
        assert_eq!(f.last().unwrap(), &("Architecture".into(), "amd64".into()));
    }

    #[test]
    fn signature_deb() {
        let dir = std::env::temp_dir().join(format!("noos-deb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.deb");
        std::fs::write(&f, b"!<arch>\ndebian-binary   1342943816  0     0     100644  4         `\n2.0\n").unwrap();
        assert!(is_deb(&f));
        std::fs::write(&f, b"!<arch>\nautre-chose----").unwrap();
        assert!(!is_deb(&f));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

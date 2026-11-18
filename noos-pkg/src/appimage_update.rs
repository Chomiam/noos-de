//! Mises à jour des AppImage : informations intégrées par l'éditeur dans la section ELF
//! `.upd_info` (spécification AppImage), lues sans exécuter le fichier.
//!
//! | Information                                          | Recherche                                |
//! |------------------------------------------------------|------------------------------------------|
//! | `zsync|<URL du .zsync>`                              | en-tête du fichier .zsync                |
//! | `gh-releases-zsync|<compte>|<dépôt>|<version>|<motif>` | API GitHub (versions) puis .zsync       |
//!
//! L'en-tête zsync donne l'empreinte SHA-1 et l'adresse de la dernière version : différente de
//! celle du fichier local = mise à jour disponible. La mise à jour télécharge le fichier
//! complet (pas de transfert différentiel), l'intègre comme une installation, et l'ancienne
//! version part à la corbeille.

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use crate::{appimage, exec::capture_codes};

/// Contenu de la section `.upd_info` (vide : pas de mises à jour intégrées).
pub fn update_info(path: &Path) -> Option<String> {
    let mut f = fs::File::open(path).ok()?;
    let mut h = [0u8; 64];
    f.read_exact(&mut h).ok()?;
    if &h[..4] != b"\x7fELF" || h[4] != 2 {
        return None;
    }
    let le = h[5] == 1;
    let rd16 = |b: &[u8]| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) } as u64;
    let rd64 = |b: &[u8]| {
        let a: [u8; 8] = b[..8].try_into().unwrap();
        if le { u64::from_le_bytes(a) } else { u64::from_be_bytes(a) }
    };
    let (shoff, shentsize, shnum, shstrndx) = (rd64(&h[0x28..]), rd16(&h[0x3A..]), rd16(&h[0x3C..]), rd16(&h[0x3E..]));
    if shentsize < 64 || shnum == 0 || shnum > 512 {
        return None;
    }
    let mut table = vec![0u8; (shentsize * shnum) as usize];
    f.seek(SeekFrom::Start(shoff)).ok()?;
    f.read_exact(&mut table).ok()?;
    let sect = |i: u64| -> (u32, u64, u64) {
        let s = &table[(i * shentsize) as usize..];
        let name = if le { u32::from_le_bytes(s[0..4].try_into().unwrap()) } else { u32::from_be_bytes(s[0..4].try_into().unwrap()) };
        (name, rd64(&s[0x18..]), rd64(&s[0x20..]))
    };
    let (_, str_off, str_size) = sect(shstrndx);
    let mut names = vec![0u8; str_size.min(1 << 16) as usize];
    f.seek(SeekFrom::Start(str_off)).ok()?;
    f.read_exact(&mut names).ok()?;
    for i in 0..shnum {
        let (name, off, size) = sect(i);
        let n = names.get(name as usize..)?;
        let end = n.iter().position(|b| *b == 0).unwrap_or(n.len());
        if &n[..end] == b".upd_info" {
            let mut data = vec![0u8; size.min(4096) as usize];
            f.seek(SeekFrom::Start(off)).ok()?;
            f.read_exact(&mut data).ok()?;
            let text = String::from_utf8_lossy(&data).trim_matches(char::from(0)).trim().to_string();
            return (!text.is_empty()).then_some(text);
        }
    }
    None
}

/// Description lisible de la méthode de mise à jour.
pub fn describe(info: &str) -> &'static str {
    match info.split('|').next() {
        Some("zsync") => "Mises à jour intégrées (serveur de l'éditeur)",
        Some("gh-releases-zsync") => "Mises à jour intégrées (versions GitHub)",
        Some("pling-v1-zsync") => "Mises à jour intégrées (Pling)",
        _ => "Mises à jour intégrées",
    }
}

/// En-tête d'un fichier .zsync : (nom de fichier, empreinte SHA-1, adresse relative ou absolue).
pub fn parse_zsync_header(text: &str) -> Option<(String, String, String)> {
    let mut name = None;
    let mut sha = None;
    let mut url = None;
    for line in text.lines() {
        if line.is_empty() {
            break;
        }
        match line.split_once(": ") {
            Some(("Filename", v)) => name = Some(v.trim().to_string()),
            Some(("SHA-1", v)) => sha = Some(v.trim().to_lowercase()),
            Some(("URL", v)) => url = Some(v.trim().to_string()),
            _ => {}
        }
    }
    Some((name?, sha?, url?))
}

/// Motif `AppImageUpdate-*x86_64.AppImage.zsync` (seulement `*`).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let mut rest = name;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(pos) = rest.find(p) else { return false };
            rest = &rest[pos + p.len()..];
        }
    }
    true
}

/// Adresse du .zsync d'après l'information de mise à jour (interroge GitHub si besoin).
fn zsync_url(info: &str) -> Result<String> {
    let f: Vec<&str> = info.split('|').collect();
    match f.first().copied() {
        Some("zsync") if f.len() >= 2 => Ok(f[1].to_string()),
        Some("gh-releases-zsync") if f.len() >= 5 => {
            let (owner, repo, tag, pattern) = (f[1], f[2], f[3], f[4]);
            let api = if tag == "latest" {
                format!("https://api.github.com/repos/{owner}/{repo}/releases/latest")
            } else {
                format!("https://api.github.com/repos/{owner}/{repo}/releases/tags/{tag}")
            };
            let out = capture_codes(&["curl", "-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", &api], &[0])?;
            let v: serde_json::Value = serde_json::from_str(&out)?;
            v["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|a| a["name"].as_str().is_some_and(|n| glob_match(pattern, n)))
                .and_then(|a| a["browser_download_url"].as_str().map(String::from))
                .context("aucun fichier .zsync dans la dernière version")
        }
        _ => bail!("méthode de mise à jour non prise en charge : {}", f.first().unwrap_or(&"")),
    }
}

fn join(base: &str, rel: &str) -> String {
    if rel.contains("://") {
        return rel.to_string();
    }
    match base.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{rel}"),
        None => rel.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Available {
    /// Nom de fichier de la nouvelle version.
    pub file_name: String,
    pub url: String,
}

fn sha1_of(path: &Path) -> Option<String> {
    capture_codes(&["sha1sum", &path.to_string_lossy()], &[0]).ok()?.split_whitespace().next().map(str::to_lowercase)
}

/// Mise à jour disponible pour cette AppImage ? `Ok(None)` : à jour.
pub fn check(path: &Path, info: &str) -> Result<Option<Available>> {
    let zurl = zsync_url(info)?;
    // L'en-tête tient dans les premiers kilo-octets ; le reste (sommes des blocs) est inutile.
    let head = capture_codes(&["curl", "-fsSL", "--max-time", "30", "-r", "0-8191", &zurl], &[0])?;
    let (file_name, sha, url) = parse_zsync_header(&head).context("en-tête zsync illisible")?;
    let local = sha1_of(path).context("lecture du fichier")?;
    if local == sha {
        return Ok(None);
    }
    Ok(Some(Available { file_name, url: join(&zurl, &url) }))
}

/// Télécharge et installe la nouvelle version ; l'ancienne (autre nom de fichier) est mise à la
/// corbeille. Renvoie le chemin de la nouvelle version.
pub fn apply(old: &Path, up: &Available) -> Result<PathBuf> {
    let dir = appimage::apps_dir();
    fs::create_dir_all(&dir)?;
    let name = Path::new(&up.file_name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "mise-a-jour.AppImage".into());
    let dest = dir.join(&name);
    let part = dir.join(format!(".{name}.part"));
    let status = std::process::Command::new("curl").args(["-fsSL", "--max-time", "3600", "-o"]).arg(&part).arg(&up.url).status()?;
    if !status.success() {
        let _ = fs::remove_file(&part);
        bail!("téléchargement impossible ({})", up.url);
    }
    if !appimage::is_appimage(&part) {
        let _ = fs::remove_file(&part);
        bail!("le fichier téléchargé n'est pas une AppImage");
    }
    fs::rename(&part, &dest)?;
    let insp = appimage::inspect(&dest)?;
    let outcome = appimage::install(&insp)?;
    let old_real = old.canonicalize().unwrap_or_else(|_| old.to_path_buf());
    if old_real != outcome.app.path && old_real.is_file() {
        let _ = capture_codes(&["gio", "trash", &old_real.to_string_lossy()], &[0]);
    }
    if let Some(prev) = outcome.previous.filter(|p| *p != outcome.app.path && p.is_file()) {
        let _ = capture_codes(&["gio", "trash", &prev.to_string_lossy()], &[0]);
    }
    Ok(outcome.app.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn en_tete_et_motifs() {
        let h = "zsync: 0.6.2\nFilename: AppImageUpdate-x86_64.AppImage\nMTime: Sat, 18 Oct 2025 19:44:31 +0000\nBlocksize: 2048\nLength: 48638456\nURL: AppImageUpdate-x86_64.AppImage\nSHA-1: 6962F0BCA82E85E8D21346BD28A928733AB8E567\n\n\u{1}\u{2}binaire";
        let (n, s, u) = parse_zsync_header(h).unwrap();
        assert_eq!((n.as_str(), u.as_str()), ("AppImageUpdate-x86_64.AppImage", "AppImageUpdate-x86_64.AppImage"));
        assert_eq!(s, "6962f0bca82e85e8d21346bd28a928733ab8e567");
        assert!(glob_match("AppImageUpdate-*x86_64.AppImage.zsync", "AppImageUpdate-x86_64.AppImage.zsync"));
        assert!(glob_match("App-*-x86_64.AppImage.zsync", "App-1.2.3-x86_64.AppImage.zsync"));
        assert!(!glob_match("App-*-x86_64.AppImage.zsync", "App-1.2.3-aarch64.AppImage.zsync"));
        assert_eq!(join("https://x.org/dl/App.zsync", "App.AppImage"), "https://x.org/dl/App.AppImage");
        assert_eq!(join("https://x.org/a.zsync", "https://cdn/b"), "https://cdn/b");
        assert_eq!(describe("gh-releases-zsync|a|b|latest|*.zsync"), "Mises à jour intégrées (versions GitHub)");
    }

    #[test]
    fn section_absente() {
        // Pas un ELF 64 bits : rien.
        let p = std::env::temp_dir().join(format!("noos-test-upd-{}", std::process::id()));
        fs::write(&p, b"pas un elf").unwrap();
        assert_eq!(update_info(&p), None);
        let _ = fs::remove_file(&p);
    }
}

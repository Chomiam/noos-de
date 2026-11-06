//! Clés SSH de l'utilisateur (`~/.ssh/*.pub`) : liste, empreinte, génération (ssh-keygen).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};

#[derive(Clone, Debug)]
pub struct SshKey {
    pub public: PathBuf,
    pub private: PathBuf,
    pub kind: String,
    pub comment: String,
    pub fingerprint: String,
    pub has_private: bool,
}

pub fn dir() -> PathBuf {
    noos_core::paths::home().join(".ssh")
}

/// Analyse une ligne de `ssh-keygen -lf` : « 256 SHA256:xxx commentaire (ED25519) ».
pub fn parse_fingerprint(line: &str) -> (String, String, String) {
    let line = line.trim();
    let kind = line.rsplit_once('(').map(|(_, k)| k.trim_end_matches(')').to_string()).unwrap_or_default();
    let mut parts = line.splitn(3, ' ');
    let _bits = parts.next();
    let fp = parts.next().unwrap_or_default().to_string();
    let comment = parts
        .next()
        .unwrap_or_default()
        .rsplit_once(" (")
        .map(|(c, _)| c.to_string())
        .unwrap_or_default();
    (kind, fp, comment)
}

pub fn list() -> Vec<SshKey> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut keys: Vec<SshKey> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pub"))
        .map(|public| {
            let private = public.with_extension("");
            let out = Command::new("ssh-keygen").arg("-lf").arg(&public).output();
            let line = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
            let (kind, fingerprint, comment) = parse_fingerprint(&line);
            SshKey { has_private: private.exists(), public, private, kind, comment, fingerprint }
        })
        .collect();
    keys.sort_by(|a, b| a.public.cmp(&b.public));
    keys
}

pub fn public_key(key: &SshKey) -> String {
    std::fs::read_to_string(&key.public).unwrap_or_default().trim().to_string()
}

/// Génère une paire de clés. `kind` : « ed25519 » ou « rsa » (4096 bits).
pub fn generate(kind: &str, name: &str, comment: &str, passphrase: &str) -> anyhow::Result<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        bail!("nom de fichier invalide");
    }
    std::fs::create_dir_all(dir())?;
    let path = dir().join(name);
    if path.exists() || path.with_extension("pub").exists() {
        bail!("une clé « {name} » existe déjà");
    }
    let mut cmd = Command::new("ssh-keygen");
    cmd.args(["-q", "-t", kind]);
    if kind == "rsa" {
        cmd.args(["-b", "4096"]);
    }
    let out = cmd
        .arg("-C")
        .arg(comment)
        .arg("-N")
        .arg(passphrase)
        .arg("-f")
        .arg(&path)
        .output()
        .context("ssh-keygen introuvable")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(path)
}

/// Supprime la clé privée et la clé publique.
pub fn delete(key: &SshKey) -> anyhow::Result<()> {
    for p in [&key.private, &key.public] {
        if Path::new(p).exists() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub fn default_comment() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    format!("{user}@{}", host.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_line() {
        let (k, fp, c) = parse_fingerprint("256 SHA256:AbCdEf0123 axel@m720q (ED25519)\n");
        assert_eq!((k.as_str(), fp.as_str(), c.as_str()), ("ED25519", "SHA256:AbCdEf0123", "axel@m720q"));
        let (k, _, c) = parse_fingerprint("4096 SHA256:xyz clé de travail (RSA)");
        assert_eq!((k.as_str(), c.as_str()), ("RSA", "clé de travail"));
    }
}

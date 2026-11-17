//! Configuration du partage et validation de tout ce qui finit dans un fichier système.

use std::path::Path;

use anyhow::bail;
use serde::{Deserialize, Serialize};

/// Configuration système (`/etc/noos/sharing.toml`, écrite par le script privilégié).
pub const SYSTEM_CONFIG: &str = "/etc/noos/sharing.toml";
/// Comptes qui ont défini un mot de passe de partage (lisible par tous : la base de Samba ne
/// l'est pas).
pub const SAMBA_USERS: &str = "/var/lib/noos/sharing/samba-users";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SystemConfig {
    pub samba: Samba,
    pub ssh: Ssh,
}

/// Niveau de chiffrement SMB (`smb encrypt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Encrypt {
    /// Chiffré si l'appareil le permet (tous les systèmes récents).
    #[default]
    Desired,
    /// Toujours chiffré : les très vieux appareils (SMB 2) ne peuvent plus se connecter.
    Required,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Samba {
    pub enabled: bool,
    pub workgroup: String,
    /// Annonce WSD (`wsdd2`) : le PC apparaît dans « Réseau » de l'Explorateur Windows.
    pub discoverable: bool,
    /// Dossier personnel de chaque compte (`[homes]`, visible par son seul propriétaire).
    pub home: bool,
    pub home_writable: bool,
    pub encrypt: Encrypt,
    pub shares: Vec<Share>,
}

impl Default for Samba {
    fn default() -> Self {
        Samba { enabled: true, workgroup: "WORKGROUP".into(), discoverable: true, home: true, home_writable: true, encrypt: Encrypt::Desired, shares: vec![] }
    }
}

/// Dossier partagé supplémentaire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Share {
    /// Nom vu depuis le réseau (`\\PC\Photos`).
    pub name: String,
    pub path: String,
    pub writable: bool,
    /// Compte autorisé (celui qui a ajouté le dossier).
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ssh {
    pub enabled: bool,
    pub port: u16,
    /// Connexion par mot de passe (sinon : clés seulement).
    pub password_auth: bool,
}

impl Default for Ssh {
    fn default() -> Self {
        Ssh { enabled: false, port: 22, password_auth: true }
    }
}

impl SystemConfig {
    pub fn load() -> SystemConfig {
        if crate::demo() {
            return crate::demo::system_config();
        }
        Self::load_from(Path::new(SYSTEM_CONFIG))
    }

    pub fn load_from(path: &Path) -> SystemConfig {
        std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Vérifie et normalise tout ce qui sera écrit dans smb.conf / sshd_config.
    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.samba.workgroup = validate_workgroup(&self.samba.workgroup)?;
        let mut seen: Vec<String> = vec!["homes".into(), "global".into(), "printers".into(), "print$".into()];
        for s in &mut self.samba.shares {
            s.name = validate_share_name(&s.name)?;
            s.path = validate_path(&s.path)?;
            s.owner = validate_user(&s.owner)?;
            let low = s.name.to_lowercase();
            if seen.contains(&low) {
                bail!("deux partages portent le nom « {} »", s.name);
            }
            seen.push(low);
        }
        if self.ssh.port == 0 {
            bail!("port SSH invalide");
        }
        Ok(())
    }
}

/// Groupe de travail : 15 caractères au plus, lettres, chiffres, « - » et « _ ».
pub fn validate_workgroup(s: &str) -> anyhow::Result<String> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return Ok("WORKGROUP".into());
    }
    if s.chars().count() > 15 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("groupe de travail invalide : 15 caractères au plus, lettres, chiffres, « - » ou « _ »");
    }
    Ok(s)
}

/// Nom d'un partage : pas de caractère de contrôle ni de `[]\/:*?"<>|=;,%`, 64 au plus.
pub fn validate_share_name(s: &str) -> anyhow::Result<String> {
    let s = s.trim();
    if s.is_empty() {
        bail!("nom de partage vide");
    }
    if s.chars().count() > 64 {
        bail!("nom de partage trop long (64 caractères au plus)");
    }
    if let Some(c) = s.chars().find(|c| c.is_control() || "[]\\/:*?\"<>|=;,%+".contains(*c)) {
        bail!("le nom de partage ne peut pas contenir « {c} »");
    }
    Ok(s.to_string())
}

/// Chemin absolu d'un dossier existant, sans caractère de contrôle (ni `%`, variable de Samba).
pub fn validate_path(s: &str) -> anyhow::Result<String> {
    let s = s.trim().trim_end_matches('/');
    let s = if s.is_empty() { "/" } else { s };
    if !s.starts_with('/') {
        bail!("le chemin doit être absolu : {s}");
    }
    if s.chars().any(|c| c.is_control() || c == '%') || s.split('/').any(|p| p == "..") {
        bail!("chemin refusé : {s}");
    }
    const FORBIDDEN: &[&str] = &["/", "/etc", "/root", "/proc", "/sys", "/dev", "/run", "/boot", "/var", "/usr", "/bin", "/sbin", "/lib"];
    if FORBIDDEN.iter().any(|f| s == *f) {
        bail!("ce dossier système ne peut pas être partagé : {s}");
    }
    if !crate::demo() && !Path::new(s).is_dir() {
        bail!("dossier introuvable : {s}");
    }
    Ok(s.to_string())
}

/// Nom de compte Unix.
pub fn validate_user(s: &str) -> anyhow::Result<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 32 || !s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) || s.starts_with('-') {
        bail!("nom de compte invalide : {s}");
    }
    Ok(s.to_string())
}

/// Nom de la machine (RFC 1123) : lettres, chiffres et tirets, 63 au plus.
pub fn validate_hostname(s: &str) -> anyhow::Result<String> {
    let s = s.trim().to_lowercase();
    let ok = !s.is_empty()
        && s.len() <= 63
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-');
    if !ok {
        bail!("nom invalide : lettres sans accent, chiffres et tirets seulement (63 au plus, sans tiret au début ni à la fin)");
    }
    Ok(s)
}

/// Nom NetBIOS dérivé du nom de la machine (15 caractères, majuscules).
pub fn netbios_name(hostname: &str) -> String {
    let n: String = hostname.split('.').next().unwrap_or(hostname).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(15).collect();
    if n.is_empty() { "NOOS".into() } else { n.to_uppercase() }
}

/// Nom proposé pour un dossier partagé : son nom, rendu valide.
pub fn share_name_for(path: &str) -> String {
    let base = Path::new(path).file_name().map(|b| b.to_string_lossy().to_string()).unwrap_or_else(|| "Partage".into());
    let clean: String = base.chars().filter(|c| !c.is_control() && !"[]\\/:*?\"<>|=;,%+".contains(*c)).take(64).collect();
    if clean.trim().is_empty() { "Partage".into() } else { clean.trim().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms() {
        assert_eq!(validate_workgroup(" maison ").unwrap(), "MAISON");
        assert_eq!(validate_workgroup("").unwrap(), "WORKGROUP");
        assert!(validate_workgroup("trop long pour netbios").is_err());
        assert!(validate_share_name("Photos de vacances").is_ok());
        assert!(validate_share_name("a]b").is_err());
        assert!(validate_share_name("a\nb").is_err());
        assert!(validate_share_name("").is_err());
        assert_eq!(validate_hostname("Mon-PC").unwrap(), "mon-pc");
        assert!(validate_hostname("mon pc").is_err());
        assert!(validate_hostname("-pc").is_err());
        assert!(validate_hostname("pc-é").is_err());
        assert_eq!(netbios_name("salon-de-jeux-axel.local"), "SALON-DE-JEUX-A");
        assert_eq!(share_name_for("/home/a/Mes [photos]"), "Mes photos");
        assert!(validate_user("axel").is_ok());
        assert!(validate_user("-x").is_err());
        assert!(validate_user("a b").is_err());
    }

    #[test]
    fn chemins() {
        assert!(validate_path("relatif").is_err());
        assert!(validate_path("/etc").is_err());
        assert!(validate_path("/").is_err());
        assert!(validate_path("/tmp/../etc").is_err());
        assert!(validate_path("/home/%U").is_err());
        assert!(validate_path("/tmp\n[x]").is_err());
        assert_eq!(validate_path("/tmp/").unwrap(), "/tmp");
    }

    #[test]
    fn doublons() {
        let mut c = SystemConfig::default();
        c.samba.shares = vec![
            Share { name: "Photos".into(), path: "/tmp".into(), writable: false, owner: "axel".into() },
            Share { name: "photos".into(), path: "/tmp".into(), writable: false, owner: "axel".into() },
        ];
        assert!(c.validate().is_err());
        c.samba.shares.pop();
        assert!(c.validate().is_ok());
        c.samba.shares[0].name = "homes".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn toml_aller_retour() {
        let mut c = SystemConfig::default();
        c.ssh.enabled = true;
        c.samba.shares.push(Share { name: "Jeux".into(), path: "/srv/jeux".into(), writable: true, owner: "axel".into() });
        let t = c.to_toml().unwrap();
        assert_eq!(toml::from_str::<SystemConfig>(&t).unwrap(), c);
        // Fichier partiel : valeurs par défaut pour le reste.
        let p: SystemConfig = toml::from_str("[ssh]\nport = 2222\n").unwrap();
        assert_eq!(p.ssh.port, 2222);
        assert!(p.samba.enabled && p.samba.home && !p.ssh.enabled);
    }
}

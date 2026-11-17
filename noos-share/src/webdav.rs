//! WebDAV de l'utilisateur : `rclone serve webdav`, lancé par le service systemd utilisateur
//! `noos-webdav` (programme `noos-webdav`). Choisi parce que rclone est dans Debian, maintenu,
//! sans démon système ni root : le serveur ne voit que ce que voit l'utilisateur. Il gère
//! HTTPS, un fichier htpasswd (bcrypt) et la lecture seule ; plusieurs dossiers sont réunis par
//! un « combine » rclone (un dossier par entrée à la racine).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{validate_path, validate_share_name, validate_user};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebDav {
    pub enabled: bool,
    pub port: u16,
    pub https: bool,
    /// Nom d'utilisateur demandé par le serveur (par défaut : le compte).
    pub user: String,
    pub folders: Vec<Folder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Folder {
    pub name: String,
    pub path: String,
    pub writable: bool,
}

impl Default for WebDav {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        WebDav { enabled: false, port: 8443, https: true, user: crate::status::current_user(), folders: vec![Folder { name: "Dossier personnel".into(), path: home, writable: true }] }
    }
}

pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("noos/webdav")
}
pub fn config_path() -> PathBuf {
    config_dir().join("webdav.toml")
}
pub fn htpasswd_path() -> PathBuf {
    config_dir().join("htpasswd")
}
pub fn cert_path() -> PathBuf {
    config_dir().join("cert.pem")
}
pub fn key_path() -> PathBuf {
    config_dir().join("key.pem")
}

impl WebDav {
    pub fn load() -> WebDav {
        if crate::demo() {
            return crate::demo::webdav();
        }
        std::fs::read_to_string(config_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if crate::demo() {
            return Ok(());
        }
        let mut c = self.clone();
        c.validate()?;
        write_private(&config_path(), &toml::to_string_pretty(&c)?)
    }

    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.user = validate_user(&self.user)?;
        if self.port < 1024 {
            anyhow::bail!("choisissez un port à partir de 1024 (le serveur tourne sans droits administrateur)");
        }
        let mut seen = vec![];
        for f in &mut self.folders {
            f.name = validate_share_name(&f.name)?;
            f.path = validate_path(&f.path)?;
            if seen.contains(&f.name.to_lowercase()) {
                anyhow::bail!("deux dossiers portent le nom « {} »", f.name);
            }
            seen.push(f.name.to_lowercase());
        }
        Ok(())
    }

    pub fn password_set(&self) -> bool {
        if crate::demo() {
            return true;
        }
        std::fs::read_to_string(htpasswd_path()).is_ok_and(|t| t.lines().any(|l| l.starts_with(&format!("{}:", self.user))))
    }

    /// Mot de passe du serveur : fichier htpasswd (bcrypt), lisible par l'utilisateur seul.
    pub fn set_password(&self, password: &str) -> anyhow::Result<()> {
        if password.chars().count() < 8 {
            anyhow::bail!("8 caractères au moins");
        }
        if crate::demo() {
            return Ok(());
        }
        let hash = bcrypt::hash(password, 11)?;
        write_private(&htpasswd_path(), &format!("{}:{hash}\n", self.user))
    }
}

/// Écrit un fichier en 0600 (dossier en 0700).
pub fn write_private(path: &Path, content: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(content.as_bytes())?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Configuration rclone et nom du « remote » à servir. Un seul dossier : servi directement ;
/// plusieurs : un `combine` (un sous-dossier par entrée) ; lecture seule par dossier : `union`
/// à un seul membre marqué `:ro`.
pub fn rclone_config(folders: &[Folder]) -> (String, String) {
    let mut conf = String::new();
    let section = |i: usize, f: &Folder| -> String {
        if f.writable {
            format!("[d{i}]\ntype = alias\nremote = {}\n\n", f.path)
        } else {
            format!("[d{i}]\ntype = union\nupstreams = \"{0}:ro\" \"{0}:ro\"\n\n", f.path)
        }
    };
    if folders.len() == 1 {
        conf.push_str(&section(0, &folders[0]));
        return (conf, "d0:".into());
    }
    for (i, f) in folders.iter().enumerate() {
        conf.push_str(&section(i, f));
    }
    let ups: Vec<String> = folders.iter().enumerate().map(|(i, f)| format!("\"{}=d{i}:\"", f.name.replace('"', "'"))).collect();
    conf.push_str(&format!("[noos]\ntype = combine\nupstreams = {}\n", ups.join(" ")));
    (conf, "noos:".into())
}

/// Arguments de `rclone serve webdav`.
pub fn rclone_args(c: &WebDav, config_file: &Path, remote: &str) -> Vec<String> {
    let mut a: Vec<String> = vec!["--config".into(), config_file.display().to_string(), "serve".into(), "webdav".into(), remote.into()];
    a.extend(["--addr".into(), format!(":{}", c.port)]);
    a.extend(["--htpasswd".into(), htpasswd_path().display().to_string()]);
    a.extend(["--realm".into(), "NOOS".into()]);
    if c.https {
        a.extend(["--cert".into(), cert_path().display().to_string(), "--key".into(), key_path().display().to_string()]);
    }
    if c.folders.iter().all(|f| !f.writable) {
        a.push("--read-only".into());
    }
    a.extend(["--log-level".into(), "NOTICE".into()]);
    a
}

/// Empreintes du certificat (à comparer sur l'autre appareil) : SHA-256 (`AB:CD:…`, navigateurs,
/// Windows) et SHA-1 (`AB CD …`, telle que l'affiche gvfs : Explorateur de NOOS, GNOME).
pub fn cert_fingerprints() -> Option<(String, String)> {
    if crate::demo() {
        return Some((
            "3F:A2:91:0C:5B:7E:D4:18:6A:C3:22:9F:E0:47:B1:58:0D:93:6C:F2:14:AE:85:3B:C9:70:1D:E6:4F:A8:2B:57".into(),
            "36 8D A8 10 F5 EC 7B F8 3C 08 08 E2 C3 A4 50 42 73 16 AF 3F".into(),
        ));
    }
    let pem = std::fs::read_to_string(cert_path()).ok()?;
    let b64: String = pem.lines().skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE")).skip(1).take_while(|l| !l.starts_with("-----")).collect();
    let der = crate::status::base64_decode(&b64)?;
    let hex = |b: &[u8], sep: &str| b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(sep);
    Some((hex(&crate::status::sha256(&der), ":"), hex(&crate::status::sha1(&der), " ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rclone() {
        let one = [Folder { name: "Perso".into(), path: "/home/a".into(), writable: true }];
        let (c, r) = rclone_config(&one);
        assert_eq!(r, "d0:");
        assert!(c.contains("type = alias\nremote = /home/a"));
        let two = [one[0].clone(), Folder { name: "Photos d'été".into(), path: "/home/a/Images".into(), writable: false }];
        let (c, r) = rclone_config(&two);
        assert_eq!(r, "noos:");
        assert!(c.contains("[d1]\ntype = union\nupstreams = \"/home/a/Images:ro\" \"/home/a/Images:ro\""));
        assert!(c.contains("upstreams = \"Perso=d0:\" \"Photos d'été=d1:\""));
        let w = WebDav { folders: vec![two[1].clone()], ..WebDav::default() };
        let a = rclone_args(&w, Path::new("/run/x.conf"), "d0:");
        assert!(a.contains(&"--read-only".to_string()) && a.contains(&"--cert".to_string()));
    }

    #[test]
    fn validation() {
        let mut w = WebDav { user: "axel".into(), port: 80, folders: vec![], ..WebDav::default() };
        assert!(w.validate().is_err());
        w.port = 8443;
        assert!(w.validate().is_ok());
    }
}

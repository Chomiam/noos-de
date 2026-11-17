//! Persistance : `/etc/noos/firewall/config.toml` (réglages) et `rules.json` (règles), écrits
//! par le démon seulement (root, 0644, écriture atomique). `NOOS_FIREWALL_DIR` pour les tests.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use crate::model::{Config, Rules};

pub fn dir() -> PathBuf {
    std::env::var_os("NOOS_FIREWALL_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/noos/firewall"))
}

fn write_atomic(path: &Path, text: &str) -> anyhow::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn ensure_dir(d: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(d)?;
    // /etc/noos est modifiable par le groupe sudo (thème de connexion) : le dossier du pare-feu
    // est réservé à root.
    if unsafe { libc::geteuid() } == 0 {
        let _ = std::os::unix::fs::chown(d, Some(0), Some(0));
    }
    fs::set_permissions(d, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

/// Réglages (défauts si le fichier manque ; copie `.illisible` si le fichier est abîmé).
pub fn load_config(d: &Path) -> Config {
    let p = d.join("config.toml");
    match fs::read_to_string(&p) {
        Ok(t) => Config::from_toml(&t).unwrap_or_else(|e| {
            eprintln!("{} illisible ({e}) : réglages par défaut", p.display());
            let _ = fs::copy(&p, p.with_extension("toml.illisible"));
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

pub fn save_config(d: &Path, c: &Config) -> anyhow::Result<()> {
    ensure_dir(d)?;
    write_atomic(&d.join("config.toml"), &format!("# Pare-feu NOOS — modifié par Paramètres › Confidentialité et sécurité › Pare-feu\n{}", c.to_toml()?))
}

/// Règles (règles livrées si le fichier manque).
pub fn load_rules(d: &Path) -> Rules {
    let p = d.join("rules.json");
    match fs::read_to_string(&p) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|e| {
            eprintln!("{} illisible ({e}) : règles livrées", p.display());
            let _ = fs::copy(&p, p.with_extension("json.illisible"));
            Rules::seed()
        }),
        Err(_) => Rules::seed(),
    }
}

pub fn save_rules(d: &Path, r: &Rules) -> anyhow::Result<()> {
    ensure_dir(d)?;
    write_atomic(&d.join("rules.json"), &serde_json::to_string_pretty(r)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Mode;

    #[test]
    fn roundtrip() {
        let d = std::env::temp_dir().join(format!("noos-fw-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        assert_eq!(load_rules(&d), Rules::seed());
        let c = Config { mode: Mode::Silent, ..Config::default() };
        save_config(&d, &c).unwrap();
        assert_eq!(load_config(&d), c);
        let mut r = Rules::seed();
        r.alloc_id();
        save_rules(&d, &r).unwrap();
        assert_eq!(load_rules(&d), r);
        fs::write(d.join("config.toml"), "mode = 12").unwrap();
        assert_eq!(load_config(&d), Config::default());
        assert!(d.join("config.toml.illisible").exists());
        fs::remove_dir_all(&d).unwrap();
    }
}

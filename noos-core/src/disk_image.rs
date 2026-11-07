//! Images disque (ISO, IMG…) : montage en lecture seule, démontage et détection, par udisks
//! (`udisksctl`, sans mot de passe pour l'utilisateur de la session active). Utilisé par
//! l'Explorateur (menu contextuel, bandeau « Démonter », protection contre la suppression)
//! et par l'application Archives.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// Extensions des images disque montables.
pub const EXTENSIONS: &[&str] = &["iso", "img", "udf", "dmg"];

pub fn is_disk_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Image montée : périphérique boucle et points de montage (une image hybride peut en avoir
/// plusieurs, une par partition).
#[derive(Debug, Clone, PartialEq)]
pub struct Mounted {
    pub image: PathBuf,
    pub loop_device: String,
    pub mountpoints: Vec<PathBuf>,
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(cmd).args(args).env("LC_ALL", "C").output().map_err(|e| format!("{cmd} : {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("{cmd} a échoué") } else { err })
    }
}

/// Valeur d'un champ texte dans un objet JSON simple (`"back-file": "/x.iso"`).
fn json_field(obj: &str, key: &str) -> Option<String> {
    let i = obj.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = obj[i..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for c in rest.chars() {
        match (esc, c) {
            (true, c) => {
                out.push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, '"') => return Some(out),
            (false, c) => out.push(c),
        }
    }
    None
}

/// Périphériques boucle et leur fichier (`losetup -J -l`, lisible sans privilèges).
fn loop_devices() -> Vec<(String, PathBuf)> {
    let Ok(json) = run("losetup", &["-J", "-l"]) else { return vec![] };
    json.split('{')
        .filter_map(|obj| {
            let name = json_field(obj, "name")?;
            let back = json_field(obj, "back-file")?;
            // « /chemin (deleted) » si le fichier a été supprimé.
            Some((name, PathBuf::from(back.trim_end_matches(" (deleted)"))))
        })
        .collect()
}

/// Points de montage d'un périphérique et de ses partitions.
fn mountpoints(dev: &str) -> Vec<PathBuf> {
    run("lsblk", &["-nrpo", "MOUNTPOINT", dev])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).map(|l| PathBuf::from(l.trim().replace("\\x20", " "))).collect())
        .unwrap_or_default()
}

/// Images actuellement attachées (montées ou non) avec leurs points de montage.
pub fn all_mounted() -> Vec<Mounted> {
    loop_devices()
        .into_iter()
        .map(|(dev, image)| Mounted { mountpoints: mountpoints(&dev), image, loop_device: dev })
        .collect()
}

/// État de montage d'une image.
pub fn mounted(image: &Path) -> Option<Mounted> {
    let image = image.canonicalize().unwrap_or_else(|_| image.to_path_buf());
    all_mounted().into_iter().find(|m| m.image == image)
}

/// Image dont `dir` est (ou se trouve dans) un point de montage.
pub fn image_for_dir(dir: &Path) -> Option<Mounted> {
    all_mounted().into_iter().find(|m| m.mountpoints.iter().any(|mp| dir.starts_with(mp)))
}

/// Monte l'image en lecture seule ; renvoie le premier point de montage (déjà montée : le
/// point existant).
pub fn mount(image: &Path) -> Result<Mounted, String> {
    if let Some(m) = mounted(image).filter(|m| !m.mountpoints.is_empty()) {
        return Ok(m);
    }
    let image = image.canonicalize().map_err(|e| e.to_string())?;
    let dev = match mounted(&image) {
        Some(m) => m.loop_device,
        None => {
            let out = run("udisksctl", &["loop-setup", "--read-only", "--no-user-interaction", "-f", &image.to_string_lossy()])?;
            // « Mapped file /x.iso as /dev/loop0. »
            out.split_whitespace().find(|w| w.starts_with("/dev/loop")).map(|w| w.trim_end_matches('.').to_string()).ok_or("udisksctl : périphérique introuvable")?
        }
    };
    // udisks peut monter tout seul (montage automatique) : on attend un peu, sinon on monte
    // nous-mêmes le système de fichiers de l'image ou de ses partitions (ISO hybride).
    for _ in 0..10 {
        if !mountpoints(&dev).is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    if mountpoints(&dev).is_empty() {
        let candidates = run("lsblk", &["-nrpo", "NAME,FSTYPE", &dev]).unwrap_or_default();
        let mut errors = Vec::new();
        for line in candidates.lines() {
            let mut it = line.split_whitespace();
            let (Some(name), Some(fs)) = (it.next(), it.next()) else { continue };
            if fs.is_empty() || fs == "swap" {
                continue;
            }
            if let Err(e) = run("udisksctl", &["mount", "--no-user-interaction", "-b", name, "-o", "ro"]) {
                errors.push(e);
            }
        }
        if mountpoints(&dev).is_empty() {
            let _ = run("udisksctl", &["loop-delete", "--no-user-interaction", "-b", &dev]);
            return Err(errors.into_iter().next().unwrap_or_else(|| "aucun système de fichiers lisible dans cette image".into()));
        }
    }
    Ok(Mounted { mountpoints: mountpoints(&dev), image, loop_device: dev })
}

/// Démonte et détache l'image. `Err` si un programme utilise encore ses fichiers.
pub fn unmount(image: &Path) -> Result<(), String> {
    let Some(m) = mounted(image) else { return Ok(()) };
    let parts = run("lsblk", &["-nrpo", "NAME,MOUNTPOINT", &m.loop_device]).unwrap_or_default();
    for line in parts.lines() {
        let mut it = line.splitn(2, ' ');
        let (Some(name), Some(mp)) = (it.next(), it.next()) else { continue };
        if mp.trim().is_empty() {
            continue;
        }
        run("udisksctl", &["unmount", "--no-user-interaction", "-b", name]).map_err(|e| {
            if e.contains("busy") || e.contains("occupé") {
                "des fichiers de l'image sont encore ouverts (fermez les fenêtres qui les utilisent)".to_string()
            } else {
                e
            }
        })?;
    }
    run("udisksctl", &["loop-delete", "--no-user-interaction", "-b", &m.loop_device]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json() {
        let j = r#"{"name": "/dev/loop0", "back-file": "/home/a b/x.iso", "ro": true}"#;
        assert_eq!(json_field(j, "name").as_deref(), Some("/dev/loop0"));
        assert_eq!(json_field(j, "back-file").as_deref(), Some("/home/a b/x.iso"));
        assert!(is_disk_image(Path::new("/a/B.ISO")));
    }

    /// Vrai montage : `NOOS_TEST_ISO=/chemin.iso cargo test -- --ignored mount_cycle`.
    #[test]
    #[ignore]
    fn mount_cycle() {
        let iso = PathBuf::from(std::env::var("NOOS_TEST_ISO").expect("NOOS_TEST_ISO"));
        let m = mount(&iso).expect("montage");
        println!("monté : {m:?}");
        assert!(!m.mountpoints.is_empty());
        assert!(std::fs::read_dir(&m.mountpoints[0]).unwrap().count() > 0);
        assert_eq!(image_for_dir(&m.mountpoints[0].join("x")).map(|x| x.image), Some(iso.canonicalize().unwrap()));
        unmount(&iso).expect("démontage");
        assert!(mounted(&iso).is_none());
        println!("démonté");
    }
}

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// `$XDG_CONFIG_HOME` ou `~/.config`
pub fn xdg_config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".config"))
}

/// `$XDG_DATA_HOME` ou `~/.local/share`
pub fn xdg_data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
}

/// `~/.config/noos`
pub fn config_dir() -> PathBuf {
    xdg_config_home().join("noos")
}

/// `~/.config/noos/noos.toml`
pub fn config_file() -> PathBuf {
    config_dir().join("noos.toml")
}

/// Répertoire des données installées de NOOS (fond d'écran, logo…).
///
/// Ordre : `$NOOS_DATA_DIR`, `~/.local/share/noos`, `/usr/local/share/noos`,
/// puis le dossier `data/` du dépôt (pratique en développement).
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NOOS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let candidates = [
        xdg_data_home().join("noos"),
        PathBuf::from("/usr/local/share/noos"),
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../data")),
    ];
    candidates
        .iter()
        .find(|p| p.join("noos-logo.svg").exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

/// Chemin du socket IPC du compositeur.
///
/// Le compositeur exporte `NOOS_IPC_SOCKET` à ses enfants ; à défaut on le dérive de
/// `WAYLAND_DISPLAY` pour que plusieurs instances (imbriquées) puissent coexister.
pub fn ipc_socket() -> PathBuf {
    if let Some(p) = std::env::var_os("NOOS_IPC_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    runtime.join(format!("noos-{display}.sock"))
}

/// Dossier « Images » de l'utilisateur (XDG_PICTURES_DIR de `~/.config/user-dirs.dirs`,
/// sinon `~/Images` ou `~/Pictures`).
pub fn pictures_dir() -> PathBuf {
    let home = home();
    let from_xdg = std::fs::read_to_string(xdg_config_home().join("user-dirs.dirs")).ok().and_then(|s| {
        s.lines().find_map(|l| {
            let v = l.trim().strip_prefix("XDG_PICTURES_DIR=")?.trim_matches('"');
            Some(PathBuf::from(v.replace("$HOME", &home.to_string_lossy())))
        })
    });
    from_xdg
        .filter(|p| p != &home)
        .or_else(|| ["Images", "Pictures"].iter().map(|d| home.join(d)).find(|p| p.is_dir()))
        .unwrap_or_else(|| home.join("Images"))
}

/// Fonds d'écran téléchargés (Bing, Wallhaven) : `~/Images/Wallpapers`, facile à retrouver.
pub fn wallpapers_dir() -> PathBuf {
    pictures_dir().join("Wallpapers")
}

/// Cache (miniatures…) : `$XDG_CACHE_HOME/noos` ou `~/.cache/noos`.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
        .join("noos")
}

/// Range dans `~/Images/Wallpapers` les fonds téléchargés par une version précédente
/// (`~/.local/share/noos/wallpapers/…`). Renvoie le nouveau chemin de `current` s'il a bougé.
pub fn migrate_old_wallpapers(current: &str) -> Option<String> {
    let old_root = xdg_data_home().join("noos/wallpapers");
    let new_dir = wallpapers_dir();
    let mut moved = None;
    for (sub, prefix) in [("bing", "bing-"), ("wallhaven", "")] {
        let Ok(rd) = std::fs::read_dir(old_root.join(sub)) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let Some(file_name) = p.file_name() else { continue };
            let name = format!("{prefix}{}", file_name.to_string_lossy());
            let _ = std::fs::create_dir_all(&new_dir);
            let dest = new_dir.join(&name);
            // Renommage (même disque), sinon copie puis suppression.
            if std::fs::rename(&p, &dest).is_err() && std::fs::copy(&p, &dest).is_ok() {
                let _ = std::fs::remove_file(&p);
            }
            if p.to_string_lossy() == current {
                moved = Some(dest.to_string_lossy().to_string());
            }
        }
    }
    // Nettoyage prudent : les miniatures (cache), puis les dossiers seulement s'ils sont vides.
    for sub in ["bing", "wallhaven"] {
        let _ = std::fs::remove_dir_all(old_root.join(sub).join("miniatures"));
        let _ = std::fs::remove_dir(old_root.join(sub));
    }
    let _ = std::fs::remove_dir(&old_root);
    moved
}

/// Images téléchargées pour un aperçu (pas encore appliquées).
pub fn preview_dir() -> PathBuf {
    cache_dir().join("wallpapers/apercu")
}

/// Image appliquée comme fond : si elle vient du cache d'aperçu, elle est rangée dans
/// `~/Images/Wallpapers`. Renvoie son emplacement définitif.
pub fn keep_wallpaper(path: &std::path::Path) -> PathBuf {
    if path.parent() != Some(preview_dir().as_path()) {
        return path.to_path_buf();
    }
    let Some(name) = path.file_name() else { return path.to_path_buf() };
    let dest = wallpapers_dir().join(name);
    let _ = std::fs::create_dir_all(wallpapers_dir());
    if std::fs::rename(path, &dest).is_ok() || std::fs::copy(path, &dest).is_ok() {
        let _ = std::fs::remove_file(path);
        dest
    } else {
        path.to_path_buf()
    }
}

//! AppImage : intégration au bureau, sans root (à la manière d'AppImageLauncher).
//!
//! Installer une AppImage = la **déplacer** dans `~/Applications`, la rendre exécutable, copier
//! son icône dans `~/.local/share/noos/appimages/icons` et écrire un lanceur
//! `~/.local/share/applications/appimage-<id>.desktop` (clé `X-NOOS-AppImage` = chemin du
//! fichier) : le menu Démarrer le voit aussitôt (GIO surveille ce dossier).
//!
//! Le nom, l'icône et le lanceur d'origine sont lus **sans exécuter le fichier** : l'image
//! SquashFS (type 2) commence juste après l'exécutable ELF du runtime, on en calcule la
//! position depuis l'en-tête ELF et `unsquashfs -offset` en extrait ce qu'il faut. Type 1
//! (ancien, ISO 9660) : `7z`. À défaut (runtime DwarFS…), le nom vient du fichier.
//!
//! Outils Debian : `libfuse2t64` (runtime classique, libfuse.so.2), `fuse3` (runtime statique
//! récent, fusermount3), `squashfs-tools`, `desktop-file-utils` — dans `debian/control`.

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

/// Clé du lanceur qui désigne le fichier AppImage intégré.
const KEY: &str = "X-NOOS-AppImage";
/// Types MIME (shared-mime-info) des AppImage.
pub const MIME_TYPES: [&str; 2] = ["application/vnd.appimage", "application/x-iso9660-appimage"];

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/share"))
}

/// `~/Applications` : où vivent les AppImage installées.
pub fn apps_dir() -> PathBuf {
    home().join("Applications")
}

fn launchers_dir() -> PathBuf {
    data_home().join("applications")
}

fn icons_dir() -> PathBuf {
    data_home().join("noos/appimages/icons")
}

/// Type d'AppImage (1 = ISO 9660, 2 = SquashFS), d'après la signature `AI\x01` / `AI\x02`
/// placée à l'octet 8 de l'en-tête ELF.
pub fn kind(path: &Path) -> Option<u8> {
    let mut head = [0u8; 11];
    fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..4] != b"\x7fELF" || &head[8..10] != b"AI" {
        return None;
    }
    matches!(head[10], 1 | 2).then_some(head[10])
}

pub fn is_appimage(path: &Path) -> bool {
    path.is_file() && kind(path).is_some()
}

/// Taille de l'exécutable ELF du runtime = début de l'image SquashFS (comme le runtime
/// lui-même : fin de la table des sections).
fn elf_size(path: &Path) -> Result<u64> {
    let mut f = fs::File::open(path)?;
    let mut h = [0u8; 64];
    f.read_exact(&mut h)?;
    let le = h[5] == 1;
    let u16_at = |o: usize| if le { u16::from_le_bytes([h[o], h[o + 1]]) } else { u16::from_be_bytes([h[o], h[o + 1]]) } as u64;
    let u32_at = |o: usize| {
        let b = [h[o], h[o + 1], h[o + 2], h[o + 3]];
        (if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) }) as u64
    };
    let u64_at = |o: usize| {
        let b: [u8; 8] = h[o..o + 8].try_into().unwrap();
        if le { u64::from_le_bytes(b) } else { u64::from_be_bytes(b) }
    };
    let (shoff, shentsize, shnum) = match h[4] {
        2 => (u64_at(0x28), u16_at(0x3A), u16_at(0x3C)),
        1 => (u32_at(0x20), u16_at(0x2E), u16_at(0x30)),
        c => bail!("classe ELF inconnue ({c})"),
    };
    let size = shoff + shentsize * shnum;
    // Vérification : une image SquashFS commence par « hsqs ».
    let mut magic = [0u8; 4];
    f.seek(SeekFrom::Start(size))?;
    f.read_exact(&mut magic)?;
    if &magic != b"hsqs" {
        bail!("pas d'image SquashFS après le runtime (DwarFS ?)");
    }
    Ok(size)
}

/// Dossier temporaire supprimé avec l'inspection.
#[derive(Debug)]
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("noos-appimage-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Lanceur `.desktop` simplifié : groupe `[Desktop Entry]` seulement.
#[derive(Debug, Clone, Default)]
pub struct Entry(BTreeMap<String, String>);

impl Entry {
    pub fn parse(text: &str) -> Self {
        let mut map = BTreeMap::new();
        let mut in_main = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_main = line == "[Desktop Entry]";
            } else if in_main && !line.starts_with('#') {
                if let Some((k, v)) = line.split_once('=') {
                    map.entry(k.trim().to_string()).or_insert_with(|| v.trim().to_string());
                }
            }
        }
        Self(map)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// Valeur traduite selon la langue de la session (`Name[fr_FR]`, `Name[fr]`, `Name`).
    pub fn localized(&self, key: &str) -> Option<&str> {
        let lang = std::env::var("LC_ALL")
            .ok()
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|v| !v.is_empty()))
            .or_else(|| std::env::var("LANG").ok())
            .unwrap_or_default();
        let lang = lang.split(['.', '@']).next().unwrap_or("");
        let short = lang.split('_').next().unwrap_or("");
        [lang, short]
            .iter()
            .filter(|l| !l.is_empty())
            .find_map(|l| self.get(&format!("{key}[{l}]")))
            .or_else(|| self.get(key))
    }

    fn pairs(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

/// Ce qu'on sait d'une AppImage avant de l'installer.
#[derive(Debug)]
pub struct Inspection {
    pub path: PathBuf,
    pub kind: u8,
    pub name: String,
    pub comment: Option<String>,
    pub version: Option<String>,
    /// Icône extraite (dans un dossier temporaire, valable tant que l'inspection vit).
    pub icon: Option<PathBuf>,
    /// Lanceur d'origine embarqué dans l'AppImage (absent si illisible).
    pub entry: Option<Entry>,
    /// Lanceur déjà présent pour ce fichier (AppImage déjà installée).
    pub installed: Option<Installed>,
    _tmp: Option<TempDir>,
}

/// Nom lisible tiré du nom de fichier : « Obsidian-1.6.7.AppImage » → « Obsidian ».
fn name_from_file(path: &Path) -> String {
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("AppImage");
    let mut stem = stem.strip_suffix(".AppImage").or_else(|| stem.strip_suffix(".appimage")).unwrap_or(stem);
    // Architecture en suffixe : « Kudu-x86_64 », « App_amd64 »…
    for arch in ["x86_64", "x86-64", "amd64", "aarch64", "arm64", "x64", "i386", "i686"] {
        if let Some(s) = stem.strip_suffix(arch).filter(|s| s.ends_with(['-', '_', '.', ' '])) {
            stem = &s[..s.len() - 1];
        }
    }
    let cut = stem
        .char_indices()
        .find(|&(i, c)| i > 0 && matches!(c, '-' | '_' | ' ') && {
            let rest = &stem[i + 1..];
            rest.starts_with(|d: char| d.is_ascii_digit())
                || (rest.starts_with(['v', 'V']) && rest[1..].starts_with(|d: char| d.is_ascii_digit()))
        })
        .map_or(stem.len(), |(i, _)| i);
    let name = stem[..cut].replace(['_', '-'], " ");
    if name.trim().is_empty() { stem.to_string() } else { name.trim().to_string() }
}

/// Extrait les fichiers racine utiles (lanceurs, icônes, `.DirIcon`) dans `dest`.
fn extract_metadata(path: &Path, kind: u8, dest: &Path) -> Result<()> {
    let wanted = ["*.desktop", ".DirIcon", "*.png", "*.svg", "*.xpm"];
    let out = if kind == 2 {
        let offset = elf_size(path)?.to_string();
        // -follow-symlinks (.DirIcon est souvent un lien) n'accepte pas les jokers : on liste
        // d'abord la racine, puis on extrait les noms exacts.
        let list = Command::new("unsquashfs")
            .args(["-no-progress", "-max-depth", "1", "-offset", &offset, "-l"])
            .arg(path)
            .stdin(Stdio::null())
            .output()
            .context("unsquashfs introuvable (paquet squashfs-tools)")?;
        let names: Vec<String> = String::from_utf8_lossy(&list.stdout)
            .lines()
            .filter_map(|l| l.trim().strip_prefix("squashfs-root/"))
            .filter(|n| !n.contains('/'))
            .filter(|n| *n == ".DirIcon" || [".desktop", ".png", ".svg", ".xpm"].iter().any(|x| n.ends_with(x)))
            .map(str::to_string)
            .collect();
        if names.is_empty() {
            bail!("aucun lanceur ni icône à la racine ({})", String::from_utf8_lossy(&list.stderr).trim());
        }
        Command::new("unsquashfs")
            .args(["-quiet", "-no-progress", "-no-xattrs", "-follow-symlinks", "-offset", &offset])
            .arg("-dest")
            .arg(dest)
            .arg(path)
            .args(&names)
            .stdin(Stdio::null())
            .output()
            .context("unsquashfs introuvable (paquet squashfs-tools)")?
    } else {
        Command::new("7z")
            .args(["x", "-tiso", "-y", "-bso0", "-bsp0"])
            .arg(format!("-o{}", dest.display()))
            .arg(path)
            .args(wanted)
            .stdin(Stdio::null())
            .output()
            .context("7z introuvable (paquet 7zip)")?
    };
    let empty = fs::read_dir(dest).map_or(true, |mut d| d.next().is_none());
    if !out.status.success() && empty {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Lit le nom, la description et l'icône d'une AppImage sans l'exécuter.
pub fn inspect(path: &Path) -> Result<Inspection> {
    let path = path.canonicalize().with_context(|| format!("{} introuvable", path.display()))?;
    let Some(kind) = kind(&path) else { bail!("{} n'est pas une AppImage", path.display()) };
    let installed = installed().into_iter().find(|i| i.path == path);
    let mut insp = Inspection {
        name: name_from_file(&path),
        path: path.clone(),
        kind,
        comment: None,
        version: None,
        icon: None,
        entry: None,
        installed,
        _tmp: None,
    };
    let Ok(tmp) = TempDir::new() else { return Ok(insp) };
    let root = tmp.0.join("root");
    if let Err(e) = extract_metadata(&path, kind, &root) {
        eprintln!("noos-pkg : AppImage {} : métadonnées illisibles : {e:#}", path.display());
    }
    let desktop = fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "desktop") && p.is_file());
    if let Some(entry) = desktop.and_then(|d| fs::read_to_string(d).ok()).map(|t| Entry::parse(&t)) {
        if let Some(n) = entry.localized("Name") {
            insp.name = n.to_string();
        }
        insp.comment = entry.localized("Comment").or_else(|| entry.localized("GenericName")).map(str::to_string);
        insp.version = entry.get("X-AppImage-Version").map(str::to_string);
        // Icône : celle que nomme le lanceur, à la racine ; sinon .DirIcon.
        if let Some(icon) = entry.get("Icon") {
            insp.icon = ["png", "svg", "xpm"].iter().map(|x| root.join(format!("{icon}.{x}"))).find(|p| p.is_file());
        }
        insp.entry = Some(entry);
    }
    if insp.icon.is_none() {
        let dir_icon = root.join(".DirIcon");
        insp.icon = dir_icon.is_file().then_some(dir_icon);
    }
    insp._tmp = Some(tmp);
    Ok(insp)
}

/// Une AppImage intégrée au menu.
#[derive(Debug, Clone)]
pub struct Installed {
    pub path: PathBuf,
    pub launcher: PathBuf,
    pub name: String,
    pub comment: Option<String>,
    pub version: Option<String>,
    pub icon: Option<PathBuf>,
}

impl Installed {
    pub fn size(&self) -> u64 {
        fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

/// AppImages intégrées (lanceurs portant la clé `X-NOOS-AppImage`).
pub fn installed() -> Vec<Installed> {
    let mut list: Vec<Installed> = fs::read_dir(launchers_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("appimage-") && n.ends_with(".desktop")))
        .filter_map(|launcher| {
            let entry = Entry::parse(&fs::read_to_string(&launcher).ok()?);
            let path = PathBuf::from(entry.get(KEY)?);
            Some(Installed {
                name: entry.localized("Name").unwrap_or("AppImage").to_string(),
                comment: entry.localized("Comment").map(str::to_string),
                version: entry.get("X-AppImage-Version").map(str::to_string),
                icon: entry.get("Icon").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file()),
                path,
                launcher,
            })
        })
        .collect();
    list.sort_by_key(|i| i.name.to_lowercase());
    list
}

/// AppImages de `~/Applications` sans lanceur (copiées à la main).
pub fn unintegrated() -> Vec<PathBuf> {
    let known: Vec<PathBuf> = installed().into_iter().map(|i| i.path).collect();
    let mut list: Vec<PathBuf> = fs::read_dir(apps_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_appimage(p) && !known.contains(p))
        .collect();
    list.sort();
    list
}

/// Identifiant de fichier sûr : minuscules, chiffres, « - ».
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "app".into() } else { out }
}

/// Nom de fichier sans les caractères qui compliquent la ligne Exec (`"`, `` ` ``, `$`, `\`, `%`).
fn safe_file_name(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("app.AppImage");
    name.chars().map(|c| if matches!(c, '"' | '`' | '$' | '\\' | '%' | '\n') { '_' } else { c }).collect()
}

/// Argument de la ligne Exec : entre guillemets s'il contient des espaces.
fn exec_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.contains([' ', '\t', '\'', '>', '<', '~', '|', '&', ';', '*', '?', '#', '(', ')']) { format!("\"{s}\"") } else { s.into_owned() }
}

/// Déplace (ou copie puis supprime, entre deux disques) `src` vers `dest`.
fn move_file(src: &Path, dest: &Path) -> Result<()> {
    if fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    let part = dest.with_extension("part");
    fs::copy(src, &part).with_context(|| format!("copie vers {}", dest.display()))?;
    fs::rename(&part, dest)?;
    fs::remove_file(src).with_context(|| format!("suppression de {}", src.display()))?;
    Ok(())
}

fn refresh_launchers() {
    let _ = Command::new("update-desktop-database").arg(launchers_dir()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Résultat d'une installation.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub app: Installed,
    /// Ancienne version de la même appli (même identifiant, autre fichier), plus dans le menu :
    /// Logiciels propose de la mettre à la corbeille.
    pub previous: Option<PathBuf>,
}

/// Installe l'AppImage inspectée : déplacement dans `~/Applications` (sauf si elle y est
/// déjà), droit d'exécution, icône et lanceur du menu Démarrer. Une AppImage du même nom de
/// fichier déjà présente est remplacée (mise à jour).
pub fn install(insp: &Inspection) -> Result<Outcome> {
    let dir = apps_dir();
    fs::create_dir_all(&dir).with_context(|| format!("création de {}", dir.display()))?;
    let dest = if insp.path.parent() == Some(dir.as_path()) { insp.path.clone() } else { dir.join(safe_file_name(&insp.path)) };
    if dest != insp.path {
        move_file(&insp.path, &dest)?;
    }
    let mut perm = fs::metadata(&dest)?.permissions();
    perm.set_mode(perm.mode() | 0o755);
    fs::set_permissions(&dest, perm)?;

    // Identifiant stable d'une version à l'autre : StartupWMClass du lanceur, sinon le nom.
    let id = slug(insp.entry.as_ref().and_then(|e| e.get("StartupWMClass")).unwrap_or(&insp.name));
    let icon = match &insp.icon {
        Some(src) => {
            let ext = match fs::read(src).ok().as_deref() {
                Some([0x89, b'P', b'N', b'G', ..]) => "png",
                Some(b) if b.starts_with(b"/* XPM") => "xpm",
                _ => "svg",
            };
            let target = icons_dir().join(format!("{id}.{ext}"));
            fs::create_dir_all(icons_dir())?;
            fs::copy(src, &target).ok().map(|_| target)
        }
        None => None,
    };

    // Lanceur : clés d'origine (noms traduits, catégories, types de fichiers…) ; Exec, TryExec
    // et Icon pointent vers le fichier installé.
    let args = insp
        .entry
        .as_ref()
        .and_then(|e| e.get("Exec"))
        .map(|exec| exec.split_once(char::is_whitespace).map_or("", |(_, rest)| rest).trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "%U".into());
    let mut text = String::from("[Desktop Entry]\nType=Application\n");
    let skip = ["Type", "Exec", "TryExec", "Icon", "Actions", "DBusActivatable", "Hidden", "NoDisplay", "X-AppImage-Integrate", KEY];
    let mut has_name = false;
    if let Some(e) = &insp.entry {
        for (k, v) in e.pairs() {
            let base = k.split('[').next().unwrap_or(k);
            if skip.contains(&base) || v.contains('\n') {
                continue;
            }
            has_name |= k == "Name";
            text.push_str(&format!("{k}={v}\n"));
        }
    }
    if !has_name {
        text.push_str(&format!("Name={}\n", insp.name));
    }
    text.push_str(&format!("Exec={} {args}\n", exec_quote(&dest)));
    text.push_str(&format!("TryExec={}\n", dest.display()));
    text.push_str(&format!("Icon={}\n", icon.as_ref().map_or("application-x-executable".into(), |p| p.display().to_string())));
    text.push_str(&format!("{KEY}={}\n", dest.display()));

    // Une seule entrée par fichier : retire un ancien lanceur pointant vers ce chemin.
    for old in installed().into_iter().filter(|i| i.path == dest) {
        let _ = fs::remove_file(&old.launcher);
    }
    let launcher = launchers_dir().join(format!("appimage-{id}.desktop"));
    let previous = installed().into_iter().find(|i| i.launcher == launcher && i.path != dest && i.path.is_file()).map(|i| i.path);
    fs::create_dir_all(launchers_dir())?;
    let tmp = launcher.with_extension("desktop.tmp");
    fs::write(&tmp, &text)?;
    fs::rename(&tmp, &launcher)?;
    refresh_launchers();

    let app = Installed {
        path: dest,
        launcher,
        name: insp.name.clone(),
        comment: insp.comment.clone(),
        version: insp.version.clone(),
        icon,
    };
    Ok(Outcome { app, previous })
}

/// Retire l'AppImage du menu (lanceur et icône). Le fichier lui-même est laissé à
/// l'appelant (Logiciels le met à la corbeille).
pub fn unintegrate(app: &Installed) -> Result<()> {
    fs::remove_file(&app.launcher).with_context(|| format!("suppression de {}", app.launcher.display()))?;
    if let Some(icon) = app.icon.as_ref().filter(|i| i.starts_with(icons_dir())) {
        let _ = fs::remove_file(icon);
    }
    refresh_launchers();
    Ok(())
}

/// Lance une AppImage directement (sans l'installer) : la rend exécutable si besoin.
pub fn run(path: &Path) -> Result<()> {
    let mut perm = fs::metadata(path)?.permissions();
    if perm.mode() & 0o111 == 0 {
        perm.set_mode(perm.mode() | 0o755);
        fs::set_permissions(path, perm).context("impossible de rendre le fichier exécutable")?;
    }
    let mut child = Command::new(path)
        .current_dir(home())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("lancement de {}", path.display()))?;
    // Attendu dans un fil à part : pas de processus zombie tant que Logiciels reste ouvert.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_depuis_le_fichier() {
        assert_eq!(name_from_file(Path::new("/x/Obsidian-1.6.7.AppImage")), "Obsidian");
        assert_eq!(name_from_file(Path::new("/x/Cursor-0.42.3-x86_64.AppImage")), "Cursor");
        assert_eq!(name_from_file(Path::new("/x/balena_etcher_v1.19.AppImage")), "balena etcher");
        assert_eq!(name_from_file(Path::new("/x/MonAppli.AppImage")), "MonAppli");
        assert_eq!(name_from_file(Path::new("/x/Mon-viewer.AppImage")), "Mon viewer");
        assert_eq!(name_from_file(Path::new("/x/Kudu-x86_64.AppImage")), "Kudu");
    }

    #[test]
    fn lanceur_traduit() {
        let e = Entry::parse("[Desktop Entry]\nName=Editor\nName[fr]=Éditeur\nExec=editor %F\n[Desktop Action new]\nName=New\n");
        assert_eq!(e.get("Name"), Some("Editor"));
        assert_eq!(e.get("Exec"), Some("editor %F"));
        // Les actions ne remplacent pas les clés du groupe principal.
        assert_eq!(e.0.len(), 3);
    }

    #[test]
    fn identifiants_et_exec() {
        assert_eq!(slug("org.Exemple.App"), "org-exemple-app");
        assert_eq!(slug("  "), "app");
        assert_eq!(safe_file_name(Path::new("/x/a$b%c.AppImage")), "a_b_c.AppImage");
        assert_eq!(exec_quote(Path::new("/home/a/Applications/Mon App.AppImage")), "\"/home/a/Applications/Mon App.AppImage\"");
        assert_eq!(exec_quote(Path::new("/home/a/Applications/x.AppImage")), "/home/a/Applications/x.AppImage");
    }

    #[test]
    fn pas_une_appimage() {
        let dir = TempDir::new().unwrap();
        let f = dir.0.join("faux.AppImage");
        fs::write(&f, b"\x7fELF\x02\x01\x01\x00\x00\x00\x00rien").unwrap();
        assert!(!is_appimage(&f));
        fs::write(&f, b"\x7fELF\x02\x01\x01\x00AI\x02reste").unwrap();
        assert_eq!(kind(&f), Some(2));
    }
}

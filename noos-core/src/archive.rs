//! Reconnaissance des archives (extensions, nom sans extension), partagée par l'application
//! Archives et l'Explorateur.

use std::path::Path;

/// Suffixes « tar compressé » lus par libarchive.
pub const TAR_SUFFIXES: &[&str] = &[
    ".tar", ".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst", ".tar.zstd", ".tar.lz4", ".tar.lz",
    ".tar.lzma", ".tlz", ".tar.z", ".taz", ".tar.lzo", ".tar.lrz", ".tar.br",
];

/// Extensions reconnues comme archives (ouverture, menu « Extraire »).
pub const EXTENSIONS: &[&str] = &[
    "7z", "zip", "zipx", "jar", "war", "ear", "apk", "xpi", "epub", "cbz", "cb7", "cbr", "cbt", "docx", "xlsx", "pptx", "odt", "ods", "odp",
    "rar", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst", "lz4", "lz", "lzma", "tlz", "z", "taz", "lzo", "br", "iso",
    "img", "udf", "dmg", "wim", "swm", "esd", "cab", "msi", "rpm", "deb", "udeb", "cpio", "squashfs", "sqfs", "snap", "vhd", "vhdx", "vmdk",
    "vdi", "qcow2", "arj", "lzh", "lha", "xar", "pkg", "chm", "nsis", "ar", "a", "appimage", "sit", "sitx", "alz", "arc", "zoo", "ace",
    "cpt", "001",
];

pub fn lower_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_archive(path: &Path) -> bool {
    let n = lower_name(path);
    TAR_SUFFIXES.iter().any(|s| n.ends_with(s)) || path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Nom sans extension d'archive (« photos.tar.gz » → « photos »).
pub fn stem(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
    let low = name.to_lowercase();
    if let Some(s) = TAR_SUFFIXES.iter().filter(|s| low.ends_with(*s)).max_by_key(|s| s.len()) {
        return name[..name.len() - s.len()].to_string();
    }
    for multi in [".7z.001", ".zip.001"] {
        if low.ends_with(multi) {
            return name[..name.len() - multi.len()].to_string();
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name,
    }
}


//! Décodage des images dans un fil de travail (gdk-pixbuf) : orientation EXIF appliquée,
//! rotation et miroir demandés par l'utilisateur. Le résultat (pixels bruts) est transformé en
//! texture sur le fil de l'interface.

use std::path::{Path, PathBuf};

use gtk::{gdk, gdk_pixbuf, gio, glib, prelude::*};

/// Pixels décodés, transférables entre fils.
pub struct Decoded {
    bytes: glib::Bytes,
    pub width: i32,
    pub height: i32,
    stride: usize,
    alpha: bool,
    pub format: String,
}

impl Decoded {
    pub fn texture(&self) -> gdk::Texture {
        let fmt = if self.alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
        gdk::MemoryTexture::new(self.width, self.height, fmt, &self.bytes, self.stride).upcast()
    }
}

/// `rotation` en quarts de tour dans le sens horaire (0 à 3).
fn decode_blocking(path: &Path, rotation: u32, flip: bool) -> Result<Decoded, String> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file(path).map_err(|e| e.message().to_string())?;
    let format = gdk_pixbuf::Pixbuf::file_info(path)
        .and_then(|(f, _, _)| f.description().map(|d| d.to_string()))
        .unwrap_or_default();
    let mut pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let angle = match rotation % 4 {
        1 => Some(gdk_pixbuf::PixbufRotation::Clockwise),
        2 => Some(gdk_pixbuf::PixbufRotation::Upsidedown),
        3 => Some(gdk_pixbuf::PixbufRotation::Counterclockwise),
        _ => None,
    };
    if let Some(a) = angle {
        pixbuf = pixbuf.rotate_simple(a).ok_or("rotation impossible")?;
    }
    if flip {
        pixbuf = pixbuf.flip(true).ok_or("miroir impossible")?;
    }
    Ok(Decoded {
        bytes: pixbuf.read_pixel_bytes(),
        width: pixbuf.width(),
        height: pixbuf.height(),
        stride: pixbuf.rowstride() as usize,
        alpha: pixbuf.has_alpha(),
        format,
    })
}

pub async fn decode(path: PathBuf, rotation: u32, flip: bool) -> Result<Decoded, String> {
    gio::spawn_blocking(move || decode_blocking(&path, rotation, flip)).await.unwrap_or_else(|_| Err("décodage interrompu".into()))
}

/// Extensions reconnues (rapide : pas besoin d'ouvrir les fichiers pour lister un dossier).
const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "webp", "gif", "bmp", "tif", "tiff", "svg", "svgz", "avif", "heic", "heif", "jxl", "ico", "tga",
    "ppm", "pgm", "pbm", "pnm", "xpm", "qoi",
    // Photos RAW des appareils (libopenraw-pixbuf-loader) : Nikon, Canon, Sony, Adobe DNG,
    // Fujifilm, Olympus, Panasonic, Pentax, Samsung, Minolta, Sigma, Kodak, Hasselblad…
    "nef", "nrw", "cr2", "cr3", "crw", "arw", "sr2", "srf", "dng", "raf", "orf", "rw2", "pef", "srw", "mrw", "x3f", "dcr", "kdc", "3fr", "erf", "mef", "mos",
];

pub fn is_image(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Images du dossier, triées comme dans l'Explorateur (ordre naturel des noms).
pub fn siblings(dir: &Path, show_hidden: bool) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| {
        is_image(p)
            && p.is_file()
            && (show_hidden || !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')))
    });
    v.sort_by_cached_key(|p| glib::FilenameCollationKey::from(p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    v
}

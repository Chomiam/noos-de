//! Miniatures (spécification freedesktop « Thumbnail Managing Standard »).
//!
//! Les miniatures 256 px sont écrites dans `~/.cache/thumbnails/large/<md5(uri)>.png`, avec
//! `Thumb::URI` et `Thumb::MTime` : elles sont partagées avec les autres applications (et
//! réutilisées par elles). Un seul fil de travail les produit, dans l'ordre des demandes :
//! - images : gdk-pixbuf (PNG, JPEG, WebP, GIF, BMP, TIFF, SVG… selon les modules installés) ;
//! - vidéos : `ffmpegthumbnailer`, s'il est installé.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use gtk::{gdk_pixbuf, gio, glib, prelude::*};

const SIZE: i32 = 256;
/// Au-delà, on ne lit pas l'image (trop lent pour une miniature).
const MAX_IMAGE_BYTES: u64 = 80 * 1024 * 1024;

struct Job {
    uri: String,
    path: PathBuf,
    mtime: i64,
    video: bool,
    reply: async_channel::Sender<Option<PathBuf>>,
}

fn queue() -> &'static async_channel::Sender<Job> {
    static QUEUE: OnceLock<async_channel::Sender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = async_channel::unbounded::<Job>();
        std::thread::Builder::new()
            .name("miniatures".into())
            .spawn(move || {
                while let Ok(job) = rx.recv_blocking() {
                    let res = generate(&job);
                    let _ = job.reply.send_blocking(res);
                }
            })
            .expect("fil des miniatures");
        tx
    })
}

/// Chemin de la miniature d'un URI dans le cache partagé.
pub fn cache_path(uri: &str) -> PathBuf {
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, uri).unwrap_or_default();
    glib::user_cache_dir().join("thumbnails").join("large").join(format!("{md5}.png"))
}

/// Une miniature à jour existe-t-elle déjà (plus récente que le fichier) ?
fn fresh(thumb: &Path, mtime: i64) -> bool {
    std::fs::metadata(thumb)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|t| t.as_secs() as i64 >= mtime)
}

fn generate(job: &Job) -> Option<PathBuf> {
    let out = cache_path(&job.uri);
    if fresh(&out, job.mtime) {
        return Some(out);
    }
    std::fs::create_dir_all(out.parent()?).ok()?;
    let tmp = out.with_extension(format!("{}.tmp", std::process::id()));
    if job.video {
        let ok = std::process::Command::new("ffmpegthumbnailer")
            .arg("-i")
            .arg(&job.path)
            .arg("-o")
            .arg(&tmp)
            .args(["-s", &SIZE.to_string(), "-c", "png", "-f"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return None;
        }
    } else {
        let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(&job.path, SIZE, SIZE, true).ok()?;
        // Orientation EXIF des photos.
        let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
        let mtime = job.mtime.to_string();
        pixbuf
            .savev(&tmp, "png", &[("tEXt::Thumb::URI", job.uri.as_str()), ("tEXt::Thumb::MTime", mtime.as_str())])
            .ok()?;
    }
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

fn has_ffmpegthumbnailer() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        std::env::var_os("PATH")
            .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("ffmpegthumbnailer").is_file()))
    })
}

/// Le fichier peut-il avoir une miniature ? (image, ou vidéo si ffmpegthumbnailer est là)
pub fn supported(info: &gio::FileInfo) -> Option<bool> {
    let ct = info.content_type()?;
    let mime = gio::content_type_get_mime_type(&ct)?.to_string();
    if mime.starts_with("image/") && info.size() as u64 <= MAX_IMAGE_BYTES {
        return Some(false);
    }
    if mime.starts_with("video/") && has_ffmpegthumbnailer() {
        return Some(true);
    }
    None
}

/// Miniature d'un fichier local : chemin du PNG (cache), produite si besoin en arrière-plan.
pub async fn thumbnail(file: &gio::File, info: &gio::FileInfo) -> Option<PathBuf> {
    let video = supported(info)?;
    let path = file.path()?;
    let uri = file.uri().to_string();
    let mtime = info.modification_date_time().map(|d| d.to_unix()).unwrap_or(0);
    let cached = cache_path(&uri);
    if fresh(&cached, mtime) {
        return Some(cached);
    }
    let (tx, rx) = async_channel::bounded(1);
    queue().send(Job { uri, path, mtime, video, reply: tx }).await.ok()?;
    rx.recv().await.ok().flatten()
}

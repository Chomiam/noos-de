//! Miniatures de la bande du bas : cache freedesktop partagé (`~/.cache/thumbnails/large`),
//! le même que l'Explorateur de NOOS. Un seul fil de travail les produit.

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use gtk::{gdk_pixbuf, gio, glib, prelude::*};

struct Job {
    path: PathBuf,
    reply: async_channel::Sender<Option<PathBuf>>,
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cache_path(path: &Path) -> Option<(String, PathBuf)> {
    let uri = gio::File::for_path(path).uri().to_string();
    let md5 = glib::compute_checksum_for_string(glib::ChecksumType::Md5, &uri)?;
    Some((uri, glib::user_cache_dir().join("thumbnails").join("large").join(format!("{md5}.png"))))
}

fn generate(path: &Path) -> Option<PathBuf> {
    let (uri, out) = cache_path(path)?;
    let source_time = mtime(path);
    if out.exists() && mtime(&out) >= source_time {
        return Some(out);
    }
    std::fs::create_dir_all(out.parent()?).ok()?;
    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(path, 256, 256, true).ok()?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let tmp = out.with_extension(format!("{}.tmp", std::process::id()));
    let mt = source_time.to_string();
    pixbuf.savev(&tmp, "png", &[("tEXt::Thumb::URI", uri.as_str()), ("tEXt::Thumb::MTime", mt.as_str())]).ok()?;
    std::fs::rename(&tmp, &out).ok()?;
    Some(out)
}

fn queue() -> &'static async_channel::Sender<Job> {
    static QUEUE: OnceLock<async_channel::Sender<Job>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (tx, rx) = async_channel::unbounded::<Job>();
        std::thread::Builder::new()
            .name("miniatures".into())
            .spawn(move || {
                while let Ok(job) = rx.recv_blocking() {
                    let _ = job.reply.send_blocking(generate(&job.path));
                }
            })
            .expect("fil des miniatures");
        tx
    })
}

pub async fn thumbnail(path: PathBuf) -> Option<PathBuf> {
    if let Some((_, out)) = cache_path(&path) {
        if out.exists() && mtime(&out) >= mtime(&path) {
            return Some(out);
        }
    }
    let (tx, rx) = async_channel::bounded(1);
    queue().send(Job { path, reply: tx }).await.ok()?;
    rx.recv().await.ok().flatten()
}

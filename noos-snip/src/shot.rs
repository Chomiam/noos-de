//! Image capturée : pixels BGRA prémultipliés (format mémoire de GDK et de cairo ARGB32).

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use gtk::{cairo, gdk, glib, prelude::*};
use noos_core::ipc::{IpcClient, Request};

#[derive(Clone)]
pub struct Shot {
    pub w: i32,
    pub h: i32,
    pub data: Vec<u8>,
}

impl Shot {
    pub fn stride(&self) -> usize {
        self.w as usize * 4
    }

    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let tex = gdk::Texture::from_filename(path).with_context(|| format!("lecture de {}", path.display()))?;
        let (w, h) = (tex.width(), tex.height());
        let mut data = vec![0u8; w as usize * h as usize * 4];
        tex.download(&mut data, w as usize * 4);
        Ok(Self { w, h, data })
    }

    pub fn crop(&self, x: i32, y: i32, w: i32, h: i32) -> Self {
        let x = x.clamp(0, self.w - 1);
        let y = y.clamp(0, self.h - 1);
        let w = w.clamp(1, self.w - x);
        let h = h.clamp(1, self.h - y);
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for row in y..y + h {
            let start = row as usize * self.stride() + x as usize * 4;
            data.extend_from_slice(&self.data[start..start + w as usize * 4]);
        }
        Self { w, h, data }
    }

    pub fn texture(&self) -> gdk::Texture {
        gdk::MemoryTexture::new(
            self.w,
            self.h,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &glib::Bytes::from(&self.data[..]),
            self.stride(),
        )
        .upcast()
    }

    pub fn surface(&self) -> cairo::ImageSurface {
        cairo::ImageSurface::create_for_data(self.data.clone(), cairo::Format::ARgb32, self.w, self.h, self.stride() as i32)
            .expect("surface cairo")
    }

    pub fn from_surface(surface: cairo::ImageSurface) -> Self {
        let (w, h, stride) = (surface.width(), surface.height(), surface.stride() as usize);
        surface.flush();
        let owned = surface.take_data().expect("données de la surface");
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for row in 0..h as usize {
            data.extend_from_slice(&owned[row * stride..row * stride + w as usize * 4]);
        }
        Self { w, h, data }
    }

    pub fn save_png(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        self.texture().save_to_png(path).with_context(|| format!("écriture de {}", path.display()))?;
        Ok(())
    }
}

/// Capture de tout le bureau (tous les écrans) dans un fichier temporaire.
/// grim (protocole ext-image-copy-capture) en priorité ; sinon l'IPC du compositeur.
pub fn grab() -> anyhow::Result<Shot> {
    let dir = glib::user_runtime_dir();
    let path = dir.join(format!("noos-snip-{}.png", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let grim_ok = std::process::Command::new("grim")
        .args(["-t", "png", "-l", "1"])
        .arg(&path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !grim_ok {
        IpcClient::fire(&Request::Screenshot { path: path.to_string_lossy().into() })
            .context("ni grim ni le compositeur NOOS ne sont disponibles")?;
        // Le compositeur écrit le fichier au rendu suivant.
        let mut waited = 0;
        while waited < 3000 && std::fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            waited += 50;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !path.exists() {
        bail!("capture d'écran impossible");
    }
    let shot = Shot::load(&path);
    let _ = std::fs::remove_file(&path);
    shot
}

/// ~/Images/Captures d'écran/Capture d'écran 2026-11-05 184512.png (comme Windows).
pub fn auto_path() -> PathBuf {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures)
        .unwrap_or_else(|| glib::home_dir().join("Images"));
    let stamp = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d %H%M%S").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "capture".into());
    pictures.join("Captures d'écran").join(format!("Capture d'écran {stamp}.png"))
}

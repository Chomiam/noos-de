//! Curseur logiciel (thème XCursor), utilisé par le backend DRM.

use std::{cell::RefCell, collections::HashMap, time::Duration};

use smithay::input::pointer::CursorIcon;
use tracing::warn;
use xcursor::{
    CursorTheme,
    parser::{Image, parse_xcursor},
};

/// Tous les curseurs (flèche, texte, main, redimensionnement…) viennent du même thème et de la
/// même taille : ceux demandés par les applis via `cursor-shape-v1` comme celui par défaut.
pub struct Cursor {
    theme: CursorTheme,
    size: u32,
    default: Vec<Image>,
    cache: RefCell<HashMap<CursorIcon, Vec<Image>>>,
}

impl Cursor {
    pub fn load() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Adwaita".into());
        let size = std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
        let theme = CursorTheme::load(&name);
        let default = load_named(&theme, &["default", "left_ptr"])
            .or_else(|| load_named(&CursorTheme::load("default"), &["default", "left_ptr"]))
            .unwrap_or_else(|| {
                warn!(theme = name, "thème de curseur introuvable, flèche de secours");
                vec![fallback_arrow()]
            });
        Self { theme, size, default, cache: RefCell::new(HashMap::new()) }
    }

    /// Image du curseur `icon` (flèche si le thème ne le fournit pas).
    pub fn get_image(&self, icon: CursorIcon, scale: u32, time: Duration) -> Image {
        if icon == CursorIcon::Default {
            return frame(time.as_millis() as u32, self.size * scale, &self.default);
        }
        let mut cache = self.cache.borrow_mut();
        let images = cache.entry(icon).or_insert_with(|| {
            let mut names = vec![icon.name()];
            names.extend(icon.alt_names().iter().copied());
            load_named(&self.theme, &names).unwrap_or_default()
        });
        let images = if images.is_empty() { &self.default } else { &*images };
        frame(time.as_millis() as u32, self.size * scale, images)
    }
}

fn load_named(theme: &CursorTheme, names: &[&str]) -> Option<Vec<Image>> {
    let path = names.iter().find_map(|n| theme.load_icon(n))?;
    let data = std::fs::read(path).ok()?;
    parse_xcursor(&data).filter(|v| !v.is_empty())
}

fn nearest_images(size: u32, images: &[Image]) -> impl Iterator<Item = &Image> {
    let nearest = images.iter().min_by_key(|i| (size as i32 - i.size as i32).abs()).unwrap();
    images.iter().filter(move |i| i.width == nearest.width && i.height == nearest.height)
}

fn frame(mut millis: u32, size: u32, images: &[Image]) -> Image {
    let total = nearest_images(size, images).fold(0, |acc, i| acc + i.delay);
    if total == 0 {
        return nearest_images(size, images).next().unwrap().clone();
    }
    millis %= total;
    for img in nearest_images(size, images) {
        if millis < img.delay {
            return img.clone();
        }
        millis -= img.delay;
    }
    unreachable!()
}

/// Petite flèche blanche bordée de noir, générée en mémoire.
fn fallback_arrow() -> Image {
    const W: u32 = 24;
    let mut px = vec![0u8; (W * W * 4) as usize];
    for y in 0..20u32 {
        for x in 0..=y.min(13) {
            let edge = x == 0 || x == y.min(13) || y == 19;
            let i = ((y * W + x) * 4) as usize;
            let c = if edge { 0 } else { 255 };
            px[i..i + 4].copy_from_slice(&[c, c, c, 255]);
        }
    }
    Image {
        size: W,
        width: W,
        height: W,
        xhot: 0,
        yhot: 0,
        delay: 0,
        pixels_rgba: px,
        pixels_argb: vec![],
    }
}

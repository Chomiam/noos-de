//! Flou « Mica » sous les panneaux translucides du shell (barre des tâches, menu Démarrer,
//! réglages rapides, centre de notifications), comme Windows 11.
//!
//! Le fond d'écran (couche « background ») est capturé, réduit 8× et flouté sur le
//! processeur — une seule fois par changement de fond d'écran. Sous chaque panneau, on
//! affiche ensuite la portion correspondante, agrandie (bilinéaire) et aux coins arrondis.
//! Coût par image : nul (simple texture), et aucune dépendance aux fenêtres en dessous.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
        },
    },
    desktop::{LayerSurface, layer_map_for_output},
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Transform},
    wayland::shell::wlr_layer::Layer,
};

use crate::{render::NoosElement, state::NoosState};

/// Réduction du fond avant flou.
const DOWNSCALE: usize = 8;
/// Panneaux floutés : espace de noms layer-shell, coins arrondis ?
const BLUR_LAYERS: &[(&str, bool)] = &[
    ("noos-taskbar", false),
    ("noos-startmenu", true),
    ("noos-quicksettings", true),
    ("noos-notification-center", true),
];

struct Wallpaper {
    generation: u64,
    computed_at: std::time::Instant,
    w: usize,
    h: usize,
    /// RGBA réduit et flouté.
    pixels: Vec<f32>,
    /// Même image agrandie à la taille réelle de l'écran (RGBA 8 bits), calculée à la
    /// première demande : un panneau ou une fenêtre n'est ensuite qu'une simple découpe.
    full: Option<(usize, usize, Vec<u8>)>,
    /// Image pleine taille en texture : fenêtres en mouvement (simple découpe, aucun calcul).
    full_buffer: Option<MemoryRenderBuffer>,
}

#[derive(PartialEq, Clone, Copy)]
struct PanelKey {
    generation: u64,
    geo: Rectangle<i32, Logical>,
    scale_milli: i64,
    radius: i32,
}

#[derive(Default)]
pub struct BlurState {
    /// Incrémenté à chaque modification du fond d'écran.
    pub generation: u64,
    wallpapers: HashMap<String, Wallpaper>,
    /// (identifiant du panneau ou de la fenêtre, écran) → tampon flouté.
    panels: HashMap<(String, String), (PanelKey, MemoryRenderBuffer)>,
}

pub fn wants_blur(layer: &LayerSurface) -> Option<bool> {
    let ns = layer.namespace();
    BLUR_LAYERS.iter().find(|(n, _)| *n == ns).map(|(_, rounded)| *rounded)
}

impl NoosState {
    /// Fond flouté à placer juste sous `layer` (None si pas de flou pour ce panneau).
    pub fn blur_element(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        layer: &LayerSurface,
        geo: Rectangle<i32, Logical>,
    ) -> Option<NoosElement> {
        let rounded = wants_blur(layer)?;
        let radius = if rounded { self.config.appearance.corner_radius as i32 } else { 0 };
        self.blur_for(renderer, output, format!("layer:{}", layer.namespace()), geo, radius, false)
    }

    /// Fond flouté (Mica) sous une fenêtre des applications NOOS (fond translucide).
    pub fn window_blur_element(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        window: &smithay::desktop::Window,
        geo: Rectangle<i32, Logical>,
        moving: bool,
    ) -> Option<NoosElement> {
        let toplevel = window.toplevel()?;
        let app_id = smithay::wayland::compositor::with_states(toplevel.wl_surface(), |states| {
            states
                .data_map
                .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().unwrap().app_id.clone())
        })?;
        if !app_id.starts_with("org.noos.") {
            return None;
        }
        let d = crate::state::window_data(window);
        if d.fullscreen.get() {
            return None;
        }
        let radius = if d.maximized.get() { 0 } else { self.config.appearance.corner_radius as i32 };
        self.blur_for(renderer, output, format!("window:{}", d.id), geo, radius, moving)
    }

    fn blur_for(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        id: String,
        geo: Rectangle<i32, Logical>,
        radius: i32,
        moving: bool,
    ) -> Option<NoosElement> {
        if self.config.appearance.panel_opacity >= 0.99 || geo.size.w <= 0 || geo.size.h <= 0 {
            return None;
        }
        self.ensure_wallpaper(renderer, output)?;
        let scale = output.current_scale().fractional_scale();
        let loc: Point<f64, Physical> = geo.loc.to_f64().to_physical(scale).to_i32_round::<i32>().to_f64();
        if moving {
            // Fenêtre déplacée ou animée : découpe de l'image pleine taille (coins carrés, invisibles
            // à cette vitesse). La version arrondie est recalculée une fois la fenêtre arrêtée.
            let wall = self.blur.wallpapers.get_mut(&output.name())?;
            let buffer = full_buffer(wall);
            let src = Rectangle::<f64, Logical>::new(
                (geo.loc.x as f64 * scale, geo.loc.y as f64 * scale).into(),
                (geo.size.w as f64 * scale, geo.size.h as f64 * scale).into(),
            );
            return MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, Some(src), Some(geo.size), Kind::Unspecified)
                .ok()
                .map(NoosElement::Memory);
        }
        let wall_generation = self.blur.wallpapers.get(&output.name())?.generation;
        let key = PanelKey { generation: wall_generation, geo, scale_milli: (scale * 1000.0) as i64, radius };
        let id = (id, output.name());
        let cached = self.blur.panels.get(&id).filter(|(k, _)| *k == key).map(|(_, b)| b.clone());
        let buffer = match cached {
            Some(b) => b,
            None => {
                let wall = self.blur.wallpapers.get_mut(&output.name())?;
                let b = panel_buffer(wall, geo, scale, radius as f64 * scale);
                self.blur.panels.insert(id, (key, b.clone()));
                b
            }
        };
        MemoryRenderBufferRenderElement::from_buffer(renderer, loc, &buffer, None, None, Some(geo.size), Kind::Unspecified)
            .ok()
            .map(NoosElement::Memory)
    }

    /// Oublie les tampons des fenêtres fermées.
    pub fn blur_forget_window(&mut self, id: u64) {
        let key = format!("window:{id}");
        self.blur.panels.retain(|(k, _), _| *k != key);
    }

    /// (Re)calcule le fond flouté de l'écran si le fond d'écran a changé.
    fn ensure_wallpaper(&mut self, renderer: &mut GlesRenderer, output: &Output) -> Option<()> {
        let generation = self.blur.generation;
        // À jour, ou recalculé il y a moins d'une seconde (fond d'écran animé ou qui se
        // redessine souvent : on garde l'ancien flou un instant).
        if self
            .blur
            .wallpapers
            .get(&output.name())
            .is_some_and(|w| w.generation == generation || w.computed_at.elapsed().as_millis() < 1000)
        {
            return Some(());
        }
        let size = crate::capture::buffer_size(output)?;
        let scale = output.current_scale().fractional_scale();
        let elements: Vec<NoosElement> = {
            let map = layer_map_for_output(output);
            map.layers_on(Layer::Background)
                .rev()
                .filter_map(|l| map.layer_geometry(l).map(|g| (g, l)))
                .flat_map(|(g, l)| {
                    smithay::backend::renderer::element::AsRenderElements::<GlesRenderer>::render_elements::<
                        WaylandSurfaceRenderElement<GlesRenderer>,
                    >(l, renderer, g.loc.to_physical_precise_round(scale), scale.into(), 1.0)
                })
                .map(NoosElement::Surface)
                .collect()
        };
        if elements.is_empty() {
            return None;
        }
        let refs: Vec<&NoosElement> = elements.iter().collect();
        let rgba = crate::capture::render_offscreen(renderer, size, scale, &refs, [0.0, 0.0, 0.0, 1.0])?;
        let (w, h, pixels) = blur_small(&rgba, size.w as usize, size.h as usize);
        self.blur.wallpapers.insert(output.name(), Wallpaper { generation, computed_at: std::time::Instant::now(), w, h, pixels, full: None, full_buffer: None });
        Some(())
    }
}

/// Réduction 8× (moyenne), puis 3 passes de flou boîte séparable (≈ flou gaussien),
/// avec une légère hausse de saturation comme le Mica de Windows.
fn blur_small(rgba: &[u8], width: usize, height: usize) -> (usize, usize, Vec<f32>) {
    let w = width.div_ceil(DOWNSCALE).max(1);
    let h = height.div_ceil(DOWNSCALE).max(1);
    let mut small = vec![0f32; w * h * 4];
    for sy in 0..h {
        for sx in 0..w {
            let mut acc = [0f32; 4];
            let mut n = 0f32;
            for y in sy * DOWNSCALE..((sy + 1) * DOWNSCALE).min(height) {
                for x in sx * DOWNSCALE..((sx + 1) * DOWNSCALE).min(width) {
                    let i = (y * width + x) * 4;
                    for c in 0..4 {
                        acc[c] += rgba[i + c] as f32;
                    }
                    n += 1.0;
                }
            }
            let o = (sy * w + sx) * 4;
            for c in 0..4 {
                small[o + c] = acc[c] / n.max(1.0);
            }
        }
    }
    let radius = 3isize;
    let mut tmp = vec![0f32; small.len()];
    for _ in 0..3 {
        box_pass(&small, &mut tmp, w, h, radius, true);
        box_pass(&tmp, &mut small, w, h, radius, false);
    }
    for px in small.chunks_exact_mut(4) {
        let l = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
        for c in 0..3 {
            px[c] = (l + (px[c] - l) * 1.25).clamp(0.0, 255.0);
        }
        px[3] = 255.0;
    }
    (w, h, small)
}

fn box_pass(src: &[f32], dst: &mut [f32], w: usize, h: usize, r: isize, horizontal: bool) {
    let (len, lines) = if horizontal { (w, h) } else { (h, w) };
    let idx = |line: usize, i: usize| if horizontal { (line * w + i) * 4 } else { (i * w + line) * 4 };
    let norm = 1.0 / (2 * r + 1) as f32;
    for line in 0..lines {
        for c in 0..4 {
            let at = |i: isize| src[idx(line, i.clamp(0, len as isize - 1) as usize) + c];
            let mut sum: f32 = (-r..=r).map(at).sum();
            for i in 0..len as isize {
                dst[idx(line, i as usize) + c] = sum * norm;
                sum += at(i + r + 1) - at(i - r);
            }
        }
    }
}

/// Portion du fond flouté sous un panneau, à sa taille réelle, coins arrondis.
/// Agrandit (bilinéaire) le fond réduit à la taille réelle de l'écran.
fn upscale(wall: &Wallpaper, width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    for y in 0..height {
        let fy = ((y as f64 + 0.5) / DOWNSCALE as f64 - 0.5).clamp(0.0, (wall.h - 1) as f64);
        let (y0, ty) = (fy.floor() as usize, (fy - fy.floor()) as f32);
        let y1 = (y0 + 1).min(wall.h - 1);
        for x in 0..width {
            let fx = ((x as f64 + 0.5) / DOWNSCALE as f64 - 0.5).clamp(0.0, (wall.w - 1) as f64);
            let (x0, tx) = (fx.floor() as usize, (fx - fx.floor()) as f32);
            let x1 = (x0 + 1).min(wall.w - 1);
            let i = (y * width + x) * 4;
            for c in 0..3 {
                let p = |x: usize, y: usize| wall.pixels[(y * wall.w + x) * 4 + c];
                let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
                let bottom = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
                out[i + c] = (top * (1.0 - ty) + bottom * ty).round() as u8;
            }
            out[i + 3] = 255;
        }
    }
    out
}

/// Image floutée pleine taille de l'écran, en texture (créée une fois par fond d'écran).
fn full_buffer(wall: &mut Wallpaper) -> MemoryRenderBuffer {
    if let Some(b) = &wall.full_buffer {
        return b.clone();
    }
    let fw = wall.w * DOWNSCALE;
    let fh = wall.h * DOWNSCALE;
    if wall.full.as_ref().is_none_or(|(w, h, _)| *w != fw || *h != fh) {
        let full = upscale(wall, fw, fh);
        wall.full = Some((fw, fh, full));
    }
    let (fw, fh, data) = wall.full.as_ref().unwrap();
    let b = MemoryRenderBuffer::from_slice(data, Fourcc::Abgr8888, (*fw as i32, *fh as i32), 1, Transform::Normal, None);
    wall.full_buffer = Some(b.clone());
    b
}

/// Portion du fond flouté sous un panneau ou une fenêtre, à sa taille réelle, coins arrondis.
/// Simple découpe de l'image pleine taille : assez rapide pour suivre une fenêtre déplacée.
fn panel_buffer(wall: &mut Wallpaper, geo: Rectangle<i32, Logical>, scale: f64, radius: f64) -> MemoryRenderBuffer {
    let fw = wall.w * DOWNSCALE;
    let fh = wall.h * DOWNSCALE;
    if wall.full.as_ref().is_none_or(|(w, h, _)| *w != fw || *h != fh) {
        let full = upscale(wall, fw, fh);
        wall.full = Some((fw, fh, full));
    }
    let (fw, fh, full) = wall.full.as_ref().unwrap();
    let (fw, fh) = (*fw, *fh);
    let pw = ((geo.size.w as f64 * scale).round() as usize).max(1);
    let ph = ((geo.size.h as f64 * scale).round() as usize).max(1);
    let ox = (geo.loc.x as f64 * scale).round() as isize;
    let oy = (geo.loc.y as f64 * scale).round() as isize;
    let mut data = vec![0u8; pw * ph * 4];
    for y in 0..ph {
        let sy = (oy + y as isize).clamp(0, fh as isize - 1) as usize;
        let row = &full[sy * fw * 4..(sy + 1) * fw * 4];
        let dst = &mut data[y * pw * 4..(y + 1) * pw * 4];
        // Partie visible à l'écran, puis bords répétés (fenêtre en partie hors écran).
        for x in 0..pw {
            let sx = (ox + x as isize).clamp(0, fw as isize - 1) as usize;
            dst[x * 4..x * 4 + 4].copy_from_slice(&row[sx * 4..sx * 4 + 4]);
        }
    }
    // Coins arrondis : seuls les pixels des coins sont touchés (prémultipliés).
    let r = radius.ceil() as usize;
    if r > 0 {
        for y in (0..r.min(ph)).chain(ph.saturating_sub(r)..ph) {
            for x in (0..r.min(pw)).chain(pw.saturating_sub(r)..pw) {
                let a = corner_coverage(x as f64 + 0.5, y as f64 + 0.5, pw as f64, ph as f64, radius);
                let i = (y * pw + x) * 4;
                for c in 0..4 {
                    data[i + c] = (data[i + c] as f64 * a).round() as u8;
                }
            }
        }
    }
    // Octets R,G,B,A = Abgr8888 en petit-boutiste.
    MemoryRenderBuffer::from_slice(&data, Fourcc::Abgr8888, (pw as i32, ph as i32), 1, Transform::Normal, None)
}

/// Couverture (0..1, anticrénelée) d'un rectangle aux coins arrondis.
fn corner_coverage(x: f64, y: f64, w: f64, h: f64, r: f64) -> f64 {
    if r <= 0.0 {
        return 1.0;
    }
    let cx = if x < r { r } else if x > w - r { w - r } else { return 1.0 };
    let cy = if y < r { r } else if y > h - r { h - r } else { return 1.0 };
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    (r - d + 0.5).clamp(0.0, 1.0)
}

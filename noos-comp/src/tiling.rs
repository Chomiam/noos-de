//! Ancrage des fenêtres (« Snap » façon Windows 11) :
//!
//! - **zones** : plein écran, moitiés, quarts, tiers, deux tiers ;
//! - **en glissant une fenêtre** : bords et coins de l'écran (Ctrl = tiers), et une
//!   **barre de dispositions** en haut de l'écran pour choisir une zone précise ;
//! - un **rectangle d'aperçu** translucide (animé) montre où la fenêtre va se placer ;
//! - les changements de place / taille (ancrage, maximisation, restauration) sont **animés** :
//!   l'image de la fenêtre glisse et s'étire vers sa nouvelle géométrie pendant que
//!   l'application redessine à la bonne taille.

use std::time::{Duration, Instant};

use smithay::{
    backend::renderer::element::{
        Kind,
        solid::{SolidColorBuffer, SolidColorRenderElement},
    },
    desktop::Window,
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size},
};

use crate::{
    render::NoosElement,
    state::{NoosState, window_data},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Zone {
    Maximize,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    LeftThird,
    CenterThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,
}

impl Zone {
    /// Fraction de la zone utile : (x0, y0, x1, y1).
    fn fractions(self) -> (f64, f64, f64, f64) {
        const T: f64 = 1.0 / 3.0;
        match self {
            Zone::Maximize => (0.0, 0.0, 1.0, 1.0),
            Zone::Left => (0.0, 0.0, 0.5, 1.0),
            Zone::Right => (0.5, 0.0, 1.0, 1.0),
            Zone::TopLeft => (0.0, 0.0, 0.5, 0.5),
            Zone::TopRight => (0.5, 0.0, 1.0, 0.5),
            Zone::BottomLeft => (0.0, 0.5, 0.5, 1.0),
            Zone::BottomRight => (0.5, 0.5, 1.0, 1.0),
            Zone::LeftThird => (0.0, 0.0, T, 1.0),
            Zone::CenterThird => (T, 0.0, 2.0 * T, 1.0),
            Zone::RightThird => (2.0 * T, 0.0, 1.0, 1.0),
            Zone::LeftTwoThirds => (0.0, 0.0, 2.0 * T, 1.0),
            Zone::RightTwoThirds => (T, 0.0, 1.0, 1.0),
        }
    }

    pub fn rect(self, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let (x0, y0, x1, y1) = self.fractions();
        let px = |f: f64| area.loc.x + (area.size.w as f64 * f).round() as i32;
        let py = |f: f64| area.loc.y + (area.size.h as f64 * f).round() as i32;
        Rectangle::new((px(x0), py(y0)).into(), (px(x1) - px(x0), py(y1) - py(y0)).into())
    }
}

/// Dispositions de la barre (comme le menu « Snap layouts » de Windows 11).
const LAYOUTS: &[&[Zone]] = &[
    &[Zone::Left, Zone::Right],
    &[Zone::LeftTwoThirds, Zone::RightThird],
    &[Zone::LeftThird, Zone::CenterThird, Zone::RightThird],
    &[Zone::Left, Zone::TopRight, Zone::BottomRight],
    &[Zone::TopLeft, Zone::TopRight, Zone::BottomLeft, Zone::BottomRight],
    &[Zone::Maximize],
];

const TILE_W: i32 = 84;
const TILE_H: i32 = 54;
const TILE_GAP: i32 = 12;
const BAR_PAD: i32 = 12;
const BAR_TOP: i32 = 10;
/// Distance du haut de l'écran à partir de laquelle la barre apparaît.
const BAR_TRIGGER: f64 = 110.0;
const EDGE: f64 = 6.0;
const CORNER: f64 = 64.0;
/// Distance à parcourir avant que les bords et coins ne réagissent (décrocher une fenêtre
/// ancrée ne doit pas la ré-ancrer aussitôt).
const ARM_DISTANCE: f64 = 24.0;

/// Géométrie de la barre : (fond, [(zone, rectangle)], cadres des vignettes).
fn bar_layout(output_geo: Rectangle<i32, Logical>) -> (Rectangle<i32, Logical>, Vec<(Zone, Rectangle<i32, Logical>)>, Vec<Rectangle<i32, Logical>>) {
    let n = LAYOUTS.len() as i32;
    let w = n * TILE_W + (n - 1) * TILE_GAP + 2 * BAR_PAD;
    let h = TILE_H + 2 * BAR_PAD;
    let bar = Rectangle::new((output_geo.loc.x + (output_geo.size.w - w) / 2, output_geo.loc.y + BAR_TOP).into(), (w, h).into());
    let mut zones = Vec::new();
    let mut tiles = Vec::new();
    for (i, layout) in LAYOUTS.iter().enumerate() {
        let tile = Rectangle::new(
            (bar.loc.x + BAR_PAD + i as i32 * (TILE_W + TILE_GAP), bar.loc.y + BAR_PAD).into(),
            (TILE_W, TILE_H).into(),
        );
        tiles.push(tile);
        for zone in layout.iter() {
            let r = zone.rect(tile);
            // Petit espace entre les zones d'une même vignette.
            let r = Rectangle::new((r.loc.x + 2, r.loc.y + 2).into(), ((r.size.w - 4).max(2), (r.size.h - 4).max(2)).into());
            zones.push((*zone, r));
        }
    }
    (bar, zones, tiles)
}

/// État pendant qu'une fenêtre est déplacée à la souris.
pub struct SnapDrag {
    pub window: Window,
    output: String,
    target: Option<Zone>,
    bar_visible: bool,
    /// Rectangle d'aperçu affiché (animé vers la cible).
    preview: Option<Rectangle<f64, Logical>>,
    last: Instant,
    buffers: Vec<SolidColorBuffer>,
    /// Point de départ du glisser.
    start: Point<f64, Logical>,
    /// Barre de dispositions activée : le pointeur est passé sous `BAR_TRIGGER` pendant ce
    /// glisser (une fenêtre ancrée a sa barre de titre tout en haut : sans cela, la barre et
    /// son aperçu se déclencheraient dès qu'on la décroche).
    bar_armed: bool,
    /// Bords et coins activés : le pointeur s'est éloigné du point de départ.
    edges_armed: bool,
}

/// Animation de déplacement / redimensionnement.
#[derive(Debug)]
pub struct Anim {
    from: Rectangle<f64, Logical>,
    to: Rectangle<i32, Logical>,
    start: Instant,
}

const ANIM: Duration = Duration::from_millis(240);

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp_rect(a: Rectangle<f64, Logical>, b: Rectangle<f64, Logical>, t: f64) -> Rectangle<f64, Logical> {
    let l = |x: f64, y: f64| x + (y - x) * t;
    Rectangle::new(
        (l(a.loc.x, b.loc.x), l(a.loc.y, b.loc.y)).into(),
        (l(a.size.w, b.size.w), l(a.size.h, b.size.h)).into(),
    )
}

impl Anim {
    /// Rectangle à afficher maintenant, ou None si l'animation est finie.
    fn current(&self, actual_size: Size<i32, Logical>) -> Option<Rectangle<f64, Logical>> {
        let elapsed = self.start.elapsed();
        let t = (elapsed.as_secs_f64() / ANIM.as_secs_f64()).min(1.0);
        // Fin : la fenêtre a pris sa nouvelle taille (ou l'appli tarde trop).
        // Fin à la durée prévue : au-delà, on affiche la fenêtre telle quelle à sa place
        // (pas d'image étirée si l'application tarde à se redessiner).
        let _ = actual_size;
        if t >= 1.0 {
            return None;
        }
        Some(lerp_rect(self.from, self.to.to_f64(), ease_out(t)))
    }
}

fn accent(state: &NoosState) -> [f32; 3] {
    let hex = state.config.appearance.accent.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("88"), 16).unwrap_or(0x88) as f32 / 255.0;
    if hex.len() >= 6 { [c(0), c(2), c(4)] } else { [0.54, 0.71, 0.98] }
}

impl NoosState {
    /// Rectangle actuel de la fenêtre (position dans l'espace + taille).
    pub fn window_rect(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        Some(Rectangle::new(self.space.element_location(window)?, window.geometry().size))
    }

    /// Lance l'animation de `window` depuis `from` vers `to`.
    pub fn animate_window(&self, window: &Window, from: Option<Rectangle<i32, Logical>>, to: Rectangle<i32, Logical>) {
        if !self.config.tiling.animations {
            window_data(window).anim.borrow_mut().take();
            return;
        }
        let Some(from) = from else { return };
        if from == to || from.size.w <= 0 || from.size.h <= 0 {
            return;
        }
        // Une animation en cours repart de l'endroit où elle en est (pas de saut).
        let data = window_data(window);
        let from = data
            .anim
            .borrow()
            .as_ref()
            .and_then(|a| a.current(window.geometry().size))
            .unwrap_or(from.to_f64());
        *data.anim.borrow_mut() = Some(Anim { from, to, start: Instant::now() });
    }

    /// Rectangle animé d'une fenêtre (None = rendu normal).
    pub fn window_anim_rect(&self, window: &Window) -> Option<Rectangle<f64, Logical>> {
        let data = window_data(window);
        let current = data.anim.borrow().as_ref().map(|a| a.current(window.geometry().size));
        match current {
            Some(Some(r)) => Some(r),
            Some(None) => {
                data.anim.borrow_mut().take();
                None
            }
            None => None,
        }
    }

    /// Ancre `window` dans `zone` (avec animation).
    pub fn snap_zone(&mut self, window: &Window, zone: Zone) {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State;
        let Some(output) = self.output_for_window(window) else { return };
        let data = window_data(window);
        if data.zone.get().is_none() && !data.maximized.get() {
            if let Some(r) = self.window_rect(window) {
                *data.restore.borrow_mut() = Some(r);
            }
        }
        let area = self.usable_area(&output);
        let target = zone.rect(area);
        let from = self.window_rect(window);
        let toplevel = window.toplevel().unwrap();
        toplevel.with_pending_state(|state| {
            for s in [State::Maximized, State::TiledLeft, State::TiledRight, State::TiledTop, State::TiledBottom] {
                state.states.unset(s);
            }
            if zone == Zone::Maximize {
                state.states.set(State::Maximized);
            } else {
                // Bords collés à l'écran : coins carrés côté écran (applis GTK).
                if target.loc.x == area.loc.x {
                    state.states.set(State::TiledLeft);
                }
                if target.loc.x + target.size.w == area.loc.x + area.size.w {
                    state.states.set(State::TiledRight);
                }
                if target.loc.y == area.loc.y {
                    state.states.set(State::TiledTop);
                }
                if target.loc.y + target.size.h == area.loc.y + area.size.h {
                    state.states.set(State::TiledBottom);
                }
            }
            state.size = Some(target.size);
        });
        toplevel.send_configure();
        data.maximized.set(true);
        data.zone.set(Some(zone));
        self.animate_window(window, from, target);
        self.space.map_element(window.clone(), target.loc, true);
        self.focus_window(window);
        self.refresh_pointer_focus_soon();
    }

    // ───────────── Glisser une fenêtre ─────────────

    /// Zone visée par le pointeur pendant un déplacement de fenêtre.
    fn zone_at(&self, output: &Output, pos: Point<f64, Logical>, bar_armed: bool, edges_armed: bool) -> (Option<Zone>, bool) {
        let Some(og) = self.space.output_geometry(output) else { return (None, false) };
        let area = self.usable_area(output);
        let ctrl = self.seat.get_keyboard().is_some_and(|k| k.modifier_state().ctrl);
        let local = pos - og.loc.to_f64();
        let t = &self.config.tiling;
        if !t.enabled {
            return (None, false);
        }
        let bar_visible = t.layout_bar && bar_armed && local.y < BAR_TRIGGER;
        if !edges_armed {
            return (None, false);
        }
        if bar_visible {
            let (_, zones, _) = bar_layout(og);
            if let Some((zone, _)) = zones.iter().find(|(_, r)| r.to_f64().contains(pos)) {
                return (Some(*zone), true);
            }
        }
        if !t.edges {
            return (None, bar_visible);
        }
        let (w, h) = (og.size.w as f64, (area.loc.y + area.size.h - og.loc.y) as f64);
        let near_left = local.x <= EDGE;
        let near_right = local.x >= w - EDGE;
        let near_top = local.y <= EDGE;
        let zone = if near_top {
            if local.x <= CORNER {
                Some(Zone::TopLeft)
            } else if local.x >= w - CORNER {
                Some(Zone::TopRight)
            } else {
                Some(Zone::Maximize)
            }
        } else if near_left {
            if local.y <= CORNER {
                Some(Zone::TopLeft)
            } else if local.y >= h - CORNER {
                Some(Zone::BottomLeft)
            } else if ctrl {
                Some(Zone::LeftThird)
            } else {
                Some(Zone::Left)
            }
        } else if near_right {
            if local.y <= CORNER {
                Some(Zone::TopRight)
            } else if local.y >= h - CORNER {
                Some(Zone::BottomRight)
            } else if ctrl {
                Some(Zone::RightThird)
            } else {
                Some(Zone::Right)
            }
        } else {
            None
        };
        (zone, bar_visible)
    }

    /// Appelé à chaque mouvement pendant le déplacement d'une fenêtre.
    pub fn update_snap_drag(&mut self, window: &Window, pos: Point<f64, Logical>) {
        let Some(output) = self.space.output_under(pos).next().cloned() else { return };
        let Some(og) = self.space.output_geometry(&output) else { return };
        let below_bar = pos.y - og.loc.y as f64 >= BAR_TRIGGER;
        let drag = self.snap_drag.get_or_insert_with(|| SnapDrag {
            window: window.clone(),
            output: output.name(),
            target: None,
            bar_visible: false,
            preview: None,
            last: Instant::now(),
            buffers: Vec::new(),
            start: pos,
            bar_armed: false,
            edges_armed: false,
        });
        drag.bar_armed |= below_bar;
        let d = pos - drag.start;
        drag.edges_armed |= d.x * d.x + d.y * d.y >= ARM_DISTANCE * ARM_DISTANCE;
        let (bar_armed, edges_armed) = (drag.bar_armed, drag.edges_armed);
        let (target, bar_visible) = self.zone_at(&output, pos, bar_armed, edges_armed);
        let Some(drag) = self.snap_drag.as_mut() else { return };
        if drag.output != output.name() {
            drag.output = output.name();
            drag.preview = None;
        }
        if drag.target != target {
            // Nouvelle cible : l'aperçu part de la fenêtre elle-même.
            if drag.preview.is_none() {
                drag.preview = self.space.element_location(window).map(|l| Rectangle::new(l, window.geometry().size).to_f64());
            }
            drag.target = target;
        }
        drag.bar_visible = bar_visible;
    }

    /// Fin du déplacement : ancrage dans la zone visée, s'il y en a une.
    pub fn finish_snap_drag(&mut self) {
        if let Some(drag) = self.snap_drag.take() {
            if let Some(zone) = drag.target {
                self.snap_zone(&drag.window, zone);
            }
        }
    }

    /// Éléments de la barre de dispositions (devant tout), pour `output`.
    pub fn snap_bar_elements(&mut self, output: &Output, scale: f64) -> Vec<NoosElement> {
        let Some(og) = self.space.output_geometry(output) else { return vec![] };
        let accent = accent(self);
        let Some(drag) = self.snap_drag.as_mut() else { return vec![] };
        if drag.output != output.name() || !drag.bar_visible {
            return vec![];
        }
        let (bar, zones, tiles) = bar_layout(og);
        let mut rects: Vec<(Rectangle<i32, Logical>, [f32; 4])> = Vec::new();
        for (zone, r) in &zones {
            let hovered = drag.target == Some(*zone) && r.to_f64().contains(self.pointer.current_location());
            rects.push((*r, if hovered { [accent[0], accent[1], accent[2], 0.95] } else { [1.0, 1.0, 1.0, 0.30] }));
        }
        for t in &tiles {
            rects.push((*t, [1.0, 1.0, 1.0, 0.06]));
        }
        rects.push((bar, [0.11, 0.11, 0.14, 0.92]));
        solid_elements(&mut drag.buffers, &rects, og, scale)
    }

    /// Rectangle d'aperçu (sous la fenêtre déplacée), animé vers la zone visée.
    pub fn snap_preview_elements(&mut self, output: &Output, scale: f64) -> Vec<NoosElement> {
        let Some(og) = self.space.output_geometry(output) else { return vec![] };
        let area = self.usable_area(output);
        let accent = accent(self);
        let show = self.config.tiling.preview;
        let Some(drag) = self.snap_drag.as_mut() else { return vec![] };
        if drag.output != output.name() || !show {
            return vec![];
        }
        let Some(target) = drag.target.map(|z| z.rect(area).to_f64()) else {
            drag.preview = None;
            return vec![];
        };
        // Rapprochement progressif (indépendant de la fréquence d'affichage).
        let dt = drag.last.elapsed().as_secs_f64().min(0.1);
        drag.last = Instant::now();
        let k = 1.0 - (-dt * 18.0).exp();
        let current = drag.preview.map(|p| lerp_rect(p, target, k)).unwrap_or(target);
        drag.preview = Some(current);
        let r = Rectangle::<i32, Logical>::new(current.loc.to_i32_round(), current.size.to_i32_round());
        let b = 2;
        let [ar, ag, ab] = accent;
        let rects = vec![
            (Rectangle::new(r.loc, (r.size.w, b).into()), [ar, ag, ab, 0.85]),
            (Rectangle::new((r.loc.x, r.loc.y + r.size.h - b).into(), (r.size.w, b).into()), [ar, ag, ab, 0.85]),
            (Rectangle::new(r.loc, (b, r.size.h).into()), [ar, ag, ab, 0.85]),
            (Rectangle::new((r.loc.x + r.size.w - b, r.loc.y).into(), (b, r.size.h).into()), [ar, ag, ab, 0.85]),
            (r, [ar, ag, ab, 0.20]),
        ];
        // Tampons 0..63 : barre de dispositions ; 64 et suivants : aperçu.
        solid_elements_from(&mut drag.buffers, 64, &rects, og, scale)
    }

    /// Une fenêtre déplacée à la souris (pour placer l'aperçu juste sous elle).
    pub fn dragged_window(&self) -> Option<&Window> {
        self.snap_drag.as_ref().map(|d| &d.window)
    }
}

/// Rectangles de couleur (logiques, globaux) → éléments de rendu, en réutilisant des tampons
/// (identifiants stables : pas de dégâts inutiles d'une image à l'autre).
fn solid_elements(
    pool: &mut Vec<SolidColorBuffer>,
    rects: &[(Rectangle<i32, Logical>, [f32; 4])],
    og: Rectangle<i32, Logical>,
    scale: f64,
) -> Vec<NoosElement> {
    solid_elements_from(pool, 0, rects, og, scale)
}

fn solid_elements_from(
    pool: &mut Vec<SolidColorBuffer>,
    start: usize,
    rects: &[(Rectangle<i32, Logical>, [f32; 4])],
    og: Rectangle<i32, Logical>,
    scale: f64,
) -> Vec<NoosElement> {
    let mut out = Vec::new();
    for (i, (r, c)) in rects.iter().enumerate() {
        let idx = start + i;
        while pool.len() <= idx {
            pool.push(SolidColorBuffer::default());
        }
        // Couleur prémultipliée par l'alpha.
        let color = [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
        pool[idx].update(r.size, color);
        let loc: Point<i32, Physical> = (r.loc - og.loc).to_physical_precise_round(scale);
        out.push(NoosElement::Solid(SolidColorRenderElement::from_buffer(&pool[idx], loc, Scale::from(scale), 1.0, Kind::Unspecified)));
    }
    out
}

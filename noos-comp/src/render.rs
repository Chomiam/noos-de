//! Construction de la liste d'éléments à afficher pour un écran.
//!
//! L'ordre est « du premier plan vers l'arrière-plan » : curseur, icône de glisser-déposer,
//! puis soit la fenêtre plein écran seule (ce qui permet le scanout direct et le VRR),
//! soit le bureau complet (layer-shell + fenêtres).

use std::sync::Mutex;

use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Kind,
            memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
        },
        gles::GlesRenderer,
    },
    desktop::{LayerSurface, Window, layer_map_for_output, space::SpaceRenderElements},
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::Output,
    utils::{IsAlive, Logical, Rectangle, Scale},
    wayland::{compositor::with_states, shell::wlr_layer::Layer},
};

use crate::state::NoosState;

smithay::backend::renderer::element::render_elements! {
    pub NoosElement<=GlesRenderer>;
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Solid=smithay::backend::renderer::element::solid::SolidColorRenderElement,
    Cropped=smithay::backend::renderer::element::utils::CropRenderElement<smithay::backend::renderer::element::utils::RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>>,
}

impl std::fmt::Debug for NoosElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Space(_) => f.write_str("Space"),
            Self::Surface(_) => f.write_str("Surface"),
            Self::Memory(_) => f.write_str("Memory"),
            Self::Solid(_) => f.write_str("Solid"),
            Self::Cropped(_) => f.write_str("Cropped"),
            Self::_GenericCatcher(_) => f.write_str("_"),
        }
    }
}

/// Couleur de fond visible tant que le shell n'a pas posé de fond d'écran.
pub fn clear_color(state: &NoosState) -> [f32; 4] {
    if state.screens_off {
        return [0.0, 0.0, 0.0, 1.0];
    }
    match state.config.appearance.color_scheme {
        noos_core::config::ColorScheme::Light => [0.90, 0.92, 0.96, 1.0],
        _ => [0.07, 0.07, 0.11, 1.0],
    }
}

/// `cursor_buffer` : image du curseur par défaut (None = le compositeur hôte dessine le curseur).
pub fn output_elements(
    state: &mut NoosState,
    renderer: &mut GlesRenderer,
    output: &Output,
    cursor_buffer: Option<(&MemoryRenderBuffer, (i32, i32))>,
) -> Vec<NoosElement> {
    let mut elements = Vec::new();
    if state.screens_off {
        return elements;
    }
    let Some(output_geo) = state.space.output_geometry(output) else { return elements };
    let scale = Scale::from(output.current_scale().fractional_scale());

    // ── Curseur ──
    let pointer_pos = state.pointer.current_location();
    if output_geo.to_f64().contains(pointer_pos) {
        let rel = pointer_pos - output_geo.loc.to_f64();
        if let CursorImageStatus::Surface(surface) = &state.cursor_status {
            if !surface.alive() {
                state.cursor_status = CursorImageStatus::default_named();
            }
        }
        match &state.cursor_status {
            CursorImageStatus::Hidden => {}
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .map(|a| a.lock().unwrap().hotspot)
                        .unwrap_or_default()
                });
                let pos = (rel - hotspot.to_f64()).to_physical(scale).to_i32_round();
                elements.extend(
                    render_elements_from_surface_tree(renderer, surface, pos, scale, 1.0, Kind::Cursor)
                        .into_iter()
                        .map(NoosElement::Surface),
                );
            }
            CursorImageStatus::Named(_) => {
                if let Some((buffer, (hx, hy))) = cursor_buffer {
                    let mut pos = rel.to_physical(scale);
                    pos.x -= hx as f64;
                    pos.y -= hy as f64;
                    if let Ok(elem) = MemoryRenderBufferRenderElement::from_buffer(
                        renderer,
                        pos,
                        buffer,
                        None,
                        None,
                        None,
                        Kind::Cursor,
                    ) {
                        elements.push(NoosElement::Memory(elem));
                    }
                }
            }
        }

        // ── Icône de glisser-déposer ──
        if let Some(icon) = state.dnd_icon.as_ref().filter(|s| s.alive()) {
            let pos = rel.to_physical(scale).to_i32_round();
            elements.extend(
                render_elements_from_surface_tree(renderer, icon, pos, scale, 1.0, Kind::Unspecified)
                    .into_iter()
                    .map(NoosElement::Surface),
            );
        }
    }

    // ── Plein écran : uniquement la fenêtre (scanout direct possible) ──
    if let Some(window) = state.fullscreen.get(&output.name()).cloned() {
        if window.alive() {
            let loc = state.space.element_location(&window).unwrap_or_default();
            let render_loc = (loc - window.geometry().loc - output_geo.loc).to_physical_precise_round(scale);
            let window_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                window.render_elements(renderer, render_loc, scale, 1.0);
            elements.extend(window_elements.into_iter().map(NoosElement::Surface));
            return elements;
        }
        state.fullscreen.remove(&output.name());
    }

    // ── Bureau, de l'avant vers l'arrière : couches Overlay puis Top (chacune avec son
    // flou « Mica » juste dessous), fenêtres, couches Bottom puis Background. ──
    let mut layers: Vec<(LayerSurface, Rectangle<i32, Logical>)> = {
        let map = layer_map_for_output(output);
        map.layers().rev().filter_map(|l| map.layer_geometry(l).map(|g| (l.clone(), g))).collect()
    };
    let rank = |l: &LayerSurface| match l.layer() {
        Layer::Overlay => 0,
        Layer::Top => 1,
        Layer::Bottom => 3,
        Layer::Background => 4,
    };
    layers.sort_by_key(|(l, _)| rank(l));
    let layer_elements = |renderer: &mut GlesRenderer, l: &LayerSurface, g: Rectangle<i32, Logical>| {
        AsRenderElements::<GlesRenderer>::render_elements::<WaylandSurfaceRenderElement<GlesRenderer>>(
            l,
            renderer,
            g.loc.to_physical_precise_round(scale),
            scale,
            1.0,
        )
        .into_iter()
        .map(NoosElement::Surface)
        .collect::<Vec<_>>()
    };
    // Barre de dispositions (fenêtre en cours de déplacement vers le haut de l'écran).
    let bar = state.snap_bar_elements(output, scale.x);
    elements.extend(bar);
    for (layer, geo) in layers.iter().filter(|(l, _)| rank(l) < 2) {
        elements.extend(layer_elements(renderer, layer, *geo));
        if let Some(blur) = state.blur_element(renderer, output, layer, *geo) {
            elements.push(blur);
        }
    }
    let windows: Vec<Window> = state
        .space
        .elements()
        .rev()
        .filter(|w| state.space.element_bbox(w).is_some_and(|b| b.overlaps(output_geo)))
        .cloned()
        .collect();
    let dragged = state.dragged_window().cloned();
    for window in windows {
        let Some(loc) = state.space.element_location(&window) else { continue };
        let wgeo = window.geometry();
        let animating = window_data_anim(&window);
        let geo = match state.window_anim_rect(&window) {
            // Animation (ancrage, maximisation…) : l'image actuelle de la fenêtre glisse et
            // s'étire vers sa nouvelle géométrie.
            Some(r) => {
                let origin = r.loc - output_geo.loc.to_f64();
                let render_loc = (origin - wgeo.loc.to_f64()).to_physical_precise_round(scale);
                // Jamais de déformation : l'échelle est la même dans les deux sens, et seulement si
                // la fenêtre grandit ou rétrécit dans les deux sens (restauration…). Sinon l'image
                // garde sa taille réelle et la fenêtre est simplement dévoilée / recadrée au
                // rectangle animé, le temps que l'appli se redessine au bon format.
                let sx = r.size.w / wgeo.size.w.max(1) as f64;
                let sy = r.size.h / wgeo.size.h.max(1) as f64;
                let s = if sx < 1.0 && sy < 1.0 {
                    sx.max(sy)
                } else if sx > 1.0 && sy > 1.0 {
                    sx.min(sy)
                } else {
                    1.0
                };
                let origin_phys = origin.to_physical_precise_round(scale);
                let crop: Rectangle<i32, smithay::utils::Physical> =
                    Rectangle::new(origin_phys, r.size.to_physical_precise_round(scale));
                let window_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    window.render_elements(renderer, render_loc, scale, 1.0);
                elements.extend(window_elements.into_iter().filter_map(|e| {
                    let scaled = smithay::backend::renderer::element::utils::RescaleRenderElement::from_element(
                        e,
                        origin_phys,
                        Scale::from(s),
                    );
                    smithay::backend::renderer::element::utils::CropRenderElement::from_element(scaled, scale, crop)
                        .map(NoosElement::Cropped)
                }));
                Rectangle::new(origin.to_i32_round(), r.size.to_i32_round())
            }
            None => {
                let render_loc = (loc - wgeo.loc - output_geo.loc).to_physical_precise_round(scale);
                let window_elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    window.render_elements(renderer, render_loc, scale, 1.0);
                elements.extend(window_elements.into_iter().map(NoosElement::Surface));
                Rectangle::new(loc - output_geo.loc, wgeo.size)
            }
        };
        // Fond translucide des applis NOOS : fond d'écran flouté juste dessous (Mica).
        let moving = animating || dragged.as_ref() == Some(&window);
        if let Some(blur) = state.window_blur_element(renderer, output, &window, geo, moving) {
            elements.push(blur);
        }
        // Aperçu de l'ancrage juste sous la fenêtre déplacée.
        if dragged.as_ref() == Some(&window) {
            let preview = state.snap_preview_elements(output, scale.x);
            elements.extend(preview);
        }
    }
    for (layer, geo) in layers.iter().filter(|(l, _)| rank(l) > 2) {
        elements.extend(layer_elements(renderer, layer, *geo));
    }
    elements
}

fn window_data_anim(window: &Window) -> bool {
    crate::state::window_data(window).anim.borrow().is_some()
}

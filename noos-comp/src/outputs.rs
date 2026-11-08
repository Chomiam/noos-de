//! Plusieurs écrans : disposition (positions choisies dans Paramètres › Écran, sinon de
//! gauche à droite), écran principal, et déplacement des fenêtres d'un écran à l'autre
//! (Super+Maj+← / →, ou glisser).

use smithay::{
    desktop::{Space, Window, layer_map_for_output},
    output::Output,
    utils::{Logical, Point, Rectangle, Size},
};
use tracing::info;

use crate::state::{NoosState, window_data};

/// Taille logique d'un écran (mode, échelle et rotation appliqués).
pub fn logical_size(o: &Output) -> Size<i32, Logical> {
    let Some(mode) = o.current_mode() else { return (0, 0).into() };
    let scale = o.current_scale().fractional_scale();
    let size = o.current_transform().transform_size(mode.size);
    size.to_f64().to_logical(scale).to_i32_round()
}

/// Écran principal : celui marqué dans la configuration, sinon le plus à gauche.
pub fn primary_name(space: &Space<Window>, config: &noos_core::NoosConfig) -> Option<String> {
    let names: Vec<String> = space.outputs().map(|o| o.name()).collect();
    if let Some(p) = config.outputs.iter().find(|c| c.primary == Some(true) && names.contains(&c.name)) {
        return Some(p.name.clone());
    }
    space
        .outputs()
        .min_by_key(|o| {
            let g = space.output_geometry(o).unwrap_or_default();
            (g.loc.x, g.loc.y)
        })
        .map(|o| o.name())
}

impl NoosState {
    /// Place chaque écran : position de la configuration, sinon à droite des autres. Les
    /// fenêtres hors de tout écran reviennent sur l'écran principal.
    pub fn apply_output_layout(&mut self) {
        let mut outputs: Vec<Output> = self.space.outputs().cloned().collect();
        outputs.sort_by_key(|o| o.name());
        let mut placed: Vec<Rectangle<i32, Logical>> = vec![];
        let mut auto = vec![];
        for o in &outputs {
            let size = logical_size(o);
            match self.config.output(&o.name()).and_then(|c| c.position) {
                Some([x, y]) => {
                    self.space.map_output(o, (x, y));
                    placed.push(Rectangle::new((x, y).into(), size));
                }
                None => auto.push((o.clone(), size)),
            }
        }
        // Sans position enregistrée : à la suite, de gauche à droite, alignés en haut.
        for (o, size) in auto {
            let x = placed.iter().map(|r| r.loc.x + r.size.w).max().unwrap_or(0);
            self.space.map_output(&o, (x, 0));
            placed.push(Rectangle::new((x, 0).into(), size));
        }
        for o in &outputs {
            o.change_current_state(None, None, None, Some(self.space.output_geometry(o).unwrap_or_default().loc));
            layer_map_for_output(o).arrange();
        }
        // Fenêtres orphelines (écran débranché, écrans réorganisés).
        if let Some(primary) = primary_name(&self.space, &self.config).and_then(|n| self.space.outputs().find(|o| o.name() == n).cloned()) {
            let area = self.usable_area(&primary);
            let orphans: Vec<Window> = self.space.elements().filter(|w| self.space.outputs_for_element(w).is_empty()).cloned().collect();
            for w in orphans {
                let size = w.geometry().size;
                let loc = area.loc + Point::from(((area.size.w - size.w).max(0) / 2, (area.size.h - size.h).max(0) / 2));
                self.space.map_element(w, loc, false);
            }
        }
        for o in &outputs {
            self.relayout_maximized(o);
        }
        let infos = self.backend.output_infos(&self.space, &self.config);
        self.ipc.broadcast(&noos_core::ipc::Event::Outputs { outputs: infos });
    }

    /// Envoie une fenêtre sur `target` en gardant sa place relative (ou son ancrage).
    pub fn move_window_to_output(&mut self, window: &Window, target: &Output) {
        let Some(from) = self.output_for_window(window) else { return };
        if &from == target {
            return;
        }
        info!(to = target.name(), "fenêtre déplacée vers un autre écran");
        let data = window_data(window);
        if data.fullscreen.get() {
            self.unset_fullscreen(window);
            self.set_fullscreen(window, target);
            return;
        }
        let (fa, ta) = (self.usable_area(&from), self.usable_area(target));
        let zone = data.zone.get().or(data.maximized.get().then_some(crate::tiling::Zone::Maximize));
        let loc = self.space.element_location(window).unwrap_or(fa.loc);
        let size = window.geometry().size;
        // Position relative conservée, sans dépasser du nouvel écran.
        let rx = (loc.x - fa.loc.x) as f64 / fa.size.w.max(1) as f64;
        let ry = (loc.y - fa.loc.y) as f64 / fa.size.h.max(1) as f64;
        let mut nx = ta.loc.x + (rx * ta.size.w as f64) as i32;
        let mut ny = ta.loc.y + (ry * ta.size.h as f64) as i32;
        nx = nx.clamp(ta.loc.x, (ta.loc.x + ta.size.w - size.w).max(ta.loc.x));
        ny = ny.clamp(ta.loc.y, (ta.loc.y + ta.size.h - size.h).max(ta.loc.y));
        let from_rect = self.window_rect(window);
        // La géométrie à restaurer suit aussi la fenêtre.
        if let Some(r) = data.restore.borrow_mut().as_mut() {
            r.loc = Point::from((ta.loc.x + (r.loc.x - fa.loc.x).clamp(0, ta.size.w / 2), ta.loc.y + (r.loc.y - fa.loc.y).clamp(0, ta.size.h / 2)));
        }
        self.space.map_element(window.clone(), (nx, ny), true);
        match zone {
            Some(z) => self.snap_zone(window, z),
            None => self.animate_window(window, from_rect, Rectangle::new((nx, ny).into(), size)),
        }
        self.focus_window(window);
        self.notify_windows_changed();
    }

    /// Écran voisin (à gauche : -1, à droite : +1), d'après la position des écrans.
    pub fn move_window_to_adjacent_output(&mut self, window: &Window, dir: i32) {
        let Some(from) = self.output_for_window(window) else { return };
        let mut outs: Vec<(Output, i32)> = self.space.outputs().map(|o| (o.clone(), self.space.output_geometry(o).map(|g| g.loc.x + g.size.w / 2).unwrap_or(0))).collect();
        outs.sort_by_key(|(_, c)| *c);
        let Some(i) = outs.iter().position(|(o, _)| o == &from) else { return };
        let j = i as i32 + dir;
        if j < 0 || j >= outs.len() as i32 {
            return;
        }
        let target = outs[j as usize].0.clone();
        self.move_window_to_output(window, &target);
    }
}

//! Bureaux virtuels (comme Windows) : chaque fenêtre appartient à un bureau ; seules celles
//! du bureau actif (et les fenêtres « sur tous les bureaux ») sont dans l'espace affiché. Les
//! autres sont « garées » avec leur position. Changement de bureau animé : l'ancien bureau
//! glisse hors de l'écran pendant que le nouveau entre.
//!
//! Raccourcis : Super+Ctrl+←/→ (changer), Super+Ctrl+D (nouveau), Super+Ctrl+F4 (fermer),
//! Super+Ctrl+Maj+←/→ (emporter la fenêtre active), Super+Tab (vue des tâches).
//! Les noms sont enregistrés dans `[desktops] names` (le compositeur est le seul à écrire
//! cette liste : Paramètres et la barre passent par l'IPC).

use std::time::{Duration, Instant};

use noos_core::ipc::Event;
use smithay::{
    desktop::Window,
    utils::{Logical, Point},
};
use tracing::info;

use crate::state::{NoosState, window_data};

pub const SLIDE: Duration = Duration::from_millis(320);

#[derive(Default)]
pub struct Desktops {
    pub current: usize,
    /// Fenêtres des autres bureaux (non réduites), avec leur position.
    pub parked: Vec<(Window, Point<i32, Logical>)>,
    /// Transition en cours : fenêtres sortantes (et leur position), sens (+1 vers la droite),
    /// début.
    pub slide: Option<Slide>,
}

pub struct Slide {
    pub leaving: Vec<(Window, Point<i32, Logical>)>,
    pub dir: i32,
    pub start: Instant,
}

fn ease(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

impl NoosState {
    pub fn desktop_names(&self) -> Vec<String> {
        let mut n = self.config.desktops.names.clone();
        if n.is_empty() {
            n.push("Bureau 1".into());
        }
        n
    }

    pub fn desktop_count(&self) -> usize {
        self.desktop_names().len()
    }

    pub fn broadcast_desktops(&mut self) {
        let names = self.desktop_names();
        let current = self.desktops.current.min(names.len() - 1);
        self.ipc.broadcast(&Event::Desktops { names, current });
    }

    fn save_desktop_names(&mut self, names: Vec<String>) {
        self.config.desktops.names = names;
        // Seul le compositeur écrit cette liste : relire le fichier pour ne rien écraser
        // d'autre (réglages modifiés entre-temps par Paramètres).
        let mut cfg = noos_core::NoosConfig::load();
        cfg.desktops.names = self.config.desktops.names.clone();
        if let Err(e) = cfg.save() {
            tracing::warn!("bureaux : sauvegarde impossible : {e:#}");
        }
    }

    /// Décalage horizontal (logique) des fenêtres entrantes / sortantes pendant la transition.
    /// `None` = pas de transition.
    pub fn desktop_slide_offsets(&mut self, width: i32) -> Option<(i32, i32)> {
        let s = self.desktops.slide.as_ref()?;
        let t = s.start.elapsed().as_secs_f64() / SLIDE.as_secs_f64();
        if t >= 1.0 {
            self.desktops.slide = None;
            return None;
        }
        let k = ease(t);
        let w = width as f64 * s.dir as f64;
        // Entrant : de +w à 0 ; sortant : de 0 à -w.
        Some(((w * (1.0 - k)).round() as i32, (-w * k).round() as i32))
    }

    /// Aller au bureau `index`.
    pub fn switch_desktop(&mut self, index: usize) {
        let count = self.desktop_count();
        if index >= count || index == self.desktops.current {
            self.broadcast_desktops();
            return;
        }
        let dir = if index > self.desktops.current { 1 } else { -1 };
        info!(from = self.desktops.current, to = index, "changement de bureau");
        // Garer les fenêtres du bureau quitté (sauf celles présentes sur tous les bureaux).
        let mut leaving = vec![];
        for w in self.space.elements().cloned().collect::<Vec<_>>() {
            let d = window_data(&w);
            if d.sticky.get() {
                continue;
            }
            let loc = self.space.element_location(&w).unwrap_or_default();
            self.space.unmap_elem(&w);
            self.fullscreen.retain(|_, f| f != &w);
            leaving.push((w.clone(), loc));
            self.desktops.parked.push((w, loc));
        }
        // Sortir celles du bureau d'arrivée.
        let mut i = 0;
        while i < self.desktops.parked.len() {
            if window_data(&self.desktops.parked[i].0).desktop.get() == index {
                let (w, loc) = self.desktops.parked.remove(i);
                self.space.map_element(w.clone(), loc, false);
                if window_data(&w).fullscreen.get() {
                    if let Some(o) = self.space.outputs_for_element(&w).into_iter().next() {
                        self.fullscreen.insert(o.name(), w.clone());
                    }
                }
            } else {
                i += 1;
            }
        }
        // Les fenêtres entrantes restent sous les éventuelles fenêtres « tous bureaux »
        // déjà affichées : on les remonte dans leur ordre d'origine.
        self.desktops.current = index;
        self.desktops.slide = self.config.desktops.animation.then(|| Slide { leaving, dir, start: Instant::now() });
        self.focus_top_window();
        self.refresh_pointer_focus_soon();
        self.update_vrr();
        self.notify_windows_changed();
        self.broadcast_desktops();
    }

    pub fn switch_desktop_relative(&mut self, delta: i32) {
        let n = self.desktop_count() as i32;
        let mut target = self.desktops.current as i32 + delta;
        if self.config.desktops.wrap {
            target = target.rem_euclid(n);
        }
        if target < 0 || target >= n {
            return;
        }
        self.switch_desktop(target as usize);
    }

    pub fn add_desktop(&mut self, name: Option<String>, switch: bool) {
        let mut names = self.desktop_names();
        let n = names.len() + 1;
        let name = name.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
            let mut k = n;
            while names.iter().any(|x| x == &format!("Bureau {k}")) {
                k += 1;
            }
            format!("Bureau {k}")
        });
        names.push(name);
        let idx = names.len() - 1;
        self.save_desktop_names(names);
        if switch {
            self.switch_desktop(idx);
        } else {
            self.broadcast_desktops();
        }
    }

    /// Supprime un bureau : ses fenêtres vont sur le bureau précédent (ou le suivant).
    pub fn remove_desktop(&mut self, index: usize) {
        let mut names = self.desktop_names();
        if names.len() <= 1 || index >= names.len() {
            return;
        }
        let target = if index > 0 { index - 1 } else { 0 };
        // Si l'on supprime le bureau actif, on va d'abord sur le bureau d'accueil.
        if self.desktops.current == index {
            self.switch_desktop(if index > 0 { index - 1 } else { 1 });
        }
        // Renuméroter toutes les fenêtres.
        let all: Vec<Window> =
            self.space.elements().cloned().chain(self.minimized.iter().cloned()).chain(self.desktops.parked.iter().map(|p| p.0.clone())).collect();
        for w in &all {
            let d = window_data(w);
            let cur = d.desktop.get();
            if cur == index {
                d.desktop.set(target);
            } else if cur > index {
                d.desktop.set(cur - 1);
            }
        }
        if self.desktops.current > index {
            self.desktops.current -= 1;
        }
        names.remove(index);
        self.save_desktop_names(names);
        // Les fenêtres du bureau supprimé arrivées sur le bureau actif : les afficher.
        let cur = self.desktops.current;
        let mut i = 0;
        while i < self.desktops.parked.len() {
            if window_data(&self.desktops.parked[i].0).desktop.get() == cur {
                let (w, loc) = self.desktops.parked.remove(i);
                self.space.map_element(w, loc, false);
            } else {
                i += 1;
            }
        }
        self.notify_windows_changed();
        self.broadcast_desktops();
    }

    pub fn rename_desktop(&mut self, index: usize, name: String) {
        let mut names = self.desktop_names();
        if let Some(n) = names.get_mut(index) {
            let name = name.trim();
            *n = if name.is_empty() { format!("Bureau {}", index + 1) } else { name.to_string() };
            self.save_desktop_names(names);
            self.broadcast_desktops();
        }
    }

    /// Réordonne les bureaux (glisser dans la vue des tâches ou Paramètres).
    pub fn move_desktop(&mut self, from: usize, to: usize) {
        let mut names = self.desktop_names();
        if from >= names.len() || to >= names.len() || from == to {
            return;
        }
        let n = names.remove(from);
        names.insert(to, n);
        let remap = |i: usize| -> usize {
            if i == from {
                to
            } else if from < to && i > from && i <= to {
                i - 1
            } else if from > to && i >= to && i < from {
                i + 1
            } else {
                i
            }
        };
        let all: Vec<Window> =
            self.space.elements().cloned().chain(self.minimized.iter().cloned()).chain(self.desktops.parked.iter().map(|p| p.0.clone())).collect();
        for w in &all {
            let d = window_data(w);
            d.desktop.set(remap(d.desktop.get()));
        }
        self.desktops.current = remap(self.desktops.current);
        self.save_desktop_names(names);
        self.notify_windows_changed();
        self.broadcast_desktops();
    }

    /// Envoie une fenêtre sur un autre bureau.
    pub fn move_window_to_desktop(&mut self, window: &Window, index: usize, follow: bool) {
        if index >= self.desktop_count() {
            return;
        }
        let d = window_data(window);
        d.sticky.set(false);
        d.desktop.set(index);
        if index != self.desktops.current && self.space.elements().any(|w| w == window) {
            let loc = self.space.element_location(window).unwrap_or_default();
            self.space.unmap_elem(window);
            self.fullscreen.retain(|_, f| f != window);
            self.desktops.parked.push((window.clone(), loc));
            if !follow {
                self.focus_top_window();
            }
        }
        if follow {
            self.switch_desktop(index);
            self.focus_window(window);
        }
        self.notify_windows_changed();
    }

    pub fn set_sticky(&mut self, window: &Window, sticky: bool) {
        let d = window_data(window);
        d.sticky.set(sticky);
        if !sticky {
            d.desktop.set(self.desktops.current);
        }
        self.notify_windows_changed();
    }

    /// Fenêtre garée sur un autre bureau : on va sur son bureau (clic dans la barre, Alt+Tab).
    pub fn bring_desktop_of(&mut self, window: &Window) {
        let parked = self.desktops.parked.iter().any(|(w, _)| w == window);
        let d = window_data(window).desktop.get();
        let minimized_elsewhere = self.minimized.contains(window) && d != self.desktops.current && !window_data(window).sticky.get();
        if parked || minimized_elsewhere {
            self.switch_desktop(d);
        }
    }

    /// Le nombre de bureaux a diminué (configuration modifiée à la main) : les fenêtres des
    /// bureaux disparus rejoignent le dernier.
    pub fn clamp_desktops(&mut self) {
        let last = self.desktop_count() - 1;
        let all: Vec<Window> =
            self.space.elements().cloned().chain(self.minimized.iter().cloned()).chain(self.desktops.parked.iter().map(|p| p.0.clone())).collect();
        for w in &all {
            let d = window_data(w);
            if d.desktop.get() > last {
                d.desktop.set(last);
            }
        }
        if self.desktops.current > last {
            self.switch_desktop(last);
        }
        self.broadcast_desktops();
    }
}

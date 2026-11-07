//! Verrouillage de session (`ext-session-lock-v1`).
//!
//! `noos-lock` demande le verrou ; tant qu'il est actif, chaque écran n'affiche que la surface
//! de verrouillage qui lui est attribuée (ou du noir), et seule elle reçoit clavier et pointeur
//! (aucun raccourci, sauf le changement de terminal virtuel). Si `noos-lock` s'arrête sans
//! déverrouiller (plantage), la session reste verrouillée et il est relancé.
//!
//! Déclencheurs : Super+L, requête IPC `Lock`, inactivité (`[lock] idle_minutes`), et avant
//! une mise en veille (`[lock] before_suspend`).

use std::time::{Duration, Instant};

use smithay::{
    output::Output,
    reexports::wayland_server::protocol::{wl_output::WlOutput, wl_surface::WlSurface},
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
};
use tracing::{info, warn};

use crate::state::NoosState;

pub const LOCK_COMMAND: &str = "noos-lock";

#[derive(Default)]
pub struct LockState {
    /// Session verrouillée (le verrou a été accordé et pas encore levé).
    pub locked: bool,
    /// Surfaces de verrouillage par écran (nom du connecteur).
    pub surfaces: Vec<(String, LockSurface)>,
    /// Dernier lancement de noos-lock (relances espacées).
    pub spawned_at: Option<Instant>,
}

impl SessionLockHandler for NoosState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        info!("session verrouillée");
        self.lock.locked = true;
        self.lock.surfaces.clear();
        // Délai de grâce : le verrouilleur crée ses surfaces juste après.
        self.lock.spawned_at = Some(Instant::now());
        // Plus de fenêtre active ; le prochain rendu n'affiche que du noir ou les surfaces de
        // verrouillage (le rendu est immédiat : on confirme sans attendre l'image suivante).
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(k) = self.seat.get_keyboard() {
            k.set_focus(self, None, serial);
        }
        confirmation.lock();
        self.render_all();
    }

    fn unlock(&mut self) {
        info!("session déverrouillée");
        self.lock.locked = false;
        self.lock.surfaces.clear();
        self.last_activity = Instant::now();
        self.refocus_after_unlock();
        self.render_all();
    }

    fn new_surface(&mut self, surface: LockSurface, wl_output: WlOutput) {
        let Some(output) = Output::from_resource(&wl_output) else { return };
        let size = self
            .space
            .output_geometry(&output)
            .map(|g| g.size)
            .unwrap_or_else(|| (1920, 1080).into());
        surface.with_pending_state(|s| s.size = Some((size.w as u32, size.h as u32).into()));
        surface.send_configure();
        // Le clavier va à la surface de l'écran principal (le premier reçu).
        if self.lock.surfaces.is_empty() {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(k) = self.seat.get_keyboard() {
                k.set_focus(self, Some(surface.wl_surface().clone()), serial);
            }
        }
        self.lock.surfaces.retain(|(name, s)| *name != output.name() && s.alive());
        self.lock.surfaces.push((output.name(), surface));
    }
}

impl NoosState {
    pub fn is_locked(&self) -> bool {
        self.lock.locked
    }

    /// Lance l'écran de verrouillage (s'il ne tourne pas déjà).
    pub fn lock_session(&mut self) {
        if self.lock_client_running() {
            return;
        }
        if crate::kiosk() {
            return;
        }
        info!("verrouillage demandé");
        self.lock.spawned_at = Some(Instant::now());
        self.spawn(LOCK_COMMAND);
    }

    fn lock_client_running(&self) -> bool {
        self.children
            .borrow_mut()
            .iter_mut()
            .any(|(cmd, child)| cmd == LOCK_COMMAND && matches!(child.try_wait(), Ok(None)))
    }

    /// Appelé régulièrement : verrou actif mais plus aucune surface de verrouillage vivante (le
    /// verrouilleur, quel qu'il soit, s'est arrêté sans déverrouiller) → noos-lock est relancé
    /// (au plus toutes les 3 s), pour ne jamais laisser l'utilisateur devant un écran noir.
    pub fn check_lock_client(&mut self) {
        if !self.lock.locked || self.lock.surfaces.iter().any(|(_, s)| s.alive()) || self.lock_client_running() {
            return;
        }
        if self.lock.spawned_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        warn!("session verrouillée sans noos-lock : relance");
        self.lock.surfaces.clear();
        self.lock.spawned_at = Some(Instant::now());
        self.spawn(LOCK_COMMAND);
    }

    /// Surface de verrouillage de l'écran `output`.
    pub fn lock_surface_for(&self, output: &Output) -> Option<&LockSurface> {
        self.lock.surfaces.iter().find(|(n, s)| *n == output.name() && s.alive()).map(|(_, s)| s)
    }

    /// Surface sous le pointeur pendant le verrouillage (coordonnées globales de son origine).
    pub fn lock_surface_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.space.output_under(pos).next()?;
        let geo = self.space.output_geometry(output)?;
        let s = self.lock_surface_for(output)?;
        Some((s.wl_surface().clone(), geo.loc.to_f64()))
    }

    /// Clic pendant le verrouillage : le clavier suit l'écran cliqué.
    pub fn focus_lock_under(&mut self, pos: Point<f64, Logical>) {
        if let Some((surface, _)) = self.lock_surface_under(pos) {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(k) = self.seat.get_keyboard() {
                k.set_focus(self, Some(surface), serial);
            }
        }
    }

    fn refocus_after_unlock(&mut self) {
        let window = self.space.elements().last().cloned();
        if let Some(w) = window {
            self.focus_window(&w);
        }
    }

    /// Mise en veille : verrouiller d'abord (si demandé), puis suspendre un peu plus tard pour
    /// que l'écran de verrouillage soit déjà affiché au réveil.
    pub fn suspend_with_lock(&mut self) {
        let lock_first = self.config.lock.before_suspend && !self.lock.locked;
        if lock_first {
            self.lock_session();
        }
        let delay = if lock_first { 1500 } else { 0 };
        let _ = self.loop_handle.insert_source(
            smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_millis(delay)),
            |_, _, _| {
                let _ = std::process::Command::new("systemctl").arg("suspend").spawn();
                smithay::reexports::calloop::timer::TimeoutAction::Drop
            },
        );
    }
}

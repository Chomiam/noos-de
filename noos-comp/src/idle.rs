//! Inactivité : extinction des écrans et mise en veille selon `[power]` de noos.toml.
//!
//! L'extinction est empêchée par :
//! - les clients qui demandent `zwp_idle_inhibit` (lecteurs vidéo, navigateurs…) ;
//! - une fenêtre en plein écran (jeux), si `inhibit_fullscreen` est activé.

use std::time::{Duration, Instant};

use smithay::{
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::IsAlive,
};
use tracing::info;

use crate::state::NoosState;

impl NoosState {
    /// À appeler à chaque événement d'entrée.
    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
        self.suspend_requested = false;
        if self.screens_off {
            info!("activité : rallumage des écrans");
            self.screens_off = false;
            self.backend.set_screens_power(true);
            if matches!(self.backend, crate::backend::Backend::Udev(_)) {
                self.loop_handle.insert_idle(|s| s.render_all());
            }
        }
    }

    fn idle_inhibited(&mut self) -> bool {
        self.idle_inhibitors.retain(|s| s.alive());
        !self.idle_inhibitors.is_empty() || (self.config.power.inhibit_fullscreen && !self.fullscreen.is_empty())
    }

    fn check_idle(&mut self) {
        let power = self.config.power.clone();
        if self.idle_inhibited() {
            self.last_activity = Instant::now();
            return;
        }
        let idle = self.last_activity.elapsed();
        let lock = self.config.lock.clone();
        if lock.idle_minutes > 0 && !self.lock.locked && idle >= Duration::from_secs(lock.idle_minutes as u64 * 60) {
            info!(minutes = lock.idle_minutes, "inactivité : verrouillage");
            self.lock_session();
        }
        if power.screen_off_minutes > 0
            && !self.screens_off
            && idle >= Duration::from_secs(power.screen_off_minutes as u64 * 60)
        {
            info!(minutes = power.screen_off_minutes, "inactivité : extinction des écrans");
            self.screens_off = true;
            self.backend.set_screens_power(false);
        }
        if power.suspend_minutes > 0
            && !self.suspend_requested
            && idle >= Duration::from_secs(power.suspend_minutes as u64 * 60)
            && !matches!(self.backend, crate::backend::Backend::Winit(_))
        {
            info!(minutes = power.suspend_minutes, "inactivité : mise en veille");
            self.suspend_requested = true;
            self.suspend_with_lock();
        }
    }

    /// Récupère les processus terminés (relance du shell, fin du kiosque).
    pub fn start_reaper(&mut self) {
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
            state.reap_children();
            state.check_lock_client();
            TimeoutAction::ToDuration(Duration::from_secs(1))
        });
    }

    pub fn start_idle_timer(&mut self) {
        self.start_reaper();
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(5)), |_, _, state| {
            state.check_idle();
            TimeoutAction::ToDuration(Duration::from_secs(5))
        });
    }
}

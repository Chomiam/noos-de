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
        if self.dim_started.take().is_some() {
            // Assombrissement annulé : l'écran revient tout de suite.
            self.loop_handle.insert_idle(|s| s.render_all());
        }
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
        if lock.idle_minutes > 0 && !self.lock.locked {
            let at = Duration::from_secs(lock.idle_minutes as u64 * 60);
            let dim = Duration::from_secs(lock.dim_seconds as u64).min(at);
            if idle >= at {
                self.dim_started = None;
                match lock.mode {
                    noos_core::config::IdleMode::Lock => {
                        info!(minutes = lock.idle_minutes, "inactivité : verrouillage");
                        self.lock_session();
                    }
                    noos_core::config::IdleMode::Screensaver => self.start_screensaver(),
                }
            } else if !dim.is_zero() && idle >= at - dim && self.dim_started.is_none() && !self.screens_off {
                info!(secondes = lock.dim_seconds, "inactivité : assombrissement");
                self.dim_started = Some(Instant::now() - (idle - (at - dim)));
                self.start_dim_animation();
            }
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
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, state| {
            state.check_idle();
            TimeoutAction::ToDuration(Duration::from_secs(1))
        });
    }

    /// Opacité du voile d'assombrissement (None : pas d'assombrissement en cours).
    pub fn dim_alpha(&self) -> Option<f32> {
        let start = self.dim_started?;
        let total = self.config.lock.dim_seconds.max(1) as f32;
        let k = (start.elapsed().as_secs_f32() / total).clamp(0.0, 1.0);
        // Courbe douce : l'essentiel de l'assombrissement arrive vite, puis se stabilise.
        Some(0.82 * (1.0 - (1.0 - k) * (1.0 - k)))
    }

    /// Redessine les écrans ~30 fois par seconde tant que l'assombrissement progresse.
    fn start_dim_animation(&mut self) {
        let _ = self.loop_handle.insert_source(Timer::from_duration(Duration::from_millis(33)), |_, _, state| {
            if state.dim_started.is_none() {
                return TimeoutAction::Drop;
            }
            state.render_all();
            TimeoutAction::ToDuration(Duration::from_millis(33))
        });
    }
}

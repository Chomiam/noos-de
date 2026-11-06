//! Backends d'affichage :
//! - `winit` : NOOS tourne dans une fenêtre de votre session actuelle (développement) ;
//! - `udev`  : NOOS pilote directement l'écran via DRM/KMS (vraie session depuis GDM).

pub mod udev;
pub mod winit;

use noos_core::{NoosConfig, ipc::OutputInfo};
use smithay::{
    backend::allocator::dmabuf::Dmabuf, desktop::Space, desktop::Window, output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
};

pub enum Backend {
    Winit(winit::WinitData),
    Udev(Box<udev::UdevData>),
    /// État transitoire pendant un rendu (le backend est sorti temporairement de l'état).
    Detached,
}

impl Backend {
    pub fn seat_name(&self) -> String {
        match self {
            Backend::Udev(d) => d.seat_name(),
            _ => "seat0".into(),
        }
    }

    pub fn early_import(&mut self, surface: &WlSurface) {
        if let Backend::Udev(d) = self {
            d.early_import(surface);
        }
    }

    pub fn import_dmabuf(&mut self, dmabuf: &Dmabuf) -> bool {
        match self {
            Backend::Winit(d) => d.import_dmabuf(dmabuf),
            Backend::Udev(d) => d.import_dmabuf(dmabuf),
            Backend::Detached => false,
        }
    }

    pub fn set_vrr(&mut self, output: &Output, enabled: bool) {
        if let Backend::Udev(d) = self {
            d.set_vrr(output, enabled);
        }
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Backend::Udev(d) = self {
            d.change_vt(vt);
        }
    }

    /// Allume / éteint les écrans (DPMS). En mode imbriqué, le rendu devient simplement noir.
    pub fn set_screens_power(&mut self, on: bool) {
        if let Backend::Udev(d) = self {
            d.set_power(on);
        }
    }

    /// Change le mode d'un écran ; renvoie l'`Output` modifié.
    pub fn set_mode(&mut self, output: &str, width: i32, height: i32, refresh: i32) -> Result<Output, String> {
        match self {
            Backend::Winit(d) => d.set_mode(width, height),
            Backend::Udev(d) => d.set_mode(output, width, height, refresh),
            Backend::Detached => Err("backend indisponible".into()),
        }
    }

    /// GPU et formats dmabuf utilisables comme cible de capture (None en mode imbriqué : shm).
    pub fn capture_dma_formats(
        &self,
    ) -> Option<(smithay::backend::drm::DrmNode, smithay::backend::allocator::format::FormatSet)> {
        match self {
            Backend::Udev(d) => d.capture_dma_formats(),
            _ => None,
        }
    }

    pub fn supports_transform(&self) -> bool {
        matches!(self, Backend::Udev(_))
    }

    pub fn schedule_render(&mut self) {
        // winit redessine en continu ; le backend DRM est piloté par les vblanks.
    }

    pub fn apply_config(&mut self, config: &NoosConfig) {
        if let Backend::Udev(d) = self {
            d.apply_config(config);
        }
    }

    pub fn output_infos(&self, space: &Space<Window>, config: &NoosConfig) -> Vec<OutputInfo> {
        match self {
            Backend::Winit(d) => vec![d.output_info()],
            Backend::Udev(d) => d.output_infos(space, config),
            Backend::Detached => vec![],
        }
    }
}

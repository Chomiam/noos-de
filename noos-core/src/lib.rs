//! Bibliothèque partagée par tous les composants NOOS.
//!
//! - [`config`] : le fichier `~/.config/noos/noos.toml` (thème, barre des tâches, gaming…)
//! - [`ipc`]    : le protocole JSON-lines entre le compositeur et les clients (shell, réglages)
//! - [`paths`]  : emplacements standards (XDG)
//! - [`bing`]   : images du jour de Bing comme fonds d'écran
//! - [`layout`] : mises en page du bureau (mode édition), annuler / rétablir, aimantation
//! - [`weather`] : météo du widget de bureau (Open-Meteo)

pub mod archive;
pub mod bing;
pub mod config;
pub mod disk_image;
pub mod edid;
pub mod holidays;
pub mod ipc;
pub mod layout;
pub mod login;
pub mod paths;
pub mod power;
pub mod updates;
pub mod wallhaven;
pub mod weather;

pub use config::NoosConfig;

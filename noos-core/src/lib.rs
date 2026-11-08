//! Bibliothèque partagée par tous les composants NOOS.
//!
//! - [`config`] : le fichier `~/.config/noos/noos.toml` (thème, barre des tâches, gaming…)
//! - [`ipc`]    : le protocole JSON-lines entre le compositeur et les clients (shell, réglages)
//! - [`paths`]  : emplacements standards (XDG)
//! - [`bing`]   : images du jour de Bing comme fonds d'écran

pub mod ai;
pub mod archive;
pub mod bing;
pub mod config;
pub mod disk_image;
pub mod ipc;
pub mod login;
pub mod paths;
pub mod wallhaven;

pub use config::NoosConfig;

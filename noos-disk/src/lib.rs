//! Bibliothèque « disques » de NOOS, partagée par l'application Disques (`noos-disk`), le
//! shell (icône « Retirer le périphérique en toute sécurité », lecture automatique) et
//! l'Explorateur.

pub mod ops;
pub mod rawio;
pub mod udisks;

pub use udisks::{Block, Drive, Model, fs_name, human};

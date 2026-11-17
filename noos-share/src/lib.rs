//! Partage de fichiers en réseau local de NOOS.
//!
//! - **Samba** (SMB) : vu par Windows (WSD : `wsdd2`), macOS et Linux (mDNS : Avahi). Le dossier
//!   personnel de chaque compte est partagé par défaut (section `[homes]`), **uniquement avec
//!   authentification** : compte de l'utilisateur et mot de passe de partage (`smbpasswd`), jamais
//!   d'invité. Accès limité au réseau local (`hosts allow` + règles du pare-feu NOOS).
//! - **SSH / SFTP** : `openssh-server`, désactivé par défaut ; port, connexion par mot de passe,
//!   clés autorisées (`~/.ssh/authorized_keys`).
//! - **WebDAV** : `rclone serve webdav`, service systemd *utilisateur* (`noos-webdav`) : il ne
//!   voit que ce que voit l'utilisateur. HTTPS (certificat auto-signé) et mot de passe bcrypt.
//!
//! La configuration système (`/etc/noos/sharing.toml`) n'est écrite que par
//! `noos-share-helper` (pkexec) ; ce crate fournit le modèle, les générateurs de fichiers (testés),
//! l'état des services et les adresses à utiliser depuis un autre appareil.

pub mod client;
pub mod conf;
pub mod demo;
pub mod model;
pub mod status;
pub mod webdav;

pub use model::*;

/// Mode démonstration (aperçu, captures) : données fictives, aucune commande exécutée.
pub fn demo() -> bool {
    std::env::var_os("NOOS_SHARE_DEMO").is_some_and(|v| !v.is_empty() && v != "0")
}

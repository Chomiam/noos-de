//! Données fictives (`NOOS_SHARE_DEMO=1`) : captures et aperçu, sans Samba ni sshd.
//! `NOOS_SHARE_DEMO=nopass` : aucun mot de passe de partage ; `=off` : tout désactivé ;
//! `=missing` : paquets absents.

use crate::{
    model::{Samba, Share, Ssh, SystemConfig},
    status::{AuthorizedKey, Peer, Service},
    webdav::{Folder, WebDav},
};

fn variant() -> String {
    std::env::var("NOOS_SHARE_DEMO").unwrap_or_default()
}

pub fn system_config() -> SystemConfig {
    let off = variant() == "off";
    SystemConfig {
        samba: Samba {
            enabled: !off,
            shares: vec![
                Share { name: "Photos".into(), path: "/home/axel/Images".into(), writable: false, owner: "axel".into() },
                Share { name: "Jeux".into(), path: "/mnt/Jeux".into(), writable: true, owner: "axel".into() },
            ],
            ..Samba::default()
        },
        ssh: Ssh { enabled: !off, port: 22, password_auth: false },
    }
}

pub fn service(unit: &str) -> Service {
    let v = variant();
    let installed = v != "missing";
    let on = installed && v != "off";
    Service { unit: unit.into(), installed, enabled: on, active: on, failed: false }
}

pub fn connections(port: u16) -> Vec<Peer> {
    if variant() == "off" {
        return vec![];
    }
    match port {
        445 => vec![
            Peer { address: "192.168.1.31".into(), name: Some("PORTABLE-ALEX".into()), user: None },
            Peer { address: "192.168.1.44".into(), name: Some("iPhone-de-Sam.local".into()), user: None },
        ],
        22 => vec![Peer { address: "192.168.1.31".into(), name: Some("portable-alex.local".into()), user: None }],
        _ => vec![],
    }
}

pub fn resolve(ip: &str) -> Option<String> {
    connections(445).into_iter().chain(connections(22)).find(|p| p.address == ip).and_then(|p| p.name)
}

pub fn keys() -> Vec<AuthorizedKey> {
    vec![
        AuthorizedKey { line: "ssh-ed25519 AAAA… alex@portable".into(), kind: "Ed25519".into(), comment: "alex@portable".into(), fingerprint: "SHA256:4b7XhQ1m0yF9aZ2tN8cK3vD6pR5sL1jW7eU0gH9iT2o".into() },
        AuthorizedKey { line: "sk-ssh-ed25519@openssh.com AAAA… yubikey".into(), kind: "Clé de sécurité (FIDO)".into(), comment: "yubikey".into(), fingerprint: "SHA256:Qm2vN8xR4tB6yC1kL9dF3gH7jS0aZ5wE2uP8iO4nM6c".into() },
    ]
}

pub fn webdav() -> WebDav {
    WebDav {
        enabled: variant() != "off",
        port: 8443,
        https: true,
        user: "axel".into(),
        folders: vec![
            Folder { name: "Dossier personnel".into(), path: "/home/axel".into(), writable: true },
            Folder { name: "Musique".into(), path: "/home/axel/Musique".into(), writable: false },
        ],
    }
}

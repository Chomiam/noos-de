//! État lu sans privilège : services, connexions actives, adresses de ce PC, clés SSH.

use std::{path::Path, process::Command};

use serde::{Deserialize, Serialize};

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).env("LC_ALL", "C.UTF-8").output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// État d'une unité systemd.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Service {
    pub unit: String,
    /// Programme présent (paquet installé).
    pub installed: bool,
    pub enabled: bool,
    pub active: bool,
    /// « failed » : arrêté sur une erreur.
    pub failed: bool,
}

/// Unité système (`user = false`) ou de la session (`systemctl --user`).
pub fn service(unit: &str, binary: &str, user: bool) -> Service {
    if crate::demo() {
        return crate::demo::service(unit);
    }
    let installed = Path::new(binary).exists();
    let mut base: Vec<&str> = vec![];
    if user {
        base.push("--user");
    }
    let q = |verb: &str| {
        let mut a = base.clone();
        a.extend([verb, unit]);
        run("systemctl", &a).map(|s| s.trim().to_string()).unwrap_or_default()
    };
    let active = q("is-active");
    let enabled = q("is-enabled");
    Service { unit: unit.into(), installed, enabled: matches!(enabled.as_str(), "enabled" | "enabled-runtime" | "alias"), active: active == "active", failed: active == "failed" }
}

pub fn smbd() -> Service {
    service("smbd.service", "/usr/sbin/smbd", false)
}
pub fn wsdd() -> Service {
    service("wsdd2.service", "/usr/sbin/wsdd2", false)
}
pub fn avahi() -> Service {
    service("avahi-daemon.service", "/usr/sbin/avahi-daemon", false)
}
pub fn sshd() -> Service {
    service("ssh.service", "/usr/sbin/sshd", false)
}
pub fn webdav() -> Service {
    service("noos-webdav.service", "/usr/bin/rclone", true)
}

/// Connexion établie vers un service de ce PC.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub address: String,
    /// Nom de l'appareil (mDNS / DNS), s'il est connu.
    pub name: Option<String>,
    /// Compte utilisé (Samba : `smbstatus`, SSH : `who`), s'il est connu.
    pub user: Option<String>,
}

/// Connexions TCP établies sur un port local (`ss`, sans privilège).
pub fn connections(port: u16) -> Vec<Peer> {
    if crate::demo() {
        return crate::demo::connections(port);
    }
    let filter = format!("( sport = :{port} )");
    let Some(out) = run("ss", &["-Htn", "state", "established", &filter]) else { return vec![] };
    let mut peers: Vec<Peer> = vec![];
    for l in out.lines() {
        // Recv-Q Send-Q Local:Port Peer:Port
        let Some(peer) = l.split_whitespace().nth(3) else { continue };
        let addr = peer.rsplit_once(':').map(|(a, _)| a).unwrap_or(peer).trim_matches(['[', ']']).trim_start_matches("::ffff:").to_string();
        if !peers.iter().any(|p| p.address == addr) {
            peers.push(Peer { address: addr, name: None, user: None });
        }
    }
    peers
}

/// Comptes connectés en SSH (`who` : sessions avec une adresse distante).
pub fn ssh_users() -> Vec<(String, String)> {
    if crate::demo() {
        return vec![("axel".into(), "192.168.1.31".into())];
    }
    run("who", &[]).unwrap_or_default().lines().filter_map(|l| {
        let user = l.split_whitespace().next()?;
        let host = l.rsplit_once('(')?.1.trim_end_matches(')');
        (!host.starts_with(':') && !host.is_empty()).then(|| (user.to_string(), host.to_string()))
    }).collect()
}

/// Nom d'un appareil du réseau (résolution inverse : mDNS si `libnss-mdns`, DNS de la box).
pub fn resolve(ip: &str) -> Option<String> {
    if crate::demo() {
        return crate::demo::resolve(ip);
    }
    let out = Command::new("timeout").args(["1", "getent", "hosts", ip]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let name = s.split_whitespace().nth(1)?.trim_end_matches('.');
    (name != ip).then(|| name.to_string())
}

/// Adresses IP de ce PC sur le réseau local (hors boucle locale, VPN et conteneurs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalAddr {
    pub interface: String,
    pub ip: String,
    pub v6: bool,
}

pub fn local_addresses() -> Vec<LocalAddr> {
    if crate::demo() {
        return vec![LocalAddr { interface: "enp5s0".into(), ip: "192.168.1.20".into(), v6: false }];
    }
    let Some(out) = run("ip", &["-o", "addr", "show", "scope", "global"]) else { return vec![] };
    parse_ip_addr(&out)
}

pub fn parse_ip_addr(out: &str) -> Vec<LocalAddr> {
    let skip = |i: &str| ["lo", "docker", "virbr", "veth", "br-", "wg", "tun", "tap", "podman", "lxc", "vnet"].iter().any(|p| i.starts_with(p));
    let mut v: Vec<LocalAddr> = out
        .lines()
        .filter_map(|l| {
            // 2: enp5s0    inet 192.168.1.20/24 brd … scope global dynamic …
            let f: Vec<&str> = l.split_whitespace().collect();
            let iface = f.get(1)?.trim_end_matches(':');
            let pos = f.iter().position(|x| *x == "inet" || *x == "inet6")?;
            let ip = f.get(pos + 1)?.split('/').next()?;
            if skip(iface) || l.contains("temporary") || l.contains("deprecated") {
                return None;
            }
            Some(LocalAddr { interface: iface.to_string(), ip: ip.to_string(), v6: f[pos] == "inet6" })
        })
        .collect();
    v.sort_by_key(|a| a.v6);
    v
}

pub fn hostname() -> String {
    if crate::demo() {
        return "salon".into();
    }
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "noos".into())
}

/// Mot de passe de partage défini pour ce compte.
pub fn samba_password_set(user: &str) -> bool {
    if crate::demo() {
        return std::env::var("NOOS_SHARE_DEMO").as_deref() != Ok("nopass");
    }
    std::fs::read_to_string(crate::model::SAMBA_USERS).map(|t| t.lines().any(|l| l.trim() == user)).unwrap_or(false)
}

pub fn current_user() -> String {
    if crate::demo() {
        return "axel".into();
    }
    std::env::var("USER").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| run("id", &["-un"]).map(|s| s.trim().to_string()).unwrap_or_default())
}

// ───────────────────────────── SSH ─────────────────────────────

/// Clé publique de `authorized_keys`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthorizedKey {
    /// Ligne complète (sert à la retirer).
    pub line: String,
    pub kind: String,
    pub comment: String,
    pub fingerprint: String,
}

pub fn authorized_keys_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".ssh/authorized_keys")
}

pub fn authorized_keys() -> Vec<AuthorizedKey> {
    if crate::demo() {
        return crate::demo::keys();
    }
    std::fs::read_to_string(authorized_keys_path()).map(|t| parse_authorized_keys(&t)).unwrap_or_default()
}

/// Types de clés reconnus (le reste d'une ligne peut commencer par des options).
const KEY_TYPES: &[&str] = &["ssh-ed25519", "ssh-rsa", "ecdsa-sha2-nistp256", "ecdsa-sha2-nistp384", "ecdsa-sha2-nistp521", "sk-ssh-ed25519@openssh.com", "sk-ecdsa-sha2-nistp256@openssh.com", "ssh-dss"];

pub fn parse_authorized_keys(t: &str) -> Vec<AuthorizedKey> {
    t.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let pos = f.iter().position(|x| KEY_TYPES.contains(x))?;
            let blob = f.get(pos + 1)?;
            let comment = f[pos + 2..].join(" ");
            Some(AuthorizedKey { line: l.to_string(), kind: key_label(f[pos]).into(), comment, fingerprint: fingerprint(blob) })
        })
        .collect()
}

/// Vérifie une clé collée (une seule ligne, type connu) et la rend normalisée.
pub fn validate_public_key(s: &str) -> anyhow::Result<String> {
    let line = s.trim();
    if line.contains('\n') {
        anyhow::bail!("une seule clé à la fois (une ligne)");
    }
    let keys = parse_authorized_keys(line);
    let Some(k) = keys.first() else { anyhow::bail!("ce n'est pas une clé publique SSH (elle commence par « ssh-ed25519 », « ssh-rsa »…)") };
    if k.fingerprint.is_empty() {
        anyhow::bail!("clé illisible");
    }
    if line.contains("PRIVATE KEY") {
        anyhow::bail!("c'est une clé privée : ne la partagez jamais. Collez le fichier .pub");
    }
    Ok(line.to_string())
}

pub fn key_label(kind: &str) -> &'static str {
    match kind {
        "ssh-ed25519" => "Ed25519",
        "ssh-rsa" => "RSA",
        "ssh-dss" => "DSA (obsolète)",
        k if k.starts_with("sk-") => "Clé de sécurité (FIDO)",
        k if k.starts_with("ecdsa") => "ECDSA",
        _ => "SSH",
    }
}

/// Empreinte SHA-256 d'une clé (format d'OpenSSH : `SHA256:…`, base64 sans `=`).
pub fn fingerprint(blob_b64: &str) -> String {
    let Some(raw) = base64_decode(blob_b64) else { return String::new() };
    format!("SHA256:{}", base64_encode(&sha256(&raw)).trim_end_matches('='))
}

/// Empreintes des clés de ce PC (`/etc/ssh/ssh_host_*_key.pub`, lisibles par tous) : à
/// comparer à ce qu'affiche le client à la première connexion.
pub fn host_fingerprints() -> Vec<(String, String)> {
    if crate::demo() {
        return vec![("Ed25519".into(), "SHA256:q3R1x7oVhJcW0bS9yZ2mT6kLp4eF8nGdA5uHrQiC1sE".into()), ("ECDSA".into(), "SHA256:Zk9mB2xT0aQ7vL5nR3wY8cE1dF6gH4jK2pS9uV0tXyA".into())];
    }
    let mut v = vec![];
    for k in ["ed25519", "ecdsa", "rsa"] {
        if let Ok(t) = std::fs::read_to_string(format!("/etc/ssh/ssh_host_{k}_key.pub")) {
            let f: Vec<&str> = t.split_whitespace().collect();
            if f.len() >= 2 {
                v.push((key_label(f[0]).to_string(), fingerprint(f[1])));
            }
        }
    }
    v
}

// Petits utilitaires (pas de dépendance pour 60 lignes).

pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=').collect();
    let mut o = vec![];
    for c in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, b) in c.iter().enumerate() {
            n |= val(*b)? << (18 - 6 * i);
        }
        o.push((n >> 16) as u8);
        if c.len() > 2 {
            o.push((n >> 8) as u8);
        }
        if c.len() > 3 {
            o.push(n as u8);
        }
    }
    Some(o)
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut o = [0u8; 20];
    for i in 0..5 {
        o[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    o
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut o = [0u8; 32];
    for i in 0..8 {
        o[4 * i..4 * i + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hachage() {
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(&[b'a'; 1000])), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
        assert_eq!(base64_encode(b"NOOS!"), "Tk9PUyE=");
        assert_eq!(base64_decode("Tk9PUyE=").unwrap(), b"NOOS!");
    }

    #[test]
    fn cles() {
        // Clé jetable ; empreinte attendue donnée par `ssh-keygen -lf`.
        let t = "# commentaire\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIOZVNI6UJstqKq/kGdLGLkrkzlq0fl++VytYfrSmi2w axel@portable\n\nfrom=\"192.168.1.0/24\" ssh-rsa AAAAB3NzaC1yc2E= téléphone\nnimporte quoi\n";
        let k = parse_authorized_keys(t);
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].kind, "Ed25519");
        assert_eq!(k[0].comment, "axel@portable");
        assert_eq!(k[0].fingerprint, "SHA256:Pkjf65dMc2mVS3AC6KiwgpeI69+jdWCCFhOD/m7br8U");
        assert_eq!(k[1].comment, "téléphone");
        assert!(validate_public_key("-----BEGIN OPENSSH PRIVATE KEY-----").is_err());
        assert!(validate_public_key("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIOZVNI6UJstqKq/kGdLGLkrkzlq0fl++VytYfrSmi2w x").is_ok());
        assert!(validate_public_key("a\nb").is_err());
    }

    #[test]
    fn adresses() {
        let out = "2: enp5s0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic noprefixroute enp5s0\\       valid_lft 80000sec\n\
                   3: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\n\
                   2: enp5s0    inet6 2a01:e0a::1234/64 scope global temporary dynamic\n\
                   2: enp5s0    inet6 2a01:e0a::20/64 scope global dynamic mngtmpaddr\n";
        let a = parse_ip_addr(out);
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].ip, "192.168.1.20");
        assert!(a[1].v6);
    }
}

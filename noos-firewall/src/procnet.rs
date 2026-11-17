//! Qui a ouvert cette connexion ? Table des sockets (`/proc/net/{tcp,udp}{,6}`) → inode →
//! processus (`/proc/<pid>/fd/*` = `socket:[inode]`) → programme (`/proc/<pid>/exe`, cmdline,
//! Flatpak, AppImage).

use std::{
    collections::HashMap,
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::Path,
};

use crate::model::Program;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SockEntry {
    pub local: IpAddr,
    pub local_port: u16,
    pub remote: IpAddr,
    pub remote_port: u16,
    /// État TCP (0x0A = LISTEN, 0x02 = SYN_SENT…), 0x07 pour UDP non connecté.
    pub state: u8,
    pub uid: u32,
    pub inode: u64,
}

pub const TCP_LISTEN: u8 = 0x0a;

/// « 0100007F:0035 » (IPv4) ou 32 chiffres hexadécimaux (IPv6), mots de 32 bits dans l'ordre
/// de la machine.
fn parse_addr(s: &str) -> Option<(IpAddr, u16)> {
    let (a, p) = s.split_once(':')?;
    let port = u16::from_str_radix(p, 16).ok()?;
    let ip = match a.len() {
        8 => IpAddr::V4(Ipv4Addr::from(u32::from_str_radix(a, 16).ok()?.to_ne_bytes())),
        32 => {
            let mut b = [0u8; 16];
            for i in 0..4 {
                let w = u32::from_str_radix(&a[i * 8..i * 8 + 8], 16).ok()?;
                b[i * 4..i * 4 + 4].copy_from_slice(&w.to_ne_bytes());
            }
            IpAddr::V6(Ipv6Addr::from(b))
        }
        _ => return None,
    };
    Some((ip, port))
}

pub fn parse_table(text: &str) -> Vec<SockEntry> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 10 {
                return None;
            }
            let (local, local_port) = parse_addr(f[1])?;
            let (remote, remote_port) = parse_addr(f[2])?;
            Some(SockEntry { local, local_port, remote, remote_port, state: u8::from_str_radix(f[3], 16).ok()?, uid: f[7].parse().ok()?, inode: f[9].parse().ok()? })
        })
        .collect()
}

/// Forme IPv4 d'une adresse IPv6 « mappée » (::ffff:a.b.c.d).
fn canon(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn addr_eq_or_any(sock: IpAddr, pkt: IpAddr) -> bool {
    sock.is_unspecified() || canon(sock) == canon(pkt)
}

/// Socket d'une connexion sortante : port local = port source du paquet. Les correspondances
/// exactes (adresse et port distants) passent avant les sockets UDP non connectés.
pub fn find_outgoing(entries: &[SockEntry], src: IpAddr, sport: u16, dst: IpAddr, dport: u16) -> Option<&SockEntry> {
    let cands: Vec<&SockEntry> = entries.iter().filter(|e| e.local_port == sport && addr_eq_or_any(e.local, src) && e.inode != 0).collect();
    cands.iter().find(|e| e.remote_port == dport && canon(e.remote) == canon(dst)).or_else(|| cands.iter().find(|e| e.remote_port == 0)).or_else(|| cands.first()).copied()
}

/// Socket qui reçoit une connexion entrante : en écoute (TCP) ou lié (UDP) au port visé.
pub fn find_incoming(entries: &[SockEntry], tcp: bool, dst: IpAddr, dport: u16) -> Option<&SockEntry> {
    entries.iter().find(|e| e.local_port == dport && (!tcp || e.state == TCP_LISTEN) && e.remote_port == 0 && addr_eq_or_any(e.local, dst) && e.inode != 0)
}

/// Lecture des deux tables (IPv4 + IPv6) d'un protocole.
pub fn read_tables(proc_root: &Path, proto: u8) -> Vec<SockEntry> {
    let names: &[&str] = match proto {
        6 => &["tcp", "tcp6"],
        17 => &["udp", "udp6"],
        136 => &["udplite", "udplite6"],
        _ => &[],
    };
    names.iter().flat_map(|n| parse_table(&fs::read_to_string(proc_root.join("net").join(n)).unwrap_or_default())).collect()
}

/// Inode de socket → PID. Le cache est refait quand un inode est introuvable.
#[derive(Default)]
pub struct PidCache {
    map: HashMap<u64, u32>,
}

impl PidCache {
    pub fn pid_of(&mut self, proc_root: &Path, inode: u64) -> Option<u32> {
        if let Some(&pid) = self.map.get(&inode)
            && fs::read_link(proc_root.join(pid.to_string()).join("fd")).is_ok()
        {
            return Some(pid);
        }
        self.rescan(proc_root);
        self.map.get(&inode).copied()
    }

    fn rescan(&mut self, proc_root: &Path) {
        self.map.clear();
        let Ok(dir) = fs::read_dir(proc_root) else {
            return;
        };
        for e in dir.flatten() {
            let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            let Ok(fds) = fs::read_dir(e.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                if let Ok(target) = fs::read_link(fd.path())
                    && let Some(ino) = target.to_str().and_then(|t| t.strip_prefix("socket:[")).and_then(|t| t.strip_suffix(']')).and_then(|t| t.parse::<u64>().ok())
                {
                    // Socket partagé (fork) : le processus le plus ancien (PID le plus petit) reste.
                    self.map.entry(ino).or_insert(pid);
                }
            }
        }
    }
}

/// Clé de règle d'un programme : Flatpak, AppImage, sinon exécutable.
pub fn program_key(exe: &str, flatpak: Option<&str>, appimage: Option<&str>) -> String {
    if let Some(id) = flatpak {
        return format!("flatpak:{id}");
    }
    if let Some(p) = appimage {
        return format!("appimage:{p}");
    }
    exe.to_string()
}

/// Identifiant Flatpak d'après `/.flatpak-info` (section [Application], clé name).
pub fn parse_flatpak_info(text: &str) -> Option<String> {
    let mut in_app = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_app = l == "[Application]";
            continue;
        }
        if in_app && let Some(v) = l.strip_prefix("name=") {
            return Some(v.trim().to_string());
        }
    }
    None
}

pub fn user_name(uid: u32) -> String {
    fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|t| {
            t.lines().find_map(|l| {
                let f: Vec<&str> = l.split(':').collect();
                (f.len() > 2 && f[2].parse::<u32>().ok() == Some(uid)).then(|| f[0].to_string())
            })
        })
        .unwrap_or_else(|| uid.to_string())
}

/// Programme d'un processus (lu dans `/proc/<pid>`, demande root pour les autres comptes).
pub fn program_of(proc_root: &Path, pid: u32, uid: u32) -> Program {
    let base = proc_root.join(pid.to_string());
    let exe = fs::read_link(base.join("exe")).map(|p| p.to_string_lossy().trim_end_matches(" (deleted)").to_string()).unwrap_or_default();
    let cmdline = fs::read(base.join("cmdline"))
        .map(|b| b.split(|&c| c == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let comm = fs::read_to_string(base.join("comm")).map(|s| s.trim().to_string()).unwrap_or_default();
    let flatpak = fs::read_to_string(base.join("root/.flatpak-info")).ok().and_then(|t| parse_flatpak_info(&t));
    let appimage =
        fs::read(base.join("environ")).ok().and_then(|b| b.split(|&c| c == 0).find_map(|kv| kv.strip_prefix(b"APPIMAGE=").map(|v| String::from_utf8_lossy(v).to_string())));
    // Le chemin d'un AppImage monté change à chaque lancement (/tmp/.mount_XXXX) : seul
    // $APPIMAGE est stable, et seulement si l'exécutable vient bien du montage.
    let appimage = appimage.filter(|_| exe.contains("/.mount_"));
    let key = if exe.is_empty() && flatpak.is_none() { "inconnu".to_string() } else { program_key(&exe, flatpak.as_deref(), appimage.as_deref()) };
    Program { key, exe, cmdline, comm, pid, uid, user: user_name(uid), flatpak }
}

/// Date de démarrage d'un processus (champ 22 de /proc/<pid>/stat), pour polkit.
pub fn start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Le nom (champ 2) peut contenir des espaces : on repart après la dernière « ) ».
    let rest = &stat[stat.rfind(')')? + 2..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:0277 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 21311 1 0000000000000000 100 0 0 10 0
   1: 0A01A8C0:C822 0370528C:01BB 02 00000001:00000000 01:00000060 00000000  1000        0 98765 2 0000000000000000 20 4 0 10 -1
";
    const TCP6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:63DD 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 55555 1 0000000000000000 100 0 0 10 0
   1: 0000000000000000FFFF00000A01A8C0:D000 0000000000000000FFFF00000370528C:0050 02 00000000:00000000 00:00000000 00000000  1000        0 66666 1 0000000000000000 100 0 0 10 0
";

    #[test]
    fn parse_v4() {
        let t = parse_table(TCP);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].local.to_string(), "127.0.0.1");
        assert_eq!(t[0].local_port, 631);
        assert_eq!(t[0].state, TCP_LISTEN);
        assert_eq!(t[1].local.to_string(), "192.168.1.10");
        assert_eq!(t[1].remote.to_string(), "140.82.112.3");
        assert_eq!((t[1].remote_port, t[1].uid, t[1].inode), (443, 1000, 98765));
    }

    #[test]
    fn parse_v6_and_find() {
        let mut t = parse_table(TCP);
        t.extend(parse_table(TCP6));
        assert_eq!(t[2].local_port, 25565);
        assert!(t[2].local.is_unspecified());
        // Socket IPv6 double pile utilisé en IPv4 (::ffff:).
        let src: IpAddr = "192.168.1.10".parse().unwrap();
        let dst: IpAddr = "140.82.112.3".parse().unwrap();
        assert_eq!(find_outgoing(&t, src, 0xd000, dst, 80).unwrap().inode, 66666);
        assert_eq!(find_outgoing(&t, src, 51234, dst, 443).unwrap().inode, 98765);
        assert!(find_outgoing(&t, src, 1, dst, 443).is_none());
        // Entrée : Minecraft écoute sur [::]:25565 et reçoit de l'IPv4.
        assert_eq!(find_incoming(&t, true, src, 25565).unwrap().inode, 55555);
        // 631 n'écoute que sur 127.0.0.1.
        assert!(find_incoming(&t, true, src, 631).is_none());
        assert!(find_incoming(&t, true, "127.0.0.1".parse().unwrap(), 631).is_some());
    }

    #[test]
    fn keys() {
        assert_eq!(program_key("/usr/bin/curl", None, None), "/usr/bin/curl");
        assert_eq!(program_key("/app/lib/firefox/firefox", Some("org.mozilla.firefox"), None), "flatpak:org.mozilla.firefox");
        assert_eq!(program_key("/tmp/.mount_x/AppRun", None, Some("/home/a/Applications/Kudu.AppImage")), "appimage:/home/a/Applications/Kudu.AppImage");
        let info = "[Application]\nname=org.mozilla.firefox\nruntime=runtime/org.freedesktop.Platform\n\n[Instance]\ninstance-id=1\n";
        assert_eq!(parse_flatpak_info(info).as_deref(), Some("org.mozilla.firefox"));
    }

    #[test]
    fn own_process() {
        let pid = std::process::id();
        let p = program_of(Path::new("/proc"), pid, 0);
        assert!(!p.exe.is_empty());
        assert_eq!(p.key, p.exe);
        assert!(start_time(pid).is_some());
    }
}

//! Accès à NetworkManager par `nmcli` (sortie « terse ») : connexions, appareils, Wi-Fi,
//! WireGuard, DNS. Les membres de la session active ont le droit de modifier les connexions
//! système sans mot de passe (polkit `settings.modify.system`).
//!
//! Démo sans NetworkManager : `NOOS_NET_DEMO=1`.

use std::{
    collections::HashMap,
    net::{SocketAddr, TcpStream, UdpSocket},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};

pub fn demo() -> bool {
    std::env::var_os("NOOS_NET_DEMO").is_some()
}

/// `nmcli <args>` ; erreur avec le message de nmcli.
pub fn nmcli(args: &[&str]) -> anyhow::Result<String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(300));
        return Ok(String::new());
    }
    let out = Command::new("nmcli").args(args).env("LC_ALL", "C.UTF-8").output().context("nmcli introuvable")?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr);
        bail!("{}", e.trim().trim_start_matches("Error: "));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Découpe une ligne « terse » (champs séparés par `:`, `\:` et `\\` échappés).
pub fn split_terse(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for c in line.chars() {
        if esc {
            out.last_mut().unwrap().push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out
}

fn rows(args: &[&str]) -> Vec<Vec<String>> {
    nmcli(args).map(|s| s.lines().filter(|l| !l.is_empty()).map(split_terse).collect()).unwrap_or_default()
}

/// `nmcli -t -f a,b show …` sous forme clé → valeur (une ligne par champ, mode -m multiline).
fn fields(args: &[&str]) -> HashMap<String, String> {
    nmcli(args)
        .map(|s| {
            s.lines()
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.to_string(), v.replace("\\:", ":").replace("\\\\", "\\")))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub name: String,
    pub uuid: String,
    pub kind: String,
    pub device: String,
    pub active: bool,
}

impl Connection {
    pub fn is_wireguard(&self) -> bool {
        self.kind == "wireguard"
    }
    pub fn is_wifi(&self) -> bool {
        self.kind == "802-11-wireless"
    }
    pub fn is_lan(&self) -> bool {
        matches!(self.kind.as_str(), "802-3-ethernet" | "802-11-wireless")
    }
}

pub fn connections() -> Vec<Connection> {
    if demo() {
        return vec![
            Connection { name: "Connexion filaire 1".into(), uuid: "a1".into(), kind: "802-3-ethernet".into(), device: "eno1".into(), active: true },
            Connection { name: "Maison-5G".into(), uuid: "w1".into(), kind: "802-11-wireless".into(), device: "".into(), active: false },
            Connection { name: "Mullvad Paris".into(), uuid: "v1".into(), kind: "wireguard".into(), device: "".into(), active: false },
            Connection { name: "Proton Suisse".into(), uuid: "v2".into(), kind: "wireguard".into(), device: "wg-ch".into(), active: true },
        ];
    }
    rows(&["-t", "-f", "NAME,UUID,TYPE,DEVICE,ACTIVE", "connection", "show"])
        .into_iter()
        .filter(|r| r.len() >= 5)
        .map(|r| Connection { name: r[0].clone(), uuid: r[1].clone(), kind: r[2].clone(), device: r[3].clone(), active: r[4] == "yes" })
        .collect()
}

/// Connexion principale : la première active, filaire ou Wi-Fi.
pub fn primary() -> Option<Connection> {
    let all = connections();
    all.iter().find(|c| c.active && c.kind == "802-3-ethernet").or_else(|| all.iter().find(|c| c.active && c.is_wifi())).cloned()
}

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub name: String,
    pub kind: String,
    pub state: String,
    pub connection: String,
}

pub fn devices() -> Vec<Device> {
    if demo() {
        return vec![
            Device { name: "eno1".into(), kind: "ethernet".into(), state: "connected".into(), connection: "Connexion filaire 1".into() },
            Device { name: "wlp2s0".into(), kind: "wifi".into(), state: "disconnected".into(), connection: "".into() },
        ];
    }
    rows(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])
        .into_iter()
        .filter(|r| r.len() >= 4 && r[1] != "loopback")
        .map(|r| Device { name: r[0].clone(), kind: r[1].clone(), state: r[2].clone(), connection: r[3].clone() })
        .collect()
}

pub fn wifi_device() -> Option<String> {
    devices().into_iter().find(|d| d.kind == "wifi").map(|d| d.name)
}

/// Détails d'un appareil : IP4.ADDRESS[1], IP4.GATEWAY, IP4.DNS[1], GENERAL.HWADDR…
pub fn device_info(dev: &str) -> HashMap<String, String> {
    if demo() {
        return [
            ("GENERAL.HWADDR", "6C:4B:90:3A:11:F2"),
            ("GENERAL.MTU", "1500"),
            ("IP4.ADDRESS[1]", "192.168.1.42/24"),
            ("IP4.GATEWAY", "192.168.1.1"),
            ("IP4.DNS[1]", "9.9.9.9"),
            ("IP4.DNS[2]", "149.112.112.112"),
            ("IP6.ADDRESS[1]", "2a01:cb0c:599:3000::42/64"),
            ("IP6.GATEWAY", "fe80::1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    }
    fields(&["-t", "-f", "GENERAL,IP4,IP6", "device", "show", dev])
}

/// Valeurs indexées (`IP4.DNS[1]`, `IP4.DNS[2]`…) dans l'ordre.
pub fn indexed(info: &HashMap<String, String>, key: &str) -> Vec<String> {
    (1..20).map_while(|i| info.get(&format!("{key}[{i}]")).cloned()).filter(|v| !v.is_empty()).collect()
}

/// Réglages d'une connexion (`ipv4.dns`, `connection.dns-over-tls`…).
pub fn con_fields(uuid: &str, keys: &str) -> HashMap<String, String> {
    if demo() {
        let mut m: HashMap<String, String> = [
            ("ipv4.method", "auto"),
            ("ipv6.method", "auto"),
            ("ipv4.dns", "9.9.9.9#dns.quad9.net,149.112.112.112#dns.quad9.net"),
            ("ipv4.ignore-auto-dns", "yes"),
            ("connection.dns-over-tls", "yes"),
            ("connection.autoconnect", "no"),
            ("ipv4.addresses", "10.64.12.7/32"),
            ("wireguard.peers", "Xk3uYw8p6hG…= endpoint=185.65.135.2:51820 allowed-ips=0.0.0.0/0;::/0 persistent-keepalive=25"),
            ("802-11-wireless.cloned-mac-address", "stable"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        if uuid == "v1" {
            m.insert("wireguard.peers".into(), "Hm1cQ…= endpoint=193.32.249.66:51820 allowed-ips=0.0.0.0/0".into());
            m.insert("ipv4.addresses".into(), "10.68.4.21/32".into());
            m.insert("ipv4.dns".into(), "10.64.0.1".into());
        }
        return m;
    }
    fields(&["-t", "-f", keys, "connection", "show", uuid])
}

pub fn modify(uuid: &str, settings: &[(&str, &str)]) -> anyhow::Result<()> {
    let mut args = vec!["connection", "modify", uuid];
    for (k, v) in settings {
        args.push(k);
        args.push(v);
    }
    nmcli(&args).map(|_| ())
}

/// Applique les changements à chaud (sans déconnecter) ; sinon réactive la connexion.
pub fn reapply(c: &Connection) -> anyhow::Result<()> {
    if c.device.is_empty() {
        return Ok(());
    }
    if nmcli(&["device", "reapply", &c.device]).is_err() {
        nmcli(&["connection", "up", &c.uuid])?;
    }
    Ok(())
}

// ───────────────────────────── Débit ─────────────────────────────

/// Octets reçus / envoyés par un appareil (compteurs du noyau).
pub fn counters(dev: &str) -> Option<(u64, u64)> {
    if demo() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64() % 3600.0 + 1800.0;
        let rx = (t * 2.2e6 + (t * 0.7).sin() * 3.0e6 + (t * 2.3).sin().abs() * 1.5e6) as u64;
        let tx = (t * 2.6e5 + (t * 1.1).cos() * 2.0e5) as u64;
        return Some((rx, tx));
    }
    let read = |f: &str| std::fs::read_to_string(format!("/sys/class/net/{dev}/statistics/{f}")).ok()?.trim().parse().ok();
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

pub fn link_speed(dev: &str) -> Option<u32> {
    if demo() {
        return Some(1000);
    }
    std::fs::read_to_string(format!("/sys/class/net/{dev}/speed")).ok()?.trim().parse().ok().filter(|s: &u32| *s > 0 && *s < 1_000_000)
}

pub fn human_rate(bps: f64) -> String {
    let (v, u) = if bps >= 1e9 {
        (bps / 1e9, "Go/s")
    } else if bps >= 1e6 {
        (bps / 1e6, "Mo/s")
    } else if bps >= 1e3 {
        (bps / 1e3, "Ko/s")
    } else {
        (bps, "o/s")
    };
    if v >= 100.0 || u == "o/s" { format!("{v:.0} {u}") } else { format!("{v:.1} {u}").replace('.', ",") }
}

pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    let (v, u) = if b >= 1e12 {
        (b / 1e12, "To")
    } else if b >= 1e9 {
        (b / 1e9, "Go")
    } else if b >= 1e6 {
        (b / 1e6, "Mo")
    } else {
        (b / 1e3, "Ko")
    };
    format!("{v:.1} {u}").replace('.', ",")
}

// ───────────────────────────── DNS ─────────────────────────────

/// Fournisseur DNS proposé : adresses, nom TLS (DNS over TLS) et politique de confidentialité.
pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub company: &'static str,
    pub flag: &'static str,
    pub country: &'static str,
    /// Droit applicable / remarques sur la juridiction.
    pub jurisdiction: &'static str,
    pub logging: &'static str,
    pub filtering: &'static str,
    /// 3 = excellente, 2 = bonne, 1 = moyenne (appréciation de NOOS).
    pub privacy: u8,
    pub no_logs: bool,
    pub blocks: Option<&'static str>,
    pub v4: &'static [&'static str],
    pub v6: &'static [&'static str],
    /// Nom pour DNS over TLS (vide = DoT non proposé).
    pub tls: &'static str,
    /// Uniquement chiffré (pas de DNS classique sur le port 53).
    pub tls_only: bool,
}

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "quad9",
        name: "Quad9",
        company: "Quad9 Foundation (fondation à but non lucratif)",
        flag: "🇨🇭",
        country: "Suisse",
        jurisdiction: "Droit suisse (protection des données parmi les plus strictes) ; applique aussi le RGPD",
        logging: "Aucune adresse IP enregistrée ; seulement des statistiques globales anonymes",
        filtering: "Bloque les domaines malveillants (hameçonnage, logiciels malveillants)",
        privacy: 3,
        no_logs: true,
        blocks: Some("Menaces"),
        v4: &["9.9.9.9", "149.112.112.112"],
        v6: &["2620:fe::fe", "2620:fe::9"],
        tls: "dns.quad9.net",
        tls_only: false,
    },
    Provider {
        id: "fdn",
        name: "FDN",
        company: "French Data Network (association, fournisseur d'accès associatif)",
        flag: "🇫🇷",
        country: "France",
        jurisdiction: "Droit français et RGPD ; association militante pour la neutralité du Net",
        logging: "Pas de journalisation des requêtes",
        filtering: "Aucun filtrage ni censure",
        privacy: 3,
        no_logs: true,
        blocks: None,
        v4: &["80.67.169.12", "80.67.169.40"],
        v6: &["2001:910:800::12", "2001:910:800::40"],
        tls: "ns0.fdn.fr",
        tls_only: false,
    },
    Provider {
        id: "adguard",
        name: "AdGuard DNS",
        company: "AdGuard Software Ltd",
        flag: "🇨🇾",
        country: "Chypre",
        jurisdiction: "Droit chypriote et RGPD",
        logging: "Pas de journaux nominatifs ; statistiques anonymes agrégées",
        filtering: "Bloque publicités, traqueurs et hameçonnage pour tout l'ordinateur",
        privacy: 2,
        no_logs: true,
        blocks: Some("Pubs et traqueurs"),
        v4: &["94.140.14.14", "94.140.15.15"],
        v6: &["2a10:50c0::ad1:ff", "2a10:50c0::ad2:ff"],
        tls: "dns.adguard-dns.com",
        tls_only: false,
    },
    Provider {
        id: "dns4eu",
        name: "DNS4EU",
        company: "Consortium mené par Whalebone, cofinancé par l'Union européenne",
        flag: "🇪🇺",
        country: "Union européenne (Tchéquie)",
        jurisdiction: "Droit européen (RGPD), serveurs dans l'UE",
        logging: "Journaux limités, conformes au RGPD",
        filtering: "Aucun (version « sans filtrage »)",
        privacy: 2,
        no_logs: false,
        blocks: None,
        v4: &["86.54.11.100", "86.54.11.200"],
        v6: &[],
        tls: "unfiltered.joindns4.eu",
        tls_only: false,
    },
    Provider {
        id: "cloudflare",
        name: "Cloudflare",
        company: "Cloudflare, Inc.",
        flag: "🇺🇸",
        country: "États-Unis",
        jurisdiction: "Droit américain (Cloud Act : accès possible des autorités américaines)",
        logging: "Adresse IP jamais écrite sur disque ; journaux anonymisés effacés sous 25 h ; audit indépendant",
        filtering: "Aucun filtrage",
        privacy: 2,
        no_logs: true,
        blocks: None,
        v4: &["1.1.1.1", "1.0.0.1"],
        v6: &["2606:4700:4700::1111", "2606:4700:4700::1001"],
        tls: "one.one.one.one",
        tls_only: false,
    },
    Provider {
        id: "cloudflare-family",
        name: "Cloudflare Familles",
        company: "Cloudflare, Inc.",
        flag: "🇺🇸",
        country: "États-Unis",
        jurisdiction: "Droit américain (Cloud Act)",
        logging: "Comme Cloudflare : journaux anonymisés effacés sous 25 h",
        filtering: "Bloque logiciels malveillants et contenus pour adultes (contrôle parental)",
        privacy: 2,
        no_logs: true,
        blocks: Some("Adultes et menaces"),
        v4: &["1.1.1.3", "1.0.0.3"],
        v6: &["2606:4700:4700::1113", "2606:4700:4700::1003"],
        tls: "family.cloudflare-dns.com",
        tls_only: false,
    },
    Provider {
        id: "google",
        name: "Google Public DNS",
        company: "Google LLC",
        flag: "🇺🇸",
        country: "États-Unis",
        jurisdiction: "Droit américain (Cloud Act)",
        logging: "Adresse IP complète conservée 24 à 48 h ; journaux permanents sans IP (ville, opérateur)",
        filtering: "Aucun filtrage",
        privacy: 1,
        no_logs: false,
        blocks: None,
        v4: &["8.8.8.8", "8.8.4.4"],
        v6: &["2001:4860:4860::8888", "2001:4860:4860::8844"],
        tls: "dns.google",
        tls_only: false,
    },
    Provider {
        id: "opendns",
        name: "OpenDNS",
        company: "Cisco Systems",
        flag: "🇺🇸",
        country: "États-Unis",
        jurisdiction: "Droit américain (Cloud Act)",
        logging: "Requêtes journalisées et utilisées pour des statistiques",
        filtering: "Bloque l'hameçonnage",
        privacy: 1,
        no_logs: false,
        blocks: Some("Hameçonnage"),
        v4: &["208.67.222.222", "208.67.220.220"],
        v6: &["2620:119:35::35", "2620:119:53::53"],
        tls: "",
        tls_only: false,
    },
];

/// Mode de chiffrement des requêtes (connection.dns-over-tls).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tls {
    Strict,
    Opportunistic,
    Off,
}

/// Réglage DNS d'une connexion.
#[derive(Debug, Clone, PartialEq)]
pub enum DnsChoice {
    Auto,
    Provider(&'static str),
    Custom(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct DnsState {
    pub choice: DnsChoice,
    pub tls: Tls,
    /// Serveurs réellement utilisés (IP4.DNS / IP6.DNS de l'appareil).
    pub in_use: Vec<String>,
}

pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

pub fn dns_state(c: &Connection) -> DnsState {
    let f = con_fields(&c.uuid, "ipv4.dns,ipv6.dns,ipv4.ignore-auto-dns,connection.dns-over-tls");
    let list: Vec<String> = [f.get("ipv4.dns"), f.get("ipv6.dns")]
        .into_iter()
        .flatten()
        .flat_map(|s| s.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>())
        .filter(|s| !s.is_empty() && s != "--")
        .collect();
    let ips: Vec<String> = list.iter().map(|s| s.split('#').next().unwrap_or("").to_string()).collect();
    let choice = if ips.is_empty() {
        DnsChoice::Auto
    } else if let Some(p) = PROVIDERS.iter().find(|p| p.v4.first().is_some_and(|a| ips.iter().any(|i| i == a))) {
        DnsChoice::Provider(p.id)
    } else {
        DnsChoice::Custom(ips)
    };
    let tls = match f.get("connection.dns-over-tls").map(String::as_str) {
        Some("yes" | "2") => Tls::Strict,
        Some("opportunistic" | "1") => Tls::Opportunistic,
        _ => Tls::Off,
    };
    let info = device_info(&c.device);
    let mut in_use = indexed(&info, "IP4.DNS");
    in_use.extend(indexed(&info, "IP6.DNS"));
    DnsState { choice, tls, in_use }
}

/// Applique un choix DNS à une connexion (puis à chaud sur l'appareil).
pub fn set_dns(c: &Connection, choice: &DnsChoice, tls: Tls) -> anyhow::Result<()> {
    let f = con_fields(&c.uuid, "ipv6.method");
    let v6_ok = !matches!(f.get("ipv6.method").map(String::as_str), Some("disabled" | "ignore"));
    let tag = |ip: &str, name: &str| if name.is_empty() || tls == Tls::Off { ip.to_string() } else { format!("{ip}#{name}") };
    let (v4, v6, auto) = match choice {
        DnsChoice::Auto => (String::new(), String::new(), true),
        DnsChoice::Provider(id) => {
            let p = provider(id).context("fournisseur inconnu")?;
            (
                p.v4.iter().map(|a| tag(a, p.tls)).collect::<Vec<_>>().join(","),
                p.v6.iter().map(|a| tag(a, p.tls)).collect::<Vec<_>>().join(","),
                false,
            )
        }
        DnsChoice::Custom(list) => {
            let (six, four): (Vec<&String>, Vec<&String>) = list.iter().partition(|a| a.split('#').next().unwrap_or("").contains(':'));
            (four.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(","), six.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(","), false)
        }
    };
    let tls_value = if auto {
        "default"
    } else {
        match tls {
            Tls::Strict => "yes",
            Tls::Opportunistic => "opportunistic",
            Tls::Off => "no",
        }
    };
    let ignore = if auto { "no" } else { "yes" };
    let mut settings = vec![("ipv4.dns", v4.as_str()), ("ipv4.ignore-auto-dns", ignore), ("connection.dns-over-tls", tls_value)];
    if v6_ok {
        settings.push(("ipv6.dns", v6.as_str()));
        settings.push(("ipv6.ignore-auto-dns", if auto || !v6.is_empty() { ignore } else { "no" }));
    }
    modify(&c.uuid, &settings)?;
    reapply(c)
}

/// Temps de réponse d'un serveur DNS (meilleur de 3 essais) : requête A sur le port 53,
/// ou connexion TLS (port 853) pour les serveurs uniquement chiffrés.
pub fn latency(ip: &str, tls_only: bool, has_tls: bool) -> Option<Duration> {
    if demo() {
        let h = ip.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
        std::thread::sleep(Duration::from_millis(150 + h % 400));
        return Some(Duration::from_millis(6 + h % 38));
    }
    let addr: std::net::IpAddr = ip.parse().ok()?;
    let tls = || {
        (0..3)
            .filter_map(|_| {
                let t = Instant::now();
                TcpStream::connect_timeout(&SocketAddr::new(addr, 853), Duration::from_millis(1500)).ok()?;
                Some(t.elapsed())
            })
            .min()
    };
    if tls_only {
        return tls();
    }
    // DNS classique ; certains serveurs ne répondent qu'en chiffré (port 853) selon le réseau.
    (0..3).filter_map(|i| udp_query(addr, 0x4e00 + i)).min().or_else(|| if has_tls { tls() } else { None })
}

fn udp_query(addr: std::net::IpAddr, id: u16) -> Option<Duration> {
    let bind = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let sock = UdpSocket::bind(bind).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
    // En-tête : id, RD, 1 question ; puis « example.com » type A classe IN.
    let mut q = vec![(id >> 8) as u8, id as u8, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for part in ["example", "com"] {
        q.push(part.len() as u8);
        q.extend_from_slice(part.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    let t = Instant::now();
    sock.send_to(&q, SocketAddr::new(addr, 53)).ok()?;
    let mut buf = [0u8; 512];
    loop {
        let (n, _) = sock.recv_from(&mut buf).ok()?;
        if n >= 2 && buf[0] == q[0] && buf[1] == q[1] {
            return Some(t.elapsed());
        }
    }
}

// ───────────────────────────── Wi-Fi ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Ap {
    pub in_use: bool,
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub band: String,
}

pub fn wifi_enabled() -> bool {
    if demo() {
        return true;
    }
    nmcli(&["radio", "wifi"]).is_ok_and(|s| s.trim() == "enabled")
}

pub fn wifi_scan(rescan: bool) -> Vec<Ap> {
    if demo() {
        return [("Maison-5G", 92, "WPA3", true), ("Livebox-7A2C", 74, "WPA2", false), ("Café du Port", 51, "", false), ("iPhone de Léa", 38, "WPA2", false), ("DIRECT-HP-Imprimante", 22, "WPA2", false)]
            .into_iter()
            .map(|(s, sig, sec, u)| Ap { in_use: u, ssid: s.into(), signal: sig, security: sec.into(), band: if sig > 60 { "5 GHz".into() } else { "2,4 GHz".into() } })
            .collect();
    }
    let mut list: Vec<Ap> = rows(&["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY,FREQ", "device", "wifi", "list", "--rescan", if rescan { "yes" } else { "auto" }])
        .into_iter()
        .filter(|r| r.len() >= 5 && !r[1].is_empty())
        .map(|r| Ap {
            in_use: r[0] == "*",
            ssid: r[1].clone(),
            signal: r[2].parse().unwrap_or(0),
            security: r[3].replace("--", ""),
            band: if r[4].starts_with('5') || r[4].starts_with('6') { "5 GHz".into() } else { "2,4 GHz".into() },
        })
        .collect();
    // Un réseau (SSID) = une ligne : le meilleur signal.
    list.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    let mut seen = std::collections::HashSet::new();
    list.retain(|a| seen.insert(a.ssid.clone()));
    list
}

pub fn signal_icon(signal: u8) -> &'static str {
    match signal {
        75.. => "network-wireless-signal-excellent-symbolic",
        50..75 => "network-wireless-signal-good-symbolic",
        25..50 => "network-wireless-signal-ok-symbolic",
        _ => "network-wireless-signal-weak-symbolic",
    }
}

/// Mot de passe d'un réseau Wi-Fi enregistré (secret lu par NetworkManager).
pub fn wifi_password(uuid: &str) -> Option<String> {
    if demo() {
        return Some("correct-horse-battery-staple".into());
    }
    nmcli(&["-s", "-g", "802-11-wireless-security.psk", "connection", "show", uuid]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ───────────────────────────── WireGuard ─────────────────────────────

/// Résumé d'un fichier WireGuard (.conf) avant import.
#[derive(Debug, Clone, Default)]
pub struct WgConf {
    pub addresses: Vec<String>,
    pub dns: Vec<String>,
    pub endpoint: String,
    pub allowed: Vec<String>,
    pub peer_key: String,
    pub has_private: bool,
    pub peers: usize,
}

impl WgConf {
    pub fn full_tunnel(&self) -> bool {
        self.allowed.iter().any(|a| a == "0.0.0.0/0" || a == "::/0")
    }
}

pub fn parse_wg(text: &str) -> anyhow::Result<WgConf> {
    let mut c = WgConf::default();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_lowercase();
            if section == "peer" {
                c.peers += 1;
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim().to_lowercase(), v.trim());
        let list = || v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>();
        match (section.as_str(), k.as_str()) {
            ("interface", "privatekey") => c.has_private = !v.is_empty(),
            ("interface", "address") => c.addresses.extend(list()),
            ("interface", "dns") => c.dns.extend(list()),
            ("peer", "endpoint") if c.endpoint.is_empty() => c.endpoint = v.to_string(),
            ("peer", "allowedips") => c.allowed.extend(list()),
            ("peer", "publickey") if c.peer_key.is_empty() => c.peer_key = v.to_string(),
            _ => {}
        }
    }
    if !c.has_private {
        bail!("ce fichier ne contient pas de clé privée ([Interface] PrivateKey)");
    }
    if c.peers == 0 {
        bail!("ce fichier ne contient aucun serveur ([Peer])");
    }
    Ok(c)
}

/// Importe un fichier WireGuard : NetworkManager exige un nom d'interface valide
/// (15 caractères, lettres / chiffres / - _) comme nom de fichier. La connexion est renommée
/// avec le nom choisi et ne démarre pas toute seule.
pub fn import_wg(path: &std::path::Path, name: &str) -> anyhow::Result<String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(600));
        return Ok("demo".into());
    }
    let text = std::fs::read_to_string(path).context("lecture du fichier")?;
    parse_wg(&text)?;
    let mut ifname: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(12).collect::<String>().to_lowercase();
    if ifname.is_empty() {
        ifname = "vpn".into();
    }
    let ifname = format!("wg-{}", &ifname[..ifname.len().min(12)]);
    let dir = std::env::temp_dir().join(format!("noos-wg-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{ifname}.conf"));
    // Clé privée : fichier lisible par l'utilisateur seul, supprimé aussitôt après.
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&file)?;
        std::io::Write::write_all(&mut f, text.as_bytes())?;
    }
    let out = nmcli(&["connection", "import", "type", "wireguard", "file", &file.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    let uuid = out.split(['(', ')']).nth(1).map(str::to_string).context("import : réponse inattendue de NetworkManager")?;
    let _ = nmcli(&["connection", "down", &uuid]);
    modify(&uuid, &[("connection.id", name), ("connection.autoconnect", "no")])?;
    Ok(uuid)
}

#[derive(Debug, Clone, Default)]
pub struct PublicIp {
    pub ip: String,
    pub city: String,
    pub country: String,
    pub org: String,
}

/// Adresse IP publique vue d'Internet (ipinfo.io), à la demande de l'utilisateur.
pub fn public_ip() -> anyhow::Result<PublicIp> {
    if demo() {
        std::thread::sleep(Duration::from_millis(700));
        return Ok(PublicIp { ip: "185.65.135.2".into(), city: "Zurich".into(), country: "CH".into(), org: "AS9009 M247 Europe SRL".into() });
    }
    let out = Command::new("curl").args(["-sf", "--max-time", "6", "https://ipinfo.io/json"]).output().context("curl introuvable")?;
    if !out.status.success() {
        bail!("pas de réponse d'ipinfo.io");
    }
    let t = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| {
        t.lines()
            .find(|l| l.trim_start().starts_with(&format!("\"{k}\"")))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().trim_end_matches(',').trim_matches('"').to_string())
            .unwrap_or_default()
    };
    Ok(PublicIp { ip: get("ip"), city: get("city"), country: get("country"), org: get("org") })
}

/// Nom de pays (code ISO à 2 lettres) pour l'affichage de l'IP publique.
pub fn country_name(code: &str) -> String {
    let n = match code {
        "FR" => "France",
        "CH" => "Suisse",
        "BE" => "Belgique",
        "DE" => "Allemagne",
        "NL" => "Pays-Bas",
        "SE" => "Suède",
        "GB" => "Royaume-Uni",
        "US" => "États-Unis",
        "CA" => "Canada",
        "ES" => "Espagne",
        "IT" => "Italie",
        "LU" => "Luxembourg",
        "JP" => "Japon",
        _ => code,
    };
    let flag: String = code.chars().filter_map(|c| char::from_u32(0x1F1E6 + (c as u32).checked_sub('A' as u32)?)).collect();
    format!("{flag} {n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse() {
        assert_eq!(split_terse(r"a\:b:c:"), vec!["a:b", "c", ""]);
    }

    #[test]
    fn wg() {
        let c = parse_wg("[Interface]\nPrivateKey = abc=\nAddress = 10.0.0.2/32, fd00::2/128\nDNS = 10.0.0.1\n\n[Peer]\nPublicKey = xyz=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        assert_eq!(c.addresses.len(), 2);
        assert!(c.full_tunnel());
        assert_eq!(c.endpoint, "1.2.3.4:51820");
        assert!(parse_wg("[Interface]\nAddress = 1.2.3.4/32\n").is_err());
    }
}

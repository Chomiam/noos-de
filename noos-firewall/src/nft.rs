//! Table nftables `inet noos`, générée en entier à partir des réglages et des règles de ports,
//! puis appliquée d'un seul coup (`nft -f -` : transaction atomique).
//!
//! - Entrée : politique `drop` ; établi / boucle locale / interfaces de confiance / ICMP utiles /
//!   DHCP ; règles de ports ; bruit (diffusion) ignoré ; le reste des nouvelles connexions part
//!   dans la file 1 **sans** `bypass` : si le démon est arrêté, l'entrée reste fermée.
//! - Sortie : politique `accept` ; règles de ports ; nouvelles connexions vers la file 0
//!   **avec** `bypass` : démon arrêté = sorties autorisées (le réseau ne casse pas).
//! - Une connexion bloquée en sortie revient du démon avec la marque [`REJECT_MARK`] (verdict
//!   « repeat ») et est refusée tout de suite (`reject`), sans attendre l'expiration côté appli.
//!
//! nftables ne sait pas reconnaître un exécutable : les règles par programme sont appliquées par
//! le démon au moment du verdict ; les règles de ports sont de vraies règles nftables.

use std::{io::Write, process::Command};

use crate::model::{Action, Config, Direction, Mode, PortRule, Proto, Zone, validate_interface, validate_ports, validate_remote};

pub const TABLE: &str = "noos";
pub const QUEUE_OUT: u16 = 0;
pub const QUEUE_IN: u16 = 1;
/// Marque posée par le démon sur un paquet sortant refusé (« NO » + 1).
pub const REJECT_MARK: u32 = 0x4e4f_0001;

const LAN4: &str = "10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16";
const LAN6: &str = "fc00::/7, fe80::/10";

fn iface_match(dir: &str, name: &str) -> String {
    format!("{dir}ifname \"{name}\"")
}

fn ports_set(ports: &str) -> String {
    if ports.contains(',') { format!("{{ {} }}", ports.replace(',', ", ")) } else { ports.to_string() }
}

fn l4_match(proto: Proto, ports: &str) -> String {
    let p = ports_set(ports);
    match proto {
        Proto::Tcp => format!("tcp dport {p}"),
        Proto::Udp => format!("udp dport {p}"),
        Proto::Any => format!("meta l4proto {{ tcp, udp }} th dport {p}"),
    }
}

/// Lignes nftables d'une règle de port (une par famille si la zone l'exige). Vide si la règle
/// est désactivée ou invalide (elle ne doit jamais casser la table entière).
pub fn port_rule_lines(r: &PortRule, dir: Direction) -> Vec<String> {
    if !r.enabled || !r.direction.covers(dir) {
        return vec![];
    }
    let Ok(ports) = validate_ports(&r.ports) else {
        return vec![];
    };
    let (io, addr) = if dir == Direction::In { ("i", "saddr") } else { ("o", "daddr") };
    let mut head = Vec::new();
    if let Some(i) = r.interface.as_deref().filter(|s| !s.trim().is_empty()) {
        let Ok(i) = validate_interface(i) else {
            return vec![];
        };
        head.push(iface_match(io, &i));
    }
    let verdict = match (r.action, dir) {
        (Action::Allow, _) => "accept",
        (Action::Block, Direction::In) => "drop",
        (Action::Block, _) => "reject with icmpx admin-prohibited",
    };
    let l4 = l4_match(r.proto, &ports);
    let comment = format!("comment \"regle {}\"", r.id);
    let mut fams: Vec<String> = Vec::new();
    if let Some(rem) = r.remote.as_deref().filter(|s| !s.trim().is_empty()) {
        let Ok(rem) = validate_remote(rem) else {
            return vec![];
        };
        let fam = if rem.contains(':') { "ip6" } else { "ip" };
        fams.push(format!("{fam} {addr} {rem}"));
    } else if r.zone == Zone::Lan {
        fams.push(format!("ip {addr} @lan4"));
        fams.push(format!("ip6 {addr} @lan6"));
    } else {
        fams.push(String::new());
    }
    fams.into_iter()
        .map(|f| {
            let mut parts = head.clone();
            if !f.is_empty() {
                parts.push(f);
            }
            parts.push(l4.clone());
            parts.push(verdict.into());
            parts.push(comment.clone());
            parts.join(" ")
        })
        .collect()
}

/// Ruleset complet (à passer à `nft -f -`). `None` si le pare-feu est désactivé : la table est
/// alors seulement supprimée.
pub fn ruleset(cfg: &Config, ports: &[PortRule]) -> String {
    let mut s = String::new();
    s.push_str(&format!("# Généré par noos-firewalld — ne pas modifier à la main\nadd table inet {TABLE}\ndelete table inet {TABLE}\n"));
    if !cfg.enabled {
        return s;
    }
    let ordered = |dir: Direction| -> Vec<String> {
        // Blocages d'abord, puis autorisations.
        let mut v: Vec<String> = ports.iter().filter(|r| r.action == Action::Block).flat_map(|r| port_rule_lines(r, dir)).collect();
        v.extend(ports.iter().filter(|r| r.action == Action::Allow).flat_map(|r| port_rule_lines(r, dir)));
        v
    };
    let open = cfg.mode == Mode::AllowAll;
    s.push_str(&format!("table inet {TABLE} {{\n"));
    s.push_str(&format!("\tset lan4 {{\n\t\ttype ipv4_addr\n\t\tflags interval\n\t\telements = {{ {LAN4} }}\n\t}}\n"));
    s.push_str(&format!("\tset lan6 {{\n\t\ttype ipv6_addr\n\t\tflags interval\n\t\telements = {{ {LAN6} }}\n\t}}\n"));

    // ── Entrée ──
    let mut input: Vec<String> = vec![
        format!("type filter hook input priority filter; policy {};", if open && !cfg.shields_up { "accept" } else { "drop" }),
        "ct state established,related accept".into(),
        "ct state invalid drop".into(),
        "iifname \"lo\" accept".into(),
    ];
    for i in &cfg.trusted_interfaces {
        if let Ok(i) = validate_interface(i) {
            input.push(format!("{} accept comment \"interface de confiance\"", iface_match("i", &i)));
        }
    }
    input.push("meta l4proto icmp icmp type { destination-unreachable, time-exceeded, parameter-problem } accept".into());
    input.push(
        "meta l4proto ipv6-icmp icmpv6 type { destination-unreachable, packet-too-big, time-exceeded, parameter-problem, nd-router-advert, nd-neighbor-solicit, nd-neighbor-advert, mld-listener-query } accept"
            .into(),
    );
    // DHCP (renouvellement du bail par UDP) et DHCPv6.
    input.push("udp sport 67 udp dport 68 accept".into());
    input.push("ip6 saddr fe80::/10 udp sport 547 udp dport 546 accept".into());
    if !cfg.shields_up {
        if cfg.allow_ping {
            input.push("icmp type echo-request limit rate 10/second accept".into());
            input.push("icmpv6 type echo-request limit rate 10/second accept".into());
        }
        if cfg.allow_lan_discovery {
            // mDNS (requêtes et réponses), SSDP, LLMNR, WS-Discovery : réseau local seulement.
            for fam in ["ip saddr @lan4", "ip6 saddr @lan6"] {
                input.push(format!("{fam} udp dport {{ 1900, 3702, 5353, 5355 }} accept comment \"decouverte du reseau\""));
                input.push(format!("{fam} udp sport 5353 accept comment \"decouverte du reseau\""));
            }
        }
        input.extend(ordered(Direction::In));
        if !open {
            input.push("meta pkttype { broadcast, multicast } drop".into());
            input.push(format!("ct state new queue num {QUEUE_IN}"));
        }
    }
    s.push_str(&format!("\tchain input {{\n{}\t}}\n", input.iter().map(|l| format!("\t\t{l}\n")).collect::<String>()));

    // ── Sortie ──
    let mut output: Vec<String> = vec![
        "type filter hook output priority filter; policy accept;".into(),
        "ct state established,related accept".into(),
        "oifname \"lo\" accept".into(),
        format!("meta mark {REJECT_MARK:#010x} reject with icmpx admin-prohibited"),
        "meta l4proto { icmp, ipv6-icmp } accept".into(),
    ];
    output.extend(ordered(Direction::Out));
    if !open {
        output.push(format!("ct state new meta l4proto {{ tcp, udp, udplite, sctp }} queue num {QUEUE_OUT} bypass"));
    }
    s.push_str(&format!("\tchain output {{\n{}\t}}\n", output.iter().map(|l| format!("\t\t{l}\n")).collect::<String>()));
    s.push_str("}\n");
    s
}

fn nft_bin() -> &'static str {
    ["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft"].into_iter().find(|p| std::path::Path::new(p).exists()).unwrap_or("nft")
}

fn run_nft(args: &[&str], input: &str, unshare: bool) -> anyhow::Result<()> {
    let mut cmd = if unshare {
        let mut c = Command::new("unshare");
        c.args(["-rn", nft_bin()]);
        c
    } else {
        Command::new(nft_bin())
    };
    let mut child = cmd.args(args).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!("nft : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Applique la table (root).
pub fn apply(text: &str) -> anyhow::Result<()> {
    run_nft(&["-f", "-"], text, false)
}

/// Vérifie la syntaxe sans rien appliquer. Sans root : dans un espace de noms réseau à soi
/// (`unshare -rn`), où l'on a CAP_NET_ADMIN.
pub fn check(text: &str) -> anyhow::Result<()> {
    let root = unsafe { libc::geteuid() } == 0;
    run_nft(&["-c", "-f", "-"], text, !root)
}

/// La table `inet noos` est en place.
pub fn table_loaded() -> bool {
    Command::new(nft_bin()).args(["list", "table", "inet", TABLE]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Supprime la table (arrêt du service, désinstallation).
pub fn remove() -> anyhow::Result<()> {
    run_nft(&["-f", "-"], &format!("add table inet {TABLE}\ndelete table inet {TABLE}\n"), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: u64, dir: Direction, action: Action, proto: Proto, ports: &str) -> PortRule {
        PortRule { id, enabled: true, direction: dir, action, proto, ports: ports.into(), ..Default::default() }
    }

    #[test]
    fn port_lines() {
        let r = rule(3, Direction::In, Action::Allow, Proto::Tcp, "25565");
        assert_eq!(port_rule_lines(&r, Direction::In), ["tcp dport 25565 accept comment \"regle 3\""]);
        assert!(port_rule_lines(&r, Direction::Out).is_empty());
        let mut r = rule(4, Direction::In, Action::Allow, Proto::Any, "27015-27030,27036");
        r.zone = Zone::Lan;
        r.interface = Some("enp3s0".into());
        let l = port_rule_lines(&r, Direction::In);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0], "iifname \"enp3s0\" ip saddr @lan4 meta l4proto { tcp, udp } th dport { 27015-27030, 27036 } accept comment \"regle 4\"");
        assert!(l[1].contains("ip6 saddr @lan6"));
        let mut r = rule(5, Direction::Out, Action::Block, Proto::Udp, "53");
        r.remote = Some("2001:db8::/32".into());
        assert_eq!(port_rule_lines(&r, Direction::Out), ["ip6 daddr 2001:db8::/32 udp dport 53 reject with icmpx admin-prohibited comment \"regle 5\""]);
    }

    #[test]
    fn invalid_rules_are_skipped() {
        let mut r = rule(6, Direction::In, Action::Allow, Proto::Tcp, "22; flush ruleset");
        assert!(port_rule_lines(&r, Direction::In).is_empty());
        r.ports = "22".into();
        r.interface = Some("eth0\" accept".into());
        assert!(port_rule_lines(&r, Direction::In).is_empty());
        r.interface = None;
        r.enabled = false;
        assert!(port_rule_lines(&r, Direction::In).is_empty());
    }

    #[test]
    fn ruleset_shape() {
        let cfg = Config::default();
        let rules = vec![rule(1, Direction::In, Action::Allow, Proto::Tcp, "22"), rule(2, Direction::In, Action::Block, Proto::Tcp, "23")];
        let t = ruleset(&cfg, &rules);
        assert!(t.starts_with("# Généré"));
        assert!(t.contains("policy drop;"));
        // Blocage avant autorisation, file d'entrée sans bypass, sortie avec bypass.
        assert!(t.find("dport 23 drop").unwrap() < t.find("dport 22 accept").unwrap());
        assert!(t.contains("ct state new queue num 1\n"));
        assert!(t.contains("queue num 0 bypass"));
        assert!(t.contains("meta mark 0x4e4f0001 reject"));
        // Bouclier : plus de ports ni de file en entrée.
        let t = ruleset(&Config { shields_up: true, ..cfg.clone() }, &rules);
        assert!(!t.contains("dport 22 accept"));
        assert!(!t.contains("queue num 1"));
        // Tout autoriser : pas de file du tout.
        let t = ruleset(&Config { mode: Mode::AllowAll, ..cfg.clone() }, &rules);
        assert!(!t.contains("queue"));
        assert!(t.contains("policy accept;"));
        // Désactivé : seulement la suppression.
        let t = ruleset(&Config { enabled: false, ..cfg }, &rules);
        assert!(!t.contains("chain"));
    }

    /// `nft -c` dans un espace de noms réseau à soi (CAP_NET_ADMIN sans root). Ignoré si nft
    /// ou les espaces de noms utilisateur manquent.
    #[test]
    fn nft_accepts_ruleset() {
        let ok = Command::new("unshare").args(["-rn", "true"]).status().is_ok_and(|s| s.success()) && std::path::Path::new(nft_bin()).exists();
        if !ok {
            eprintln!("nft ou unshare indisponible : test ignoré");
            return;
        }
        let mut lan = rule(4, Direction::Both, Action::Allow, Proto::Any, "27015-27030,27036");
        lan.zone = Zone::Lan;
        lan.interface = Some("wg*".into());
        let mut out = rule(5, Direction::Out, Action::Block, Proto::Udp, "53");
        out.remote = Some("9.9.9.9".into());
        let rules = vec![rule(1, Direction::In, Action::Allow, Proto::Tcp, "22"), lan, out];
        for cfg in [
            Config::default(),
            Config { allow_ping: true, ..Config::default() },
            Config { shields_up: true, ..Config::default() },
            Config { mode: Mode::AllowAll, ..Config::default() },
            Config { enabled: false, ..Config::default() },
        ] {
            let t = ruleset(&cfg, &rules);
            check(&t).unwrap_or_else(|e| panic!("{e}\n{t}"));
        }
    }
}

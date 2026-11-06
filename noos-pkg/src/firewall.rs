//! Pare-feu : détection du backend (firewalld, ufw, nftables, iptables) et gestion des ports.
//!
//! | Backend   | Lecture sans droits | Ouverture de ports         | Distributions typiques  |
//! |-----------|---------------------|----------------------------|-------------------------|
//! | firewalld | oui (D-Bus)         | runtime + permanent (zone) | Fedora, RHEL, openSUSE  |
//! | ufw       | état seulement      | `ufw allow` (persistant)   | Ubuntu, Debian, Mint    |
//! | nftables  | non                 | non (règles manuelles)     | Arch (config perso)     |
//! | iptables  | non                 | non (règles manuelles)     | anciens systèmes        |

use serde::{Deserialize, Serialize};

use crate::{
    Plan, SystemInfo,
    exec::{capture, capture_codes, have, privilege_prefix},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FwBackend {
    Firewalld,
    Ufw,
    Nftables,
    Iptables,
    None,
}

impl FwBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Firewalld => "firewalld",
            Self::Ufw => "ufw",
            Self::Nftables => "nftables",
            Self::Iptables => "iptables",
            Self::None => "aucun",
        }
    }

    /// Le backend permet d'ouvrir / fermer des ports depuis NOOS.
    pub fn manageable(self) -> bool {
        matches!(self, Self::Firewalld | Self::Ufw)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Proto {
    Tcp,
    Udp,
    Both,
}

impl Proto {
    fn list(self) -> &'static [&'static str] {
        match self {
            Proto::Tcp => &["tcp"],
            Proto::Udp => &["udp"],
            Proto::Both => &["tcp", "udp"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortRule {
    /// « 8080 » ou plage « 6000-6010 ».
    pub port: String,
    /// « tcp », « udp » ou « » (les deux, ufw).
    pub proto: String,
}

impl PortRule {
    pub fn label(&self) -> String {
        if self.proto.is_empty() { format!("{} (TCP+UDP)", self.port) } else { format!("{}/{}", self.port, self.proto.to_uppercase()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FwStatus {
    pub backend: FwBackend,
    pub installed: bool,
    /// Le pare-feu filtre actuellement le trafic.
    pub active: bool,
    pub enabled_at_boot: bool,
    pub zone: Option<String>,
    pub ports: Vec<PortRule>,
    /// Services autorisés (firewalld : ssh, mdns… ; ufw : profils d'applications).
    pub services: Vec<String>,
    /// La liste des règles est connue (sinon il faut les lire en administrateur).
    pub rules_known: bool,
    pub detail: String,
}

fn systemctl(args: &[&str]) -> String {
    capture_codes(&["systemctl"].iter().chain(args).copied().collect::<Vec<_>>(), &[0, 1, 3, 4])
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn unit_active(unit: &str) -> bool {
    systemctl(&["is-active", unit]) == "active"
}

fn unit_enabled(unit: &str) -> bool {
    systemctl(&["is-enabled", unit]) == "enabled"
}

fn ufw_enabled_conf() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .map(|t| t.lines().any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes")))
        .unwrap_or(false)
}

/// Backend en service, sinon backend installé, sinon `None`.
pub fn detect() -> FwBackend {
    let firewalld = have("firewall-cmd");
    let ufw = have("ufw");
    if firewalld && unit_active("firewalld") {
        return FwBackend::Firewalld;
    }
    if ufw && ufw_enabled_conf() {
        return FwBackend::Ufw;
    }
    if firewalld {
        return FwBackend::Firewalld;
    }
    if ufw {
        return FwBackend::Ufw;
    }
    if have("nft") && unit_active("nftables") {
        return FwBackend::Nftables;
    }
    if have("iptables") && (unit_active("iptables") || unit_active("netfilter-persistent")) {
        return FwBackend::Iptables;
    }
    FwBackend::None
}

/// Sortie de `ufw status` : ports et profils autorisés en entrée.
pub fn parse_ufw_status(out: &str) -> (bool, Vec<PortRule>, Vec<String>) {
    let active = out.lines().any(|l| l.trim() == "Status: active");
    let mut ports = Vec::new();
    let mut services = Vec::new();
    let mut in_table = false;
    for line in out.lines() {
        let t = line.trim();
        if t.starts_with("--") {
            in_table = true;
            continue;
        }
        if !in_table || t.is_empty() || t.contains("(v6)") {
            continue;
        }
        // « 22/tcp   ALLOW IN   Anywhere » ou « OpenSSH   ALLOW   Anywhere »
        let Some(pos) = t.find(" ALLOW") else { continue };
        let target = t[..pos].trim();
        if let Some((port, proto)) = target.split_once('/') {
            ports.push(PortRule { port: port.replace(':', "-"), proto: proto.into() });
        } else if target.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            ports.push(PortRule { port: target.replace(':', "-"), proto: String::new() });
        } else {
            services.push(target.to_string());
        }
    }
    (active, ports, services)
}

pub fn status() -> FwStatus {
    let backend = detect();
    match backend {
        FwBackend::Firewalld => {
            let active = capture_codes(&["firewall-cmd", "--state"], &[0, 252]).map(|s| s.trim() == "running").unwrap_or(false);
            let mut st = FwStatus {
                backend,
                installed: true,
                active,
                enabled_at_boot: unit_enabled("firewalld"),
                zone: None,
                ports: vec![],
                services: vec![],
                rules_known: active,
                detail: String::new(),
            };
            if active {
                let zone = capture(&["firewall-cmd", "--get-default-zone"]).map(|s| s.trim().to_string()).unwrap_or_default();
                let z = format!("--zone={zone}");
                st.ports = capture(&["firewall-cmd", &z, "--list-ports"])
                    .unwrap_or_default()
                    .split_whitespace()
                    .filter_map(|p| p.split_once('/').map(|(port, proto)| PortRule { port: port.into(), proto: proto.into() }))
                    .collect();
                st.services = capture(&["firewall-cmd", &z, "--list-services"]).unwrap_or_default().split_whitespace().map(String::from).collect();
                st.zone = Some(zone);
            } else {
                st.detail = "Le service firewalld est arrêté".into();
            }
            st
        }
        FwBackend::Ufw => {
            let enabled = ufw_enabled_conf();
            FwStatus {
                backend,
                installed: true,
                active: enabled,
                enabled_at_boot: enabled,
                zone: None,
                ports: vec![],
                services: vec![],
                rules_known: false,
                detail: "Les règles ufw ne sont lisibles qu'en administrateur".into(),
            }
        }
        FwBackend::Nftables | FwBackend::Iptables => FwStatus {
            backend,
            installed: true,
            active: true,
            enabled_at_boot: true,
            zone: None,
            ports: vec![],
            services: vec![],
            rules_known: false,
            detail: "Règles gérées manuellement : NOOS les affiche sans les modifier".into(),
        },
        FwBackend::None => FwStatus {
            backend,
            installed: false,
            active: false,
            enabled_at_boot: false,
            zone: None,
            ports: vec![],
            services: vec![],
            rules_known: true,
            detail: "Aucun pare-feu n'est installé ni actif".into(),
        },
    }
}

/// Lecture des règles qui demande les droits administrateur (ufw, nftables, iptables).
pub fn read_rules_privileged(backend: FwBackend) -> anyhow::Result<(Vec<PortRule>, Vec<String>, String)> {
    let mut argv = privilege_prefix();
    match backend {
        FwBackend::Ufw => argv.extend(["ufw".into(), "status".into()]),
        FwBackend::Nftables => argv.extend(["nft".into(), "list".into(), "ruleset".into()]),
        FwBackend::Iptables => argv.extend(["iptables".into(), "-S".into()]),
        _ => anyhow::bail!("lecture non nécessaire pour ce backend"),
    }
    let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
    let out = capture(&refs)?;
    Ok(match backend {
        FwBackend::Ufw => {
            let (_, ports, services) = parse_ufw_status(&out);
            (ports, services, out)
        }
        _ => (vec![], vec![], out),
    })
}

pub fn validate_port(port: &str) -> anyhow::Result<String> {
    let p = port.trim().replace(':', "-");
    let ok = |s: &str| s.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n));
    let valid = match p.split_once('-') {
        Some((a, b)) => ok(a) && ok(b) && a.parse::<u32>().unwrap() < b.parse::<u32>().unwrap(),
        None => ok(&p),
    };
    if !valid {
        anyhow::bail!("port invalide : « {port} » (1–65535 ou plage 6000-6010)");
    }
    Ok(p)
}

pub fn set_active(backend: FwBackend, on: bool) -> anyhow::Result<Plan> {
    Ok(match (backend, on) {
        (FwBackend::Firewalld, true) => Plan::new().root(&["systemctl", "enable", "--now", "firewalld"]),
        (FwBackend::Firewalld, false) => Plan::new().root(&["systemctl", "disable", "--now", "firewalld"]),
        (FwBackend::Ufw, true) => Plan::new().root(&["ufw", "--force", "enable"]),
        (FwBackend::Ufw, false) => Plan::new().root(&["ufw", "disable"]),
        _ => anyhow::bail!("ce pare-feu ne peut pas être piloté depuis NOOS"),
    })
}

pub fn open_port(backend: FwBackend, port: &str, proto: Proto, zone: Option<&str>) -> anyhow::Result<Plan> {
    let port = validate_port(port)?;
    let mut plan = Plan::new();
    match backend {
        FwBackend::Firewalld => {
            let zone = format!("--zone={}", zone.unwrap_or("public"));
            for p in proto.list() {
                let spec = format!("--add-port={port}/{p}");
                // Effet immédiat + persistant au redémarrage.
                plan = plan.root(&["firewall-cmd", &zone, &spec]).root(&["firewall-cmd", "--permanent", &zone, &spec]);
            }
        }
        FwBackend::Ufw => {
            let port = port.replace('-', ":");
            for p in proto.list() {
                plan = plan.root_owned(vec!["ufw".into(), "allow".into(), format!("{port}/{p}")]);
            }
        }
        _ => anyhow::bail!("ouverture de ports non gérée pour {}", backend.label()),
    }
    Ok(plan)
}

pub fn close_port(backend: FwBackend, rule: &PortRule, zone: Option<&str>) -> anyhow::Result<Plan> {
    let protos: Vec<&str> = if rule.proto.is_empty() { vec![] } else { vec![rule.proto.as_str()] };
    let mut plan = Plan::new();
    match backend {
        FwBackend::Firewalld => {
            let zone = format!("--zone={}", zone.unwrap_or("public"));
            let spec = format!("--remove-port={}/{}", rule.port, rule.proto);
            plan = plan.root(&["firewall-cmd", &zone, &spec]).root(&["firewall-cmd", "--permanent", &zone, &spec]);
        }
        FwBackend::Ufw => {
            let port = rule.port.replace('-', ":");
            let target = if protos.is_empty() { port } else { format!("{port}/{}", rule.proto) };
            plan = plan.root_owned(vec!["ufw".into(), "delete".into(), "allow".into(), target]);
        }
        _ => anyhow::bail!("fermeture de ports non gérée pour {}", backend.label()),
    }
    Ok(plan)
}

pub fn remove_service(backend: FwBackend, service: &str, zone: Option<&str>) -> anyhow::Result<Plan> {
    match backend {
        FwBackend::Firewalld => {
            let zone = format!("--zone={}", zone.unwrap_or("public"));
            let spec = format!("--remove-service={service}");
            Ok(Plan::new().root(&["firewall-cmd", &zone, &spec]).root(&["firewall-cmd", "--permanent", &zone, &spec]))
        }
        FwBackend::Ufw => Ok(Plan::new().root(&["ufw", "delete", "allow", service])),
        _ => anyhow::bail!("non géré pour {}", backend.label()),
    }
}

/// Pare-feu recommandé pour la distribution, et plan d'installation.
pub fn install_plan(sys: &SystemInfo) -> Option<(&'static str, Plan)> {
    if sys.atomic {
        // firewalld fait partie de l'image Fedora Atomic : il suffit de l'activer.
        return Some(("firewalld", Plan::new().root(&["systemctl", "enable", "--now", "firewalld"])));
    }
    if have("dnf") || have("dnf5") {
        return Some((
            "firewalld",
            Plan::new().root(&["dnf", "install", "-y", "firewalld"]).root(&["systemctl", "enable", "--now", "firewalld"]),
        ));
    }
    if have("apt-get") {
        return Some((
            "ufw",
            Plan::new()
                .root(&["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "ufw"])
                .root(&["ufw", "--force", "enable"]),
        ));
    }
    if have("pacman") {
        return Some((
            "ufw",
            Plan::new()
                .root(&["pacman", "-Syu", "--needed", "--noconfirm", "ufw"])
                .root(&["systemctl", "enable", "--now", "ufw"])
                .root(&["ufw", "--force", "enable"]),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ufw_parse() {
        let out = "Status: active\n\nTo                         Action      From\n--                         ------      ----\n22/tcp                     ALLOW       Anywhere\n8080                       ALLOW       Anywhere\n6000:6010/udp              ALLOW IN    Anywhere\nOpenSSH                    ALLOW       Anywhere\n22/tcp (v6)                ALLOW       Anywhere (v6)\n";
        let (active, ports, services) = parse_ufw_status(out);
        assert!(active);
        assert_eq!(ports.len(), 3);
        assert_eq!(ports[2], PortRule { port: "6000-6010".into(), proto: "udp".into() });
        assert_eq!(ports[1].label(), "8080 (TCP+UDP)");
        assert_eq!(services, ["OpenSSH"]);
    }

    #[test]
    fn ports() {
        assert_eq!(validate_port("8080").unwrap(), "8080");
        assert_eq!(validate_port("6000:6010").unwrap(), "6000-6010");
        assert!(validate_port("0").is_err());
        assert!(validate_port("70000").is_err());
        assert!(validate_port("10-5").is_err());
        assert!(validate_port("abc").is_err());
    }

    #[test]
    fn plans() {
        let p = open_port(FwBackend::Firewalld, "25565", Proto::Both, Some("FedoraWorkstation")).unwrap();
        let d = p.describe();
        assert_eq!(d.len(), 4);
        assert_eq!(d[1], "# firewall-cmd --permanent --zone=FedoraWorkstation --add-port=25565/tcp");
        let p = open_port(FwBackend::Ufw, "6000-6010", Proto::Udp, None).unwrap();
        assert_eq!(p.describe(), ["# ufw allow 6000:6010/udp"]);
    }
}

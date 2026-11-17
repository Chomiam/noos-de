//! Données du pare-feu : réglages, règles par programme, règles de ports, demandes en attente,
//! journal. Sérialisées en TOML (`/etc/noos/firewall/config.toml`) et JSON (règles, protocole).

use serde::{Deserialize, Serialize};

/// Comportement face à une connexion qu'aucune règle ne couvre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Demander pour chaque nouveau programme (sorties et entrées).
    #[default]
    Interactive,
    /// Façon Windows : sorties autorisées, entrées demandées.
    AllowOutgoing,
    /// Règles appliquées sans rien demander : sorties selon `default_action`, entrées bloquées.
    Silent,
    /// Aucun filtrage (connexions seulement journalisées par les règles de blocage).
    AllowAll,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Interactive, Mode::AllowOutgoing, Mode::Silent, Mode::AllowAll];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Interactive => "Interactif : demander pour chaque programme",
            Mode::AllowOutgoing => "Autoriser les sorties, demander pour les entrées",
            Mode::Silent => "Appliquer les règles sans rien demander",
            Mode::AllowAll => "Tout autoriser (pare-feu ouvert)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Allow,
    #[default]
    Block,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Allow => "Autorisé",
            Action::Block => "Bloqué",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    #[default]
    Out,
    In,
    Both,
}

impl Direction {
    pub fn covers(self, d: Direction) -> bool {
        self == Direction::Both || self == d
    }
    pub fn label(self) -> &'static str {
        match self {
            Direction::Out => "Sortant",
            Direction::In => "Entrant",
            Direction::Both => "Entrant et sortant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Proto {
    #[default]
    Any,
    Tcp,
    Udp,
}

impl Proto {
    pub fn from_number(n: u8) -> Proto {
        match n {
            6 => Proto::Tcp,
            17 => Proto::Udp,
            _ => Proto::Any,
        }
    }
    pub fn covers(self, p: Proto) -> bool {
        self == Proto::Any || self == p
    }
    pub fn label(self) -> &'static str {
        match self {
            Proto::Any => "TCP et UDP",
            Proto::Tcp => "TCP",
            Proto::Udp => "UDP",
        }
    }
    /// Protocoles nftables couverts.
    pub fn nft(self) -> &'static [&'static str] {
        match self {
            Proto::Any => &["tcp", "udp"],
            Proto::Tcp => &["tcp"],
            Proto::Udp => &["udp"],
        }
    }
}

/// Zone d'une règle de port : d'où (entrée) ou vers où (sortie) elle s'applique.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Zone {
    /// Toutes les adresses.
    #[default]
    Any,
    /// Réseau local seulement (10/8, 172.16/12, 192.168/16, 169.254/16, fc00::/7, fe80::/10).
    Lan,
}

impl Zone {
    pub fn label(self) -> &'static str {
        match self {
            Zone::Any => "Tous les réseaux",
            Zone::Lan => "Réseau local seulement",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Table nftables installée (sinon : aucun filtrage par NOOS).
    pub enabled: bool,
    pub mode: Mode,
    /// Bloquer toutes les connexions entrantes, y compris celles des applications autorisées.
    pub shields_up: bool,
    /// Décision appliquée « pour cette fois » quand personne ne répond à temps.
    pub default_action: Action,
    /// Délai d'attente d'une réponse (secondes).
    pub timeout_secs: u32,
    /// Sorties quand aucune fenêtre d'autorisation n'est connectée (session pas encore ouverte,
    /// paquet tout juste installé) : appliqué après 3 s d'attente d'un agent. Les entrées
    /// suivent `default_action`.
    pub no_agent_action: Action,
    /// Services du système (comptes < 1000 : NetworkManager, apt, horloge…) autorisés en sortie
    /// sans demander.
    pub allow_system: bool,
    /// Répondre au ping (ICMP echo).
    pub allow_ping: bool,
    /// Découverte du réseau local (mDNS 5353, imprimantes, partages) en entrée.
    pub allow_lan_discovery: bool,
    /// Interfaces de confiance (machines virtuelles, conteneurs) : entrées acceptées. « * » final
    /// = préfixe (`virbr*`).
    pub trusted_interfaces: Vec<String>,
    /// Garder les connexions bloquées dans le journal.
    pub log_blocked: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            mode: Mode::Interactive,
            shields_up: false,
            default_action: Action::Block,
            timeout_secs: 30,
            no_agent_action: Action::Allow,
            allow_system: true,
            allow_ping: false,
            allow_lan_discovery: true,
            trusted_interfaces: vec!["virbr*".into(), "docker*".into(), "podman*".into(), "br-*".into(), "veth*".into()],
            log_blocked: true,
        }
    }
}

impl Config {
    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
    pub fn from_toml(s: &str) -> anyhow::Result<Config> {
        Ok(toml::from_str(s)?)
    }
}

/// Règle par programme. `app` est la clé du programme : chemin de l'exécutable
/// (`/usr/bin/curl`), préfixe (`/opt/noos/bin/*`), `flatpak:<id>` ou `appimage:<chemin>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppRule {
    pub id: u64,
    pub enabled: bool,
    pub app: String,
    /// Nom lisible (fichier .desktop), facultatif.
    pub label: String,
    pub direction: Direction,
    pub action: Action,
    pub proto: Proto,
    /// Adresse distante (IP ou réseau CIDR), facultative.
    pub remote: Option<String>,
    /// Port distant (sortie) ou local (entrée) : « 443 », « 6000-6010 ».
    pub port: Option<String>,
    /// Compte concerné ; `None` = tous les comptes.
    pub user: Option<u32>,
    pub created: u64,
    pub hits: u64,
    pub last_hit: u64,
}

/// Règle de port, traduite directement en règle nftables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PortRule {
    pub id: u64,
    pub enabled: bool,
    pub direction: Direction,
    pub action: Action,
    pub proto: Proto,
    /// « 25565 », « 6000-6010 », « 80,443 ».
    pub ports: String,
    pub zone: Zone,
    /// Interface (`enp3s0`, `wg*`), facultative.
    pub interface: Option<String>,
    /// Adresse distante (IP ou CIDR), facultative.
    pub remote: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Rules {
    pub next_id: u64,
    pub apps: Vec<AppRule>,
    pub ports: Vec<PortRule>,
}

impl Rules {
    pub fn alloc_id(&mut self) -> u64 {
        self.next_id = self.next_id.max(self.apps.iter().map(|r| r.id).chain(self.ports.iter().map(|r| r.id)).max().unwrap_or(0)) + 1;
        self.next_id
    }

    /// Règles livrées : programmes de NOOS et méthodes d'apt autorisés en sortie.
    pub fn seed() -> Rules {
        let mut r = Rules::default();
        for (app, label) in [("/opt/noos/bin/*", "Applications NOOS"), ("/usr/lib/apt/methods/*", "Téléchargements d'apt")] {
            let id = r.alloc_id();
            r.apps.push(AppRule { id, enabled: true, app: app.into(), label: label.into(), direction: Direction::Out, action: Action::Allow, ..Default::default() });
        }
        r
    }
}

/// Programme à l'origine d'une connexion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Program {
    /// Clé utilisée par les règles (voir [`AppRule::app`]).
    pub key: String,
    /// Exécutable (`/proc/<pid>/exe`), vide si inconnu.
    pub exe: String,
    pub cmdline: String,
    /// Nom court du processus (`comm`).
    pub comm: String,
    pub pid: u32,
    pub uid: u32,
    pub user: String,
    pub flatpak: Option<String>,
}

impl Program {
    pub fn unknown(uid: u32) -> Program {
        Program { key: "inconnu".into(), comm: "Programme inconnu".into(), uid, ..Default::default() }
    }
}

/// Connexion vue par le démon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Conn {
    pub direction: Direction,
    pub proto: Proto,
    /// Numéro du protocole IP (6, 17, 1…).
    pub ip_proto: u8,
    /// Adresse et port distants.
    pub remote: String,
    pub remote_port: u16,
    pub local: String,
    pub local_port: u16,
    /// Interface (entrée ou sortie).
    pub interface: String,
}

impl Conn {
    pub fn port(&self) -> u16 {
        if self.direction == Direction::In { self.local_port } else { self.remote_port }
    }
}

/// Demande d'autorisation envoyée à la session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Prompt {
    pub id: u64,
    pub program: Program,
    pub conn: Conn,
    /// Paquets en attente regroupés dans cette demande (réémissions).
    pub packets: u32,
    /// Secondes restantes avant la décision par défaut.
    pub deadline_secs: u32,
    pub default_action: Action,
    /// Autoriser demande les droits administrateur (connexion entrante, service du système).
    pub needs_admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Duration {
    #[default]
    Once,
    Always,
}

/// Portée de la règle créée par une réponse « toujours ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// Le programme, toutes destinations.
    #[default]
    Program,
    /// Le programme, ce port seulement.
    ProgramPort,
    /// Le programme, cette adresse et ce port.
    ProgramDestination,
    /// Ce port pour tous les programmes (règle nftables).
    Port,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Decision {
    pub action: Action,
    pub duration: Duration,
    pub scope: Scope,
    /// Règle valable pour tous les comptes (sinon : celui du programme).
    pub all_users: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LogEntry {
    pub time: u64,
    pub program: Program,
    pub conn: Conn,
    pub action: Action,
    /// Raison : « règle 12 », « délai dépassé », « aucun programme à l'écoute »…
    pub reason: String,
    /// Répétitions regroupées.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Status {
    pub version: String,
    pub config: Config,
    /// Table `inet noos` en place.
    pub table_loaded: bool,
    pub nft_error: Option<String>,
    pub pending: usize,
    pub agents: usize,
    pub app_rules: usize,
    pub port_rules: usize,
    pub blocked_total: u64,
    pub allowed_total: u64,
    /// Autres pare-feu actifs (ufw, firewalld) : ils filtrent en plus de NOOS.
    pub conflicts: Vec<String>,
    /// Le client est administrateur (groupe sudo) : modifications possibles (avec polkit).
    pub is_admin: bool,
    pub uid: u32,
}

/// Validation d'un port ou d'une liste « 80,443,6000-6010 ».
pub fn validate_ports(s: &str) -> anyhow::Result<String> {
    let s = s.trim().replace(':', "-").replace(' ', "");
    if s.is_empty() {
        anyhow::bail!("aucun port indiqué");
    }
    let ok = |p: &str| p.parse::<u32>().ok().filter(|n| (1..=65535).contains(n));
    for part in s.split(',') {
        let valid = match part.split_once('-') {
            Some((a, b)) => matches!((ok(a), ok(b)), (Some(a), Some(b)) if a < b),
            None => ok(part).is_some(),
        };
        if !valid {
            anyhow::bail!("port invalide : « {part} » (1–65535, plage 6000-6010 ou liste 80,443)");
        }
    }
    Ok(s)
}

/// Le port `p` fait partie de la liste `spec` (« 80,443,6000-6010 »).
pub fn port_matches(spec: &str, p: u16) -> bool {
    spec.split(',').any(|part| match part.trim().split_once('-') {
        Some((a, b)) => matches!((a.parse::<u16>(), b.parse::<u16>()), (Ok(a), Ok(b)) if (a..=b).contains(&p)),
        None => part.trim().parse::<u16>() == Ok(p),
    })
}

/// Adresse IP ou réseau CIDR valide (`192.168.1.10`, `10.0.0.0/8`, `2001:db8::/32`).
pub fn validate_remote(s: &str) -> anyhow::Result<String> {
    let s = s.trim();
    let (addr, len) = match s.split_once('/') {
        Some((a, l)) => (a, Some(l)),
        None => (s, None),
    };
    let ip: std::net::IpAddr = addr.parse().map_err(|_| anyhow::anyhow!("adresse invalide : « {s} »"))?;
    if let Some(l) = len {
        let max = if ip.is_ipv4() { 32 } else { 128 };
        if !l.parse::<u8>().is_ok_and(|n| n <= max) {
            anyhow::bail!("longueur de préfixe invalide : « {s} »");
        }
    }
    Ok(s.to_string())
}

/// `ip` appartient à `net` (IP seule ou CIDR).
pub fn remote_matches(net: &str, ip: &str) -> bool {
    let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
        return false;
    };
    let ip = match ip {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    };
    let (addr, len) = match net.split_once('/') {
        Some((a, l)) => (a, l.parse::<u32>().ok()),
        None => (net, None),
    };
    let Ok(base) = addr.parse::<std::net::IpAddr>() else {
        return false;
    };
    match (base, ip) {
        (std::net::IpAddr::V4(b), std::net::IpAddr::V4(i)) => {
            let l = len.unwrap_or(32).min(32);
            let mask = if l == 0 { 0 } else { u32::MAX << (32 - l) };
            u32::from(b) & mask == u32::from(i) & mask
        }
        (std::net::IpAddr::V6(b), std::net::IpAddr::V6(i)) => {
            let l = len.unwrap_or(128).min(128);
            let mask = if l == 0 { 0 } else { u128::MAX << (128 - l) };
            u128::from(b) & mask == u128::from(i) & mask
        }
        _ => false,
    }
}

/// Nom d'interface valide pour nftables (pas de guillemets ni d'espaces).
pub fn validate_interface(s: &str) -> anyhow::Result<String> {
    let s = s.trim();
    let body = s.strip_suffix('*').unwrap_or(s);
    if body.is_empty() || s.len() > 15 || !body.chars().all(|c| c.is_ascii_alphanumeric() || "-_.@".contains(c)) {
        anyhow::bail!("nom d'interface invalide : « {s} »");
    }
    Ok(s.to_string())
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        assert_eq!(validate_ports("8080").unwrap(), "8080");
        assert_eq!(validate_ports("6000:6010").unwrap(), "6000-6010");
        assert_eq!(validate_ports("80, 443").unwrap(), "80,443");
        assert!(validate_ports("0").is_err());
        assert!(validate_ports("70000").is_err());
        assert!(validate_ports("10-5").is_err());
        assert!(validate_ports("22;reboot").is_err());
        assert!(port_matches("80,443,6000-6010", 6005));
        assert!(!port_matches("80,443", 8080));
    }

    #[test]
    fn remotes() {
        assert!(validate_remote("10.0.0.0/8").is_ok());
        assert!(validate_remote("2001:db8::1").is_ok());
        assert!(validate_remote("10.0.0.0/33").is_err());
        assert!(validate_remote("example.com").is_err());
        assert!(remote_matches("10.0.0.0/8", "10.2.3.4"));
        assert!(remote_matches("192.168.1.5", "::ffff:192.168.1.5"));
        assert!(!remote_matches("192.168.1.0/24", "192.168.2.1"));
        assert!(remote_matches("fe80::/10", "fe80::1"));
    }

    #[test]
    fn interfaces() {
        assert!(validate_interface("virbr*").is_ok());
        assert!(validate_interface("enp3s0").is_ok());
        assert!(validate_interface("eth0\"; flush").is_err());
        assert!(validate_interface("*").is_err());
    }

    #[test]
    fn config_roundtrip() {
        let c = Config::default();
        assert_eq!(Config::from_toml(&c.to_toml().unwrap()).unwrap(), c);
        // Fichier partiel : valeurs par défaut pour le reste.
        let c = Config::from_toml("mode = \"silent\"\n").unwrap();
        assert_eq!(c.mode, Mode::Silent);
        assert_eq!(c.timeout_secs, 30);
    }
}

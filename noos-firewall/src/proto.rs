//! Protocole démon ↔ session : socket Unix `/run/noos-firewall/control.sock`, une requête JSON
//! par ligne, une réponse JSON par ligne. Le démon identifie l'appelant par `SO_PEERCRED`
//! (UID, PID fournis par le noyau, infalsifiables) et demande l'accord de polkit
//! (`org.noos.firewall.manage`) pour toute modification qui touche d'autres comptes que le sien.
//!
//! Une connexion qui envoie `Agent` devient un flux d'[`Event`] : c'est ainsi que la fenêtre
//! d'autorisation de la session reçoit les demandes.

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration as StdDuration,
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::model::{AppRule, Config, Decision, LogEntry, PortRule, Prompt, Rules, Status};

pub const SOCKET: &str = "/run/noos-firewall/control.sock";
pub const POLKIT_ACTION: &str = "org.noos.firewall.manage";

pub fn socket_path() -> PathBuf {
    std::env::var_os("NOOS_FIREWALL_SOCKET").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(SOCKET))
}

/// UID du processus courant.
pub fn my_uid() -> u32 {
    if demo() { 1000 } else { unsafe { libc::getuid() } }
}

/// Mode démonstration (aperçu, captures) : données fictives, aucun démon.
pub fn demo() -> bool {
    std::env::var_os("NOOS_FIREWALL_DEMO").is_some_and(|v| !v.is_empty() && v != "0")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "args", rename_all = "kebab-case")]
pub enum Request {
    Status,
    Rules,
    SaveAppRule(AppRule),
    DeleteAppRule {
        id: u64,
    },
    SavePortRule(PortRule),
    DeletePortRule {
        id: u64,
    },
    SetConfig(Config),
    Pending,
    Decide {
        id: u64,
        decision: Decision,
    },
    Log {
        limit: usize,
    },
    ClearLog,
    /// Transforme la connexion en flux d'événements (fenêtre d'autorisation).
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl Reply {
    pub fn ok(data: impl Serialize) -> Reply {
        Reply { ok: true, error: None, data: serde_json::to_value(data).unwrap_or_default() }
    }
    pub fn err(e: impl std::fmt::Display) -> Reply {
        Reply { ok: false, error: Some(e.to_string()), data: serde_json::Value::Null }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "args", rename_all = "kebab-case")]
pub enum Event {
    Prompt(Prompt),
    /// Demande close (réponse donnée ailleurs, délai dépassé).
    Done {
        id: u64,
    },
}

/// Résultat d'une décision : règle créée (révocable) le cas échéant.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecideResult {
    pub app_rule: Option<u64>,
    pub port_rule: Option<u64>,
}

fn connect() -> anyhow::Result<UnixStream> {
    let path = socket_path();
    UnixStream::connect(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            anyhow::anyhow!("le service noos-firewalld ne fonctionne pas")
        }
        _ => anyhow::anyhow!("{}: {e}", path.display()),
    })
}

/// Requête simple (bloquante : à appeler hors du fil de l'interface). Les modifications peuvent
/// attendre la fenêtre d'authentification de polkit : pas de délai de lecture.
pub fn call(req: &Request) -> anyhow::Result<serde_json::Value> {
    if demo() {
        return crate::demo::handle(req);
    }
    let mut s = connect()?;
    s.set_write_timeout(Some(StdDuration::from_secs(5)))?;
    let mut line = serde_json::to_string(req)?;
    line.push('\n');
    s.write_all(line.as_bytes())?;
    let mut r = BufReader::new(s);
    let mut resp = String::new();
    r.read_line(&mut resp)?;
    if resp.is_empty() {
        anyhow::bail!("le service a fermé la connexion");
    }
    let reply: Reply = serde_json::from_str(&resp)?;
    if !reply.ok {
        anyhow::bail!("{}", reply.error.unwrap_or_else(|| "erreur inconnue".into()));
    }
    Ok(reply.data)
}

pub fn call_as<T: DeserializeOwned>(req: &Request) -> anyhow::Result<T> {
    Ok(serde_json::from_value(call(req)?)?)
}

pub fn status() -> anyhow::Result<Status> {
    call_as(&Request::Status)
}
pub fn rules() -> anyhow::Result<Rules> {
    call_as(&Request::Rules)
}
pub fn pending() -> anyhow::Result<Vec<Prompt>> {
    call_as(&Request::Pending)
}
pub fn log(limit: usize) -> anyhow::Result<Vec<LogEntry>> {
    call_as(&Request::Log { limit })
}
pub fn decide(id: u64, decision: Decision) -> anyhow::Result<DecideResult> {
    call_as(&Request::Decide { id, decision })
}

/// Flux d'événements pour la fenêtre d'autorisation. `on_event` est appelé pour chaque
/// événement ; la fonction rend la main quand la connexion se ferme.
pub fn agent(mut on_event: impl FnMut(Event)) -> anyhow::Result<()> {
    if demo() {
        for p in crate::demo::prompts() {
            on_event(Event::Prompt(p));
        }
        loop {
            std::thread::sleep(StdDuration::from_secs(3600));
        }
    }
    let mut s = connect()?;
    s.write_all(format!("{}\n", serde_json::to_string(&Request::Agent)?).as_bytes())?;
    let r = BufReader::new(s);
    for line in r.lines() {
        let line = line?;
        if let Ok(ev) = serde_json::from_str::<Event>(&line) {
            on_event(ev);
        }
    }
    Ok(())
}

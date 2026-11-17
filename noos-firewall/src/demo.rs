//! Données fictives (`NOOS_FIREWALL_DEMO=1`, ou `in` / `out` pour la fenêtre d'autorisation) :
//! la page Pare-feu et la fenêtre fonctionnent dans l'aperçu sans démon ni root.

use std::sync::Mutex;

use crate::{
    model::*,
    proto::{DecideResult, Request},
};

struct Demo {
    config: Config,
    rules: Rules,
    log: Vec<LogEntry>,
    pending: Vec<Prompt>,
}

static STATE: Mutex<Option<Demo>> = Mutex::new(None);

fn prog(key: &str, exe: &str, comm: &str, pid: u32, uid: u32, flatpak: Option<&str>) -> Program {
    Program {
        key: key.into(),
        exe: exe.into(),
        cmdline: exe.into(),
        comm: comm.into(),
        pid,
        uid,
        user: if uid == 0 { "root".into() } else { "alex".into() },
        flatpak: flatpak.map(String::from),
    }
}

fn conn(dir: Direction, proto: Proto, remote: &str, port: u16) -> Conn {
    let (rp, lp) = if dir == Direction::In { (51000, port) } else { (port, 51000) };
    Conn {
        direction: dir,
        proto,
        ip_proto: if proto == Proto::Udp { 17 } else { 6 },
        remote: remote.into(),
        remote_port: rp,
        local: "192.168.1.24".into(),
        local_port: lp,
        interface: "enp5s0".into(),
    }
}

pub fn prompts() -> Vec<Prompt> {
    let out = Prompt {
        id: 101,
        program: prog("/usr/games/steam", "/home/alex/.local/share/Steam/ubuntu12_32/steam", "steam", 4242, 1000, None),
        conn: conn(Direction::Out, Proto::Tcp, "155.133.226.71", 27019),
        packets: 2,
        deadline_secs: 30,
        default_action: Action::Block,
        needs_admin: false,
    };
    let inc = Prompt {
        id: 102,
        program: prog("/usr/lib/jvm/java-21-openjdk-amd64/bin/java", "/usr/lib/jvm/java-21-openjdk-amd64/bin/java", "java", 5150, 1000, None),
        conn: conn(Direction::In, Proto::Tcp, "192.168.1.37", 25565),
        packets: 1,
        deadline_secs: 30,
        default_action: Action::Block,
        needs_admin: true,
    };
    match std::env::var("NOOS_FIREWALL_DEMO").as_deref() {
        Ok("in") => vec![inc, out],
        _ => vec![out, inc],
    }
}

fn init() -> Demo {
    let mut rules = Rules::seed();
    let t = now();
    let mut add = |app: &str, label: &str, dir: Direction, action: Action, port: Option<&str>, user: Option<u32>, hits: u64| {
        let id = rules.alloc_id();
        rules.apps.push(AppRule {
            id,
            enabled: true,
            app: app.into(),
            label: label.into(),
            direction: dir,
            action,
            port: port.map(String::from),
            user,
            created: t - 86_400 * id,
            hits,
            last_hit: t - 60 * id,
            ..Default::default()
        });
    };
    add("flatpak:org.mozilla.firefox", "Firefox", Direction::Out, Action::Allow, None, Some(1000), 18_342);
    add("flatpak:com.discordapp.Discord", "Discord", Direction::Out, Action::Allow, None, Some(1000), 2_104);
    add("/usr/bin/curl", "curl", Direction::Out, Action::Allow, Some("443"), Some(1000), 57);
    add("/usr/lib/jvm/java-21-openjdk-amd64/bin/java", "Serveur Minecraft", Direction::In, Action::Allow, Some("25565"), None, 311);
    add("/usr/bin/telemetry-agent", "telemetry-agent", Direction::Out, Action::Block, None, None, 940);
    let mut port = |dir: Direction, action: Action, proto: Proto, ports: &str, zone: Zone, iface: Option<&str>, note: &str| {
        let id = rules.alloc_id();
        rules.ports.push(PortRule {
            id,
            enabled: true,
            direction: dir,
            action,
            proto,
            ports: ports.into(),
            zone,
            interface: iface.map(String::from),
            remote: None,
            note: note.into(),
        });
    };
    port(Direction::In, Action::Allow, Proto::Tcp, "22", Zone::Lan, None, "SSH");
    port(Direction::In, Action::Allow, Proto::Any, "27015-27030,27036", Zone::Lan, None, "Steam (jeu en réseau local)");
    port(Direction::In, Action::Allow, Proto::Udp, "51820", Zone::Any, Some("enp5s0"), "WireGuard");
    port(Direction::Out, Action::Block, Proto::Tcp, "25", Zone::Any, None, "Courrier sortant (SMTP)");
    let log = vec![
        LogEntry {
            time: t - 40,
            program: prog("/usr/bin/telemetry-agent", "/usr/bin/telemetry-agent", "telemetry-agent", 911, 0, None),
            conn: conn(Direction::Out, Proto::Tcp, "34.120.5.18", 443),
            action: Action::Block,
            reason: "règle 7".into(),
            count: 12,
        },
        LogEntry {
            time: t - 300,
            program: Program { comm: "Aucun programme".into(), ..Default::default() },
            conn: conn(Direction::In, Proto::Tcp, "203.0.113.77", 3389),
            action: Action::Block,
            reason: "aucun programme à l'écoute".into(),
            count: 3,
        },
        LogEntry {
            time: t - 1800,
            program: prog("/usr/bin/python3.13", "/usr/bin/python3.13", "python3", 7001, 1000, None),
            conn: conn(Direction::In, Proto::Tcp, "192.168.1.50", 8000),
            action: Action::Block,
            reason: "délai dépassé".into(),
            count: 1,
        },
        LogEntry {
            time: t - 7200,
            program: prog("flatpak:com.spotify.Client", "/app/extra/share/spotify/spotify", "spotify", 6600, 1000, Some("com.spotify.Client")),
            conn: conn(Direction::Out, Proto::Udp, "35.186.224.24", 4070),
            action: Action::Block,
            reason: "bloqué pour cette fois".into(),
            count: 4,
        },
    ];
    Demo { config: Config::default(), rules, log, pending: prompts() }
}

pub fn handle(req: &Request) -> anyhow::Result<serde_json::Value> {
    let mut g = STATE.lock().unwrap();
    let d = g.get_or_insert_with(init);
    Ok(match req {
        Request::Status => v(&Status {
            version: env!("CARGO_PKG_VERSION").into(),
            config: d.config.clone(),
            table_loaded: d.config.enabled,
            nft_error: None,
            pending: d.pending.len(),
            agents: 1,
            app_rules: d.rules.apps.len(),
            port_rules: d.rules.ports.len(),
            blocked_total: 1_284,
            allowed_total: 48_210,
            conflicts: vec![],
            is_admin: true,
            uid: 1000,
        }),
        Request::Rules => v(&d.rules),
        Request::SaveAppRule(r) => {
            let mut r = r.clone();
            if r.id == 0 {
                r.id = d.rules.alloc_id();
                d.rules.apps.push(r.clone());
            } else if let Some(x) = d.rules.apps.iter_mut().find(|x| x.id == r.id) {
                *x = r.clone();
            }
            v(&r.id)
        }
        Request::DeleteAppRule { id } => {
            d.rules.apps.retain(|r| r.id != *id);
            serde_json::Value::Null
        }
        Request::SavePortRule(r) => {
            let mut r = r.clone();
            r.ports = validate_ports(&r.ports)?;
            if r.id == 0 {
                r.id = d.rules.alloc_id();
                d.rules.ports.push(r.clone());
            } else if let Some(x) = d.rules.ports.iter_mut().find(|x| x.id == r.id) {
                *x = r.clone();
            }
            v(&r.id)
        }
        Request::DeletePortRule { id } => {
            d.rules.ports.retain(|r| r.id != *id);
            serde_json::Value::Null
        }
        Request::SetConfig(c) => {
            d.config = c.clone();
            serde_json::Value::Null
        }
        Request::Pending => v(&d.pending),
        Request::Decide { id, decision } => {
            let Some(pos) = d.pending.iter().position(|p| p.id == *id) else { anyhow::bail!("demande introuvable") };
            let p = d.pending.remove(pos);
            let mut res = DecideResult::default();
            if decision.duration == Duration::Always {
                let rid = d.rules.alloc_id();
                d.rules.apps.push(AppRule {
                    id: rid,
                    enabled: true,
                    app: p.program.key.clone(),
                    label: p.program.comm.clone(),
                    direction: p.conn.direction,
                    action: decision.action,
                    user: (!decision.all_users).then_some(p.program.uid),
                    created: now(),
                    ..Default::default()
                });
                res.app_rule = Some(rid);
            }
            v(&res)
        }
        Request::Log { limit } => v(&d.log.iter().take(*limit).cloned().collect::<Vec<_>>()),
        Request::ClearLog => {
            d.log.clear();
            serde_json::Value::Null
        }
        Request::Agent => serde_json::Value::Null,
    })
}

fn v<T: serde::Serialize>(x: &T) -> serde_json::Value {
    serde_json::to_value(x).unwrap_or_default()
}

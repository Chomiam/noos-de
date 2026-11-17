//! Démon `noos-firewalld` (root, service systemd).
//!
//! - **Fil de la file** : reçoit les nouvelles connexions (NFQUEUE 0 = sorties, 1 = entrées),
//!   identifie le programme, décide tout de suite (règle, service du système, mode) ou garde le
//!   paquet en attente et publie une demande aux fenêtres d'autorisation. Lui seul donne les
//!   verdicts (le socket netlink n'est pas partagé).
//! - **Socket de contrôle** (`/run/noos-firewall/control.sock`, un fil par client) : l'appelant
//!   est identifié par `SO_PEERCRED` ; ce qui touche aux autres comptes, aux entrées ou aux
//!   ports passe par polkit (`pkcheck`, action `org.noos.firewall.manage`, mot de passe
//!   administrateur demandé par l'agent polkit de la session).

use std::{
    collections::{HashMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    net::IpAddr,
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration as StdDuration, Instant},
};

use crate::{
    matching,
    model::*,
    nft, packet, procnet,
    proto::{DecideResult, Event, POLKIT_ACTION, Reply, Request},
    store,
};

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

/// Mémoire courte des décisions « pour cette fois » et des délais dépassés : évite de redemander
/// pour chaque réémission ou chaque nouvelle connexion du même programme.
const TEMP_SECS: u64 = 60;
/// Paquets gardés au plus en attente d'une réponse (au-delà : décision par défaut).
const MAX_HELD: usize = 2048;
const LOG_MAX: usize = 1000;

struct Pending {
    prompt: Prompt,
    deadline: Instant,
    /// Réponse en cours d'authentification (polkit) : le délai est suspendu.
    authorizing: bool,
    /// Aucune fenêtre d'autorisation ne pouvait recevoir la demande.
    no_agent: bool,
}

/// Attente d'une fenêtre d'autorisation quand aucune n'est connectée.
const NO_AGENT_WAIT: u64 = 3;

struct Temp {
    key: String,
    uid: u32,
    direction: Direction,
    /// Destination précise (« once ») ou toutes (délai dépassé).
    dest: Option<(String, u16)>,
    action: Action,
    until: Instant,
}

struct Agent {
    id: u64,
    uid: u32,
    admin: bool,
    tx: mpsc::Sender<Event>,
}

pub struct State {
    dir: PathBuf,
    config: Config,
    rules: Rules,
    rules_dirty: bool,
    pending: Vec<Pending>,
    temp: Vec<Temp>,
    agents: Vec<Agent>,
    log: VecDeque<LogEntry>,
    next_id: u64,
    nft_error: Option<String>,
    blocked_total: u64,
    allowed_total: u64,
}

enum Outcome {
    Now(Action, String),
    Wait(u64),
}

enum Cmd {
    Verdict { prompt: u64, action: Action },
}

pub struct Shared {
    state: Mutex<State>,
    cmd: Mutex<mpsc::Sender<Cmd>>,
}

fn is_system_uid(uid: u32) -> bool {
    uid < 1000 || uid == 65534
}

/// Membre du groupe `sudo` (ou root) : administrateur de la machine.
pub fn is_admin(uid: u32) -> bool {
    if uid == 0 {
        return true;
    }
    let name = procnet::user_name(uid);
    let primary = std::fs::read_to_string("/etc/passwd").ok().and_then(|t| {
        t.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.len() > 3 && f[0] == name).then(|| f[3].to_string())
        })
    });
    std::fs::read_to_string("/etc/group")
        .map(|t| {
            t.lines().any(|l| {
                let f: Vec<&str> = l.split(':').collect();
                f.len() > 3 && f[0] == "sudo" && (f[3].split(',').any(|m| m == name) || primary.as_deref() == Some(f[2]))
            })
        })
        .unwrap_or(false)
}

impl State {
    fn new(dir: PathBuf) -> State {
        let config = store::load_config(&dir);
        let rules = store::load_rules(&dir);
        State {
            dir,
            config,
            rules,
            rules_dirty: false,
            pending: vec![],
            temp: vec![],
            agents: vec![],
            log: VecDeque::new(),
            next_id: 1,
            nft_error: None,
            blocked_total: 0,
            allowed_total: 0,
        }
    }

    fn apply_nft(&mut self) {
        let text = nft::ruleset(&self.config, &self.rules.ports);
        self.nft_error = nft::apply(&text).err().map(|e| format!("{e:#}"));
        if let Some(e) = &self.nft_error {
            eprintln!("noos-firewalld : table non appliquée : {e}");
        }
    }

    fn save_rules(&mut self) -> anyhow::Result<()> {
        store::save_rules(&self.dir, &self.rules)?;
        self.rules_dirty = false;
        Ok(())
    }

    fn push_log(&mut self, program: &Program, conn: &Conn, action: Action, reason: &str) {
        match action {
            Action::Allow => self.allowed_total += 1,
            Action::Block => self.blocked_total += 1,
        }
        if action == Action::Allow || !self.config.log_blocked {
            return;
        }
        let t = now();
        if let Some(last) = self.log.iter_mut().find(|e| {
            t.saturating_sub(e.time) < 60 && e.program.key == program.key && e.conn.remote == conn.remote && e.conn.port() == conn.port() && e.conn.direction == conn.direction
        }) {
            last.count += 1;
            last.time = t;
            return;
        }
        self.log.push_front(LogEntry { time: t, program: program.clone(), conn: conn.clone(), action, reason: reason.into(), count: 1 });
        self.log.truncate(LOG_MAX);
    }

    fn agents_for(&self, p: &Prompt) -> Vec<&Agent> {
        let own: Vec<&Agent> = self.agents.iter().filter(|a| a.uid == p.program.uid && !is_system_uid(p.program.uid)).collect();
        if !own.is_empty() {
            return own;
        }
        self.agents.iter().filter(|a| a.admin).collect()
    }

    fn broadcast_prompt(&self, p: &Pending) {
        let mut pr = p.prompt.clone();
        pr.deadline_secs = p.deadline.saturating_duration_since(Instant::now()).as_secs() as u32;
        for a in self.agents_for(&p.prompt) {
            let _ = a.tx.send(Event::Prompt(pr.clone()));
        }
    }

    fn broadcast_done(&self, id: u64) {
        for a in &self.agents {
            let _ = a.tx.send(Event::Done { id });
        }
    }

    fn decide(&mut self, prog: &Program, conn: &Conn) -> Outcome {
        if let Some(r) = matching::find(&self.rules.apps, prog, conn) {
            let (id, action) = (r.id, r.action);
            if let Some(r) = self.rules.apps.iter_mut().find(|r| r.id == id) {
                r.hits += 1;
                r.last_hit = now();
            }
            self.rules_dirty = true;
            return Outcome::Now(action, format!("règle {id}"));
        }
        let n = Instant::now();
        self.temp.retain(|t| t.until > n);
        if let Some(t) = self
            .temp
            .iter()
            .find(|t| t.key == prog.key && t.uid == prog.uid && t.direction == conn.direction && t.dest.as_ref().is_none_or(|(r, p)| *r == conn.remote && *p == conn.port()))
        {
            return Outcome::Now(t.action, "décision temporaire".into());
        }
        let cfg = &self.config;
        match conn.direction {
            Direction::In if prog.key == "aucun" => {
                return Outcome::Now(Action::Block, "aucun programme à l'écoute".into());
            }
            Direction::In => match cfg.mode {
                Mode::AllowAll => return Outcome::Now(Action::Allow, "tout est autorisé".into()),
                Mode::Silent => return Outcome::Now(Action::Block, "mode silencieux".into()),
                _ => {}
            },
            _ => {
                if is_system_uid(prog.uid) && cfg.allow_system {
                    return Outcome::Now(Action::Allow, "service du système".into());
                }
                match cfg.mode {
                    Mode::AllowAll | Mode::AllowOutgoing => {
                        return Outcome::Now(Action::Allow, "sorties autorisées".into());
                    }
                    Mode::Silent => {
                        return Outcome::Now(cfg.default_action, "mode silencieux".into());
                    }
                    Mode::Interactive => {}
                }
            }
        }
        // Demande déjà ouverte pour ce programme : le paquet s'y ajoute.
        if let Some(p) = self.pending.iter_mut().find(|p| p.prompt.program.key == prog.key && p.prompt.program.uid == prog.uid && p.prompt.conn.direction == conn.direction) {
            p.prompt.packets += 1;
            return Outcome::Wait(p.prompt.id);
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut pending = Pending {
            prompt: Prompt {
                id,
                program: prog.clone(),
                conn: conn.clone(),
                packets: 1,
                deadline_secs: cfg.timeout_secs,
                default_action: cfg.default_action,
                needs_admin: conn.direction == Direction::In || is_system_uid(prog.uid),
            },
            deadline: n + StdDuration::from_secs(cfg.timeout_secs.clamp(5, 600) as u64),
            authorizing: false,
            no_agent: false,
        };
        if self.agents_for(&pending.prompt).is_empty() {
            pending.no_agent = true;
            pending.deadline = n + StdDuration::from_secs(NO_AGENT_WAIT);
            if conn.direction == Direction::Out {
                pending.prompt.default_action = self.config.no_agent_action;
            }
        }
        self.broadcast_prompt(&pending);
        self.pending.push(pending);
        Outcome::Wait(id)
    }

    /// Demandes arrivées à échéance : décision par défaut, retenue un moment.
    fn expire(&mut self) -> Vec<(u64, Action)> {
        let n = Instant::now();
        let (gone, keep): (Vec<Pending>, Vec<Pending>) = std::mem::take(&mut self.pending).into_iter().partition(|p| !p.authorizing && p.deadline <= n);
        self.pending = keep;
        gone.into_iter()
            .map(|p| {
                let action = p.prompt.default_action;
                self.temp.push(Temp {
                    key: p.prompt.program.key.clone(),
                    uid: p.prompt.program.uid,
                    direction: p.prompt.conn.direction,
                    dest: None,
                    action,
                    until: n + StdDuration::from_secs(TEMP_SECS),
                });
                self.push_log(&p.prompt.program, &p.prompt.conn, action, if p.no_agent { "aucune fenêtre d'autorisation" } else { "délai dépassé" });
                self.broadcast_done(p.prompt.id);
                (p.prompt.id, action)
            })
            .collect()
    }

    fn status(&self, uid: u32) -> Status {
        Status {
            version: env!("CARGO_PKG_VERSION").into(),
            config: self.config.clone(),
            table_loaded: self.config.enabled && self.nft_error.is_none(),
            nft_error: self.nft_error.clone(),
            pending: self.pending.len(),
            agents: self.agents.len(),
            app_rules: self.rules.apps.len(),
            port_rules: self.rules.ports.len(),
            blocked_total: self.blocked_total,
            allowed_total: self.allowed_total,
            conflicts: conflicts(),
            is_admin: is_admin(uid),
            uid,
        }
    }
}

/// Autres pare-feu actifs : ils filtrent en plus de NOOS (une connexion doit passer les deux).
pub fn conflicts() -> Vec<String> {
    let mut v = Vec::new();
    let ufw_on = std::fs::read_to_string("/etc/ufw/ufw.conf").is_ok_and(|t| t.lines().any(|l| l.trim().eq_ignore_ascii_case("ENABLED=yes")));
    if ufw_on && Path::new("/usr/sbin/ufw").exists() {
        v.push("ufw".into());
    }
    let active = |u: &str| std::process::Command::new("systemctl").args(["is-active", "--quiet", u]).status().is_ok_and(|s| s.success());
    if active("firewalld.service") {
        v.push("firewalld".into());
    }
    v
}

// ───────────────────────── File NFQUEUE ─────────────────────────

fn ifname(index: u32) -> String {
    if index == 0 {
        return String::new();
    }
    let mut buf = [0 as libc::c_char; libc::IF_NAMESIZE];
    let p = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if p.is_null() {
        return index.to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().to_string()
}

/// Programme et connexion d'un paquet. `None` : paquet sans intérêt (accepté).
fn identify(proc_root: &Path, cache: &mut procnet::PidCache, msg: &nfq::Message) -> Option<(Program, Conn)> {
    let pkt = packet::parse(msg.get_payload())?;
    let incoming = msg.get_queue_num() == nft::QUEUE_IN;
    let proto = Proto::from_number(pkt.proto);
    let (remote, rport, local, lport) = if incoming { (pkt.src, pkt.sport, pkt.dst, pkt.dport) } else { (pkt.dst, pkt.dport, pkt.src, pkt.sport) };
    let conn = Conn {
        direction: if incoming { Direction::In } else { Direction::Out },
        proto,
        ip_proto: pkt.proto,
        remote: canon(remote).to_string(),
        remote_port: rport,
        local: canon(local).to_string(),
        local_port: lport,
        interface: ifname(if incoming { msg.get_indev() } else { msg.get_outdev() }),
    };
    let table = procnet::read_tables(proc_root, pkt.proto);
    let entry = if incoming { procnet::find_incoming(&table, pkt.proto == 6, pkt.dst, pkt.dport) } else { procnet::find_outgoing(&table, pkt.src, pkt.sport, pkt.dst, pkt.dport) };
    let uid = msg.get_uid().or(entry.map(|e| e.uid)).unwrap_or(0);
    let program = match entry {
        Some(e) => match cache.pid_of(proc_root, e.inode) {
            Some(pid) => procnet::program_of(proc_root, pid, e.uid),
            None => Program { uid: e.uid, user: procnet::user_name(e.uid), ..Program::unknown(e.uid) },
        },
        None if incoming => Program { key: "aucun".into(), comm: "Aucun programme".into(), ..Default::default() },
        None => Program { user: procnet::user_name(uid), ..Program::unknown(uid) },
    };
    Some((program, conn))
}

fn canon(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn give_verdict(q: &mut nfq::Queue, mut msg: nfq::Message, action: Action) {
    match action {
        Action::Allow => msg.set_verdict(nfq::Verdict::Accept),
        // Sortie refusée : repasse dans la chaîne avec la marque → `reject` immédiat.
        Action::Block if msg.get_queue_num() == nft::QUEUE_OUT => {
            msg.set_nfmark(nft::REJECT_MARK);
            msg.set_verdict(nfq::Verdict::Repeat);
        }
        Action::Block => msg.set_verdict(nfq::Verdict::Drop),
    }
    if let Err(e) = q.verdict(msg) {
        eprintln!("noos-firewalld : verdict : {e}");
    }
}

fn open_queue() -> anyhow::Result<nfq::Queue> {
    let mut q = nfq::Queue::open()?;
    for (num, fail_open) in [(nft::QUEUE_OUT, true), (nft::QUEUE_IN, false)] {
        q.bind(num)?;
        q.set_copy_range(num, 128)?;
        q.set_queue_max_len(num, 4096)?;
        q.set_fail_open(num, fail_open)?;
        // UID / GID du socket : pas toujours fournis (selon le noyau) → repli sur /proc/net.
        let _ = q.set_recv_uid_gid(num, true);
    }
    q.set_nonblocking(true);
    Ok(q)
}

fn queue_loop(shared: Arc<Shared>, mut q: nfq::Queue, rx: mpsc::Receiver<Cmd>) {
    let proc_root = PathBuf::from("/proc");
    let mut cache = procnet::PidCache::default();
    let mut held: HashMap<u64, Vec<nfq::Message>> = HashMap::new();
    let mut last_save = Instant::now();
    while !STOP.load(Ordering::SeqCst) {
        let mut pfd = libc::pollfd { fd: q.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        unsafe { libc::poll(&mut pfd, 1, 200) };
        loop {
            let msg = match q.recv() {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    eprintln!("noos-firewalld : lecture de la file : {e}");
                    break;
                }
            };
            // Paquet déjà refusé qui revient (table modifiée entre-temps) : jamais de boucle.
            if msg.get_nfmark() == nft::REJECT_MARK {
                let mut m = msg;
                m.set_verdict(nfq::Verdict::Drop);
                let _ = q.verdict(m);
                continue;
            }
            let Some((prog, conn)) = identify(&proc_root, &mut cache, &msg) else {
                give_verdict(&mut q, msg, Action::Allow);
                continue;
            };
            let outcome = shared.state.lock().unwrap().decide(&prog, &conn);
            match outcome {
                Outcome::Now(action, reason) => {
                    shared.state.lock().unwrap().push_log(&prog, &conn, action, &reason);
                    give_verdict(&mut q, msg, action);
                }
                Outcome::Wait(id) => {
                    if held.values().map(Vec::len).sum::<usize>() >= MAX_HELD {
                        let a = shared.state.lock().unwrap().config.default_action;
                        give_verdict(&mut q, msg, a);
                    } else {
                        held.entry(id).or_default().push(msg);
                    }
                }
            }
        }
        while let Ok(Cmd::Verdict { prompt, action }) = rx.try_recv() {
            for m in held.remove(&prompt).unwrap_or_default() {
                give_verdict(&mut q, m, action);
            }
        }
        let expired = shared.state.lock().unwrap().expire();
        for (id, action) in expired {
            for m in held.remove(&id).unwrap_or_default() {
                give_verdict(&mut q, m, action);
            }
        }
        if last_save.elapsed() > StdDuration::from_secs(300) {
            last_save = Instant::now();
            let mut st = shared.state.lock().unwrap();
            if st.rules_dirty {
                let _ = st.save_rules();
            }
        }
    }
    // Arrêt : les paquets en attente reçoivent la décision par défaut.
    let a = shared.state.lock().unwrap().config.default_action;
    for (_, msgs) in held.drain() {
        for m in msgs {
            give_verdict(&mut q, m, a);
        }
    }
}

// ───────────────────────── Socket de contrôle ─────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct Peer {
    pub pid: u32,
    pub uid: u32,
}

fn peer_cred(s: &UnixStream) -> std::io::Result<Peer> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Peer { pid: cred.pid as u32, uid: cred.uid })
}

/// Accord de polkit pour le processus appelant (fenêtre de mot de passe dans sa session).
fn authorize(peer: Peer) -> anyhow::Result<()> {
    if peer.uid == 0 {
        return Ok(());
    }
    let start = procnet::start_time(peer.pid).ok_or_else(|| anyhow::anyhow!("processus appelant introuvable"))?;
    let st = std::process::Command::new("pkcheck")
        .args(["--action-id", POLKIT_ACTION, "--process", &format!("{},{start},{}", peer.pid, peer.uid), "--allow-user-interaction"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if !st.success() {
        anyhow::bail!("autorisation refusée (droits administrateur nécessaires)");
    }
    Ok(())
}

fn validate_app_rule(r: &mut AppRule) -> anyhow::Result<()> {
    r.app = r.app.trim().to_string();
    if r.app.is_empty() || r.app == "*" {
        anyhow::bail!("programme manquant");
    }
    r.port = match r.port.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => Some(validate_ports(p)?),
        None => None,
    };
    r.remote = match r.remote.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => Some(validate_remote(p)?),
        None => None,
    };
    Ok(())
}

fn validate_port_rule(r: &mut PortRule) -> anyhow::Result<()> {
    r.ports = validate_ports(&r.ports)?;
    r.interface = match r.interface.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(i) => Some(validate_interface(i)?),
        None => None,
    };
    r.remote = match r.remote.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => Some(validate_remote(p)?),
        None => None,
    };
    r.note = r.note.chars().filter(|c| !c.is_control()).take(120).collect();
    Ok(())
}

/// Règle « à soi » : sortante, limitée au compte de l'appelant (modifiable sans mot de passe).
fn own_rule(r: &AppRule, uid: u32) -> bool {
    r.user == Some(uid) && r.direction == Direction::Out && !is_system_uid(uid)
}

fn handle(shared: &Arc<Shared>, peer: Peer, req: Request) -> anyhow::Result<serde_json::Value> {
    let admin = is_admin(peer.uid);
    Ok(match req {
        Request::Status => v(&shared.state.lock().unwrap().status(peer.uid)),
        Request::Rules => {
            let st = shared.state.lock().unwrap();
            let mut r = st.rules.clone();
            if !admin {
                r.apps.retain(|a| a.user.is_none_or(|u| u == peer.uid));
            }
            v(&r)
        }
        Request::SaveAppRule(mut r) => {
            validate_app_rule(&mut r)?;
            let existing = shared.state.lock().unwrap().rules.apps.iter().find(|x| x.id == r.id).cloned();
            if r.id != 0 && existing.is_none() {
                anyhow::bail!("règle introuvable");
            }
            if !(own_rule(&r, peer.uid) && existing.as_ref().is_none_or(|e| own_rule(e, peer.uid))) {
                authorize(peer)?;
            }
            let mut st = shared.state.lock().unwrap();
            if r.id == 0 {
                r.id = st.rules.alloc_id();
                r.created = now();
                st.rules.apps.push(r.clone());
            } else if let Some(x) = st.rules.apps.iter_mut().find(|x| x.id == r.id) {
                r.hits = x.hits;
                r.last_hit = x.last_hit;
                r.created = x.created;
                *x = r.clone();
            }
            st.save_rules()?;
            v(&r.id)
        }
        Request::DeleteAppRule { id } => {
            let existing = shared.state.lock().unwrap().rules.apps.iter().find(|x| x.id == id).cloned().ok_or_else(|| anyhow::anyhow!("règle introuvable"))?;
            if !own_rule(&existing, peer.uid) {
                authorize(peer)?;
            }
            let mut st = shared.state.lock().unwrap();
            st.rules.apps.retain(|r| r.id != id);
            st.save_rules()?;
            serde_json::Value::Null
        }
        Request::SavePortRule(mut r) => {
            validate_port_rule(&mut r)?;
            authorize(peer)?;
            let mut st = shared.state.lock().unwrap();
            if r.id == 0 {
                r.id = st.rules.alloc_id();
                st.rules.ports.push(r.clone());
            } else if let Some(x) = st.rules.ports.iter_mut().find(|x| x.id == r.id) {
                *x = r.clone();
            } else {
                anyhow::bail!("règle introuvable");
            }
            st.save_rules()?;
            st.apply_nft();
            v(&r.id)
        }
        Request::DeletePortRule { id } => {
            authorize(peer)?;
            let mut st = shared.state.lock().unwrap();
            st.rules.ports.retain(|r| r.id != id);
            st.save_rules()?;
            st.apply_nft();
            serde_json::Value::Null
        }
        Request::SetConfig(mut c) => {
            c.timeout_secs = c.timeout_secs.clamp(5, 600);
            c.trusted_interfaces.retain(|i| validate_interface(i).is_ok());
            authorize(peer)?;
            let mut st = shared.state.lock().unwrap();
            store::save_config(&st.dir, &c)?;
            st.config = c;
            st.apply_nft();
            serde_json::Value::Null
        }
        Request::Pending => {
            let st = shared.state.lock().unwrap();
            let n = Instant::now();
            let list: Vec<Prompt> = st
                .pending
                .iter()
                .filter(|p| admin || p.prompt.program.uid == peer.uid)
                .map(|p| Prompt { deadline_secs: p.deadline.saturating_duration_since(n).as_secs() as u32, ..p.prompt.clone() })
                .collect();
            v(&list)
        }
        Request::Decide { id, decision } => v(&decide(shared, peer, id, decision)?),
        Request::Log { limit } => {
            let st = shared.state.lock().unwrap();
            v(&st.log.iter().filter(|e| admin || e.program.uid == peer.uid).take(limit.min(LOG_MAX)).cloned().collect::<Vec<_>>())
        }
        Request::ClearLog => {
            authorize(peer)?;
            shared.state.lock().unwrap().log.clear();
            serde_json::Value::Null
        }
        Request::Agent => anyhow::bail!("déjà traité"),
    })
}

fn decide(shared: &Arc<Shared>, peer: Peer, id: u64, d: Decision) -> anyhow::Result<DecideResult> {
    let prompt = {
        let mut st = shared.state.lock().unwrap();
        let p = st.pending.iter_mut().find(|p| p.prompt.id == id).ok_or_else(|| anyhow::anyhow!("cette demande n'est plus en attente"))?;
        let own = p.prompt.program.uid == peer.uid && !is_system_uid(peer.uid);
        if !own && !is_admin(peer.uid) {
            anyhow::bail!("cette demande ne vous concerne pas");
        }
        // Sans mot de passe : bloquer pour cette fois, ou décider pour ses propres sorties.
        let harmless = d.action == Action::Block && d.duration == Duration::Once;
        let own_out = own && p.prompt.conn.direction == Direction::Out && d.scope != Scope::Port && !d.all_users;
        let needs_auth = !(harmless || own_out) && peer.uid != 0;
        if needs_auth {
            p.authorizing = true;
        }
        (p.prompt.clone(), needs_auth)
    };
    let (prompt, needs_auth) = prompt;
    if needs_auth && let Err(e) = authorize(peer) {
        // Refus : la demande reste ouverte (délai repris).
        if let Some(p) = shared.state.lock().unwrap().pending.iter_mut().find(|p| p.prompt.id == id) {
            p.authorizing = false;
        }
        return Err(e);
    }
    let mut st = shared.state.lock().unwrap();
    let Some(pos) = st.pending.iter().position(|p| p.prompt.id == id) else { anyhow::bail!("cette demande n'est plus en attente") };
    st.pending.remove(pos);
    let (prog, conn) = (&prompt.program, &prompt.conn);
    let mut res = DecideResult::default();
    match d.duration {
        Duration::Once => {
            st.temp.push(Temp {
                key: prog.key.clone(),
                uid: prog.uid,
                direction: conn.direction,
                dest: Some((conn.remote.clone(), conn.port())),
                action: d.action,
                until: Instant::now() + StdDuration::from_secs(TEMP_SECS),
            });
        }
        Duration::Always if d.scope == Scope::Port => {
            let rid = st.rules.alloc_id();
            st.rules.ports.push(PortRule {
                id: rid,
                enabled: true,
                direction: conn.direction,
                action: d.action,
                proto: conn.proto,
                ports: conn.port().to_string(),
                note: format!("Créée pour {}", if prog.comm.is_empty() { &prog.key } else { &prog.comm }),
                ..Default::default()
            });
            st.save_rules()?;
            st.apply_nft();
            res.port_rule = Some(rid);
        }
        Duration::Always => {
            let rid = st.rules.alloc_id();
            let (port, remote) = match d.scope {
                Scope::ProgramPort => (Some(conn.port().to_string()), None),
                Scope::ProgramDestination => (Some(conn.port().to_string()), Some(conn.remote.clone())),
                _ => (None, None),
            };
            st.rules.apps.push(AppRule {
                id: rid,
                enabled: true,
                app: prog.key.clone(),
                label: prog.comm.clone(),
                direction: conn.direction,
                action: d.action,
                proto: if port.is_some() { conn.proto } else { Proto::Any },
                remote,
                port,
                user: if d.all_users || is_system_uid(prog.uid) { None } else { Some(prog.uid) },
                created: now(),
                ..Default::default()
            });
            st.save_rules()?;
            res.app_rule = Some(rid);
        }
    }
    let reason = match d.duration {
        Duration::Once => "pour cette fois",
        Duration::Always => "règle créée",
    };
    st.push_log(prog, conn, d.action, reason);
    st.broadcast_done(id);
    drop(st);
    let _ = shared.cmd.lock().unwrap().send(Cmd::Verdict { prompt: id, action: d.action });
    Ok(res)
}

static AGENT_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn serve_agent(shared: &Arc<Shared>, peer: Peer, mut stream: UnixStream) {
    let (tx, rx) = mpsc::channel();
    let id = AGENT_IDS.fetch_add(1, Ordering::SeqCst);
    let admin = is_admin(peer.uid);
    {
        let mut st = shared.state.lock().unwrap();
        st.agents.push(Agent { id, uid: peer.uid, admin, tx });
        let n = Instant::now();
        // Demandes faites sans fenêtre connectée : délai normal à partir de maintenant.
        let (timeout, default) = (StdDuration::from_secs(st.config.timeout_secs.clamp(5, 600) as u64), st.config.default_action);
        for p in st.pending.iter_mut().filter(|p| p.no_agent && (p.prompt.program.uid == peer.uid || admin)) {
            p.no_agent = false;
            p.deadline = n + timeout;
            p.prompt.default_action = default;
        }
        // Demandes déjà ouvertes qui concernent ce compte.
        let mine: Vec<Prompt> = st
            .pending
            .iter()
            .filter(|p| st.agents_for(&p.prompt).iter().any(|a| a.id == id))
            .map(|p| Prompt { deadline_secs: p.deadline.saturating_duration_since(n).as_secs() as u32, ..p.prompt.clone() })
            .collect();
        let last = st.agents.last().unwrap();
        for p in mine {
            let _ = last.tx.send(Event::Prompt(p));
        }
    }
    let _ = stream.write_all(format!("{}\n", serde_json::to_string(&Reply::ok(serde_json::Value::Null)).unwrap()).as_bytes());
    // Fermeture détectée tout de suite (fin de flux), pas au battement suivant : sinon les
    // demandes attendraient une fenêtre disparue au lieu du repli « aucune fenêtre ».
    if let Ok(mut r) = stream.try_clone() {
        let sh = shared.clone();
        let _ = r.set_read_timeout(None);
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            while matches!(r.read(&mut buf), Ok(n) if n > 0) {}
            sh.state.lock().unwrap().agents.retain(|a| a.id != id);
        });
    }
    loop {
        let line = match rx.recv_timeout(StdDuration::from_secs(30)) {
            Ok(ev) => serde_json::to_string(&ev).unwrap_or_default(),
            Err(mpsc::RecvTimeoutError::Timeout) => String::new(), // battement : détecte la fermeture
            Err(_) => break,
        };
        if stream.write_all(format!("{line}\n").as_bytes()).is_err() {
            break;
        }
    }
    shared.state.lock().unwrap().agents.retain(|a| a.id != id);
}

fn serve_client(shared: Arc<Shared>, stream: UnixStream) {
    let Ok(peer) = peer_cred(&stream) else { return };
    let _ = stream.set_read_timeout(Some(StdDuration::from_secs(30)));
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read.take(64 * 1024));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let reply = match serde_json::from_str::<Request>(&line) {
        Ok(Request::Agent) => return serve_agent(&shared, peer, stream),
        Ok(req) => match handle(&shared, peer, req) {
            Ok(v) => Reply::ok(v),
            Err(e) => Reply::err(format!("{e:#}")),
        },
        Err(e) => Reply::err(format!("requête invalide : {e}")),
    };
    let mut stream = stream;
    let _ = stream.write_all(format!("{}\n", serde_json::to_string(&reply).unwrap()).as_bytes());
}

fn v<T: serde::Serialize>(x: &T) -> serde_json::Value {
    serde_json::to_value(x).unwrap_or_default()
}

/// Point d'entrée du démon.
pub fn run(socket: &Path) -> anyhow::Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        anyhow::bail!("noos-firewalld doit être lancé en root (service systemd noos-firewall)");
    }
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    let (tx, rx) = mpsc::channel();
    let shared = Arc::new(Shared { state: Mutex::new(State::new(store::dir())), cmd: Mutex::new(tx) });
    // File ouverte avant la table : aucune connexion n'arrive dans une file sans lecteur.
    let q = open_queue().map_err(|e| anyhow::anyhow!("NFQUEUE : {e} (module nfnetlink_queue ?)"))?;
    shared.state.lock().unwrap().apply_nft();

    if let Some(d) = socket.parent() {
        std::fs::create_dir_all(d)?;
        std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755))?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    // Tout le monde peut se connecter : les droits sont vérifiés par requête (SO_PEERCRED, polkit).
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    let sh = shared.clone();
    std::thread::spawn(move || {
        for s in listener.incoming().flatten() {
            let sh = sh.clone();
            std::thread::spawn(move || serve_client(sh, s));
        }
    });
    {
        // Un seul verrou par instruction : deux lock() dans la même expression = interblocage.
        let st = shared.state.lock().unwrap();
        eprintln!("noos-firewalld : prêt (mode {:?}, {} règles)", st.config.mode, st.rules.apps.len());
    }
    queue_loop(shared.clone(), q, rx);
    let mut st = shared.state.lock().unwrap();
    let _ = st.save_rules();
    let _ = std::fs::remove_file(socket);
    eprintln!("noos-firewalld : arrêt (la table reste en place : entrées fermées, sorties autorisées)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let d = std::env::temp_dir().join(format!("noos-fw-daemon-{}-{:?}", std::process::id(), std::thread::current().id()));
        State::new(d)
    }

    fn prog(key: &str, uid: u32) -> Program {
        Program { key: key.into(), exe: key.into(), comm: key.rsplit('/').next().unwrap().into(), uid, ..Default::default() }
    }

    fn out(remote: &str, port: u16) -> Conn {
        Conn { direction: Direction::Out, proto: Proto::Tcp, ip_proto: 6, remote: remote.into(), remote_port: port, ..Default::default() }
    }

    #[test]
    fn decisions() {
        let mut st = state();
        // Programmes de NOOS : règle livrée.
        assert!(matches!(st.decide(&prog("/opt/noos/bin/noos-software", 1000), &out("1.1.1.1", 443)), Outcome::Now(Action::Allow, _)));
        // Service du système.
        assert!(matches!(st.decide(&prog("/usr/sbin/NetworkManager", 0), &out("1.1.1.1", 443)), Outcome::Now(Action::Allow, _)));
        // Nouveau programme : une seule demande, les connexions suivantes s'y ajoutent.
        let a = st.decide(&prog("/usr/bin/curl", 1000), &out("1.1.1.1", 443));
        let b = st.decide(&prog("/usr/bin/curl", 1000), &out("8.8.8.8", 80));
        let (Outcome::Wait(a), Outcome::Wait(b)) = (a, b) else { panic!() };
        assert_eq!(a, b);
        assert_eq!(st.pending.len(), 1);
        assert_eq!(st.pending[0].prompt.packets, 2);
        assert!(!st.pending[0].prompt.needs_admin);
        // Entrée sans programme à l'écoute : bloquée sans demander.
        let inc = Conn { direction: Direction::In, ..out("203.0.113.1", 3389) };
        assert!(matches!(st.decide(&Program { key: "aucun".into(), ..Default::default() }, &inc), Outcome::Now(Action::Block, _)));
        // Aucune fenêtre connectée : sortie autorisée au bout de 3 s (session pas encore prête).
        assert!(st.pending[0].no_agent);
        assert_eq!(st.pending[0].prompt.default_action, Action::Allow);
        st.pending[0].deadline = Instant::now();
        assert_eq!(st.expire(), vec![(a, Action::Allow)]);
        // Avec une fenêtre connectée : délai normal, puis décision par défaut, retenue.
        let (tx, _rx) = mpsc::channel();
        st.agents.push(Agent { id: 1, uid: 1000, admin: false, tx });
        let Outcome::Wait(c) = st.decide(&prog("/usr/bin/curl2", 1000), &out("1.1.1.1", 443)) else { panic!() };
        assert!(!st.pending[0].no_agent);
        st.pending[0].deadline = Instant::now();
        assert_eq!(st.expire(), vec![(c, Action::Block)]);
        assert!(matches!(st.decide(&prog("/usr/bin/curl2", 1000), &out("9.9.9.9", 443)), Outcome::Now(Action::Block, _)));
        assert_eq!(st.log.len(), 1); // seuls les blocages sont journalisés
        // Modes.
        st.config.mode = Mode::AllowOutgoing;
        assert!(matches!(st.decide(&prog("/usr/bin/wget", 1000), &out("1.1.1.1", 443)), Outcome::Now(Action::Allow, _)));
        let java = Conn { direction: Direction::In, ..out("192.168.1.5", 25565) };
        assert!(matches!(st.decide(&prog("/usr/bin/java", 1000), &java), Outcome::Wait(_)));
        assert!(st.pending.last().unwrap().prompt.needs_admin);
        st.config.mode = Mode::Silent;
        assert!(matches!(st.decide(&prog("/usr/bin/nc", 1000), &java), Outcome::Now(Action::Block, _)));
    }

    #[test]
    fn rule_validation() {
        let mut r = AppRule { app: " /usr/bin/x ".into(), port: Some("80, 443".into()), remote: Some(" ".into()), ..Default::default() };
        validate_app_rule(&mut r).unwrap();
        assert_eq!((r.app.as_str(), r.port.as_deref(), r.remote.as_deref()), ("/usr/bin/x", Some("80,443"), None));
        let mut r = AppRule { app: "*".into(), ..Default::default() };
        assert!(validate_app_rule(&mut r).is_err());
        let mut p = PortRule { ports: "22".into(), interface: Some("eth0 accept".into()), ..Default::default() };
        assert!(validate_port_rule(&mut p).is_err());
        assert!(own_rule(&AppRule { user: Some(1000), direction: Direction::Out, ..Default::default() }, 1000));
        assert!(!own_rule(&AppRule { user: None, direction: Direction::Out, ..Default::default() }, 1000));
        assert!(!own_rule(&AppRule { user: Some(1000), direction: Direction::In, ..Default::default() }, 1000));
    }
}

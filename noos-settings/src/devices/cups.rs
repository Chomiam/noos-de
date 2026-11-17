//! Imprimantes (CUPS) et scanners (SANE) par leurs outils en ligne de commande, sortie analysée
//! en `LC_ALL=C` : `lpstat`, `lpoptions -p` (état, modèle, adresse, niveaux d'encre), `lpq`
//! (file d'attente), `lpinfo -l -v` / `driverless` (détection réseau et USB), `lpinfo -m`
//! (pilotes), `lpadmin` (ajout, suppression, imprimante par défaut), `scanimage -L`.
//!
//! Droits : CUPS accepte les opérations d'administration des membres du groupe `lpadmin`
//! (authentification locale, sans mot de passe) ; à défaut, la commande est relancée par
//! pkexec (fenêtre d'élévation de NOOS). Jamais sudo.
//!
//! Démo sans CUPS : `NOOS_PRINT_DEMO=1`.

use std::{collections::HashMap, process::Command, time::Duration};

pub fn demo() -> bool {
    std::env::var_os("NOOS_PRINT_DEMO").is_some()
}

pub fn demo_mode() -> String {
    std::env::var("NOOS_PRINT_DEMO").unwrap_or_default()
}

pub fn have(bin: &str) -> bool {
    noos_pkg::exec::have(bin) || sbin(bin).is_some()
}

/// Debian installe `lpadmin`, `lpinfo`, `cupsenable`… dans /usr/sbin, absent du PATH des
/// utilisateurs : chemin complet.
fn sbin(bin: &str) -> Option<String> {
    ["/usr/sbin", "/sbin"].iter().map(|d| format!("{d}/{bin}")).find(|p| std::path::Path::new(p).exists())
}

fn tool(bin: &str) -> String {
    if bin.contains('/') || noos_pkg::exec::have(bin) {
        return bin.to_string();
    }
    sbin(bin).unwrap_or_else(|| bin.to_string())
}

fn run(argv: &[&str]) -> Result<String, String> {
    let out = Command::new(tool(argv[0]))
        .args(&argv[1..])
        .env("LC_ALL", "C.UTF-8")
        .output()
        .map_err(|e| format!("{} : {e}", argv[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("{} a échoué ({})", argv[0], out.status) } else { err })
    }
}

/// Refus d'autorisation de CUPS (compte hors du groupe lpadmin).
fn denied(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("forbidden") || e.contains("unauthorized") || e.contains("not authorized") || e.contains("authentication") || e.contains("permission denied")
}

/// Commande d'administration : directe (groupe lpadmin), sinon par pkexec.
pub fn admin(argv: &[&str]) -> Result<String, String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(600));
        return Ok(String::new());
    }
    match run(argv) {
        Err(e) if denied(&e) => {
            let mut full: Vec<String> = noos_pkg::exec::privilege_prefix();
            full.push(tool(argv[0]));
            full.extend(argv[1..].iter().map(|s| s.to_string()));
            let refs: Vec<&str> = full.iter().map(String::as_str).collect();
            run(&refs).map_err(|e| explain(&e))
        }
        r => r.map_err(|e| explain(&e)),
    }
}

/// Message clair pour une erreur de CUPS.
pub fn explain(err: &str) -> String {
    let e = err.trim().trim_start_matches("lpadmin: ").trim_start_matches("lp: ").trim_start_matches("cancel: ").trim_start_matches("lpinfo: ");
    let l = e.to_lowercase();
    if l.contains("scheduler is not running") || l.contains("connection refused") || l.contains("bad file descriptor") {
        "Le service d'impression (CUPS) ne répond pas.".into()
    } else if l.contains("not authorized") || l.contains("dismissed") || l.contains("not authenticated") {
        "Autorisation refusée.".into()
    } else if l.contains("unable to connect to printer") || l.contains("no route to host") {
        "Impossible de joindre l'imprimante : vérifiez qu'elle est allumée et sur le même réseau.".into()
    } else if l.contains("printer drivers are deprecated") {
        // Simple avertissement de CUPS 2.4 : pas une erreur.
        String::new()
    } else if l.contains("unable to get printer attributes") || l.contains("unable to query printer") {
        "L'imprimante ne répond pas à la requête IPP : elle n'est peut-être pas compatible IPP Everywhere. Choisissez un pilote.".into()
    } else {
        e.to_string()
    }
}

// ───────────────────────────── État du service ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Running,
    Stopped,
    NotInstalled,
}

pub fn service() -> Service {
    if demo() {
        return if demo_mode() == "missing" { Service::NotInstalled } else { Service::Running };
    }
    if !have("lpstat") {
        return Service::NotInstalled;
    }
    match run(&["lpstat", "-r"]) {
        Ok(s) if s.contains("is running") => Service::Running,
        _ => Service::Stopped,
    }
}

/// L'utilisateur est-il dans le groupe lpadmin (administration sans mot de passe) ?
pub fn in_lpadmin() -> bool {
    if demo() {
        return true;
    }
    run(&["id", "-Gn"]).map(|s| s.split_whitespace().any(|g| g == "lpadmin")).unwrap_or(false)
}

// ───────────────────────────── Imprimantes ─────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Idle,
    Printing,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub name: String,
    pub color: String,
    /// 0–100 ; négatif = inconnu.
    pub level: i32,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Printer {
    pub name: String,
    pub info: String,
    pub location: String,
    pub model: String,
    pub uri: String,
    pub state: State,
    pub accepting: bool,
    pub reasons: Vec<String>,
    pub shared: bool,
    pub default: bool,
    pub markers: Vec<Marker>,
    pub jobs: usize,
}

impl Printer {
    pub fn title(&self) -> &str {
        if self.info.is_empty() { &self.name } else { &self.info }
    }

    /// Avertissements à afficher (raisons d'état de CUPS traduites).
    pub fn problems(&self) -> Vec<String> {
        self.reasons.iter().filter_map(|r| reason_text(r)).collect()
    }

    pub fn state_text(&self) -> String {
        let p = self.problems();
        match (&self.state, p.first()) {
            (State::Stopped, _) => "En pause".into(),
            (_, Some(first)) => first.clone(),
            (State::Printing, _) => "Impression en cours".into(),
            (State::Idle, _) if self.jobs > 0 => format!("{} document{} en attente", self.jobs, if self.jobs > 1 { "s" } else { "" }),
            (State::Idle, _) => "Prête".into(),
        }
    }

    pub fn connection(&self) -> &'static str {
        connection(&self.uri)
    }
}

/// Raison d'état CUPS (`printer-state-reasons`) → phrase ; None pour les raisons sans intérêt.
pub fn reason_text(reason: &str) -> Option<String> {
    let base = reason.trim_end_matches("-error").trim_end_matches("-warning").trim_end_matches("-report");
    Some(
        match base {
            "none" | "" | "paused" | "cups-waiting-for-job-completed" | "cups-ipp-conformance-failure" => return None,
            "media-empty" | "media-needed" => "Bac à papier vide",
            "media-jam" => "Bourrage papier",
            "toner-low" => "Toner bientôt vide",
            "toner-empty" => "Toner vide",
            "marker-supply-low" => "Encre bientôt vide",
            "marker-supply-empty" => "Cartouche vide",
            "door-open" | "cover-open" => "Capot ouvert",
            "offline" | "connecting-to-device" => "Imprimante hors ligne ou injoignable",
            "cups-missing-filter" => "Pilote manquant pour ce type de document",
            "input-tray-missing" => "Bac à papier absent",
            "output-area-full" | "output-tray-missing" => "Bac de sortie plein",
            "other" => return None,
            _ => return None,
        }
        .into(),
    )
}

/// Type de connexion d'après l'adresse du périphérique.
pub fn connection(uri: &str) -> &'static str {
    let scheme = uri.split(':').next().unwrap_or("");
    match scheme {
        "usb" => "USB",
        "ipp" | "ipps" | "http" | "https" | "dnssd" | "socket" | "lpd" | "smb" => {
            if uri.contains("ipp-usb") || uri.contains("localhost") || uri.contains("127.0.0.1") {
                "USB (IPP)"
            } else {
                "Réseau"
            }
        }
        "implicitclass" => "Réseau (partagée)",
        "hp" | "hpfax" => "HPLIP",
        "parallel" | "serial" => "Port série / parallèle",
        "bluetooth" => "Bluetooth",
        _ => "Autre",
    }
}

/// Découpe la sortie de `lpoptions -p` (« clé=valeur » séparés par des espaces, valeurs entre
/// apostrophes ou avec `\` d'échappement).
pub fn parse_options(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut chars = s.trim().chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut token = String::new();
        let mut quote: Option<char> = None;
        while let Some(c) = chars.next() {
            match (quote, c) {
                (None, c) if c.is_whitespace() => break,
                (None, '\'' | '"') => quote = Some(c),
                (Some(q), c) if c == q => quote = None,
                (_, '\\') => {
                    if let Some(n) = chars.next() {
                        token.push(n);
                    }
                }
                (_, c) => token.push(c),
            }
        }
        match token.split_once('=') {
            Some((k, v)) => out.insert(k.to_string(), v.to_string()),
            None => out.insert(token, String::new()),
        };
    }
    out
}

/// Niveaux d'encre / de toner (`marker-*`, listes séparées par des virgules).
pub fn parse_markers(o: &HashMap<String, String>) -> Vec<Marker> {
    let list = |k: &str| -> Vec<String> { o.get(k).map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default() };
    let names = list("marker-names");
    let colors = list("marker-colors");
    let levels = list("marker-levels");
    let kinds = list("marker-types");
    names
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_empty())
        .map(|(i, n)| Marker {
            name: n.clone(),
            color: colors.get(i).cloned().unwrap_or_default(),
            level: levels.get(i).and_then(|l| l.parse().ok()).unwrap_or(-1),
            kind: kinds.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

fn printer_from(name: &str, o: &HashMap<String, String>, default: &str) -> Printer {
    let g = |k: &str| o.get(k).cloned().unwrap_or_default();
    Printer {
        name: name.to_string(),
        info: g("printer-info"),
        location: g("printer-location"),
        model: g("printer-make-and-model"),
        uri: g("device-uri"),
        state: match g("printer-state").as_str() {
            "4" => State::Printing,
            "5" => State::Stopped,
            _ => State::Idle,
        },
        accepting: g("printer-is-accepting-jobs") != "false",
        reasons: g("printer-state-reasons").split(',').map(str::to_string).filter(|r| !r.is_empty() && r != "none").collect(),
        shared: g("printer-is-shared") == "true",
        default: name == default,
        markers: parse_markers(o),
        jobs: 0,
    }
}

/// `lpstat -p <file>` : « printer X is idle. », « printer X now printing X-3. »,
/// « printer X disabled since … ».
pub fn parse_lpstat_state(s: &str) -> State {
    if s.contains(" disabled") {
        State::Stopped
    } else if s.contains("now printing") {
        State::Printing
    } else {
        State::Idle
    }
}

/// `lpstat -o <file>` (sans `lpq`, paquet cups-bsd) : « X-12  alex  1024  Tue 17 Nov … ».
pub fn parse_lpstat_jobs(s: &str, printer: &str) -> Vec<Job> {
    s.lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            let id = t.first()?.strip_prefix(printer)?.strip_prefix('-')?.to_string();
            Some(Job { title: format!("Document n° {id}"), owner: t.get(1)?.to_string(), size: t.get(2)?.parse().ok()?, active: false, id })
        })
        .collect()
}

/// `lpstat -d` → nom de l'imprimante par défaut.
pub fn parse_default(s: &str) -> String {
    s.lines().find_map(|l| l.split_once("system default destination:").map(|(_, n)| n.trim().to_string())).unwrap_or_default()
}

pub fn printers() -> Vec<Printer> {
    if demo() {
        return demo_printers();
    }
    let names: Vec<String> = run(&["lpstat", "-e"]).map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default();
    let default = run(&["lpstat", "-d"]).map(|s| parse_default(&s)).unwrap_or_default();
    let mut out: Vec<Printer> = names
        .iter()
        .filter(|n| !n.contains('/'))
        .map(|n| {
            let o = run(&["lpoptions", "-p", n]).map(|s| parse_options(&s)).unwrap_or_default();
            let mut p = printer_from(n, &o, &default);
            // Files créées par cups-browsed (implicitclass) : pas de printer-state dans
            // lpoptions ; `lpstat -p` le donne toujours.
            if !o.contains_key("printer-state") {
                if let Ok(st) = run(&["lpstat", "-p", n]) {
                    p.state = parse_lpstat_state(&st);
                }
            }
            p.jobs = jobs(n).len();
            p
        })
        .collect();
    out.sort_by(|a, b| (!a.default, a.title().to_lowercase()).cmp(&(!b.default, b.title().to_lowercase())));
    out
}

// ───────────────────────────── File d'attente ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: String,
    pub owner: String,
    pub title: String,
    pub size: u64,
    pub active: bool,
}

/// Sortie de `lpq -P <imprimante>` :
/// `Rank    Owner   Job     File(s)                         Total Size`
/// `active  alex    12      Rapport annuel.pdf              123456 bytes`
pub fn parse_lpq(s: &str) -> Vec<Job> {
    s.lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            if t.len() < 6 || t[t.len() - 1] != "bytes" || t[0] == "Rank" {
                return None;
            }
            let size = t[t.len() - 2].parse().ok()?;
            Some(Job {
                active: t[0] == "active",
                owner: t[1].to_string(),
                id: t[2].to_string(),
                title: t[3..t.len() - 2].join(" "),
                size,
            })
        })
        .collect()
}

pub fn jobs(printer: &str) -> Vec<Job> {
    if demo() {
        return demo_printers().into_iter().find(|p| p.name == printer).map(|p| demo_jobs(&p)).unwrap_or_default();
    }
    if have("lpq") {
        return run(&["lpq", "-P", printer]).map(|s| parse_lpq(&s)).unwrap_or_default();
    }
    let mut v = run(&["lpstat", "-o", printer]).map(|s| parse_lpstat_jobs(&s, printer)).unwrap_or_default();
    if let Some(first) = v.first_mut() {
        first.active = run(&["lpstat", "-p", printer]).is_ok_and(|s| parse_lpstat_state(&s) == State::Printing);
    }
    v
}

pub fn cancel_job(printer: &str, id: &str) -> Result<(), String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(300));
        return Ok(());
    }
    // Ses propres travaux : sans droits particuliers ; ceux des autres : admin.
    let job = format!("{printer}-{id}");
    admin(&["cancel", &job]).map(|_| ())
}

pub fn cancel_all(printer: &str) -> Result<(), String> {
    admin(&["cancel", "-a", printer]).map(|_| ())
}

pub fn set_default(printer: &str) -> Result<(), String> {
    admin(&["lpadmin", "-d", printer]).map(|_| ())
}

pub fn set_paused(printer: &str, paused: bool) -> Result<(), String> {
    admin(&[if paused { "cupsdisable" } else { "cupsenable" }, printer]).map(|_| ())
}

pub fn set_shared(printer: &str, shared: bool) -> Result<(), String> {
    admin(&["lpadmin", "-p", printer, "-o", &format!("printer-is-shared={shared}")]).map(|_| ())
}

pub fn remove(printer: &str) -> Result<(), String> {
    admin(&["lpadmin", "-x", printer]).map(|_| ())
}

/// Page de test de CUPS (fichier du paquet cups) ; renvoie le numéro du travail.
pub fn test_page(printer: &str) -> Result<String, String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(500));
        return Ok("42".into());
    }
    let file = ["/usr/share/cups/data/testprint", "/usr/share/cups/data/default-testpage.pdf"]
        .into_iter()
        .find(|f| std::path::Path::new(f).exists())
        .ok_or("Page de test introuvable (/usr/share/cups/data/testprint).")?;
    let out = run(&["lp", "-d", printer, "-t", "Page de test NOOS", file]).map_err(|e| explain(&e))?;
    // « request id is HP-42 (1 file(s)) »
    Ok(out.split_whitespace().nth(3).and_then(|id| id.rsplit('-').next()).unwrap_or("").to_string())
}

// ───────────────────────────── Détection ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub uri: String,
    /// « network » ou « direct ».
    pub class: String,
    pub model: String,
    pub info: String,
    pub location: String,
    pub device_id: String,
    /// IPP Everywhere / AirPrint : ajout en un clic, sans pilote.
    pub driverless: bool,
}

impl Found {
    pub fn title(&self) -> String {
        let t = if !self.info.is_empty() && self.info != "Unknown" { &self.info } else { &self.model };
        let t = t.trim_end_matches(" (driverless)").trim();
        if t.is_empty() || t == "Unknown" { self.uri.clone() } else { t.to_string() }
    }

    pub fn connection(&self) -> &'static str {
        connection(&self.uri)
    }
}

/// Sortie de `lpinfo -l -v` : blocs « Device: uri = … » suivis de « class = », « info = »…
pub fn parse_lpinfo(s: &str) -> Vec<Found> {
    let mut out: Vec<Found> = vec![];
    for line in s.lines() {
        let t = line.trim();
        if let Some(uri) = t.strip_prefix("Device: uri = ") {
            out.push(Found {
                uri: uri.trim().to_string(),
                class: String::new(),
                model: String::new(),
                info: String::new(),
                location: String::new(),
                device_id: String::new(),
                driverless: false,
            });
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let Some((k, v)) = t.split_once(" = ") else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "class" => cur.class = v,
            "info" => cur.info = v,
            "make-and-model" => cur.model = v,
            "device-id" => cur.device_id = v,
            "location" => cur.location = v,
            _ => {}
        }
    }
    // Seuls les vrais appareils (une adresse complète), pas les moteurs (« socket », « lpd »).
    out.retain(|f| {
        (f.uri.contains(":/") || f.uri.contains("://"))
            && (f.class == "network" || f.class == "direct")
            && !["testfile:", "cups-brf:", "cups-pdf:", "beh:"].iter().any(|p| f.uri.starts_with(p))
    });
    for f in &mut out {
        f.driverless = looks_driverless(f);
    }
    out
}

/// Indices d'une imprimante sans pilote : service IPP annoncé par DNS-SD, formats PWG/URF
/// dans l'identifiant IEEE 1284, ou passerelle ipp-usb.
pub fn looks_driverless(f: &Found) -> bool {
    let id = f.device_id.to_uppercase();
    (f.uri.starts_with("dnssd://") && (f.uri.contains("._ipp._tcp") || f.uri.contains("._ipps._tcp")))
        || f.uri.starts_with("ipp://") && f.uri.contains("/ipp/print")
        || f.uri.starts_with("ipps://")
        || id.contains("URF") && id.contains("CMD:") && (id.contains("PWG") || id.contains("URF"))
        || id.contains("PWGRASTER") || id.contains("PWG-RASTER")
}

/// Recherche des imprimantes (≈ 5 à 10 s) : `lpinfo -l -v` complété par `driverless`.
pub fn discover(configured: &[Printer]) -> Result<Vec<Found>, String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(2500));
        return Ok(demo_found());
    }
    let mut found = match admin(&["lpinfo", "-l", "-v", "--timeout", "6"]) {
        Ok(s) => parse_lpinfo(&s),
        Err(e) => return Err(e),
    };
    // URI « driverless » (ipp://…/ipp/print) pour les imprimantes IPP Everywhere.
    if have("driverless") {
        if let Ok(s) = run(&["driverless"]) {
            for uri in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if let Some(f) = found.iter_mut().find(|f| same_device(&f.uri, uri)) {
                    f.driverless = true;
                } else if !found.iter().any(|f| f.uri == uri) {
                    found.push(Found {
                        uri: uri.to_string(),
                        class: "network".into(),
                        model: String::new(),
                        info: String::new(),
                        location: String::new(),
                        device_id: String::new(),
                        driverless: true,
                    });
                }
            }
        }
    }
    // Déjà installées (y compris les files créées automatiquement par cups-browsed, dont
    // l'adresse diffère : même nom ou même modèle) : écartées.
    found.retain(|f| {
        let t = f.title().to_lowercase();
        !configured.iter().any(|p| p.uri == f.uri || p.title().to_lowercase() == t || (!f.model.is_empty() && p.model.to_lowercase().ends_with(&f.model.to_lowercase())))
    });
    // Une même imprimante annoncée en IPP et en « socket » : on garde la meilleure entrée.
    found.sort_by_key(|f| (!f.driverless, f.class != "direct", f.title().to_lowercase()));
    let mut seen: Vec<String> = vec![];
    found.retain(|f| {
        let k = f.title().to_lowercase();
        if seen.contains(&k) {
            return false;
        }
        seen.push(k);
        true
    });
    Ok(found)
}

/// Deux adresses désignent-elles le même appareil (même nom d'hôte DNS-SD) ?
fn same_device(a: &str, b: &str) -> bool {
    let host = |u: &str| -> String {
        let rest = u.split("://").nth(1).unwrap_or("");
        let h = rest.split(['/', ':', '?']).next().unwrap_or("");
        percent_decode(h).to_lowercase().replace("._ipp._tcp.local", "").replace("._ipps._tcp.local", "").replace(".local", "")
    };
    let (ha, hb) = (host(a), host(b));
    !ha.is_empty() && ha == hb
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Nom de file CUPS valide d'après le modèle : « HP LaserJet Pro M404 » → « HP_LaserJet_Pro_M404 ».
pub fn queue_name(title: &str, taken: &[String]) -> String {
    let mut base: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if base.is_empty() {
        base = "Imprimante".into();
    }
    base.truncate(100);
    let mut name = base.clone();
    let mut n = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// Ajoute une imprimante ; `driver` = None → IPP Everywhere (`-m everywhere`).
pub fn add(name: &str, f: &Found, driver: Option<&str>) -> Result<(), String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(1500));
        if demo_mode() == "fail" {
            return Err(explain("lpadmin: Unable to get printer attributes"));
        }
        return Ok(());
    }
    let info = f.title();
    let mut argv = vec!["lpadmin", "-p", name, "-E", "-v", &f.uri, "-m", driver.unwrap_or("everywhere"), "-D", &info];
    if !f.location.is_empty() {
        argv.extend(["-L", &f.location]);
    }
    argv.extend(["-o", "printer-is-shared=false"]);
    match admin(&argv) {
        Ok(_) => Ok(()),
        // CUPS 2.4 avertit que les pilotes sont « obsolètes » mais ajoute l'imprimante.
        Err(e) if e.is_empty() => Ok(()),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Driver {
    pub id: String,
    pub name: String,
    pub recommended: bool,
}

/// Sortie de `lpinfo -m` : « <ppd> <description> ».
pub fn parse_drivers(s: &str) -> Vec<Driver> {
    s.lines()
        .filter_map(|l| {
            let (id, name) = l.trim().split_once(' ')?;
            Some(Driver { id: id.to_string(), name: name.trim().to_string(), recommended: name.contains("recommended") })
        })
        .collect()
}

/// Pilotes : ceux qui correspondent au modèle d'abord, puis tous (pour la recherche).
pub fn drivers(f: &Found) -> (Vec<Driver>, Vec<Driver>) {
    if demo() {
        let all = demo_drivers();
        let m: Vec<Driver> = all.iter().filter(|d| d.name.contains("DCP-L2530")).cloned().collect();
        return (m, all);
    }
    let mut matching = vec![];
    if !f.device_id.is_empty() {
        if let Ok(s) = run(&["lpinfo", "--device-id", &f.device_id, "-m"]) {
            matching = parse_drivers(&s);
        }
    }
    if matching.is_empty() && !f.model.is_empty() {
        if let Ok(s) = run(&["lpinfo", "--make-and-model", &f.model, "-m"]) {
            matching = parse_drivers(&s);
        }
    }
    let all = run(&["lpinfo", "-m"]).map(|s| parse_drivers(&s)).unwrap_or_default();
    // « everywhere » et « driverless » d'abord, puis les pilotes recommandés.
    matching.sort_by_key(|d| (!d.id.starts_with("driverless"), !d.recommended, d.name.to_lowercase()));
    (matching, all)
}

// ───────────────────────────── Pages Web ─────────────────────────────

pub const CUPS_WEB: &str = "http://localhost:631/";

/// Page Web d'administration de l'imprimante (son serveur intégré), d'après son adresse.
pub fn web_page(uri: &str) -> Option<String> {
    let scheme = uri.split("://").next()?;
    let rest = uri.split("://").nth(1)?;
    let host = rest.split(['/', '?']).next()?.rsplit('@').next()?;
    let hostname = host.rsplit_once(':').map(|(h, port)| if port.chars().all(|c| c.is_ascii_digit()) { h } else { host }).unwrap_or(host);
    match scheme {
        "ipp" | "ipps" | "http" | "https" | "socket" | "lpd" if !hostname.is_empty() => {
            // ipp-usb : la page de l'imprimante USB est servie par ipp-usb sur localhost:<port>.
            if hostname == "localhost" || hostname == "127.0.0.1" {
                return Some(format!("http://{host}/"));
            }
            Some(format!("http://{hostname}/"))
        }
        "dnssd" => {
            // dnssd://Nom%20de%20service._ipp._tcp.local/?uuid=… : résolu par Avahi.
            let service = percent_decode(hostname);
            let name = service.split("._").next()?.to_string();
            let kind = if service.contains("._ipps._tcp") { "_ipps._tcp" } else { "_ipp._tcp" };
            resolve_dnssd(&name, kind)
        }
        _ => None,
    }
}

/// Résout un service DNS-SD annoncé (`avahi-browse -rpt`) : `adminurl` s'il est annoncé,
/// sinon `http://<hôte>/`.
fn resolve_dnssd(name: &str, kind: &str) -> Option<String> {
    if !have("avahi-browse") {
        return None;
    }
    let out = run(&["avahi-browse", "-rptk", kind]).ok()?;
    parse_avahi(&out, name)
}

/// Lignes « = » de `avahi-browse -rptk` (champs séparés par `;`, noms échappés en `\NNN`).
pub fn parse_avahi(out: &str, name: &str) -> Option<String> {
    for l in out.lines().filter(|l| l.starts_with('=')) {
        let f: Vec<&str> = l.split(';').collect();
        if f.len() < 9 || avahi_unescape(f[3]) != name {
            continue;
        }
        if let Some(admin) = f[9..].join(";").split('"').find_map(|t| t.strip_prefix("adminurl=").map(str::to_string)) {
            return Some(admin);
        }
        let host = f[6].trim_end_matches('.');
        return Some(format!("http://{host}/"));
    }
    None
}

fn avahi_unescape(s: &str) -> String {
    let mut out = String::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            out.push(s[i + 1..i + 4].parse::<u8>().unwrap_or(b'?') as char);
            i += 4;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

// ───────────────────────────── Scanners ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Scanner {
    pub device: String,
    pub name: String,
    pub kind: String,
}

/// `scanimage -L` : « device `airscan:e0:HP LaserJet' is a eSCL HP LaserJet ip=192.168.1.20 ».
pub fn parse_scanners(s: &str) -> Vec<Scanner> {
    s.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("device `")?;
            let (device, desc) = rest.split_once("' is a ")?;
            let desc = desc.trim();
            let (kind, name) = if let Some(n) = desc.strip_prefix("eSCL ") {
                ("Réseau (eSCL / AirScan)", n)
            } else if let Some(n) = desc.strip_prefix("WSD ") {
                ("Réseau (WSD)", n)
            } else if device.starts_with("airscan:") || device.starts_with("escl:") {
                ("Réseau (eSCL / AirScan)", desc)
            } else if device.contains("libusb") || device.starts_with("hpaio:/usb") || device.contains(":usb") {
                ("USB", desc)
            } else if device.starts_with("v4l:") {
                ("Webcam", desc)
            } else {
                ("Scanner", desc)
            };
            let ip = desc.split(" ip=").nth(1).map(|s| s.trim().to_string());
            let name = name.split(" ip=").next().unwrap_or(name).trim();
            let kind = if device.contains(":net:") { "Réseau" } else { kind };
            Some((Scanner { device: device.to_string(), name: name.to_string(), kind: kind.to_string() }, ip))
        })
        // Les webcams (v4l) ne sont pas des scanners.
        .filter(|(s, _)| s.kind != "Webcam")
        .fold(Vec::<(Scanner, Vec<String>)>::new(), |mut acc, (s, ip)| {
            // Même appareil vu par plusieurs moteurs (escl et epson2 de SANE, sane-airscan) :
            // même adresse IP ou même modèle ; l'entrée airscan (nom complet) est gardée.
            let mut keys = vec![scanner_key(&s.name)];
            keys.extend(ip.into_iter().chain(scanner_host(&s.device)));
            match acc.iter_mut().find(|(_, k)| k.iter().any(|x| keys.contains(x))) {
                Some((o, k)) => {
                    k.extend(keys);
                    if s.device.starts_with("airscan:") {
                        *o = s;
                    }
                }
                None => acc.push((s, keys)),
            }
            acc
        })
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

/// Adresse d'un scanner réseau d'après son périphérique SANE (`epson2:net:192.168.1.10`,
/// `escl:https://192.168.1.10:443`).
fn scanner_host(device: &str) -> Option<String> {
    if let Some(h) = device.split("://").nth(1) {
        return h.split([':', '/']).next().map(str::to_string).filter(|h| !h.is_empty());
    }
    device.split(":net:").nth(1).map(|h| h.trim().to_string())
}

/// « Epson ET-4800 Series platen,adf scanner » → « epson et-4800 series ».
fn scanner_key(name: &str) -> String {
    let n = name.to_lowercase();
    let cut = [" platen", " adf", " flatbed", " scanner", " sheetfed"].iter().filter_map(|w| n.find(w)).min().unwrap_or(n.len());
    n[..cut].trim().to_string()
}

pub fn scanners() -> Result<Vec<Scanner>, String> {
    if demo() {
        std::thread::sleep(Duration::from_millis(1800));
        return Ok(vec![
            Scanner { device: "airscan:e0:HP LaserJet Pro M404".into(), name: "HP LaserJet Pro M404".into(), kind: "Réseau (eSCL / AirScan)".into() },
            Scanner { device: "airscan:e1:EPSON ET-2850 Series".into(), name: "EPSON ET-2850 Series".into(), kind: "Réseau (eSCL / AirScan)".into() },
        ]);
    }
    if !have("scanimage") {
        return Err("Les outils de numérisation (sane-utils) ne sont pas installés.".into());
    }
    run(&["scanimage", "-L"]).map(|s| parse_scanners(&s))
}

/// Logiciel de numérisation installé (lancé par « Numériser »).
pub fn scan_app() -> Option<&'static str> {
    if demo() {
        return Some("simple-scan");
    }
    ["simple-scan", "skanpage", "skanlite", "xsane"].into_iter().find(|b| have(b))
}

// ───────────────────────────── Démo ─────────────────────────────

fn demo_printers() -> Vec<Printer> {
    let o = |s: &str| parse_options(s);
    let mut v = vec![
        printer_from(
            "HP_LaserJet_Pro_M404",
            &o("device-uri=ipp://HP-M404.local:631/ipp/print printer-info='HP LaserJet Pro M404' printer-location=Bureau printer-make-and-model='HP LaserJet Pro M404 (IPP Everywhere)' printer-state=3 printer-state-reasons=none printer-is-accepting-jobs=true marker-names='Black\\ Toner' marker-colors=#000000 marker-levels=64 marker-types=toner"),
            "HP_LaserJet_Pro_M404",
        ),
        printer_from(
            "EPSON_ET-2850",
            &o("device-uri=dnssd://EPSON%20ET-2850%20Series._ipp._tcp.local/?uuid=cfe92100 printer-info='EPSON ET-2850 Series' printer-location=Salon printer-make-and-model='EPSON ET-2850 Series (IPP Everywhere)' printer-state=4 printer-state-reasons=marker-supply-low-warning marker-names=Black,Cyan,Magenta,Yellow marker-colors=#000000,#00FFFF,#FF00FF,#FFFF00 marker-levels=72,45,12,88 marker-types=ink-tank,ink-tank,ink-tank,ink-tank"),
            "HP_LaserJet_Pro_M404",
        ),
        printer_from(
            "Brother_HL-L2350DW",
            &o("device-uri=usb://Brother/HL-L2350DW%20series?serial=E78123 printer-info='Brother HL-L2350DW' printer-make-and-model='Brother HL-L2350DW series, using brlaser v6' printer-state=5 printer-state-reasons=paused"),
            "HP_LaserJet_Pro_M404",
        ),
    ];
    for p in &mut v {
        p.jobs = demo_jobs(p).len();
    }
    v
}

fn demo_jobs(p: &Printer) -> Vec<Job> {
    if !p.name.starts_with("EPSON") {
        return vec![];
    }
    vec![
        Job { id: "57".into(), owner: "alex".into(), title: "Photos vacances Bretagne.pdf".into(), size: 18_420_118, active: true },
        Job { id: "58".into(), owner: "alex".into(), title: "Attestation CAF.pdf".into(), size: 245_320, active: false },
    ]
}

fn demo_found() -> Vec<Found> {
    let f = |uri: &str, class: &str, model: &str, info: &str, loc: &str, id: &str| {
        let mut f = Found { uri: uri.into(), class: class.into(), model: model.into(), info: info.into(), location: loc.into(), device_id: id.into(), driverless: false };
        f.driverless = looks_driverless(&f);
        f
    };
    if demo_mode() == "none" {
        return vec![];
    }
    vec![
        f("dnssd://Canon%20TS5300%20series._ipp._tcp.local/?uuid=00000000-0000-1000-8000-60128b123456", "network", "Canon TS5300 series", "Canon TS5300 series", "Chambre", "MFG:Canon;CMD:URF,PWGRaster;MDL:TS5300 series;"),
        f("ipp://Brother-MFC.local:631/ipp/print", "network", "Brother MFC-L2710DW series", "Brother MFC-L2710DW", "", ""),
        f("usb://Brother/DCP-L2530DW%20series?serial=U63887B9N123456", "direct", "Brother DCP-L2530DW series", "Brother DCP-L2530DW series", "", "MFG:Brother;CMD:PJL,PCL,PCLXL;MDL:DCP-L2530DW series;"),
        f("socket://192.168.1.42", "network", "Kyocera ECOSYS P2135dn", "Kyocera ECOSYS P2135dn", "", "MFG:Kyocera;CMD:PCLXL,PJL;MDL:ECOSYS P2135dn;"),
    ]
}

fn demo_drivers() -> Vec<Driver> {
    parse_drivers(
        "drv:///brlaser.drv/brl2530.ppd Brother DCP-L2530DW series, using brlaser v6 (recommended)\n\
         drv:///sample.drv/generic.ppd Generic PostScript Printer\n\
         drv:///sample.drv/generpcl.ppd Generic PCL Laser Printer\n\
         gutenprint.5.3://brother-dcp-l2530dw/expert Brother DCP-L2530DW - CUPS+Gutenprint v5.3.6\n\
         drv:///hpcups.drv/hp-laserjet_pro_m404-m405.ppd HP LaserJet Pro M404-M405, hpcups 3.26.4\n\
         drv:///brlaser.drv/brl2350.ppd Brother HL-L2350DW series, using brlaser v6\n\
         lsb/usr/cupsfilters/Fuji_Xerox-DocuPrint_P115_b.ppd Fuji Xerox DocuPrint P115 b",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options() {
        let o = parse_options("copies=1 device-uri=ipp://hp.local:631/ipp/print printer-info='HP LaserJet Pro' marker-names='Black\\ Toner,Cyan' printer-location printer-make-and-model='HP\\ M404 (IPP Everywhere)'");
        assert_eq!(o["printer-info"], "HP LaserJet Pro");
        assert_eq!(o["device-uri"], "ipp://hp.local:631/ipp/print");
        assert_eq!(o["marker-names"], "Black Toner,Cyan");
        assert_eq!(o["printer-location"], "");
        assert_eq!(o["printer-make-and-model"], "HP M404 (IPP Everywhere)");
    }

    #[test]
    fn markers() {
        let o = parse_options("marker-names=Black,Cyan marker-colors=#000000,#00FFFF marker-levels=72,-1 marker-types=ink,ink");
        let m = parse_markers(&o);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].level, 72);
        assert_eq!(m[1].level, -1);
        assert_eq!(m[1].color, "#00FFFF");
    }

    #[test]
    fn lpq() {
        let s = "EPSON is ready and printing\nRank    Owner   Job     File(s)                         Total Size\nactive  alex    57      Photos vacances.pdf             18420118 bytes\n1st     alex    58      Attestation.pdf                 245320 bytes\n";
        let j = parse_lpq(s);
        assert_eq!(j.len(), 2);
        assert!(j[0].active);
        assert_eq!(j[0].title, "Photos vacances.pdf");
        assert_eq!(j[1].id, "58");
        assert_eq!(j[1].size, 245_320);
        assert!(parse_lpq("no entries\n").is_empty());
    }

    #[test]
    fn lpinfo() {
        let s = "Device: uri = socket\n        class = network\n        info = AppSocket/HP JetDirect\n        make-and-model = Unknown\n        device-id = \n        location = \n\
Device: uri = dnssd://Canon%20TS5300%20series._ipp._tcp.local/?uuid=0000\n        class = network\n        info = Canon TS5300 series\n        make-and-model = Canon TS5300 series\n        device-id = MFG:Canon;CMD:URF;\n        location = Chambre\n\
Device: uri = usb://Brother/DCP-L2530DW%20series?serial=U6\n        class = direct\n        info = Brother DCP-L2530DW series\n        make-and-model = Brother DCP-L2530DW series\n        device-id = MFG:Brother;CMD:PJL,PCL;\n        location = \n";
        let f = parse_lpinfo(s);
        assert_eq!(f.len(), 2);
        assert!(f[0].driverless);
        assert_eq!(f[0].location, "Chambre");
        assert!(!f[1].driverless);
        assert_eq!(f[1].connection(), "USB");
    }

    #[test]
    fn default_printer() {
        assert_eq!(parse_default("system default destination: HP_M404\n"), "HP_M404");
        assert_eq!(parse_default("no system default destination\n"), "");
    }

    #[test]
    fn names() {
        assert_eq!(queue_name("HP LaserJet Pro M404 (IPP)", &[]), "HP_LaserJet_Pro_M404_IPP");
        assert_eq!(queue_name("Canon TS5300", &["canon_ts5300".into()]), "Canon_TS5300_2");
        assert_eq!(queue_name("///", &[]), "Imprimante");
    }

    #[test]
    fn web() {
        assert_eq!(web_page("ipp://HP-M404.local:631/ipp/print").as_deref(), Some("http://HP-M404.local/"));
        assert_eq!(web_page("socket://192.168.1.42:9100").as_deref(), Some("http://192.168.1.42/"));
        assert_eq!(web_page("usb://Brother/HL?serial=1"), None);
        assert_eq!(percent_decode("Canon%20TS5300"), "Canon TS5300");
    }

    #[test]
    fn avahi() {
        let out = "=;wlan0;IPv4;Canon\\032TS5300\\032series;_ipp._tcp;local;Canon123.local;192.168.1.30;631;\"adminurl=http://Canon123.local./index.html\" \"ty=Canon TS5300\"\n";
        assert_eq!(parse_avahi(out, "Canon TS5300 series").as_deref(), Some("http://Canon123.local./index.html"));
        assert_eq!(parse_avahi(out, "Autre"), None);
    }

    #[test]
    fn scan() {
        let s = "device `airscan:e0:HP LaserJet Pro M404' is a eSCL HP LaserJet Pro M404 ip=192.168.1.20\ndevice `v4l:/dev/video0' is a Noname Integrated Camera virtual device\ndevice `genesys:libusb:001:004' is a Canon LiDE 220 flatbed scanner\n";
        let v = parse_scanners(s);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "HP LaserJet Pro M404");
        assert_eq!(v[1].kind, "USB");
    }

    #[test]
    fn reel_debian() {
        // Sorties relevées sur Debian sid (EPSON ET-4800, file créée par cups-browsed).
        let s = "device `epson2:net:192.168.1.10' is a Epson PID flatbed scanner\ndevice `escl:https://192.168.1.10:443' is a Epson ET-4800 Series platen,adf scanner\ndevice `airscan:e0:EPSON ET-4800 Series' is a eSCL EPSON ET-4800 Series ip=192.168.1.10\n";
        let v = parse_scanners(&format!("<html>\n<body></body></HTML>{s}"));
        assert_eq!(v.len(), 1);
        assert!(v[0].device.starts_with("airscan:"));
        let l = "Device: uri = testfile:/tmp/lpr-testfile.out\n        class = direct\n        info = Unknown\n        make-and-model = Unknown\n\
Device: uri = cups-brf:/\n        class = file\n        info = CUPS-BRF\n        make-and-model = Virtual Braille BRF Printer\n\
Device: uri = ipps://EPSON%20ET-4800%20Series._ipps._tcp.local/\n        class = network\n        info = EPSON ET-4800 Series (driverless)\n        make-and-model = EPSON ET-4800 Series\n        device-id = MFG:EPSON;MDL:ET-4800 Series;CMD:PWGRaster,AppleRaster,PWG,URF,JPEG;\n";
        let f = parse_lpinfo(l);
        assert_eq!(f.len(), 1);
        assert!(f[0].driverless);
        assert_eq!(f[0].title(), "EPSON ET-4800 Series");
        assert_eq!(parse_lpstat_state("printer EPSON is idle.  enabled since Tue Nov 17 2026\n"), State::Idle);
        assert_eq!(parse_lpstat_state("printer EPSON disabled since Tue Nov 17 2026 -\n\tPaused\n"), State::Stopped);
        let j = parse_lpstat_jobs("EPSON_ET_4800_Series-12 chomiam 1024 Tue Nov 17 10:00:00 2026\n", "EPSON_ET_4800_Series");
        assert_eq!((j[0].id.as_str(), j[0].size), ("12", 1024));
        let avahi = "=;eno1;IPv4;EPSON\\032ET-4800\\032Series;_ipps._tcp;local;EPSONF1F248.local;192.168.1.10;631;\"TLS=1.2\" \"adminurl=http://EPSONF1F248.local.:80/PRESENTATION/BONJOUR\"\n";
        assert_eq!(parse_avahi(avahi, "EPSON ET-4800 Series").as_deref(), Some("http://EPSONF1F248.local.:80/PRESENTATION/BONJOUR"));
    }

    #[test]
    fn reasons() {
        assert_eq!(reason_text("media-empty-error").as_deref(), Some("Bac à papier vide"));
        assert_eq!(reason_text("none"), None);
        let p = &demo_printers()[1];
        assert_eq!(p.state_text(), "Encre bientôt vide");
        assert_eq!(demo_printers()[2].state_text(), "En pause");
    }

    #[test]
    fn drivers_parse() {
        let d = demo_drivers();
        assert!(d[0].recommended);
        assert_eq!(d[1].id, "drv:///sample.drv/generic.ppd");
    }
}

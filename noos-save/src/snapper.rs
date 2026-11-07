//! Lecture de l'état des instantanés (sans privilège) et appel du script privilégié.
//!
//! - liste : `snapper --jsonout -c root list` (snapperd ; les membres de « wheel » y ont accès
//!   grâce à `ALLOW_GROUPS=wheel`, réglé par `noos-save-helper setup`) ;
//! - réglages : `/etc/snapper/configs/root` et `/etc/noos/save.conf` ;
//! - restauration : `/var/lib/noos-save/pending` (prévue) et `restored` (faite) ;
//! - modifications : `pkexec noos-save-helper …` (action polkit org.noos.save.helper).

use std::{collections::HashMap, path::Path, process::Command};

use anyhow::{Context, bail};
use gtk::glib;
use serde_json::Value;

pub const HELPER: &str = "/opt/noos/libexec/noos-save-helper";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Single,
    Pre,
    Post,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub number: u32,
    pub kind: Kind,
    pub pre_number: Option<u32>,
    pub date: String,
    pub description: String,
    pub cleanup: String,
    pub userdata: HashMap<String, String>,
}

impl Snapshot {
    pub fn manual(&self) -> bool {
        self.userdata.get("noos").is_some_and(|v| v == "manuel")
    }
    pub fn timeline(&self) -> bool {
        self.cleanup == "timeline"
    }
    /// « À conserver » : exclu du nettoyage automatique.
    pub fn important(&self) -> bool {
        self.userdata.get("important").is_some_and(|v| v == "yes")
    }
    pub fn origin(&self) -> Origin {
        if self.timeline() {
            Origin::Timeline
        } else if self.manual() || (self.cleanup.is_empty() && !self.userdata.contains_key("noos") && self.kind == Kind::Single) {
            Origin::Manual
        } else {
            Origin::Dnf
        }
    }
}

/// D'où vient un instantané : créé par l'utilisateur ou automatiquement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Manual,
    Dnf,
    Timeline,
}

impl Origin {
    pub fn automatic(self) -> bool {
        self != Origin::Manual
    }
}

/// Une ligne de la liste : un instantané seul, ou une paire avant / après (dnf).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub main: Snapshot,
    pub post: Option<Snapshot>,
}

impl Entry {
    pub fn numbers(&self) -> Vec<u32> {
        std::iter::once(self.main.number).chain(self.post.as_ref().map(|p| p.number)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Status {
    pub btrfs: bool,
    pub configured: bool,
    pub config: HashMap<String, String>,
    pub dnf_snapshots: bool,
    /// Restauration prévue au prochain redémarrage (SNAPSHOT, OLD, KERNEL, DATE).
    pub pending: Option<HashMap<String, String>>,
    /// Restauration effectuée (ancienne racine conservée dans OLD, si pas encore supprimée).
    pub restored: Option<HashMap<String, String>>,
    pub entries: Result<Vec<Entry>, String>,
    /// Démarré sur un instantané (entrée GRUB, `noos.snapshot=N`) : session temporaire.
    pub boot_snapshot: Option<u32>,
    /// Sous-menu GRUB « Restaurer depuis un instantané » et son nombre d'entrées.
    pub grub_snapshots: bool,
    pub grub_max: u32,
    /// Espace du système de fichiers Btrfs (octets) : taille, utilisé.
    pub disk: Option<(u64, u64)>,
    pub device: String,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            btrfs: false,
            configured: false,
            config: HashMap::new(),
            dnf_snapshots: true,
            pending: None,
            restored: None,
            entries: Ok(vec![]),
            boot_snapshot: None,
            grub_snapshots: true,
            grub_max: 20,
            disk: None,
            device: String::new(),
        }
    }
}

impl Status {
    pub fn get(&self, key: &str) -> &str {
        self.config.get(key).map(String::as_str).unwrap_or("")
    }
    pub fn number(&self, key: &str, default: u32) -> u32 {
        self.get(key).parse().unwrap_or(default)
    }
    pub fn list(&self) -> &[Entry] {
        self.entries.as_deref().unwrap_or(&[])
    }
    /// Numéros présents dans le sous-menu GRUB (les `grub_max` plus récents, tous types).
    pub fn in_grub(&self, number: u32) -> bool {
        if !self.grub_snapshots {
            return false;
        }
        let mut all: Vec<u32> = self.list().iter().flat_map(Entry::numbers).collect();
        all.sort_unstable_by(|a, b| b.cmp(a));
        all.iter().take(self.grub_max as usize).any(|&n| n == number)
    }
}

/// Détails d'un instantané, lus à la demande (dépliage d'une ligne).
#[derive(Debug, Clone, Default)]
pub struct Details {
    pub kernel: Option<String>,
    pub packages: Option<usize>,
}

pub fn snapshot_root(number: u32) -> String {
    format!("/.snapshots/{number}/snapshot")
}

pub fn details(number: u32) -> Details {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        std::thread::sleep(std::time::Duration::from_millis(300));
        return Details { kernel: Some("7.2.15-300.fc45.x86_64".into()), packages: Some(1180 + number as usize * 3) };
    }
    let root = snapshot_root(number);
    let kernel = std::fs::read_dir(format!("{root}/usr/lib/modules"))
        .ok()
        .and_then(|d| {
            let mut v: Vec<String> = d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v.pop()
        });
    let packages = packages(Some(number)).ok().map(|p| p.len());
    Details { kernel, packages }
}

/// Paquets installés (nom.arch → version-release) dans un instantané, ou le système actuel.
pub fn packages(number: Option<u32>) -> anyhow::Result<HashMap<String, String>> {
    let mut cmd = Command::new("rpm");
    if let Some(n) = number {
        cmd.args(["--dbpath", &format!("{}/usr/lib/sysimage/rpm", snapshot_root(n))]);
    }
    let out = cmd.args(["-qa", "--qf", "%{NAME}.%{ARCH} %{VERSION}-%{RELEASE}\\n"]).output().context("rpm")?;
    if !out.status.success() {
        bail!("base rpm illisible");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with("gpg-pubkey"))
        .filter_map(|l| l.split_once(' '))
        .map(|(n, v)| (n.trim_end_matches(".x86_64").trim_end_matches(".noarch").to_string(), v.to_string()))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    Added,
    Removed,
    Upgraded,
    Downgraded,
}

/// Différences de paquets de `from` vers `to` (None = système actuel).
pub fn package_diff(from: u32, to: Option<u32>) -> anyhow::Result<Vec<(Change, String, String)>> {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        std::thread::sleep(std::time::Duration::from_millis(500));
        return Ok(vec![
            (Change::Added, "git".into(), "2.51.0-1.fc45".into()),
            (Change::Added, "perl-Git".into(), "2.51.0-1.fc45".into()),
            (Change::Upgraded, "noos".into(), "0.1.0-1.git0faf7d21 → 0.1.0-1.git295c319b".into()),
            (Change::Upgraded, "kernel-core".into(), "7.2.14-300.fc45 → 7.2.15-300.fc45".into()),
            (Change::Removed, "nano-default-editor".into(), "8.4-2.fc45".into()),
        ]);
    }
    let a = packages(Some(from))?;
    let b = packages(to)?;
    let mut out = Vec::new();
    for (name, v) in &b {
        match a.get(name) {
            None => out.push((Change::Added, name.clone(), v.clone())),
            Some(old) if old != v => {
                let newer = glib_vercmp(v, old);
                out.push((if newer { Change::Upgraded } else { Change::Downgraded }, name.clone(), format!("{old} → {v}")));
            }
            _ => {}
        }
    }
    for (name, v) in &a {
        if !b.contains_key(name) {
            out.push((Change::Removed, name.clone(), v.clone()));
        }
    }
    out.sort();
    Ok(out)
}

/// Comparaison simple de versions rpm (segments numériques / alphabétiques), suffisante ici.
fn glib_vercmp(a: &str, b: &str) -> bool {
    fn parts(s: &str) -> Vec<Result<u64, String>> {
        let mut v = Vec::new();
        let mut cur = String::new();
        let mut digit = None;
        for c in s.chars() {
            if !c.is_ascii_alphanumeric() {
                if !cur.is_empty() {
                    v.push(if digit == Some(true) { Ok(cur.parse().unwrap_or(0)) } else { Err(cur.clone()) });
                    cur.clear();
                }
                digit = None;
                continue;
            }
            let d = c.is_ascii_digit();
            if digit.is_some_and(|x| x != d) && !cur.is_empty() {
                v.push(if digit == Some(true) { Ok(cur.parse().unwrap_or(0)) } else { Err(cur.clone()) });
                cur.clear();
            }
            digit = Some(d);
            cur.push(c);
        }
        if !cur.is_empty() {
            v.push(if digit == Some(true) { Ok(cur.parse().unwrap_or(0)) } else { Err(cur) });
        }
        v
    }
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(pb.iter()) {
        let ord = match (x, y) {
            (Ok(x), Ok(y)) => x.cmp(y),
            (Ok(_), Err(_)) => std::cmp::Ordering::Greater,
            (Err(_), Ok(_)) => std::cmp::Ordering::Less,
            (Err(x), Err(y)) => x.cmp(y),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord == std::cmp::Ordering::Greater;
        }
    }
    pa.len() > pb.len()
}

/// Fichiers modifiés de `from` vers `to` (0 = système actuel) : `snapper status` (« c..... /x »).
pub fn file_changes(from: u32, to: u32) -> anyhow::Result<Vec<(char, String)>> {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        std::thread::sleep(std::time::Duration::from_millis(900));
        return Ok(vec![
            ('+', "/usr/bin/git".into()),
            ('+', "/usr/share/doc/git/README.md".into()),
            ('c', "/etc/dnf/dnf.conf".into()),
            ('c', "/var/lib/rpm/rpmdb.sqlite".into()),
            ('-', "/usr/bin/nano".into()),
        ]);
    }
    let out = Command::new("snapper")
        .args(["-c", "root", "status", &format!("{from}..{to}")])
        .env("LC_ALL", "C.UTF-8")
        .output()
        .context("snapper")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (flags, path) = l.split_once(' ')?;
            Some((flags.chars().next().unwrap_or('c'), path.trim().to_string()))
        })
        .collect())
}

/// Fichier `CLÉ="valeur"` (configs snapper, save.conf, pending…).
pub fn parse_kv(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        .collect()
}

fn read_kv(path: &str) -> Option<HashMap<String, String>> {
    std::fs::read_to_string(path).ok().map(|t| parse_kv(&t))
}

fn num(v: &Value) -> Option<u32> {
    v.as_u64().map(|n| n as u32).or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

/// Sortie JSON de `snapper list` : `{"root": [ {number, type, pre-number, date, …} ]}`.
pub fn parse_list(json: &str) -> anyhow::Result<Vec<Snapshot>> {
    let v: Value = serde_json::from_str(json).context("sortie de snapper illisible")?;
    let list = match &v {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o.values().find_map(|x| x.as_array().cloned()).unwrap_or_default(),
        _ => vec![],
    };
    Ok(list
        .iter()
        .filter_map(|s| {
            let number = num(s.get("number")?)?;
            let kind = match s.get("type").and_then(Value::as_str).unwrap_or("single") {
                "pre" => Kind::Pre,
                "post" => Kind::Post,
                _ => Kind::Single,
            };
            let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let userdata = s
                .get("userdata")
                .and_then(Value::as_object)
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
                .unwrap_or_default();
            Some(Snapshot {
                number,
                kind,
                pre_number: s.get("pre-number").and_then(num),
                date: text("date"),
                description: text("description"),
                cleanup: text("cleanup"),
                userdata,
            })
        })
        .collect())
}

/// Regroupe les paires avant / après, du plus récent au plus ancien, sans l'état courant (0).
pub fn group(snaps: Vec<Snapshot>) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for s in snaps.iter().filter(|s| s.number != 0) {
        match s.kind {
            Kind::Post => {}
            Kind::Pre => {
                let post = snaps.iter().find(|p| p.kind == Kind::Post && p.pre_number == Some(s.number)).cloned();
                entries.push(Entry { main: s.clone(), post });
            }
            Kind::Single => entries.push(Entry { main: s.clone(), post: None }),
        }
    }
    // Instantané « après » orphelin (avant supprimé) : affiché seul.
    for s in snaps.iter().filter(|s| s.kind == Kind::Post) {
        if !entries.iter().any(|e| e.post.as_ref().is_some_and(|p| p.number == s.number)) {
            entries.push(Entry { main: s.clone(), post: None });
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.main.number));
    entries
}

fn fstype(path: &str) -> String {
    Command::new("findmnt")
        .args(["-no", "FSTYPE", "-T", path])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn read() -> Status {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        return demo();
    }
    let btrfs = fstype("/") == "btrfs";
    let configured = Path::new("/etc/snapper/configs/root").exists();
    let config = read_kv("/etc/snapper/configs/root").unwrap_or_default();
    let save = read_kv("/etc/noos/save.conf").unwrap_or_default();
    let entries = if configured {
        Command::new("snapper")
            .args(["--jsonout", "-c", "root", "list", "--disable-used-space"])
            .env("LC_ALL", "C.UTF-8")
            .output()
            .map_err(|e| format!("snapper : {e}"))
            .and_then(|o| {
                if o.status.success() {
                    parse_list(&String::from_utf8_lossy(&o.stdout)).map(group).map_err(|e| format!("{e:#}"))
                } else {
                    Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
                }
            })
    } else {
        Ok(vec![])
    };
    let boot_snapshot = std::fs::read_to_string("/proc/cmdline")
        .ok()
        .and_then(|c| c.split_whitespace().find_map(|a| a.strip_prefix("noos.snapshot=").and_then(|n| n.parse().ok())));
    // Démarré sur un instantané, la racine est un overlay : le Btrfs est celui de /home.
    let btrfs = btrfs || (boot_snapshot.is_some() && fstype("/home") == "btrfs");
    let fs_dir = if boot_snapshot.is_some() { "/home" } else { "/" };
    let df = Command::new("df").args(["-B1", "--output=size,used,source", fs_dir]).output().ok();
    let (disk, device) = df
        .and_then(|o| {
            let t = String::from_utf8_lossy(&o.stdout).lines().nth(1)?.to_string();
            let mut it = t.split_whitespace();
            let size = it.next()?.parse().ok()?;
            let used = it.next()?.parse().ok()?;
            Some((Some((size, used)), it.next().unwrap_or("").to_string()))
        })
        .unwrap_or((None, String::new()));
    Status {
        btrfs,
        configured,
        config,
        boot_snapshot,
        grub_snapshots: save.get("GRUB_SNAPSHOTS").is_none_or(|v| v != "no"),
        grub_max: save.get("GRUB_SNAPSHOTS_MAX").and_then(|v| v.parse().ok()).unwrap_or(20),
        disk,
        device,
        dnf_snapshots: save.get("DNF_SNAPSHOTS").is_none_or(|v| v != "no"),
        pending: read_kv("/var/lib/noos-save/pending"),
        restored: read_kv("/var/lib/noos-save/restored").filter(|r| r.contains_key("OLD")),
        entries,
    }
}

/// Lance le script privilégié (pkexec, sauf si on est déjà root) et renvoie sa sortie.
pub fn helper(args: &[&str]) -> anyhow::Result<String> {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        std::thread::sleep(std::time::Duration::from_millis(600));
        return Ok(format!("(démo) noos-save-helper {}", args.join(" ")));
    }
    let helper = std::env::var("NOOS_SAVE_HELPER").unwrap_or_else(|_| HELPER.to_string());
    let root = Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout.starts_with(b"0"));
    let mut cmd = if root { Command::new(&helper) } else { Command::new("pkexec") };
    if !root {
        cmd.arg(&helper);
    }
    let out = cmd.args(args).env("LC_ALL", "C.UTF-8").output().context("lancement de noos-save-helper")?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    match out.status.code() {
        Some(0) => Ok(stdout),
        Some(126) => bail!("authentification refusée ou annulée"),
        Some(127) if !root => bail!("pas d'agent d'authentification polkit"),
        _ => bail!("{}", if stderr.is_empty() { stdout } else { stderr }),
    }
}

fn demo() -> Status {
    let now = glib::DateTime::now_local().expect("heure locale");
    let at = |days: i32, h: i32, m: i32, s: i32| {
        let d = now.add_days(-days).expect("date");
        format!("{:04}-{:02}-{:02} {h:02}:{m:02}:{s:02}", d.year(), d.month(), d.day_of_month())
    };
    let snap = |number, kind, pre: Option<u32>, date: String, description: &str, cleanup: &str, noos: &str| Snapshot {
        number,
        kind,
        pre_number: pre,
        date,
        description: description.into(),
        cleanup: cleanup.into(),
        userdata: if noos.is_empty() { HashMap::new() } else { [("noos".to_string(), noos.to_string())].into() },
    };
    let mut important = snap(6, Kind::Single, None, at(1, 21, 4, 0), "Avant d'installer les pilotes NVIDIA", "", "manuel");
    important.userdata.insert("important".into(), "yes".into());
    let list = vec![
        snap(1, Kind::Single, None, at(9, 20, 26, 2), "Installation de NOOS", "", "manuel"),
        snap(2, Kind::Single, None, at(6, 10, 0, 0), "timeline", "timeline", ""),
        snap(3, Kind::Pre, None, at(3, 6, 31, 10), "dnf install -y git gh", "number", "dnf"),
        snap(4, Kind::Post, Some(3), at(3, 6, 31, 40), "dnf install -y git gh", "number", "dnf"),
        snap(5, Kind::Single, None, at(1, 9, 0, 0), "timeline", "timeline", ""),
        important,
        snap(7, Kind::Pre, None, at(0, 7, 40, 12), "dnf upgrade --refresh", "number", "dnf"),
        snap(8, Kind::Post, Some(7), at(0, 7, 43, 3), "dnf upgrade --refresh", "number", "dnf"),
        snap(9, Kind::Single, None, at(0, 9, 12, 45), "Avant de tester le thème sombre", "", "manuel"),
    ];
    Status {
        btrfs: true,
        configured: true,
        config: parse_kv("NUMBER_LIMIT=\"10\"\nTIMELINE_CREATE=\"no\"\nTIMELINE_LIMIT_DAILY=\"7\"\nTIMELINE_LIMIT_WEEKLY=\"2\"\nTIMELINE_LIMIT_MONTHLY=\"1\"\n"),
        dnf_snapshots: true,
        pending: std::env::var_os("NOOS_SAVE_DEMO_PENDING")
            .map(|_| parse_kv("SNAPSHOT=4\nOLD=@.avant-restauration-20261107-091500\nKERNEL=7.2.15-300.fc45.x86_64\n")),
        restored: None,
        entries: Ok(group(list)),
        boot_snapshot: std::env::var("NOOS_SAVE_DEMO_BOOT").ok().and_then(|v| v.parse().ok()),
        grub_snapshots: true,
        grub_max: 20,
        disk: Some((254_000_000_000, 61_500_000_000)),
        device: "/dev/nvme0n1p3".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_and_pairs() {
        let json = r#"{"root":[
          {"subvolume":"/","number":0,"default":false,"active":false,"type":"single","pre-number":null,"date":"","user":"root","cleanup":"","description":"current","userdata":null},
          {"subvolume":"/","number":1,"type":"single","pre-number":null,"date":"2026-11-07 06:00:00","cleanup":"","description":"Installation de NOOS","userdata":{"noos":"manuel"}},
          {"subvolume":"/","number":2,"type":"pre","pre-number":null,"date":"2026-11-07 06:31:10","cleanup":"number","description":"dnf install git","userdata":{"noos":"dnf"}},
          {"subvolume":"/","number":3,"type":"post","pre-number":2,"date":"2026-11-07 06:31:40","cleanup":"number","description":"dnf install git","userdata":{"noos":"dnf"}}
        ]}"#;
        let snaps = parse_list(json).unwrap();
        assert_eq!(snaps.len(), 4);
        let e = group(snaps);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].main.number, 2);
        assert_eq!(e[0].post.as_ref().unwrap().number, 3);
        assert_eq!(e[0].numbers(), [2, 3]);
        assert!(e[1].main.manual());
    }

    #[test]
    fn kv() {
        let m = parse_kv("# x\nNUMBER_LIMIT=\"10\"\nTIMELINE_CREATE=\"no\"\n");
        assert_eq!(m["NUMBER_LIMIT"], "10");
        assert_eq!(m["TIMELINE_CREATE"], "no");
    }
}

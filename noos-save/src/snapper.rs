//! Lecture de l'état des instantanés (sans privilège) et appel du script privilégié.
//!
//! - liste : `snapper --jsonout -c root list` (snapperd ; les membres de « wheel » y ont accès
//!   grâce à `ALLOW_GROUPS=wheel`, réglé par `noos-save-helper setup`) ;
//! - réglages : `/etc/snapper/configs/root` et `/etc/noos/save.conf` ;
//! - restauration : `/var/lib/noos-save/pending` (prévue) et `restored` (faite) ;
//! - modifications : `pkexec noos-save-helper …` (action polkit org.noos.save.helper).

use std::{collections::HashMap, path::Path, process::Command};

use anyhow::{Context, bail};
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
    pub fn from_dnf(&self) -> bool {
        self.userdata.get("noos").is_some_and(|v| v == "dnf") || self.cleanup == "number"
    }
    pub fn manual(&self) -> bool {
        self.userdata.get("noos").is_some_and(|v| v == "manuel")
    }
    pub fn timeline(&self) -> bool {
        self.cleanup == "timeline"
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

pub fn read() -> Status {
    if std::env::var_os("NOOS_SAVE_DEMO").is_some() {
        return demo();
    }
    let btrfs = Command::new("findmnt")
        .args(["-no", "FSTYPE", "/"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "btrfs")
        .unwrap_or(false);
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
    Status {
        btrfs,
        configured,
        config,
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
    let snap = |number, kind, pre: Option<u32>, date: &str, description: &str, cleanup: &str, noos: &str| Snapshot {
        number,
        kind,
        pre_number: pre,
        date: date.into(),
        description: description.into(),
        cleanup: cleanup.into(),
        userdata: [("noos".to_string(), noos.to_string())].into(),
    };
    let list = vec![
        snap(1, Kind::Single, None, "2026-11-06 20:26:02", "Installation de NOOS", "", "manuel"),
        snap(2, Kind::Pre, None, "2026-11-07 06:31:10", "dnf install -y git", "number", "dnf"),
        snap(3, Kind::Post, Some(2), "2026-11-07 06:31:40", "dnf install -y git", "number", "dnf"),
        snap(4, Kind::Pre, None, "2026-11-07 07:40:12", "dnf upgrade --refresh noos", "number", "dnf"),
        snap(5, Kind::Post, Some(4), "2026-11-07 07:41:03", "dnf upgrade --refresh noos", "number", "dnf"),
        snap(6, Kind::Single, None, "2026-11-07 09:00:00", "Avant d'installer les pilotes NVIDIA", "", "manuel"),
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

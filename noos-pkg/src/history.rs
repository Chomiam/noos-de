//! Historique des opérations : journal d'apt (`/var/log/apt/history.log*`, aussi les commandes
//! tapées en terminal), historique de Flatpak (`flatpak history`, journal systemd) et journal
//! de Logiciels (`~/.local/state/noos/software-history.json`). Chaque opération apt est
//! rapprochée de l'instantané « avant » de noos-save / snapper pour revenir en arrière.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{BackendKind, Plan, backends::apt::apt_root, exec::capture_codes};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Change {
    pub name: String,
    /// Nouvelle version (ou version retirée).
    pub version: String,
    /// Ancienne version (mise à jour).
    pub old: Option<String>,
    /// Installé comme dépendance.
    pub automatic: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Operation {
    pub backend: Option<BackendKind>,
    /// Secondes depuis 1970 (heure locale du journal, pour trier et rapprocher).
    pub timestamp: i64,
    /// « 2026-11-18 16:46 ».
    pub date: String,
    pub command: String,
    pub requested_by: String,
    pub installed: Vec<Change>,
    pub upgraded: Vec<Change>,
    pub removed: Vec<Change>,
    /// Instantané « avant » (noos-save / snapper).
    pub snapshot: Option<u32>,
    /// Titre (journal de Logiciels) ; vide pour apt.
    pub title: String,
    pub success: bool,
}

impl Operation {
    /// Titre lisible : « Installation : gimp (+31 dépendances) ».
    pub fn summary(&self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        let main = |list: &[Change]| {
            let explicit: Vec<&str> = list.iter().filter(|c| !c.automatic).map(|c| c.name.as_str()).collect();
            let names = if explicit.is_empty() { list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>() } else { explicit };
            let shown = if names.len() > 3 { format!("{} et {} autres", names[..3].join(", "), names.len() - 3) } else { names.join(", ") };
            let deps = list.iter().filter(|c| c.automatic).count();
            if deps > 0 && list.iter().any(|c| !c.automatic) { format!("{shown} (+{deps} dépendances)") } else { shown }
        };
        let mut parts = Vec::new();
        if !self.installed.is_empty() {
            parts.push(format!("Installation : {}", main(&self.installed)));
        }
        if !self.upgraded.is_empty() {
            parts.push(match self.upgraded.len() {
                1 => format!("Mise à jour : {}", self.upgraded[0].name),
                n => format!("Mise à jour de {n} paquets"),
            });
        }
        if !self.removed.is_empty() {
            parts.push(format!("Suppression : {}", main(&self.removed)));
        }
        if parts.is_empty() { self.command.clone() } else { parts.join(" · ") }
    }

    /// Annulation simple d'une installation : retirer ce qu'elle a ajouté (s'il est encore là).
    /// Les mises à jour et suppressions s'annulent par l'instantané.
    pub fn undo_plan(&self, still_installed: &dyn Fn(&str) -> bool) -> Option<Plan> {
        if self.installed.is_empty() || !self.upgraded.is_empty() || !self.removed.is_empty() {
            return None;
        }
        let names: Vec<String> = self.installed.iter().map(|c| c.name.clone()).filter(|n| still_installed(n)).collect();
        if names.is_empty() {
            return None;
        }
        match self.backend {
            Some(BackendKind::Apt) => {
                let mut argv = apt_root(&["remove"]);
                argv.extend(names);
                Some(Plan::new().root_owned(argv))
            }
            Some(BackendKind::Flatpak) => {
                let mut argv: Vec<String> = vec!["flatpak".into(), "uninstall".into(), "--noninteractive".into(), "-y".into()];
                argv.extend(names);
                Some(Plan::new().user_owned(argv))
            }
            None => None,
        }
    }
}

/// Jours depuis 1970 d'une date grégorienne (algorithme de H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// « 2026-11-18  16:46:59 » ou « 2026-11-18 16:46:58 » → secondes.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let mut it = s.split_whitespace();
    let date: Vec<i64> = it.next()?.split('-').filter_map(|x| x.parse().ok()).collect();
    let time: Vec<i64> = it.next().unwrap_or("00:00:00").split(':').filter_map(|x| x.parse().ok()).collect();
    if date.len() != 3 {
        return None;
    }
    let (h, mi, se) = (*time.first().unwrap_or(&0), *time.get(1).unwrap_or(&0), *time.get(2).unwrap_or(&0));
    Some(days_from_civil(date[0], date[1], date[2]) * 86400 + h * 3600 + mi * 60 + se)
}

/// « gimp:amd64 (3.2.8-1, automatic), libfoo:amd64 (1.0, 1.1) » → modifications.
pub fn parse_changes(s: &str, upgrade: bool) -> Vec<Change> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let Some(open) = rest.find(" (") else { break };
        let Some(close) = rest[open..].find(')').map(|c| c + open) else { break };
        let name = rest[..open].trim().trim_start_matches(',').trim();
        let inner: Vec<&str> = rest[open + 2..close].split(", ").collect();
        let name = name.split(':').next().unwrap_or(name).to_string();
        let automatic = inner.contains(&"automatic");
        let versions: Vec<&str> = inner.iter().copied().filter(|v| *v != "automatic").collect();
        let (old, version) = if upgrade && versions.len() >= 2 {
            (Some(versions[0].to_string()), versions[1].to_string())
        } else {
            (None, versions.first().unwrap_or(&"").to_string())
        };
        out.push(Change { name, version, old, automatic });
        rest = rest[close + 1..].trim_start_matches(',').trim_start();
    }
    out
}

/// Journal `history.log` d'apt.
pub fn parse_apt_history(text: &str) -> Vec<Operation> {
    let mut out = Vec::new();
    let mut cur: Option<Operation> = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(": ") else {
            if line.trim().is_empty() {
                if let Some(op) = cur.take() {
                    out.push(op);
                }
            }
            continue;
        };
        match k {
            "Start-Date" => {
                if let Some(op) = cur.take() {
                    out.push(op);
                }
                let ts = parse_datetime(v).unwrap_or(0);
                let date = v.split_whitespace().collect::<Vec<_>>();
                cur = Some(Operation {
                    backend: Some(BackendKind::Apt),
                    timestamp: ts,
                    date: format!("{} {}", date.first().unwrap_or(&""), date.get(1).map(|t| &t[..t.len().min(5)]).unwrap_or("")),
                    success: true,
                    ..Default::default()
                });
            }
            _ => {
                let Some(op) = cur.as_mut() else { continue };
                match k {
                    "Commandline" => op.command = v.trim().to_string(),
                    "Requested-By" => op.requested_by = v.split(" (").next().unwrap_or(v).trim().to_string(),
                    "Install" | "Reinstall" => op.installed.extend(parse_changes(v, false)),
                    "Upgrade" | "Downgrade" => op.upgraded.extend(parse_changes(v, true)),
                    "Remove" | "Purge" => op.removed.extend(parse_changes(v, false)),
                    "Error" => op.success = false,
                    _ => {}
                }
            }
        }
    }
    if let Some(op) = cur {
        out.push(op);
    }
    out
}

/// Instantané snapper (sortie `snapper --jsonout -c root list`).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub number: u32,
    pub timestamp: i64,
    pub description: String,
    pub pre: bool,
    /// Créé par noos-save (`userdata.noos`).
    pub noos: bool,
}

pub fn parse_snapper(json: &str) -> Vec<Snapshot> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return vec![] };
    v.get("root")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(Snapshot {
                number: s["number"].as_u64()? as u32,
                timestamp: parse_datetime(s["date"].as_str()?)?,
                description: s["description"].as_str().unwrap_or("").to_string(),
                pre: s["type"].as_str() == Some("pre"),
                noos: s["userdata"].get("noos").is_some(),
            })
        })
        .collect()
}

/// Instantané « avant » d'une opération : un `pre` pris dans les 2 minutes qui précèdent son
/// début (celui de noos-save de préférence).
pub fn match_snapshot(op: &Operation, snaps: &[Snapshot]) -> Option<u32> {
    snaps
        .iter()
        .filter(|s| s.pre && s.timestamp <= op.timestamp + 2 && s.timestamp >= op.timestamp - 120)
        .max_by_key(|s| (s.noos, s.description.len() > 3, s.timestamp))
        .map(|s| s.number)
}

/// Opération enregistrée par Logiciels (Flatpak surtout : apt a son propre journal).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub timestamp: i64,
    pub date: String,
    pub title: String,
    pub backend: String,
    pub success: bool,
    #[serde(default)]
    pub installed: Vec<String>,
}

pub fn journal_path() -> PathBuf {
    noos_core::paths::home().join(".local/state/noos/software-history.json")
}

/// Heure locale actuelle (secondes « locales » comme celles des journaux) et date lisible.
pub fn now_local() -> (i64, String) {
    let out = capture_codes(&["date", "+%Y-%m-%d %H:%M:%S"], &[0]).unwrap_or_default();
    let ts = parse_datetime(out.trim()).unwrap_or(0);
    (ts, out.trim().chars().take(16).collect())
}

/// Ajoute une opération au journal de Logiciels (500 dernières).
pub fn record(title: &str, backend: BackendKind, success: bool, installed: &[String]) {
    let path = journal_path();
    let mut list: Vec<Record> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let (timestamp, date) = now_local();
    list.push(Record { timestamp, date, title: title.into(), backend: backend.id().into(), success, installed: installed.to_vec() });
    if list.len() > 500 {
        list.drain(..list.len() - 500);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_vec_pretty(&list).unwrap_or_default());
}

/// Historique complet, le plus récent d'abord.
pub fn load(limit: usize) -> Vec<Operation> {
    let mut ops: Vec<Operation> = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir("/var/log/apt")
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("history.log"))).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        let text = if f.extension().is_some_and(|e| e == "gz") {
            capture_codes(&["gzip", "-dc", &f.to_string_lossy()], &[0]).unwrap_or_default()
        } else {
            std::fs::read_to_string(&f).unwrap_or_default()
        };
        ops.extend(parse_apt_history(&text));
    }
    // Journal de Logiciels : seulement ce qu'apt n'a pas déjà noté.
    let journal: Vec<Record> = std::fs::read(journal_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    for r in journal.into_iter().filter(|r| r.backend != "apt") {
        ops.push(Operation {
            backend: BackendKind::parse(&r.backend),
            timestamp: r.timestamp,
            date: r.date,
            title: r.title,
            success: r.success,
            installed: r.installed.into_iter().map(|n| Change { name: n, ..Default::default() }).collect(),
            ..Default::default()
        });
    }
    let snaps = capture_codes(&["snapper", "--jsonout", "-c", "root", "list", "--disable-used-space"], &[0]).map(|o| parse_snapper(&o)).unwrap_or_default();
    for op in ops.iter_mut().filter(|o| o.backend == Some(BackendKind::Apt)) {
        op.snapshot = match_snapshot(op, &snaps);
    }
    ops.sort_by_key(|o| std::cmp::Reverse(o.timestamp));
    ops.truncate(limit);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\nStart-Date: 2026-11-18  03:34:37\nCommandline: apt install adb\nRequested-By: chomiam (1000)\n\
Install: android-liblog:amd64 (1:34.0.5-13, automatic), adb:amd64 (1:34.0.5-13), android-libbase:amd64 (1:34.0.5-13, automatic)\nEnd-Date: 2026-11-18  03:34:41\n\n\
Start-Date: 2026-11-18  16:46:59\nCommandline: apt-get -y full-upgrade\nRequested-By: chomiam (1000)\n\
Upgrade: noos:amd64 (0.1.0~git1, 0.1.0~git2), libgd3:amd64 (2.3.3-16, 2.3.3-17)\nRemove: oldpkg:amd64 (1.0)\nEnd-Date: 2026-11-18  16:47:05\n";

    #[test]
    fn journal_apt() {
        let ops = parse_apt_history(LOG);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].date, "2026-11-18 03:34");
        assert_eq!(ops[0].installed.len(), 3);
        assert_eq!(ops[0].installed[1], Change { name: "adb".into(), version: "1:34.0.5-13".into(), old: None, automatic: false });
        assert_eq!(ops[0].summary(), "Installation : adb (+2 dépendances)");
        assert_eq!(ops[1].upgraded[0].old.as_deref(), Some("0.1.0~git1"));
        assert_eq!(ops[1].upgraded[1].version, "2.3.3-17");
        assert_eq!(ops[1].summary(), "Mise à jour de 2 paquets · Suppression : oldpkg");
        let undo = ops[0].undo_plan(&|n| n != "android-libbase").unwrap();
        let d = undo.describe().join(" ");
        assert!(d.contains("remove android-liblog adb") && !d.contains("android-libbase"), "{d}");
        assert!(ops[1].undo_plan(&|_| true).is_none());
    }

    #[test]
    fn dates_et_instantanes() {
        assert_eq!(parse_datetime("1970-01-02 00:00:01"), Some(86401));
        assert_eq!(parse_datetime("2026-11-18  16:46:59").unwrap() - parse_datetime("2026-11-18 16:46:58").unwrap(), 1);
        let json = r#"{"root":[{"number":0,"type":"single","pre-number":null,"date":"","description":"current","userdata":null},
            {"number":111,"type":"pre","pre-number":null,"date":"2026-11-18 16:46:58","description":"apt-get -y full-upgrade","userdata":{"noos":"apt"}},
            {"number":112,"type":"pre","pre-number":null,"date":"2026-11-18 16:46:58","description":"apt","userdata":null},
            {"number":113,"type":"post","pre-number":111,"date":"2026-11-18 16:47:06","description":"apt-get","userdata":{"noos":"apt"}}]}"#;
        let snaps = parse_snapper(json);
        assert_eq!(snaps.len(), 3);
        let ops = parse_apt_history(LOG);
        assert_eq!(match_snapshot(&ops[1], &snaps), Some(111));
        assert_eq!(match_snapshot(&ops[0], &snaps), None);
    }
}

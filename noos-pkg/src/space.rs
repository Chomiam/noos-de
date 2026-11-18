//! Place occupée par les logiciels installés : taille installée (dpkg `Installed-Size`,
//! `flatpak list --columns=size`) et données de l'utilisateur (`~/.var/app/<id>` pour Flatpak,
//! `~/.config|.local/share|.cache/<paquet>` pour apt quand ces dossiers existent).

use std::{
    collections::HashMap,
    path::PathBuf,
};

use crate::{BackendKind, Plan, cleanup::dir_size, details::parse_size, exec::capture_codes};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sizes {
    pub installed: u64,
    pub data: u64,
    pub data_dirs: Vec<PathBuf>,
}

/// Clé d'un logiciel : « apt:gimp », « flatpak:org.gimp.GIMP ».
pub fn key(kind: BackendKind, id: &str) -> String {
    format!("{}:{id}", kind.id())
}

/// `dpkg-query -W -f='${binary:Package}\t${Installed-Size}\n'` → paquet → octets.
pub fn parse_dpkg_sizes(out: &str) -> HashMap<String, u64> {
    out.lines()
        .filter_map(|l| {
            let (n, s) = l.split_once('\t')?;
            Some((n.split(':').next().unwrap_or(n).to_string(), s.trim().parse::<u64>().ok()? * 1024))
        })
        .collect()
}

/// `flatpak list --app --columns=application,size` → appli → octets.
pub fn parse_flatpak_sizes(out: &str) -> HashMap<String, u64> {
    out.lines().filter_map(|l| {
        let mut c = l.split('\t');
        let id = c.next()?.trim();
        Some((id.to_string(), parse_size(c.next()?)?))
    }).collect()
}

/// Dossiers de données d'un logiciel (seulement ceux qui existent).
pub fn data_dirs(kind: BackendKind, id: &str) -> Vec<PathBuf> {
    let home = noos_core::paths::home();
    let cands: Vec<PathBuf> = match kind {
        BackendKind::Flatpak => vec![home.join(".var/app").join(id)],
        BackendKind::Apt => [".config", ".local/share", ".cache"].iter().map(|d| home.join(d).join(id)).collect(),
    };
    cands.into_iter().filter(|p| p.is_dir()).collect()
}

/// Tailles de logiciels installés (lent : parcourt les dossiers de données).
pub fn sizes(items: &[(BackendKind, String)]) -> HashMap<String, Sizes> {
    let apt = capture_codes(&["dpkg-query", "-W", "-f=${binary:Package}\t${Installed-Size}\n"], &[0, 1]).map(|o| parse_dpkg_sizes(&o)).unwrap_or_default();
    let fp = capture_codes(&["flatpak", "list", "--app", "--columns=application,size"], &[0]).map(|o| parse_flatpak_sizes(&o)).unwrap_or_default();
    items
        .iter()
        .map(|(k, id)| {
            let dirs = data_dirs(*k, id);
            let installed = match k {
                BackendKind::Apt => apt.get(id).copied().unwrap_or(0),
                BackendKind::Flatpak => fp.get(id).copied().unwrap_or(0),
            };
            (key(*k, id), Sizes { installed, data: dirs.iter().map(|d| dir_size(d)).sum(), data_dirs: dirs })
        })
        .collect()
}

/// Effacer les données d'une appli : dossiers mis à la corbeille (récupérables).
pub fn clear_data_plan(dirs: &[PathBuf]) -> Plan {
    let mut plan = Plan::new();
    for d in dirs {
        plan = plan.user_owned(vec!["gio".into(), "trash".into(), d.to_string_lossy().into_owned()]);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse() {
        let d = parse_dpkg_sizes("gimp\t33386\nlibfoo1:i386\t12\nbad\t\n");
        assert_eq!(d["gimp"], 33386 * 1024);
        assert_eq!(d["libfoo1"], 12 * 1024);
        assert!(!d.contains_key("bad"));
        let f = parse_flatpak_sizes("app.fluxer.Fluxer\t431.4 MB\norg.onlyoffice.desktopeditors\t1.1 GB\n");
        assert_eq!(f["app.fluxer.Fluxer"], 431_400_000);
        assert_eq!(f["org.onlyoffice.desktopeditors"], 1_100_000_000);
        assert_eq!(key(BackendKind::Flatpak, "x"), "flatpak:x");
        assert!(clear_data_plan(&[PathBuf::from("/home/a/.var/app/x")]).describe().join(" ").contains("gio trash /home/a/.var/app/x"));
    }
}

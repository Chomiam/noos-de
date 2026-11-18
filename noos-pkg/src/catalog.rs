//! Logiciels : un même logiciel peut exister en paquet Debian (apt) et en Flatpak. Les
//! résultats des deux backends sont rapprochés par leur **identifiant AppStream** :
//! le paquet `gimp` déclare `ID: org.gimp.GIMP` dans les données DEP-11 de Debian et l'appli Flatpak s'appelle
//! `org.gimp.GIMP` → une seule ligne « GIMP » avec deux façons de l'installer.

use std::collections::HashMap;

use crate::{BackendKind, Package};

#[derive(Debug, Clone, PartialEq)]
pub struct Software {
    /// Identifiant AppStream (minuscules) ou, à défaut, `apt:<nom>`.
    pub key: String,
    pub name: String,
    pub summary: String,
    /// Façons de l'installer, Flatpak d'abord (isolé, toujours à jour), puis apt.
    pub offers: Vec<Package>,
}

impl Software {
    pub fn installed(&self) -> impl Iterator<Item = &Package> {
        self.offers.iter().filter(|p| p.installed)
    }

    pub fn is_installed(&self) -> bool {
        self.offers.iter().any(|p| p.installed)
    }

    pub fn has(&self, kind: BackendKind) -> bool {
        self.offers.iter().any(|p| p.backend == Some(kind))
    }
}

/// Paquets de dépôts tiers sans données AppStream (DEP-11) : identifiant de l'appli Flatpak
/// correspondante, pour une seule fiche avec les deux façons de l'installer.
const KNOWN_APPSTREAM: &[(&str, &str)] = &[("google-chrome-stable", "com.google.chrome")];

fn key_of(p: &Package, appstream: &HashMap<String, String>) -> String {
    match p.backend {
        Some(BackendKind::Flatpak) => p.id.to_lowercase(),
        _ => appstream
            .get(&p.id)
            .cloned()
            .or_else(|| KNOWN_APPSTREAM.iter().find(|(pkg, _)| *pkg == p.id).map(|(_, id)| id.to_string()))
            .unwrap_or_else(|| format!("apt:{}", p.id)),
    }
}

/// Regroupe des paquets (déjà classés par pertinence) en logiciels, dans le même ordre.
/// `appstream` : nom de paquet apt → identifiant AppStream (voir `Backend::appstream_ids`).
pub fn merge(packages: Vec<Package>, appstream: &HashMap<String, String>) -> Vec<Software> {
    let mut out: Vec<Software> = Vec::new();
    for p in packages {
        let key = key_of(&p, appstream);
        match out.iter_mut().find(|s| s.key == key) {
            Some(s) => {
                // Le nom et la description de l'appli Flatpak sont les plus lisibles.
                if p.backend == Some(BackendKind::Flatpak) {
                    s.name = p.name.clone();
                    if !p.summary.is_empty() {
                        s.summary = p.summary.clone();
                    }
                }
                // Un même Flatpak peut venir de deux remotes (Flathub, autre remote) : on garde les deux.
                if !s.offers.iter().any(|o| o.backend == p.backend && o.id == p.id && o.origin == p.origin) {
                    s.offers.push(p);
                }
            }
            None => out.push(Software { key, name: p.name.clone(), summary: p.summary.clone(), offers: vec![p] }),
        }
    }
    for s in &mut out {
        s.offers.sort_by_key(|o| (o.backend != Some(BackendKind::Flatpak), o.origin != "flathub"));
    }
    out
}

/// Un résultat de recherche : le logiciel et ses paquets associés (aide, traductions,
/// données, greffons…) repliés sous lui.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub software: Software,
    pub related: Vec<Software>,
}

/// Le logiciel est-il une application (AppStream ou Flatpak), pas un simple paquet ?
pub fn is_app(sw: &Software) -> bool {
    !sw.key.starts_with("apt:")
}

/// Marqueurs de paquets associés : `gimp-help-fr`, `libreoffice-l10n-fr`, `gimp-data`…
const RELATED: &[&str] = &[
    "-help", "-l10n", "-locale", "-i18n", "-lang", "-data", "-common", "-doc", "-dbg", "-dev", "-plugin", "-extra", "-examples", "-theme",
    "-dict", "-hyph", "-mythes",
];

/// Paquet de base d'un paquet associé (`gimp-help-fr` → `gimp`), s'il ressemble à un associé.
pub fn related_base(id: &str) -> Option<&str> {
    RELATED.iter().filter_map(|m| id.find(m).filter(|p| *p > 0).map(|p| &id[..p])).min_by_key(|b| b.len())
}

/// Organise des logiciels (déjà fusionnés) : pertinence du nom, applications avant les simples
/// paquets à pertinence égale, paquets associés repliés sous leur logiciel de base.
pub fn organize(query: &str, software: Vec<Software>) -> Vec<Entry> {
    let apt_ids: std::collections::HashSet<String> =
        software.iter().flat_map(|s| s.offers.iter().filter(|o| o.backend == Some(BackendKind::Apt)).map(|o| o.id.clone())).collect();
    let mut entries: Vec<Entry> = Vec::new();
    let mut later: Vec<(String, Software)> = Vec::new();
    for sw in software {
        let only_apt = sw.offers.iter().all(|o| o.backend == Some(BackendKind::Apt));
        let base = if only_apt && sw.offers.len() == 1 { related_base(&sw.offers[0].id).filter(|b| apt_ids.contains(*b)).map(str::to_string) } else { None };
        match base {
            Some(b) => later.push((b, sw)),
            None => entries.push(Entry { software: sw, related: vec![] }),
        }
    }
    for (base, sw) in later {
        match entries.iter_mut().find(|e| e.software.offers.iter().any(|o| o.backend == Some(BackendKind::Apt) && o.id == base)) {
            Some(e) => e.related.push(sw),
            None => entries.push(Entry { software: sw, related: vec![] }),
        }
    }
    let score = |sw: &Software| {
        std::iter::once(crate::match_score(query, &sw.name, &sw.name))
            .chain(sw.offers.iter().map(|o| crate::match_score(query, &o.id, &o.name)))
            .min()
            .unwrap_or(3)
    };
    entries.sort_by_key(|e| (score(&e.software), !is_app(&e.software)));
    entries
}

/// Compare deux versions (« 1:3.2.8-1+b1 », « 3.2.8 », « 2026.1114 ») sur leurs nombres, sans
/// l'époque ni la révision Debian.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let core = |v: &str| -> Vec<u64> {
        let v = v.split_once(':').map(|x| x.1).unwrap_or(v);
        let v = v.rsplit_once('-').filter(|(u, _)| !u.is_empty()).map(|x| x.0).unwrap_or(v);
        let v = v.split(['+', '~']).next().unwrap_or(v);
        v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
    };
    core(a).cmp(&core(b))
}

/// Façon d'installer conseillée parmi des offres (backend, version) : la plus récente ; à
/// égalité, le paquet du système (intégré, mis à jour avec Debian).
pub fn recommend(offers: &[(BackendKind, String)]) -> Option<usize> {
    (0..offers.len()).max_by(|&i, &j| {
        compare_versions(&offers[i].1, &offers[j].1).then_with(|| (offers[i].0 == BackendKind::Apt).cmp(&(offers[j].0 == BackendKind::Apt)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(kind: BackendKind, id: &str, name: &str, origin: &str) -> Package {
        Package { backend: Some(kind), id: id.into(), name: name.into(), origin: origin.into(), ..Default::default() }
    }

    #[test]
    fn gimp_both_ways() {
        let appstream: HashMap<String, String> = [("gimp".to_string(), "org.gimp.gimp".to_string())].into();
        let found = vec![
            pkg(BackendKind::Apt, "gimp", "gimp", "debian"),
            pkg(BackendKind::Apt, "gimp-devel", "gimp-devel", "debian"),
            pkg(BackendKind::Flatpak, "org.gimp.GIMP", "GIMP", "flathub"),
            pkg(BackendKind::Flatpak, "org.gimp.GIMP", "GIMP", "debian"),
        ];
        let s = merge(found, &appstream);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "GIMP");
        assert_eq!(s[0].offers.len(), 3);
        // Flathub, puis le Flatpak d'un autre remote, puis le paquet apt.
        assert_eq!(s[0].offers[0].origin, "flathub");
        assert_eq!(s[0].offers[2].backend, Some(BackendKind::Apt));
        assert_eq!(s[1].key, "apt:gimp-devel");
    }

    #[test]
    fn associes_et_applications() {
        let appstream: HashMap<String, String> = [("gimp".to_string(), "org.gimp.gimp".to_string())].into();
        let found = vec![
            pkg(BackendKind::Apt, "gimp", "gimp", "debian"),
            pkg(BackendKind::Apt, "gimp-data", "gimp-data", "debian"),
            pkg(BackendKind::Apt, "gimp-help-fr", "gimp-help-fr", "debian"),
            pkg(BackendKind::Apt, "gimplensfun", "gimplensfun", "debian"),
            pkg(BackendKind::Flatpak, "org.gimp.GIMP", "GIMP", "flathub"),
            pkg(BackendKind::Apt, "photo-help-x", "photo-help-x", "debian"),
        ];
        let e = organize("gimp", merge(found, &appstream));
        assert_eq!(e[0].software.name, "GIMP");
        assert_eq!(e[0].related.iter().map(|s| s.offers[0].id.as_str()).collect::<Vec<_>>(), ["gimp-data", "gimp-help-fr"]);
        assert_eq!(e[1].software.key, "apt:gimplensfun");
        // Base absente des résultats : reste une ligne à part.
        assert_eq!(e[2].software.key, "apt:photo-help-x");
        assert_eq!(related_base("libreoffice-l10n-fr"), Some("libreoffice"));
        assert_eq!(related_base("htop"), None);
    }

    #[test]
    fn versions_et_conseil() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("1:3.2.8-1+b1", "3.2.8"), Equal);
        assert_eq!(compare_versions("3.2.6-1", "3.2.8"), Less);
        assert_eq!(compare_versions("4:26.08.2-1", "26.08.3"), Less);
        let o = vec![(BackendKind::Flatpak, "3.2.8".to_string()), (BackendKind::Apt, "3.2.8-1+b1".to_string())];
        assert_eq!(recommend(&o), Some(1));
        let o = vec![(BackendKind::Flatpak, "3.4.0".to_string()), (BackendKind::Apt, "3.2.8-1".to_string())];
        assert_eq!(recommend(&o), Some(0));
    }

    #[test]
    fn chrome_depot_google_et_flathub() {
        let found = vec![
            pkg(BackendKind::Flatpak, "com.google.Chrome", "Google Chrome", "flathub"),
            pkg(BackendKind::Apt, "google-chrome-beta", "google-chrome-beta", "dl.google.com"),
            pkg(BackendKind::Apt, "google-chrome-stable", "google-chrome-stable", "dl.google.com"),
        ];
        let s = merge(found, &HashMap::new());
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].name.as_str(), s[0].offers.len()), ("Google Chrome", 2));
        assert_eq!(s[0].offers[1].id, "google-chrome-stable");
    }
}

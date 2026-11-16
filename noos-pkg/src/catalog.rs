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

//! Logiciels : un même logiciel peut exister en paquet Fedora (dnf) et en Flatpak. Les
//! résultats des deux backends sont rapprochés par leur **identifiant AppStream** :
//! `gimp` fournit `metainfo(org.gimp.GIMP.appdata.xml)` et l'appli Flatpak s'appelle
//! `org.gimp.GIMP` → une seule ligne « GIMP » avec deux façons de l'installer.

use std::collections::HashMap;

use crate::{BackendKind, Package};

#[derive(Debug, Clone, PartialEq)]
pub struct Software {
    /// Identifiant AppStream (minuscules) ou, à défaut, `dnf:<nom>`.
    pub key: String,
    pub name: String,
    pub summary: String,
    /// Façons de l'installer, Flatpak d'abord (isolé, toujours à jour), puis dnf.
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

fn key_of(p: &Package, appstream: &HashMap<String, String>) -> String {
    match p.backend {
        Some(BackendKind::Flatpak) => p.id.to_lowercase(),
        _ => appstream.get(&p.id).cloned().unwrap_or_else(|| format!("dnf:{}", p.id)),
    }
}

/// Regroupe des paquets (déjà classés par pertinence) en logiciels, dans le même ordre.
/// `appstream` : nom de paquet dnf → identifiant AppStream (voir `Backend::appstream_ids`).
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
                // Un même Flatpak peut venir de deux remotes (Flathub, Fedora) : on garde les deux.
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
            pkg(BackendKind::Dnf, "gimp", "gimp", "fedora"),
            pkg(BackendKind::Dnf, "gimp-devel", "gimp-devel", "fedora"),
            pkg(BackendKind::Flatpak, "org.gimp.GIMP", "GIMP", "flathub"),
            pkg(BackendKind::Flatpak, "org.gimp.GIMP", "GIMP", "fedora"),
        ];
        let s = merge(found, &appstream);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "GIMP");
        assert_eq!(s[0].offers.len(), 3);
        // Flathub, puis Fedora Flatpak, puis le paquet dnf.
        assert_eq!(s[0].offers[0].origin, "flathub");
        assert_eq!(s[0].offers[2].backend, Some(BackendKind::Dnf));
        assert_eq!(s[1].key, "dnf:gimp-devel");
    }
}

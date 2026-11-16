//! Système : NOOS est basé sur Debian sid ; on ne lit que le nom et la version
//! (`/etc/os-release`). Debian sid n'a pas de `VERSION_ID` : la suite vient de
//! `VERSION_CODENAME` (« forky ») et du nom (« forky/sid »).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub id: String,
    pub name: String,
    pub version_id: String,
    #[serde(default)]
    pub codename: String,
}

impl SystemInfo {
    pub fn detect() -> Self {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        Self::parse(&text)
    }

    pub fn parse(os_release: &str) -> Self {
        let get = |key: &str| {
            os_release
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .map(|v| v.trim().trim_matches('"').to_string())
                .unwrap_or_default()
        };
        Self { id: get("ID"), name: get("PRETTY_NAME"), version_id: get("VERSION_ID"), codename: get("VERSION_CODENAME") }
    }

    /// Version majeure (« 13 ») ; vide sur Debian sid.
    pub fn major(&self) -> String {
        self.version_id.split('.').next().unwrap_or("").to_string()
    }

    /// Debian unstable (sid) : pas de numéro de version, « …/sid » dans le nom.
    pub fn is_sid(&self) -> bool {
        self.version_id.is_empty() && (self.name.contains("sid") || self.codename == "sid" || self.id == "debian")
    }

    /// Version lisible : « sid », « 13 (trixie) »…
    pub fn release_label(&self) -> String {
        if self.is_sid() {
            "sid".into()
        } else if self.codename.is_empty() {
            self.major()
        } else {
            format!("{} ({})", self.major(), self.codename)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_debian_sid() {
        let s = SystemInfo::parse("PRETTY_NAME=\"Debian GNU/Linux forky/sid\"
NAME=\"Debian GNU/Linux\"
VERSION_CODENAME=forky
ID=debian
");
        assert_eq!(s.id, "debian");
        assert!(s.is_sid());
        assert_eq!(s.release_label(), "sid");
        assert_eq!(s.name, "Debian GNU/Linux forky/sid");
    }

    #[test]
    fn parse_debian_stable() {
        let s = SystemInfo::parse("PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"
VERSION_ID=\"13\"
VERSION_CODENAME=trixie
ID=debian
");
        assert!(!s.is_sid());
        assert_eq!(s.release_label(), "13 (trixie)");
    }
}

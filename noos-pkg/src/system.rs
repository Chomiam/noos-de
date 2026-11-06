//! Détection de la distribution (`/etc/os-release`) et du mode atomique (ostree).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub id: String,
    pub id_like: Vec<String>,
    pub name: String,
    pub version_id: String,
    pub version_codename: String,
    /// Système immuable piloté par rpm-ostree / bootc.
    pub atomic: bool,
}

impl SystemInfo {
    pub fn detect() -> Self {
        let text = std::fs::read_to_string("/etc/os-release")
            .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
            .unwrap_or_default();
        Self::parse(&text, std::path::Path::new("/run/ostree-booted").exists())
    }

    pub fn parse(os_release: &str, atomic: bool) -> Self {
        let get = |key: &str| {
            os_release
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .map(|v| v.trim().trim_matches('"').to_string())
                .unwrap_or_default()
        };
        Self {
            id: get("ID"),
            id_like: get("ID_LIKE").split_whitespace().map(String::from).collect(),
            name: get("PRETTY_NAME"),
            version_id: get("VERSION_ID"),
            version_codename: {
                let c = get("VERSION_CODENAME");
                if c.is_empty() { get("UBUNTU_CODENAME") } else { c }
            },
            atomic,
        }
    }

    pub fn is(&self, id: &str) -> bool {
        self.id == id || self.id_like.iter().any(|l| l == id)
    }

    /// Fedora « pure » (pas RHEL/Alma/Rocky).
    pub fn is_fedora(&self) -> bool {
        self.id == "fedora" || (self.is("fedora") && !self.is("rhel"))
    }

    pub fn is_enterprise_linux(&self) -> bool {
        self.is("rhel") || self.is("centos")
    }

    pub fn is_ubuntu(&self) -> bool {
        self.is("ubuntu")
    }

    /// Version majeure (« 45 », « 9 »…).
    pub fn major(&self) -> String {
        self.version_id.split('.').next().unwrap_or("").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mint() {
        let s = SystemInfo::parse(
            "NAME=\"Linux Mint\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\nVERSION_ID=\"22.1\"\nUBUNTU_CODENAME=noble\n",
            false,
        );
        assert!(s.is_ubuntu());
        assert!(s.is("debian"));
        assert_eq!(s.version_codename, "noble");
    }

    #[test]
    fn parse_alma() {
        let s = SystemInfo::parse("ID=\"almalinux\"\nID_LIKE=\"rhel centos fedora\"\nVERSION_ID=\"9.5\"\n", false);
        assert!(s.is_enterprise_linux());
        assert!(!s.is_fedora());
        assert_eq!(s.major(), "9");
    }
}

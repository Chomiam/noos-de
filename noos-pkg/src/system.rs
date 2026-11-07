//! Système : NOOS est basé sur Fedora ; on ne lit que le nom et la version (`/etc/os-release`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SystemInfo {
    pub id: String,
    pub name: String,
    pub version_id: String,
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
        Self { id: get("ID"), name: get("PRETTY_NAME"), version_id: get("VERSION_ID") }
    }

    /// Version majeure de Fedora (« 45 »).
    pub fn major(&self) -> String {
        self.version_id.split('.').next().unwrap_or("").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fedora() {
        let s = SystemInfo::parse("NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=45\nPRETTY_NAME=\"Fedora Linux 45\"\n");
        assert_eq!(s.id, "fedora");
        assert_eq!(s.major(), "45");
        assert_eq!(s.name, "Fedora Linux 45");
    }
}

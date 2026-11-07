//! Sources lisibles : les dépôts bruts (24 sections dnf pour Fedora + RPM Fusion…) sont
//! regroupés par famille, avec un seul interrupteur, et séparés en **stables** et **tests**.
//!
//! | Dépôts bruts                                          | Source                          |
//! |-------------------------------------------------------|---------------------------------|
//! | `fedora`, `updates`                                   | Fedora (stable)                 |
//! | `updates-testing`                                     | Fedora — tests                  |
//! | `rpmfusion-free`, `rpmfusion-free-updates`            | RPM Fusion — libre (stable)     |
//! | `rpmfusion-free-updates-testing`                      | RPM Fusion — libre — tests      |
//! | `*-debuginfo`, `*-source`                             | masqués (outils de développeur) |
//! | remote Flatpak `flathub` / `flathub-beta`             | Flathub / Flathub — tests       |
//!
//! Les sources connues absentes (RPM Fusion, Flathub) apparaissent aussi, « non installées »,
//! avec le préréglage qui les ajoute : on voit tout de suite ce qui est actif ou non.

use crate::{BackendKind, Repo, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Stable,
    Testing,
}

/// État d'une source pour l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Enabled,
    Disabled,
    /// Une partie seulement des dépôts de la source est active.
    Partial,
    /// Source connue mais absente : `preset` permet de l'ajouter.
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub backend: BackendKind,
    /// Clé stable de la famille (`dnf:rpmfusion-free`, `flatpak:system:flathub`…).
    pub key: String,
    pub name: String,
    pub description: String,
    pub channel: Channel,
    pub state: State,
    /// Dépôts réels de la source (sans débogage ni sources).
    pub repos: Vec<Repo>,
    /// Fournie par Fedora / NOOS : ne peut pas être supprimée (seulement désactivée).
    pub builtin: bool,
    /// Ne doit pas être désactivée (dépôts principaux de Fedora, NOOS).
    pub essential: bool,
    /// Préréglage à utiliser quand la source est absente.
    pub preset: Option<&'static str>,
}

impl Source {
    pub fn enabled(&self) -> bool {
        matches!(self.state, State::Enabled | State::Partial)
    }
}

/// Dépôt technique (symboles de débogage, paquets sources) : masqué.
pub fn is_technical(r: &Repo) -> bool {
    ["-debuginfo", "-source", "-debug"].iter().any(|s| r.id.ends_with(s))
}

/// (famille, canal) d'un dépôt dnf : `rpmfusion-free-updates-testing` → (`rpmfusion-free`, tests).
fn dnf_family(id: &str) -> (String, Channel) {
    let (base, channel) = if let Some(b) = id.strip_suffix("updates-testing") {
        (b.trim_end_matches('-'), Channel::Testing)
    } else if let Some(b) = id.strip_suffix("-testing") {
        (b, Channel::Testing)
    } else if let Some(b) = id.strip_suffix("updates") {
        (b.trim_end_matches('-'), Channel::Stable)
    } else {
        (id, Channel::Stable)
    };
    // `updates` / `updates-testing` appartiennent à Fedora.
    let base = if base.is_empty() { "fedora" } else { base };
    (base.to_string(), channel)
}

/// (nom, description, essentielle, intégrée) d'une famille dnf connue.
fn dnf_label(family: &str, sample: &Repo, version: &str) -> (String, String, bool, bool) {
    let copr = family.strip_prefix("copr:copr.fedorainfracloud.org:").map(|rest| rest.replacen(':', "/", 1));
    match (family, copr.as_deref()) {
        ("fedora", _) => (
            format!("Fedora {version}"),
            "Dépôts officiels de Fedora : système, applications et mises à jour".into(),
            true,
            true,
        ),
        ("fedora-cisco-openh264", _) => (
            "Codec OpenH264 (Cisco)".into(),
            "Codec vidéo H.264 distribué par Cisco pour Fedora".into(),
            false,
            true,
        ),
        ("rpmfusion-free", _) => (
            "RPM Fusion — libre".into(),
            "Codecs et logiciels libres absents de Fedora (ffmpeg complet, VLC…)".into(),
            false,
            false,
        ),
        ("rpmfusion-nonfree", _) => (
            "RPM Fusion — non libre".into(),
            "Pilotes NVIDIA, Steam, firmwares et codecs propriétaires".into(),
            false,
            false,
        ),
        ("rpmfusion-free-tainted" | "rpmfusion-nonfree-tainted", _) => (
            format!("RPM Fusion — {}", if family.contains("nonfree") { "non libre (tainted)" } else { "libre (tainted)" }),
            "Paquets soumis à des restrictions légales selon les pays (libdvdcss…)".into(),
            false,
            false,
        ),
        (_, Some("chomiam/noos")) => ("NOOS".into(), "Bureau NOOS et ses applications (COPR chomiam/noos)".into(), true, true),
        (_, Some(project)) => (format!("COPR {project}"), "Dépôt communautaire Fedora (COPR)".into(), false, false),
        _ => {
            let name = sample
                .name
                .replace("$releasever", version)
                .replace("$basearch", std::env::consts::ARCH)
                .replace(" - Test Updates", "")
                .replace(" - Updates", "");
            (if name.is_empty() { family.to_string() } else { name }, sample.url.clone(), false, false)
        }
    }
}

fn state_of(repos: &[Repo]) -> State {
    match (repos.iter().all(|r| r.enabled), repos.iter().any(|r| r.enabled)) {
        (true, _) => State::Enabled,
        (false, true) => State::Partial,
        _ => State::Disabled,
    }
}

/// Regroupe les dépôts dnf en sources (stables puis tests), avec RPM Fusion même absent.
pub fn group_dnf(repos: &[Repo], version: &str) -> Vec<Source> {
    let mut families: Vec<(String, Channel, Vec<Repo>)> = Vec::new();
    for r in repos.iter().filter(|r| !is_technical(r)) {
        let (family, channel) = dnf_family(&r.id);
        match families.iter_mut().find(|(f, c, _)| *f == family && *c == channel) {
            Some(e) => e.2.push(r.clone()),
            None => families.push((family, channel, vec![r.clone()])),
        }
    }
    let mut out: Vec<Source> = families
        .into_iter()
        .map(|(family, channel, members)| {
            let (mut name, mut description, essential, builtin) = dnf_label(&family, &members[0], version);
            if channel == Channel::Testing {
                name = format!("{name} — tests");
                description = "Mises à jour en cours de validation : plus récentes, mais peuvent casser le système".into();
            }
            Source {
                backend: BackendKind::Dnf,
                key: format!("dnf:{family}:{channel:?}"),
                name,
                description,
                channel,
                state: state_of(&members),
                builtin: builtin || members.iter().any(|m| m.builtin),
                essential: essential && channel == Channel::Stable,
                repos: members,
                preset: None,
            }
        })
        .collect();
    for (family, preset, label) in [
        ("rpmfusion-free", "rpmfusion-free", "RPM Fusion — libre"),
        ("rpmfusion-nonfree", "rpmfusion-nonfree", "RPM Fusion — non libre"),
    ] {
        if !out.iter().any(|s| s.key == format!("dnf:{family}:Stable")) {
            let sample = Repo {
                backend: BackendKind::Dnf,
                id: family.into(),
                name: label.into(),
                url: String::new(),
                enabled: false,
                scope: None,
                file: String::new(),
                builtin: false,
            };
            let (name, description, _, _) = dnf_label(family, &sample, version);
            out.push(Source {
                backend: BackendKind::Dnf,
                key: format!("dnf:{family}:Stable"),
                name,
                description,
                channel: Channel::Stable,
                state: State::Missing,
                repos: vec![],
                builtin: false,
                essential: false,
                preset: Some(preset),
            });
        }
    }
    sort(&mut out);
    out
}

/// Regroupe les remotes Flatpak : `flathub-beta` → Flathub — tests ; Flathub même absent.
pub fn group_flatpak(repos: &[Repo]) -> Vec<Source> {
    let mut out: Vec<Source> = repos
        .iter()
        .map(|r| {
            let remote = r.file.as_str();
            let (base, channel) = match remote.strip_suffix("-testing").or_else(|| remote.strip_suffix("-beta")) {
                Some(b) => (b, Channel::Testing),
                None => (remote, Channel::Stable),
            };
            let scope = r.scope.unwrap_or(Scope::System);
            let (mut name, description, builtin) = match base {
                "flathub" => (
                    "Flathub".to_string(),
                    "Le principal magasin d'applications Flatpak (navigateurs, jeux, créativité…)".to_string(),
                    false,
                ),
                "fedora" => (
                    "Fedora Flatpaks".to_string(),
                    "Applications Flatpak construites par Fedora à partir de ses paquets".to_string(),
                    true,
                ),
                _ => (r.name.clone(), r.url.clone(), false),
            };
            if scope == Scope::User {
                name.push_str(" (votre compte)");
            }
            let description = if channel == Channel::Testing {
                name = format!("{name} — tests");
                "Versions bêta / en cours de validation".to_string()
            } else {
                description
            };
            let filtered = r.name.contains("filtré");
            Source {
                backend: BackendKind::Flatpak,
                key: format!("flatpak:{}:{remote}", if scope == Scope::User { "user" } else { "system" }),
                name,
                description: if filtered { format!("{description} — limité à une sélection par Fedora") } else { description },
                channel,
                state: if r.enabled { State::Enabled } else { State::Disabled },
                repos: vec![r.clone()],
                builtin,
                essential: false,
                preset: None,
            }
        })
        .collect();
    if !repos.iter().any(|r| r.file == "flathub") {
        out.push(Source {
            backend: BackendKind::Flatpak,
            key: "flatpak:system:flathub".into(),
            name: "Flathub".into(),
            description: "Le principal magasin d'applications Flatpak (navigateurs, jeux, créativité…)".into(),
            channel: Channel::Stable,
            state: State::Missing,
            repos: vec![],
            builtin: false,
            essential: false,
            preset: Some("flathub-system"),
        });
    }
    sort(&mut out);
    out
}

/// Ordre : essentielles, puis actives, puis le reste par nom.
fn sort(v: &mut [Source]) {
    v.sort_by_key(|s| (s.channel, !s.essential, s.state == State::Missing, !s.builtin, s.name.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(id: &str, enabled: bool) -> Repo {
        Repo {
            backend: BackendKind::Dnf,
            id: id.into(),
            name: id.into(),
            url: String::new(),
            enabled,
            scope: None,
            file: format!("/usr/share/dnf5/repos.d/{id}.repo"),
            builtin: id.starts_with("fedora") || id.starts_with("updates"),
        }
    }

    #[test]
    fn families() {
        assert_eq!(dnf_family("updates"), ("fedora".into(), Channel::Stable));
        assert_eq!(dnf_family("updates-testing"), ("fedora".into(), Channel::Testing));
        assert_eq!(dnf_family("rpmfusion-free-updates-testing"), ("rpmfusion-free".into(), Channel::Testing));
        assert_eq!(dnf_family("rpmfusion-nonfree-updates"), ("rpmfusion-nonfree".into(), Channel::Stable));
        assert_eq!(dnf_family("fedora-cisco-openh264"), ("fedora-cisco-openh264".into(), Channel::Stable));
    }

    #[test]
    fn fedora_with_rpmfusion() {
        let repos: Vec<Repo> = [
            ("fedora", true),
            ("fedora-debuginfo", false),
            ("updates", true),
            ("updates-testing", false),
            ("updates-testing-source", false),
            ("rpmfusion-free", true),
            ("rpmfusion-free-updates", true),
            ("rpmfusion-free-updates-testing", false),
            ("copr:copr.fedorainfracloud.org:chomiam:noos", true),
        ]
        .iter()
        .map(|(id, e)| repo(id, *e))
        .collect();
        let s = group_dnf(&repos, "45");
        let names: Vec<_> = s.iter().map(|s| (s.name.as_str(), s.channel, s.state)).collect();
        assert_eq!(names[0], ("Fedora 45", Channel::Stable, State::Enabled));
        assert!(s[0].essential && s[0].repos.len() == 2);
        assert!(names.contains(&("NOOS", Channel::Stable, State::Enabled)));
        assert!(names.contains(&("RPM Fusion — libre", Channel::Stable, State::Enabled)));
        // Non libre absent : proposé.
        let nonfree = s.iter().find(|s| s.key == "dnf:rpmfusion-nonfree:Stable").unwrap();
        assert_eq!((nonfree.state, nonfree.preset), (State::Missing, Some("rpmfusion-nonfree")));
        let testing: Vec<_> = s.iter().filter(|s| s.channel == Channel::Testing).map(|s| s.name.as_str()).collect();
        assert_eq!(testing, ["Fedora 45 — tests", "RPM Fusion — libre — tests"]);
        assert!(s.iter().all(|s| s.repos.iter().all(|r| !is_technical(r))));
    }

    #[test]
    fn partial() {
        let s = group_dnf(&[repo("rpmfusion-free", true), repo("rpmfusion-free-updates", false)], "45");
        assert_eq!(s.iter().find(|s| s.key == "dnf:rpmfusion-free:Stable").unwrap().state, State::Partial);
    }

    #[test]
    fn flatpak() {
        let r = |name: &str, enabled: bool| Repo {
            backend: BackendKind::Flatpak,
            id: format!("system:{name}"),
            name: name.into(),
            url: String::new(),
            enabled,
            scope: Some(Scope::System),
            file: name.into(),
            builtin: false,
        };
        let s = group_flatpak(&[r("fedora", true), r("fedora-testing", false)]);
        assert_eq!(s.len(), 3);
        assert_eq!(s.iter().find(|s| s.name == "Flathub").unwrap().state, State::Missing);
        assert_eq!(s.iter().find(|s| s.name == "Fedora Flatpaks — tests").unwrap().channel, Channel::Testing);
    }
}

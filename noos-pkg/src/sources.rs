//! Sources lisibles : les dépôts bruts (entrées de sources.list / *.sources, remotes Flatpak)
//! sont regroupés par famille, avec un seul interrupteur, et séparés en **stables** et **tests**.
//!
//! | Dépôts bruts                                          | Source                          |
//! |-------------------------------------------------------|---------------------------------|
//! | `deb.debian.org/debian sid …`                         | Debian sid                      |
//! | `deb.debian.org/debian experimental …`                | Debian experimental — tests     |
//! | `chomiam.github.io/noos-de sid main`                  | NOOS                            |
//! | entrées `deb-src` seules                              | masquées (outils de développeur)|
//! | remote Flatpak `flathub` / `flathub-beta`             | Flathub / Flathub — tests       |
//!
//! Les sources connues absentes (contrib / non-free de Debian, NOOS, experimental, Flathub)
//! apparaissent aussi, « non installées », avec le préréglage qui les ajoute : on voit tout de
//! suite ce qui est actif ou non.

use crate::{
    BackendKind, Repo, Scope,
    backends::aptsources::{is_debian_uri, is_noos_uri},
};

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
    /// Clé stable de la famille (`apt:debian:Stable`, `flatpak:system:flathub`…).
    pub key: String,
    pub name: String,
    pub description: String,
    pub channel: Channel,
    pub state: State,
    /// Dépôts réels de la source (sans paquets sources).
    pub repos: Vec<Repo>,
    /// Fournie par Debian / NOOS : ne peut pas être supprimée (seulement désactivée).
    pub builtin: bool,
    /// Ne doit pas être désactivée (dépôt principal de Debian, NOOS).
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

fn state_of(repos: &[Repo]) -> State {
    match (repos.iter().all(|r| r.enabled), repos.iter().any(|r| r.enabled)) {
        (true, _) => State::Enabled,
        (false, true) => State::Partial,
        _ => State::Disabled,
    }
}

/// (famille, canal) d'une entrée APT.
fn apt_family(r: &Repo) -> (String, Channel) {
    let testing = r.suites.iter().any(|s| s == "experimental" || s.ends_with("-proposed-updates"));
    let channel = if testing { Channel::Testing } else { Channel::Stable };
    if is_noos_uri(&r.url) {
        ("noos".into(), Channel::Stable)
    } else if r.url.contains("security.debian.org") || r.suites.iter().any(|s| s.ends_with("-security")) {
        ("debian-security".into(), Channel::Stable)
    } else if is_debian_uri(&r.url) {
        (if testing { "debian-experimental".into() } else { "debian".into() }, channel)
    } else {
        let host = r.url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string();
        (format!("{host} {}", r.suites.join(" ")), channel)
    }
}

fn suite_label(repos: &[Repo]) -> String {
    let suites: Vec<&str> = repos.iter().flat_map(|r| r.suites.iter().map(String::as_str)).collect();
    if suites.iter().any(|s| *s == "sid" || *s == "unstable") {
        "sid".into()
    } else {
        suites.first().copied().unwrap_or("").to_string()
    }
}

fn components_label(repos: &[Repo]) -> String {
    let mut comps: Vec<&str> = Vec::new();
    for c in repos.iter().flat_map(|r| r.components.iter()) {
        if !comps.contains(&c.as_str()) {
            comps.push(c);
        }
    }
    comps.join(", ")
}

/// (nom, description, essentielle, intégrée) d'une famille APT.
fn apt_label(family: &str, members: &[Repo]) -> (String, String, bool, bool) {
    let comps = components_label(members);
    match family {
        "debian" => (
            format!("Debian {}", suite_label(members)),
            format!("Dépôts officiels de Debian : système, applications et mises à jour (composants : {comps})"),
            true,
            true,
        ),
        "debian-security" => ("Debian — sécurité".into(), "Correctifs de sécurité de Debian".into(), false, true),
        "debian-experimental" => (
            "Debian experimental".into(),
            "Versions en préparation, installées seulement si vous les demandez".into(),
            false,
            false,
        ),
        "noos" => ("NOOS".into(), "Bureau NOOS et ses applications (dépôt APT sur GitHub Pages)".into(), true, true),
        _ => {
            let m = &members[0];
            let host = m.url.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("").to_string();
            let desc = format!("{} — {} {}", m.url, m.suites.join(" "), comps).trim().to_string();
            (if host.is_empty() { family.to_string() } else { host }, desc, false, false)
        }
    }
}

fn missing(family: &str, name: &str, description: &str, channel: Channel, preset: &'static str) -> Source {
    Source {
        backend: BackendKind::Apt,
        key: format!("apt:{family}:{channel:?}"),
        name: name.into(),
        description: description.into(),
        channel,
        state: State::Missing,
        repos: vec![],
        builtin: false,
        essential: false,
        preset: Some(preset),
    }
}

/// Regroupe les dépôts APT en sources (stables puis tests), avec contrib / non-free, NOOS et
/// experimental même absents.
pub fn group_apt(repos: &[Repo]) -> Vec<Source> {
    let mut families: Vec<(String, Channel, Vec<Repo>)> = Vec::new();
    for r in repos.iter().filter(|r| !is_technical(r)) {
        let (family, channel) = apt_family(r);
        match families.iter_mut().find(|(f, c, _)| *f == family && *c == channel) {
            Some(e) => e.2.push(r.clone()),
            None => families.push((family, channel, vec![r.clone()])),
        }
    }
    let mut out: Vec<Source> = families
        .into_iter()
        .map(|(family, channel, members)| {
            let (mut name, description, essential, builtin) = apt_label(&family, &members);
            if channel == Channel::Testing {
                name = format!("{name} — tests");
            }
            Source {
                backend: BackendKind::Apt,
                key: format!("apt:{family}:{channel:?}"),
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
    let debian: Vec<&Repo> = repos.iter().filter(|r| !is_technical(r) && apt_family(r).0 == "debian").collect();
    let has_nonfree = debian.iter().any(|r| r.components.iter().any(|c| c == "contrib" || c == "non-free"));
    if !debian.is_empty() && !has_nonfree {
        out.push(missing(
            "debian-nonfree",
            "Debian — contrib et non-free",
            "Pilotes NVIDIA, firmwares (Wi-Fi, cartes graphiques), Steam et logiciels non libres",
            Channel::Stable,
            "debian-nonfree",
        ));
    }
    if !out.iter().any(|s| s.key == "apt:noos:Stable") {
        out.push(missing("noos", "NOOS", "Bureau NOOS et ses applications : mises à jour de NOOS", Channel::Stable, "noos"));
    }
    if !out.iter().any(|s| s.key == "apt:debian-experimental:Testing") {
        out.push(missing(
            "debian-experimental",
            "Debian experimental — tests",
            "Versions en préparation, installées seulement si vous les demandez",
            Channel::Testing,
            "debian-experimental",
        ));
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
                description: if filtered { format!("{description} — limité à une sélection par la distribution") } else { description },
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

    fn repo(id: &str, url: &str, suites: &[&str], comps: &[&str], enabled: bool, builtin: bool) -> Repo {
        Repo {
            backend: BackendKind::Apt,
            id: id.into(),
            name: id.into(),
            url: url.into(),
            enabled,
            scope: None,
            file: "/etc/apt/sources.list.d/x.sources".into(),
            builtin,
            suites: suites.iter().map(|s| s.to_string()).collect(),
            components: comps.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn debian_sid() {
        let repos = vec![
            repo("debian:0", "http://deb.debian.org/debian", &["sid"], &["main"], true, true),
            repo("debian:1-source", "http://deb.debian.org/debian", &["sid"], &["main"], false, true),
            repo("noos:0", "https://chomiam.github.io/noos-de", &["sid"], &["main"], true, true),
            repo("vscode:0", "https://packages.microsoft.com/repos/code", &["stable"], &["main"], false, false),
        ];
        let s = group_apt(&repos);
        let names: Vec<_> = s.iter().map(|s| (s.name.as_str(), s.channel, s.state)).collect();
        assert!(names[0] == ("Debian sid", Channel::Stable, State::Enabled) || names[0].0 == "NOOS");
        let debian = s.iter().find(|s| s.key == "apt:debian:Stable").unwrap();
        assert!(debian.essential && debian.repos.len() == 1);
        assert!(names.contains(&("NOOS", Channel::Stable, State::Enabled)));
        assert!(names.contains(&("packages.microsoft.com", Channel::Stable, State::Disabled)));
        let nonfree = s.iter().find(|s| s.key == "apt:debian-nonfree:Stable").unwrap();
        assert_eq!((nonfree.state, nonfree.preset), (State::Missing, Some("debian-nonfree")));
        let exp = s.iter().find(|s| s.channel == Channel::Testing).unwrap();
        assert_eq!((exp.state, exp.preset), (State::Missing, Some("debian-experimental")));
        assert!(s.iter().all(|s| s.repos.iter().all(|r| !is_technical(r))));
    }

    #[test]
    fn nonfree_present_and_experimental() {
        let repos = vec![
            repo("debian:0", "http://deb.debian.org/debian", &["sid"], &["main", "contrib", "non-free", "non-free-firmware"], true, true),
            repo("debian-experimental:0", "http://deb.debian.org/debian", &["experimental"], &["main"], false, false),
        ];
        let s = group_apt(&repos);
        assert!(s.iter().all(|s| s.key != "apt:debian-nonfree:Stable"));
        let debian = s.iter().find(|s| s.key == "apt:debian:Stable").unwrap();
        assert!(debian.description.contains("non-free"));
        let exp = s.iter().find(|s| s.key == "apt:debian-experimental:Testing").unwrap();
        assert_eq!((exp.name.as_str(), exp.state), ("Debian experimental — tests", State::Disabled));
        // NOOS absent : proposé.
        assert_eq!(s.iter().find(|s| s.key == "apt:noos:Stable").unwrap().state, State::Missing);
    }

    #[test]
    fn partial() {
        let s = group_apt(&[
            repo("debian:0", "http://deb.debian.org/debian", &["sid"], &["main"], true, true),
            repo("sources:3", "http://ftp.fr.debian.org/debian", &["sid"], &["main"], false, true),
        ]);
        assert_eq!(s.iter().find(|s| s.key == "apt:debian:Stable").unwrap().state, State::Partial);
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
            suites: vec![],
            components: vec![],
        };
        let s = group_flatpak(&[r("flathub-beta", false)]);
        assert_eq!(s.len(), 2);
        assert_eq!(s.iter().find(|s| s.name == "Flathub").unwrap().state, State::Missing);
        assert_eq!(s.iter().find(|s| s.name == "Flathub — tests").unwrap().channel, Channel::Testing);
    }
}

//! Sources APT : `/etc/apt/sources.list` (format une ligne) et `/etc/apt/sources.list.d/`
//! (`*.list` une ligne, `*.sources` deb822). Lecture, activation / désactivation, suppression,
//! ajout des composants non libres de Debian et fichiers des dépôts tiers.
//!
//! | Format    | Désactivé                         | Identifiant du dépôt          |
//! |-----------|-----------------------------------|-------------------------------|
//! | une ligne | ligne commentée `# deb …`         | `<fichier>:<n° de ligne>`     |
//! | deb822    | `Enabled: no` dans la strophe     | `<fichier>:<n° de strophe>`   |
//!
//! Une entrée `deb-src` seule (paquets sources) reçoit le suffixe `-source` : elle est masquée
//! comme les dépôts techniques (voir `sources::is_technical`).

use crate::{BackendKind, Repo};

pub const SOURCES_LIST: &str = "/etc/apt/sources.list";
pub const SOURCES_DIR: &str = "/etc/apt/sources.list.d";
pub const KEYRINGS_DIR: &str = "/etc/apt/keyrings";
/// Fichier deb822 de Debian (installateur NOOS, debootstrap récents).
pub const DEBIAN_SOURCES: &str = "/etc/apt/sources.list.d/debian.sources";
/// Dépôt APT de NOOS (GitHub Pages), installé par le paquet `noos`.
pub const NOOS_SOURCES: &str = "/etc/apt/sources.list.d/noos.sources";
pub const NOOS_URL: &str = "https://chomiam.github.io/noos-de";
pub const NOOS_KEYRING: &str = "/usr/share/keyrings/noos-archive-keyring.asc";
pub const DEBIAN_KEYRING: &str = "/usr/share/keyrings/debian-archive-keyring.gpg";
pub const DEBIAN_MIRROR: &str = "http://deb.debian.org/debian";
/// Composants non libres de Debian (pilotes NVIDIA, firmwares, codecs restreints).
pub const NONFREE_COMPONENTS: &[&str] = &["contrib", "non-free", "non-free-firmware"];

/// Google Chrome : même fichier et même ligne que ceux que le paquet google-chrome-stable
/// écrit lui-même (sa tâche cron les retrouve et ne crée pas de doublon) ; clé dans
/// trusted.gpg.d, comme le fait Google.
pub const CHROME_LIST: &str = "/etc/apt/sources.list.d/google-chrome.list";
pub const CHROME_URL: &str = "https://dl.google.com/linux/chrome/deb/";
pub const CHROME_KEY_URL: &str = "https://dl.google.com/linux/linux_signing_key.pub";
pub const CHROME_KEYRING: &str = "/etc/apt/trusted.gpg.d/google-chrome.asc";
/// Dépôt officiel de NVIDIA (pilotes récents, CUDA) pour Debian 13, utilisable sur sid.
pub const NVIDIA_SOURCES: &str = "/etc/apt/sources.list.d/nvidia-cuda.sources";
pub const NVIDIA_URL: &str = "https://developer.download.nvidia.com/compute/cuda/repos/debian13/x86_64/";
pub const NVIDIA_KEY_URL: &str = "https://developer.download.nvidia.com/compute/cuda/repos/debian13/x86_64/8793F200.pub";
pub const NVIDIA_KEYRING: &str = "/etc/apt/keyrings/nvidia-cuda.asc";

/// Une entrée de dépôt, quel que soit son format.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub types: Vec<String>,
    pub uris: Vec<String>,
    pub suites: Vec<String>,
    pub components: Vec<String>,
    pub enabled: bool,
    pub file: String,
    /// N° de ligne (une ligne) ou de strophe (deb822), à partir de 0.
    pub index: usize,
    pub deb822: bool,
}

impl Entry {
    pub fn id(&self) -> String {
        let stem = std::path::Path::new(&self.file)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let stem = stem.trim_end_matches(".sources").trim_end_matches(".list");
        let src = if self.types.iter().all(|t| t == "deb-src") { "-source" } else { "" };
        format!("{stem}:{}{src}", self.index)
    }

    pub fn to_repo(&self) -> Repo {
        let url = self.uris.first().cloned().unwrap_or_default();
        let short = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        let name = format!("{short} {} {}", self.suites.join(" "), self.components.join(" ")).trim().to_string();
        Repo {
            backend: BackendKind::Apt,
            id: self.id(),
            name,
            url,
            enabled: self.enabled,
            scope: None,
            file: self.file.clone(),
            builtin: is_builtin(self),
            suites: self.suites.clone(),
            components: self.components.clone(),
        }
    }
}

/// Dépôt fourni par la distribution ou par NOOS : désactivable, pas supprimable.
fn is_builtin(e: &Entry) -> bool {
    let distro_file = e.file == SOURCES_LIST || e.file == DEBIAN_SOURCES || e.file == NOOS_SOURCES;
    distro_file && e.uris.iter().any(|u| is_debian_uri(u) || is_noos_uri(u))
}

pub fn is_debian_uri(u: &str) -> bool {
    let host = u.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("");
    host.ends_with("debian.org") || host.ends_with("debian.net") || u.starts_with("mirror+") || u.trim_end_matches('/').ends_with("/debian")
}

pub fn is_noos_uri(u: &str) -> bool {
    u.trim_end_matches('/').starts_with(NOOS_URL)
}

pub fn is_chrome_uri(u: &str) -> bool {
    u.contains("dl.google.com/linux/chrome")
}

pub fn is_nvidia_uri(u: &str) -> bool {
    u.contains("developer.download.nvidia.com/compute/cuda/repos")
}

/// Format une ligne : `deb [options] URI suite [composants…]`, éventuellement commenté.
pub fn parse_list(path: &str, text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let mut line = raw.trim();
        let mut enabled = true;
        if let Some(rest) = line.strip_prefix('#') {
            let rest = rest.trim_start();
            if !(rest.starts_with("deb ") || rest.starts_with("deb-src ")) {
                continue;
            }
            enabled = false;
            line = rest;
        }
        let Some((ty, rest)) = line.split_once(char::is_whitespace) else { continue };
        if ty != "deb" && ty != "deb-src" {
            continue;
        }
        let mut rest = rest.trim_start();
        // Options entre crochets : [ arch=amd64 signed-by=… ]
        if rest.starts_with('[') {
            match rest.find(']') {
                Some(end) => rest = rest[end + 1..].trim_start(),
                None => continue,
            }
        }
        // Commentaire en fin de ligne.
        let rest = rest.split('#').next().unwrap_or("");
        let mut words = rest.split_whitespace();
        let (Some(uri), Some(suite)) = (words.next(), words.next()) else { continue };
        out.push(Entry {
            types: vec![ty.to_string()],
            uris: vec![uri.to_string()],
            suites: vec![suite.to_string()],
            components: words.map(String::from).collect(),
            enabled,
            file: path.to_string(),
            index: n,
            deb822: false,
        });
    }
    out
}

/// Strophes deb822 (séparées par une ligne vide), lignes de continuation comprises.
fn stanzas(text: &str) -> Vec<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut cur: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if raw.trim_start().starts_with('#') {
            continue;
        }
        if raw.starts_with(' ') || raw.starts_with('\t') {
            if let Some(last) = cur.last_mut() {
                last.1.push('\n');
                last.1.push_str(raw.trim());
            }
            continue;
        }
        if let Some((k, v)) = raw.split_once(':') {
            cur.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse_sources(path: &str, text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for (n, st) in stanzas(text).into_iter().enumerate() {
        let get = |key: &str| st.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone()).unwrap_or_default();
        let words = |key: &str| get(key).split_whitespace().map(String::from).collect::<Vec<_>>();
        let uris = words("URIs");
        if uris.is_empty() {
            continue;
        }
        let enabled = !matches!(get("Enabled").to_lowercase().as_str(), "no" | "false" | "0");
        let types = words("Types");
        out.push(Entry {
            types: if types.is_empty() { vec!["deb".into()] } else { types },
            uris,
            suites: words("Suites"),
            components: words("Components"),
            enabled,
            file: path.to_string(),
            index: n,
            deb822: true,
        });
    }
    out
}

pub fn parse_file(path: &str, text: &str) -> Vec<Entry> {
    if path.ends_with(".sources") { parse_sources(path, text) } else { parse_list(path, text) }
}

fn files() -> Vec<String> {
    let mut v = vec![SOURCES_LIST.to_string()];
    let mut dir: Vec<String> = std::fs::read_dir(SOURCES_DIR)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path().to_string_lossy().to_string())
                .filter(|p| p.ends_with(".list") || p.ends_with(".sources"))
                .collect()
        })
        .unwrap_or_default();
    dir.sort();
    v.extend(dir);
    v
}

pub fn entries() -> Vec<Entry> {
    files()
        .into_iter()
        .flat_map(|f| {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            parse_file(&f, &text)
        })
        .collect()
}

pub fn list() -> anyhow::Result<Vec<Repo>> {
    Ok(entries().iter().map(Entry::to_repo).collect())
}

/// `idx` d'un identifiant de dépôt (`debian:0`, `sources.list:12-source`).
fn index_of(id: &str) -> Option<usize> {
    id.rsplit(':').next()?.trim_end_matches("-source").parse().ok()
}

/// Strophe deb822 n° `idx` : (première ligne, ligne suivant la dernière) dans `text`.
fn stanza_range(text: &str, idx: usize) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut n = 0;
    let mut i = 0;
    while i < lines.len() {
        while i < lines.len() && lines[i].trim().is_empty() {
            i += 1;
        }
        if i >= lines.len() {
            break;
        }
        let start = i;
        while i < lines.len() && !lines[i].trim().is_empty() {
            i += 1;
        }
        // Une strophe faite uniquement de commentaires n'est pas comptée (comme `stanzas`).
        if lines[start..i].iter().any(|l| !l.trim_start().starts_with('#')) {
            if n == idx {
                return Some((start, i));
            }
            n += 1;
        }
    }
    None
}

/// Réécrit (ou ajoute) le champ `key` dans la strophe `idx`.
pub fn set_field(text: &str, idx: usize, key: &str, value: &str) -> Option<String> {
    let (start, end) = stanza_range(text, idx)?;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let pos = (start..end).find(|&i| lines[i].split(':').next().is_some_and(|k| k.trim().eq_ignore_ascii_case(key)));
    let new = format!("{key}: {value}");
    match pos {
        Some(i) => lines[i] = new,
        None => lines.insert(end, new),
    }
    Some(lines.join("\n") + "\n")
}

/// Commente / décommente la ligne `idx` d'un fichier une ligne.
pub fn set_line_enabled(text: &str, idx: usize, enabled: bool) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let line = lines.get_mut(idx)?;
    let bare = line.trim_start().trim_start_matches('#').trim_start().to_string();
    *line = if enabled { bare } else { format!("# {bare}") };
    Some(lines.join("\n") + "\n")
}

/// Contenus à écrire (chemin → texte complet) pour activer / désactiver des dépôts ; plusieurs
/// dépôts d'un même fichier sont appliqués à la suite sur le même texte.
pub fn toggle_many(repos: &[Repo], enabled: bool) -> anyhow::Result<Vec<(String, String)>> {
    let mut files: Vec<(String, String)> = Vec::new();
    for r in repos {
        let idx = index_of(&r.id).ok_or_else(|| anyhow::anyhow!("identifiant de dépôt invalide : {}", r.id))?;
        let text = match files.iter().find(|(p, _)| *p == r.file) {
            Some((_, t)) => t.clone(),
            None => std::fs::read_to_string(&r.file).map_err(|e| anyhow::anyhow!("lecture de {} : {e}", r.file))?,
        };
        let new = if r.file.ends_with(".sources") {
            set_field(&text, idx, "Enabled", if enabled { "yes" } else { "no" })
        } else {
            set_line_enabled(&text, idx, enabled)
        }
        .ok_or_else(|| anyhow::anyhow!("dépôt {} introuvable dans {}", r.id, r.file))?;
        match files.iter_mut().find(|(p, _)| *p == r.file) {
            Some(e) => e.1 = new,
            None => files.push((r.file.clone(), new)),
        }
    }
    Ok(files)
}

/// Texte du fichier sans le dépôt `idx` ; `None` si le fichier n'a plus de dépôt.
pub fn remove_entry(path: &str, text: &str, idx: usize) -> Option<String> {
    let rest = if path.ends_with(".sources") {
        let (start, end) = stanza_range(text, idx)?;
        let lines: Vec<&str> = text.lines().collect();
        let mut kept: Vec<&str> = lines[..start].to_vec();
        kept.extend_from_slice(&lines[end.min(lines.len())..]);
        kept.join("\n") + "\n"
    } else {
        let lines: Vec<&str> = text.lines().enumerate().filter(|(n, _)| *n != idx).map(|(_, l)| l).collect();
        lines.join("\n") + "\n"
    };
    (!parse_file(path, &rest).is_empty()).then_some(rest)
}

/// Ajoute `contrib non-free non-free-firmware` aux dépôts Debian de `text`.
pub fn add_components(path: &str, text: &str, wanted: &[&str]) -> String {
    if path.ends_with(".sources") {
        let mut out = text.to_string();
        for e in parse_sources(path, text) {
            if !e.uris.iter().any(|u| is_debian_uri(u)) {
                continue;
            }
            let mut comps = e.components.clone();
            for w in wanted {
                if !comps.iter().any(|c| c == w) {
                    comps.push(w.to_string());
                }
            }
            if comps != e.components {
                out = set_field(&out, e.index, "Components", &comps.join(" ")).unwrap_or(out);
            }
        }
        out
    } else {
        let entries = parse_list(path, text);
        let mut lines: Vec<String> = text.lines().map(String::from).collect();
        for e in entries.iter().filter(|e| e.uris.iter().any(|u| is_debian_uri(u))) {
            let missing: Vec<&str> = wanted.iter().copied().filter(|w| !e.components.iter().any(|c| c == w)).collect();
            if !missing.is_empty() {
                let l = &mut lines[e.index];
                let (body, comment) = match l.find(" #") {
                    Some(p) => (l[..p].to_string(), l[p..].to_string()),
                    None => (l.clone(), String::new()),
                };
                *l = format!("{} {}{comment}", body.trim_end(), missing.join(" "));
            }
        }
        lines.join("\n") + "\n"
    }
}

/// Strophe deb822 d'un dépôt.
pub fn deb822(uri: &str, suites: &str, components: &str, signed_by: &str) -> String {
    let mut s = format!("Types: deb\nURIs: {uri}\nSuites: {suites}\n");
    if !components.is_empty() {
        s.push_str(&format!("Components: {components}\n"));
    }
    if !signed_by.is_empty() {
        s.push_str(&format!("Signed-By: {signed_by}\n"));
    }
    s
}

/// Fichier `noos.sources` (identique à celui du paquet `noos`).
pub fn noos_sources() -> String {
    format!(
        "# Dépôt APT de NOOS (GitHub Pages) : bureau NOOS et ses applications.\n{}",
        deb822(NOOS_URL, "sid", "main", NOOS_KEYRING)
    )
}

/// `google-chrome.list` (format d'une ligne, celui de Google).
pub fn chrome_list() -> String {
    format!("# Google Chrome (ajouté par Logiciels de NOOS)\ndeb [arch=amd64] {CHROME_URL} stable main\n")
}

/// Dépôt de NVIDIA (dépôt « plat » : suite « / », sans composants).
pub fn nvidia_sources() -> String {
    format!(
        "# Dépôt officiel de NVIDIA : pilotes récents et CUDA (ajouté par Logiciels de NOOS).\n{}",
        deb822(NVIDIA_URL, "/", "", NVIDIA_KEYRING)
    )
}

/// Dépôt « experimental » de Debian (paquets jamais installés automatiquement : NotAutomatic).
pub fn experimental_sources() -> String {
    format!(
        "# Debian experimental : versions en cours de préparation (installation à la demande).\n{}",
        deb822(DEBIAN_MIRROR, "experimental", "main contrib non-free non-free-firmware", DEBIAN_KEYRING)
    )
}

/// Nom de fichier sûr pour un dépôt tiers (`https://repo.example.org/apt` → `repo.example.org`).
pub fn file_name_for(uri: &str) -> String {
    let host = uri.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or("depot");
    let name: String = host.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' }).collect();
    if name.is_empty() { "depot".into() } else { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEB822: &str = "Types: deb deb-src\nURIs: http://deb.debian.org/debian\nSuites: sid\nComponents: main\nSigned-By: /usr/share/keyrings/debian-archive-keyring.gpg\n\n# commentaire\n\nTypes: deb\nURIs: https://example.org/apt\nSuites: stable\nComponents: main\nEnabled: no\nSigned-By:\n -----BEGIN PGP PUBLIC KEY BLOCK-----\n .\n abc\n -----END PGP PUBLIC KEY BLOCK-----\n";

    #[test]
    fn parse_deb822() {
        let e = parse_sources(DEBIAN_SOURCES, DEB822);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].suites, ["sid"]);
        assert!(e[0].enabled && !e[1].enabled);
        assert_eq!(e[1].index, 1);
        let r = e[0].to_repo();
        assert!(r.builtin);
        assert_eq!(r.id, "debian:0");
        assert!(!e[1].to_repo().builtin);
    }

    #[test]
    fn parse_one_line() {
        let t = "# Debian\ndeb http://deb.debian.org/debian sid main contrib # miroir\n# deb-src http://deb.debian.org/debian sid main\ndeb [arch=amd64 signed-by=/etc/apt/keyrings/x.gpg] https://dl.example.com/linux stable main\n#pas un dépôt\n";
        let e = parse_list(SOURCES_LIST, t);
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].components, ["main", "contrib"]);
        assert!(!e[1].enabled);
        assert_eq!(e[1].to_repo().id, "sources:2-source");
        assert_eq!(e[2].uris[0], "https://dl.example.com/linux");
        assert_eq!(e[2].index, 3);
    }

    #[test]
    fn toggle_and_remove() {
        let t = set_field(DEB822, 0, "Enabled", "no").unwrap();
        assert!(!parse_sources("x.sources", &t)[0].enabled);
        let t = set_field(&t, 1, "Enabled", "yes").unwrap();
        let e = parse_sources("x.sources", &t);
        assert!(!e[0].enabled && e[1].enabled);
        // La clé sur plusieurs lignes reste intacte.
        assert!(t.contains(" -----END PGP PUBLIC KEY BLOCK-----"));

        let l = "deb http://deb.debian.org/debian sid main\ndeb https://x.org/apt stable main\n";
        let off = set_line_enabled(l, 1, false).unwrap();
        assert!(off.contains("# deb https://x.org/apt"));
        assert!(!parse_list("a.list", &off)[1].enabled);
        let on = set_line_enabled(&off, 1, true).unwrap();
        assert_eq!(on, l);

        let rest = remove_entry("x.sources", DEB822, 1).unwrap();
        assert_eq!(parse_sources("x.sources", &rest).len(), 1);
        assert!(remove_entry("y.list", "deb https://x.org/apt stable main\n", 0).is_none());
    }

    #[test]
    fn nonfree() {
        let t = add_components(DEBIAN_SOURCES, DEB822, NONFREE_COMPONENTS);
        let e = parse_sources(DEBIAN_SOURCES, &t);
        assert_eq!(e[0].components, ["main", "contrib", "non-free", "non-free-firmware"]);
        assert_eq!(e[1].components, ["main"]);
        let l = add_components(SOURCES_LIST, "deb http://deb.debian.org/debian sid main contrib # x\n", NONFREE_COMPONENTS);
        assert_eq!(l, "deb http://deb.debian.org/debian sid main contrib non-free non-free-firmware # x\n");
    }

    #[test]
    fn names() {
        assert_eq!(file_name_for("https://repo.example.org/apt/"), "repo.example.org");
        assert!(noos_sources().contains("Signed-By: /usr/share/keyrings/noos-archive-keyring.asc"));
        assert!(is_noos_uri("https://chomiam.github.io/noos-de/"));
    }
}

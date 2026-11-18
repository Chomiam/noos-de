//! Fiche détaillée d'un paquet (Logiciels › clic sur un résultat) : descriptif, versions,
//! tailles, dépôt, dépendances, licence, captures et icône AppStream.
//!
//! | Source     | Données                                                                     |
//! |------------|-----------------------------------------------------------------------------|
//! | apt        | `apt-cache show` (description traduite : `Acquire::Languages`), `apt-cache policy` |
//! | DEP-11     | `/var/lib/apt/lists/*_dep11_Components-<arch>.yml.gz` (nom, description, licence, captures) |
//! | Flatpak    | `flatpak remote-info` / `flatpak info`, AppStream du remote (`appstream.xml.gz`) |
//!
//! La simulation (`apt-get -s install|remove`) dit ce que l'opération ajoutera ou retirera.
//! Tout est lu sans droits root.

use std::path::{Path, PathBuf};

/// Capture d'écran AppStream : vignette (affichage) et image d'origine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Screenshot {
    pub thumbnail: String,
    pub full: String,
    pub caption: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Details {
    /// Identifiant du paquet (nom apt ou ID Flatpak).
    pub id: String,
    /// Nom lisible (AppStream) ; à défaut, le nom du paquet.
    pub name: String,
    pub summary: String,
    /// Description longue en texte brut (paragraphes séparés par une ligne vide).
    pub description: String,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    /// Taille du téléchargement, en octets.
    pub download_size: Option<u64>,
    /// Taille une fois installé, en octets.
    pub installed_size: Option<u64>,
    /// Origine courte (`debian`, `noos`, `flathub`…).
    pub origin: String,
    /// Dépôt lisible : « deb.debian.org/debian · sid/main », « flathub (système) ».
    pub repository: String,
    pub section: String,
    pub homepage: String,
    pub license: String,
    /// Développeur (AppStream).
    pub developer: String,
    /// Mainteneur du paquet Debian (« Nom <adresse> »).
    pub maintainer: String,
    pub architecture: String,
    pub depends: Vec<String>,
    pub recommends: Vec<String>,
    pub suggests: Vec<String>,
    /// Commandes fournies (champ `Commands` de Debian).
    pub commands: Vec<String>,
    /// Runtime Flatpak (`org.gnome.Platform/x86_64/49`).
    pub runtime: String,
    pub appstream_id: Option<String>,
    /// Icône du thème (`stock` AppStream).
    pub icon_name: Option<String>,
    /// Icône locale (cache AppStream de Flatpak).
    pub icon_file: Option<PathBuf>,
    /// Icône distante (médias DEP-11).
    pub icon_url: Option<String>,
    pub screenshots: Vec<Screenshot>,
    /// Lanceur `.desktop` (identifiant) pour « Ouvrir », si le logiciel est installé.
    pub launchable: Option<String>,
}

// ───────────────────────────── Langues ─────────────────────────────

/// Langues préférées de la session, de la plus précise à la plus générale :
/// `fr_FR.UTF-8` → `fr_FR`, `fr-FR`, `fr`. Toujours suivies de `C` et `en`.
pub fn languages() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    let raw = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty() && v != "C" && !v.starts_with("C.") && v != "POSIX")
        .unwrap_or_default();
    for part in raw.split(':') {
        let base = part.split(['.', '@']).next().unwrap_or("");
        if base.is_empty() {
            continue;
        }
        push(base.to_string());
        push(base.replace('_', "-"));
        push(base.split('_').next().unwrap_or("").to_string());
    }
    push("C".into());
    push("en".into());
    push("en_US".into());
    push("en-US".into());
    push("en_GB".into());
    push("en-GB".into());
    out
}

/// Codes de langue courts pour apt (`Acquire::Languages`) : `fr`, puis `en`.
fn apt_languages() -> Vec<String> {
    let mut v: Vec<String> = languages().into_iter().filter(|l| l.len() == 2 && l != "C").collect();
    if !v.contains(&"en".to_string()) {
        v.push("en".into());
    }
    v
}

/// Valeur dans la langue préférée parmi (langue, texte) ; sinon la première.
pub fn pick_lang<'a>(values: &'a [(String, String)], langs: &[String]) -> Option<&'a str> {
    langs
        .iter()
        .find_map(|l| values.iter().find(|(k, _)| k == l).map(|(_, v)| v.as_str()))
        .or_else(|| values.first().map(|(_, v)| v.as_str()))
}

// ───────────────────────────── Texte ─────────────────────────────

/// Texte brut d'une description AppStream (`<p>`, `<ul><li>`, `<em>`…).
pub fn plain(markup: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in markup.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let closing = tag.starts_with('/');
                let t = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_lowercase();
                match t.as_str() {
                    "p" | "ul" | "ol" if !closing && !out.trim().is_empty() => out.push_str("\n\n"),
                    "li" if !closing => out.push_str("\n • "),
                    "br" | "br/" => out.push('\n'),
                    _ => {}
                }
            }
            _ if in_tag => tag.push(c),
            // Les retours à la ligne du texte ne sont que de la mise en page du fichier.
            '\n' | '\r' | '\t' => out.push(' '),
            _ => out.push(c),
        }
    }
    let lines: Vec<String> = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    let mut text = lines.join("\n");
    // « \n • » : la puce reste collée à sa ligne après la normalisation.
    text = text.replace("\n•", "\n •");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    unescape_xml(text.trim())
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

/// Description longue Debian (lignes de continuation, sans la 1re espace) en texte lisible :
/// « . » = paragraphe ; les lignes d'un paragraphe sont rejointes ; les lignes en retrait
/// (listes, exemples) gardent leur retour à la ligne.
pub fn debian_long_description(lines: &[String]) -> String {
    let mut paras: Vec<String> = Vec::new();
    let mut cur = String::new();
    for l in lines {
        if l.trim() == "." {
            if !cur.is_empty() {
                paras.push(std::mem::take(&mut cur));
            }
            continue;
        }
        // Ligne de liste (« * », « - », « 1. »…) : retour à la ligne. Un simple retrait ne
        // suffit pas : les traductions en ont souvent par erreur.
        let verbatim = {
            let t = l.trim_start();
            ["* ", "- ", "+ ", "o ", "• "].iter().any(|m| t.starts_with(m))
                || t.split_once(". ").is_some_and(|(n, _)| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()))
        };
        if cur.is_empty() {
            cur = l.trim().to_string();
        } else if verbatim {
            cur.push('\n');
            cur.push_str(l.trim());
        } else {
            cur.push(' ');
            cur.push_str(l.trim());
        }
    }
    if !cur.is_empty() {
        paras.push(cur);
    }
    paras.join("\n\n")
}

/// « libc6 (>= 2.43), foo | bar:any, gimp-data (<= 3) » → « libc6 », « foo | bar », « gimp-data » (sans doublons).
pub fn parse_relations(field: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dep in field.split(',') {
        let alts: Vec<String> = dep
            .split('|')
            .map(|a| {
                let name = a.trim().split([' ', '(', '[', '<']).next().unwrap_or("");
                name.trim_end_matches(":any").to_string()
            })
            .filter(|a| !a.is_empty())
            .collect();
        if alts.is_empty() {
            continue;
        }
        let d = alts.join(" | ");
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// « 102.5 MB », « 1,2 GB », « 512 bytes », « 3.4 MiB » → octets (unités de `g_format_size`).
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().replace('\u{a0}', " ");
    let (num, unit) = s.split_once(' ').unwrap_or((&s, "B"));
    let n: f64 = num.replace(',', ".").parse().ok()?;
    let mult: f64 = match unit.trim().to_lowercase().as_str() {
        "b" | "bytes" | "byte" | "octets" | "o" => 1.0,
        "kb" | "ko" => 1e3,
        "mb" | "mo" => 1e6,
        "gb" | "go" => 1e9,
        "tb" | "to" => 1e12,
        "kib" | "kio" => 1024.0,
        "mib" | "mio" => 1024.0 * 1024.0,
        "gib" | "gio" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((n * mult).round() as u64)
}

// ───────────────────────────── apt-cache show ─────────────────────────────

/// Champ d'un paragraphe de `apt-cache show` : valeur de la 1re ligne et lignes de
/// continuation (sans leur première espace).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field {
    pub key: String,
    pub value: String,
    pub more: Vec<String>,
}

pub type Stanza = Vec<Field>;

/// Paragraphes de `apt-cache show` (un par version disponible).
pub fn parse_show(out: &str) -> Vec<Stanza> {
    let mut stanzas: Vec<Stanza> = Vec::new();
    let mut cur: Stanza = Vec::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                stanzas.push(std::mem::take(&mut cur));
            }
        } else if line.starts_with([' ', '\t']) {
            if let Some(f) = cur.last_mut() {
                f.more.push(line[1..].to_string());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            cur.push(Field { key: k.trim().to_string(), value: v.trim().to_string(), more: vec![] });
        }
    }
    if !cur.is_empty() {
        stanzas.push(cur);
    }
    stanzas
}

pub fn field<'a>(s: &'a Stanza, key: &str) -> Option<&'a Field> {
    s.iter().find(|f| f.key.eq_ignore_ascii_case(key))
}

fn value(s: &Stanza, key: &str) -> String {
    field(s, key).map(|f| {
        if f.more.is_empty() { f.value.clone() } else { format!("{} {}", f.value, f.more.iter().map(|m| m.trim()).collect::<Vec<_>>().join(" ")) }
    })
    .unwrap_or_default()
}

/// Paragraphe de la version voulue (candidate, sinon installée, sinon le premier).
pub fn pick_stanza<'a>(stanzas: &'a [Stanza], versions: &[Option<&str>]) -> Option<&'a Stanza> {
    versions
        .iter()
        .flatten()
        .find_map(|v| stanzas.iter().find(|s| field(s, "Version").is_some_and(|f| f.value == *v)))
        .or(stanzas.first())
}

/// Remplit une fiche depuis un paragraphe de `apt-cache show`.
pub fn details_from_stanza(s: &Stanza) -> Details {
    // Description traduite (`Description-fr`), anglaise (`Description-en`) ou d'origine.
    let desc = s
        .iter()
        .find(|f| f.key.starts_with("Description") && f.key != "Description-md5" && !f.value.is_empty());
    let (summary, description) = desc.map(|f| (f.value.clone(), debian_long_description(&f.more))).unwrap_or_default();
    let id = value(s, "Package");
    Details {
        name: id.clone(),
        id,
        summary,
        description,
        download_size: value(s, "Size").parse().ok(),
        installed_size: value(s, "Installed-Size").parse::<u64>().ok().map(|k| k * 1024),
        section: value(s, "Section"),
        homepage: value(s, "Homepage"),
        maintainer: value(s, "Maintainer"),
        architecture: value(s, "Architecture"),
        depends: parse_relations(&format!("{}, {}", value(s, "Pre-Depends"), value(s, "Depends"))),
        recommends: parse_relations(&value(s, "Recommends")),
        suggests: parse_relations(&value(s, "Suggests")),
        commands: value(s, "Commands").split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect(),
        ..Default::default()
    }
}

/// « 500 http://deb.debian.org/debian sid/main amd64 Packages » → « deb.debian.org/debian · sid/main ».
pub fn repository_label(source_line: &str) -> String {
    let words: Vec<&str> = source_line.split_whitespace().collect();
    let Some(pos) = words.iter().position(|w| w.contains("://")) else { return source_line.trim().to_string() };
    let url = words[pos].split("://").nth(1).unwrap_or(words[pos]).trim_end_matches('/');
    match words.get(pos + 1) {
        Some(suite) => format!("{url} · {suite}"),
        None => url.to_string(),
    }
}

/// Licence d'un paquet installé : 1er champ `License:` d'un `copyright` lisible par machine.
pub fn license_from_copyright(text: &str) -> Option<String> {
    if !text.starts_with("Format:") {
        return None;
    }
    // Le paragraphe « Files: * » donne la licence principale.
    let mut in_main = false;
    let mut first: Option<String> = None;
    for l in text.lines() {
        if let Some(v) = l.strip_prefix("Files:") {
            in_main = v.trim() == "*";
        } else if let Some(v) = l.strip_prefix("License:") {
            let v = v.trim().to_string();
            if v.is_empty() {
                continue;
            }
            if in_main {
                return Some(v);
            }
            first.get_or_insert(v);
        }
    }
    first
}

pub fn apt_languages_opts() -> Vec<String> {
    let mut v = Vec::new();
    for l in apt_languages() {
        v.push("-o".to_string());
        v.push(format!("Acquire::Languages::={l}"));
    }
    v
}

// ───────────────────────────── Mini-lecteur YAML (DEP-11) ─────────────────────────────

/// Sous-ensemble de YAML suffisant pour DEP-11 : tables, listes, scalaires simples, entre
/// apostrophes / guillemets, blocs `|` et `>`. Pas de références ni de flux `{…}`.
#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Str(String),
    Map(Vec<(String, Yaml)>),
    Seq(Vec<Yaml>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn seq(&self) -> &[Yaml] {
        match self {
            Yaml::Seq(s) => s,
            _ => &[],
        }
    }
    /// Table de traductions `{C: …, fr: …}` → paires (langue, texte).
    pub fn translations(&self) -> Vec<(String, String)> {
        match self {
            Yaml::Map(m) => m.iter().filter_map(|(k, v)| v.str().map(|s| (k.clone(), s.to_string()))).collect(),
            Yaml::Str(s) => vec![("C".into(), s.clone())],
            _ => vec![],
        }
    }
}

type Lines = Vec<(usize, String)>;

pub fn parse_yaml(text: &str) -> Yaml {
    let mut lines: Lines = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#') && !l.starts_with('%') && t != "---" && t != "..."
        })
        .map(|l| {
            let t = l.trim_start_matches(' ');
            (l.len() - t.len(), t.trim_end().to_string())
        })
        .collect();
    let mut i = 0;
    let mut all: Vec<(String, Yaml)> = Vec::new();
    while i < lines.len() {
        let before = i;
        let indent = lines[i].0;
        match block(&mut lines, &mut i, indent) {
            Yaml::Map(m) => all.extend(m),
            other => all.push((String::new(), other)),
        }
        if i == before {
            i += 1;
        }
    }
    Yaml::Map(all)
}

fn is_item(s: &str) -> bool {
    s == "-" || s.starts_with("- ")
}

/// « clé: valeur » / « clé: » (clé éventuellement entre guillemets).
fn split_key(s: &str) -> Option<(String, String)> {
    if let Some(q) = s.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let end = s[1..].find(q)? + 1;
        let rest = &s[end + 1..];
        let v = rest.strip_prefix(':')?;
        if !v.is_empty() && !v.starts_with(' ') {
            return None;
        }
        return Some((s[1..end].to_string(), v.trim().to_string()));
    }
    if let Some(k) = s.strip_suffix(':') {
        if !k.contains(": ") {
            return Some((k.to_string(), String::new()));
        }
    }
    let (k, v) = s.split_once(": ")?;
    Some((k.to_string(), v.trim().to_string()))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                Some(o) => out.push(o),
                None => {}
            }
        }
        return out;
    }
    s.to_string()
}

/// Scalaire sur plusieurs lignes : les lignes plus en retrait que la clé le continuent.
fn scalar(lines: &Lines, i: &mut usize, indent: usize, first: &str) -> String {
    let mut raw = first.to_string();
    while *i < lines.len() && lines[*i].0 > indent {
        raw.push(' ');
        raw.push_str(&lines[*i].1);
        *i += 1;
    }
    unquote(&raw)
}

fn block(lines: &mut Lines, i: &mut usize, indent: usize) -> Yaml {
    if is_item(&lines[*i].1) { seq(lines, i, indent) } else { map(lines, i, indent) }
}

fn seq(lines: &mut Lines, i: &mut usize, indent: usize) -> Yaml {
    let mut out = Vec::new();
    while *i < lines.len() && lines[*i].0 == indent && is_item(&lines[*i].1) {
        let text = lines[*i].1.clone();
        let rest = text[1..].trim_start().to_string();
        let col = indent + (text.len() - rest.len());
        if rest.is_empty() {
            *i += 1;
            if *i < lines.len() && lines[*i].0 > indent {
                let ind = lines[*i].0;
                out.push(block(lines, i, ind));
            } else {
                out.push(Yaml::Str(String::new()));
            }
        } else if is_item(&rest) {
            lines[*i] = (col, rest);
            out.push(seq(lines, i, col));
        } else if split_key(&rest).is_some() {
            lines[*i] = (col, rest);
            out.push(map(lines, i, col));
        } else {
            *i += 1;
            out.push(Yaml::Str(scalar(lines, i, indent, &rest)));
        }
    }
    Yaml::Seq(out)
}

fn map(lines: &mut Lines, i: &mut usize, indent: usize) -> Yaml {
    let mut out = Vec::new();
    while *i < lines.len() && lines[*i].0 == indent && !is_item(&lines[*i].1) {
        let Some((k, v)) = split_key(&lines[*i].1) else {
            *i += 1;
            continue;
        };
        *i += 1;
        if v.is_empty() {
            let nested = *i < lines.len() && (lines[*i].0 > indent || (lines[*i].0 == indent && is_item(&lines[*i].1)));
            if nested {
                let ind = lines[*i].0;
                out.push((k, block(lines, i, ind)));
            } else {
                out.push((k, Yaml::Str(String::new())));
            }
        } else if v.starts_with('|') || v.starts_with('>') {
            let fold = v.starts_with('>');
            let mut parts: Vec<String> = Vec::new();
            while *i < lines.len() && lines[*i].0 > indent {
                parts.push(lines[*i].1.clone());
                *i += 1;
            }
            out.push((k, Yaml::Str(parts.join(if fold { " " } else { "\n" }))));
        } else {
            out.push((k, Yaml::Str(scalar(lines, i, indent, &v))));
        }
    }
    Yaml::Map(out)
}

// ───────────────────────────── DEP-11 ─────────────────────────────

/// Composant AppStream lu dans DEP-11 ou dans l'AppStream XML de Flatpak.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    pub license: String,
    pub developer: String,
    pub homepage: String,
    pub icon_stock: Option<String>,
    pub icon_url: Option<String>,
    pub screenshots: Vec<Screenshot>,
    pub launchable: Option<String>,
}

fn join_url(base: &str, path: &str) -> String {
    if path.contains("://") || base.is_empty() { path.to_string() } else { format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/')) }
}

/// Composant DEP-11 du paquet `package` dans le texte d'un fichier `Components-<arch>.yml`.
/// Plusieurs composants pour un paquet : l'application d'abord.
pub fn dep11_component(text: &str, package: &str, langs: &[String]) -> Option<Component> {
    // En-tête (« File: DEP-11 », MediaBaseUrl) : premier document, après « %YAML 1.2 ».
    let mut docs = text.split("\n---\n").skip_while(|d| !d.contains("File: DEP-11") && d.trim_start().starts_with('%'));
    let header = parse_yaml(docs.next().unwrap_or(""));
    let base = header.get("MediaBaseUrl").and_then(Yaml::str).unwrap_or("").to_string();
    let needles = [format!("\nPackage: {package}\n"), format!("\nPackage: \"{package}\"\n"), format!("\nPackage: '{package}'\n")];
    let mut found: Vec<Yaml> = Vec::new();
    for d in docs {
        let probe = format!("\n{d}\n");
        if needles.iter().any(|n| probe.contains(n.as_str())) {
            found.push(parse_yaml(d));
        }
    }
    let rank = |y: &Yaml| match y.get("Type").and_then(Yaml::str) {
        Some("desktop-application") => 0,
        Some("console-application") => 1,
        Some("web-application") => 2,
        _ => 3,
    };
    found.sort_by_key(rank);
    let y = found.into_iter().next()?;
    let tr = |key: &str| y.get(key).map(|v| pick_lang(&v.translations(), langs).unwrap_or("").to_string()).unwrap_or_default();
    let developer = y
        .get("Developer")
        .and_then(|d| d.get("name"))
        .map(|n| pick_lang(&n.translations(), langs).unwrap_or("").to_string())
        .or_else(|| y.get("DeveloperName").map(|n| pick_lang(&n.translations(), langs).unwrap_or("").to_string()))
        .unwrap_or_default();
    let icon = y.get("Icon");
    let icon_url = icon
        .and_then(|i| i.get("remote"))
        .map(|r| r.seq())
        .and_then(|list| {
            list.iter()
                .max_by_key(|e| e.get("width").and_then(Yaml::str).and_then(|w| w.parse::<u32>().ok()).unwrap_or(0))
                .and_then(|e| e.get("url").and_then(Yaml::str))
        })
        .map(|u| join_url(&base, u));
    let screenshots = y
        .get("Screenshots")
        .map(|s| s.seq())
        .unwrap_or(&[])
        .iter()
        .filter_map(|s| {
            let full = s.get("source-image").and_then(|i| i.get("url")).and_then(Yaml::str).map(|u| join_url(&base, u));
            // Vignette la plus grande sous 1300 px de large.
            let thumb = s
                .get("thumbnails")
                .map(|t| t.seq())
                .unwrap_or(&[])
                .iter()
                .filter_map(|t| {
                    let w = t.get("width").and_then(Yaml::str).and_then(|w| w.parse::<u32>().ok()).unwrap_or(0);
                    t.get("url").and_then(Yaml::str).map(|u| (w, u))
                })
                .filter(|(w, _)| *w <= 1300)
                .max_by_key(|(w, _)| *w)
                .map(|(_, u)| join_url(&base, u));
            let full = full.or(thumb.clone())?;
            let caption = s.get("caption").map(|c| pick_lang(&c.translations(), langs).unwrap_or("").to_string()).unwrap_or_default();
            Some(Screenshot { thumbnail: thumb.unwrap_or_else(|| full.clone()), full, caption })
        })
        .take(8)
        .collect();
    let launchable = y
        .get("Launchable")
        .and_then(|l| l.get("desktop-id"))
        .and_then(|d| d.seq().first().and_then(Yaml::str).map(String::from).or_else(|| d.str().map(String::from)));
    Some(Component {
        id: y.get("ID").and_then(Yaml::str).unwrap_or("").to_string(),
        name: tr("Name"),
        summary: tr("Summary"),
        description: plain(&tr("Description")),
        license: y.get("ProjectLicense").and_then(Yaml::str).unwrap_or("").to_string(),
        developer,
        homepage: y.get("Url").and_then(|u| u.get("homepage")).and_then(Yaml::str).unwrap_or("").to_string(),
        icon_stock: icon.and_then(|i| i.get("stock")).and_then(Yaml::str).map(String::from),
        icon_url,
        screenshots,
        launchable,
    })
}

/// Fichiers DEP-11 de la machine pour l'architecture `arch`.
pub fn dep11_files(arch: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir("/var/lib/apt/lists") else { return vec![] };
    let mut files: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            n.contains("_dep11_Components-") && n.contains(&format!("-{arch}.yml"))
        })
        .collect();
    files.sort();
    files
}

// ───────────────────────────── AppStream XML (Flatpak) ─────────────────────────────

/// Éléments `<tag …>contenu</tag>` (sans imbrication du même nom) : (attributs, contenu).
pub fn xml_elements<'a>(xml: &'a str, tag: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // `<name>` mais pas `<name_x>` ni `<namespace>`.
        if !after.starts_with([' ', '>', '/', '\n', '\t']) {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        let attrs = &after[..gt];
        if attrs.ends_with('/') {
            out.push((attrs.trim_end_matches('/'), ""));
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push((attrs, &body[..end]));
        rest = &body[end + close.len()..];
    }
    out
}

pub fn xml_attr(attrs: &str, name: &str) -> Option<String> {
    let pat = format!("{name}=");
    let pos = attrs.find(&pat)? + pat.len();
    let q = attrs[pos..].chars().next()?;
    let v = &attrs[pos + 1..];
    Some(unescape_xml(&v[..v.find(q)?]))
}

/// Retire les blocs `<tag>…</tag>` (versions, développeur…) pour ne garder que les champs
/// directs du composant.
fn without(xml: &str, tags: &[&str]) -> String {
    let mut s = xml.to_string();
    for tag in tags {
        let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
        while let Some(a) = s.find(&open) {
            let next = s[a + open.len()..].chars().next();
            if !matches!(next, Some(' ' | '>' | '\n' | '\t' | '/')) {
                break;
            }
            let end = match s[a..].find(&close) {
                Some(e) => a + e + close.len(),
                None => match s[a..].find("/>") {
                    Some(e) => a + e + 2,
                    None => break,
                },
            };
            s.replace_range(a..end, "");
        }
    }
    s
}

/// Composant `id` d'un AppStream XML (catalogue d'un remote Flatpak).
pub fn appstream_xml_component(xml: &str, id: &str, langs: &[String]) -> Option<Component> {
    let needle = format!("<id>{id}</id>");
    let at = xml.find(&needle)?;
    let start = xml[..at].rfind("<component")?;
    let end = at + xml[at..].find("</component>")?;
    let comp = &xml[start..end];
    let localized = |body: &str, tag: &str| -> String {
        let values: Vec<(String, String)> = xml_elements(body, tag)
            .into_iter()
            .map(|(attrs, inner)| (xml_attr(attrs, "xml:lang").unwrap_or_else(|| "C".into()), inner.to_string()))
            .collect();
        pick_lang(&values, langs).unwrap_or("").to_string()
    };
    let direct = without(comp, &["releases", "developer", "screenshots", "keywords", "content_rating", "requires", "recommends", "supports", "languages", "categories", "branding", "custom", "provides"]);
    let mut description = plain(&localized(&direct, "description"));
    if description.is_empty() {
        description = String::new();
    }
    let developer = xml_elements(comp, "developer")
        .first()
        .map(|(_, d)| plain(&localized(d, "name")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| plain(&localized(&direct, "developer_name")));
    let homepage = xml_elements(&direct, "url")
        .into_iter()
        .find(|(a, _)| xml_attr(a, "type").as_deref() == Some("homepage"))
        .map(|(_, u)| unescape_xml(u.trim()))
        .unwrap_or_default();
    let screenshots = xml_elements(comp, "screenshot")
        .into_iter()
        .filter_map(|(_, body)| {
            let imgs = xml_elements(body, "image");
            let source = imgs.iter().find(|(a, _)| xml_attr(a, "type").as_deref() != Some("thumbnail")).map(|(_, u)| unescape_xml(u.trim()));
            let thumb = imgs
                .iter()
                .filter(|(a, _)| xml_attr(a, "type").as_deref() == Some("thumbnail"))
                .filter_map(|(a, u)| Some((xml_attr(a, "width")?.parse::<u32>().ok()?, u)))
                .filter(|(w, _)| *w <= 1300)
                .max_by_key(|(w, _)| *w)
                .map(|(_, u)| unescape_xml(u.trim()));
            let full = source.or(thumb.clone())?;
            Some(Screenshot { thumbnail: thumb.unwrap_or_else(|| full.clone()), full, caption: plain(&localized(body, "caption")) })
        })
        .take(8)
        .collect();
    let icon_stock = xml_elements(&direct, "icon").into_iter().find(|(a, _)| xml_attr(a, "type").as_deref() == Some("stock")).map(|(_, v)| v.trim().to_string());
    let icon_url = xml_elements(&direct, "icon").into_iter().find(|(a, _)| xml_attr(a, "type").as_deref() == Some("remote")).map(|(_, v)| unescape_xml(v.trim()));
    let launchable = xml_elements(&direct, "launchable").into_iter().find(|(a, _)| xml_attr(a, "type").as_deref() == Some("desktop-id")).map(|(_, v)| v.trim().to_string());
    Some(Component {
        id: id.to_string(),
        name: plain(&localized(&direct, "name")),
        summary: plain(&localized(&direct, "summary")),
        description,
        license: xml_elements(&direct, "project_license").first().map(|(_, v)| unescape_xml(v.trim())).unwrap_or_default(),
        developer,
        homepage,
        icon_stock,
        icon_url,
        screenshots,
        launchable,
    })
}

// ───────────────────────────── flatpak info / remote-info ─────────────────────────────

/// Sortie de `flatpak remote-info` / `flatpak info` (en `LC_ALL=C`) : 1re ligne
/// « Nom - résumé », puis « Clé: valeur » alignés à droite.
pub fn parse_flatpak_info(out: &str) -> (String, String, Vec<(String, String)>) {
    let mut title = (String::new(), String::new());
    let mut fields = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        match t.split_once(": ") {
            Some((k, v)) if !k.contains(" - ") && k.len() < 30 => fields.push((k.trim().to_string(), v.trim().to_string())),
            _ if title.0.is_empty() && fields.is_empty() => {
                title = match t.split_once(" - ") {
                    Some((n, s)) => (n.trim().to_string(), s.trim().to_string()),
                    None => (t.to_string(), String::new()),
                };
            }
            _ => {}
        }
    }
    (title.0, title.1, fields)
}

/// Fichier AppStream d'un remote Flatpak (`appstream.xml.gz` ou `.xml`).
pub fn flatpak_appstream_file(root: &Path, remote: &str, arch: &str) -> Option<PathBuf> {
    let dir = root.join("appstream").join(remote).join(arch).join("active");
    ["appstream.xml", "appstream.xml.gz"].iter().map(|f| dir.join(f)).find(|p| p.exists())
}

/// Icône locale du cache AppStream de Flatpak (`icons/128x128/<id>.png`).
pub fn flatpak_icon(root: &Path, remote: &str, arch: &str, id: &str) -> Option<PathBuf> {
    let dir = root.join("appstream").join(remote).join(arch).join("active/icons");
    ["128x128", "64x64"].iter().map(|s| dir.join(s).join(format!("{id}.png"))).find(|p| p.exists())
}

// ───────────────────────────── Journal des modifications ─────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangelogEntry {
    pub version: String,
    /// Lignes de l'entrée (« * Nouvelle version »…), sans la signature.
    pub text: String,
    /// Date de la signature (« Sun, 16 Aug 2026 18:12:00 +0200 »).
    pub date: String,
}

/// `debian/changelog` (sortie de `apt-get changelog`) : entrées plus récentes que `since`
/// (version installée), `max` au plus.
pub fn parse_debian_changelog(text: &str, since: Option<&str>, max: usize) -> Vec<ChangelogEntry> {
    let mut out: Vec<ChangelogEntry> = Vec::new();
    let mut cur: Option<ChangelogEntry> = None;
    for line in text.lines() {
        if !line.starts_with(' ') && line.contains(" (") && line.contains(';') {
            let version = line.split_once(" (").and_then(|(_, r)| r.split_once(')')).map(|(v, _)| v.to_string()).unwrap_or_default();
            if since == Some(version.as_str()) || out.len() >= max {
                cur = None;
                break;
            }
            cur = Some(ChangelogEntry { version, ..Default::default() });
        } else if let Some(sig) = line.strip_prefix(" -- ") {
            if let Some(mut e) = cur.take() {
                e.date = sig.rsplit_once("  ").map(|(_, d)| d.trim().to_string()).unwrap_or_default();
                e.text = e.text.trim().to_string();
                out.push(e);
            }
        } else if let Some(e) = cur.as_mut() {
            if !line.trim().is_empty() {
                e.text.push_str(line.strip_prefix("  ").unwrap_or(line));
                e.text.push('\n');
            }
        }
    }
    out
}

// ───────────────────────────── Simulation ─────────────────────────────

/// Effet d'une opération, simulé sans droits root.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Effect {
    /// Paquets supplémentaires installés (dépendances, runtimes), hors le paquet lui-même.
    pub extra: Vec<String>,
    /// Paquets mis à jour au passage.
    pub upgraded: Vec<String>,
    /// Paquets retirés (conflits, ou dépendants lors d'une suppression).
    pub removed: Vec<String>,
}

/// Sortie de `apt-get -s install|remove` : `Inst nom (…)`, `Inst nom [ancienne] (…)`, `Remv nom […]`.
/// `target` (le paquet lui-même) est exclu des listes.
pub fn parse_apt_simulation(stdout: &str, target: &str) -> Effect {
    let mut e = Effect::default();
    for line in stdout.lines() {
        let mut words = line.split_whitespace();
        let (Some(kind), Some(name)) = (words.next(), words.next()) else { continue };
        let name = name.split(':').next().unwrap_or(name).to_string();
        if name == target {
            continue;
        }
        match kind {
            // « Inst nom [ancienne] (nouvelle …) » : le crochet juste après le nom = mise à jour
            // (« [amd64] » en fin de ligne n'en est pas une).
            "Inst" if words.next().is_some_and(|w| w.starts_with('[')) => e.upgraded.push(name),
            "Inst" => e.extra.push(name),
            "Remv" => e.removed.push(name),
            _ => {}
        }
    }
    e
}

/// Explication lisible d'un refus d'apt (dépendances non satisfaites…).
pub fn apt_refusal(stdout: &str, stderr: &str) -> String {
    let explain: Vec<&str> = stdout
        .lines()
        .skip_while(|l| !l.contains("unmet dependencies"))
        .chain(stderr.lines().filter(|l| l.starts_with("E:")))
        .collect();
    if explain.is_empty() { "apt refuse cette opération".to_string() } else { explain.join("\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOW: &str = "Package: gimp\nSource: gimp (3.2.8-1)\nVersion: 3.2.8-1+b1\nInstalled-Size: 33386\n\
Maintainer: Debian GNOME Extras Maintainers <pkg-gnome-extras-maintainers@lists.alioth.debian.org>\n\
Architecture: amd64\nDepends: gimp-data (<= 3.2.8-z), gimp-data (>= 3.2.8), libc6 (>= 2.43), python3-gi | python3-gi-alt:any, xdg-utils\n\
Recommends: ghostscript\nSuggests: gimp-data-extras, gimp-help-en | gimp-help, gvfs-backends\n\
Description-fr: programme de manipulation d’image GIMP\n GIMP est un éditeur d'images perfectionné. Il permet\n d'éditer des photos.\n .\n Formats pris en charge :\n  * JPEG ;\n  * PNG.\n\
Description-md5: 0c13253910a0a1bd77c64d38c07351a0\nHomepage: https://www.gimp.org/\n\
Commands: gimp, gimp-3, gimp-console\nTag: field::arts, implemented-in::c,\n interface::x11\nSection: graphics\nPriority: optional\n\
Filename: pool/main/g/gimp/gimp_3.2.8-1+b1_amd64.deb\nSize: 7143876\n\n\
Package: gimp\nVersion: 3.0.4-1\nInstalled-Size: 30000\nDescription-fr: ancienne\n .\nSize: 100\n\n";

    #[test]
    fn apt_cache_show() {
        let st = parse_show(SHOW);
        assert_eq!(st.len(), 2);
        let s = pick_stanza(&st, &[Some("3.2.8-1+b1"), None]).unwrap();
        let d = details_from_stanza(s);
        assert_eq!(d.id, "gimp");
        assert_eq!(d.summary, "programme de manipulation d’image GIMP");
        assert_eq!(d.description, "GIMP est un éditeur d'images perfectionné. Il permet d'éditer des photos.\n\nFormats pris en charge :\n* JPEG ;\n* PNG.");
        assert_eq!(d.download_size, Some(7143876));
        assert_eq!(d.installed_size, Some(33386 * 1024));
        assert_eq!(d.section, "graphics");
        assert_eq!(d.homepage, "https://www.gimp.org/");
        assert_eq!(d.depends, ["gimp-data", "libc6", "python3-gi | python3-gi-alt", "xdg-utils"]);
        assert_eq!(d.recommends, ["ghostscript"]);
        assert_eq!(d.suggests, ["gimp-data-extras", "gimp-help-en | gimp-help", "gvfs-backends"]);
        assert_eq!(d.commands, ["gimp", "gimp-3", "gimp-console"]);
        assert!(d.maintainer.starts_with("Debian GNOME Extras"));
        // Version installée plus ancienne, plus de candidate : son paragraphe.
        let old = pick_stanza(&st, &[None, Some("3.0.4-1")]).unwrap();
        assert_eq!(details_from_stanza(old).download_size, Some(100));
        // Version inconnue : le premier paragraphe.
        assert_eq!(field(pick_stanza(&st, &[Some("9")]).unwrap(), "Version").unwrap().value, "3.2.8-1+b1");
    }

    #[test]
    fn description_sans_traduction() {
        let st = parse_show("Package: noos\nVersion: 1\nDescription: Bureau Wayland NOOS\n Première ligne\n suite.\nDescription-md5: \n");
        let d = details_from_stanza(&st[0]);
        assert_eq!((d.summary.as_str(), d.description.as_str()), ("Bureau Wayland NOOS", "Première ligne suite."));
        assert_eq!(d.download_size, None);
    }

    #[test]
    fn depot_et_licence() {
        assert_eq!(repository_label("500 http://deb.debian.org/debian sid/main amd64 Packages"), "deb.debian.org/debian · sid/main");
        assert_eq!(repository_label("500 https://chomiam.github.io/noos-de/ sid/main amd64 Packages"), "chomiam.github.io/noos-de · sid/main");
        let c = "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\nUpstream-Name: x\n\nFiles: debian/*\nLicense: GPL-2+\n\nFiles: *\nCopyright: 2020 X\nLicense: MIT\n";
        assert_eq!(license_from_copyright(c).as_deref(), Some("MIT"));
        assert_eq!(license_from_copyright("This package was debianized"), None);
    }

    #[test]
    fn tailles_et_relations() {
        assert_eq!(parse_size("102.5 MB"), Some(102_500_000));
        assert_eq!(parse_size("1,5 GB"), Some(1_500_000_000));
        assert_eq!(parse_size("512 bytes"), Some(512));
        assert_eq!(parse_size("3 MiB"), Some(3 * 1024 * 1024));
        assert_eq!(parse_size("?"), None);
        assert_eq!(parse_relations(""), Vec::<String>::new());
    }

    #[test]
    fn langues() {
        let v: Vec<(String, String)> = vec![("C".into(), "Image editor".into()), ("fr".into(), "Éditeur".into())];
        let langs = vec!["fr_FR".to_string(), "fr-FR".into(), "fr".into(), "C".into()];
        assert_eq!(pick_lang(&v, &langs), Some("Éditeur"));
        assert_eq!(pick_lang(&v, &["de".to_string()]), Some("Image editor"));
    }

    const DEP11: &str = "%YAML 1.2\n---\nFile: DEP-11\nVersion: \"1.0\"\nOrigin: debian-sid-main\nMediaBaseUrl: https://appstream.debian.org/media/sid\n\
---\nType: desktop-application\nID: org.example.Other\nPackage: other\nName:\n  C: Other\n\
---\nType: addon\nID: org.gimp.GIMP.Plugin\nPackage: gimp\nName:\n  C: Plugin\n\
---\nType: desktop-application\nID: org.gimp.GIMP\nPackage: gimp\nProjectLicense: GPL-3.0+ AND LGPL-3.0+\nName:\n  C: GNU Image Manipulation Program\n  fr: Éditeur d’image GIMP\n\
Summary:\n  C: High-end image creation and manipulation\nDescription:\n  C: >-\n    <p>GIMP is an acronym for GNU Image Manipulation Program.</p>\n\n    <ul>\n      <li>Paint</li>\n      <li>Retouch</li>\n    </ul>\n  \
nl: |-\n    <p>GIMP is een acroniem.</p>\nDeveloper:\n  id: gimp.org\n  name:\n    C: The GIMP Team\n    de: Das GIMP-Team\n\
Url:\n  homepage: https://www.gimp.org/\n  help: https://www.gimp.org/docs/\nIcon:\n  cached:\n  - name: gimp_gimp.jxl\n    width: 64\n    height: 64\n  \
remote:\n  - url: org/gimp/GIMP/4981/icons/64x64/gimp_gimp.jxl\n    width: 64\n    height: 64\n  - url: org/gimp/GIMP/4981/icons/128x128/gimp_gimp.jxl\n    width: 128\n    height: 128\n  stock: gimp\n\
Launchable:\n  desktop-id:\n  - gimp.desktop\nScreenshots:\n- default: true\n  caption:\n    C: Scene 4 cut 15 of \"ZeMarmot\" being edited in GIMP\n    sk: 'Scéna 4, strih ''15'''\n  \
thumbnails:\n  - url: org/gimp/GIMP/4981/screenshots/image-1_752x423@1.jxl\n    width: 752\n    height: 423\n  - url: org/gimp/GIMP/4981/screenshots/image-1_1248x702@1.jxl\n    width: 1248\n    height: 702\n  \
- url: org/gimp/GIMP/4981/screenshots/image-1_1600x900@1.jxl\n    width: 1600\n    height: 900\n  source-image:\n    url: org/gimp/GIMP/4981/screenshots/image-1_orig.jxl\n    width: 1920\n    height: 1080\n\
- caption:\n    C: \"Second \\\"shot\\\"\"\n  source-image:\n    url: org/gimp/GIMP/4981/screenshots/image-2_orig.png\n    width: 800\n    height: 600\n\
Releases:\n- version: 3.2.8\n  unix-timestamp: 1700000000\n---\nType: desktop-application\nID: \"2048.desktop\"\nPackage: \"2048\"\nName:\n  C: \"2048\"\n";

    #[test]
    fn yaml_dep11() {
        let langs = vec!["fr".to_string(), "C".into()];
        let c = dep11_component(DEP11, "gimp", &langs).unwrap();
        assert_eq!(c.id, "org.gimp.GIMP");
        assert_eq!(c.name, "Éditeur d’image GIMP");
        assert_eq!(c.summary, "High-end image creation and manipulation");
        assert_eq!(c.description, "GIMP is an acronym for GNU Image Manipulation Program.\n\n • Paint\n • Retouch");
        assert_eq!(c.license, "GPL-3.0+ AND LGPL-3.0+");
        assert_eq!(c.developer, "The GIMP Team");
        assert_eq!(c.homepage, "https://www.gimp.org/");
        assert_eq!(c.icon_stock.as_deref(), Some("gimp"));
        assert_eq!(c.icon_url.as_deref(), Some("https://appstream.debian.org/media/sid/org/gimp/GIMP/4981/icons/128x128/gimp_gimp.jxl"));
        assert_eq!(c.launchable.as_deref(), Some("gimp.desktop"));
        assert_eq!(c.screenshots.len(), 2);
        assert_eq!(c.screenshots[0].thumbnail, "https://appstream.debian.org/media/sid/org/gimp/GIMP/4981/screenshots/image-1_1248x702@1.jxl");
        assert_eq!(c.screenshots[0].full, "https://appstream.debian.org/media/sid/org/gimp/GIMP/4981/screenshots/image-1_orig.jxl");
        assert_eq!(c.screenshots[0].caption, "Scene 4 cut 15 of \"ZeMarmot\" being edited in GIMP");
        assert_eq!(c.screenshots[1].thumbnail, c.screenshots[1].full);
        assert_eq!(c.screenshots[1].caption, "Second \"shot\"");
        // Nom de paquet entre guillemets.
        assert_eq!(dep11_component(DEP11, "2048", &langs).unwrap().name, "2048");
        assert!(dep11_component(DEP11, "absent", &langs).is_none());
    }

    #[test]
    fn yaml_icone() {
        let y = parse_yaml("Icon:\n  cached:\n  - name: a.jxl\n    width: 64\n  remote:\n  - url: x/a.jxl\n    width: 64\n  stock: gimp\nNext: 1\n");
        assert_eq!(y.get("Icon").and_then(|i| i.get("stock")).and_then(Yaml::str), Some("gimp"));
    }

    #[test]
    fn yaml_sous_ensemble() {
        let y = parse_yaml("a: 1\nb:\n- x\n- y: 2\n  z: 'it''s'\n- - n1\n  - n2\nc: long\n  suite\nd: |\n  l1\n  l2\n");
        assert_eq!(y.get("a").and_then(Yaml::str), Some("1"));
        let b = y.get("b").unwrap().seq();
        assert_eq!(b[0], Yaml::Str("x".into()));
        assert_eq!(b[1].get("z").and_then(Yaml::str), Some("it's"));
        assert_eq!(b[2].seq().len(), 2);
        assert_eq!(y.get("c").and_then(Yaml::str), Some("long suite"));
        assert_eq!(y.get("d").and_then(Yaml::str), Some("l1\nl2"));
    }

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<components version="0.8" origin="flathub">
  <component type="desktop-application">
    <id>org.other.App</id>
    <name>Other</name>
  </component>
  <component type="desktop-application">
    <id>org.gimp.GIMP</id>
    <name>GNU Image Manipulation Program</name>
    <name xml:lang="fr">Éditeur d’image GIMP</name>
    <summary>High-end image creation &amp; manipulation</summary>
    <project_license>GPL-3.0+ AND LGPL-3.0+</project_license>
    <description xml:lang="de"><p>Deutsch</p></description>
    <description>
      <p>GIMP is an acronym for GNU Image Manipulation Program.</p>
      <ul><li>Paint</li><li>Retouch</li></ul>
    </description>
    <icon type="cached" width="128" height="128">org.gimp.GIMP.png</icon>
    <icon type="remote" width="128" height="128">https://dl.flathub.org/media/org/gimp/GIMP/icon.png</icon>
    <launchable type="desktop-id">org.gimp.GIMP.desktop</launchable>
    <url type="bugtracker">https://gitlab.gnome.org/GNOME/gimp/issues</url>
    <url type="homepage">https://www.gimp.org/</url>
    <developer id="org.gimp"><name>The GIMP Team</name><name xml:lang="de">Das GIMP-Team</name></developer>
    <screenshots>
      <screenshot type="default">
        <caption>Editing &quot;ZeMarmot&quot;</caption>
        <image type="source" width="1920" height="1080">https://dl.flathub.org/media/shot1_orig.png</image>
        <image type="thumbnail" width="624" height="351">https://dl.flathub.org/media/shot1_624.png</image>
        <image type="thumbnail" width="1248" height="702">https://dl.flathub.org/media/shot1_1248.png</image>
      </screenshot>
    </screenshots>
    <releases>
      <release version="3.2.8" timestamp="1700000000"><description><p>Corrections</p></description></release>
    </releases>
  </component>
</components>"#;

    #[test]
    fn appstream_xml() {
        let langs = vec!["fr".to_string(), "C".into()];
        let c = appstream_xml_component(XML, "org.gimp.GIMP", &langs).unwrap();
        assert_eq!(c.name, "Éditeur d’image GIMP");
        assert_eq!(c.summary, "High-end image creation & manipulation");
        assert_eq!(c.description, "GIMP is an acronym for GNU Image Manipulation Program.\n\n • Paint\n • Retouch");
        assert_eq!(c.license, "GPL-3.0+ AND LGPL-3.0+");
        assert_eq!(c.developer, "The GIMP Team");
        assert_eq!(c.homepage, "https://www.gimp.org/");
        assert_eq!(c.icon_url.as_deref(), Some("https://dl.flathub.org/media/org/gimp/GIMP/icon.png"));
        assert_eq!(c.launchable.as_deref(), Some("org.gimp.GIMP.desktop"));
        assert_eq!(c.screenshots.len(), 1);
        assert_eq!(c.screenshots[0].thumbnail, "https://dl.flathub.org/media/shot1_1248.png");
        assert_eq!(c.screenshots[0].full, "https://dl.flathub.org/media/shot1_orig.png");
        assert_eq!(c.screenshots[0].caption, "Editing \"ZeMarmot\"");
        assert!(appstream_xml_component(XML, "org.absent", &langs).is_none());
    }

    #[test]
    fn flatpak_info() {
        let out = "\nÉditeur d’image GIMP - High-end image creation and manipulation\n\n            ID: org.gimp.GIMP\n           Ref: app/org.gimp.GIMP/x86_64/stable\n\
       Version: 3.2.8\n       License: GPL-3.0+ AND LGPL-3.0+\n Download Size: 102.5 MB\nInstalled Size: 268.6 MB\n       Runtime: org.gnome.Platform/x86_64/51\n\n        Commit: fc90\n       Subject: openexr: Update v3.5.3.tar.gz to 3.5.4 (#786)\n";
        let (name, summary, f) = parse_flatpak_info(out);
        assert_eq!(name, "Éditeur d’image GIMP");
        assert_eq!(summary, "High-end image creation and manipulation");
        let get = |k: &str| f.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Download Size"), Some("102.5 MB"));
        assert_eq!(get("Runtime"), Some("org.gnome.Platform/x86_64/51"));
        assert_eq!(get("Subject"), Some("openexr: Update v3.5.3.tar.gz to 3.5.4 (#786)"));
    }

    #[test]
    fn simulation() {
        let out = "NOTE: This is only a simulation!\nInst gimp-data (3.2.8-1 Debian:unstable [all])\nInst libfoo1 [1.0-1] (1.1-1 Debian:unstable [amd64])\nInst gimp (3.2.8-1+b1 Debian:unstable [amd64])\nRemv oldpkg [2.0]\nConf gimp-data (3.2.8-1 Debian:unstable [all])\n";
        let e = parse_apt_simulation(out, "gimp");
        assert_eq!(e.extra, ["gimp-data"]);
        assert_eq!(e.upgraded, ["libfoo1"]);
        assert_eq!(e.removed, ["oldpkg"]);
        let r = parse_apt_simulation("Remv gimp [3.2.8-1+b1]\nRemv gimp-plugin [1]\n", "gimp");
        assert_eq!(r.removed, ["gimp-plugin"]);
    }

    #[test]
    fn journal_debian() {
        let t = "htop (3.5.3-1) unstable; urgency=medium\n\n  * New upstream release\n    - détail\n\n -- Daniel Lange <DLange@debian.org>  Sun, 16 Aug 2026 18:12:00 +0200\n\n\
htop (3.5.2-1) unstable; urgency=medium\n\n  * Add Netlink\n\n -- Daniel Lange <DLange@debian.org>  Sat, 18 Jul 2026 17:30:00 +0200\n\n\
htop (3.5.1-1) unstable; urgency=medium\n\n  * Old\n\n -- D <d@d>  Mon, 1 Jun 2026 10:00:00 +0200\n";
        let e = parse_debian_changelog(t, Some("3.5.1-1"), 10);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].version, "3.5.3-1");
        assert_eq!(e[0].text, "* New upstream release\n  - détail");
        assert_eq!(e[0].date, "Sun, 16 Aug 2026 18:12:00 +0200");
        assert_eq!(parse_debian_changelog(t, None, 1).len(), 1);
        assert!(crate::needs_reboot(&["vim", "linux-image-6.17-amd64"]));
        assert!(!crate::needs_reboot(&["vim", "htop"]));
    }

    #[test]
    fn texte_appstream() {
        assert_eq!(plain("<p>Un &amp; deux</p><p>Trois <em>quatre</em></p>"), "Un & deux\n\nTrois quatre");
    }
}

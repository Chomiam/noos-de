//! Miroirs Debian (Logiciels › Sources) : liste officielle (`Mirrors.masterlist` de
//! mirror-master.debian.org, gardée 7 jours), test de débit (1 Mio de `Packages.xz` de sid,
//! sans root) et changement de miroir dans les sources APT (pkexec, puis `apt-get update`).

use std::{path::PathBuf, time::Duration};

use crate::{Plan, backends::aptsources, exec::capture_codes};

pub const MASTERLIST: &str = "https://mirror-master.debian.org/status/Mirrors.masterlist";
/// Réseau de distribution de Debian (choisit lui-même un serveur proche) : toujours proposé.
pub const CDN: &str = "http://deb.debian.org/debian";

#[derive(Debug, Clone, PartialEq)]
pub struct Mirror {
    pub site: String,
    /// Code pays ISO (« FR »), vide pour le CDN.
    pub country_code: String,
    pub country: String,
    /// Adresse de l'archive : « http://ftp.fr.debian.org/debian ».
    pub url: String,
    pub sponsor: String,
}

/// Drapeau d'un code pays (« FR » → 🇫🇷) ; globe pour le CDN.
pub fn flag(cc: &str) -> String {
    if cc.len() != 2 || !cc.chars().all(|c| c.is_ascii_alphabetic()) {
        return "🌐".into();
    }
    cc.to_ascii_uppercase().chars().filter_map(|c| char::from_u32(0x1F1E6 + (c as u32 - 'A' as u32))).collect()
}

/// `Mirrors.masterlist` : miroirs HTTP de l'archive pour amd64.
pub fn parse_masterlist(text: &str) -> Vec<Mirror> {
    let mut out = Vec::new();
    for stanza in text.split("\n\n") {
        let get = |k: &str| stanza.lines().find_map(|l| l.strip_prefix(&format!("{k}: ")).map(|v| v.trim().to_string()));
        let (Some(site), Some(path)) = (get("Site"), get("Archive-http")) else { continue };
        if get("Archive-architecture").is_some_and(|a| !a.split_whitespace().any(|x| x == "amd64" || x == "any")) {
            continue;
        }
        let country = get("Country").unwrap_or_default();
        let (cc, name) = country.split_once(' ').map(|(c, n)| (c.to_string(), n.to_string())).unwrap_or((String::new(), country.clone()));
        out.push(Mirror {
            url: format!("http://{site}{}", path.trim_end_matches('/')),
            site,
            country_code: cc,
            country: name,
            sponsor: get("Sponsor").map(|s| s.split(" http").next().unwrap_or(&s).trim().to_string()).unwrap_or_default(),
        });
    }
    out
}

fn cache_file() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".cache"));
    base.join("noos-software/Mirrors.masterlist")
}

/// Liste des miroirs : le CDN, puis ceux du pays de la session, puis les autres.
pub fn load() -> Vec<Mirror> {
    let file = cache_file();
    let fresh = file.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e < Duration::from_secs(7 * 86400)));
    if !fresh {
        if let Some(d) = file.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = file.with_extension("part");
        let ok = std::process::Command::new("curl").args(["-fsSL", "--max-time", "30", "-o"]).arg(&tmp).arg(MASTERLIST).status().is_ok_and(|s| s.success());
        if ok {
            let _ = std::fs::rename(&tmp, &file);
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    let mut list = parse_masterlist(&std::fs::read_to_string(&file).unwrap_or_default());
    let cc = session_country();
    list.sort_by_key(|m| (m.country_code != cc, m.country.clone(), m.site.clone()));
    list.insert(0, Mirror { site: "deb.debian.org".into(), country_code: String::new(), country: "Réseau mondial (CDN)".into(), url: CDN.into(), sponsor: "Fastly".into() });
    list
}

/// Pays de la session d'après la langue (« fr_FR » → « FR »).
pub fn session_country() -> String {
    crate::details::languages().into_iter().find_map(|l| l.split_once('_').map(|(_, c)| c.to_uppercase())).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed {
    /// Établissement de la connexion (ms).
    pub latency_ms: u32,
    /// Débit (octets par seconde).
    pub bytes_per_s: f64,
}

/// Sortie de `curl -w "%{time_connect} %{speed_download}"`.
pub fn parse_curl_speed(out: &str) -> Option<Speed> {
    let mut it = out.split_whitespace();
    let c: f64 = it.next()?.replace(',', ".").parse().ok()?;
    let s: f64 = it.next()?.replace(',', ".").parse().ok()?;
    (s > 0.0).then_some(Speed { latency_ms: (c * 1000.0).round() as u32, bytes_per_s: s })
}

/// Télécharge 1 Mio de l'index des paquets de sid (au plus 10 s).
pub fn test(mirror: &Mirror) -> anyhow::Result<Speed> {
    let url = format!("{}/dists/sid/main/binary-amd64/Packages.xz", mirror.url);
    let out = capture_codes(
        &["curl", "-fsS", "-o", "/dev/null", "--max-time", "10", "-r", "0-1048575", "-w", "%{time_connect} %{speed_download}", &url],
        &[0],
    )?;
    parse_curl_speed(&out).ok_or_else(|| anyhow::anyhow!("mesure impossible"))
}

/// Miroir Debian utilisé aujourd'hui (première source Debian hors sécurité).
pub fn current() -> Option<String> {
    aptsources::entries()
        .into_iter()
        .filter(|e| e.enabled)
        .flat_map(|e| e.uris)
        .find(|u| aptsources::is_debian_uri(u) && !u.contains("security") && u.trim_end_matches('/').ends_with("/debian"))
        .map(|u| u.trim_end_matches('/').to_string())
}

/// Remplace l'adresse d'un miroir (mot entier, avec ou sans « / » final) dans un fichier de
/// sources (une ligne ou deb822).
pub fn replace_uri(text: &str, old: &str, new: &str) -> String {
    let old = old.trim_end_matches('/');
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let replaced: Vec<String> = line
            .split(' ')
            .map(|tok| if tok.trim_end_matches('/') == old { new.to_string() } else { tok.to_string() })
            .collect();
        out.push(replaced.join(" "));
    }
    let mut s = out.join("\n");
    if text.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Plan : toutes les sources Debian passent de l'ancien miroir au nouveau, puis `apt-get update`.
pub fn switch_plan(new_url: &str) -> anyhow::Result<Plan> {
    let old = current().ok_or_else(|| anyhow::anyhow!("aucune source Debian trouvée"))?;
    let mut plan = Plan::new();
    let mut files: Vec<String> = aptsources::entries().into_iter().filter(|e| e.uris.iter().any(|u| u.trim_end_matches('/') == old)).map(|e| e.file).collect();
    files.dedup();
    for f in files {
        let text = std::fs::read_to_string(&f)?;
        let new_text = replace_uri(&text, &old, new_url);
        if new_text != text {
            plan = plan.write(&f, &new_text);
        }
    }
    Ok(plan.root(&["apt-get", "update"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = "Site: ftp.fr.debian.org\nCountry: FR France\nArchive-architecture: amd64 arm64 i386\nArchive-http: /debian/\nSponsor: Ecole Polytechnique http://x\n\n\
Site: mirror.example.jp\nCountry: JP Japan\nArchive-architecture: arm64\nArchive-http: /debian/\n\n\
Site: debian.example.de\nCountry: DE Germany\nArchive-http: /pub/debian/\n\nSite: cdimage-only.org\nCountry: US United States\nCDImage-http: /cd/\n";

    #[test]
    fn liste_et_drapeaux() {
        let m = parse_masterlist(LIST);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].url, "http://ftp.fr.debian.org/debian");
        assert_eq!((m[0].country_code.as_str(), m[0].country.as_str(), m[0].sponsor.as_str()), ("FR", "France", "Ecole Polytechnique"));
        assert_eq!(m[1].url, "http://debian.example.de/pub/debian");
        assert_eq!(flag("FR"), "🇫🇷");
        assert_eq!(flag(""), "🌐");
    }

    #[test]
    fn mesure_et_remplacement() {
        let s = parse_curl_speed("0.023412 1843200.000").unwrap();
        assert_eq!(s.latency_ms, 23);
        assert!(parse_curl_speed("0.1 0").is_none());
        let one = "deb http://deb.debian.org/debian sid main\ndeb http://deb.debian.org/debian-security x main\n# deb http://deb.debian.org/debian/ experimental main\n";
        assert_eq!(
            replace_uri(one, "http://deb.debian.org/debian", "http://ftp.fr.debian.org/debian"),
            "deb http://ftp.fr.debian.org/debian sid main\ndeb http://deb.debian.org/debian-security x main\n# deb http://ftp.fr.debian.org/debian experimental main\n"
        );
        let d822 = "Types: deb\nURIs: http://deb.debian.org/debian\nSuites: sid\n";
        assert!(replace_uri(d822, "http://deb.debian.org/debian/", "https://m/debian").contains("URIs: https://m/debian\n"));
    }
}

//! Données de la vitrine Flathub (API publique v2) avec cache disque.
//!
//! - collections : popular, trending, recently-added, recently-updated, category/<cat>
//! - sélections : apps-of-the-week, app-of-the-day
//! - fiche : appstream/<app_id> (description, captures, couleurs de marque)
//!
//! Les requêtes passent par `curl` (présent partout, y compris sur Silverblue) et sont mises
//! en cache dans `~/.cache/noos-software` (6 h pour les listes, 30 jours pour les images).

use std::{
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime},
};

use serde_json::Value;

const API: &str = "https://flathub.org/api/v2";

#[derive(Debug, Clone, Default)]
pub struct AppSummary {
    pub app_id: String,
    pub name: String,
    pub summary: String,
    pub icon_url: String,
    pub developer: String,
    pub installs_last_month: u64,
    pub verified: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AppDetails {
    pub summary: AppSummary,
    pub description: String,
    pub screenshots: Vec<String>,
    pub brand_light: Option<String>,
    pub brand_dark: Option<String>,
    pub license: String,
    pub homepage: String,
    pub version: String,
}

/// Catégories principales de Flathub : (identifiant API, libellé, icône).
pub const CATEGORIES: &[(&str, &str, &str)] = &[
    ("Game", "Jeux", "input-gaming-symbolic"),
    ("AudioVideo", "Multimédia", "applications-multimedia-symbolic"),
    ("Graphics", "Graphisme", "applications-graphics-symbolic"),
    ("Office", "Bureautique", "x-office-document-symbolic"),
    ("Network", "Internet", "network-wireless-symbolic"),
    ("Development", "Développement", "applications-engineering-symbolic"),
    ("Education", "Éducation", "accessories-dictionary-symbolic"),
    ("Science", "Sciences", "applications-science-symbolic"),
    ("System", "Système", "applications-system-symbolic"),
    ("Utility", "Utilitaires", "applications-utilities-symbolic"),
];

pub fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())).join(".cache"));
    base.join("noos-software")
}

fn fresh(path: &std::path::Path, ttl: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < ttl)
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect()
}

/// GET JSON avec cache ; en cas d'échec réseau, renvoie la copie en cache même périmée.
fn get_json(path: &str, ttl: Duration) -> anyhow::Result<Value> {
    let dir = cache_dir().join("api");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{}.json", safe_name(path)));
    if !fresh(&file, ttl) {
        let out = Command::new("curl")
            .args(["-fsSL", "--max-time", "20", "-H", "Accept: application/json", &format!("{API}/{path}")])
            .output();
        match out {
            Ok(o) if o.status.success() && !o.stdout.is_empty() => {
                let tmp = file.with_extension("tmp");
                std::fs::write(&tmp, &o.stdout)?;
                std::fs::rename(&tmp, &file)?;
            }
            Ok(o) if !file.exists() => anyhow::bail!("Flathub injoignable : {}", String::from_utf8_lossy(&o.stderr).trim()),
            Err(e) if !file.exists() => anyhow::bail!("curl introuvable : {e}"),
            _ => {}
        }
    }
    Ok(serde_json::from_slice(&std::fs::read(&file)?)?)
}

/// Télécharge une image (icône, capture) dans le cache et renvoie son chemin local.
pub fn cached_image(url: &str) -> Option<PathBuf> {
    if !url.starts_with("https://") {
        return None;
    }
    let dir = cache_dir().join("img");
    std::fs::create_dir_all(&dir).ok()?;
    let name = url.trim_start_matches("https://").replace('/', "_");
    let file = dir.join(safe_name(&name));
    if fresh(&file, Duration::from_secs(30 * 24 * 3600)) {
        return Some(file);
    }
    let tmp = file.with_extension("part");
    let ok = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", "-o"])
        .arg(&tmp)
        .arg(url)
        .status()
        .is_ok_and(|s| s.success());
    if ok && std::fs::rename(&tmp, &file).is_ok() {
        Some(file)
    } else {
        let _ = std::fs::remove_file(&tmp);
        file.exists().then_some(file)
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn summary_from_hit(h: &Value) -> AppSummary {
    AppSummary {
        app_id: s(h, "app_id"),
        name: s(h, "name"),
        summary: s(h, "summary"),
        icon_url: s(h, "icon"),
        developer: s(h, "developer_name"),
        installs_last_month: h.get("installs_last_month").and_then(|x| x.as_u64()).unwrap_or(0),
        verified: h.get("verification_verified").and_then(|x| x.as_bool()).unwrap_or(false),
    }
}

/// Collection (« popular », « trending », « recently-added », « recently-updated »…).
pub fn collection(name: &str, count: usize) -> anyhow::Result<Vec<AppSummary>> {
    let v = get_json(&format!("collection/{name}?page=1&per_page={count}"), Duration::from_secs(6 * 3600))?;
    Ok(v["hits"].as_array().into_iter().flatten().map(summary_from_hit).filter(|a| !a.app_id.is_empty()).collect())
}

pub fn category(cat: &str, count: usize) -> anyhow::Result<Vec<AppSummary>> {
    collection(&format!("category/{cat}"), count)
}

fn today() -> String {
    // Date UTC sans dépendance : jours depuis l'époque → AAAA-MM-JJ (algorithme de H. Hinnant).
    let days = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() / 86400).unwrap_or(0) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Applications mises en avant cette semaine (pour la bannière).
pub fn apps_of_the_week() -> anyhow::Result<Vec<String>> {
    let v = get_json(&format!("app-picks/apps-of-the-week/{}", today()), Duration::from_secs(12 * 3600))?;
    let mut apps: Vec<(i64, String)> = v["apps"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| (a["position"].as_i64().unwrap_or(99), s(a, "app_id")))
        .filter(|(_, id)| !id.is_empty())
        .collect();
    apps.sort();
    Ok(apps.into_iter().map(|(_, id)| id).collect())
}

pub fn app_of_the_day() -> anyhow::Result<String> {
    Ok(s(&get_json(&format!("app-picks/app-of-the-day/{}", today()), Duration::from_secs(12 * 3600))?, "app_id"))
}

/// Texte brut d'une description AppStream (Flathub renvoie du HTML simplifié).
fn plain(desc: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in desc.chars() {
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let t = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_lowercase();
                match t.as_str() {
                    "p" if !tag.starts_with('/') && !out.is_empty() => out.push_str("\n\n"),
                    "li" if !tag.starts_with('/') => out.push_str("\n • "),
                    "br" => out.push('\n'),
                    _ => {}
                }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let lines: Vec<String> = out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
    let mut text = lines.join("\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").trim().to_string()
}

pub fn details(app_id: &str) -> anyhow::Result<AppDetails> {
    let v = get_json(&format!("appstream/{app_id}"), Duration::from_secs(24 * 3600))?;
    let mut brand_light = None;
    let mut brand_dark = None;
    for b in v["branding"].as_array().into_iter().flatten() {
        match b["scheme_preference"].as_str() {
            Some("dark") => brand_dark = b["value"].as_str().map(String::from),
            _ => brand_light = b["value"].as_str().map(String::from),
        }
    }
    // Capture de taille raisonnable (≈ 600–1300 px de large).
    let screenshots = v["screenshots"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|shot| {
            let sizes = shot["sizes"].as_array()?;
            let pick = sizes
                .iter()
                .filter(|z| z["width"].as_str().and_then(|w| w.parse::<u32>().ok()).is_some_and(|w| w <= 1300))
                .max_by_key(|z| z["width"].as_str().and_then(|w| w.parse::<u32>().ok()).unwrap_or(0))
                .or(sizes.first())?;
            pick["src"].as_str().map(String::from)
        })
        .take(6)
        .collect();
    Ok(AppDetails {
        summary: AppSummary {
            app_id: app_id.into(),
            name: s(&v, "name"),
            summary: s(&v, "summary"),
            icon_url: s(&v, "icon"),
            developer: s(&v, "developer_name"),
            installs_last_month: 0,
            verified: v["metadata"]["flathub::verification::verified"].as_str() == Some("true"),
        },
        description: plain(&s(&v, "description")),
        screenshots,
        brand_light,
        brand_dark,
        license: s(&v, "project_license"),
        homepage: v["urls"]["homepage"].as_str().unwrap_or("").to_string(),
        version: v["releases"].as_array().and_then(|r| r.first()).map(|r| s(r, "version")).unwrap_or_default(),
    })
}

pub fn format_installs(n: u64) -> String {
    match n {
        0 => String::new(),
        n if n >= 1_000_000 => format!("{:.1} M installations/mois", n as f64 / 1e6),
        n if n >= 1_000 => format!("{} k installations/mois", n / 1000),
        n => format!("{n} installations/mois"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text() {
        let d = "<p>Hello &amp; welcome</p><ul><li>One</li><li>Two</li></ul><p>End</p>";
        assert_eq!(plain(d), "Hello & welcome\n• One\n• Two\n\nEnd");
    }

    #[test]
    fn date_format() {
        let t = today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20"));
    }
}

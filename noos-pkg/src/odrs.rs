//! Notes et avis des utilisateurs : ODRS (Open Desktop Ratings Service, odrs.gnome.org), le
//! service de GNOME Logiciels et KDE Discover. Désactivé par défaut (`[software] reviews`).
//! L'identifiant envoyé est un hachage local (machine + compte), sans lien avec le nom réel.

use serde::Deserialize;

use crate::exec::capture_codes;

pub const API: &str = "https://odrs.gnome.org/1.0/reviews/api";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Ratings {
    #[serde(default)]
    pub star0: u64,
    #[serde(default)]
    pub star1: u64,
    #[serde(default)]
    pub star2: u64,
    #[serde(default)]
    pub star3: u64,
    #[serde(default)]
    pub star4: u64,
    #[serde(default)]
    pub star5: u64,
    #[serde(default)]
    pub total: u64,
}

impl Ratings {
    pub fn stars(&self) -> [u64; 5] {
        [self.star1, self.star2, self.star3, self.star4, self.star5]
    }
    /// Moyenne sur 5 (les notes « 0 étoile » ne comptent pas).
    pub fn average(&self) -> Option<f64> {
        let s = self.stars();
        let n: u64 = s.iter().sum();
        (n > 0).then(|| s.iter().enumerate().map(|(i, c)| (i as u64 + 1) * c).sum::<u64>() as f64 / n as f64)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Review {
    #[serde(default)]
    pub review_id: u64,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    /// 0 à 100.
    #[serde(default)]
    pub rating: u32,
    #[serde(default)]
    pub user_display: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub date_created: f64,
    #[serde(default)]
    pub karma_up: i64,
    #[serde(default)]
    pub user_skey: Option<String>,
}

impl Review {
    pub fn stars(&self) -> u32 {
        (self.rating + 10) / 20
    }
}

/// Identifiant anonyme : SHA-1 de « noos-software[compte:machine-id] ».
pub fn user_hash() -> String {
    let machine = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
    let user = std::env::var("USER").unwrap_or_default();
    let input = format!("noos-software[{user}:{}]", machine.trim());
    let out = std::process::Command::new("sha1sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().map(|mut i| i.write_all(input.as_bytes()));
            c.wait_with_output()
        });
    out.map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap_or("").to_string()).unwrap_or_default()
}

fn post(path: &str, body: &serde_json::Value) -> anyhow::Result<String> {
    let out = std::process::Command::new("curl")
        .args(["-fsS", "--max-time", "20", "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-"])
        .arg(format!("{API}/{path}"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().map(|mut i| i.write_all(body.to_string().as_bytes()));
            c.wait_with_output()
        })?;
    if !out.status.success() {
        anyhow::bail!("odrs.gnome.org : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn parse_ratings(json: &str) -> Option<Ratings> {
    serde_json::from_str(json).ok()
}

pub fn parse_reviews(json: &str) -> Vec<Review> {
    serde_json::from_str(json).unwrap_or_default()
}

fn locale() -> String {
    crate::details::languages().into_iter().find(|l| l.contains('_')).unwrap_or_else(|| "fr_FR".into())
}

pub fn ratings(app_id: &str) -> anyhow::Result<Ratings> {
    let out = capture_codes(&["curl", "-fsS", "--max-time", "20", &format!("{API}/ratings/{app_id}")], &[0])?;
    parse_ratings(&out).ok_or_else(|| anyhow::anyhow!("réponse illisible"))
}

/// Avis dans la langue de la session ; renvoie aussi la clé de publication (`user_skey`).
pub fn fetch(app_id: &str, version: &str, limit: u32) -> anyhow::Result<(Vec<Review>, Option<String>)> {
    let ids = [app_id.to_string(), format!("{}.desktop", app_id.trim_end_matches(".desktop"))];
    let body = serde_json::json!({
        "user_hash": user_hash(), "app_id": app_id, "compat_ids": ids, "locale": locale(),
        "distro": "Debian", "version": version, "limit": limit,
    });
    let list = parse_reviews(&post("fetch", &body)?);
    let skey = list.iter().find_map(|r| r.user_skey.clone());
    Ok((list, skey))
}

/// Publie un avis (note 1 à 5 étoiles).
pub fn submit(app_id: &str, skey: &str, version: &str, stars: u32, summary: &str, description: &str, name: &str) -> anyhow::Result<()> {
    let body = serde_json::json!({
        "user_hash": user_hash(), "user_skey": skey, "app_id": app_id, "locale": locale(), "distro": "Debian",
        "version": version, "rating": stars.clamp(1, 5) * 20, "summary": summary, "description": description,
        "user_display": if name.trim().is_empty() { "Anonyme" } else { name.trim() },
    });
    post("submit", &body).map(|_| ())
}

/// « upvote », « downvote » ou « report » (signaler un avis abusif).
pub fn vote(action: &str, app_id: &str, skey: &str, review_id: u64) -> anyhow::Result<()> {
    let body = serde_json::json!({ "user_hash": user_hash(), "user_skey": skey, "app_id": app_id, "review_id": review_id });
    post(action, &body).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_et_avis() {
        let r = parse_ratings(r#"{"star0":0,"star1":137,"star2":47,"star3":38,"star4":77,"star5":444,"total":743}"#).unwrap();
        assert_eq!(r.total, 743);
        let avg = r.average().unwrap();
        assert!((avg - 3.867).abs() < 0.001, "{avg}");
        let list = parse_reviews(r#"[{"review_id":1,"summary":"Bien","description":"Très bien","rating":80,"user_display":null,"version":"3.2.2","date_created":1794431726.0,"user_skey":"abc"}]"#);
        assert_eq!(list[0].stars(), 4);
        assert_eq!(list[0].user_skey.as_deref(), Some("abc"));
        assert!(parse_reviews("pas du json").is_empty());
        assert_eq!(Ratings::default().average(), None);
    }
}

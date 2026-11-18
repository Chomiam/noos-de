//! Météo du widget de bureau : Open-Meteo (https://open-meteo.com, sans compte ni clé).
//!
//! - prévisions : `api.open-meteo.com/v1/forecast` (conditions actuelles + 6 jours) ;
//! - villes : `geocoding-api.open-meteo.com/v1/search?name=…&language=fr`.
//!
//! Requêtes avec `curl` (comme Bing / Wallhaven), à faire hors du thread de l'interface. La
//! dernière réponse est gardée dans `~/.cache/noos/weather/` (affichage hors ligne).

use std::{path::PathBuf, process::Command};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Day {
    /// AAAA-MM-JJ
    pub date: String,
    pub code: u32,
    pub max: f64,
    pub min: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Weather {
    pub temp: f64,
    pub feels: f64,
    pub humidity: f64,
    /// km/h (mph en °F).
    pub wind: f64,
    pub code: u32,
    pub is_day: bool,
    pub days: Vec<Day>,
    /// Heure de la mesure (secondes Unix, à la réception).
    pub fetched: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    /// Région, pays.
    pub detail: String,
    pub lat: f64,
    pub lon: f64,
}

/// Libellé et famille d'une condition (code WMO).
pub fn describe(code: u32) -> &'static str {
    match code {
        0 => "Ciel dégagé",
        1 => "Plutôt dégagé",
        2 => "Partiellement nuageux",
        3 => "Couvert",
        45 | 48 => "Brouillard",
        51 | 53 | 55 => "Bruine",
        56 | 57 => "Bruine verglaçante",
        61 => "Pluie faible",
        63 => "Pluie",
        65 => "Forte pluie",
        66 | 67 => "Pluie verglaçante",
        71 => "Neige faible",
        73 => "Neige",
        75 => "Fortes chutes de neige",
        77 => "Grains de neige",
        80 => "Averses",
        81 => "Averses",
        82 => "Violentes averses",
        85 | 86 => "Averses de neige",
        95 => "Orage",
        96 | 99 => "Orage et grêle",
        _ => "Conditions inconnues",
    }
}

/// Ambiance (classe CSS du fond du widget) : `clear`, `night`, `cloudy`, `rain`, `snow`,
/// `storm`, `fog`.
pub fn mood(code: u32, is_day: bool) -> &'static str {
    match code {
        0 | 1 if !is_day => "night",
        0 | 1 => "clear",
        2 | 3 if !is_day => "night",
        2 | 3 => "cloudy",
        45 | 48 => "fog",
        71..=77 | 85 | 86 => "snow",
        95..=99 => "storm",
        51..=67 | 80..=82 => "rain",
        _ => "cloudy",
    }
}

/// Icône symbolique (thème Adwaita).
pub fn icon(code: u32, is_day: bool) -> &'static str {
    match code {
        0 | 1 if is_day => "weather-clear-symbolic",
        0 | 1 => "weather-clear-night-symbolic",
        2 if is_day => "weather-few-clouds-symbolic",
        2 => "weather-few-clouds-night-symbolic",
        3 => "weather-overcast-symbolic",
        45 | 48 => "weather-fog-symbolic",
        51..=57 | 61 | 80 => "weather-showers-scattered-symbolic",
        63..=67 | 81 | 82 => "weather-showers-symbolic",
        71..=77 | 85 | 86 => "weather-snow-symbolic",
        95..=99 => "weather-storm-symbolic",
        _ => "weather-overcast-symbolic",
    }
}

#[derive(Deserialize)]
struct RawForecast {
    current: RawCurrent,
    daily: RawDaily,
}

#[derive(Deserialize)]
struct RawCurrent {
    temperature_2m: f64,
    #[serde(default)]
    apparent_temperature: f64,
    #[serde(default)]
    relative_humidity_2m: f64,
    #[serde(default)]
    is_day: u8,
    weather_code: u32,
    #[serde(default)]
    wind_speed_10m: f64,
}

#[derive(Deserialize)]
struct RawDaily {
    time: Vec<String>,
    weather_code: Vec<u32>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
}

/// Lit une réponse de `v1/forecast`.
pub fn parse_forecast(json: &str, fetched: u64) -> anyhow::Result<Weather> {
    let r: RawForecast = serde_json::from_str(json).context("réponse météo illisible")?;
    let d = &r.daily;
    let days = (0..d.time.len())
        .filter_map(|i| Some(Day { date: d.time[i].clone(), code: *d.weather_code.get(i)?, max: *d.temperature_2m_max.get(i)?, min: *d.temperature_2m_min.get(i)? }))
        .collect();
    let c = r.current;
    Ok(Weather { temp: c.temperature_2m, feels: c.apparent_temperature, humidity: c.relative_humidity_2m, wind: c.wind_speed_10m, code: c.weather_code, is_day: c.is_day == 1, days, fetched })
}

#[derive(Deserialize)]
struct RawPlaces {
    #[serde(default)]
    results: Vec<RawPlace>,
}

#[derive(Deserialize)]
struct RawPlace {
    name: String,
    latitude: f64,
    longitude: f64,
    #[serde(default)]
    country: String,
    #[serde(default)]
    admin1: String,
}

/// Lit une réponse de `v1/search`.
pub fn parse_places(json: &str) -> anyhow::Result<Vec<Place>> {
    let r: RawPlaces = serde_json::from_str(json).context("réponse de recherche illisible")?;
    Ok(r
        .results
        .into_iter()
        .map(|p| {
            let detail = [p.admin1, p.country].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
            Place { name: p.name, detail, lat: p.latitude, lon: p.longitude }
        })
        .collect())
}

/// Encodage d'un paramètre d'URL.
pub fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn curl(url: &str) -> anyhow::Result<String> {
    let out = Command::new("curl").args(["-fsSL", "--max-time", "20", "-A", "NOOS", url]).output().context("curl introuvable")?;
    if !out.status.success() {
        bail!("pas de réponse du service météo");
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn forecast_url(lat: f64, lon: f64, fahrenheit: bool) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.3}&longitude={lon:.3}\
         &current=temperature_2m,apparent_temperature,relative_humidity_2m,is_day,weather_code,wind_speed_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=6{}",
        if fahrenheit { "&temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "" }
    )
}

fn cache_file(lat: f64, lon: f64, fahrenheit: bool) -> PathBuf {
    paths::cache_dir().join("weather").join(format!("{lat:.2}_{lon:.2}{}.json", if fahrenheit { "_f" } else { "" }))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Prévisions (réseau), gardées en cache.
pub fn fetch(lat: f64, lon: f64, fahrenheit: bool) -> anyhow::Result<Weather> {
    let w = parse_forecast(&curl(&forecast_url(lat, lon, fahrenheit))?, now())?;
    let f = cache_file(lat, lon, fahrenheit);
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(f, serde_json::to_string(&w).unwrap_or_default());
    Ok(w)
}

/// Dernières prévisions reçues (hors ligne).
pub fn cached(lat: f64, lon: f64, fahrenheit: bool) -> Option<Weather> {
    serde_json::from_str(&std::fs::read_to_string(cache_file(lat, lon, fahrenheit)).ok()?).ok()
}

/// Recherche de villes (en français).
pub fn search(name: &str) -> anyhow::Result<Vec<Place>> {
    parse_places(&curl(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=6&language=fr&format=json", urlencode(name.trim())))?)
}

/// Données fictives (`NOOS_WEATHER_DEMO=1`, captures).
pub fn demo() -> Weather {
    parse_forecast(DEMO, now()).expect("exemple valide")
}

const DEMO: &str = r#"{"current":{"temperature_2m":14.2,"apparent_temperature":12.6,"relative_humidity_2m":62,"is_day":1,"weather_code":2,"wind_speed_10m":14.8},
"daily":{"time":["2026-11-18","2026-11-19","2026-11-20","2026-11-21","2026-11-22","2026-11-23"],"weather_code":[2,61,3,0,71,95],
"temperature_2m_max":[15.1,12.4,11.0,13.8,4.2,9.7],"temperature_2m_min":[7.3,6.8,5.1,3.9,-1.4,4.0]}}"#;

/// Jour de la semaine abrégé d'une date AAAA-MM-JJ (« lun. »…), calcul de Zeller.
pub fn weekday_short(date: &str) -> String {
    let mut it = date.split('-').filter_map(|p| p.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d)) = (it.next(), it.next(), it.next()) else { return String::new() };
    let (y, m) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
    let h = (d + 13 * (m + 1) / 5 + y + y / 4 - y / 100 + y / 400) % 7; // 0 = samedi
    ["sam.", "dim.", "lun.", "mar.", "mer.", "jeu.", "ven."][h as usize].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previsions() {
        let w = demo();
        assert_eq!(w.temp, 14.2);
        assert!(w.is_day);
        assert_eq!(w.days.len(), 6);
        assert_eq!(w.days[4].min, -1.4);
        assert_eq!(describe(w.code), "Partiellement nuageux");
        assert_eq!(icon(w.code, w.is_day), "weather-few-clouds-symbolic");
        assert_eq!(mood(0, false), "night");
        assert_eq!(mood(63, true), "rain");
        assert_eq!(mood(75, true), "snow");
        assert_eq!(icon(99, true), "weather-storm-symbolic");
    }

    #[test]
    fn villes() {
        let p = parse_places(r#"{"results":[{"name":"Lyon","latitude":45.75,"longitude":4.85,"country":"France","admin1":"Auvergne-Rhône-Alpes"},{"name":"Lyons","latitude":43.0,"longitude":-77.0,"country":"États-Unis"}]}"#).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].detail, "Auvergne-Rhône-Alpes, France");
        assert_eq!(p[1].detail, "États-Unis");
        assert!(parse_places("{}").unwrap().is_empty());
        assert_eq!(urlencode("Saint-Étienne du Rouvray"), "Saint-%C3%89tienne%20du%20Rouvray");
    }

    /// Service réel (réseau) : `cargo test -p noos-core -- --ignored reseau`.
    #[test]
    #[ignore]
    fn reseau() {
        let p = search("Paris").unwrap();
        assert!(p[0].detail.contains("France"), "{p:?}");
        let w = fetch(p[0].lat, p[0].lon, false).unwrap();
        assert_eq!(w.days.len(), 6);
        assert!(w.temp > -40.0 && w.temp < 50.0);
        assert_eq!(cached(p[0].lat, p[0].lon, false), Some(w));
    }

    #[test]
    fn jours() {
        assert_eq!(weekday_short("2026-11-18"), "mer.");
        assert_eq!(weekday_short("2027-01-01"), "ven.");
        assert_eq!(weekday_short("2024-02-29"), "jeu.");
    }
}

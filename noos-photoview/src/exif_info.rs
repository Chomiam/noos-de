//! Informations de prise de vue (EXIF) : appareil, objectif, exposition, ouverture, ISO,
//! focale, date, GPS. Lecture en Rust pur (kamadak-exif), en arrière-plan.

use std::path::{Path, PathBuf};

use exif::{In, Tag, Value};
use gtk::gio;

#[derive(Debug, Clone, Default)]
pub struct Shot {
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub date: Option<String>,
    pub exposure: Option<String>,
    pub aperture: Option<String>,
    pub iso: Option<String>,
    pub focal: Option<String>,
    pub flash: Option<String>,
    /// (latitude, longitude) en degrés décimaux.
    pub gps: Option<(f64, f64)>,
}

impl Shot {
    pub fn is_empty(&self) -> bool {
        self.camera.is_none() && self.exposure.is_none() && self.date.is_none() && self.gps.is_none()
    }
}

fn text(e: &exif::Exif, tag: Tag) -> Option<String> {
    let f = e.get_field(tag, In::PRIMARY)?;
    let s = f.display_value().to_string();
    let s = s.trim().trim_matches('"').trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn rational(e: &exif::Exif, tag: Tag) -> Option<f64> {
    match &e.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(v) => v.first().map(|r| r.to_f64()),
        Value::SRational(v) => v.first().map(|r| r.to_f64()),
        _ => None,
    }
}

fn gps_coord(e: &exif::Exif, tag: Tag, reference: Tag) -> Option<f64> {
    let Value::Rational(v) = &e.get_field(tag, In::PRIMARY)?.value else { return None };
    if v.len() < 3 {
        return None;
    }
    let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
    let r = text(e, reference).unwrap_or_default();
    Some(if r.starts_with('S') || r.starts_with('W') { -deg } else { deg })
}

/// « 2024:06:14 18:03:22 » → « 14/06/2024 à 18:03 ».
fn nice_date(raw: &str) -> String {
    let raw = raw.trim();
    let (d, t) = raw.split_once(' ').unwrap_or((raw, ""));
    let p: Vec<&str> = d.split([':', '-']).collect();
    if p.len() == 3 {
        let hm: String = t.chars().take(5).collect();
        format!("{}/{}/{}{}", p[2], p[1], p[0], if hm.is_empty() { String::new() } else { format!(" à {hm}") })
    } else {
        raw.to_string()
    }
}

fn read_blocking(path: &Path) -> Option<Shot> {
    let file = std::fs::File::open(path).ok()?;
    let e = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)).ok()?;
    let make = text(&e, Tag::Make).unwrap_or_default();
    let model = text(&e, Tag::Model).unwrap_or_default();
    // Le modèle répète souvent la marque (« Canon Canon EOS R6 ») : on évite le doublon.
    let camera = if model.to_lowercase().starts_with(&make.to_lowercase()) || make.is_empty() {
        model
    } else {
        format!("{make} {model}")
    };
    let exposure = rational(&e, Tag::ExposureTime).map(|t| {
        if t >= 1.0 { format!("{t:.1} s") } else if t > 0.0 { format!("1/{:.0} s", 1.0 / t) } else { String::new() }
    });
    let focal = rational(&e, Tag::FocalLength).map(|f| {
        let eq = e
            .get_field(Tag::FocalLengthIn35mmFilm, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
            .filter(|&v| v > 0)
            .map(|v| format!(" (équiv. {v} mm)"))
            .unwrap_or_default();
        format!("{f:.0} mm{eq}")
    });
    let flash = e.get_field(Tag::Flash, In::PRIMARY).and_then(|f| f.value.get_uint(0)).map(|v| {
        if v & 1 == 1 { "Déclenché".to_string() } else { "Non déclenché".to_string() }
    });
    let shot = Shot {
        camera: (!camera.is_empty()).then_some(camera),
        lens: text(&e, Tag::LensModel),
        date: text(&e, Tag::DateTimeOriginal).or_else(|| text(&e, Tag::DateTime)).map(|d| nice_date(&d)),
        exposure: exposure.filter(|s| !s.is_empty()),
        aperture: rational(&e, Tag::FNumber).filter(|f| *f > 0.0).map(|f| format!("ƒ/{f:.1}")),
        iso: e
            .get_field(Tag::PhotographicSensitivity, In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
            .map(|v| format!("ISO {v}")),
        focal,
        flash,
        gps: gps_coord(&e, Tag::GPSLatitude, Tag::GPSLatitudeRef).zip(gps_coord(&e, Tag::GPSLongitude, Tag::GPSLongitudeRef)),
    };
    Some(shot)
}

pub async fn read(path: PathBuf) -> Option<Shot> {
    gio::spawn_blocking(move || read_blocking(&path)).await.ok().flatten()
}

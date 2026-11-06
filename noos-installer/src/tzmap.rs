//! Carte du monde interactive pour choisir le fuseau horaire.
//!
//! Fond de carte et villes : données de libtimezonemap (`/usr/share/libtimezonemap`,
//! GPL-3.0, la carte des installateurs Ubuntu et Anaconda), recolorées aux couleurs de
//! NOOS. Projection de Miller tronquée (81° N – 59° S), décalée de 6° comme l'image.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::{gdk, gdk_pixbuf, glib, prelude::*};

const DATA: &str = "/usr/share/libtimezonemap";
const MAP_W: f64 = 1671.12;
const MAP_H: f64 = 856.49;

#[derive(Debug, Clone)]
pub struct City {
    pub name: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
    pub population: u64,
}

impl City {
    pub fn label(&self) -> String {
        if self.country.is_empty() { self.name.clone() } else { format!("{}, {}", self.name, self.country) }
    }
}

pub fn lon_to_x(lon: f64, w: f64) -> f64 {
    w * (180.0 + lon) / 360.0 + w * -6.0 / 180.0
}

pub fn lat_to_y(lat: f64, h: f64) -> f64 {
    let top_lat: f64 = 81.0;
    let bottom_lat: f64 = -59.0;
    let miller = |lat: f64| 1.25 * (std::f64::consts::FRAC_PI_4 + 0.4 * lat.to_radians()).tan().ln();
    let full_range = 4.6068250867599998;
    let top_offset = full_range * top_lat / 180.0;
    let map_range = (miller(bottom_lat) - top_offset).abs();
    (miller(lat) - top_offset).abs() / map_range * h
}

fn wrap_x(x: f64, w: f64) -> f64 {
    x.rem_euclid(w)
}

/// Coordonnées ISO 6709 de zone1970.tab : ±DDMM[SS]±DDDMM[SS].
fn iso6709(s: &str) -> Option<(f64, f64)> {
    let split = s[1..].find(['+', '-'])? + 1;
    let (a, b) = s.split_at(split);
    let parse = |p: &str, deg_len: usize| -> Option<f64> {
        let sign = if p.starts_with('-') { -1.0 } else { 1.0 };
        let d = &p[1..];
        let deg: f64 = d.get(..deg_len)?.parse().ok()?;
        let min: f64 = d.get(deg_len..deg_len + 2)?.parse().ok()?;
        let sec: f64 = d.get(deg_len + 2..deg_len + 4).and_then(|x| x.parse().ok()).unwrap_or(0.0);
        Some(sign * (deg + min / 60.0 + sec / 3600.0))
    };
    Some((parse(a, 2)?, parse(b, 3)?))
}

/// Villes : chefs-lieux de fuseaux (zone1970.tab) + villes de plus de 100 000 habitants.
pub fn load_cities() -> Vec<City> {
    let countries: HashMap<String, String> = std::fs::read_to_string(format!("{DATA}/countryInfo.txt"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some((f.first()?.to_string(), french_country(f.get(4)?).to_string()))
        })
        .collect();
    let mut cities = Vec::new();
    for line in std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab").unwrap_or_default().lines() {
        if line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        let Some((lat, lon)) = iso6709(f[1]) else { continue };
        let cc = f[0].split(',').next().unwrap_or("");
        let name = f[2].rsplit('/').next().unwrap_or(f[2]).replace('_', " ");
        cities.push(City {
            name,
            country: countries.get(cc).cloned().unwrap_or_default(),
            lat,
            lon,
            tz: f[2].to_string(),
            population: u64::MAX,
        });
    }
    for line in std::fs::read_to_string(format!("{DATA}/citiesInfo.txt")).unwrap_or_default().lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 18 {
            continue;
        }
        let pop: u64 = f[14].parse().unwrap_or(0);
        if pop < 100_000 {
            continue;
        }
        let (Ok(lat), Ok(lon)) = (f[4].parse(), f[5].parse()) else { continue };
        cities.push(City {
            name: f[1].to_string(),
            country: countries.get(f[8]).cloned().unwrap_or_default(),
            lat,
            lon,
            tz: f[17].to_string(),
            population: pop,
        });
    }
    cities
}

fn french_country(en: &str) -> &str {
    match en {
        "France" => "France",
        "Belgium" => "Belgique",
        "Switzerland" => "Suisse",
        "Canada" => "Canada",
        "Germany" => "Allemagne",
        "Spain" => "Espagne",
        "Italy" => "Italie",
        "United Kingdom" => "Royaume-Uni",
        "United States" => "États-Unis",
        "Netherlands" => "Pays-Bas",
        "Portugal" => "Portugal",
        "Brazil" => "Brésil",
        "Japan" => "Japon",
        "China" => "Chine",
        "Morocco" => "Maroc",
        "Algeria" => "Algérie",
        "Tunisia" => "Tunisie",
        "Senegal" => "Sénégal",
        "Luxembourg" => "Luxembourg",
        "Austria" => "Autriche",
        "Poland" => "Pologne",
        "Russia" => "Russie",
        "India" => "Inde",
        "Australia" => "Australie",
        "Mexico" => "Mexique",
        other => other,
    }
}

pub fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            '-' | '_' | '\'' => ' ',
            c => c,
        })
        .collect()
}

/// Carte recolorée : terres (`land`), océans transparents.
fn map_pixbuf(width: i32, land: (u8, u8, u8)) -> Option<gdk_pixbuf::Pixbuf> {
    let src = gdk_pixbuf::Pixbuf::from_file_at_scale(format!("{DATA}/time_zones_countryInfo-orig.svg"), width, -1, true).ok()?;
    let src = src.add_alpha(false, 0, 0, 0).ok()?;
    let (w, h, stride) = (src.width() as usize, src.height() as usize, src.rowstride() as usize);
    // SAFETY : pixbuf neuf (add_alpha), non partagé.
    let px = unsafe { src.pixels() };
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * 4;
            // Océan ≈ (0x60, 0x9e, 0xe8), terres blanches : la composante rouge les distingue.
            let t = ((px[i] as f64 - 0x60 as f64) / (255.0 - 0x60 as f64)).clamp(0.0, 1.0);
            px[i] = land.0;
            px[i + 1] = land.1;
            px[i + 2] = land.2;
            px[i + 3] = (t * 255.0) as u8;
        }
    }
    Some(src)
}

pub struct TzMap {
    pub widget: gtk::DrawingArea,
    pub cities: Rc<Vec<City>>,
    selected: RefCell<Option<City>>,
    hover: RefCell<Option<City>>,
    pixbuf: RefCell<Option<gdk_pixbuf::Pixbuf>>,
    on_select: RefCell<Option<Box<dyn Fn(&City)>>>,
}

impl TzMap {
    pub fn new(cities: Rc<Vec<City>>) -> Rc<Self> {
        let widget = gtk::DrawingArea::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_content_height(360);
        widget.add_css_class("tz-map");
        widget.set_cursor_from_name(Some("crosshair"));
        let map = Rc::new(Self {
            widget,
            cities,
            selected: RefCell::new(None),
            hover: RefCell::new(None),
            pixbuf: RefCell::new(map_pixbuf(1671, (0x58, 0x5b, 0x70))),
            on_select: RefCell::new(None),
        });
        let weak = Rc::downgrade(&map);
        map.widget.set_draw_func(move |w, cr, width, height| {
            if let Some(m) = weak.upgrade() {
                m.draw(w, cr, width as f64, height as f64);
            }
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&map);
        click.connect_released(move |_, _, x, y| {
            if let Some(m) = weak.upgrade() {
                if let Some(c) = m.nearest(x, y) {
                    m.select(&c);
                    if let Some(f) = m.on_select.borrow().as_ref() {
                        f(&c);
                    }
                }
            }
        });
        map.widget.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&map);
        motion.connect_motion(move |_, x, y| {
            if let Some(m) = weak.upgrade() {
                let c = m.nearest(x, y);
                let changed = m.hover.borrow().as_ref().map(|h| &h.tz) != c.as_ref().map(|c| &c.tz)
                    || m.hover.borrow().as_ref().map(|h| &h.name) != c.as_ref().map(|c| &c.name);
                if changed {
                    *m.hover.borrow_mut() = c;
                    m.widget.queue_draw();
                }
            }
        });
        let weak = Rc::downgrade(&map);
        motion.connect_leave(move |_| {
            if let Some(m) = weak.upgrade() {
                m.hover.borrow_mut().take();
                m.widget.queue_draw();
            }
        });
        map.widget.add_controller(motion);
        map
    }

    pub fn connect_select(&self, f: impl Fn(&City) + 'static) {
        *self.on_select.borrow_mut() = Some(Box::new(f));
    }

    pub fn select(&self, c: &City) {
        *self.selected.borrow_mut() = Some(c.clone());
        self.widget.queue_draw();
    }

    /// Zone de dessin de la carte (proportions conservées, centrée).
    fn frame(&self, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let scale = (w / MAP_W).min(h / MAP_H);
        let (mw, mh) = (MAP_W * scale, MAP_H * scale);
        ((w - mw) / 2.0, (h - mh) / 2.0, mw, mh)
    }

    fn project(&self, c: &City, w: f64, h: f64) -> (f64, f64) {
        let (ox, oy, mw, mh) = self.frame(w, h);
        (ox + wrap_x(lon_to_x(c.lon, mw), mw), oy + lat_to_y(c.lat, mh))
    }

    /// Ville la plus proche du point cliqué (les grandes villes d'abord à distance égale).
    fn nearest(&self, x: f64, y: f64) -> Option<City> {
        let (w, h) = (self.widget.width() as f64, self.widget.height() as f64);
        self.cities
            .iter()
            .map(|c| {
                let (cx, cy) = self.project(c, w, h);
                let d = (cx - x).powi(2) + (cy - y).powi(2);
                (d - (c.population.min(20_000_000) as f64).ln() * 4.0, c)
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .filter(|(d, _)| *d < 60.0f64.powi(2))
            .map(|(_, c)| c.clone())
    }

    fn draw(&self, w: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let (ox, oy, mw, mh) = self.frame(width, height);
        if let Some(pb) = self.pixbuf.borrow().as_ref() {
            cr.save().ok();
            cr.translate(ox, oy);
            cr.scale(mw / pb.width() as f64, mh / pb.height() as f64);
            cr.set_source_pixbuf(pb, 0.0, 0.0);
            cr.source().set_filter(gtk::cairo::Filter::Good);
            let _ = cr.paint();
            cr.restore().ok();
        }
        let accent = w.color();
        let (ar, ag, ab) = (accent.red() as f64, accent.green() as f64, accent.blue() as f64);
        let accent = gdk::RGBA::parse("#89b4fa").ok();
        let (ar, ag, ab) = accent.map(|a| (a.red() as f64, a.green() as f64, a.blue() as f64)).unwrap_or((ar, ag, ab));
        let selected = self.selected.borrow().clone();
        // Villes du fuseau choisi : points d'accent discrets.
        if let Some(sel) = &selected {
            cr.set_source_rgba(ar, ag, ab, 0.55);
            for c in self.cities.iter().filter(|c| c.tz == sel.tz) {
                let (x, y) = self.project(c, width, height);
                cr.arc(x, y, 2.2, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
            }
        }
        if let Some(hv) = self.hover.borrow().as_ref() {
            let (x, y) = self.project(hv, width, height);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(1.5);
            cr.arc(x, y, 6.0, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            label(cr, &hv.name, x, y - 12.0, width, (0.12, 0.12, 0.18, 0.85));
        }
        if let Some(sel) = &selected {
            let (x, y) = self.project(sel, width, height);
            // Épingle : halo + point.
            cr.set_source_rgba(ar, ag, ab, 0.30);
            cr.arc(x, y, 14.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgb(ar, ag, ab);
            cr.arc(x, y, 6.5, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            cr.arc(x, y, 6.5, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();
            let text = match glib::DateTime::now(&glib::TimeZone::new(Some(&sel.tz))).ok() {
                Some(now) => format!("{} · {:02}:{:02}", sel.name, now.hour(), now.minute()),
                None => sel.name.clone(),
            };
            label(cr, &text, x, y + 30.0, width, (ar * 0.6, ag * 0.6, ab * 0.6, 0.95));
        }
    }
}

/// Étiquette arrondie centrée en (x, y), gardée dans la carte.
fn label(cr: &gtk::cairo::Context, text: &str, x: f64, y: f64, width: f64, bg: (f64, f64, f64, f64)) {
    cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Bold);
    cr.set_font_size(13.0);
    let Ok(ext) = cr.text_extents(text) else { return };
    let (pw, ph) = (ext.width() + 16.0, 22.0);
    let lx = (x - pw / 2.0).clamp(2.0, (width - pw - 2.0).max(2.0));
    let ly = y - ph / 2.0;
    let r = 8.0;
    cr.new_sub_path();
    cr.arc(lx + pw - r, ly + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(lx + pw - r, ly + ph - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(lx + r, ly + ph - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(lx + r, ly + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
    cr.set_source_rgba(bg.0, bg.1, bg.2, bg.3);
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(lx + 8.0 - ext.x_bearing(), ly + ph / 2.0 - ext.y_bearing() - ext.height() / 2.0);
    let _ = cr.show_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coords() {
        let (lat, lon) = iso6709("+4852+00220").unwrap();
        assert!((lat - 48.866).abs() < 0.01 && (lon - 2.333).abs() < 0.01);
        let (lat, lon) = iso6709("-3352+15113").unwrap();
        assert!(lat < -33.0 && lon > 151.0);
        let (lat, _) = iso6709("+404251-0740023").unwrap();
        assert!((lat - 40.714).abs() < 0.01);
        assert_eq!(normalize("Évry-Courcouronnes"), "evry courcouronnes");
    }
}

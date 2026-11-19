//! Horloges du monde : une ligne par fuseau (cadran clair le jour, sombre la nuit), heure,
//! décalage avec l'heure locale et « Demain » / « Hier ».

use std::f64::consts::PI;

use gtk::{glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, every, section};

pub const CSS: &str = r#"
.desk-world-row { padding: 6px 2px; }
.desk-world-row + .desk-world-row { border-top: 1px solid alpha(currentColor, 0.08); }
.desk-world-city { font-weight: 700; }
.desk-world-off { font-size: 0.82em; opacity: 0.65; }
.desk-world-time { font-size: 1.6em; font-weight: 300; font-feature-settings: "tnum"; }
.size-l .desk-world-time { font-size: 2em; }
.size-s .desk-world-time { font-size: 1.2em; }
.desk-world-zone { padding: 2px 4px; }
.desk-world-bar { margin: 0 2px; }
"#;

const DEFAULT: &str = "America/New_York;Europe/London;Asia/Tokyo";

/// Nom français des villes les plus courantes (sinon : dernier élément de l'identifiant).
fn city(id: &str) -> String {
    let last = id.rsplit('/').next().unwrap_or(id);
    let fr = match last {
        "New_York" => "New York",
        "London" => "Londres",
        "Los_Angeles" => "Los Angeles",
        "Montreal" => "Montréal",
        "Moscow" => "Moscou",
        "Singapore" => "Singapour",
        "Dubai" => "Dubaï",
        "Hong_Kong" => "Hong Kong",
        "Mexico_City" => "Mexico",
        "Sao_Paulo" => "São Paulo",
        "Brussels" => "Bruxelles",
        "Lisbon" => "Lisbonne",
        "Athens" => "Athènes",
        "Cairo" => "Le Caire",
        "Seoul" => "Séoul",
        "Reunion" => "La Réunion",
        "Noumea" => "Nouméa",
        "Algiers" => "Alger",
        "Vienna" => "Vienne",
        "Warsaw" => "Varsovie",
        "Copenhagen" => "Copenhague",
        "Kolkata" => "Calcutta",
        "Buenos_Aires" => "Buenos Aires",
        "Kyiv" | "Kiev" => "Kyiv",
        "Rome" => "Rome",
        "Berlin" => "Berlin",
        "Madrid" => "Madrid",
        "Tahiti" => "Tahiti",
        other => return other.replace('_', " "),
    };
    fr.to_string()
}

/// Fuseaux configurés (`zones` = identifiants séparés par « ; »).
fn zones(w: &DesktopWidget) -> Vec<String> {
    w.option("zones").unwrap_or(DEFAULT).split(';').map(str::trim).filter(|z| !z.is_empty()).map(String::from).collect()
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let t = gtk::Label::new(Some("Horloges du monde"));
    t.add_css_class("desk-title");
    t.set_xalign(0.0);
    t.set_margin_bottom(4);
    if w.size() != "s" {
        b.append(&t);
    }
    let face = by_size(w, 0, 40, 52);
    let small = w.size() == "s";
    let mut ticks: Vec<Box<dyn Fn()>> = vec![];
    for id in zones(w) {
        // Identifiants tirés de la base du système : un nom inconnu donnerait UTC.
        let tz = glib::TimeZone::new(Some(&id));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("desk-world-row");
        let area = gtk::DrawingArea::new();
        if face > 0 {
            area.set_content_width(face);
            area.set_content_height(face);
            row.append(&area);
            let tz2 = tz.clone();
            area.set_draw_func(move |_, cr, wd, ht| {
                if let Ok(now) = glib::DateTime::now(&tz2) {
                    draw_mini(cr, wd as f64, ht as f64, &now);
                }
            });
        }
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.set_hexpand(true);
        col.set_valign(gtk::Align::Center);
        let name = gtk::Label::new(Some(&city(&id)));
        name.add_css_class("desk-world-city");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&name);
        let off = gtk::Label::new(None);
        off.add_css_class("desk-world-off");
        off.set_xalign(0.0);
        off.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&off);
        row.append(&col);
        let time = gtk::Label::new(None);
        time.add_css_class("desk-world-time");
        time.set_valign(gtk::Align::Center);
        row.append(&time);
        // Grande taille : la journée de la ville sur 24 h (nuit, aube, jour, soir) et l'heure
        // actuelle en repère.
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(10);
        bar.add_css_class("desk-world-bar");
        let large = w.size() == "l";
        if large {
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            v.add_css_class("desk-world-row");
            row.remove_css_class("desk-world-row");
            v.append(&row);
            v.append(&bar);
            b.append(&v);
            let tz2 = tz.clone();
            bar.set_draw_func(move |a, cr, wd, ht| {
                if let Ok(now) = glib::DateTime::now(&tz2) {
                    draw_day(a, cr, wd as f64, ht as f64, &now);
                }
            });
        } else {
            b.append(&row);
        }
        ticks.push(Box::new(move || {
            let (Ok(here), Ok(there)) = (glib::DateTime::now_local(), glib::DateTime::now(&tz)) else { return };
            let hm = there.format("%H:%M").map(|s| s.to_string()).unwrap_or_default();
            // Petite taille : jour d'écart en exposant après l'heure (« 04:22⁺¹ »).
            let jump = match (there.ymd(), here.ymd()) {
                (a, b) if a > b => "+1",
                (a, b) if a < b => "−1",
                _ => "",
            };
            if small && !jump.is_empty() {
                time.set_markup(&format!("{hm}<span size=\"60%\" rise=\"6pt\">{jump}</span>"));
            } else {
                time.set_text(&hm);
            }
            let diff = (there.utc_offset().as_seconds() - here.utc_offset().as_seconds()) / 60;
            let day = match (there.ymd(), here.ymd()) {
                (a, b) if a > b => "Demain",
                (a, b) if a < b => "Hier",
                _ => "Aujourd'hui",
            };
            let off_txt = if diff == 0 {
                "même heure".to_string()
            } else {
                let (h, m) = (diff.abs() / 60, diff.abs() % 60);
                let sign = if diff > 0 { "+" } else { "−" };
                if m == 0 { format!("{sign}{h} h") } else { format!("{sign}{h} h {m:02}") }
            };
            // Petite taille : le décalage seul (le jour seulement s'il change).
            off.set_text(&match (small, day) {
                (true, _) => off_txt,
                _ => format!("{day}, {off_txt}"),
            });
            area.queue_draw();
            bar.queue_draw();
        }));
    }
    if ticks.is_empty() {
        let l = gtk::Label::new(Some("Aucun fuseau : ajoutez-en dans les réglages."));
        l.add_css_class("desk-dim");
        b.append(&l);
    }
    let tick = move || ticks.iter().for_each(|f| f());
    tick();
    every(&b, 10, tick);
    b.upcast()
}

/// Petit cadran : clair entre 7 h et 19 h, sombre la nuit.
fn draw_mini(cr: &gtk::cairo::Context, w: f64, h: f64, now: &glib::DateTime) {
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 1.0);
    let day = (7..19).contains(&now.hour());
    let (bg, fg) = if day { ((0.97, 0.97, 0.98), (0.12, 0.13, 0.16)) } else { ((0.12, 0.14, 0.24), (0.93, 0.94, 0.98)) };
    cr.set_source_rgb(bg.0, bg.1, bg.2);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.fill_preserve();
    // Liseré : un cadran de nuit reste visible sur un fond sombre.
    if day {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.18);
    } else {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.35);
    }
    cr.set_line_width(1.2);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.5);
    for i in 0..12 {
        let a = i as f64 * PI / 6.0;
        cr.set_line_width(1.2);
        cr.move_to(cx + r * 0.78 * a.sin(), cy - r * 0.78 * a.cos());
        cr.line_to(cx + r * 0.9 * a.sin(), cy - r * 0.9 * a.cos());
        let _ = cr.stroke();
    }
    let (hh, mm) = (now.hour() as f64 % 12.0, now.minute() as f64);
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    for (angle, len, width) in [((hh + mm / 60.0) * PI / 6.0, r * 0.5, r * 0.12), (mm * PI / 30.0, r * 0.75, r * 0.08)] {
        cr.set_line_width(width);
        cr.move_to(cx, cy);
        cr.line_to(cx + len * angle.sin(), cy - len * angle.cos());
        let _ = cr.stroke();
    }
}

/// Barre de 24 h : nuit (bleu nuit), aube et crépuscule (orangés), jour (bleu ciel) ; trait
/// blanc à l'heure actuelle.
fn draw_day(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, now: &glib::DateTime) {
    let _ = a;
    let g = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
    for (hour, (r, gg, b)) in [(0.0, (0.10, 0.12, 0.28)), (5.5, (0.14, 0.16, 0.36)), (7.0, (0.98, 0.62, 0.36)), (9.0, (0.40, 0.68, 0.98)), (17.0, (0.40, 0.68, 0.98)), (19.0, (0.95, 0.50, 0.42)), (20.5, (0.16, 0.16, 0.38)), (24.0, (0.10, 0.12, 0.28))] {
        g.add_color_stop_rgb(hour / 24.0, r, gg, b);
    }
    let r = h / 2.0;
    cr.new_sub_path();
    cr.arc(r, r, r, PI / 2.0, 1.5 * PI);
    cr.arc(w - r, r, r, -PI / 2.0, PI / 2.0);
    cr.close_path();
    let _ = cr.set_source(&g);
    let _ = cr.fill();
    let x = (now.hour() as f64 + now.minute() as f64 / 60.0) / 24.0 * w;
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.arc(x.clamp(r, w - r), r, r * 0.9, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
    cr.set_line_width(1.0);
    cr.arc(x.clamp(r, w - r), r, r * 0.9, 0.0, 2.0 * PI);
    let _ = cr.stroke();
}

/// Fuseaux connus du système (`/usr/share/zoneinfo/tzdata.zi` ou `zone1970.tab`).
fn all_zones() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2).map(String::from))
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Villes");
    let list = zones(w);
    for (i, id) in list.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("desk-world-zone");
        let l = gtk::Label::new(Some(&city(id)));
        l.set_xalign(0.0);
        l.set_hexpand(true);
        l.set_tooltip_text(Some(id));
        row.append(&l);
        for (icon, tip, delta) in [("go-up-symbolic", "Monter", -1i32), ("window-close-symbolic", "Retirer", 0)] {
            if delta == -1 && i == 0 {
                continue;
            }
            let btn = gtk::Button::from_icon_name(icon);
            btn.add_css_class("flat");
            btn.add_css_class("circular");
            btn.set_tooltip_text(Some(tip));
            let (ctx, list, name) = (ctx.clone(), list.clone(), city(id));
            btn.connect_clicked(move |_| {
                let mut l = list.clone();
                let label = if delta == 0 {
                    l.remove(i);
                    format!("{name} retirée des horloges")
                } else {
                    l.swap(i, i - 1);
                    format!("{name} remontée")
                };
                ctx.set_option(&label, "zones", &l.join(";"));
            });
            row.append(&btn);
        }
        b.append(&row);
    }
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Ajouter une ville (ex. Tokyo, Montréal)…"));
    b.append(&entry);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.append(&results);
    let all = std::rc::Rc::new(all_zones());
    let (ctx, list) = (ctx.clone(), list.clone());
    entry.connect_search_changed(move |e| {
        while let Some(c) = results.first_child() {
            results.remove(&c);
        }
        let q = e.text().to_lowercase();
        if q.len() < 2 {
            return;
        }
        for id in all.iter().filter(|id| id.to_lowercase().contains(&q) || city(id).to_lowercase().contains(&q)).take(6) {
            let btn = gtk::Button::with_label(&format!("{} — {}", city(id), id));
            btn.add_css_class("flat");
            if let Some(l) = btn.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let (ctx, list, id) = (ctx.clone(), list.clone(), id.clone());
            btn.connect_clicked(move |_| {
                let mut l = list.clone();
                if !l.contains(&id) {
                    l.push(id.clone());
                }
                ctx.set_option(&format!("{} ajoutée aux horloges", city(&id)), "zones", &l.join(";"));
            });
            results.append(&btn);
        }
    });
}

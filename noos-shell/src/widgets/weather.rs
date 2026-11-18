//! Météo (Open-Meteo) : conditions actuelles, ressenti, vent, humidité et prévisions ; le
//! fond suit le temps qu'il fait et le jour / la nuit. Ville choisie dans les réglages.

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Instant};

use gtk::{glib, prelude::*};
use noos_core::{config::DesktopWidget, weather::{self, Weather}};

use super::{Ctx, by_size, choice, every, section};

pub const CSS: &str = r#"
.kind-weather.style-glass, .kind-weather.style-solid { color: white; border-color: alpha(white, 0.14); }
.kind-weather.style-glass.mood-clear, .kind-weather.style-solid.mood-clear { background-image: linear-gradient(160deg, #3a8dde, #64b5f6 60%, #9fd3ff); }
.kind-weather.style-glass.mood-night, .kind-weather.style-solid.mood-night { background-image: linear-gradient(160deg, #0f1a3c, #24306b 65%, #3d3f8a); }
.kind-weather.style-glass.mood-cloudy, .kind-weather.style-solid.mood-cloudy { background-image: linear-gradient(160deg, #56708f, #7d93ab 60%, #a3b3c4); }
.kind-weather.style-glass.mood-rain, .kind-weather.style-solid.mood-rain { background-image: linear-gradient(160deg, #34495e, #4d6a85 60%, #6f8aa3); }
.kind-weather.style-glass.mood-snow, .kind-weather.style-solid.mood-snow { background-image: linear-gradient(160deg, #7f9cc0, #b7cbe0 60%, #dfe9f5); }
.kind-weather.style-glass.mood-storm, .kind-weather.style-solid.mood-storm { background-image: linear-gradient(160deg, #241f3a, #3e3560 60%, #5b4a7d); }
.kind-weather.style-glass.mood-fog, .kind-weather.style-solid.mood-fog { background-image: linear-gradient(160deg, #6d7882, #98a2ab 60%, #c0c7cd); }
.desk-weather-temp { font-size: 56px; font-weight: 300; }
.size-s .desk-weather-temp { font-size: 40px; }
.size-l .desk-weather-temp { font-size: 72px; font-weight: 200; }
.desk-weather-place { font-weight: 700; font-size: 1.1em; }
.desk-weather-small { font-size: 0.85em; opacity: 0.85; }
.desk-weather-day { padding: 6px 4px; border-radius: 10px; background-color: alpha(white, 0.10); min-width: 52px; }
.desk-weather-day .day { font-weight: 700; font-size: 0.85em; }
.desk-weather-empty button { margin-top: 6px; }
.style-clear .desk-weather-temp, .style-clear .desk-weather-place { text-shadow: 0 1px 10px rgba(0,0,0,0.5); color: white; }
"#;

thread_local! {
    /// Dernières prévisions par lieu : un widget reconstruit (réglage changé) ne refait pas de
    /// requête.
    static CACHE: RefCell<HashMap<String, (Weather, Instant)>> = RefCell::new(HashMap::new());
}

const REFRESH_SECS: u64 = 30 * 60;

fn demo() -> bool {
    std::env::var_os("NOOS_WEATHER_DEMO").is_some()
}

pub fn build(ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let place = w.option("city").map(String::from).or_else(|| demo().then(|| "Lyon".to_string()));
    let coords = w.option("lat").zip(w.option("lon")).and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))).or_else(|| demo().then_some((45.75, 4.85)));
    let fahrenheit = w.option("unit") == Some("f");
    let (Some(place), Some((lat, lon))) = (place, coords) else {
        // Pas encore de ville.
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.add_css_class("desk-weather-empty");
        let i = gtk::Image::from_icon_name("weather-few-clouds-symbolic");
        i.set_pixel_size(48);
        b.append(&i);
        let t = gtk::Label::new(Some("Météo"));
        t.add_css_class("desk-title");
        b.append(&t);
        let l = gtk::Label::new(Some("Choisissez une ville pour voir le temps qu'il fait."));
        l.set_wrap(true);
        l.set_max_width_chars(26);
        l.set_justify(gtk::Justification::Center);
        l.add_css_class("desk-dim");
        b.append(&l);
        let btn = gtk::Button::with_label("Choisir une ville");
        btn.add_css_class("suggested-action");
        btn.add_css_class("pill");
        let c = ctx.clone();
        btn.connect_clicked(move |_| c.open_settings());
        b.append(&btn);
        b.set_size_request(by_size(w, 220, 280, 320), -1);
        root.append(&b);
        let r = root.clone();
        glib::idle_add_local_once(move || {
            if let Some(card) = find_card(r.upcast_ref()) {
                card.add_css_class("mood-cloudy");
            }
        });
        return root.upcast();
    };
    let key = format!("{lat:.2},{lon:.2},{fahrenheit}");
    let size = w.size().to_string();
    let show = {
        let (root, place, size) = (root.clone(), place.clone(), size.clone());
        Rc::new(move |data: Option<&Weather>, error: Option<&str>| {
            while let Some(c) = root.first_child() {
                root.remove(&c);
            }
            root.append(&view(&place, data, error, &size, fahrenheit));
            // L'ambiance est une classe du cadre (fond dégradé).
            if let Some(card) = find_card(root.upcast_ref()) {
                for m in ["clear", "night", "cloudy", "rain", "snow", "storm", "fog"] {
                    card.remove_css_class(&format!("mood-{m}"));
                }
                let mood = data.map(|d| weather::mood(d.code, d.is_day)).unwrap_or("cloudy");
                card.add_css_class(&format!("mood-{mood}"));
            }
        })
    };
    // Données en mémoire ou en cache disque d'abord, puis requête si elles sont trop vieilles.
    let fresh = CACHE.with(|c| c.borrow().get(&key).filter(|(_, t)| t.elapsed().as_secs() < REFRESH_SECS).map(|(w, _)| w.clone()));
    let refresh = {
        let (show, key) = (show.clone(), key.clone());
        Rc::new(move || {
            if demo() {
                let d = weather::demo();
                show(Some(&d), None);
                return;
            }
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(weather::fetch(lat, lon, fahrenheit).map_err(|e| e.to_string()));
            });
            let (show, key) = (show.clone(), key.clone());
            glib::spawn_future_local(async move {
                match rx.recv().await {
                    Ok(Ok(d)) => {
                        CACHE.with(|c| c.borrow_mut().insert(key, (d.clone(), Instant::now())));
                        show(Some(&d), None);
                    }
                    Ok(Err(_)) | Err(_) => {
                        let old = weather::cached(lat, lon, fahrenheit);
                        show(old.as_ref(), Some("Hors ligne"));
                    }
                }
            });
        })
    };
    // Affichage différé : le cadre (classe d'ambiance) n'est connu qu'une fois le contenu posé.
    {
        let (show, refresh) = (show.clone(), refresh.clone());
        glib::idle_add_local_once(move || match fresh {
            Some(d) => show(Some(&d), None),
            None => {
                let old = weather::cached(lat, lon, fahrenheit);
                show(old.as_ref(), None);
                refresh();
            }
        });
    }
    every(&root, REFRESH_SECS as u32, move || refresh());
    root.upcast()
}

/// Cadre du widget (classe `desk-card`) qui contient `w`.
fn find_card(w: &gtk::Widget) -> Option<gtk::Widget> {
    let mut cur = Some(w.clone());
    while let Some(c) = cur {
        if c.has_css_class("desk-card") {
            return Some(c);
        }
        cur = c.parent();
    }
    None
}

fn deg(v: f64) -> String {
    format!("{}°", v.round() as i64)
}

fn view(place: &str, d: Option<&Weather>, error: Option<&str>, size: &str, fahrenheit: bool) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_size_request(match size { "s" => 200, "l" => 400, _ => 300 }, -1);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let p = gtk::Label::new(Some(place));
    p.add_css_class("desk-weather-place");
    p.set_xalign(0.0);
    p.set_hexpand(true);
    p.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&p);
    if let Some(d) = d {
        let when = glib::DateTime::from_unix_local(d.fetched as i64).ok().and_then(|t| t.format("%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
        let l = gtk::Label::new(Some(&match error {
            Some(e) => format!("{e} · {when}"),
            None if size == "s" => String::new(),
            None => format!("à {when}"),
        }));
        l.add_css_class("desk-weather-small");
        top.append(&l);
    }
    b.append(&top);
    let Some(d) = d else {
        let l = gtk::Label::new(Some(if error.is_some() { "Météo indisponible (pas de réseau ?)" } else { "Chargement…" }));
        l.add_css_class("desk-weather-small");
        l.set_xalign(0.0);
        b.append(&l);
        return b.upcast();
    };
    let mid = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name(weather::icon(d.code, d.is_day));
    icon.set_pixel_size(match size { "s" => 40, "l" => 76, _ => 58 });
    mid.append(&icon);
    let t = gtk::Label::new(Some(&deg(d.temp)));
    t.add_css_class("desk-weather-temp");
    t.add_css_class("numeric");
    mid.append(&t);
    if size != "s" {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_valign(gtk::Align::Center);
        col.set_hexpand(true);
        let desc = gtk::Label::new(Some(weather::describe(d.code)));
        desc.set_xalign(0.0);
        desc.add_css_class("heading");
        desc.set_wrap(true);
        col.append(&desc);
        let feels = gtk::Label::new(Some(&format!("Ressenti {}", deg(d.feels))));
        feels.set_xalign(0.0);
        feels.add_css_class("desk-weather-small");
        col.append(&feels);
        if let Some(today) = d.days.first() {
            let mm = gtk::Label::new(Some(&format!("↑ {}   ↓ {}", deg(today.max), deg(today.min))));
            mm.set_xalign(0.0);
            mm.add_css_class("desk-weather-small");
            col.append(&mm);
        }
        mid.append(&col);
    }
    b.append(&mid);
    if size != "s" {
        let unit = if fahrenheit { "mph" } else { "km/h" };
        let info = gtk::Label::new(Some(&format!("Humidité {} %  ·  Vent {} {unit}", d.humidity.round(), d.wind.round())));
        info.set_xalign(0.0);
        info.add_css_class("desk-weather-small");
        b.append(&info);
    }
    if size == "l" {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.set_homogeneous(true);
        for (i, day) in d.days.iter().skip(1).take(5).enumerate() {
            let c = gtk::Box::new(gtk::Orientation::Vertical, 4);
            c.add_css_class("desk-weather-day");
            let n = gtk::Label::new(Some(&if i == 0 { "Demain".to_string() } else { super::capitalize(&weather::weekday_short(&day.date)) }));
            n.add_css_class("day");
            c.append(&n);
            let ic = gtk::Image::from_icon_name(weather::icon(day.code, true));
            ic.set_pixel_size(24);
            ic.set_tooltip_text(Some(weather::describe(day.code)));
            c.append(&ic);
            let mm = gtk::Label::new(Some(&format!("{} / {}", deg(day.max), deg(day.min))));
            mm.add_css_class("desk-weather-small");
            c.append(&mm);
            row.append(&c);
        }
        b.append(&row);
    }
    b.upcast()
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Ville");
    let current = gtk::Label::new(Some(w.option("city").unwrap_or("Aucune ville choisie")));
    current.set_xalign(0.0);
    current.add_css_class("desk-dim");
    b.append(&current);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Rechercher une ville…"));
    b.append(&entry);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&results);
    let spinner = gtk::Spinner::new();
    let run = {
        let (ctx, results, spinner) = (ctx.clone(), results.clone(), spinner.clone());
        move |text: String| {
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            if text.trim().len() < 2 {
                return;
            }
            results.append(&spinner);
            spinner.start();
            let (tx, rx) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let _ = tx.send_blocking(weather::search(&text).map_err(|e| e.to_string()));
            });
            let (ctx, results) = (ctx.clone(), results.clone());
            glib::spawn_future_local(async move {
                let res = rx.recv().await.unwrap_or_else(|_| Err("annulé".into()));
                while let Some(c) = results.first_child() {
                    results.remove(&c);
                }
                match res {
                    Ok(places) if places.is_empty() => {
                        let l = gtk::Label::new(Some("Aucune ville trouvée"));
                        l.add_css_class("desk-dim");
                        results.append(&l);
                    }
                    Ok(places) => {
                        for p in places {
                            let btn = gtk::Button::new();
                            btn.add_css_class("flat");
                            let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
                            let n = gtk::Label::new(Some(&p.name));
                            n.set_xalign(0.0);
                            n.add_css_class("heading");
                            v.append(&n);
                            let dl = gtk::Label::new(Some(&p.detail));
                            dl.set_xalign(0.0);
                            dl.add_css_class("desk-dim");
                            v.append(&dl);
                            btn.set_child(Some(&v));
                            let ctx = ctx.clone();
                            btn.connect_clicked(move |_| {
                                let (name, lat, lon) = (p.name.clone(), p.lat, p.lon);
                                ctx.edit(&format!("Météo : {name}"), move |w| {
                                    w.options.insert("city".into(), name);
                                    w.options.insert("lat".into(), format!("{lat:.4}"));
                                    w.options.insert("lon".into(), format!("{lon:.4}"));
                                });
                            });
                            results.append(&btn);
                        }
                    }
                    Err(_) => {
                        let l = gtk::Label::new(Some("Recherche impossible (pas de réseau ?)"));
                        l.add_css_class("desk-dim");
                        results.append(&l);
                    }
                }
            });
        }
    };
    entry.connect_activate(move |e| run(e.text().to_string()));
    let hint = gtk::Label::new(Some("Entrée pour chercher · données Open-Meteo"));
    hint.add_css_class("desk-dim");
    hint.add_css_class("caption");
    hint.set_xalign(0.0);
    b.append(&hint);
    section(b, "Unités");
    let c = ctx.clone();
    choice(b, &[("c", "°C · km/h"), ("f", "°F · mph")], w.option("unit").unwrap_or("c"), move |v| c.set_option("Unités de la météo", "unit", v));
}

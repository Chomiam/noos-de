//! Horloge : numérique (grands chiffres) ou analogique (cadran dessiné).
//!
//! - petite : l'heure et la date courte (« mer. 18 nov. ») ; cadran seul ;
//! - moyenne : l'heure et la date complète ;
//! - grande : date avec l'année, numéro de semaine, jour de l'année et avancement de la
//!   journée ; cadran chiffré avec l'heure numérique dessous.

use std::f64::consts::PI;

use gtk::{gdk, glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, capitalize, choice, every, section, switch};

pub const CSS: &str = r#"
.desk-clock-time { font-size: 72px; font-weight: 300; }
.size-s .desk-clock-time { font-size: 44px; font-weight: 400; }
.size-l .desk-clock-time { font-size: 112px; font-weight: 200; }
.desk-clock-sec { font-size: 22px; font-weight: 400; opacity: 0.7; margin-bottom: 14px; }
.size-s .desk-clock-sec { font-size: 15px; margin-bottom: 8px; }
.size-l .desk-clock-sec { font-size: 32px; margin-bottom: 22px; }
.desk-clock-date { font-size: 20px; font-weight: 500; }
.size-s .desk-clock-date { font-size: 14px; font-weight: 600; }
.size-l .desk-clock-date { font-size: 26px; }
.desk-clock-info { font-size: 14px; opacity: 0.8; font-feature-settings: "tnum"; }
.desk-clock-digital { font-size: 26px; font-weight: 300; font-feature-settings: "tnum"; }
.desk-clock-day trough { min-height: 4px; border-radius: 99px; background-color: alpha(currentColor, 0.18); }
.desk-clock-day progress { min-height: 4px; border-radius: 99px; background-color: @accent_bg_color; }
.style-clear .desk-clock-time, .style-clear .desk-clock-sec, .style-clear .desk-clock-digital { color: white; text-shadow: 0 2px 16px rgba(0,0,0,0.45); }
.style-clear .desk-clock-date, .style-clear .desk-clock-info { color: rgba(255,255,255,0.92); text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.desk-clock-face { color: @accent_bg_color; }
"#;

fn fmt(now: &glib::DateTime, f: &str) -> String {
    now.format(f).map(|s| s.to_string()).unwrap_or_default()
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let size = w.size().to_string();
    let analog = w.option("face") == Some("analog");
    let seconds = w.option("seconds") == Some("1");
    let show_date = w.option("date") != Some("0");
    let clear = w.style.is_empty() || w.style == "clear";
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let date = gtk::Label::new(None);
    date.add_css_class("desk-clock-date");
    // Petit cadran : seul (il dit déjà tout).
    date.set_visible(show_date && !(analog && size == "s"));
    // Grande taille : semaine, jour de l'année, avancement de la journée.
    let info = gtk::Label::new(None);
    info.add_css_class("desk-clock-info");
    let day = gtk::ProgressBar::new();
    day.add_css_class("desk-clock-day");
    day.set_margin_top(4);
    let large = size == "l";
    info.set_visible(large && show_date);
    day.set_visible(large);
    let digital = gtk::Label::new(None);
    digital.add_css_class("desk-clock-digital");
    let tick: Box<dyn Fn()> = if analog {
        let px = match size.as_str() {
            "s" => 130,
            "l" => 260,
            _ => 200,
        };
        let area = gtk::DrawingArea::new();
        area.set_content_width(px);
        area.set_content_height(px);
        area.add_css_class("desk-clock-face");
        area.set_halign(gtk::Align::Center);
        area.set_draw_func(move |a, cr, wd, ht| draw_face(a, cr, wd as f64, ht as f64, seconds, clear, large));
        b.append(&area);
        for l in [&digital, &date, &info] {
            l.set_halign(gtk::Align::Center);
            b.append(l);
        }
        digital.set_visible(large);
        day.set_size_request(px, -1);
        day.set_halign(gtk::Align::Center);
        b.append(&day);
        let (a2, d2, i2, g2, day2, size) = (area.clone(), date.clone(), info.clone(), digital.clone(), day.clone(), size.clone());
        Box::new(move || {
            a2.queue_draw();
            let Ok(now) = glib::DateTime::now_local() else { return };
            g2.set_text(&fmt(&now, if seconds { "%H:%M:%S" } else { "%H:%M" }));
            texts(&now, &size, &d2, &i2, &day2);
        })
    } else {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let time = gtk::Label::new(None);
        time.add_css_class("desk-clock-time");
        time.add_css_class("numeric");
        row.append(&time);
        let sec = gtk::Label::new(None);
        sec.add_css_class("desk-clock-sec");
        sec.add_css_class("numeric");
        sec.set_valign(gtk::Align::End);
        sec.set_visible(seconds);
        row.append(&sec);
        b.append(&row);
        for l in [&date, &info] {
            l.set_xalign(0.0);
            b.append(l);
        }
        b.append(&day);
        let (d2, i2, day2, size) = (date.clone(), info.clone(), day.clone(), size.clone());
        Box::new(move || {
            let Ok(now) = glib::DateTime::now_local() else { return };
            time.set_text(&fmt(&now, "%H:%M"));
            sec.set_text(&fmt(&now, "%S"));
            texts(&now, &size, &d2, &i2, &day2);
        })
    };
    tick();
    every(&b, 1, tick);
    b.upcast()
}

/// Date (courte, complète ou avec l'année), ligne d'informations et avancement du jour.
fn texts(now: &glib::DateTime, size: &str, date: &gtk::Label, info: &gtk::Label, day: &gtk::ProgressBar) {
    date.set_text(&capitalize(&fmt(now, match size {
        "s" => "%a %-d %b",
        "l" => "%A %-d %B %Y",
        _ => "%A %-d %B",
    })));
    if size == "l" {
        let year_days = if glib::DateTime::from_local(now.year(), 12, 31, 0, 0, 0.0).map(|d| d.day_of_year()).unwrap_or(365) == 366 { 366 } else { 365 };
        info.set_text(&format!("Semaine {} · jour {} sur {}", now.week_of_year(), now.day_of_year(), year_days));
        let secs = now.hour() * 3600 + now.minute() * 60 + now.second();
        day.set_fraction(secs as f64 / 86400.0);
        day.set_tooltip_text(Some(&format!("{} % de la journée écoulée", secs * 100 / 86400)));
    }
}

/// Cadran : graduations (chiffres en grande taille), aiguilles, trotteuse à la couleur d'accent.
fn draw_face(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, seconds: bool, clear: bool, numerals: bool) {
    let accent = area.color();
    let fg = area.parent().map(|p| p.color()).unwrap_or(gdk::RGBA::WHITE);
    let fg = if clear { gdk::RGBA::WHITE } else { fg };
    let (fr, fgc, fb) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 4.0);
    let Ok(now) = glib::DateTime::now_local() else { return };
    let (hh, mm, ss) = (now.hour() as f64 % 12.0, now.minute() as f64, now.second() as f64);
    if clear {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.22);
        cr.arc(cx, cy + 3.0, r, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
    cr.set_source_rgba(fr, fgc, fb, 0.08);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(fr, fgc, fb, 0.25);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    for i in 0..60 {
        let hour = i % 5 == 0;
        // Avec les chiffres : seulement les minutes (les heures sont écrites).
        if hour && numerals {
            continue;
        }
        let a = i as f64 * PI / 30.0;
        let (r0, r1) = if hour { (r * 0.80, r * 0.92) } else { (r * 0.87, r * 0.92) };
        cr.set_line_width(if hour { r * 0.035 } else { r * 0.012 });
        cr.set_source_rgba(fr, fgc, fb, if hour { 0.9 } else { 0.4 });
        cr.move_to(cx + r0 * a.sin(), cy - r0 * a.cos());
        cr.line_to(cx + r1 * a.sin(), cy - r1 * a.cos());
        let _ = cr.stroke();
    }
    if numerals {
        cr.select_font_face("Sans", gtk::cairo::FontSlant::Normal, gtk::cairo::FontWeight::Normal);
        cr.set_font_size(r * 0.16);
        cr.set_source_rgba(fr, fgc, fb, 0.9);
        for n in 1..=12 {
            let a = n as f64 * PI / 6.0;
            let t = n.to_string();
            if let Ok(e) = cr.text_extents(&t) {
                let (x, y) = (cx + r * 0.74 * a.sin(), cy - r * 0.74 * a.cos());
                cr.move_to(x - e.width() / 2.0 - e.x_bearing(), y - e.height() / 2.0 - e.y_bearing());
                let _ = cr.show_text(&t);
            }
        }
    }
    let hand = |angle: f64, len: f64, width: f64, tail: f64, rgba: (f64, f64, f64)| {
        cr.set_source_rgb(rgba.0, rgba.1, rgba.2);
        cr.set_line_width(width);
        cr.move_to(cx - tail * angle.sin(), cy + tail * angle.cos());
        cr.line_to(cx + len * angle.sin(), cy - len * angle.cos());
        let _ = cr.stroke();
    };
    hand((hh + mm / 60.0) * PI / 6.0, r * 0.5, r * 0.075, r * 0.08, (fr, fgc, fb));
    hand((mm + ss / 60.0) * PI / 30.0, r * 0.74, r * 0.05, r * 0.08, (fr, fgc, fb));
    let a = (accent.red() as f64, accent.green() as f64, accent.blue() as f64);
    if seconds {
        hand(ss * PI / 30.0, r * 0.82, r * 0.02, r * 0.18, a);
    }
    cr.set_source_rgb(a.0, a.1, a.2);
    cr.arc(cx, cy, r * 0.055, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Cadran");
    let c = ctx.clone();
    choice(b, &[("digital", "Numérique"), ("analog", "Analogique")], w.option("face").unwrap_or("digital"), move |v| c.set_option("Cadran de l'horloge", "face", v));
    let c = ctx.clone();
    switch(b, "Secondes", w.option("seconds") == Some("1"), move |on| c.set_option("Secondes de l'horloge", "seconds", if on { "1" } else { "0" }));
    let c = ctx.clone();
    switch(b, "Date", w.option("date") != Some("0"), move |on| c.set_option("Date de l'horloge", "date", if on { "1" } else { "0" }));
}

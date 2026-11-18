//! Horloge : numérique (grands chiffres) ou analogique (cadran dessiné), secondes et date
//! en option.

use std::f64::consts::PI;

use gtk::{gdk, glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, capitalize, choice, every, section, switch};

pub const CSS: &str = r#"
.desk-clock-time { font-size: 72px; font-weight: 300; }
.size-s .desk-clock-time { font-size: 46px; }
.size-l .desk-clock-time { font-size: 112px; font-weight: 200; }
.desk-clock-sec { font-size: 22px; font-weight: 400; opacity: 0.7; margin-bottom: 14px; }
.size-s .desk-clock-sec { font-size: 15px; margin-bottom: 9px; }
.size-l .desk-clock-sec { font-size: 32px; margin-bottom: 22px; }
.desk-clock-date { font-size: 20px; font-weight: 500; }
.size-s .desk-clock-date { font-size: 14px; }
.size-l .desk-clock-date { font-size: 27px; }
.style-clear .desk-clock-time, .style-clear .desk-clock-sec { color: white; text-shadow: 0 2px 16px rgba(0,0,0,0.45); }
.style-clear .desk-clock-date { color: rgba(255,255,255,0.92); text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.desk-clock-face { color: @accent_bg_color; }
"#;

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let analog = w.option("face") == Some("analog");
    let seconds = w.option("seconds") == Some("1");
    let show_date = w.option("date") != Some("0");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let date = gtk::Label::new(None);
    date.add_css_class("desk-clock-date");
    date.set_visible(show_date);
    let tick: Box<dyn Fn()> = if analog {
        let px = by_size(w, 150, 210, 300);
        let area = gtk::DrawingArea::new();
        area.set_content_width(px);
        area.set_content_height(px);
        area.add_css_class("desk-clock-face");
        area.set_halign(gtk::Align::Center);
        let clear = w.style.is_empty() || w.style == "clear";
        area.set_draw_func(move |a, cr, wd, ht| draw_face(a, cr, wd as f64, ht as f64, seconds, clear));
        b.append(&area);
        date.set_halign(gtk::Align::Center);
        b.append(&date);
        let (a2, d2) = (area.clone(), date.clone());
        Box::new(move || {
            a2.queue_draw();
            set_date(&d2);
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
        date.set_xalign(0.0);
        b.append(&date);
        Box::new(move || {
            if let Ok(now) = glib::DateTime::now_local() {
                time.set_text(&now.format("%H:%M").map(|s| s.to_string()).unwrap_or_default());
                sec.set_text(&now.format("%S").map(|s| s.to_string()).unwrap_or_default());
            }
            set_date(&date);
        })
    };
    tick();
    every(&b, 1, tick);
    b.upcast()
}

fn set_date(l: &gtk::Label) {
    if let Ok(now) = glib::DateTime::now_local() {
        l.set_text(&capitalize(&now.format("%A %-d %B").map(|s| s.to_string()).unwrap_or_default()));
    }
}

/// Cadran : graduations, aiguilles des heures et minutes, trotteuse à la couleur d'accent.
fn draw_face(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, seconds: bool, clear: bool) {
    let accent = area.color();
    let fg = area.parent().map(|p| p.color()).unwrap_or(gdk::RGBA::WHITE);
    let fg = if clear { gdk::RGBA::WHITE } else { fg };
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 4.0);
    let Ok(now) = glib::DateTime::now_local() else { return };
    let (hh, mm, ss) = (now.hour() as f64 % 12.0, now.minute() as f64, now.second() as f64);
    // Fond du cadran, légère ombre en mode « sans fond ».
    if clear {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.22);
        cr.arc(cx, cy + 3.0, r, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.08);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.25);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    // Graduations.
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    for i in 0..60 {
        let hour = i % 5 == 0;
        let a = i as f64 * PI / 30.0;
        let (r0, r1) = if hour { (r * 0.80, r * 0.92) } else { (r * 0.87, r * 0.92) };
        cr.set_line_width(if hour { r * 0.035 } else { r * 0.012 });
        cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, if hour { 0.9 } else { 0.4 });
        cr.move_to(cx + r0 * a.sin(), cy - r0 * a.cos());
        cr.line_to(cx + r1 * a.sin(), cy - r1 * a.cos());
        let _ = cr.stroke();
    }
    let hand = |angle: f64, len: f64, width: f64, tail: f64, rgba: (f64, f64, f64, f64)| {
        cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
        cr.set_line_width(width);
        cr.move_to(cx - tail * angle.sin(), cy + tail * angle.cos());
        cr.line_to(cx + len * angle.sin(), cy - len * angle.cos());
        let _ = cr.stroke();
    };
    let f = (fg.red() as f64, fg.green() as f64, fg.blue() as f64, 1.0);
    hand((hh + mm / 60.0) * PI / 6.0, r * 0.5, r * 0.075, r * 0.08, f);
    hand((mm + ss / 60.0) * PI / 30.0, r * 0.74, r * 0.05, r * 0.08, f);
    let a = (accent.red() as f64, accent.green() as f64, accent.blue() as f64, 1.0);
    if seconds {
        hand(ss * PI / 30.0, r * 0.82, r * 0.02, r * 0.18, a);
    }
    cr.set_source_rgba(a.0, a.1, a.2, 1.0);
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

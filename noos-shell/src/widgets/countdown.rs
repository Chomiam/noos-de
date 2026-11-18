//! Compte à rebours : un événement nommé, les jours restants dans un anneau de progression
//! (depuis la création du compte à rebours), puis heures et minutes.

use std::f64::consts::PI;

use gtk::{glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, capitalize, every, section};

pub const CSS: &str = r#"
.desk-count-ring { color: @accent_bg_color; }
.desk-count-num { font-size: 34px; font-weight: 700; font-feature-settings: "tnum"; }
.size-s .desk-count-num { font-size: 24px; }
.size-l .desk-count-num { font-size: 48px; }
.desk-count-unit { font-size: 0.8em; opacity: 0.7; margin-top: -4px; }
.desk-count-title { font-weight: 700; font-size: 1.2em; }
.size-l .desk-count-title { font-size: 1.45em; }
.desk-count-rest { font-feature-settings: "tnum"; }
"#;

/// Valeurs d'un compte à rebours neuf : nouvel an prochain, créé maintenant.
pub fn defaults(w: &mut DesktopWidget) {
    let Ok(now) = glib::DateTime::now_local() else { return };
    w.options.entry("title".into()).or_insert_with(|| "Nouvel an".into());
    w.options.entry("target".into()).or_insert_with(|| format!("{}-01-01T00:00", now.year() + 1));
    w.options.entry("created".into()).or_insert_with(|| now.format("%Y-%m-%dT%H:%M").map(|s| s.to_string()).unwrap_or_default());
}

/// « 2026-12-25T18:30 » → date locale.
pub fn parse(s: &str) -> Option<glib::DateTime> {
    let (d, t) = s.split_once('T').unwrap_or((s, "00:00"));
    let mut dp = d.split('-').filter_map(|p| p.parse::<i32>().ok());
    let mut tp = t.split(':').filter_map(|p| p.parse::<i32>().ok());
    glib::DateTime::from_local(dp.next()?, dp.next()?, dp.next()?, tp.next().unwrap_or(0), tp.next().unwrap_or(0), 0.0).ok()
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let title = w.option("title").unwrap_or("Événement").to_string();
    let target = w.option("target").and_then(parse);
    let created = w.option("created").and_then(parse);
    let b = gtk::Box::new(gtk::Orientation::Horizontal, by_size(w, 12, 18, 22));
    let ring_px = by_size(w, 88, 116, 156);
    let ring = gtk::Overlay::new();
    let area = gtk::DrawingArea::new();
    area.set_content_width(ring_px);
    area.set_content_height(ring_px);
    area.add_css_class("desk-count-ring");
    ring.set_child(Some(&area));
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inner.set_halign(gtk::Align::Center);
    inner.set_valign(gtk::Align::Center);
    let num = gtk::Label::new(None);
    num.add_css_class("desk-count-num");
    inner.append(&num);
    let unit = gtk::Label::new(None);
    unit.add_css_class("desk-count-unit");
    inner.append(&unit);
    ring.add_overlay(&inner);
    b.append(&ring);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    col.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(&title));
    t.add_css_class("desk-count-title");
    t.set_xalign(0.0);
    t.set_wrap(true);
    t.set_max_width_chars(18);
    col.append(&t);
    let when = gtk::Label::new(target.as_ref().and_then(|d| d.format("%A %-d %B %Y · %H:%M").ok()).map(|s| capitalize(&s)).as_deref());
    when.add_css_class("desk-dim");
    when.set_xalign(0.0);
    when.set_visible(w.size() != "s");
    col.append(&when);
    let rest = gtk::Label::new(None);
    rest.add_css_class("desk-count-rest");
    rest.set_xalign(0.0);
    col.append(&rest);
    b.append(&col);
    let progress = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    {
        let progress = progress.clone();
        area.set_draw_func(move |a, cr, wd, ht| draw_ring(a, cr, wd as f64, ht as f64, progress.get()));
    }
    let tick = move || {
        let Some(target) = &target else {
            num.set_text("?");
            unit.set_text("");
            rest.set_text("Choisissez une date dans les réglages");
            return;
        };
        let Ok(now) = glib::DateTime::now_local() else { return };
        let secs = target.difference(&now).as_seconds();
        if secs <= 0 {
            let past = (-secs) / 86400;
            progress.set(1.0);
            num.set_text(if past == 0 { "🎉" } else { "✓" });
            unit.set_text("");
            rest.set_text(&if past == 0 { "C'est aujourd'hui !".to_string() } else if past == 1 { "C'était hier".to_string() } else { format!("Depuis {past} jours") });
        } else {
            let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
            if d > 0 {
                num.set_text(&d.to_string());
                unit.set_text(if d == 1 { "jour" } else { "jours" });
                rest.set_text(&format!("et {h} h {m:02} min"));
            } else {
                num.set_text(&format!("{h}:{m:02}"));
                unit.set_text("heures");
                rest.set_text("C'est bientôt !");
            }
            let total = created.as_ref().map(|c| target.difference(c).as_seconds()).filter(|t| *t > 0).unwrap_or(30 * 86400);
            progress.set((1.0 - secs as f64 / total as f64).clamp(0.0, 1.0));
        }
        area.queue_draw();
    };
    tick();
    every(&b, 30, tick);
    b.upcast()
}

/// Anneau : piste discrète, arc d'accent (part écoulée), extrémités arrondies.
fn draw_ring(a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, p: f64) {
    let accent = a.color();
    let fg = a.parent().and_then(|p| p.parent()).map(|p| p.color()).unwrap_or(accent);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let lw = w.min(h) * 0.085;
    let r = w.min(h) / 2.0 - lw / 2.0 - 1.0;
    cr.set_line_width(lw);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.12);
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
    let _ = cr.stroke();
    if p > 0.0 {
        let start = -PI / 2.0;
        cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 1.0);
        cr.arc(cx, cy, r, start, start + 2.0 * PI * p.max(0.01));
        let _ = cr.stroke();
    }
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Événement");
    let title = gtk::Entry::new();
    title.set_text(w.option("title").unwrap_or(""));
    title.set_placeholder_text(Some("Nom de l'événement"));
    b.append(&title);
    let target = w.option("target").and_then(parse).or_else(|| glib::DateTime::now_local().ok());
    let cal = gtk::Calendar::new();
    if let Some(t) = &target {
        cal.select_day(t);
    }
    b.append(&cal);
    let time = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    time.append(&gtk::Label::new(Some("Heure")));
    let hh = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let mm = gtk::SpinButton::with_range(0.0, 59.0, 5.0);
    hh.set_value(target.as_ref().map(|t| t.hour() as f64).unwrap_or(0.0));
    mm.set_value(target.as_ref().map(|t| t.minute() as f64).unwrap_or(0.0));
    for s in [&hh, &mm] {
        s.set_orientation(gtk::Orientation::Horizontal);
        s.set_numeric(true);
    }
    time.append(&hh);
    time.append(&gtk::Label::new(Some(":")));
    time.append(&mm);
    b.append(&time);
    let save = gtk::Button::with_label("Enregistrer");
    save.add_css_class("suggested-action");
    let ctx = ctx.clone();
    save.connect_clicked(move |_| {
        let d = cal.date();
        let target = format!("{:04}-{:02}-{:02}T{:02}:{:02}", d.year(), d.month(), d.day_of_month(), hh.value() as i32, mm.value() as i32);
        let name = title.text().trim().to_string();
        let now = glib::DateTime::now_local().ok().and_then(|n| n.format("%Y-%m-%dT%H:%M").ok()).map(|s| s.to_string()).unwrap_or_default();
        ctx.edit("Compte à rebours modifié", move |w| {
            w.options.insert("title".into(), if name.is_empty() { "Événement".into() } else { name });
            if w.options.get("target") != Some(&target) {
                // Nouvelle échéance : l'anneau repart de maintenant.
                w.options.insert("created".into(), now);
            }
            w.options.insert("target".into(), target);
        });
    });
    b.append(&save);
}

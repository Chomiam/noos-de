//! Batterie : charge, autonomie et mode d'alimentation (données du panneau Énergie : UPower,
//! démo `NOOS_POWER_DEMO`). Sur un PC fixe : le mode d'alimentation.
//!
//! - petite : jauge ronde et pourcentage, état en un mot ;
//! - moyenne : batterie dessinée, pourcentage, autonomie, mode ;
//! - grande : en plus, santé, consommation, cycles et modèle, et l'accès aux modes.

use std::{cell::RefCell, f64::consts::PI, rc::Rc};

use gtk::prelude::*;
use noos_core::{
    config::DesktopWidget,
    power::{self as pw, Battery, ChargeState},
};

use super::{Ctx, every, ring};

pub const CSS: &str = r#"
.desk-bat-pct { font-size: 40px; font-weight: 300; font-feature-settings: "tnum"; }
.size-s .desk-bat-pct { font-size: 17px; font-weight: 800; }
.size-l .desk-bat-pct { font-size: 52px; font-weight: 200; }
.desk-bat-pct.mode { font-size: 30px; font-weight: 400; }
.desk-bat-state { font-weight: 600; }
.size-s .desk-bat-state { font-size: 0.82em; opacity: 0.75; font-weight: 500; }
.desk-bat-mode { font-size: 0.85em; opacity: 0.75; }
.desk-bat-row { padding: 6px 10px; border-radius: 10px; background-color: alpha(currentColor, 0.06); }
.desk-bat-row .k { opacity: 0.7; font-size: 0.85em; }
.desk-bat-row .v { font-weight: 700; font-feature-settings: "tnum"; }
"#;

/// Couleur de la charge : vert en charge, orange sous 20 %, rouge sous 10 %, sinon bleu.
fn color(b: &Battery) -> (f64, f64, f64) {
    if b.state == ChargeState::Charging || b.state == ChargeState::Full {
        (0.18, 0.76, 0.49)
    } else if b.critical() {
        (0.90, 0.28, 0.30)
    } else if b.low() {
        (1.0, 0.60, 0.20)
    } else {
        (0.36, 0.56, 0.95)
    }
}

/// Batterie dessinée : corps arrondi, borne, remplissage proportionnel, éclair en charge.
fn draw_battery(cr: &gtk::cairo::Context, w: f64, h: f64, b: &Battery, fg: (f64, f64, f64)) {
    let rr = |x: f64, y: f64, w: f64, h: f64, r: f64| {
        cr.new_sub_path();
        cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
        cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
        cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
        cr.arc(x + r, y + r, r, PI, 1.5 * PI);
        cr.close_path();
    };
    let (bw, bh) = (w - 8.0, h - 2.0);
    cr.set_line_width(2.5);
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.85);
    rr(1.0, 1.0, bw, bh, 9.0);
    let _ = cr.stroke();
    rr(bw + 2.5, h / 2.0 - 7.0, 5.0, 14.0, 2.0);
    let _ = cr.fill();
    let c = color(b);
    cr.set_source_rgb(c.0, c.1, c.2);
    let fill = (bw - 8.0) * (b.percent / 100.0).clamp(0.02, 1.0);
    rr(5.0, 5.0, fill, bh - 8.0, 6.0);
    let _ = cr.fill();
    if b.state == ChargeState::Charging {
        let (cx, cy, s) = (1.0 + bw / 2.0, h / 2.0, bh * 0.36);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.move_to(cx + s * 0.15, cy - s);
        cr.line_to(cx - s * 0.45, cy + s * 0.12);
        cr.line_to(cx - s * 0.02, cy + s * 0.12);
        cr.line_to(cx - s * 0.15, cy + s);
        cr.line_to(cx + s * 0.45, cy - s * 0.12);
        cr.line_to(cx + s * 0.02, cy - s * 0.12);
        cr.close_path();
        let _ = cr.fill();
    }
}

/// Batterie actuelle et mode d'alimentation (depuis l'élément Énergie de la barre).
fn read(ctx: &Ctx) -> (Option<Battery>, String) {
    ctx.shell.upgrade().and_then(|s| s.power.borrow().as_ref().map(|p| p.snapshot())).unwrap_or_default()
}

/// Mot court de l'état (petite taille).
fn short(b: &Battery) -> String {
    match b.state {
        ChargeState::Charging => "En charge".into(),
        ChargeState::Full => "Chargée".into(),
        ChargeState::PendingCharge => "Branchée".into(),
        _ if b.time_to_empty > 0 => pw::duration(b.time_to_empty),
        _ => "Sur batterie".into(),
    }
}

pub fn build(ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let size = w.size().to_string();
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let state: Rc<RefCell<(Option<Battery>, String)>> = Rc::new(RefCell::new(read(ctx)));
    let fg_of = |a: &gtk::DrawingArea| {
        let c = a.color();
        (c.red() as f64, c.green() as f64, c.blue() as f64)
    };
    // Dessin : jauge ronde (petite) ou batterie (moyenne, grande).
    let area = gtk::DrawingArea::new();
    {
        let (st, size) = (state.clone(), size.clone());
        area.set_draw_func(move |a, cr, wd, ht| {
            let st = st.borrow();
            let fg = fg_of(a);
            match (&st.0, size.as_str()) {
                (Some(bat), "s") => ring(cr, wd as f64, ht as f64, bat.percent / 100.0, color(bat), fg),
                (Some(bat), _) => draw_battery(cr, wd as f64, ht as f64, bat, fg),
                // PC fixe : jauge du mode d'alimentation (économie, équilibré, performances).
                (None, _) => {
                    let (p, c) = match st.1.as_str() {
                        "power-saver" => (0.34, (0.18, 0.76, 0.49)),
                        "performance" => (1.0, (1.0, 0.55, 0.20)),
                        _ => (0.67, (0.36, 0.56, 0.95)),
                    };
                    let side = (wd.min(ht)) as f64;
                    cr.translate((wd as f64 - side) / 2.0, (ht as f64 - side) / 2.0);
                    ring(cr, side, side, p, c, fg);
                }
            }
        });
    }
    let pct = gtk::Label::new(None);
    pct.add_css_class("desk-bat-pct");
    let stat = gtk::Label::new(None);
    stat.add_css_class("desk-bat-state");
    stat.set_wrap(true);
    let mode = gtk::Label::new(None);
    mode.add_css_class("desk-bat-mode");
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
    if size == "s" {
        // Jauge, pourcentage au centre, état dessous.
        area.set_content_width(84);
        area.set_content_height(84);
        let o = gtk::Overlay::new();
        o.set_child(Some(&area));
        o.add_overlay(&pct);
        o.set_halign(gtk::Align::Center);
        b.append(&o);
        stat.set_halign(gtk::Align::Center);
        stat.set_justify(gtk::Justification::Center);
        b.append(&stat);
    } else {
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        area.set_content_width(if size == "l" { 120 } else { 96 });
        area.set_content_height(if size == "l" { 56 } else { 46 });
        area.set_valign(gtk::Align::Center);
        top.append(&area);
        pct.set_valign(gtk::Align::Center);
        top.append(&pct);
        b.append(&top);
        for l in [&stat, &mode] {
            l.set_xalign(0.0);
            b.append(l);
        }
        if size == "l" {
            b.append(&rows);
            let more = gtk::Button::with_label("Modes d'alimentation…");
            more.add_css_class("pill");
            let c = ctx.clone();
            more.connect_clicked(move |_| {
                if let Some(p) = c.shell.upgrade().and_then(|s| s.power.borrow().clone()) {
                    p.toggle();
                }
            });
            b.append(&more);
        }
    }
    let refresh = {
        let (ctx, st, area, size) = (ctx.clone(), state.clone(), area.clone(), size.clone());
        move || {
            *st.borrow_mut() = read(&ctx);
            let (bat, prof) = st.borrow().clone();
            let prof_label = if prof.is_empty() { String::new() } else { format!("Mode : {}", pw::profile_label(&prof)) };
            match &bat {
                Some(bat) => {
                    pct.set_text(&format!("{:.0} %", bat.percent));
                    stat.set_text(&if size == "s" { short(bat) } else { bat.status_text() });
                }
                None => {
                    let name = if prof.is_empty() { "Secteur".to_string() } else { pw::profile_label(&prof).to_string() };
                    pct.set_text(if size == "s" { "⚡" } else { &name });
                    if size != "s" {
                        // Jauge ronde à la place de la batterie dessinée, titre moins grand.
                        area.set_content_width(area.content_height());
                        pct.add_css_class("mode");
                    }
                    stat.set_text(if size == "s" { &name } else { "Sur secteur · aucune batterie" });
                }
            }
            // Sur PC fixe, le mode est déjà le titre.
            mode.set_text(&prof_label);
            mode.set_visible(!prof_label.is_empty() && bat.is_some());
            if size == "l" {
                while let Some(c) = rows.first_child() {
                    rows.remove(&c);
                }
                // Détails de la batterie physique (lus dans /sys, comme le panneau Énergie).
                let det = pw::read_sysfs();
                let add = |k: &str, v: String| {
                    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    r.add_css_class("desk-bat-row");
                    let kl = gtk::Label::new(Some(k));
                    kl.add_css_class("k");
                    kl.set_hexpand(true);
                    kl.set_xalign(0.0);
                    r.append(&kl);
                    let vl = gtk::Label::new(Some(&v));
                    vl.add_css_class("v");
                    r.append(&vl);
                    rows.append(&r);
                };
                if let Some(bat) = &bat {
                    let d = det.as_ref().unwrap_or(bat);
                    if d.capacity > 0.0 {
                        add("Santé", format!("{:.0} %", d.capacity));
                    }
                    if bat.rate > 0.0 {
                        add(if bat.state == ChargeState::Charging { "Puissance de charge" } else { "Consommation" }, format!("{:.1} W", bat.rate).replace('.', ","));
                    }
                    if d.cycles > 0 {
                        add("Cycles", d.cycles.to_string());
                    }
                    if !d.model.is_empty() {
                        add("Modèle", d.model.clone());
                    }
                }
            }
            area.queue_draw();
        }
    };
    refresh();
    every(&b, 5, refresh);
    b.upcast()
}

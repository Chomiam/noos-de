//! Calendrier façon Windows 11 : grille du mois (aujourd'hui en pastille d'accent), mois
//! précédent / suivant, numéros de semaine et prochain jour férié en option.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};
use noos_core::{config::DesktopWidget, holidays};

use super::{Ctx, by_size, capitalize, every, section, switch};

pub const CSS: &str = r#"
.desk-cal-head { font-weight: 700; font-size: 1.1em; }
.desk-cal-nav { min-width: 28px; min-height: 28px; padding: 0; border-radius: 99px; }
.desk-cal-dow { font-size: 0.78em; font-weight: 700; opacity: 0.6; }
.desk-cal-week { font-size: 0.72em; opacity: 0.45; }
.desk-cal-day { min-width: 34px; min-height: 34px; border-radius: 99px; font-feature-settings: "tnum"; }
.size-s .desk-cal-day { min-width: 26px; min-height: 26px; font-size: 0.85em; }
.size-l .desk-cal-day { min-width: 44px; min-height: 44px; font-size: 1.1em; }
.desk-cal-day.other { opacity: 0.3; }
.desk-cal-day.weekend { opacity: 0.7; }
.desk-cal-day.holiday { color: @accent_color; font-weight: 700; }
.desk-cal-day.today { background-color: @accent_bg_color; color: @accent_fg_color; font-weight: 700; opacity: 1; }
.desk-cal-wd { font-size: 0.8em; font-weight: 800; letter-spacing: 0.08em; color: @accent_color; }
.style-clear .desk-cal-wd { color: white; }
.desk-cal-big { font-size: 64px; font-weight: 300; font-feature-settings: "tnum"; }
.desk-cal-mo { font-weight: 600; }
.desk-cal-next { font-size: 0.82em; opacity: 0.7; }
.desk-cal-chip { min-width: 44px; padding: 4px 0; border-radius: 10px; background-color: alpha(@accent_bg_color, 0.2); }
.desk-cal-chip .n { font-weight: 800; font-size: 1.15em; }
.desk-cal-chip .m { font-size: 0.72em; opacity: 0.8; }
.desk-cal-foot { font-size: 0.85em; padding: 8px 10px; border-radius: 10px; background-color: alpha(currentColor, 0.07); }
"#;

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    match w.size() {
        "s" => today(),
        "l" => {
            // Grande taille : la grille (avec les semaines) et, à côté, les prochains fériés.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
            row.append(&month(w, true));
            row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
            row.append(&upcoming());
            row.upcast()
        }
        _ => month(w, false),
    }
}

/// Petite taille : le jour (jour de la semaine, quantième, mois) et le prochain férié.
fn today() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_size_request(150, -1);
    let wd = gtk::Label::new(None);
    wd.add_css_class("desk-cal-wd");
    let num = gtk::Label::new(None);
    num.add_css_class("desk-cal-big");
    let mo = gtk::Label::new(None);
    mo.add_css_class("desk-cal-mo");
    let next = gtk::Label::new(None);
    next.add_css_class("desk-cal-next");
    next.set_margin_top(8);
    next.set_wrap(true);
    next.set_max_width_chars(18);
    for l in [&wd, &num, &mo, &next] {
        l.set_xalign(0.0);
        b.append(l);
    }
    let fill = move || {
        let Ok(now) = glib::DateTime::now_local() else { return };
        wd.set_text(&now.format("%A").map(|s| s.to_uppercase()).unwrap_or_default());
        num.set_text(&now.day_of_month().to_string());
        mo.set_text(&capitalize(&now.format("%B %Y").map(|s| s.to_string()).unwrap_or_default()));
        let (y, m, d) = now.ymd();
        let (_, _, _, name) = holidays::next_holiday(y, m as u32, d as u32);
        next.set_text(&format!("{name} {}", days_until(&now, holidays::next_holiday(y, m as u32, d as u32))));
    };
    fill();
    every(&b, 60, fill);
    b.upcast()
}

/// « dans 37 jours », « demain », « aujourd'hui ».
fn days_until(now: &glib::DateTime, (hy, hm, hd, _): (i32, u32, u32, &str)) -> String {
    let (ty, tm, td) = now.ymd();
    let (Ok(h), Ok(t)) = (glib::DateTime::from_local(hy, hm as i32, hd as i32, 0, 0, 0.0), glib::DateTime::from_local(ty, tm, td, 0, 0, 0.0)) else { return String::new() };
    match h.difference(&t).as_seconds() / 86400 {
        0 => "aujourd'hui".into(),
        1 => "demain".into(),
        n => format!("dans {n} jours"),
    }
}

/// Grande taille : les quatre prochains jours fériés.
fn upcoming() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_size_request(190, -1);
    let t = gtk::Label::new(Some("À venir"));
    t.add_css_class("desk-cal-head");
    t.set_xalign(0.0);
    b.append(&t);
    let Ok(now) = glib::DateTime::now_local() else { return b.upcast() };
    let (mut y, mut m, mut d) = now.ymd();
    for _ in 0..4 {
        let h = holidays::next_holiday(y, m as u32, d as u32);
        let Ok(date) = glib::DateTime::from_local(h.0, h.1 as i32, h.2 as i32, 0, 0, 0.0) else { break };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("desk-cal-event");
        let chip = gtk::Box::new(gtk::Orientation::Vertical, 0);
        chip.add_css_class("desk-cal-chip");
        let dn = gtk::Label::new(Some(&h.2.to_string()));
        dn.add_css_class("n");
        chip.append(&dn);
        let mn = gtk::Label::new(Some(&date.format("%b").map(|s| s.trim_end_matches('.').to_uppercase()).unwrap_or_default()));
        mn.add_css_class("m");
        chip.append(&mn);
        row.append(&chip);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
        col.set_valign(gtk::Align::Center);
        let n = gtk::Label::new(Some(h.3));
        n.add_css_class("heading");
        n.set_xalign(0.0);
        col.append(&n);
        let w = gtk::Label::new(Some(&format!("{} · {}", capitalize(&date.format("%A").map(|s| s.to_string()).unwrap_or_default()), days_until(&now, h))));
        w.add_css_class("desk-cal-next");
        w.set_xalign(0.0);
        col.append(&w);
        row.append(&col);
        b.append(&row);
        // Lendemain du férié trouvé.
        let Ok(next) = date.add_days(1) else { break };
        (y, m, d) = next.ymd();
    }
    b.upcast()
}

fn month(w: &DesktopWidget, large: bool) -> gtk::Widget {
    let weeks = large || w.option("weeks") == Some("1");
    let footer = !large && w.option("holidays") != Some("0");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let title = gtk::Label::new(None);
    title.add_css_class("desk-cal-head");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    head.append(&title);
    let today_btn = gtk::Button::with_label("Aujourd'hui");
    today_btn.add_css_class("flat");
    head.append(&today_btn);
    let prev = gtk::Button::from_icon_name("go-up-symbolic");
    let next = gtk::Button::from_icon_name("go-down-symbolic");
    for (btn, tip) in [(&prev, "Mois précédent"), (&next, "Mois suivant")] {
        btn.add_css_class("flat");
        btn.add_css_class("desk-cal-nav");
        btn.set_tooltip_text(Some(tip));
        head.append(btn);
    }
    b.append(&head);
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(false);
    grid.set_row_spacing(by_size(w, 0, 2, 4));
    grid.set_column_spacing(by_size(w, 0, 2, 4));
    grid.set_halign(gtk::Align::Center);
    b.append(&grid);
    let foot = gtk::Label::new(None);
    foot.add_css_class("desk-cal-foot");
    foot.set_xalign(0.0);
    foot.set_wrap(true);
    foot.set_visible(footer);
    b.append(&foot);

    let offset = Rc::new(Cell::new(0i32));
    let fill = {
        let (grid, title, today_btn, foot, offset) = (grid.clone(), title.clone(), today_btn.clone(), foot.clone(), offset.clone());
        Rc::new(move || {
            while let Some(c) = grid.first_child() {
                grid.remove(&c);
            }
            let Ok(now) = glib::DateTime::now_local() else { return };
            let (ty, tm, td) = now.ymd();
            let Ok(first) = glib::DateTime::from_local(ty, tm, 1, 0, 0, 0.0).and_then(|d| d.add_months(offset.get())) else { return };
            let (_, m, _) = first.ymd();
            title.set_text(&capitalize(&first.format("%B %Y").map(|s| s.to_string()).unwrap_or_default()));
            today_btn.set_visible(offset.get() != 0);
            let col0 = if weeks { 1 } else { 0 };
            for (i, d) in ["lu", "ma", "me", "je", "ve", "sa", "di"].iter().enumerate() {
                let l = gtk::Label::new(Some(d));
                l.add_css_class("desk-cal-dow");
                grid.attach(&l, col0 + i as i32, 0, 1, 1);
            }
            // Lundi de la première semaine affichée.
            let start = first.add_days(1 - first.day_of_week()).unwrap_or(first.clone());
            for row in 0..6 {
                let Ok(monday) = start.add_days(row * 7) else { continue };
                if weeks {
                    let l = gtk::Label::new(Some(&monday.week_of_year().to_string()));
                    l.add_css_class("desk-cal-week");
                    grid.attach(&l, 0, row + 1, 1, 1);
                }
                for col in 0..7 {
                    let Ok(day) = monday.add_days(col) else { continue };
                    let (dy, dm, dd) = day.ymd();
                    let l = gtk::Label::new(Some(&dd.to_string()));
                    l.add_css_class("desk-cal-day");
                    if dm != m {
                        l.add_css_class("other");
                    } else if col >= 5 {
                        l.add_css_class("weekend");
                    }
                    if (dy, dm, dd) == (ty, tm, td) {
                        l.add_css_class("today");
                    }
                    if let Some(name) = holidays::holiday_name(dy, dm as u32, dd as u32) {
                        l.add_css_class("holiday");
                        l.set_tooltip_text(Some(name));
                    }
                    grid.attach(&l, col0 + col, row + 1, 1, 1);
                }
            }
            // Prochain jour férié.
            let (hy, hm, hd, name) = holidays::next_holiday(ty, tm as u32, td as u32);
            if let Ok(h) = glib::DateTime::from_local(hy, hm as i32, hd as i32, 0, 0, 0.0) {
                let today = glib::DateTime::from_local(ty, tm, td, 0, 0, 0.0).ok();
                let days = today.map(|t| (h.difference(&t).as_seconds() / 86400) as i64).unwrap_or(0);
                let when = match days {
                    0 => "aujourd'hui".to_string(),
                    1 => "demain".to_string(),
                    n => format!("dans {n} jours"),
                };
                foot.set_text(&format!("Prochain jour férié : {name}, {} ({when})", h.format("%A %-d %B").map(|s| s.to_string()).unwrap_or_default()));
            }
        })
    };
    fill();
    for (btn, delta) in [(&prev, -1), (&next, 1), (&today_btn, 0)] {
        let (fill, offset) = (fill.clone(), offset.clone());
        btn.connect_clicked(move |_| {
            offset.set(if delta == 0 { 0 } else { offset.get() + delta });
            fill();
        });
    }
    // Molette : mois voisin.
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
    {
        let (fill, offset) = (fill.clone(), offset.clone());
        scroll.connect_scroll(move |_, _, dy| {
            offset.set(offset.get() + if dy > 0.0 { 1 } else { -1 });
            fill();
            glib::Propagation::Stop
        });
    }
    grid.add_controller(scroll);
    // Changement de jour : grille refaite (sans perdre le mois affiché).
    let day = Rc::new(Cell::new(glib::DateTime::now_local().map(|d| d.day_of_year()).unwrap_or(0)));
    every(&b, 60, move || {
        let d = glib::DateTime::now_local().map(|d| d.day_of_year()).unwrap_or(0);
        if day.replace(d) != d {
            fill();
        }
    });
    b.upcast()
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Affichage");
    let c = ctx.clone();
    switch(b, "Numéros de semaine", w.option("weeks") == Some("1"), move |on| c.set_option("Numéros de semaine", "weeks", if on { "1" } else { "0" }));
    let c = ctx.clone();
    switch(b, "Prochain jour férié", w.option("holidays") != Some("0"), move |on| c.set_option("Jours fériés du calendrier", "holidays", if on { "1" } else { "0" }));
}

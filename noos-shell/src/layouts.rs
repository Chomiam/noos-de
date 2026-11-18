//! Mises en page (mode édition) : modèles prédéfinis avec miniature dessinée, mises en page
//! enregistrées (barres + widgets), import / export d'un fichier, réinitialisation.

use std::rc::Rc;

use gtk::{gdk, gio, prelude::*};
use noos_core::{
    NoosConfig,
    layout::{self, Layout},
};

use crate::Shell;

pub const CSS: &str = r#"
.edit-pop { padding: 8px 6px; }
.edit-pop .pop-title { font-weight: 800; font-size: 1.05em; }
.layout-card { padding: 8px; border-radius: 14px; border: 2px solid transparent; }
.layout-card:hover { background-color: alpha(@accent_bg_color, 0.10); }
.layout-card.active { border-color: @accent_bg_color; background-color: alpha(@accent_bg_color, 0.12); }
.layout-card .layout-name { font-weight: 700; font-size: 0.9em; }
.layout-thumb { color: @accent_bg_color; }
.saved-row { padding: 2px 0; }
"#;

/// Éléments affichés d'une barre (zones gauche, centre, droite).
fn shown(cfg: &NoosConfig, status: bool) -> [Vec<String>; 3] {
    let z = cfg.bar_zones();
    let k = if status { 3 } else { 0 };
    let off = |id: &str| match id {
        "search" => !cfg.taskbar.show_search,
        "vitals" => !cfg.taskbar.show_vitals,
        "power" => !cfg.taskbar.show_power,
        "taskview" => !cfg.desktops.taskbar_button,
        _ => false,
    };
    let f = |l: &Vec<String>| l.iter().filter(|i| !off(i)).cloned().collect::<Vec<_>>();
    let (mut l, mut c, r) = (f(&z[k]), f(&z[k + 1]), f(&z[k + 2]));
    if !status && cfg.taskbar.alignment == noos_core::config::TaskbarAlignment::Left && !cfg.taskbar.vertical() {
        l.append(&mut c);
    }
    [l, c, r]
}

/// Miniature d'une disposition : écran, barre des tâches, barre d'état, coin des notifications.
pub fn draw(cr: &gtk::cairo::Context, w: f64, h: f64, l: &Layout, accent: &gdk::RGBA) {
    use std::f64::consts::PI;
    let mut cfg = NoosConfig::default();
    l.apply(&mut cfg);
    let rr = |x: f64, y: f64, w: f64, h: f64, r: f64| {
        let r = r.min(w / 2.0).min(h / 2.0);
        cr.new_sub_path();
        cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
        cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
        cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
        cr.arc(x + r, y + r, r, PI, 1.5 * PI);
        cr.close_path();
    };
    // Écran.
    let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, w, h);
    grad.add_color_stop_rgb(0.0, 0.22, 0.30, 0.55);
    grad.add_color_stop_rgb(1.0, 0.09, 0.11, 0.22);
    rr(0.5, 0.5, w - 1.0, h - 1.0, 7.0);
    let _ = cr.set_source(&grad);
    let _ = cr.fill();
    let (ar, ag, ab) = (accent.red() as f64, accent.green() as f64, accent.blue() as f64);
    let mut work = (0.0, 0.0, w, h);
    for status in [false, true] {
        if status && !cfg.topbar.enabled {
            continue;
        }
        let edge = crate::taskbar::bar_edge(&cfg, status);
        let t = if status { 7.0 } else { 10.0 };
        let vertical = matches!(edge, "left" | "right");
        let (bx, by, bw, bh) = match edge {
            "top" => (0.0, 0.0, w, t),
            "left" => (0.0, 0.0, t, h),
            "right" => (w - t, 0.0, t, h),
            _ => (0.0, h - t, w, t),
        };
        cr.save().ok();
        rr(0.5, 0.5, w - 1.0, h - 1.0, 7.0);
        cr.clip();
        cr.set_source_rgba(1.0, 1.0, 1.0, if status { 0.16 } else { 0.22 });
        cr.rectangle(bx, by, bw, bh);
        let _ = cr.fill();
        cr.restore().ok();
        match edge {
            "top" => {
                work.1 += t;
                work.3 -= t;
            }
            "left" => {
                work.0 += t;
                work.2 -= t;
            }
            "right" => work.2 -= t,
            _ => work.3 -= t,
        }
        // Éléments : petits pavés le long de la barre.
        let s = t - 4.0;
        let len = if vertical { h } else { w };
        let width = |id: &str| match id {
            "tasks" => 3.0 * s + 4.0,
            "clock" => 2.2 * s,
            "search" => 3.0 * s,
            "vitals" => 2.6 * s,
            "tray" | "status" => 1.6 * s,
            "desktops" => 2.6 * s,
            _ => s,
        };
        let zones = shown(&cfg, status);
        for (k, ids) in zones.iter().enumerate() {
            let total: f64 = ids.iter().map(|i| width(i) + 2.0).sum::<f64>() - 2.0;
            let mut p = match k {
                0 => 3.0,
                1 => (len - total) / 2.0,
                _ => len - 3.0 - total,
            };
            for id in ids {
                let wd = width(id);
                let (ix, iy, iw, ih) = if vertical { (bx + 2.0, p, s, wd) } else { (p, by + 2.0, wd, s) };
                match id.as_str() {
                    "start" => cr.set_source_rgba(ar, ag, ab, 1.0),
                    "tasks" => cr.set_source_rgba(1.0, 1.0, 1.0, 0.75),
                    "clock" => cr.set_source_rgba(1.0, 1.0, 1.0, 0.65),
                    "desktops" => cr.set_source_rgba(ar, ag, ab, 0.75),
                    _ => cr.set_source_rgba(1.0, 1.0, 1.0, 0.42),
                }
                if id == "tasks" {
                    for j in 0..3 {
                        let o = j as f64 * (s + 2.0);
                        if vertical { rr(ix, iy + o, s, s, 1.2) } else { rr(ix + o, iy, s, s, 1.2) }
                    }
                } else {
                    rr(ix, iy, iw, ih, 1.2);
                }
                let _ = cr.fill();
                p += wd + 2.0;
            }
        }
    }
    // Notifications.
    let corner = cfg.notifications.position.as_str();
    let (nw, nh) = (w * 0.2, h * 0.11);
    let nx = if corner.ends_with("left") { work.0 + 4.0 } else { work.0 + work.2 - nw - 4.0 };
    let ny = if corner.starts_with("top") { work.1 + 4.0 } else { work.1 + work.3 - nh - 4.0 };
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.3);
    rr(nx, ny, nw, nh, 2.5);
    let _ = cr.fill();
}

/// Zone de dessin d'une miniature.
pub fn thumb(l: Layout, w: i32, h: i32) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(w);
    a.set_content_height(h);
    a.add_css_class("layout-thumb");
    a.set_draw_func(move |a, cr, w, h| draw(cr, w as f64, h as f64, &l, &a.color()));
    a
}

/// Remplit le panneau « Mises en page » (refait à chaque ouverture).
pub fn fill(shell: &Rc<Shell>, b: &gtk::Box, pop: &gtk::Popover) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    let cfg = shell.config();
    let current = Layout::capture(&cfg, false);
    let title = |t: &str, sub: &str| {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let l = gtk::Label::new(Some(t));
        l.add_css_class("pop-title");
        l.set_xalign(0.0);
        v.append(&l);
        let s = gtk::Label::new(Some(sub));
        s.add_css_class("dim-label");
        s.add_css_class("caption");
        s.set_xalign(0.0);
        v.append(&s);
        v
    };
    b.append(&title("Modèles", "Disposition des barres et des notifications — vos widgets restent en place"));
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::None);
    grid.set_min_children_per_line(3);
    grid.set_max_children_per_line(3);
    grid.set_homogeneous(true);
    for p in layout::presets() {
        let card = gtk::Button::new();
        card.add_css_class("flat");
        card.add_css_class("layout-card");
        let active = p.layout.same_bars(&current);
        if active {
            card.add_css_class("active");
        }
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        v.append(&thumb(p.layout.clone(), 150, 90));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row.set_halign(gtk::Align::Center);
        if active {
            row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let n = gtk::Label::new(Some(p.name));
        n.add_css_class("layout-name");
        row.append(&n);
        v.append(&row);
        card.set_child(Some(&v));
        card.set_tooltip_text(Some(p.description));
        let (shell, pop, l, name) = (Rc::downgrade(shell), pop.clone(), p.layout.clone(), p.name);
        card.connect_clicked(move |_| {
            pop.popdown();
            if let Some(s) = shell.upgrade() {
                let l = l.clone();
                s.edit_layout(&format!("Modèle « {name} »"), move |c| l.apply(c));
            }
        });
        grid.insert(&card, -1);
    }
    b.append(&grid);
    b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    b.append(&title("Mes mises en page", "Barres et widgets, pour passer d'un bureau à l'autre"));
    let saved = layout::list_saved();
    if saved.is_empty() {
        let l = gtk::Label::new(Some("Aucune pour l'instant : donnez un nom à la disposition actuelle."));
        l.add_css_class("dim-label");
        l.set_xalign(0.0);
        l.set_wrap(true);
        b.append(&l);
    }
    for (path, l) in saved {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("saved-row");
        row.append(&thumb(l.clone(), 48, 29));
        let n = gtk::Label::new(Some(&l.name));
        n.set_hexpand(true);
        n.set_xalign(0.0);
        let count = l.widgets.as_ref().map(|w| w.len()).unwrap_or(0);
        n.set_tooltip_text(Some(&format!("{count} widget{}", if count > 1 { "s" } else { "" })));
        row.append(&n);
        let apply = gtk::Button::with_label("Appliquer");
        {
            let (shell, pop, l) = (Rc::downgrade(shell), pop.clone(), l.clone());
            apply.connect_clicked(move |_| {
                pop.popdown();
                if let Some(s) = shell.upgrade() {
                    let l = l.clone();
                    s.edit_layout(&format!("Mise en page « {} »", l.name), move |c| l.apply(c));
                }
            });
        }
        row.append(&apply);
        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.set_tooltip_text(Some("Supprimer cette mise en page"));
        {
            let (shell, b2, pop2) = (Rc::downgrade(shell), b.clone(), pop.clone());
            del.connect_clicked(move |_| {
                let _ = std::fs::remove_file(&path);
                if let Some(s) = shell.upgrade() {
                    fill(&s, &b2, &pop2);
                }
            });
        }
        row.append(&del);
        b.append(&row);
    }
    let save_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("Nom (ex. Travail, Jeu)"));
    entry.set_hexpand(true);
    save_row.append(&entry);
    let save = gtk::Button::with_label("Enregistrer");
    save.add_css_class("suggested-action");
    let do_save = {
        let (shell, entry, b2, pop2) = (Rc::downgrade(shell), entry.clone(), b.clone(), pop.clone());
        move || {
            let name = entry.text().trim().to_string();
            if name.is_empty() {
                entry.grab_focus();
                return;
            }
            let Some(s) = shell.upgrade() else { return };
            let mut l = Layout::capture(&s.config(), true);
            l.name = name.clone();
            match layout::save_named(&l) {
                Ok(_) => toast(&s, &format!("Mise en page « {name} » enregistrée")),
                Err(e) => toast(&s, &format!("Enregistrement impossible : {e}")),
            }
            fill(&s, &b2, &pop2);
        }
    };
    let do_save = Rc::new(do_save);
    {
        let f = do_save.clone();
        save.connect_clicked(move |_| f());
    }
    entry.connect_activate(move |_| do_save());
    save_row.append(&save);
    b.append(&save_row);
    b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    actions.set_homogeneous(true);
    let import = gtk::Button::with_label("Importer…");
    let export = gtk::Button::with_label("Exporter…");
    let reset = gtk::Button::with_label("Réinitialiser");
    reset.set_tooltip_text(Some("Disposition d'origine des barres (Ctrl+Z pour revenir en arrière)"));
    {
        let (shell, pop) = (Rc::downgrade(shell), pop.clone());
        import.connect_clicked(move |btn| {
            pop.popdown();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Mises en page NOOS (.toml)"));
            filter.add_pattern("*.toml");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let d = gtk::FileDialog::builder().title("Importer une mise en page").filters(&filters).build();
            let shell = shell.clone();
            let win = btn.root().and_downcast::<gtk::Window>();
            d.open(win.as_ref(), gio::Cancellable::NONE, move |r| {
                let (Some(s), Ok(f)) = (shell.upgrade(), r) else { return };
                let Some(path) = f.path() else { return };
                match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|t| Layout::from_toml(&t)) {
                    Ok(mut l) => {
                        if l.name.is_empty() {
                            l.name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        }
                        let name = l.name.clone();
                        s.edit_layout(&format!("Mise en page « {name} » importée"), move |c| l.apply(c));
                    }
                    Err(e) => toast(&s, &format!("Import impossible : {e}")),
                }
            });
        });
    }
    {
        let (shell, pop) = (Rc::downgrade(shell), pop.clone());
        export.connect_clicked(move |btn| {
            pop.popdown();
            let d = gtk::FileDialog::builder().title("Exporter la mise en page").initial_name("mise-en-page-noos.toml").build();
            let shell = shell.clone();
            let win = btn.root().and_downcast::<gtk::Window>();
            d.save(win.as_ref(), gio::Cancellable::NONE, move |r| {
                let (Some(s), Ok(f)) = (shell.upgrade(), r) else { return };
                let Some(path) = f.path() else { return };
                let mut l = Layout::capture(&s.config(), true);
                l.name = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let res = l.to_toml().and_then(|t| Ok(std::fs::write(&path, t)?));
                toast(&s, &match res {
                    Ok(()) => format!("Mise en page exportée : {}", path.display()),
                    Err(e) => format!("Export impossible : {e}"),
                });
            });
        });
    }
    {
        let (shell, pop) = (Rc::downgrade(shell), pop.clone());
        reset.connect_clicked(move |_| {
            pop.popdown();
            if let Some(s) = shell.upgrade() {
                let l = layout::presets().remove(0).layout;
                s.edit_layout("Barres réinitialisées", move |c| l.apply(c));
            }
        });
    }
    actions.append(&import);
    actions.append(&export);
    actions.append(&reset);
    b.append(&actions);
}

fn toast(s: &Rc<Shell>, text: &str) {
    if let Some(e) = s.editmode.borrow().as_ref() {
        e.toast(text, false);
    }
}

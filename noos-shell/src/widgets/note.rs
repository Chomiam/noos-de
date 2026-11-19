//! Note : pense-bête de couleur, texte enregistré au fil de la frappe.

use std::{cell::Cell, rc::Rc};

use gtk::{glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, section};

pub const COLORS: &[(&str, &str)] = &[("yellow", "Jaune"), ("pink", "Rose"), ("green", "Vert"), ("blue", "Bleu"), ("purple", "Violet")];

pub const CSS: &str = r#"
.kind-note { padding: 0; background: none; border: none; box-shadow: none; }
.desk-note { border-radius: 6px 6px 18px 6px; padding: 14px; box-shadow: 0 10px 26px rgba(0,0,0,0.32); }
.desk-note textview, .desk-note text { background: transparent; font-size: 1.05em; }
.size-l .desk-note textview { font-size: 1.25em; }
.desk-note.yellow { background-image: linear-gradient(170deg, #fff6b8, #ffe97a); }
.desk-note.pink { background-image: linear-gradient(170deg, #ffd6e7, #ffb3d1); }
.desk-note.green { background-image: linear-gradient(170deg, #d8f8d0, #b4ecaa); }
.desk-note.blue { background-image: linear-gradient(170deg, #d4ecff, #b0d8ff); }
.desk-note.purple { background-image: linear-gradient(170deg, #e8dcff, #d2bdff); }
.desk-note, .desk-note textview, .desk-note text { color: #34302a; caret-color: #34302a; }
.desk-note-hint { color: rgba(52,48,42,0.45); font-size: 1.05em; }
.size-l .desk-note-hint { font-size: 1.25em; }
.note-swatch { min-width: 26px; min-height: 26px; padding: 0; border-radius: 99px; border: 2px solid alpha(currentColor, 0.15); }
.note-swatch:checked { border-color: @accent_bg_color; box-shadow: 0 0 0 2px alpha(@accent_bg_color, 0.4); }
.note-swatch.yellow { background: #ffe97a; } .note-swatch.pink { background: #ffb3d1; } .note-swatch.green { background: #b4ecaa; }
.note-swatch.blue { background: #b0d8ff; } .note-swatch.purple { background: #d2bdff; }
"#;

pub fn build(ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let color = w.option("color").filter(|c| COLORS.iter().any(|(id, _)| id == c)).unwrap_or("yellow");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("desk-note");
    b.add_css_class(color);
    let tv = gtk::TextView::new();
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    // Largeur commune (moins les marges du papier) ; la grande taille est plus haute.
    tv.set_size_request(super::width(w) - 28, by_size(w, 120, 170, 300));
    tv.buffer().set_text(&w.text);
    // Texte indicatif (pas du vrai texte à effacer) tant que la note est vide.
    let o = gtk::Overlay::new();
    o.set_child(Some(&tv));
    let hint = gtk::Label::new(Some("Écrivez ici…"));
    hint.add_css_class("desk-note-hint");
    hint.set_halign(gtk::Align::Start);
    hint.set_valign(gtk::Align::Start);
    hint.set_can_target(false);
    hint.set_visible(w.text.is_empty());
    o.add_overlay(&hint);
    {
        let hint = hint.clone();
        tv.buffer().connect_changed(move |b| hint.set_visible(b.char_count() == 0));
    }
    // Enregistrée après une courte pause, sans reconstruire le widget (le curseur resterait
    // en place) ni passer par l'historique.
    let pending: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let ctx = ctx.clone();
    tv.buffer().connect_changed(move |buf| {
        let n = pending.get() + 1;
        pending.set(n);
        let (p, ctx, text) = (pending.clone(), ctx.clone(), buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string());
        glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
            if p.get() == n {
                ctx.save_quietly(|w| w.text = text);
            }
        });
    });
    b.append(&o);
    b.upcast()
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Couleur");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let current = w.option("color").unwrap_or("yellow");
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, label) in COLORS {
        let t = gtk::ToggleButton::new();
        t.add_css_class("note-swatch");
        t.add_css_class(id);
        t.set_tooltip_text(Some(label));
        if let Some(g) = &first {
            t.set_group(Some(g));
        } else {
            first = Some(t.clone());
        }
        t.set_active(*id == current);
        let c = ctx.clone();
        t.connect_toggled(move |t| {
            if t.is_active() {
                c.set_option("Couleur de la note", "color", id);
            }
        });
        row.append(&t);
    }
    b.append(&row);
}

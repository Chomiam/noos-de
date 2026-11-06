//! Clavier visuel (accessibilité) : tape dans le dernier champ de saisie actif, sans
//! jamais prendre le focus. Disposition AZERTY si le système est en français/belge.

use std::{cell::{Cell, RefCell}, rc::Rc};

use gtk::prelude::*;

const SYMBOLS: [&str; 3] = ["!@#$%^&*()", "-_=+[]{};:", "'\",.<>/?\\|"];

pub struct Osk {
    pub widget: gtk::Revealer,
    target: RefCell<Option<gtk::Editable>>,
    shift: Cell<bool>,
    symbols: Cell<bool>,
    rows: gtk::Box,
    layout: Vec<&'static str>,
    on_enter: RefCell<Option<Box<dyn Fn()>>>,
}

impl Osk {
    pub fn new(xkb_layout: &str) -> Rc<Self> {
        let layout = match xkb_layout {
            "fr" | "be" => vec!["1234567890", "azertyuiop", "qsdfghjklm", "wxcvbn"],
            "de" | "ch" | "at" => vec!["1234567890", "qwertzuiop", "asdfghjkl", "yxcvbnm"],
            _ => vec!["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"],
        };
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 6);
        rows.add_css_class("osk");
        rows.set_halign(gtk::Align::Center);
        let widget = gtk::Revealer::new();
        widget.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        widget.set_child(Some(&rows));
        widget.set_valign(gtk::Align::End);
        let osk = Rc::new(Self {
            widget,
            target: RefCell::new(None),
            shift: Cell::new(false),
            symbols: Cell::new(false),
            rows,
            layout,
            on_enter: RefCell::new(None),
        });
        osk.rebuild();
        osk
    }

    pub fn set_target(&self, editable: &impl IsA<gtk::Editable>) {
        *self.target.borrow_mut() = Some(editable.clone().upcast());
    }

    pub fn connect_enter(&self, f: impl Fn() + 'static) {
        *self.on_enter.borrow_mut() = Some(Box::new(f));
    }

    fn key(self: &Rc<Self>, label: &str, width: i32, action: impl Fn(&Rc<Self>) + 'static) -> gtk::Button {
        let b = gtk::Button::with_label(label);
        b.add_css_class("osk-key");
        b.set_focus_on_click(false);
        b.set_can_focus(false);
        b.set_size_request(width, 48);
        let weak = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            if let Some(o) = weak.upgrade() {
                action(&o);
            }
        });
        b
    }

    fn type_text(&self, text: &str) {
        let Some(e) = self.target.borrow().clone() else { return };
        e.delete_selection();
        let mut pos = e.position();
        e.insert_text(text, &mut pos);
        e.set_position(pos);
    }

    fn backspace(&self) {
        let Some(e) = self.target.borrow().clone() else { return };
        if e.selection_bounds().is_some() {
            e.delete_selection();
            return;
        }
        let pos = e.position();
        if pos > 0 {
            e.delete_text(pos - 1, pos);
            e.set_position(pos - 1);
        }
    }

    fn rebuild(self: &Rc<Self>) {
        while let Some(c) = self.rows.first_child() {
            self.rows.remove(&c);
        }
        let shift = self.shift.get();
        let rows: Vec<String> = if self.symbols.get() {
            std::iter::once(self.layout[0].to_string()).chain(SYMBOLS.iter().map(|s| s.to_string())).collect()
        } else {
            self.layout.iter().map(|r| if shift { r.to_uppercase() } else { r.to_string() }).collect()
        };
        for (i, row) in rows.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.set_halign(gtk::Align::Center);
            if i == rows.len() - 1 {
                let label = if self.symbols.get() { "abc" } else if shift { "⇧ Maj" } else { "⇧" };
                let s = self.key(label, 84, |o| {
                    if o.symbols.get() {
                        o.symbols.set(false);
                    } else {
                        o.shift.set(!o.shift.get());
                    }
                    o.rebuild();
                });
                if shift {
                    s.add_css_class("suggested-action");
                }
                line.append(&s);
            }
            for ch in row.chars() {
                let text = ch.to_string();
                let label = text.clone();
                line.append(&self.key(&label, 48, move |o| {
                    o.type_text(&text);
                    if o.shift.get() {
                        o.shift.set(false);
                        o.rebuild();
                    }
                }));
            }
            if i == rows.len() - 1 {
                line.append(&self.key("⌫", 84, |o| o.backspace()));
            }
            self.rows.append(&line);
        }
        let last = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        last.set_halign(gtk::Align::Center);
        last.append(&self.key(if self.symbols.get() { "abc" } else { "?123" }, 84, |o| {
            o.symbols.set(!o.symbols.get());
            o.rebuild();
        }));
        last.append(&self.key("@", 48, |o| o.type_text("@")));
        last.append(&self.key("Espace", 330, |o| o.type_text(" ")));
        last.append(&self.key(".", 48, |o| o.type_text(".")));
        let enter = self.key("Entrée ⏎", 120, |o| {
            if let Some(f) = o.on_enter.borrow().as_ref() {
                f();
            }
        });
        enter.add_css_class("suggested-action");
        last.append(&enter);
        self.rows.append(&last);
    }
}

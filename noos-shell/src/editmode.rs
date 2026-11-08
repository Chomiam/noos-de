//! Mode édition du bureau (comme KDE) : la barre des tâches se réorganise par glisser-déposer
//! (éléments déplacés d'une zone à l'autre, barre en haut ou en bas) et des widgets se posent
//! librement sur le fond d'écran (horloge, ressources, note, calendrier).
//!
//! Entrées : clic droit sur le bureau ou la barre, Paramètres › Barre des tâches, IPC
//! `toggle-edit-mode`. Échap ou « Terminé » pour sortir.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::{NoosConfig, config::DesktopWidget};

use crate::Shell;

pub const CSS: &str = r#"
.edit-shield { border-radius: 8px; }
.editing .edit-shield { border: 1.5px dashed alpha(@accent_bg_color, 0.85); background-color: alpha(@accent_bg_color, 0.10); }
.editing .edit-shield:hover { background-color: alpha(@accent_bg_color, 0.28); }
.editing .edit-zone { border-radius: 10px; min-width: 56px; min-height: 30px; background-color: alpha(@window_fg_color, 0.04);
  box-shadow: inset 0 0 0 1.5px alpha(@window_fg_color, 0.18); }
.editing .edit-zone.drop-hover { box-shadow: inset 0 0 0 2px @accent_bg_color; background-color: alpha(@accent_bg_color, 0.12); }
.edit-toolbar { padding: 10px 14px; border-radius: 16px; background-color: @window_bg_color; color: @window_fg_color;
  border: 1px solid alpha(@window_fg_color, 0.12); box-shadow: 0 10px 32px rgba(0,0,0,0.4); }
.edit-toolbar .edit-title { font-weight: 700; }
.desk-widget { border-radius: 16px; }
.desk-widget.card-bg { padding: 14px 16px; background-color: alpha(@window_bg_color, 0.78); color: @window_fg_color;
  border: 1px solid alpha(@window_fg_color, 0.08); box-shadow: 0 8px 24px rgba(0,0,0,0.25); }
.desk-widget.editing { box-shadow: 0 0 0 2px @accent_bg_color, 0 8px 24px rgba(0,0,0,0.3); }
.desk-clock-time { font-size: 72px; font-weight: 300; color: white; text-shadow: 0 2px 16px rgba(0,0,0,0.45); }
.desk-clock-date { font-size: 20px; font-weight: 500; color: rgba(255,255,255,0.92); text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.desk-note { background-color: #fff3a8; color: #3a3320; border-radius: 6px 6px 16px 6px; padding: 12px; box-shadow: 0 8px 22px rgba(0,0,0,0.3); }
.desk-note textview, .desk-note text { background: transparent; color: #3a3320; font-size: 1.05em; }
.desk-handle { min-height: 22px; border-radius: 99px; padding: 0 8px; background-color: @accent_bg_color; color: @accent_fg_color; font-size: 0.82em; font-weight: 700; }
"#;

/// Libellés des éléments de la barre.
pub fn item_label(id: &str) -> &'static str {
    match id {
        "vitals" => "Ressources",
        "start" => "Démarrer",
        "search" => "Recherche",
        "taskview" => "Vue des tâches",
        "tasks" => "Applications",
        "tray" => "Zone de notification",
        "status" => "Réseau, son, réglages",
        "clock" => "Horloge",
        "notifications" => "Notifications",
        "showdesktop" => "Afficher le bureau",
        _ => "Élément",
    }
}

/// Élément de la barre enveloppé d'un « bouclier » : en mode édition, le bouclier (visible,
/// en pointillé) intercepte les clics et se glisse vers une autre zone.
pub fn frame(id: &str, child: &impl IsA<gtk::Widget>) -> (gtk::Overlay, gtk::Box) {
    let o = gtk::Overlay::new();
    o.set_child(Some(child));
    o.set_valign(gtk::Align::Center);
    let shield = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shield.add_css_class("edit-shield");
    shield.set_visible(false);
    shield.set_tooltip_text(Some(&format!("{} — glisser pour déplacer", item_label(id))));
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    let id2 = id.to_string();
    drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&format!("noos-bar-item:{id2}").to_value())));
    {
        let o2 = o.clone();
        drag.connect_drag_begin(move |src, _| src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&o2))), 20, 20));
    }
    shield.add_controller(drag);
    o.add_overlay(&shield);
    (o, shield)
}

/// Zone de la barre (0 gauche, 1 centre, 2 droite) qui accepte les éléments glissés.
pub fn zone_drop(zone: usize, b: &gtk::Box, shell: Weak<Shell>) {
    b.add_css_class("edit-zone");
    let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    {
        let b2 = b.clone();
        drop.connect_enter(move |_, _, _| {
            b2.add_css_class("drop-hover");
            gdk::DragAction::MOVE
        });
    }
    {
        let b2 = b.clone();
        drop.connect_leave(move |_| b2.remove_css_class("drop-hover"));
    }
    let b2 = b.clone();
    drop.connect_drop(move |_, value, x, _| {
        b2.remove_css_class("drop-hover");
        let Ok(s) = value.get::<String>() else { return false };
        let Some(id) = s.strip_prefix("noos-bar-item:").map(String::from) else { return false };
        // Position d'insertion : avant le premier élément dont le milieu est à droite du point.
        let mut index = 0;
        let mut c = b2.first_child();
        while let Some(w) = c {
            let mid = w.compute_bounds(&b2).map(|r| r.x() as f64 + r.width() as f64 / 2.0).unwrap_or(0.0);
            let is_self = w.widget_name() == format!("bar-{id}");
            if mid > x {
                break;
            }
            if !is_self {
                index += 1;
            }
            c = w.next_sibling();
        }
        let Some(shell) = shell.upgrade() else { return false };
        shell.update_config(|c| {
            let mut z = c.taskbar.zones();
            for list in z.iter_mut() {
                list.retain(|x| *x != id);
            }
            let list = &mut z[zone];
            list.insert(index.min(list.len()), id.clone());
            [c.taskbar.zone_left, c.taskbar.zone_center, c.taskbar.zone_right] = z;
        });
        let s2 = shell.clone();
        glib::idle_add_local_once(move || s2.relayout_taskbars());
        true
    });
    b.add_controller(drop);
}

// ───────────────────────────── Widgets du bureau ─────────────────────────────

/// Widget posé sur le fond d'écran d'un écran (dans un `gtk::Fixed`).
pub struct DeskWidget {
    pub frame: gtk::Box,
    handle: gtk::Box,
}

impl DeskWidget {
    pub fn set_editing(&self, on: bool) {
        self.handle.set_visible(on);
        if on {
            self.frame.add_css_class("editing");
        } else {
            self.frame.remove_css_class("editing");
        }
    }
}

/// Construit le widget n° `index` de la configuration, placé dans `fixed` (taille de l'écran
/// `size`). Glisser la poignée le déplace (position enregistrée en fraction de l'écran).
pub fn build_widget(shell: &Rc<Shell>, index: usize, w: &DesktopWidget, fixed: &gtk::Fixed, size: (i32, i32)) -> DeskWidget {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 6);
    frame.add_css_class("desk-widget");
    // Poignée (mode édition) : nom, déplacer, supprimer.
    let handle = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    handle.add_css_class("desk-handle");
    handle.set_halign(gtk::Align::Start);
    let title = gtk::Label::new(Some(match w.kind.as_str() {
        "clock" => "Horloge",
        "vitals" => "Ressources",
        "note" => "Note",
        "calendar" => "Calendrier",
        _ => "Widget",
    }));
    handle.append(&gtk::Image::from_icon_name("list-drag-handle-symbolic"));
    handle.append(&title);
    let del = gtk::Button::from_icon_name("window-close-symbolic");
    del.add_css_class("flat");
    del.add_css_class("circular");
    del.set_tooltip_text(Some("Retirer ce widget"));
    {
        let shell = Rc::downgrade(shell);
        del.connect_clicked(move |_| {
            if let Some(s) = shell.upgrade() {
                s.update_config(|c| {
                    if index < c.desktop_widgets.len() {
                        c.desktop_widgets.remove(index);
                    }
                });
                let s2 = s.clone();
                glib::idle_add_local_once(move || s2.rebuild_desktop_widgets());
            }
        });
    }
    handle.append(&del);
    handle.set_visible(false);
    frame.append(&handle);

    let content: gtk::Widget = match w.kind.as_str() {
        "clock" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let time = gtk::Label::new(None);
            time.add_css_class("desk-clock-time");
            time.add_css_class("numeric");
            time.set_xalign(0.0);
            let date = gtk::Label::new(None);
            date.add_css_class("desk-clock-date");
            date.set_xalign(0.0);
            b.append(&time);
            b.append(&date);
            let tick = move || {
                if let Ok(now) = glib::DateTime::now_local() {
                    time.set_text(&now.format("%H:%M").map(|s| s.to_string()).unwrap_or_default());
                    let d = now.format("%A %-d %B").map(|s| s.to_string()).unwrap_or_default();
                    let mut c = d.chars();
                    date.set_text(&c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default());
                }
            };
            tick();
            let bw = b.downgrade();
            glib::timeout_add_seconds_local(5, move || {
                if bw.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }
                tick();
                glib::ControlFlow::Continue
            });
            b.upcast()
        }
        "vitals" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
            b.add_css_class("card-bg");
            b.add_css_class("desk-widget");
            let t = gtk::Label::new(Some("Ressources"));
            t.add_css_class("heading");
            t.set_xalign(0.0);
            b.append(&t);
            let v = noos_widget_vitals::Vitals::new();
            v.widget().set_sensitive(true);
            b.append(v.widget());
            std::mem::forget(v);
            b.upcast()
        }
        "calendar" => {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.add_css_class("card-bg");
            b.add_css_class("desk-widget");
            b.append(&gtk::Calendar::new());
            b.upcast()
        }
        _ => {
            // Note : texte enregistré à chaque modification (après une courte pause).
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.add_css_class("desk-note");
            let tv = gtk::TextView::new();
            tv.set_wrap_mode(gtk::WrapMode::WordChar);
            tv.set_size_request(220, 160);
            tv.buffer().set_text(if w.text.is_empty() { "Nouvelle note…" } else { &w.text });
            let pending: Rc<Cell<u32>> = Rc::new(Cell::new(0));
            let shell = Rc::downgrade(shell);
            tv.buffer().connect_changed(move |buf| {
                let n = pending.get() + 1;
                pending.set(n);
                let (p, shell, text) = (pending.clone(), shell.clone(), buf.text(&buf.start_iter(), &buf.end_iter(), false).to_string());
                glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                    if p.get() != n {
                        return;
                    }
                    if let Some(s) = shell.upgrade() {
                        // Sans reconstruire les widgets (le curseur resterait en place).
                        let mut cfg = s.config();
                        if let Some(w) = cfg.desktop_widgets.get_mut(index) {
                            w.text = text;
                            let _ = cfg.save();
                        }
                    }
                });
            });
            b.append(&tv);
            b.upcast()
        }
    };
    frame.append(&content);
    let x = (w.x * size.0 as f64) as f64;
    let y = (w.y * size.1 as f64) as f64;
    fixed.put(&frame, x, y);

    // Déplacement par la poignée.
    let drag = gtk::GestureDrag::new();
    let start: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((x, y)));
    {
        let (start, fixed, frame) = (start.clone(), fixed.clone(), frame.clone());
        drag.connect_drag_begin(move |_, _, _| {
            let p = fixed.child_position(&frame);
            *start.borrow_mut() = p;
        });
    }
    {
        let (start, fixed, frame) = (start.clone(), fixed.clone(), frame.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            let (sx, sy) = *start.borrow();
            let nx = (sx + dx).clamp(0.0, (size.0 - 40) as f64);
            let ny = (sy + dy).clamp(0.0, (size.1 - 40) as f64);
            fixed.move_(&frame, nx, ny);
        });
    }
    {
        let (fixed, frame, shell) = (fixed.clone(), frame.clone(), Rc::downgrade(shell));
        drag.connect_drag_end(move |_, _, _| {
            let (px, py) = fixed.child_position(&frame);
            if let Some(s) = shell.upgrade() {
                let mut cfg = s.config();
                if let Some(w) = cfg.desktop_widgets.get_mut(index) {
                    w.x = px / size.0.max(1) as f64;
                    w.y = py / size.1.max(1) as f64;
                    let _ = cfg.save();
                }
            }
        });
    }
    handle.add_controller(drag);
    DeskWidget { frame, handle }
}

// ───────────────────────────── Barre d'outils ─────────────────────────────

pub struct EditMode {
    shell: Weak<Shell>,
    toolbar: gtk::Window,
    pub active: Cell<bool>,
    pos_btn: gtk::Button,
}

impl EditMode {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let toolbar = gtk::Window::builder().application(&shell.app).decorated(false).build();
        toolbar.add_css_class("noos-layer");
        toolbar.init_layer_shell();
        toolbar.set_namespace(Some("noos-edit-toolbar"));
        toolbar.set_layer(Layer::Overlay);
        toolbar.set_keyboard_mode(KeyboardMode::OnDemand);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("edit-toolbar");
        let icon = gtk::Image::from_icon_name("document-edit-symbolic");
        row.append(&icon);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let t = gtk::Label::new(Some("Mode édition"));
        t.add_css_class("edit-title");
        t.set_xalign(0.0);
        let s = gtk::Label::new(Some("Glissez les éléments de la barre d'une zone à l'autre ; déplacez les widgets par leur poignée."));
        s.add_css_class("dim-label");
        s.add_css_class("caption");
        s.set_xalign(0.0);
        v.append(&t);
        v.append(&s);
        row.append(&v);
        // Ajouter un widget.
        let add = gtk::MenuButton::new();
        let ac = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        ac.append(&gtk::Image::from_icon_name("list-add-symbolic"));
        ac.append(&gtk::Label::new(Some("Ajouter un widget")));
        add.set_child(Some(&ac));
        add.set_always_show_arrow(true);
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu.set_margin_top(6);
        menu.set_margin_bottom(6);
        let pop = gtk::Popover::new();
        pop.set_child(Some(&menu));
        add.set_popover(Some(&pop));
        for (kind, label, icon) in [
            ("clock", "Horloge", "preferences-system-time-symbolic"),
            ("vitals", "Ressources", "utilities-system-monitor-symbolic"),
            ("note", "Note", "accessories-text-editor-symbolic"),
            ("calendar", "Calendrier", "x-office-calendar-symbolic"),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            c.append(&gtk::Image::from_icon_name(icon));
            c.append(&gtk::Label::new(Some(label)));
            b.set_child(Some(&c));
            let (shell, pop) = (Rc::downgrade(shell), pop.clone());
            b.connect_clicked(move |_| {
                pop.popdown();
                let Some(s) = shell.upgrade() else { return };
                // Posé en haut à gauche de l'écran principal, décalé si d'autres y sont déjà.
                let n = s.config().desktop_widgets.len() as f64;
                s.update_config(|c| {
                    c.desktop_widgets.push(DesktopWidget { kind: kind.into(), output: String::new(), x: 0.06 + 0.04 * (n % 5.0), y: 0.08 + 0.05 * (n % 5.0), text: String::new() })
                });
                let s2 = s.clone();
                glib::idle_add_local_once(move || s2.rebuild_desktop_widgets());
            });
            menu.append(&b);
        }
        row.append(&add);
        let pos_btn = gtk::Button::new();
        {
            let shell = Rc::downgrade(shell);
            pos_btn.connect_clicked(move |_| {
                let Some(s) = shell.upgrade() else { return };
                s.update_config(|c| c.taskbar.position = if c.taskbar.position == "top" { "bottom".into() } else { "top".into() });
                let s2 = s.clone();
                glib::idle_add_local_once(move || {
                    s2.relayout_taskbars();
                    if let Some(e) = s2.editmode.borrow().as_ref() {
                        e.sync_labels();
                    }
                });
            });
        }
        row.append(&pos_btn);
        let reset = gtk::Button::with_label("Réinitialiser la barre");
        {
            let shell = Rc::downgrade(shell);
            reset.connect_clicked(move |_| {
                let Some(s) = shell.upgrade() else { return };
                s.update_config(|c| {
                    let d = noos_core::config::Taskbar::default();
                    (c.taskbar.zone_left, c.taskbar.zone_center, c.taskbar.zone_right) = (d.zone_left, d.zone_center, d.zone_right);
                });
                let s2 = s.clone();
                glib::idle_add_local_once(move || s2.relayout_taskbars());
            });
        }
        row.append(&reset);
        let done = gtk::Button::with_label("Terminé");
        done.add_css_class("suggested-action");
        row.append(&done);
        toolbar.set_child(Some(&row));
        let e = Rc::new(Self { shell: Rc::downgrade(shell), toolbar, active: Cell::new(false), pos_btn });
        {
            let w = Rc::downgrade(&e);
            done.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.set_active(false);
                }
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(&e);
            keys.connect_key_pressed(move |_, k, _, _| {
                if k == gdk::Key::Escape {
                    if let Some(e) = w.upgrade() {
                        e.set_active(false);
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            e.toolbar.add_controller(keys);
        }
        if std::env::var_os("NOOS_EDIT_DEMO").is_some() {
            let w = Rc::downgrade(&e);
            glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                if let Some(e) = w.upgrade() {
                    e.set_active(true);
                }
            });
        }
        e
    }

    fn sync_labels(&self) {
        let top = self.shell.upgrade().is_some_and(|s| s.config().taskbar.position == "top");
        self.pos_btn.set_label(if top { "Barre en bas" } else { "Barre en haut" });
        // La barre d'outils se place du côté opposé à la barre des tâches.
        self.toolbar.set_anchor(Edge::Top, !top);
        self.toolbar.set_anchor(Edge::Bottom, top);
        self.toolbar.set_margin(if top { Edge::Bottom } else { Edge::Top }, 18);
    }

    pub fn toggle(&self) {
        self.set_active(!self.active.get());
    }

    pub fn set_active(&self, on: bool) {
        self.active.set(on);
        if let Some(s) = self.shell.upgrade() {
            s.close_flyouts();
            s.set_editing(on);
        }
        if on {
            self.sync_labels();
            self.toolbar.present();
        } else {
            self.toolbar.set_visible(false);
        }
    }
}

/// Place un panneau (menu Démarrer, réglages rapides…) contre la barre des tâches, en bas ou
/// en haut selon la configuration.
pub fn anchor_to_bar(window: &gtk::Window, cfg: &NoosConfig) {
    let top = cfg.taskbar.position == "top";
    window.set_anchor(Edge::Bottom, !top);
    window.set_anchor(Edge::Top, top);
    window.set_margin(Edge::Bottom, if top { 0 } else { 12 });
    window.set_margin(Edge::Top, if top { 12 } else { 0 });
}

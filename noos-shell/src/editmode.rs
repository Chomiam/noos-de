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
.edit-shield .edit-remove { min-width: 18px; min-height: 18px; padding: 0; margin: 1px 1px 0 0; border-radius: 99px;
  background-color: #e5484d; color: white; -gtk-icon-size: 10px; }
.edit-toolbar .edge-btn { min-width: 30px; padding: 4px; }
.edit-toolbar .edge-btn:checked, .edit-toolbar .accent-toggle:checked { background-color: @accent_bg_color; color: @accent_fg_color; }
.notif-placeholder .corner-btn { min-width: 34px; font-size: 1.2em; }
.notif-placeholder { padding: 18px 22px; border-radius: 14px; border: 2px dashed @accent_bg_color; background-color: alpha(@window_bg_color, 0.9); color: @window_fg_color; }
.noos-taskbar .bar-desktops button.desk-pill { border-radius: 99px; padding: 0 12px; min-height: 22px; min-width: 22px; font-size: 0.9em; font-weight: 600; }
.noos-taskbar.vertical .bar-desktops button.desk-pill { padding: 0; }
.noos-taskbar .bar-desktops button.desk-pill.current { background-color: @accent_bg_color; color: @accent_fg_color; }
.noos-taskbar.noos-topbar button { min-height: 0; padding-top: 1px; padding-bottom: 1px; }
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
        "desktops" => "Bureaux virtuels",
        _ => "Élément",
    }
}

/// Élément de la barre enveloppé d'un « bouclier » : en mode édition, le bouclier (visible,
/// en pointillé) intercepte les clics et se glisse vers une autre zone.
pub fn frame(id: &str, child: &impl IsA<gtk::Widget>, shell: Weak<Shell>) -> (gtk::Overlay, gtk::Box) {
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
    // × : retirer l'élément de la barre (il revient par « Ajouter à la barre »).
    let x = gtk::Button::from_icon_name("window-close-symbolic");
    x.add_css_class("edit-remove");
    x.set_halign(gtk::Align::End);
    x.set_valign(gtk::Align::Start);
    x.set_hexpand(true);
    x.set_tooltip_text(Some(&format!("Retirer « {} » de la barre", item_label(id))));
    let id3 = id.to_string();
    x.connect_clicked(move |_| {
        let Some(s) = shell.upgrade() else { return };
        let id = id3.clone();
        s.update_config(move |c| {
            if !c.taskbar.hidden.contains(&id) {
                c.taskbar.hidden.push(id.clone());
            }
            match id.as_str() {
                "search" => c.taskbar.show_search = false,
                "vitals" => c.taskbar.show_vitals = false,
                "taskview" => c.desktops.taskbar_button = false,
                _ => {}
            }
        });
        let s2 = s.clone();
        glib::idle_add_local_once(move || s2.relayout_taskbars());
    });
    shield.append(&x);
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
            let mut z = c.bar_zones();
            for list in z.iter_mut() {
                list.retain(|x| *x != id);
            }
            let list = &mut z[zone];
            list.insert(index.min(list.len()), id.clone());
            c.set_bar_zones(z);
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
    /// Geste de déplacement, posé sur le `gtk::Fixed` (à retirer avec le widget).
    pub drag: gtk::GestureDrag,
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
    // Clic droit sur un widget (hors mode édition aussi) : le retirer, ou modifier le bureau.
    {
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let pop = gtk::Popover::new();
        pop.set_has_arrow(false);
        pop.set_child(Some(&menu));
        pop.set_parent(&frame);
        for (label, icon, remove) in [("Retirer ce widget", "user-trash-symbolic", true), ("Modifier le bureau", "document-edit-symbolic", false)] {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            c.append(&gtk::Image::from_icon_name(icon));
            c.append(&gtk::Label::new(Some(label)));
            b.set_child(Some(&c));
            let (shell, pop2) = (Rc::downgrade(shell), pop.clone());
            b.connect_clicked(move |_| {
                pop2.popdown();
                let Some(s) = shell.upgrade() else { return };
                if remove {
                    s.update_config(|c| {
                        if index < c.desktop_widgets.len() {
                            c.desktop_widgets.remove(index);
                        }
                    });
                    let s2 = s.clone();
                    glib::idle_add_local_once(move || s2.rebuild_desktop_widgets());
                } else if let Some(e) = s.editmode.borrow().as_ref() {
                    e.set_active(true);
                }
            });
            menu.append(&b);
        }
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        frame.add_controller(click);
    }

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

    // Déplacement par la poignée. Le geste est posé sur le `gtk::Fixed`, qui ne bouge pas :
    // posé sur la poignée, ses coordonnées suivraient le widget déplacé, d'où des allers-retours
    // à chaque image (clignotement).
    let drag = gtk::GestureDrag::new();
    let start: Rc<RefCell<(f64, f64)>> = Rc::new(RefCell::new((x, y)));
    let mine = Rc::new(Cell::new(false));
    {
        let (start, fixed, frame, handle, mine) = (start.clone(), fixed.clone(), frame.clone(), handle.clone(), mine.clone());
        drag.connect_drag_begin(move |g, px, py| {
            mine.set(false);
            let picked = fixed.pick(px, py, gtk::PickFlags::DEFAULT);
            let on_handle = handle.is_visible() && picked.as_ref().is_some_and(|w| *w == handle || w.is_ancestor(&handle));
            // Le × de la poignée garde son clic (sinon ce geste le lui prend : widget impossible
            // à retirer).
            let on_button = picked.as_ref().is_some_and(|w| w.is::<gtk::Button>() || w.ancestor(gtk::Button::static_type()).is_some());
            if !on_handle || on_button {
                g.set_state(gtk::EventSequenceState::Denied);
                return;
            }
            g.set_state(gtk::EventSequenceState::Claimed);
            mine.set(true);
            *start.borrow_mut() = fixed.child_position(&frame);
        });
    }
    {
        let (start, fixed, frame, mine) = (start.clone(), fixed.clone(), frame.clone(), mine.clone());
        drag.connect_drag_update(move |_, dx, dy| {
            if !mine.get() {
                return;
            }
            let (sx, sy) = *start.borrow();
            let nx = (sx + dx).clamp(0.0, (size.0 - 40) as f64);
            let ny = (sy + dy).clamp(0.0, (size.1 - 40) as f64);
            fixed.move_(&frame, nx, ny);
        });
    }
    {
        let (fixed, frame, shell) = (fixed.clone(), frame.clone(), Rc::downgrade(shell));
        drag.connect_drag_end(move |_, _, _| {
            if !mine.replace(false) {
                return;
            }
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
    fixed.add_controller(drag.clone());
    DeskWidget { frame, handle, drag }
}

// ───────────────────────────── Barre d'outils ─────────────────────────────

pub struct EditMode {
    shell: Weak<Shell>,
    toolbar: gtk::Window,
    pub active: Cell<bool>,
    edge_btns: Vec<(&'static str, gtk::ToggleButton)>,
    topbar_btn: gtk::ToggleButton,
    /// Emplacement des notifications (cadre glissable vers un coin).
    notif: gtk::Window,
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
        v.append(&t);
        v.set_tooltip_text(Some("Glissez les éléments de la barre d'une zone à l'autre (× pour en retirer un) ; déplacez les widgets par leur poignée ; le cadre des notifications se glisse vers un coin."));
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
        // Bord du dock : bas, haut, gauche, droite.
        let edges = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        edges.add_css_class("linked");
        let edge_label = gtk::Label::new(Some("Dock :"));
        edge_label.add_css_class("dim-label");
        row.append(&edge_label);
        let mut first: Option<gtk::ToggleButton> = None;
        let mut edge_btns = vec![];
        for (edge, icon, tip) in [
            ("bottom", "go-bottom-symbolic", "Dock en bas"),
            ("top", "go-top-symbolic", "Dock en haut"),
            ("left", "go-first-symbolic", "Dock à gauche"),
            ("right", "go-last-symbolic", "Dock à droite"),
        ] {
            let b = gtk::ToggleButton::new();
            b.set_icon_name(icon);
            b.add_css_class("edge-btn");
            b.set_tooltip_text(Some(tip));
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            let shell = Rc::downgrade(shell);
            b.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                let Some(s) = shell.upgrade() else { return };
                if s.config().taskbar.position == edge {
                    return;
                }
                // Le shell reconstruit les barres de lui-même (apply_style).
                s.update_config(|c| c.taskbar.position = edge.into());
                let s2 = s.clone();
                glib::idle_add_local_once(move || {
                    if let Some(e) = s2.editmode.borrow().as_ref() {
                        e.sync_labels();
                    }
                });
            });
            edges.append(&b);
            edge_btns.push((edge, b));
        }
        row.append(&edges);
        // Barre fine.
        let topbar = gtk::ToggleButton::with_label("Barre fine");
        topbar.add_css_class("accent-toggle");
        topbar.set_tooltip_text(Some("Barre supplémentaire au bord opposé : bureaux virtuels, éléments déplacés…"));
        {
            let shell = Rc::downgrade(shell);
            topbar.connect_toggled(move |b| {
                let Some(s) = shell.upgrade() else { return };
                let on = b.is_active();
                if s.config().topbar.enabled == on {
                    return;
                }
                s.update_config(|c| c.topbar.enabled = on);
            });
        }
        row.append(&topbar);
        // Éléments retirés : les remettre.
        let restore = gtk::MenuButton::new();
        let rc = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        rc.append(&gtk::Image::from_icon_name("view-list-bullet-symbolic"));
        rc.append(&gtk::Label::new(Some("Ajouter à la barre")));
        restore.set_child(Some(&rc));
        let rmenu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let rpop = gtk::Popover::new();
        rpop.set_child(Some(&rmenu));
        restore.set_popover(Some(&rpop));
        {
            let (shell, rmenu, rpop2) = (Rc::downgrade(shell), rmenu.clone(), rpop.clone());
            rpop.connect_show(move |_| {
                while let Some(c) = rmenu.first_child() {
                    rmenu.remove(&c);
                }
                let Some(s) = shell.upgrade() else { return };
                let hidden = s.config().taskbar.hidden;
                if hidden.is_empty() {
                    let l = gtk::Label::new(Some("Aucun élément retiré"));
                    l.add_css_class("dim-label");
                    l.set_margin_top(6);
                    l.set_margin_bottom(6);
                    rmenu.append(&l);
                }
                for id in hidden {
                    let b = gtk::Button::with_label(item_label(&id));
                    b.add_css_class("flat");
                    let (shell, pop) = (Rc::downgrade(&s), rpop2.clone());
                    b.connect_clicked(move |_| {
                        pop.popdown();
                        let Some(s) = shell.upgrade() else { return };
                        let id = id.clone();
                        s.update_config(move |c| {
                            c.taskbar.hidden.retain(|x| *x != id);
                            match id.as_str() {
                                "search" => c.taskbar.show_search = true,
                                "vitals" => c.taskbar.show_vitals = true,
                                "taskview" => c.desktops.taskbar_button = true,
                                _ => {}
                            }
                        });
                        let s2 = s.clone();
                        glib::idle_add_local_once(move || s2.relayout_taskbars());
                    });
                    rmenu.append(&b);
                }
            });
        }
        row.append(&restore);
        let reset = gtk::Button::from_icon_name("edit-undo-symbolic");
        reset.set_tooltip_text(Some("Réinitialiser la barre"));
        {
            let shell = Rc::downgrade(shell);
            reset.connect_clicked(move |_| {
                let Some(s) = shell.upgrade() else { return };
                s.update_config(|c| {
                    let d = noos_core::config::Taskbar::default();
                    (c.taskbar.zone_left, c.taskbar.zone_center, c.taskbar.zone_right) = (d.zone_left, d.zone_center, d.zone_right);
                    c.taskbar.hidden.clear();
                    let t = noos_core::config::TopBar::default();
                    (c.topbar.zone_left, c.topbar.zone_center, c.topbar.zone_right) = (t.zone_left, t.zone_center, t.zone_right);
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
        let notif = notif_placeholder(shell);
        let e = Rc::new(Self { shell: Rc::downgrade(shell), toolbar, active: Cell::new(false), edge_btns, topbar_btn: topbar, notif });
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
        let Some(s) = self.shell.upgrade() else { return };
        let cfg = s.config();
        for (edge, b) in &self.edge_btns {
            if *edge == cfg.taskbar.position.as_str() && !b.is_active() {
                b.set_active(true);
            }
        }
        self.topbar_btn.set_active(cfg.topbar.enabled);
        // Barre d'outils : en haut, sauf si le dock ou les notifications y sont.
        let top = cfg.taskbar.position == "top" || cfg.notifications.position.starts_with("top");
        self.toolbar.set_anchor(Edge::Top, !top);
        self.toolbar.set_anchor(Edge::Bottom, top);
        self.toolbar.set_margin(if top { Edge::Bottom } else { Edge::Top }, 56);
        place_notif(&self.notif, &cfg.notifications.position);
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
            self.notif.present();
        } else {
            self.toolbar.set_visible(false);
            self.notif.set_visible(false);
        }
    }
}

/// Place un panneau (menu Démarrer, réglages rapides…) contre le dock : en bas, en haut ou
/// sur un côté selon la configuration. Les panneaux ancrés à droite le restent (sauf dock à
/// gauche) ; le menu Démarrer suit le côté du dock.
pub fn anchor_to_bar(window: &gtk::Window, cfg: &NoosConfig) {
    let edge = cfg.taskbar.position.as_str();
    // Menu Démarrer : centré (ou du côté du dock) ; les autres panneaux : côté droit.
    let right = window.namespace().as_deref() != Some("noos-startmenu");
    let (top, bottom, left, rgt) = match edge {
        "top" => (true, false, false, right),
        "left" => (false, true, true, false),
        "right" => (false, true, false, true),
        _ => (false, true, false, right),
    };
    window.set_anchor(Edge::Top, top);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_anchor(Edge::Left, left);
    window.set_anchor(Edge::Right, rgt);
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        let anchored = window.is_anchor(e);
        window.set_margin(e, if anchored { 12 } else { 0 });
    }
}

/// Coin des bannières de notification.
pub fn place_notif(window: &gtk::Window, corner: &str) {
    let top = corner.starts_with("top");
    let left = corner.ends_with("left");
    window.set_anchor(Edge::Top, top);
    window.set_anchor(Edge::Bottom, !top);
    window.set_anchor(Edge::Left, left);
    window.set_anchor(Edge::Right, !left);
    for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_margin(e, 12);
    }
}

/// Cadre « Les notifications apparaîtront ici » (mode édition) : le glisser vers un coin
/// (ou cliquer une flèche) choisit l'emplacement des bannières.
fn notif_placeholder(shell: &Rc<Shell>) -> gtk::Window {
    let w = gtk::Window::builder().application(&shell.app).decorated(false).build();
    w.add_css_class("noos-layer");
    w.init_layer_shell();
    w.set_namespace(Some("noos-edit-notifications"));
    w.set_layer(Layer::Overlay);
    w.set_keyboard_mode(KeyboardMode::None);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("notif-placeholder");
    let t = gtk::Label::new(Some("Les notifications apparaîtront ici"));
    t.add_css_class("heading");
    b.append(&t);
    let sub = gtk::Label::new(Some("Glissez ce cadre vers un coin, ou :"));
    sub.add_css_class("dim-label");
    b.append(&sub);
    let corners = gtk::Grid::new();
    corners.set_halign(gtk::Align::Center);
    corners.set_column_spacing(4);
    corners.set_row_spacing(4);
    let set_corner = {
        let (shell, w) = (Rc::downgrade(shell), w.clone());
        Rc::new(move |corner: &'static str| {
            let Some(s) = shell.upgrade() else { return };
            s.update_config(|c| c.notifications.position = corner.into());
            place_notif(&w, corner);
            if let Some(n) = s.notify.borrow().as_ref() {
                n.apply_position(corner);
            }
            let s2 = s.clone();
            glib::idle_add_local_once(move || {
                if let Some(e) = s2.editmode.borrow().as_ref() {
                    e.sync_labels();
                }
            });
        })
    };
    for (corner, arrow, col, row) in [("top-left", "↖", 0, 0), ("top-right", "↗", 1, 0), ("bottom-left", "↙", 0, 1), ("bottom-right", "↘", 1, 1)] {
        let btn = gtk::Button::with_label(arrow);
        btn.add_css_class("corner-btn");
        btn.set_tooltip_text(Some(match corner {
            "top-left" => "En haut à gauche",
            "top-right" => "En haut à droite",
            "bottom-left" => "En bas à gauche",
            _ => "En bas à droite",
        }));
        let f = set_corner.clone();
        btn.connect_clicked(move |_| f(corner));
        corners.attach(&btn, col, row, 1, 1);
    }
    b.append(&corners);
    // Glisser : le coin le plus proche du point d'arrivée (le cadre ne bouge pas pendant le
    // geste, les décalages restent donc stables).
    let drag = gtk::GestureDrag::new();
    {
        let (w2, f, shell) = (w.clone(), set_corner.clone(), Rc::downgrade(shell));
        drag.connect_drag_end(move |_, dx, dy| {
            if dx.abs() < 20.0 && dy.abs() < 20.0 {
                return;
            }
            let Some(s) = shell.upgrade() else { return };
            let cur = s.config().notifications.position;
            let mon = w2.surface().and_then(|sf| gdk::Display::default()?.monitor_at_surface(&sf));
            let (mw, mh) = mon.map(|m| (m.geometry().width() as f64, m.geometry().height() as f64)).unwrap_or((1920.0, 1080.0));
            let (ww, wh) = (w2.width() as f64, w2.height() as f64);
            // Centre actuel du cadre, puis point d'arrivée.
            let cx = if cur.ends_with("left") { 12.0 + ww / 2.0 } else { mw - 12.0 - ww / 2.0 } + dx;
            let cy = if cur.starts_with("top") { 12.0 + wh / 2.0 } else { mh - 12.0 - wh / 2.0 } + dy;
            let corner = match (cy < mh / 2.0, cx < mw / 2.0) {
                (true, true) => "top-left",
                (true, false) => "top-right",
                (false, true) => "bottom-left",
                (false, false) => "bottom-right",
            };
            f(corner);
        });
    }
    b.add_controller(drag);
    w.set_child(Some(&b));
    place_notif(&w, &shell.config().notifications.position);
    w
}

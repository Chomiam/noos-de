//! Mode édition du bureau : la barre des tâches et la barre d'état se réorganisent par
//! glisser-déposer, des widgets se posent librement sur le bureau (`desk.rs`, galerie dans
//! `gallery.rs`), des mises en page toutes faites s'appliquent en un clic (`layouts.rs`).
//! Tout est annulable (Ctrl+Z / Ctrl+Y, `Shell::edit_layout`).
//!
//! Entrées : clic droit sur le bureau ou la barre, Paramètres › Barre des tâches, IPC
//! `toggle-edit-mode`. Échap, Entrée ou « Terminé » pour sortir.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::{NoosConfig, ipc::Request, layout::Layout};

use crate::{Shell, apps, gallery, layouts};

pub const CSS: &str = r#"
.edit-shield { border-radius: 8px; }
.editing .edit-shield { border: 1.5px dashed alpha(@accent_bg_color, 0.85); background-color: alpha(@accent_bg_color, 0.10); }
.editing .edit-shield:hover { background-color: alpha(@accent_bg_color, 0.28); }
.edit-shield .edit-remove { min-width: 16px; min-height: 16px; padding: 0; margin: 0; border-radius: 99px;
  background-color: #e5484d; color: white; -gtk-icon-size: 9px; opacity: 0; transition: opacity 120ms; }
.edit-shield:hover .edit-remove { opacity: 1; }
.edit-insert { min-width: 3px; min-height: 3px; border-radius: 99px; background-color: @accent_bg_color; box-shadow: 0 0 8px @accent_bg_color; }
.notif-placeholder .corner-btn { min-width: 34px; font-size: 1.2em; }
.notif-placeholder { padding: 18px 22px; border-radius: 14px; border: 2px dashed @accent_bg_color; color: @window_fg_color; }
.noos-taskbar .bar-desktops button.desk-pill { border-radius: 99px; padding: 0 12px; min-height: 22px; min-width: 22px; font-size: 0.9em; font-weight: 600; }
.noos-taskbar.vertical .bar-desktops button.desk-pill { padding: 0; }
.noos-taskbar .bar-desktops button.desk-pill.current { background-color: @accent_bg_color; color: @accent_fg_color; }
.noos-taskbar.noos-topbar button { min-height: 0; padding-top: 1px; padding-bottom: 1px; }
.editing .edit-zone { border-radius: 10px; min-width: 56px; min-height: 30px; background-color: alpha(@window_fg_color, 0.04);
  box-shadow: inset 0 0 0 1.5px alpha(@window_fg_color, 0.18); }
.editing .edit-zone.drop-hover { box-shadow: inset 0 0 0 2px @accent_bg_color; background-color: alpha(@accent_bg_color, 0.12); }
.edit-toolbar { padding: 8px 10px 8px 16px; border-radius: 18px; color: @window_fg_color;
  border: 1px solid alpha(@window_fg_color, 0.12); box-shadow: 0 12px 36px rgba(0,0,0,0.42); }
.edit-toolbar .edit-title { font-weight: 800; }
.edit-toolbar .edit-sub { font-size: 0.78em; opacity: 0.65; }
.edit-toolbar button.tool { padding: 6px 12px; border-radius: 10px; }
.edit-toolbar button.tool:checked { background-color: alpha(@accent_bg_color, 0.2); color: @accent_bg_color; }
.edit-toolbar button.icon-tool { min-width: 34px; min-height: 34px; padding: 0; border-radius: 10px; font-weight: 800; font-size: 1.1em; }
.edit-toolbar .edit-sep { min-width: 1px; margin: 6px 4px; background-color: alpha(@window_fg_color, 0.14); }
.edit-toolbar button.done { padding: 6px 18px; border-radius: 10px; font-weight: 700; }
.edit-hint { padding: 5px 8px 5px 14px; border-radius: 99px; color: @window_fg_color; font-size: 0.88em;
  box-shadow: 0 6px 18px rgba(0,0,0,0.3); transition: background-color 200ms; }
.edit-hint.toast { background-color: @accent_bg_color; color: @accent_fg_color; }
.edit-hint button { padding: 0 10px; min-height: 24px; border-radius: 99px; font-weight: 700; }
.edit-hint.toast button { background-color: alpha(black, 0.18); color: @accent_fg_color; }
.edge-card { padding: 6px; border-radius: 12px; border: 2px solid transparent; }
.edge-card:checked { border-color: @accent_bg_color; background-color: alpha(@accent_bg_color, 0.12); }
.edit-pop .item-row { padding: 2px 0; }
.edit-tour { padding: 6px 4px; }
.edit-tour .tour-step { font-size: 0.75em; font-weight: 800; color: @accent_color; letter-spacing: 0.06em; }
.edit-tour .tour-title { font-weight: 800; font-size: 1.15em; }
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
        "power" => "Énergie (batterie)",
        "clock" => "Horloge",
        "notifications" => "Notifications",
        "showdesktop" => "Afficher le bureau",
        "desktops" => "Bureaux virtuels",
        _ => "Élément",
    }
}

fn item_icon(id: &str) -> &'static str {
    match id {
        "vitals" => "power-profile-performance-symbolic",
        "start" => "start-here-symbolic",
        "search" => "system-search-symbolic",
        "taskview" => "view-grid-symbolic",
        "tasks" => "view-app-grid-symbolic",
        "tray" => "pan-up-symbolic",
        "status" => "network-wireless-symbolic",
        "power" => "battery-good-symbolic",
        "clock" => "preferences-system-time-symbolic",
        "notifications" => "preferences-system-notifications-symbolic",
        "showdesktop" => "user-desktop-symbolic",
        "desktops" => "view-paged-symbolic",
        _ => "view-grid-symbolic",
    }
}

const ZONE_NAMES: [&str; 6] = [
    "Barre des tâches · gauche",
    "Barre des tâches · centre",
    "Barre des tâches · droite",
    "Barre d'état · gauche",
    "Barre d'état · centre",
    "Barre d'état · droite",
];

/// Retire un élément de la barre (il revient par « Barres › Éléments retirés »).
fn remove_item(shell: &Rc<Shell>, id: &str) {
    let id = id.to_string();
    shell.edit_layout(&format!("« {} » retiré de la barre", item_label(&id)), move |c| {
        if !c.taskbar.hidden.contains(&id) {
            c.taskbar.hidden.push(id.clone());
        }
        match id.as_str() {
            "search" => c.taskbar.show_search = false,
            "vitals" => c.taskbar.show_vitals = false,
            "power" => c.taskbar.show_power = false,
            "taskview" => c.desktops.taskbar_button = false,
            _ => {}
        }
    });
}

fn restore_item(shell: &Rc<Shell>, id: &str) {
    let id = id.to_string();
    shell.edit_layout(&format!("« {} » remis dans la barre", item_label(&id)), move |c| {
        c.taskbar.hidden.retain(|x| *x != id);
        match id.as_str() {
            "search" => c.taskbar.show_search = true,
            "vitals" => c.taskbar.show_vitals = true,
            "power" => c.taskbar.show_power = true,
            "taskview" => c.desktops.taskbar_button = true,
            _ => {}
        }
    });
}

/// Déplace un élément dans la zone `zone` (0–2 barre des tâches, 3–5 barre d'état), à la
/// position `index` (à la fin sinon).
fn move_item(shell: &Rc<Shell>, id: &str, zone: usize, index: Option<usize>) {
    let id = id.to_string();
    let label = format!("« {} » déplacé", item_label(&id));
    shell.edit_layout(&label, move |c| {
        // Vers la barre d'état : on l'affiche.
        if zone >= 3 {
            c.topbar.enabled = true;
        }
        let mut z = c.bar_zones();
        for list in z.iter_mut() {
            list.retain(|x| *x != id);
        }
        let list = &mut z[zone];
        let at = index.unwrap_or(list.len()).min(list.len());
        list.insert(at, id.clone());
        c.set_bar_zones(z);
    });
}

/// Élément de la barre enveloppé d'un « bouclier » : en mode édition, le bouclier (visible,
/// en pointillé) intercepte les clics et se glisse vers une autre zone ; « × » au survol ;
/// clic droit : « Déplacer vers… », « Retirer ».
pub fn frame(id: &str, child: &impl IsA<gtk::Widget>, shell: Weak<Shell>) -> (gtk::Overlay, gtk::Box) {
    let o = gtk::Overlay::new();
    o.set_child(Some(child));
    o.set_valign(gtk::Align::Center);
    let shield = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    shield.add_css_class("edit-shield");
    shield.set_visible(false);
    shield.set_tooltip_text(Some(&format!("{} — glisser vers une autre zone, clic droit pour plus d'options", item_label(id))));
    let drag = gtk::DragSource::new();
    drag.set_actions(gdk::DragAction::MOVE);
    let id2 = id.to_string();
    drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&format!("noos-bar-item:{id2}").to_value())));
    {
        let o2 = o.clone();
        drag.connect_drag_begin(move |src, _| {
            src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&o2))), 20, 20);
            o2.set_opacity(0.35);
        });
    }
    {
        let o2 = o.clone();
        drag.connect_drag_end(move |_, _, _| o2.set_opacity(1.0));
    }
    shield.add_controller(drag);
    let x = gtk::Button::from_icon_name("window-close-symbolic");
    x.add_css_class("edit-remove");
    x.set_halign(gtk::Align::End);
    x.set_valign(gtk::Align::Start);
    x.set_hexpand(true);
    x.set_tooltip_text(Some(&format!("Retirer « {} » de la barre", item_label(id))));
    {
        let (id, shell) = (id.to_string(), shell.clone());
        x.connect_clicked(move |_| {
            if let Some(s) = shell.upgrade() {
                remove_item(&s, &id);
            }
        });
    }
    shield.append(&x);
    // Clic droit : déplacer sans glisser.
    {
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let pop = gtk::Popover::new();
        pop.set_has_arrow(false);
        pop.set_child(Some(&menu));
        pop.set_parent(&shield);
        {
            let (menu, id, shell, pop2) = (menu.clone(), id.to_string(), shell.clone(), pop.clone());
            pop.connect_show(move |_| {
                while let Some(c) = menu.first_child() {
                    menu.remove(&c);
                }
                let Some(s) = shell.upgrade() else { return };
                let cfg = s.config();
                let z = cfg.bar_zones();
                let here = z.iter().position(|l| l.contains(&id));
                let t = gtk::Label::new(Some(item_label(&id)));
                t.add_css_class("heading");
                t.set_xalign(0.0);
                t.set_margin_start(8);
                t.set_margin_bottom(4);
                menu.append(&t);
                for (zone, name) in ZONE_NAMES.iter().enumerate() {
                    if Some(zone) == here {
                        continue;
                    }
                    let b = gtk::Button::new();
                    b.add_css_class("flat");
                    let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    c.append(&gtk::Image::from_icon_name("go-next-symbolic"));
                    c.append(&gtk::Label::new(Some(&format!("Vers {}", name.to_lowercase().replacen("barre", "la barre", 1)))));
                    b.set_child(Some(&c));
                    let (s, id, pop) = (s.clone(), id.clone(), pop2.clone());
                    b.connect_clicked(move |_| {
                        pop.popdown();
                        move_item(&s, &id, zone, None);
                    });
                    menu.append(&b);
                }
                menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
                let b = gtk::Button::new();
                b.add_css_class("flat");
                let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                c.append(&gtk::Image::from_icon_name("user-trash-symbolic"));
                c.append(&gtk::Label::new(Some("Retirer de la barre")));
                b.set_child(Some(&c));
                let (s, id, pop) = (s.clone(), id.clone(), pop2.clone());
                b.connect_clicked(move |_| {
                    pop.popdown();
                    remove_item(&s, &id);
                });
                menu.append(&b);
            });
        }
        let click = gtk::GestureClick::new();
        click.set_button(3);
        click.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            pop.popup();
        });
        shield.add_controller(click);
    }
    o.add_overlay(&shield);
    (o, shield)
}

/// Zone de la barre (0 gauche, 1 centre, 2 droite ; +3 barre d'état) qui accepte les éléments
/// glissés ; un trait d'accent montre où l'élément va s'insérer.
pub fn zone_drop(zone: usize, b: &gtk::Box, shell: Weak<Shell>) {
    b.add_css_class("edit-zone");
    let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indicator.add_css_class("edit-insert");
    indicator.set_can_target(false);
    // Indice d'insertion : avant le premier élément dont le milieu est après le point.
    let index_at = {
        let (b, indicator) = (b.clone(), indicator.clone());
        move |x: f64, y: f64, skip: &str| -> (usize, Option<gtk::Widget>) {
            let vertical = b.orientation() == gtk::Orientation::Vertical;
            let p = if vertical { y } else { x };
            let mut index = 0;
            let mut prev: Option<gtk::Widget> = None;
            let mut c = b.first_child();
            while let Some(w) = c {
                c = w.next_sibling();
                if w == indicator || !w.is_visible() {
                    continue;
                }
                let mid = w.compute_bounds(&b).map(|r| if vertical { r.y() + r.height() / 2.0 } else { r.x() + r.width() / 2.0 } as f64).unwrap_or(0.0);
                if mid > p {
                    break;
                }
                if w.widget_name() != format!("bar-{skip}") {
                    index += 1;
                }
                prev = Some(w);
            }
            (index, prev)
        }
    };
    let index_at = Rc::new(index_at);
    {
        let (b2, indicator, index_at) = (b.clone(), indicator.clone(), index_at.clone());
        drop.connect_motion(move |_, x, y| {
            b2.add_css_class("drop-hover");
            let vertical = b2.orientation() == gtk::Orientation::Vertical;
            if vertical {
                indicator.set_size_request(22, 3);
            } else {
                indicator.set_size_request(3, 22);
            }
            indicator.set_valign(gtk::Align::Center);
            indicator.set_halign(gtk::Align::Center);
            let (_, prev) = index_at(x, y, "");
            if indicator.parent().is_none() {
                b2.append(&indicator);
            }
            b2.reorder_child_after(&indicator, prev.as_ref());
            gdk::DragAction::MOVE
        });
    }
    {
        let (b2, indicator) = (b.clone(), indicator.clone());
        drop.connect_leave(move |_| {
            b2.remove_css_class("drop-hover");
            if indicator.parent().is_some() {
                b2.remove(&indicator);
            }
        });
    }
    let b2 = b.clone();
    drop.connect_drop(move |_, value, x, y| {
        b2.remove_css_class("drop-hover");
        let Ok(s) = value.get::<String>() else { return false };
        let Some(id) = s.strip_prefix("noos-bar-item:").map(String::from) else { return false };
        let (index, _) = index_at(x, y, &id);
        if indicator.parent().is_some() {
            b2.remove(&indicator);
        }
        let Some(shell) = shell.upgrade() else { return false };
        move_item(&shell, &id, zone, Some(index));
        true
    });
    b.add_controller(drop);
}

// ───────────────────────────── Barre d'outils ─────────────────────────────

const HINT: &str = "Glissez les icônes d'une zone à l'autre · déplacez les widgets · Échap pour terminer";

pub struct EditMode {
    shell: Weak<Shell>,
    toolbar: gtk::Window,
    column: gtk::Box,
    pill_rev: gtk::Revealer,
    pub active: Cell<bool>,
    widgets_btn: gtk::ToggleButton,
    bars_btn: gtk::MenuButton,
    layouts_btn: gtk::MenuButton,
    undo_btn: gtk::Button,
    redo_btn: gtk::Button,
    help_btn: gtk::Button,
    done_btn: gtk::Button,
    title_box: gtk::Box,
    gallery_rev: gtk::Revealer,
    gallery: gallery::Gallery,
    hint_box: gtk::Box,
    hint: gtk::Label,
    hint_undo: gtk::Button,
    toast_n: Cell<u32>,
    /// Barre d'outils en bas de l'écran (choisi à l'entrée du mode : le côté qui cache le
    /// moins de widgets ; toujours en bas si la barre des tâches est en haut).
    pub at_bottom: Cell<bool>,
    tour: RefCell<Option<gtk::Popover>>,
    /// Emplacement des notifications (cadre glissable vers un coin).
    notif: gtk::Window,
}

fn tool_button(icon: &str, label: &str) -> gtk::Box {
    let c = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    c.append(&gtk::Image::from_icon_name(icon));
    c.append(&gtk::Label::new(Some(label)));
    c
}

fn sep() -> gtk::Box {
    let s = gtk::Box::new(gtk::Orientation::Vertical, 0);
    s.add_css_class("edit-sep");
    s
}

/// Fichier « visite guidée vue » (`~/.local/state/noos/edit-tour-done`).
fn tour_flag() -> std::path::PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".local/state")).join("noos/edit-tour-done")
}

impl EditMode {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let toolbar = gtk::Window::builder().application(&shell.app).decorated(false).build();
        toolbar.add_css_class("noos-layer");
        toolbar.init_layer_shell();
        toolbar.set_namespace(Some("noos-edit-toolbar"));
        toolbar.set_layer(Layer::Overlay);
        // Le clavier dès l'ouverture (raccourcis), repris par un clic ailleurs.
        toolbar.set_keyboard_mode(KeyboardMode::Exclusive);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.set_halign(gtk::Align::Center);

        // ── Barre principale ──
        let pill = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        pill.add_css_class("edit-toolbar");
        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = gtk::Image::from_icon_name("document-edit-symbolic");
        icon.set_pixel_size(20);
        title_box.append(&icon);
        let tv = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tv.set_valign(gtk::Align::Center);
        let t = gtk::Label::new(Some("Mode édition"));
        t.add_css_class("edit-title");
        t.set_xalign(0.0);
        tv.append(&t);
        let st = gtk::Label::new(Some("Personnalisez votre bureau"));
        st.add_css_class("edit-sub");
        st.set_xalign(0.0);
        tv.append(&st);
        title_box.append(&tv);
        title_box.set_margin_end(6);
        pill.append(&title_box);
        pill.append(&sep());

        let widgets_btn = gtk::ToggleButton::new();
        widgets_btn.add_css_class("flat");
        widgets_btn.add_css_class("tool");
        widgets_btn.set_child(Some(&tool_button("view-app-grid-symbolic", "Widgets")));
        widgets_btn.set_tooltip_text(Some("Galerie de widgets (W)"));
        pill.append(&widgets_btn);

        let bars_btn = gtk::MenuButton::new();
        bars_btn.add_css_class("flat");
        bars_btn.add_css_class("tool");
        bars_btn.set_child(Some(&tool_button("view-continuous-symbolic", "Barres")));
        bars_btn.set_always_show_arrow(true);
        bars_btn.set_tooltip_text(Some("Bord de la barre des tâches, barre d'état, éléments retirés"));
        let bars_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        bars_box.add_css_class("edit-pop");
        let bars_pop = gtk::Popover::new();
        bars_pop.set_child(Some(&bars_box));
        bars_btn.set_popover(Some(&bars_pop));
        pill.append(&bars_btn);

        let layouts_btn = gtk::MenuButton::new();
        layouts_btn.add_css_class("flat");
        layouts_btn.add_css_class("tool");
        layouts_btn.set_child(Some(&tool_button("view-dual-symbolic", "Mises en page")));
        layouts_btn.set_always_show_arrow(true);
        layouts_btn.set_tooltip_text(Some("Modèles, mises en page enregistrées, import / export"));
        let layouts_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        layouts_box.add_css_class("edit-pop");
        layouts_box.set_size_request(520, -1);
        let layouts_pop = gtk::Popover::new();
        layouts_pop.set_child(Some(&layouts_box));
        layouts_btn.set_popover(Some(&layouts_pop));
        pill.append(&layouts_btn);
        pill.append(&sep());

        let undo_btn = gtk::Button::from_icon_name("edit-undo-symbolic");
        undo_btn.add_css_class("flat");
        undo_btn.add_css_class("icon-tool");
        let redo_btn = gtk::Button::from_icon_name("edit-redo-symbolic");
        redo_btn.add_css_class("flat");
        redo_btn.add_css_class("icon-tool");
        pill.append(&undo_btn);
        pill.append(&redo_btn);
        let desk_btn = gtk::Button::from_icon_name("user-desktop-symbolic");
        desk_btn.add_css_class("flat");
        desk_btn.add_css_class("icon-tool");
        desk_btn.set_tooltip_text(Some("Afficher le bureau (réduire les fenêtres)"));
        desk_btn.connect_clicked(|_| apps::request(Request::ShowDesktop));
        pill.append(&desk_btn);
        let help_btn = gtk::Button::with_label("?");
        help_btn.add_css_class("flat");
        help_btn.add_css_class("icon-tool");
        help_btn.set_tooltip_text(Some("Visite guidée"));
        pill.append(&help_btn);
        pill.append(&sep());
        let done_btn = gtk::Button::with_label("Terminé");
        done_btn.add_css_class("suggested-action");
        done_btn.add_css_class("done");
        done_btn.set_tooltip_text(Some("Quitter le mode édition (Échap)"));
        pill.append(&done_btn);
        let pill_rev = gtk::Revealer::new();
        pill_rev.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        pill_rev.set_transition_duration(260);
        pill_rev.set_child(Some(&pill));

        // ── Ligne d'aide / message « … — Annuler » ──
        let hint_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        hint_box.add_css_class("edit-hint");
        hint_box.set_halign(gtk::Align::Center);
        let hint = gtk::Label::new(Some(HINT));
        hint_box.append(&hint);
        let hint_undo = gtk::Button::with_label("Annuler");
        hint_undo.add_css_class("flat");
        hint_undo.set_visible(false);
        hint_box.append(&hint_undo);

        // ── Galerie ──
        let gallery_rev = gtk::Revealer::new();
        gallery_rev.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        gallery_rev.set_transition_duration(220);
        let gw: Rc<RefCell<Option<Weak<EditMode>>>> = Rc::new(RefCell::new(None));
        let gallery = {
            let gw = gw.clone();
            gallery::build(shell, move || {
                if let Some(e) = gw.borrow().as_ref().and_then(|w| w.upgrade()) {
                    e.widgets_btn.set_active(false);
                }
            })
        };
        gallery_rev.set_child(Some(&gallery.panel));

        column.append(&pill_rev);
        column.append(&hint_box);
        column.append(&gallery_rev);
        toolbar.set_child(Some(&column));
        let notif = notif_placeholder(shell);
        let e = Rc::new(Self {
            shell: Rc::downgrade(shell),
            toolbar,
            column,
            pill_rev,
            active: Cell::new(false),
            widgets_btn,
            bars_btn,
            layouts_btn,
            undo_btn,
            redo_btn,
            help_btn,
            done_btn,
            title_box,
            gallery_rev,
            gallery,
            hint_box,
            hint,
            hint_undo,
            toast_n: Cell::new(0),
            at_bottom: Cell::new(false),
            tour: RefCell::new(None),
            notif,
        });
        *gw.borrow_mut() = Some(Rc::downgrade(&e));
        {
            let w = Rc::downgrade(&e);
            e.widgets_btn.connect_toggled(move |b| {
                if let Some(e) = w.upgrade() {
                    e.gallery_rev.set_reveal_child(b.is_active());
                    if b.is_active() {
                        e.gallery.search.grab_focus();
                    }
                }
            });
        }
        {
            let (shell, bars_box, pop2) = (Rc::downgrade(shell), bars_box.clone(), bars_pop.clone());
            bars_pop.connect_show(move |_| {
                if let Some(s) = shell.upgrade() {
                    fill_bars(&s, &bars_box, &pop2);
                }
            });
        }
        {
            let (shell, b, pop2) = (Rc::downgrade(shell), layouts_box.clone(), layouts_pop.clone());
            layouts_pop.connect_show(move |_| {
                if let Some(s) = shell.upgrade() {
                    layouts::fill(&s, &b, &pop2);
                }
            });
        }
        for (btn, redo) in [(&e.undo_btn, false), (&e.redo_btn, true)] {
            let shell = Rc::downgrade(shell);
            btn.connect_clicked(move |_| {
                if let Some(s) = shell.upgrade() {
                    s.undo(redo);
                }
            });
        }
        {
            let shell = Rc::downgrade(shell);
            e.hint_undo.connect_clicked(move |_| {
                if let Some(s) = shell.upgrade() {
                    s.undo(false);
                }
            });
        }
        {
            let w = Rc::downgrade(&e);
            e.done_btn.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.set_active(false);
                }
            });
        }
        {
            let w = Rc::downgrade(&e);
            e.help_btn.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.show_tour(0);
                }
            });
        }
        {
            let search = e.gallery.search.clone();
            let w = Rc::downgrade(&e);
            search.connect_stop_search(move |s| {
                if let Some(e) = w.upgrade() {
                    if s.text().is_empty() {
                        e.widgets_btn.set_active(false);
                    } else {
                        s.set_text("");
                    }
                }
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(&e);
            keys.connect_key_pressed(move |_, k, _, m| {
                let Some(e) = w.upgrade() else { return glib::Propagation::Proceed };
                let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
                let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
                let Some(s) = e.shell.upgrade() else { return glib::Propagation::Proceed };
                match k {
                    gdk::Key::Escape | gdk::Key::Return | gdk::Key::KP_Enter => {
                        if e.widgets_btn.is_active() && k == gdk::Key::Escape {
                            e.widgets_btn.set_active(false);
                        } else {
                            e.set_active(false);
                        }
                    }
                    gdk::Key::z | gdk::Key::Z if ctrl => s.undo(shift),
                    gdk::Key::y | gdk::Key::Y if ctrl => s.undo(true),
                    gdk::Key::f | gdk::Key::F if ctrl => {
                        e.widgets_btn.set_active(true);
                        e.gallery.search.grab_focus();
                    }
                    gdk::Key::w | gdk::Key::W if !ctrl => e.widgets_btn.set_active(!e.widgets_btn.is_active()),
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
            e.toolbar.add_controller(keys);
        }
        if std::env::var_os("NOOS_EDIT_DEMO").is_some() {
            let w = Rc::downgrade(&e);
            glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                let Some(e) = w.upgrade() else { return };
                e.set_active(true);
                e.demo();
            });
        }
        e
    }

    /// Variables de démonstration (captures) : `NOOS_EDIT_OPEN` = étapes séparées par « ; »,
    /// jouées toutes les 1,5 s (`widgets`, `bars`, `layouts`, `settings:n`, `toast:texte`,
    /// `preset:id`, `add:type`, `remove:n`, `undo`, `redo`, `tour:n`, `guides:x,y`).
    fn demo(self: &Rc<Self>) {
        let mut steps: Vec<String> = std::env::var("NOOS_EDIT_OPEN").unwrap_or_default().split(';').filter(|s| !s.is_empty()).map(String::from).collect();
        if let Ok(n) = std::env::var("NOOS_EDIT_TOUR") {
            steps.push(format!("tour:{n}"));
        }
        if let Ok(g) = std::env::var("NOOS_EDIT_GUIDES") {
            steps.push(format!("guides:{g}"));
        }
        let every: u64 = std::env::var("NOOS_EDIT_STEP_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500);
        for (i, step) in steps.into_iter().enumerate() {
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(900 + every * i as u64), move || {
                let Some(e) = w.upgrade() else { return };
                let Some(s) = e.shell.upgrade() else { return };
                let (cmd, arg) = step.split_once(':').unwrap_or((step.as_str(), ""));
                let n: usize = arg.parse().unwrap_or(0);
                match cmd {
                    "widgets" => e.widgets_btn.set_active(true),
                    "bars" => e.bars_btn.popup(),
                    "layouts" => e.layouts_btn.popup(),
                    "settings" => {
                        if let Some(id) = s.config().desktop_widgets.get(n).map(|w| w.id.clone()) {
                            crate::desk::open_settings(&s, &id);
                        }
                    }
                    "toast" => e.toast(arg, true),
                    "preset" => {
                        if let Some(p) = noos_core::layout::presets().into_iter().find(|p| p.id == arg) {
                            let l = p.layout;
                            s.edit_layout(&format!("Modèle « {} »", p.name), move |c| l.apply(c));
                        }
                    }
                    "add" => crate::desk::add_widget(&s, arg, None, None),
                    "remove" => {
                        if let Some(w) = s.config().desktop_widgets.get(n) {
                            crate::desk::remove(&s, &w.id, crate::widgets::spec(&w.kind).name);
                        }
                    }
                    "undo" => s.undo(false),
                    "redo" => s.undo(true),
                    "tour" => e.show_tour(n),
                    "guides" => {
                        if let Some((x, y)) = arg.split_once(',').and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))) {
                            crate::desk::demo_guides(&s, x, y);
                        }
                    }
                    _ => {}
                }
            });
        }
    }

    /// Message dans la ligne d'aide (« … — Annuler » si `undo`), puis retour à l'aide.
    pub fn toast(&self, text: &str, undo: bool) {
        if !self.active.get() {
            return;
        }
        let n = self.toast_n.get() + 1;
        self.toast_n.set(n);
        self.hint.set_text(text);
        self.hint_undo.set_visible(undo);
        self.hint_box.add_css_class("toast");
        self.sync_history();
        let (hint, box_, undo_btn) = (self.hint.downgrade(), self.hint_box.downgrade(), self.hint_undo.downgrade());
        let counter = self.shell.upgrade().and_then(|s| s.editmode.borrow().as_ref().map(Rc::downgrade));
        glib::timeout_add_local_once(std::time::Duration::from_secs(6), move || {
            let Some(e) = counter.and_then(|w| w.upgrade()) else { return };
            if e.toast_n.get() != n {
                return;
            }
            if let (Some(h), Some(b), Some(u)) = (hint.upgrade(), box_.upgrade(), undo_btn.upgrade()) {
                h.set_text(HINT);
                b.remove_css_class("toast");
                u.set_visible(false);
            }
        });
    }

    fn sync_history(&self) {
        let Some(s) = self.shell.upgrade() else { return };
        let h = s.history.borrow();
        self.undo_btn.set_sensitive(h.undo_label().is_some());
        self.redo_btn.set_sensitive(h.redo_label().is_some());
        self.undo_btn.set_tooltip_text(Some(&match h.undo_label() {
            Some(l) => format!("Annuler : {l} (Ctrl+Z)"),
            None => "Rien à annuler".into(),
        }));
        self.redo_btn.set_tooltip_text(Some(&match h.redo_label() {
            Some(l) => format!("Rétablir : {l} (Ctrl+Y)"),
            None => "Rien à rétablir".into(),
        }));
    }

    pub fn sync_labels(&self) {
        let Some(s) = self.shell.upgrade() else { return };
        let cfg = s.config();
        self.sync_history();
        // Barre d'outils en haut, sauf si la barre des tâches y est (la galerie s'ouvre alors
        // vers le haut).
        let bottom = self.at_bottom.get();
        self.toolbar.set_anchor(Edge::Top, !bottom);
        self.toolbar.set_anchor(Edge::Bottom, bottom);
        self.toolbar.set_margin(if bottom { Edge::Bottom } else { Edge::Top }, 14);
        self.toolbar.set_margin(if bottom { Edge::Top } else { Edge::Bottom }, 0);
        let order: [&gtk::Widget; 3] = if bottom {
            [self.gallery_rev.upcast_ref(), self.hint_box.upcast_ref(), self.pill_rev.upcast_ref()]
        } else {
            [self.pill_rev.upcast_ref(), self.hint_box.upcast_ref(), self.gallery_rev.upcast_ref()]
        };
        self.column.reorder_child_after(order[0], None::<&gtk::Widget>);
        self.column.reorder_child_after(order[1], Some(order[0]));
        self.column.reorder_child_after(order[2], Some(order[1]));
        self.gallery_rev.set_transition_type(if bottom { gtk::RevealerTransitionType::SlideUp } else { gtk::RevealerTransitionType::SlideDown });
        self.pill_rev.set_transition_type(if bottom { gtk::RevealerTransitionType::SlideUp } else { gtk::RevealerTransitionType::SlideDown });
        place_notif(&self.notif, &cfg.notifications.position);
        // Écran principal.
        if let Some(p) = s.primary_output() {
            let monitors = gdk::Display::default().map(|d| d.monitors());
            if let Some(m) = monitors.and_then(|ms| (0..ms.n_items()).filter_map(|i| ms.item(i).and_downcast::<gdk::Monitor>()).find(|m| m.connector().is_some_and(|c| c == p))) {
                self.toolbar.set_monitor(Some(&m));
                self.notif.set_monitor(Some(&m));
            }
        }
    }

    pub fn toggle(&self) {
        self.set_active(!self.active.get());
    }

    pub fn set_active(&self, on: bool) {
        if self.active.replace(on) == on && on {
            return;
        }
        if let Some(s) = self.shell.upgrade() {
            s.close_flyouts();
            s.set_editing(on);
        }
        if on {
            if let Some(s) = self.shell.upgrade() {
                let bottom = s.config().taskbar.position == "top" || crate::desk::toolbar_overlap(&s, true) > crate::desk::toolbar_overlap(&s, false) + 1.0;
                self.at_bottom.set(bottom);
            }
            self.sync_labels();
            self.hint.set_text(HINT);
            self.hint_box.remove_css_class("toast");
            self.hint_undo.set_visible(false);
            self.toolbar.present();
            self.notif.present();
            let rev = self.pill_rev.clone();
            glib::idle_add_local_once(move || rev.set_reveal_child(true));
            // Première fois : visite guidée.
            if !tour_flag().exists() && std::env::var_os("NOOS_EDIT_DEMO").is_none() {
                let shell = self.shell.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    if let Some(e) = shell.upgrade().and_then(|s| s.editmode.borrow().clone()) {
                        e.show_tour(0);
                    }
                });
            }
        } else {
            self.close_tour();
            self.widgets_btn.set_active(false);
            self.pill_rev.set_reveal_child(false);
            self.toolbar.set_visible(false);
            self.notif.set_visible(false);
        }
    }

    /// Ouvre le mode édition avec la galerie (menu du bureau « Ajouter des widgets »).
    pub fn open_gallery(&self) {
        self.set_active(true);
        self.widgets_btn.set_active(true);
    }

    fn close_tour(&self) {
        if let Some(p) = self.tour.borrow_mut().take() {
            p.popdown();
            p.unparent();
        }
    }

    /// Visite guidée : une bulle par étape, pointée sur le bouton concerné.
    pub fn show_tour(self: &Rc<Self>, step: usize) {
        self.close_tour();
        let steps: [(&gtk::Widget, &str, &str); 6] = [
            (self.title_box.upcast_ref(), "Bienvenue dans le mode édition", "Ici, tout se déplace : les icônes de la barre, les widgets du bureau, les notifications. Rien n'est définitif : Ctrl+Z annule."),
            (self.widgets_btn.upcast_ref(), "Ajoutez des widgets", "Météo, horloges du monde, lecteur multimédia, cadre photo… Glissez une carte sur le bureau ou cliquez dessus. Survolez un widget : sa barre (⚙ réglages, × retirer) apparaît ; la poignée en bas à droite change sa taille."),
            (self.bars_btn.upcast_ref(), "Réorganisez les barres", "Dans la barre, glissez une icône vers une autre zone (un trait montre où elle ira), ou clic droit › « Vers… ». Ici : le bord de la barre des tâches et la barre d'état."),
            (self.layouts_btn.upcast_ref(), "Mises en page", "Un modèle en un clic (Windows 11, Classique, GNOME…), ou enregistrez la vôtre pour la retrouver plus tard, et partagez-la."),
            (self.undo_btn.upcast_ref(), "Annuler, rétablir", "Ctrl+Z et Ctrl+Y fonctionnent pour tout. Après chaque action, la ligne sous la barre propose aussi « Annuler »."),
            (self.done_btn.upcast_ref(), "À vous de jouer !", "Échap ou « Terminé » pour quitter. Le bouton ? relance cette visite."),
        ];
        let Some((target, title, text)) = steps.get(step).cloned() else {
            let _ = std::fs::create_dir_all(tour_flag().parent().unwrap());
            let _ = std::fs::write(tour_flag(), "");
            return;
        };
        let pop = gtk::Popover::new();
        pop.set_autohide(false);
        pop.set_position(if self.toolbar.is_anchor(Edge::Bottom) { gtk::PositionType::Top } else { gtk::PositionType::Bottom });
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.add_css_class("edit-tour");
        b.set_size_request(320, -1);
        let n = gtk::Label::new(Some(&format!("ÉTAPE {} SUR {}", step + 1, steps.len())));
        n.add_css_class("tour-step");
        n.set_xalign(0.0);
        b.append(&n);
        let t = gtk::Label::new(Some(title));
        t.add_css_class("tour-title");
        t.set_xalign(0.0);
        b.append(&t);
        let l = gtk::Label::new(Some(text));
        l.set_wrap(true);
        l.set_max_width_chars(40);
        l.set_xalign(0.0);
        b.append(&l);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_margin_top(6);
        let skip = gtk::Button::with_label("Passer");
        skip.add_css_class("flat");
        skip.set_hexpand(true);
        skip.set_halign(gtk::Align::Start);
        let last = step + 1 == steps.len();
        skip.set_visible(!last);
        let next = gtk::Button::with_label(if last { "C'est parti" } else { "Suivant" });
        next.add_css_class("suggested-action");
        row.append(&skip);
        row.append(&next);
        b.append(&row);
        pop.set_child(Some(&b));
        pop.set_parent(target);
        {
            let w = Rc::downgrade(self);
            next.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.show_tour(step + 1);
                }
            });
        }
        {
            let w = Rc::downgrade(self);
            skip.connect_clicked(move |_| {
                if let Some(e) = w.upgrade() {
                    e.show_tour(usize::MAX);
                }
            });
        }
        *self.tour.borrow_mut() = Some(pop.clone());
        glib::idle_add_local_once(move || pop.popup());
    }
}

/// Panneau « Barres » : bord de la barre des tâches (schémas), alignement, barre d'état,
/// éléments retirés.
fn fill_bars(shell: &Rc<Shell>, b: &gtk::Box, pop: &gtk::Popover) {
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
        if !sub.is_empty() {
            let s = gtk::Label::new(Some(sub));
            s.add_css_class("dim-label");
            s.add_css_class("caption");
            s.set_xalign(0.0);
            s.set_wrap(true);
            s.set_max_width_chars(48);
            v.append(&s);
        }
        v
    };
    b.append(&title("Barre des tâches", "Bord de l'écran où elle se place"));
    let edges = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    edges.set_homogeneous(true);
    let mut first: Option<gtk::ToggleButton> = None;
    for (edge, label) in [("bottom", "En bas"), ("top", "En haut"), ("left", "À gauche"), ("right", "À droite")] {
        let t = gtk::ToggleButton::new();
        t.add_css_class("flat");
        t.add_css_class("edge-card");
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let mut l = current.clone();
        l.position = edge.into();
        v.append(&layouts::thumb(l, 96, 58));
        v.append(&gtk::Label::new(Some(label)));
        t.set_child(Some(&v));
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
        }
        t.set_active(cfg.taskbar.position == edge || (edge == "bottom" && !matches!(cfg.taskbar.position.as_str(), "top" | "left" | "right")));
        let s = Rc::downgrade(shell);
        t.connect_toggled(move |t| {
            if !t.is_active() {
                return;
            }
            let Some(s) = s.upgrade() else { return };
            if s.config().taskbar.position == edge {
                return;
            }
            s.edit_layout(&format!("Barre des tâches {}", label.to_lowercase()), move |c| c.taskbar.position = edge.into());
        });
        edges.append(&t);
    }
    b.append(&edges);
    let switch_row = |label: &str, sub: &str, on: bool, f: Box<dyn Fn(bool)>| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let v = title(label, sub);
        v.set_hexpand(true);
        row.append(&v);
        let sw = gtk::Switch::new();
        sw.set_active(on);
        sw.set_valign(gtk::Align::Center);
        sw.connect_active_notify(move |sw| f(sw.is_active()));
        row.append(&sw);
        row
    };
    if !cfg.taskbar.vertical() {
        let s = Rc::downgrade(shell);
        b.append(&switch_row(
            "Icônes centrées",
            "Sinon, alignées à gauche comme sur Windows 10",
            cfg.taskbar.alignment == noos_core::config::TaskbarAlignment::Center,
            Box::new(move |on| {
                if let Some(s) = s.upgrade() {
                    s.edit_layout(if on { "Icônes centrées" } else { "Icônes alignées à gauche" }, move |c| {
                        c.taskbar.alignment = if on { noos_core::config::TaskbarAlignment::Center } else { noos_core::config::TaskbarAlignment::Left };
                    });
                }
            }),
        ));
    }
    b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    {
        let s = Rc::downgrade(shell);
        b.append(&switch_row(
            "Barre d'état",
            "Seconde barre, au bord opposé : bureaux virtuels, et tout ce que vous y glissez (horloge, réseau, notifications…)",
            cfg.topbar.enabled,
            Box::new(move |on| {
                if let Some(s) = s.upgrade() {
                    s.edit_layout(if on { "Barre d'état affichée" } else { "Barre d'état masquée" }, move |c| c.topbar.enabled = on);
                }
            }),
        ));
    }
    b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    b.append(&title("Éléments retirés", "Remettez dans la barre ce que vous en avez retiré"));
    let off: Vec<&str> = ["vitals", "start", "search", "taskview", "tasks", "tray", "status", "power", "clock", "notifications", "showdesktop", "desktops"]
        .into_iter()
        .filter(|id| {
            cfg.taskbar.hidden.iter().any(|h| h == id)
                || match *id {
                    "search" => !cfg.taskbar.show_search,
                    "vitals" => !cfg.taskbar.show_vitals,
                    "power" => !cfg.taskbar.show_power,
                    "taskview" => !cfg.desktops.taskbar_button,
                    _ => false,
                }
        })
        .collect();
    if off.is_empty() {
        let l = gtk::Label::new(Some("Aucun : tous les éléments sont dans les barres."));
        l.add_css_class("dim-label");
        l.set_xalign(0.0);
        b.append(&l);
    }
    for id in off {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("item-row");
        row.append(&gtk::Image::from_icon_name(item_icon(id)));
        let l = gtk::Label::new(Some(item_label(id)));
        l.set_hexpand(true);
        l.set_xalign(0.0);
        row.append(&l);
        let btn = gtk::Button::with_label("Remettre");
        let (s, pop2, b2) = (Rc::downgrade(shell), pop.clone(), b.clone());
        btn.connect_clicked(move |_| {
            if let Some(s) = s.upgrade() {
                restore_item(&s, id);
                fill_bars(&s, &b2, &pop2);
            }
        });
        row.append(&btn);
        b.append(&row);
    }
    let more = gtk::Button::with_label("Plus de réglages de la barre…");
    more.add_css_class("flat");
    {
        let pop = pop.clone();
        more.connect_clicked(move |_| {
            pop.popdown();
            apps::spawn("noos-settings taskbar");
        });
    }
    b.append(&more);
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

fn corner_label(corner: &str) -> &'static str {
    match corner {
        "top-left" => "en haut à gauche",
        "top-right" => "en haut à droite",
        "bottom-left" => "en bas à gauche",
        _ => "en bas à droite",
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
        let shell = Rc::downgrade(shell);
        Rc::new(move |corner: &'static str| {
            let Some(s) = shell.upgrade() else { return };
            if s.config().notifications.position == corner {
                return;
            }
            s.edit_layout(&format!("Notifications {}", corner_label(corner)), |c| c.notifications.position = corner.into());
        })
    };
    for (corner, arrow, col, row) in [("top-left", "↖", 0, 0), ("top-right", "↗", 1, 0), ("bottom-left", "↙", 0, 1), ("bottom-right", "↘", 1, 1)] {
        let btn = gtk::Button::with_label(arrow);
        btn.add_css_class("corner-btn");
        let mut tip = corner_label(corner).to_string();
        if let Some(f) = tip.get_mut(0..1) {
            f.make_ascii_uppercase();
        }
        btn.set_tooltip_text(Some(&tip));
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

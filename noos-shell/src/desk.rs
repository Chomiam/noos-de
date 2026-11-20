//! Surface des widgets du bureau (une par écran, couche « bottom », jamais dans le fond
//! d'écran : chaque image d'un widget y ferait recalculer le flou Mica).
//!
//! En mode édition : bureau assombri avec une grille de points, chaque widget a une barre de
//! titre (nom, ⚙ réglages, × retirer) et une poignée de taille ; il se déplace par tout son
//! corps, aimanté à la grille et aux autres widgets (repères d'alignement ; Alt = libre). Les
//! cartes de la galerie se déposent ici. Toute modification passe par `Shell::edit_layout`
//! (annulable).

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::{
    NoosConfig,
    config::DesktopWidget,
    layout::{self, Guide, Rect, SnapArea},
};

use crate::{Shell, widgets};

pub const CSS: &str = r#"
.desk-card { border-radius: 18px; padding: 16px 18px; transition: opacity 260ms ease-out, transform 320ms cubic-bezier(0.2, 0.9, 0.3, 1.15); }
.desk-card.style-glass { color: @window_fg_color; border: 1px solid alpha(white, 0.10); box-shadow: 0 0 0 0 transparent, 0 10px 30px rgba(0,0,0,0.28); }
.desk-card.style-solid { color: @window_fg_color; border: 1px solid alpha(@window_fg_color, 0.08); box-shadow: 0 0 0 0 transparent, 0 10px 30px rgba(0,0,0,0.32); }
.desk-card.style-clear { background: none; border: none; box-shadow: none; padding: 12px 14px; color: white; }
.desk-card.style-clear label { text-shadow: 0 1px 8px rgba(0,0,0,0.55); }
.desk-card.appear { opacity: 0; transform: scale(0.86); }
.desk-card.dragging { transform: scale(1.02); box-shadow: 0 0 0 2px @accent_bg_color, 0 22px 50px rgba(0,0,0,0.45); }
.editing-desk .desk-card { box-shadow: 0 0 0 1.5px alpha(@accent_bg_color, 0.75), 0 10px 30px rgba(0,0,0,0.3); }
.editing-desk .desk-card.style-clear { background-color: alpha(black, 0.18); }
.desk-shield { border-radius: 18px; }
.desk-shield:hover { background-color: alpha(@accent_bg_color, 0.10); }
.desk-chrome { min-height: 32px; opacity: 0; transition: opacity 140ms ease-out; }
.desk-outer:hover .desk-chrome, .desk-outer.active .desk-chrome, .desk-outer.selected .desk-chrome { opacity: 1; }
.editing-desk .desk-outer.selected .desk-card { box-shadow: 0 0 0 3px @accent_bg_color, 0 14px 36px rgba(0,0,0,0.4); }
.desk-chip { padding: 0 12px 0 8px; min-height: 30px; border-radius: 99px; background-color: @accent_bg_color; color: @accent_fg_color; font-weight: 700; font-size: 0.85em; box-shadow: 0 4px 12px rgba(0,0,0,0.3); }
.desk-chip-btn { min-width: 30px; min-height: 30px; padding: 0; border-radius: 99px; color: @window_fg_color; box-shadow: 0 4px 12px rgba(0,0,0,0.3); }
.desk-chip-btn:hover { }
.desk-chip-btn.danger:hover { background-color: #e5484d; color: white; }
.desk-resize { min-width: 20px; min-height: 20px; border-radius: 99px; margin: 3px; background-color: @accent_bg_color; color: @accent_fg_color; box-shadow: 0 2px 8px rgba(0,0,0,0.35); }
.desk-guides { color: @accent_bg_color; }
.desk-screen-name { padding: 6px 16px; border-radius: 99px; background-color: rgba(0,0,0,0.5); color: white; font-weight: 700; }
.desk-settings { padding: 6px 4px; }
.desk-settings .desk-set-head { font-weight: 800; font-size: 1.1em; }
.desk-settings-tile { min-width: 40px; min-height: 40px; border-radius: 12px; color: white; }
"#;

/// Hauteur de la barre de titre (mode édition) au-dessus du widget, espacement compris.
const CHROME_H: f64 = 40.0;

pub struct DeskWidget {
    pub conf: RefCell<DesktopWidget>,
    outer: gtk::Box,
    chrome: gtk::Box,
    card: gtk::Box,
    shield: gtk::Box,
    resize: gtk::Box,
    pub settings: gtk::Popover,
    settings_btn: gtk::Button,
    ctx: widgets::Ctx,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Move,
    Resize,
}

struct Drag {
    id: String,
    mode: Mode,
    /// Position de départ du cadre (coin supérieur gauche de la carte).
    start: (f64, f64),
    size0: usize,
}

pub struct Inner {
    pub window: gtk::Window,
    fixed: gtk::Fixed,
    guides: gtk::DrawingArea,
    /// Nom de l'écran (mode édition, plusieurs écrans).
    screen_label: gtk::Label,
    widgets: RefCell<Vec<Rc<DeskWidget>>>,
    editing: Cell<bool>,
    input: RefCell<Vec<(i32, i32, i32, i32)>>,
    pub output: String,
    monitor: gdk::Monitor,
    shell: Weak<Shell>,
    lines: RefCell<Vec<Guide>>,
    ghost: RefCell<Option<Rect>>,
    drag: RefCell<Option<Drag>>,
    built: Cell<bool>,
}

pub type DeskLayer = Rc<Inner>;

const SIZES: [&str; 3] = ["s", "m", "l"];

fn size_index(s: &str) -> usize {
    SIZES.iter().position(|x| *x == s).unwrap_or(1)
}

impl DeskWidget {
    fn id(&self) -> String {
        self.conf.borrow().id.clone()
    }

    fn name(&self) -> &'static str {
        widgets::spec(&self.conf.borrow().kind).name
    }

    /// Rectangle de la carte dans la surface.
    fn rect(&self, fixed: &gtk::Fixed, editing: bool) -> Rect {
        let (x, y) = fixed.child_position(&self.outer);
        let (w, h) = self.size();
        Rect::new(x, y + if editing { CHROME_H } else { 0.0 }, w, h)
    }

    /// Taille de la carte, bordure et marges intérieures comprises (`width()` ne donne que le
    /// contenu).
    fn size(&self) -> (f64, f64) {
        match self.card.compute_bounds(&self.card) {
            Some(b) if b.width() > 0.0 => (b.width() as f64, b.height() as f64),
            _ => {
                let (min, _) = self.card.preferred_size();
                (min.width() as f64, min.height() as f64)
            }
        }
    }

    fn set_classes(&self, w: &DesktopWidget) {
        let style = widgets::style_of(w);
        for s in SIZES {
            self.card.remove_css_class(&format!("size-{s}"));
        }
        for s in ["glass", "solid", "clear"] {
            self.card.remove_css_class(&format!("style-{s}"));
        }
        self.card.add_css_class(&format!("size-{}", w.size()));
        self.card.add_css_class(&format!("style-{style}"));
        // Même largeur pour tous les widgets d'une taille (le contenu s'y range).
        self.card.set_size_request(widgets::width(w), -1);
    }

    fn rebuild_content(&self) {
        while let Some(c) = self.card.first_child() {
            self.card.remove(&c);
        }
        let w = self.conf.borrow().clone();
        self.card.append(&widgets::build(&self.ctx, &w));
    }

    fn set_editing(&self, on: bool) {
        self.chrome.set_visible(on);
        self.shield.set_visible(on);
        self.resize.set_visible(on);
    }

    /// Remplit le panneau de réglages avec la configuration actuelle.
    fn fill_settings(&self, shell: &Rc<Shell>) {
        let Some(w) = self.ctx.conf() else { return };
        let spec = widgets::spec(&w.kind);
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.add_css_class("desk-settings");
        b.set_size_request(300, -1);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tile.add_css_class("desk-settings-tile");
        tile.add_css_class(&format!("tile-{}", w.kind));
        tile.set_hexpand(false);
        tile.set_valign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name(spec.icon);
        icon.set_pixel_size(22);
        icon.set_hexpand(true);
        tile.append(&icon);
        head.append(&tile);
        let hv = gtk::Box::new(gtk::Orientation::Vertical, 0);
        hv.set_valign(gtk::Align::Center);
        let n = gtk::Label::new(Some(spec.name));
        n.add_css_class("desk-set-head");
        n.set_xalign(0.0);
        hv.append(&n);
        let d = gtk::Label::new(Some(spec.description));
        d.add_css_class("desk-dim");
        d.add_css_class("caption");
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_max_width_chars(30);
        hv.append(&d);
        head.append(&hv);
        b.append(&head);
        widgets::section(&b, "Taille");
        let c = self.ctx.clone();
        let name = spec.name;
        widgets::choice(&b, &[("s", "Petite"), ("m", "Moyenne"), ("l", "Grande")], w.size(), move |v| c.edit(&format!("Taille de « {name} »"), move |w| w.size = v.into()));
        if w.kind != "note" {
            widgets::section(&b, "Fond");
            let c = self.ctx.clone();
            widgets::choice(&b, &[("glass", "Verre"), ("solid", "Opaque"), ("clear", "Aucun")], widgets::style_of(&w), move |v| c.edit(&format!("Fond de « {name} »"), move |w| w.style = v.into()));
        }
        // Écran (plusieurs écrans).
        let outs: Vec<(String, bool)> = shell.outputs.borrow().iter().map(|o| (o.name.clone(), o.primary)).collect();
        if outs.len() > 1 {
            widgets::section(&b, "Écran");
            let labels: Vec<String> = outs.iter().enumerate().map(|(i, (n, p))| format!("Écran {} · {n}{}", i + 1, if *p { " (principal)" } else { "" })).collect();
            let dd = gtk::DropDown::from_strings(&labels.iter().map(String::as_str).collect::<Vec<_>>());
            let cur = if w.output.is_empty() { outs.iter().position(|o| o.1) } else { outs.iter().position(|o| o.0 == w.output) };
            dd.set_selected(cur.unwrap_or(0) as u32);
            let (c, outs) = (self.ctx.clone(), outs.clone());
            dd.connect_selected_notify(move |dd| {
                if let Some((n, _)) = outs.get(dd.selected() as usize) {
                    let n = n.clone();
                    c.edit(&format!("« {name} » déplacé sur {n}"), move |w| {
                        w.output = n;
                    });
                }
            });
            b.append(&dd);
        }
        widgets::settings(&self.ctx, &w, &b);
        b.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dup = gtk::Button::with_label("Dupliquer");
        dup.set_hexpand(true);
        {
            let (shell, w) = (Rc::downgrade(shell), w.clone());
            dup.connect_clicked(move |_| {
                let Some(s) = shell.upgrade() else { return };
                let mut copy = w.clone();
                copy.id = noos_core::config::new_widget_id();
                copy.x = (copy.x + 0.03).min(0.9);
                copy.y = (copy.y + 0.04).min(0.9);
                s.edit_layout(&format!("« {name} » dupliqué"), move |c| c.desktop_widgets.push(copy));
            });
        }
        row.append(&dup);
        let del = gtk::Button::with_label("Retirer");
        del.add_css_class("destructive-action");
        del.set_hexpand(true);
        {
            let (shell, id, pop) = (Rc::downgrade(shell), w.id.clone(), self.settings.clone());
            del.connect_clicked(move |_| {
                pop.popdown();
                if let Some(s) = shell.upgrade() {
                    remove(&s, &id, name);
                }
            });
        }
        row.append(&del);
        b.append(&row);
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_propagate_natural_height(true);
        sc.set_max_content_height(560);
        sc.set_child(Some(&b));
        self.settings.set_child(Some(&sc));
    }

    fn open_settings(&self, shell: &Rc<Shell>, at: Option<&gtk::Widget>) {
        self.fill_settings(shell);
        // En mode édition : contre le bouton ⚙, vers le bas ou vers le haut selon la place
        // (et la hauteur du panneau limitée à cette place) ; sinon à côté du widget.
        let at = at.cloned().or_else(|| self.chrome.is_visible().then(|| self.settings_btn.clone().upcast()));
        let target = at.clone().unwrap_or_else(|| self.card.clone().upcast());
        let screen_h = self.outer.root().map(|r| r.height() as f64).unwrap_or(1080.0);
        let y = self.outer.root().and_then(|r| target.compute_bounds(&r)).map(|b| (b.y() as f64, b.height() as f64)).unwrap_or((0.0, 0.0));
        // Place libre : sans les barres ni la barre d'outils du mode édition.
        let cfg = shell.config();
        let (bar_top, bar_bottom) = {
            let t = |top: bool| crate::taskbar::bar_height(&cfg, top) as f64;
            let dock = crate::taskbar::bar_edge(&cfg, false);
            let status = if cfg.topbar.enabled { crate::taskbar::bar_edge(&cfg, true) } else { "" };
            let on = |edge: &str| (if dock == edge { t(false) } else { 0.0 }) + (if status == edge { t(true) } else { 0.0 });
            (on("top"), on("bottom"))
        };
        let tool = |bottom: bool| match shell.editmode.borrow().as_ref() {
            Some(e) if e.active.get() && e.at_bottom.get() == bottom => 150.0,
            _ => 0.0,
        };
        let below = screen_h - bar_bottom - tool(true) - y.0 - y.1 - 16.0;
        let above = y.0 - bar_top - tool(false) - 16.0;
        let down = below >= 560.0 || below >= above;
        self.settings.set_position(match (at.is_some(), down) {
            (false, _) => gtk::PositionType::Right,
            (true, true) => gtk::PositionType::Bottom,
            (true, false) => gtk::PositionType::Top,
        });
        let room = if at.is_some() { if down { below } else { above } } else { screen_h - 48.0 };
        if let Some(sc) = self.settings.child().and_downcast::<gtk::ScrolledWindow>() {
            sc.set_max_content_height((room as i32 - 40).clamp(200, 600));
        }
        if let Some(r) = target.compute_bounds(&self.outer) {
            self.settings.set_pointing_to(Some(&gdk::Rectangle::new(r.x() as i32, r.y() as i32, r.width() as i32, r.height() as i32)));
        }
        self.settings.popup();
    }
}

/// Retire un widget (annulable : son contenu, le texte d'une note par exemple, revient).
pub fn remove(shell: &Rc<Shell>, id: &str, name: &str) {
    let id = id.to_string();
    shell.edit_layout(&format!("« {name} » retiré"), move |c| c.desktop_widgets.retain(|w| w.id != id));
}

// ───────────────────────────── Clavier (mode édition) ─────────────────────────────

/// Écrans de gauche à droite, et leurs widgets de haut en bas, de gauche à droite.
fn ordered(shell: &Rc<Shell>) -> Vec<(DeskLayer, Rc<DeskWidget>)> {
    let mut layers: Vec<DeskLayer> = shell.wallpapers.borrow().iter().map(|w| w.desk.clone()).collect();
    layers.sort_by_key(|l| (l.monitor.geometry().x(), l.monitor.geometry().y()));
    let mut v = vec![];
    for l in layers {
        let mut ws: Vec<Rc<DeskWidget>> = l.widgets.borrow().clone();
        ws.sort_by(|a, b| {
            let (ra, rb) = (a.rect(&l.fixed, true), b.rect(&l.fixed, true));
            ((ra.y / 40.0).round(), ra.x).partial_cmp(&((rb.y / 40.0).round(), rb.x)).unwrap_or(std::cmp::Ordering::Equal)
        });
        v.extend(ws.into_iter().map(|w| (l.clone(), w)));
    }
    v
}

fn find(shell: &Rc<Shell>, id: &str) -> Option<(DeskLayer, Rc<DeskWidget>)> {
    ordered(shell).into_iter().find(|(_, d)| d.id() == id)
}

/// Widget suivant (`delta` = 1) ou précédent après `current` ; son identifiant et son nom.
pub fn cycle(shell: &Rc<Shell>, current: Option<&str>, delta: i32) -> Option<(String, &'static str)> {
    let all = ordered(shell);
    if all.is_empty() {
        return None;
    }
    let n = all.len() as i32;
    let i = match current.and_then(|c| all.iter().position(|(_, d)| d.id() == c)) {
        Some(i) => (i as i32 + delta).rem_euclid(n),
        None if delta < 0 => n - 1,
        None => 0,
    };
    let (_, d) = &all[i as usize];
    Some((d.id(), d.name()))
}

/// Widget mis en avant (clavier) : barre de titre visible, contour marqué.
pub fn mark_selected(shell: &Rc<Shell>, id: Option<&str>) {
    for (l, d) in ordered(shell) {
        if Some(d.id().as_str()) == id {
            d.outer.add_css_class("selected");
            // Au premier plan (dernier enfant du `gtk::Fixed`, dessiné par-dessus les autres).
            d.outer.insert_before(&l.fixed, None::<&gtk::Widget>);
        } else {
            d.outer.remove_css_class("selected");
        }
    }
}

/// Déplace un widget de (dx, dy) pixels (flèches du clavier), sans sortir de l'écran.
pub fn nudge(shell: &Rc<Shell>, id: &str, dx: f64, dy: f64) {
    let Some((l, d)) = find(shell, id) else { return };
    let (sw, sh) = l.size();
    let r = d.rect(&l.fixed, true);
    let x = (r.x + dx).min(sw - r.w).max(0.0);
    let y = (r.y + dy).min(sh - r.h).max(CHROME_H);
    let (fx, fy) = (x / sw.max(1.0), y / sh.max(1.0));
    let id = id.to_string();
    shell.edit_layout(&format!("« {} » déplacé", d.name()), move |c| {
        if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
            w.x = fx;
            w.y = fy;
        }
    });
}

/// Taille voisine (`delta` = 1 : plus grande).
pub fn resize_step(shell: &Rc<Shell>, id: &str, delta: i32) {
    let Some((_, d)) = find(shell, id) else { return };
    let i = (size_index(d.conf.borrow().size()) as i32 + delta).clamp(0, 2) as usize;
    let size = SIZES[i];
    let label = format!("« {} » : taille {}", d.name(), match size {
        "s" => "petite",
        "l" => "grande",
        _ => "moyenne",
    });
    let id = id.to_string();
    shell.edit_layout(&label, move |c| {
        if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
            w.size = size.into();
        }
    });
}

/// Envoie un widget sur l'écran voisin (`dir` = -1 : à gauche), à la même place relative.
pub fn to_neighbor_output(shell: &Rc<Shell>, id: &str, dir: i32) -> Option<String> {
    let Some((l, d)) = find(shell, id) else { return None };
    let mut layers: Vec<DeskLayer> = shell.wallpapers.borrow().iter().map(|w| w.desk.clone()).collect();
    layers.sort_by_key(|x| (x.monitor.geometry().x(), x.monitor.geometry().y()));
    let i = layers.iter().position(|x| Rc::ptr_eq(x, &l))? as i32 + dir;
    let target = layers.get(usize::try_from(i).ok()?)?.output.clone();
    let primary = shell.primary_output();
    let out = if primary.as_deref() == Some(target.as_str()) { String::new() } else { target.clone() };
    let id = id.to_string();
    shell.edit_layout(&format!("« {} » déplacé sur {target}", d.name()), move |c| {
        if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
            w.output = out;
        }
    });
    Some(target)
}

/// Un panneau de réglages de widget est-il ouvert ?
pub fn settings_open(shell: &Rc<Shell>) -> bool {
    shell.wallpapers.borrow().iter().any(|w| w.desk.widgets.borrow().iter().any(|d| d.settings.is_visible()))
}

/// Retire un widget par son identifiant (touche Suppr).
pub fn remove_by_id(shell: &Rc<Shell>, id: &str) {
    if let Some((_, d)) = find(shell, id) {
        remove(shell, id, d.name());
    }
}

/// Ouvre les réglages d'un widget, où qu'il soit.
pub fn open_settings(shell: &Rc<Shell>, id: &str) {
    for w in shell.wallpapers.borrow().iter() {
        let found = w.desk.widgets.borrow().iter().find(|d| d.id() == id).cloned();
        if let Some(d) = found {
            d.open_settings(shell, None);
            return;
        }
    }
}

/// Démonstration (`NOOS_EDIT_GUIDES=x,y`) : le premier widget de l'écran principal est
/// « en cours de déplacement » vers (x, y), repères affichés (rien n'est enregistré).
pub fn demo_guides(shell: &Rc<Shell>, x: f64, y: f64) {
    let primary = shell.primary_output();
    for w in shell.wallpapers.borrow().iter() {
        let l = &w.desk;
        if primary.as_ref().is_some_and(|p| *p != l.output) {
            continue;
        }
        let Some(d) = l.widgets.borrow().first().cloned() else { continue };
        let (cw, ch) = d.size();
        let r = Rect::new(x, y, cw, ch);
        let others: Vec<Rect> = l.widgets.borrow().iter().skip(1).map(|o| o.rect(&l.fixed, true)).collect();
        let (nx, ny, lines) = layout::snap(r, &others, &l.snap_area());
        l.fixed.move_(&d.outer, nx, ny - CHROME_H);
        d.card.add_css_class("dragging");
        d.outer.add_css_class("active");
        *l.lines.borrow_mut() = lines;
        l.guides.queue_draw();
        return;
    }
}

/// Surface des widgets (barres de titre comprises) de l'écran principal que recouvrirait la
/// barre d'outils du mode édition, en haut (`top`) ou en bas.
pub fn toolbar_overlap(shell: &Rc<Shell>, top: bool) -> f64 {
    let primary = shell.primary_output();
    let cfg = shell.config();
    let mut total = 0.0;
    for w in shell.wallpapers.borrow().iter() {
        let l = &w.desk;
        if primary.as_ref().is_some_and(|p| *p != l.output) {
            continue;
        }
        let (sw, sh) = l.size();
        let work = work_area(shell, &cfg, &l.output, sw, sh);
        let (tw, th) = (980.0, 150.0);
        let bar = Rect::new((sw - tw) / 2.0, if top { work.y } else { work.y + work.h - th }, tw, th);
        for d in l.widgets.borrow().iter() {
            let r = d.rect(&l.fixed, l.editing.get());
            let (x0, y0, x1, y1) = (r.x.max(bar.x), (r.y - CHROME_H).max(bar.y), (r.x + r.w).min(bar.x + bar.w), (r.y + r.h).min(bar.y + bar.h));
            if x1 > x0 && y1 > y0 {
                total += (x1 - x0) * (y1 - y0);
            }
        }
    }
    total
}

/// Zone utile d'un écran (sans la barre des tâches ni la barre d'état).
fn work_area(shell: &Shell, cfg: &NoosConfig, output: &str, w: f64, h: f64) -> Rect {
    let primary = shell.primary_output();
    let mut r = Rect::new(0.0, 0.0, w, h);
    if !(cfg.taskbar.all_outputs || primary.as_ref().is_none_or(|p| p == output)) {
        return r;
    }
    let mut cut = |edge: &str, th: f64| match edge {
        "top" => {
            r.y += th;
            r.h -= th;
        }
        "left" => {
            r.x += th;
            r.w -= th;
        }
        "right" => r.w -= th,
        _ => r.h -= th,
    };
    cut(crate::taskbar::bar_edge(cfg, false), crate::taskbar::bar_height(cfg, false) as f64);
    if cfg.topbar.enabled {
        cut(crate::taskbar::bar_edge(cfg, true), crate::taskbar::bar_height(cfg, true) as f64);
    }
    r
}

/// Ajoute un widget : à la position donnée (dépôt depuis la galerie) ou au premier endroit
/// libre de l'écran (`output`, sinon le principal).
pub fn add_widget(shell: &Rc<Shell>, kind: &str, output: Option<&str>, at: Option<(f64, f64)>) {
    let primary = shell.primary_output();
    let layers: Vec<DeskLayer> = shell.wallpapers.borrow().iter().map(|w| w.desk.clone()).collect();
    let layer = layers
        .iter()
        .find(|l| output.map_or_else(|| primary.as_deref().is_none_or(|p| p == l.output), |o| o == l.output))
        .or(layers.first())
        .cloned();
    let spec = widgets::spec(kind);
    let (mut fx, mut fy) = (0.06, 0.08);
    let mut out = String::new();
    if let Some(l) = &layer {
        let (sw, sh) = l.size();
        let cfg = shell.config();
        let area = SnapArea::new(sw, sh, work_area(shell, &cfg, &l.output, sw, sh));
        let editing = l.editing.get();
        // Rectangles avec leur barre de titre (mode édition) : rien ne doit la recouvrir.
        let mut others: Vec<Rect> = l
            .widgets
            .borrow()
            .iter()
            .map(|d| {
                let r = d.rect(&l.fixed, editing);
                Rect::new(r.x, r.y - CHROME_H, r.w, r.h + CHROME_H)
            })
            .collect();
        // Cadre « Les notifications apparaîtront ici » (coin choisi) : à éviter aussi.
        if editing && primary.as_deref().is_none_or(|p| p == l.output) {
            let c = cfg.notifications.position.as_str();
            let (nw, nh) = (330.0, 180.0);
            let nx = if c.ends_with("left") { 0.0 } else { sw - nw };
            let wk = work_area(shell, &cfg, &l.output, sw, sh);
            let ny = if c.starts_with("top") { wk.y } else { wk.y + wk.h - nh };
            others.push(Rect::new(nx, ny, nw, nh));
        }
        // Barre d'outils du mode édition (écran principal, en haut ou en bas) : à éviter aussi.
        if editing && primary.as_deref().is_none_or(|p| p == l.output) {
            let bottom = shell.editmode.borrow().as_ref().is_some_and(|e| e.at_bottom.get());
            let work = work_area(shell, &cfg, &l.output, sw, sh);
            let y = if bottom { work.y + work.h - 150.0 } else { work.y };
            others.push(Rect::new((sw - 980.0) / 2.0, y, 980.0, 150.0));
        }
        let (x, y) = match at {
            Some((x, y)) => {
                let r = Rect::new(x - spec.approx.0 / 2.0, y - 24.0, spec.approx.0, spec.approx.1);
                let (x, y, _) = layout::snap(r, &others, &area);
                (x, y)
            }
            None => {
                let (x, y) = layout::free_spot((spec.approx.0, spec.approx.1 + CHROME_H), &others, &area);
                (x, y + CHROME_H)
            }
        };
        fx = x / sw.max(1.0);
        fy = y / sh.max(1.0);
        if primary.as_deref().is_some_and(|p| p != l.output) {
            out = l.output.clone();
        }
    }
    let mut w = DesktopWidget::new(kind, fx, fy);
    w.output = out;
    widgets::defaults(&mut w);
    shell.edit_layout(&format!("« {} » ajouté", spec.name), move |c| c.desktop_widgets.push(w));
}

impl Inner {
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> DeskLayer {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-desktop-widgets"));
        window.set_layer(Layer::Bottom);
        window.set_monitor(Some(monitor));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        let overlay = gtk::Overlay::new();
        let guides = gtk::DrawingArea::new();
        guides.add_css_class("desk-guides");
        guides.set_can_target(false);
        guides.set_hexpand(true);
        guides.set_vexpand(true);
        overlay.set_child(Some(&guides));
        let fixed = gtk::Fixed::new();
        fixed.set_hexpand(true);
        fixed.set_vexpand(true);
        let screen_label = gtk::Label::new(None);
        screen_label.add_css_class("desk-screen-name");
        screen_label.set_halign(gtk::Align::Center);
        screen_label.set_valign(gtk::Align::End);
        screen_label.set_can_target(false);
        screen_label.set_visible(false);
        overlay.add_overlay(&screen_label);
        overlay.add_overlay(&fixed);
        window.set_child(Some(&overlay));
        let layer = Rc::new(Inner {
            window,
            fixed,
            guides,
            screen_label,
            widgets: RefCell::new(vec![]),
            editing: Cell::new(false),
            input: RefCell::new(vec![]),
            output: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            monitor: monitor.clone(),
            shell: Rc::downgrade(shell),
            lines: RefCell::new(vec![]),
            ghost: RefCell::new(None),
            drag: RefCell::new(None),
            built: Cell::new(false),
        });
        {
            let w = Rc::downgrade(&layer);
            layer.guides.set_draw_func(move |a, cr, wd, ht| {
                if let Some(l) = w.upgrade() {
                    l.draw(a, cr, wd as f64, ht as f64);
                }
            });
        }
        // Clavier du mode édition (le widget cliqué donne le clavier à cette surface).
        {
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let (w, win) = (Rc::downgrade(shell), layer.window.clone());
            keys.connect_key_pressed(move |_, k, _, m| {
                let Some(s) = w.upgrade() else { return glib::Propagation::Proceed };
                // Panneau de réglages ouvert (champ de recherche…) : les touches sont à lui.
                if settings_open(&s) {
                    return glib::Propagation::Proceed;
                }
                let e = s.editmode.borrow().clone();
                match e {
                    Some(e) => e.handle_key(&win, k, m),
                    None => glib::Propagation::Proceed,
                }
            });
            layer.window.add_controller(keys);
        }
        layer.setup_drag();
        layer.setup_drop();
        // Après chaque dessin (un widget a pu changer de taille) : zone de clic à jour.
        {
            let w = Rc::downgrade(&layer);
            layer.window.connect_realize(move |win| {
                let w = w.clone();
                if let Some(clock) = win.frame_clock() {
                    clock.connect_after_paint(move |_| {
                        if let Some(l) = w.upgrade() {
                            l.sync_input();
                        }
                    });
                }
            });
        }
        layer
    }

    fn size(&self) -> (f64, f64) {
        let g = self.monitor.geometry();
        (g.width() as f64, g.height() as f64)
    }

    /// Zone de clic : les widgets (le reste laisse passer les clics vers le fond d'écran), ou
    /// toute la surface en mode édition.
    fn sync_input(&self) {
        let desk = &self.window;
        let Some(surface) = desk.surface() else { return };
        let rects: Vec<(i32, i32, i32, i32)> = if self.editing.get() {
            vec![(0, 0, desk.width().max(1), desk.height().max(1))]
        } else {
            let mut v = vec![];
            let mut child = self.fixed.first_child();
            while let Some(c) = child {
                if c.is_visible() {
                    if let Some(b) = c.compute_bounds(desk) {
                        v.push((b.x().floor() as i32 - 4, b.y().floor() as i32 - 4, b.width().ceil() as i32 + 8, b.height().ceil() as i32 + 8));
                    }
                }
                child = c.next_sibling();
            }
            v
        };
        if *self.input.borrow() == rects {
            return;
        }
        let region = gtk::cairo::Region::create();
        for (x, y, w, h) in &rects {
            let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(*x, *y, *w, *h));
        }
        surface.set_input_region(Some(&region));
        *self.input.borrow_mut() = rects;
        desk.queue_draw();
    }

    fn place(&self, d: &DeskWidget) {
        let (sw, sh) = self.size();
        let c = d.conf.borrow();
        // Toujours entièrement à l'écran (`gtk::Fixed` donne aux enfants leur taille minimale,
        // connue avant l'allocation).
        let (min, _) = d.card.preferred_size();
        let (cw, ch) = (min.width().max(40) as f64, min.height().max(40) as f64);
        let top = if self.editing.get() { CHROME_H } else { 0.0 };
        let x = (c.x * sw).min(sw - cw).max(0.0);
        let y = (c.y * sh).min(sh - ch).max(top);
        let y = if self.editing.get() { y - CHROME_H } else { y };
        self.fixed.move_(&d.outer, x, y);
    }

    /// (Re)construit les widgets de cet écran d'après la configuration : seuls ceux qui ont
    /// changé sont refaits (un réglage modifié ne recrée que le contenu, dans le même cadre).
    pub fn sync(self: &Rc<Self>, shell: &Rc<Shell>, editing: bool) {
        let cfg = shell.config();
        let primary = shell.primary_output();
        let mine: Vec<DesktopWidget> = cfg
            .desktop_widgets
            .iter()
            .filter(|w| if w.output.is_empty() || !shell.outputs.borrow().iter().any(|o| o.name == w.output) { primary.as_ref().is_none_or(|p| *p == self.output) } else { w.output == self.output })
            .cloned()
            .collect();
        self.editing.set(editing);
        {
            let mut list = self.widgets.borrow_mut();
            list.retain(|d| {
                let keep = mine.iter().any(|w| w.id == d.id() && w.kind == d.conf.borrow().kind);
                if !keep {
                    d.settings.popdown();
                    self.fixed.remove(&d.outer);
                }
                keep
            });
        }
        for w in &mine {
            let existing = self.widgets.borrow().iter().find(|d| d.id() == w.id).cloned();
            match existing {
                Some(d) => {
                    let old = d.conf.replace(w.clone());
                    d.set_classes(w);
                    let text_changed = old.text != w.text && w.kind != "note";
                    if old.options != w.options || old.size() != w.size() || text_changed {
                        d.rebuild_content();
                    }
                    if d.settings.is_visible() && (old.options != w.options || old.size() != w.size() || old.style != w.style) {
                        d.fill_settings(shell);
                    }
                }
                None => {
                    let d = self.build_widget(shell, w);
                    if self.built.get() {
                        // Apparition animée (sauf au démarrage).
                        d.card.add_css_class("appear");
                        let c = d.card.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || c.remove_css_class("appear"));
                    }
                    self.widgets.borrow_mut().push(d);
                }
            }
        }
        for d in self.widgets.borrow().iter() {
            d.set_editing(editing);
            self.place(d);
        }
        self.built.set(true);
        let any = !self.widgets.borrow().is_empty();
        self.window.set_keyboard_mode(if any || editing { KeyboardMode::OnDemand } else { KeyboardMode::None });
        if editing {
            self.window.add_css_class("editing-desk");
        } else {
            self.window.remove_css_class("editing-desk");
            self.lines.borrow_mut().clear();
            *self.ghost.borrow_mut() = None;
        }
        // Nom de l'écran au bas de la zone utile, quand il y en a plusieurs.
        let outs = shell.outputs.borrow().clone();
        if editing && outs.len() > 1 {
            let i = outs.iter().position(|o| o.name == self.output).unwrap_or(0);
            let primary = outs.get(i).is_some_and(|o| o.primary);
            self.screen_label.set_text(&format!("Écran {} · {}{}", i + 1, self.output, if primary { " · principal" } else { "" }));
            let (sw, sh) = self.size();
            let work = work_area(shell, &cfg, &self.output, sw, sh);
            self.screen_label.set_margin_bottom((sh - work.y - work.h) as i32 + 28);
            self.screen_label.set_visible(true);
        } else {
            self.screen_label.set_visible(false);
        }
        // Pas de widget hors du mode édition : surface masquée (rien à composer).
        self.window.set_visible(any || editing);
        if any || editing {
            self.window.present();
        }
        self.input.borrow_mut().clear();
        self.sync_input();
        self.guides.queue_draw();
    }

    fn build_widget(self: &Rc<Self>, shell: &Rc<Shell>, w: &DesktopWidget) -> Rc<DeskWidget> {
        let spec = widgets::spec(&w.kind);
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        outer.add_css_class("desk-outer");
        // Barre de titre (mode édition), au-dessus de la carte.
        let chrome = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chrome.add_css_class("desk-chrome");
        chrome.set_size_request(-1, (CHROME_H - 6.0) as i32);
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        chip.add_css_class("desk-chip");
        chip.set_valign(gtk::Align::Center);
        chip.append(&gtk::Image::from_icon_name("list-drag-handle-symbolic"));
        // Nom raccourci si besoin : la barre de titre garde la largeur du widget.
        let chip_label = gtk::Label::new(Some(spec.name));
        chip_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chip.append(&chip_label);
        chip.set_tooltip_text(Some("Glisser pour déplacer (Alt : sans aimantation)"));
        chrome.append(&chip);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        chrome.append(&spacer);
        let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings_btn.add_css_class("desk-chip-btn");
        settings_btn.set_valign(gtk::Align::Center);
        settings_btn.set_tooltip_text(Some("Réglages du widget"));
        chrome.append(&settings_btn);
        let del = gtk::Button::from_icon_name("window-close-symbolic");
        del.add_css_class("desk-chip-btn");
        del.add_css_class("danger");
        del.set_valign(gtk::Align::Center);
        del.set_tooltip_text(Some("Retirer ce widget (Ctrl+Z pour annuler)"));
        chrome.append(&del);
        outer.append(&chrome);
        let overlay = gtk::Overlay::new();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("desk-card");
        card.add_css_class(&format!("kind-{}", w.kind));
        overlay.set_child(Some(&card));
        // Bouclier : en mode édition, il reçoit les clics (le contenu ne réagit plus, le geste
        // de déplacement du `gtk::Fixed` prend la main).
        let shield = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        shield.add_css_class("desk-shield");
        shield.set_visible(false);
        overlay.add_overlay(&shield);
        let resize = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        resize.add_css_class("desk-resize");
        resize.set_halign(gtk::Align::End);
        resize.set_valign(gtk::Align::End);
        resize.set_tooltip_text(Some("Glisser pour changer la taille"));
        let grip = gtk::DrawingArea::new();
        grip.set_content_width(20);
        grip.set_content_height(20);
        grip.set_draw_func(|a, cr, w, h| {
            let c = a.color();
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, 1.0);
            cr.set_line_width(1.6);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            let (w, h) = (w as f64, h as f64);
            // Deux traits obliques (poignée classique).
            for (x0, y0, x1, y1) in [(0.68, 0.34, 0.34, 0.68), (0.68, 0.52, 0.52, 0.68)] {
                cr.move_to(w * x0, h * y0);
                cr.line_to(w * x1, h * y1);
            }
            let _ = cr.stroke();
        });
        resize.append(&grip);
        resize.set_visible(false);
        overlay.add_overlay(&resize);
        outer.append(&overlay);
        let settings = gtk::Popover::new();
        settings.set_parent(&outer);
        // Barre de titre visible tant que les réglages sont ouverts.
        {
            let o = outer.clone();
            settings.connect_show(move |_| o.add_css_class("active"));
            let o = outer.clone();
            settings.connect_closed(move |_| o.remove_css_class("active"));
        }
        settings.set_position(gtk::PositionType::Right);
        let d = Rc::new(DeskWidget {
            conf: RefCell::new(w.clone()),
            outer,
            chrome,
            card,
            shield,
            resize,
            settings,
            settings_btn,
            ctx: widgets::Ctx { shell: Rc::downgrade(shell), id: w.id.clone() },
        });
        d.set_classes(w);
        d.rebuild_content();
        {
            let (dw, shell) = (Rc::downgrade(&d), Rc::downgrade(shell));
            d.settings_btn.connect_clicked(move |b| {
                if let (Some(d), Some(s)) = (dw.upgrade(), shell.upgrade()) {
                    d.open_settings(&s, Some(b.upcast_ref()));
                }
            });
        }
        {
            let (shell, id, name) = (Rc::downgrade(shell), w.id.clone(), spec.name);
            del.connect_clicked(move |_| {
                if let Some(s) = shell.upgrade() {
                    remove(&s, &id, name);
                }
            });
        }
        // Clic droit (hors mode édition aussi) : réglages, retirer, modifier le bureau.
        {
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let pop = gtk::Popover::new();
            pop.set_has_arrow(false);
            pop.set_child(Some(&menu));
            pop.set_parent(&d.outer);
            for (label, icon, act) in [("Réglages du widget", "emblem-system-symbolic", 0), ("Retirer ce widget", "user-trash-symbolic", 1), ("Modifier le bureau", "document-edit-symbolic", 2)] {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                c.append(&gtk::Image::from_icon_name(icon));
                c.append(&gtk::Label::new(Some(label)));
                b.set_child(Some(&c));
                let (shell, pop2, dw) = (Rc::downgrade(shell), pop.clone(), Rc::downgrade(&d));
                b.connect_clicked(move |_| {
                    pop2.popdown();
                    let (Some(s), Some(d)) = (shell.upgrade(), dw.upgrade()) else { return };
                    match act {
                        0 => {
                            let s2 = s.clone();
                            glib::idle_add_local_once(move || d.open_settings(&s2, None));
                        }
                        1 => remove(&s, &d.id(), d.name()),
                        _ => {
                            if let Some(e) = s.editmode.borrow().as_ref() {
                                e.set_active(true);
                            }
                        }
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
            d.outer.add_controller(click);
        }
        let (sw, sh) = self.size();
        self.fixed.put(&d.outer, w.x * sw, w.y * sh);
        d
    }

    /// Un seul geste pour tous les widgets, posé sur le `gtk::Fixed` (qui ne bouge pas) :
    /// posé sur l'élément déplacé, ses coordonnées le suivraient (clignotement).
    fn setup_drag(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        {
            let w = Rc::downgrade(self);
            drag.connect_drag_begin(move |g, px, py| {
                let Some(l) = w.upgrade() else { return };
                if !l.editing.get() {
                    g.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                let Some(picked) = l.fixed.pick(px, py, gtk::PickFlags::DEFAULT) else {
                    g.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                // Les boutons de la barre de titre gardent leur clic.
                if picked.is::<gtk::Button>() || picked.ancestor(gtk::Button::static_type()).is_some() {
                    g.set_state(gtk::EventSequenceState::Denied);
                    return;
                }
                // La zone vide d'une barre de titre (invisible hors survol) laisse passer vers le
                // widget dessous.
                let on_empty_chrome = l.widgets.borrow().iter().any(|d| picked == d.chrome);
                let found = if on_empty_chrome {
                    l.widgets.borrow().iter().find(|d| {
                        let r = d.rect(&l.fixed, true);
                        px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h
                    }).cloned()
                } else {
                    l.widgets.borrow().iter().find(|d| picked == d.outer || picked.is_ancestor(&d.outer)).cloned()
                };
                let Some(d) = found else {
                    g.set_state(gtk::EventSequenceState::Denied);
                    return;
                };
                let mode = if picked == d.resize || picked.is_ancestor(&d.resize) { Mode::Resize } else { Mode::Move };
                g.set_state(gtk::EventSequenceState::Claimed);
                let r = d.rect(&l.fixed, true);
                d.card.add_css_class("dragging");
                d.outer.add_css_class("active");
                if let Some(s) = l.shell.upgrade() {
                    if let Some(e) = s.editmode.borrow().as_ref() {
                        e.select(Some(d.id()), false);
                    }
                }
                *l.drag.borrow_mut() = Some(Drag { id: d.id(), mode, start: (r.x, r.y), size0: size_index(d.conf.borrow().size()) });
            });
        }
        {
            let w = Rc::downgrade(self);
            drag.connect_drag_update(move |g, dx, dy| {
                let Some(l) = w.upgrade() else { return };
                let Some((id, mode, start, size0)) = l.drag.borrow().as_ref().map(|d| (d.id.clone(), d.mode, d.start, d.size0)) else { return };
                let Some(d) = l.widgets.borrow().iter().find(|x| x.id() == id).cloned() else { return };
                match mode {
                    Mode::Move => {
                        let (sw, sh) = l.size();
                        let (cw, ch) = d.size();
                        let r = Rect::new(start.0 + dx, start.1 + dy, cw, ch);
                        let free = g.current_event_state().contains(gdk::ModifierType::ALT_MASK);
                        let (x, y, lines) = if free {
                            (r.x.min(sw - r.w).max(0.0), r.y.min(sh - r.h).max(0.0), vec![])
                        } else {
                            let others: Vec<Rect> = l.widgets.borrow().iter().filter(|o| o.id() != id).map(|o| o.rect(&l.fixed, true)).collect();
                            let area = l.snap_area();
                            layout::snap(r, &others, &area)
                        };
                        l.fixed.move_(&d.outer, x, y - CHROME_H);
                        *l.lines.borrow_mut() = lines;
                        l.guides.queue_draw();
                    }
                    Mode::Resize => {
                        let step = ((dx + dy) / 120.0).round() as i64;
                        let idx = (size0 as i64 + step).clamp(0, 2) as usize;
                        if d.conf.borrow().size() != SIZES[idx] {
                            d.conf.borrow_mut().size = SIZES[idx].into();
                            let c = d.conf.borrow().clone();
                            d.set_classes(&c);
                            d.rebuild_content();
                        }
                    }
                }
            });
        }
        {
            let w = Rc::downgrade(self);
            drag.connect_drag_end(move |_, _, _| {
                let Some(l) = w.upgrade() else { return };
                let Some(drag) = l.drag.borrow_mut().take() else { return };
                l.lines.borrow_mut().clear();
                l.guides.queue_draw();
                let Some(d) = l.widgets.borrow().iter().find(|x| x.id() == drag.id).cloned() else { return };
                d.card.remove_css_class("dragging");
                d.outer.remove_css_class("active");
                let Some(s) = l.shell.upgrade() else { return };
                let name = d.name();
                match drag.mode {
                    Mode::Move => {
                        let r = d.rect(&l.fixed, true);
                        if (r.x - drag.start.0).abs() < 1.0 && (r.y - drag.start.1).abs() < 1.0 {
                            return;
                        }
                        let (sw, sh) = l.size();
                        let (fx, fy) = (r.x / sw.max(1.0), r.y / sh.max(1.0));
                        let id = drag.id.clone();
                        s.edit_layout(&format!("« {name} » déplacé"), move |c| {
                            if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
                                w.x = fx;
                                w.y = fy;
                            }
                        });
                    }
                    Mode::Resize => {
                        let size = d.conf.borrow().size().to_string();
                        // La configuration a encore l'ancienne taille : on la remet pour que la
                        // synchronisation voie le changement… sans refaire le contenu.
                        let id = drag.id.clone();
                        let label = format!("« {name} » : taille {}", match size.as_str() {
                            "s" => "petite",
                            "l" => "grande",
                            _ => "moyenne",
                        });
                        s.edit_layout(&label, move |c| {
                            if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
                                w.size = size;
                            }
                        });
                    }
                }
            });
        }
        self.fixed.add_controller(drag);
    }

    fn snap_area(&self) -> SnapArea {
        let (sw, sh) = self.size();
        let work = match self.shell.upgrade() {
            Some(s) => work_area(&s, &s.config(), &self.output, sw, sh),
            None => Rect::new(0.0, 0.0, sw, sh),
        };
        SnapArea::new(sw, sh, work)
    }

    /// Cartes de la galerie déposées sur ce bureau.
    fn setup_drop(self: &Rc<Self>) {
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        {
            let w = Rc::downgrade(self);
            drop.connect_motion(move |_, x, y| {
                let Some(l) = w.upgrade() else { return gdk::DragAction::empty() };
                *l.ghost.borrow_mut() = Some(Rect::new(x, y, 0.0, 0.0));
                l.guides.queue_draw();
                gdk::DragAction::COPY
            });
        }
        {
            let w = Rc::downgrade(self);
            drop.connect_leave(move |_| {
                if let Some(l) = w.upgrade() {
                    *l.ghost.borrow_mut() = None;
                    l.guides.queue_draw();
                }
            });
        }
        {
            let w = Rc::downgrade(self);
            drop.connect_drop(move |_, value, x, y| {
                let Some(l) = w.upgrade() else { return false };
                *l.ghost.borrow_mut() = None;
                l.guides.queue_draw();
                let Ok(s) = value.get::<String>() else { return false };
                let Some(kind) = s.strip_prefix("noos-widget:") else { return false };
                let Some(shell) = l.shell.upgrade() else { return false };
                add_widget(&shell, kind, Some(&l.output), Some((x, y)));
                true
            });
        }
        self.fixed.add_controller(drop);
    }

    /// Mode édition : voile, grille de points, nom de l'écran, repères et emplacement du dépôt.
    fn draw(&self, a: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64) {
        if !self.editing.get() {
            return;
        }
        let accent = a.color();
        let (ar, ag, ab) = (accent.red() as f64, accent.green() as f64, accent.blue() as f64);
        cr.set_source_rgba(0.02, 0.03, 0.08, 0.30);
        cr.paint().ok();
        // Grille (un point toutes les deux cases de 16 px).
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.16);
        let mut y = 32.0;
        while y < h {
            let mut x = 32.0;
            while x < w {
                cr.rectangle(x - 1.0, y - 1.0, 2.0, 2.0);
                x += 32.0;
            }
            y += 32.0;
        }
        let _ = cr.fill();
        // Repères d'alignement.
        cr.set_source_rgba(ar, ag, ab, 0.95);
        cr.set_line_width(1.5);
        cr.set_dash(&[6.0, 4.0], 0.0);
        for g in self.lines.borrow().iter() {
            if g.vertical {
                cr.move_to(g.pos + 0.5, g.from);
                cr.line_to(g.pos + 0.5, g.to);
            } else {
                cr.move_to(g.from, g.pos + 0.5);
                cr.line_to(g.to, g.pos + 0.5);
            }
        }
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
        // Dépôt depuis la galerie : cercle sous le pointeur.
        if let Some(g) = *self.ghost.borrow() {
            cr.set_source_rgba(ar, ag, ab, 0.25);
            cr.arc(g.x, g.y, 34.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(ar, ag, ab, 0.9);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
        }
    }

    pub fn destroy(&self) {
        for d in self.widgets.borrow().iter() {
            d.settings.unparent();
        }
        self.window.destroy();
    }
}

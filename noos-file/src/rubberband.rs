//! Cadre de sélection « en direct », comme sous Windows : un élément est surligné dès qu'il
//! entre dans le cadre, et désélectionné si le cadre rétrécit. Le cadre intégré de GTK
//! (`set_enable_rubberband`) ne sélectionne qu'au relâchement : il est désactivé au profit
//! de celui-ci.
//!
//! Les rectangles des éléments sont relevés sur les cases affichées (marquées par
//! `tag_item`) et gardés en coordonnées du contenu (vue + défilement) : un élément sorti de
//! l'écran pendant le défilement automatique reste donc dans le calcul.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, glib, graphene};

/// Déplacement minimal (px) avant que le cadre n'apparaisse (en deçà : simple clic).
const THRESHOLD: f64 = 4.0;
/// Bande (px) en haut et en bas de la vue où le cadre fait défiler le contenu.
const EDGE: f64 = 28.0;
/// Vitesse maximale du défilement automatique (px par image).
const MAX_SPEED: f64 = 40.0;

#[derive(Default)]
struct Drag {
    /// Un glisser a commencé dans le vide de la vue.
    armed: bool,
    /// Le seuil est franchi : le cadre est affiché et la sélection suit.
    active: bool,
    /// Point de départ, coordonnées du contenu (y compris le défilement).
    anchor: (f64, f64),
    /// Point de départ et position du pointeur, coordonnées de la vue.
    origin: (f64, f64),
    pointer: (f64, f64),
    /// Sélection gardée en plus du cadre (glisser avec Ctrl ou Maj).
    keep: Option<gtk::Bitset>,
    /// Rectangle de chaque élément vu pendant le glisser, coordonnées du contenu.
    seen: HashMap<u32, graphene::Rect>,
    tick: Option<gtk::TickCallbackId>,
}

pub struct Rubberband {
    view: gtk::Widget,
    overlay: gtk::Overlay,
    band: gtk::Box,
    selection: gtk::MultiSelection,
    state: RefCell<Drag>,
}

/// Installe le cadre sur une vue (grille ou détails) ; il est dessiné dans `overlay`.
pub fn attach(view: &gtk::Widget, overlay: &gtk::Overlay, selection: &gtk::MultiSelection) -> Rc<Rubberband> {
    let band = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    band.add_css_class("noos-rubberband");
    band.set_halign(gtk::Align::Start);
    band.set_valign(gtk::Align::Start);
    band.set_can_target(false);
    band.set_visible(false);
    overlay.add_overlay(&band);

    let rb = Rc::new(Rubberband {
        view: view.clone(),
        overlay: overlay.clone(),
        band,
        selection: selection.clone(),
        state: RefCell::new(Drag::default()),
    });

    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    let r = rb.clone();
    drag.connect_drag_begin(move |g, x, y| {
        // Sur un élément : c'est un glisser-déposer de fichiers, pas un cadre.
        if crate::on_item(&r.view, x, y) || r.in_header(x, y) {
            return;
        }
        let keep = g.current_event_state().intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK);
        r.begin(x, y, keep);
    });
    let r = rb.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let (ox, oy) = r.state.borrow().origin;
        r.motion(ox + dx, oy + dy);
    });
    let r = rb.clone();
    drag.connect_drag_end(move |_, _, _| r.finish());
    let r = rb.clone();
    drag.connect_cancel(move |_, _| r.finish());
    view.add_controller(drag);

    // Test sans souris : NOOS_FILE_RUBBER_DEMO=x0,y0,x1,y1 (vue en icônes, cadre laissé affiché).
    if let Ok(spec) = std::env::var("NOOS_FILE_RUBBER_DEMO") {
        if view.css_name().as_str() == "gridview" {
            let v: Vec<f64> = spec.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            if let [x0, y0, x1, y1] = v[..] {
                let r = rb.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || {
                    r.begin(x0, y0, false);
                    r.motion(x1, y1);
                    eprintln!("noos-file : démo cadre ({x0}, {y0}) → ({x1}, {y1}) : {} élément(s)", r.selection.selection().size());
                });
            }
        }
    }
    rb
}

impl Rubberband {
    fn begin(&self, x: f64, y: f64, keep: bool) {
        let mut s = self.state.borrow_mut();
        if let Some(t) = s.tick.take() {
            t.remove();
        }
        *s = Drag {
            armed: true,
            anchor: (x, y + self.scroll()),
            origin: (x, y),
            pointer: (x, y),
            keep: keep.then(|| self.selection.selection()),
            ..Default::default()
        };
    }

    fn motion(self: &Rc<Self>, x: f64, y: f64) {
        {
            let mut s = self.state.borrow_mut();
            if !s.armed {
                return;
            }
            s.pointer = (x, y);
            if !s.active {
                if (x - s.origin.0).abs() < THRESHOLD && (y - s.origin.1).abs() < THRESHOLD {
                    return;
                }
                s.active = true;
                self.band.set_visible(true);
                let r = self.clone();
                s.tick = Some(self.view.add_tick_callback(move |_, _| {
                    r.autoscroll();
                    glib::ControlFlow::Continue
                }));
            }
        }
        self.refresh();
    }

    fn finish(&self) {
        let mut s = self.state.borrow_mut();
        if let Some(t) = s.tick.take() {
            t.remove();
        }
        s.armed = false;
        s.active = false;
        s.keep = None;
        s.seen.clear();
        self.band.set_visible(false);
    }

    /// Pointeur près du haut ou du bas (ou au-delà) : défile, puis recalcule le cadre.
    fn autoscroll(&self) {
        let Some(adj) = self.vadjustment() else { return };
        let y = self.state.borrow().pointer.1;
        let h = self.view.height() as f64;
        let speed = if y < EDGE {
            -(EDGE - y)
        } else if y > h - EDGE {
            y - (h - EDGE)
        } else {
            return;
        };
        let speed = speed.clamp(-MAX_SPEED, MAX_SPEED) * 0.5;
        let max = (adj.upper() - adj.page_size()).max(adj.lower());
        let value = (adj.value() + speed).clamp(adj.lower(), max);
        if (value - adj.value()).abs() > f64::EPSILON {
            adj.set_value(value);
            self.refresh();
        }
    }

    /// Relève les éléments affichés, applique la sélection et redessine le cadre.
    fn refresh(&self) {
        let off = self.scroll();
        let (w, h) = (self.view.width() as f64, self.view.height() as f64);
        let mut s = self.state.borrow_mut();

        // Rectangles des éléments affichés (une ligne de la vue Détails = toutes ses cellules).
        let mut now: HashMap<u32, graphene::Rect> = HashMap::new();
        collect(&self.view, &self.view, off, &mut now);
        s.seen.extend(now);

        // Cadre, coordonnées du contenu ; le pointeur est ramené dans la vue.
        let (ax, ay) = s.anchor;
        let (px, py) = (s.pointer.0.clamp(0.0, w), s.pointer.1.clamp(0.0, h) + off);
        let (x0, x1, y0, y1) = (ax.min(px), ax.max(px), ay.min(py), ay.max(py));

        let n = self.selection.n_items();
        let wanted = s.keep.as_ref().map_or_else(gtk::Bitset::new_empty, |b| b.copy());
        for (&pos, r) in &s.seen {
            let (rx, ry) = (r.x() as f64, r.y() as f64);
            let (rw, rh) = (r.width() as f64, r.height() as f64);
            if pos < n && rx <= x1 && rx + rw >= x0 && ry <= y1 && ry + rh >= y0 {
                wanted.add(pos);
            }
        }
        drop(s);
        self.selection.set_selection(&wanted, &gtk::Bitset::new_range(0, n));

        // Partie visible du cadre, dans l'overlay.
        let (top, bottom) = ((y0 - off).max(0.0), (y1 - off).min(h));
        let Some(p) = self.view.compute_point(&self.overlay, &graphene::Point::new(x0 as f32, top as f32)) else { return };
        self.band.set_margin_start(p.x().max(0.0) as i32);
        self.band.set_margin_top(p.y().max(0.0) as i32);
        self.band.set_size_request(((x1 - x0) as i32).max(1), ((bottom - top) as i32).max(1));
    }

    fn vadjustment(&self) -> Option<gtk::Adjustment> {
        self.view.dynamic_cast_ref::<gtk::Scrollable>().and_then(|s| s.vadjustment())
    }

    fn scroll(&self) -> f64 {
        self.vadjustment().map_or(0.0, |a| a.value())
    }

    /// En-tête des colonnes (vue Détails) : ne démarre pas de cadre.
    fn in_header(&self, x: f64, y: f64) -> bool {
        let mut w = self.view.pick(x, y, gtk::PickFlags::DEFAULT);
        while let Some(cur) = w {
            if cur == self.view {
                return false;
            }
            if cur.css_name().as_str() == "header" {
                return true;
            }
            w = cur.parent();
        }
        false
    }
}

/// Rectangles (coordonnées du contenu) des cases affichées sous `w`.
fn collect(view: &gtk::Widget, w: &gtk::Widget, off: f64, out: &mut HashMap<u32, graphene::Rect>) {
    let mut child = w.first_child();
    while let Some(c) = child {
        if !c.is_mapped() {
            // Case recyclée ou masquée : ignorée.
        } else if let Some(pos) = crate::item_position(&c) {
            if let Some(b) = c.compute_bounds(view) {
                let r = graphene::Rect::new(b.x(), b.y() + off as f32, b.width(), b.height());
                out.entry(pos).and_modify(|e| *e = e.union(&r)).or_insert(r);
            }
        } else {
            collect(view, &c, off, out);
        }
        child = c.next_sibling();
    }
}

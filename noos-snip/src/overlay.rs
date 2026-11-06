//! Sélection de la zone à capturer : image figée de l'écran, assombrie, avec la barre
//! d'outils de Windows (Rectangle, Fenêtre, Plein écran, Forme libre).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{cairo, gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::ipc::{Event, IpcClient, Request};

use crate::shot::Shot;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Rect,
    Window,
    Full,
    Free,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rect" | "rectangle" => Some(Self::Rect),
            "window" | "fenetre" => Some(Self::Window),
            "full" | "screen" => Some(Self::Full),
            "free" | "freeform" => Some(Self::Free),
            _ => None,
        }
    }
}

/// Rectangle en coordonnées logiques globales.
type Rect = [f64; 4];

struct Monitor {
    geo: Rect,
    area: gtk::DrawingArea,
    window: gtk::Window,
}

pub struct Overlay {
    shot: Shot,
    /// Origine du bureau (coordonnées logiques) et échelle image / logique.
    origin: (f64, f64),
    scale: f64,
    mode: Cell<Mode>,
    monitors: RefCell<Vec<Monitor>>,
    windows: Vec<Rect>,
    drag_start: Cell<Option<(f64, f64)>>,
    rect: Cell<Option<Rect>>,
    path: RefCell<Vec<(f64, f64)>>,
    hover: Cell<Option<Rect>>,
    done: RefCell<Option<Box<dyn FnOnce(Option<Shot>)>>>,
    mode_buttons: RefCell<Vec<(Mode, gtk::ToggleButton)>>,
}

fn normalize(a: (f64, f64), b: (f64, f64)) -> Rect {
    [a.0.min(b.0), a.1.min(b.1), (a.0 - b.0).abs(), (a.1 - b.1).abs()]
}

fn contains(r: &Rect, p: (f64, f64)) -> bool {
    p.0 >= r[0] && p.1 >= r[1] && p.0 < r[0] + r[2] && p.1 < r[1] + r[3]
}

impl Overlay {
    pub fn start(app: &adap::Application, shot: Shot, mode: Mode, done: impl FnOnce(Option<Shot>) + 'static) -> Rc<Self> {
        let display = gdk::Display::default().unwrap();
        let monitors: Vec<gdk::Monitor> =
            (0..display.monitors().n_items()).filter_map(|i| display.monitors().item(i).and_downcast()).collect();
        let geos: Vec<Rect> = monitors
            .iter()
            .map(|m| {
                let g = m.geometry();
                [g.x() as f64, g.y() as f64, g.width() as f64, g.height() as f64]
            })
            .collect();
        let min_x = geos.iter().map(|g| g[0]).fold(f64::MAX, f64::min);
        let min_y = geos.iter().map(|g| g[1]).fold(f64::MAX, f64::min);
        let max_x = geos.iter().map(|g| g[0] + g[2]).fold(f64::MIN, f64::max);
        let scale = if max_x > min_x { shot.w as f64 / (max_x - min_x) } else { 1.0 };

        // Fenêtres visibles (pour le mode Fenêtre).
        let windows = match IpcClient::request(&Request::GetWindows) {
            Ok(Event::Windows { windows }) => windows
                .iter()
                .filter(|w| !w.minimized)
                .filter_map(|w| w.geometry)
                .map(|g| [g[0] as f64, g[1] as f64, g[2] as f64, g[3] as f64])
                .collect(),
            _ => Vec::new(),
        };

        let ov = Rc::new(Self {
            shot,
            origin: (min_x, min_y),
            scale,
            mode: Cell::new(mode),
            monitors: RefCell::new(Vec::new()),
            windows,
            drag_start: Cell::new(None),
            rect: Cell::new(None),
            path: RefCell::new(Vec::new()),
            hover: Cell::new(None),
            done: RefCell::new(Some(Box::new(done))),
            mode_buttons: RefCell::new(Vec::new()),
        });

        for (i, (monitor, geo)) in monitors.iter().zip(geos).enumerate() {
            let m = ov.monitor_window(app, monitor, geo, i == 0);
            ov.monitors.borrow_mut().push(m);
        }
        for m in ov.monitors.borrow().iter() {
            m.window.present();
        }
        ov
    }

    fn monitor_window(self: &Rc<Self>, app: &adap::Application, monitor: &gdk::Monitor, geo: Rect, toolbar: bool) -> Monitor {
        let window = gtk::Window::builder().application(app).decorated(false).build();
        window.add_css_class("snip-overlay");
        window.init_layer_shell();
        window.set_namespace(Some("noos-snip"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(Some(monitor));
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        // Image figée de cet écran.
        let (ix, iy) = ((geo[0] - self.origin.0) * self.scale, (geo[1] - self.origin.1) * self.scale);
        let part = self.shot.crop(ix.round() as i32, iy.round() as i32, (geo[2] * self.scale).round() as i32, (geo[3] * self.scale).round() as i32);
        let picture = gtk::Picture::for_paintable(&part.texture());
        picture.set_content_fit(gtk::ContentFit::Fill);
        picture.set_can_shrink(true);

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_cursor_from_name(Some("crosshair"));
        {
            let ov = self.clone();
            area.set_draw_func(move |_, cr, w, h| ov.draw(cr, geo, w, h));
        }

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.add_overlay(&area);
        if toolbar {
            overlay.add_overlay(&self.toolbar());
        }
        window.set_child(Some(&overlay));

        // Glisser : rectangle ou forme libre ; clic : fenêtre ou écran.
        let drag = gtk::GestureDrag::new();
        {
            let ov = self.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let p = (geo[0] + x, geo[1] + y);
                ov.drag_start.set(Some(p));
                ov.path.borrow_mut().clear();
                ov.path.borrow_mut().push(p);
                ov.rect.set(None);
                ov.redraw();
            });
        }
        {
            let ov = self.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                let Some(start) = ov.drag_start.get() else { return };
                let p = (start.0 + dx, start.1 + dy);
                match ov.mode.get() {
                    Mode::Rect => ov.rect.set(Some(normalize(start, p))),
                    Mode::Free => ov.path.borrow_mut().push(p),
                    _ => {}
                }
                ov.redraw();
            });
        }
        {
            let ov = self.clone();
            drag.connect_drag_end(move |_, dx, dy| {
                let Some(start) = ov.drag_start.take() else { return };
                let end = (start.0 + dx, start.1 + dy);
                match ov.mode.get() {
                    Mode::Rect => {
                        let r = normalize(start, end);
                        if r[2] >= 4.0 && r[3] >= 4.0 {
                            ov.finish(Some(r), None);
                        } else {
                            ov.rect.set(None);
                            ov.redraw();
                        }
                    }
                    Mode::Free => {
                        let pts = ov.path.borrow().clone();
                        if pts.len() > 3 {
                            ov.finish(None, Some(pts));
                        }
                    }
                    Mode::Window | Mode::Full => {
                        if let Some(r) = ov.target_at(end) {
                            ov.finish(Some(r), None);
                        }
                    }
                }
            });
        }
        area.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        {
            let ov = self.clone();
            motion.connect_motion(move |_, x, y| {
                let target = ov.target_at((geo[0] + x, geo[1] + y));
                if target != ov.hover.get() {
                    ov.hover.set(target);
                    ov.redraw();
                }
            });
        }
        area.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        {
            let ov = self.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    ov.finish(None, None);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);

        Monitor { geo, area, window }
    }

    fn toolbar(self: &Rc<Self>) -> gtk::Widget {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        bar.add_css_class("snip-toolbar");
        bar.set_halign(gtk::Align::Center);
        bar.set_valign(gtk::Align::Start);
        bar.set_margin_top(16);
        let mut group: Option<gtk::ToggleButton> = None;
        for (mode, icon, label) in [
            (Mode::Rect, "edit-select-all-symbolic", "Rectangle"),
            (Mode::Window, "focus-windows-symbolic", "Fenêtre"),
            (Mode::Full, "view-fullscreen-symbolic", "Plein écran"),
            (Mode::Free, "document-edit-symbolic", "Forme libre"),
        ] {
            let b = gtk::ToggleButton::new();
            b.set_child(Some(&adap::ButtonContent::builder().icon_name(icon).label(label).build()));
            b.set_tooltip_text(Some(label));
            b.set_group(group.as_ref());
            if group.is_none() {
                group = Some(b.clone());
            }
            b.set_active(mode == self.mode.get());
            let ov = self.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    ov.mode.set(mode);
                    ov.rect.set(None);
                    ov.path.borrow_mut().clear();
                    ov.hover.set(None);
                    ov.redraw();
                }
            });
            self.mode_buttons.borrow_mut().push((mode, b.clone()));
            bar.append(&b);
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some("Annuler (Échap)"));
        {
            let ov = self.clone();
            close.connect_clicked(move |_| ov.finish(None, None));
        }
        bar.append(&close);
        bar.upcast()
    }

    /// Fenêtre (la plus petite contenant le point) ou écran sous le pointeur.
    fn target_at(&self, p: (f64, f64)) -> Option<Rect> {
        match self.mode.get() {
            Mode::Window => self
                .windows
                .iter()
                .filter(|r| contains(r, p))
                .min_by(|a, b| (a[2] * a[3]).total_cmp(&(b[2] * b[3])))
                .copied(),
            Mode::Full => self.monitors.borrow().iter().map(|m| m.geo).find(|g| contains(g, p)),
            _ => None,
        }
    }

    fn redraw(&self) {
        for m in self.monitors.borrow().iter() {
            m.area.queue_draw();
        }
    }

    fn draw(&self, cr: &cairo::Context, geo: Rect, w: i32, h: i32) {
        cr.translate(-geo[0], -geo[1]);
        let highlight = match self.mode.get() {
            Mode::Rect => self.rect.get(),
            Mode::Window | Mode::Full => self.hover.get(),
            Mode::Free => None,
        };
        // Voile sombre, sauf sur la zone choisie.
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(geo[0], geo[1], w as f64, h as f64);
        let path = self.path.borrow();
        if let Some(r) = highlight {
            cr.rectangle(r[0], r[1], r[2], r[3]);
        } else if self.mode.get() == Mode::Free && path.len() > 2 {
            cr.move_to(path[0].0, path[0].1);
            for p in path.iter().skip(1) {
                cr.line_to(p.0, p.1);
            }
            cr.close_path();
        }
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
        let _ = cr.fill();

        // Contour.
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
        cr.set_line_width(1.5);
        if let Some(r) = highlight {
            cr.rectangle(r[0] + 0.75, r[1] + 0.75, r[2] - 1.5, r[3] - 1.5);
            let _ = cr.stroke();
            if self.mode.get() == Mode::Rect {
                // Taille en pixels réels, comme l'outil de Windows.
                let label = format!("{} × {}", (r[2] * self.scale).round(), (r[3] * self.scale).round());
                cr.set_font_size(12.0);
                cr.move_to(r[0] + 6.0, (r[1] - 8.0).max(geo[1] + 14.0));
                let _ = cr.show_text(&label);
            }
        } else if path.len() > 1 {
            cr.move_to(path[0].0, path[0].1);
            for p in path.iter().skip(1) {
                cr.line_to(p.0, p.1);
            }
            let _ = cr.stroke();
        }
    }

    fn finish(self: &Rc<Self>, rect: Option<Rect>, free: Option<Vec<(f64, f64)>>) {
        let to_img = |x: f64, y: f64| ((x - self.origin.0) * self.scale, (y - self.origin.1) * self.scale);
        let result = if let Some(r) = rect {
            let (x, y) = to_img(r[0], r[1]);
            Some(self.shot.crop(x.round() as i32, y.round() as i32, (r[2] * self.scale).round() as i32, (r[3] * self.scale).round() as i32))
        } else if let Some(points) = free {
            // Boîte englobante, puis masque transparent hors du tracé.
            let pts: Vec<(f64, f64)> = points.iter().map(|p| to_img(p.0, p.1)).collect();
            let min_x = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min).floor();
            let min_y = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min).floor();
            let max_x = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max).ceil();
            let max_y = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max).ceil();
            let crop = self.shot.crop(min_x as i32, min_y as i32, (max_x - min_x) as i32, (max_y - min_y) as i32);
            let out = cairo::ImageSurface::create(cairo::Format::ARgb32, crop.w, crop.h).expect("surface");
            {
                let cr = cairo::Context::new(&out).expect("contexte cairo");
                cr.move_to(pts[0].0 - min_x, pts[0].1 - min_y);
                for p in pts.iter().skip(1) {
                    cr.line_to(p.0 - min_x, p.1 - min_y);
                }
                cr.close_path();
                cr.clip();
                let _ = cr.set_source_surface(crop.surface(), 0.0, 0.0);
                let _ = cr.paint();
            }
            Some(Shot::from_surface(out))
        } else {
            None
        };
        if let Some(done) = self.done.borrow_mut().take() {
            done(result);
        }
        for m in self.monitors.borrow_mut().drain(..) {
            m.window.destroy();
        }
    }
}

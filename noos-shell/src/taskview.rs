//! Vue des tâches (Super+Tab, bouton de la barre) : fenêtres du bureau actif en grandes
//! cartes, et bande des bureaux virtuels en bas (miniatures dessinées : fond d'écran et
//! emplacement des fenêtres). Cliquer un bureau y va ; glisser une fenêtre sur un bureau l'y
//! envoie ; « + » crée un bureau ; double-clic sur un nom le renomme ; « × » le ferme.
//! Aussi : l'indicateur affiché brièvement au changement de bureau.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use adap::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::ipc::{IpcClient, Request, WindowInfo};

use crate::{Shell, apps};

const CSS: &str = r#"
.taskview { background-color: rgba(8, 10, 18, 0.80); }
.taskview .tv-title { color: white; font-size: 1.5em; font-weight: 700; }
.taskview .tv-sub { color: rgba(255,255,255,0.7); }
.taskview .tv-window { border-radius: 14px; padding: 18px 14px 14px 14px; min-width: 180px;
  background-color: alpha(@window_bg_color, 0.88); border: 1px solid alpha(white, 0.08); }
.taskview .tv-window:hover { background-color: alpha(@window_bg_color, 0.98); box-shadow: 0 0 0 2px @accent_bg_color; }
.taskview .tv-window.minimized { opacity: 0.6; }
.taskview .tv-window label { color: @window_fg_color; }
.taskview .tv-close { min-width: 26px; min-height: 26px; padding: 0; border-radius: 99px; background-color: rgba(0,0,0,0.65); color: white; }
.taskview .tv-strip { padding: 16px 24px; border-radius: 18px; background-color: alpha(black, 0.35); }
.taskview .tv-desk { padding: 6px; border-radius: 12px; background: transparent; }
.taskview .tv-desk:hover { background-color: alpha(white, 0.08); }
.taskview .tv-desk.current .tv-thumb { box-shadow: 0 0 0 3px @accent_bg_color; }
.taskview .tv-thumb { border-radius: 10px; }
.taskview .tv-desk-name { color: white; font-weight: 600; }
.taskview .tv-new { border-radius: 12px; min-width: 120px; color: white; background-color: alpha(white, 0.08); }
.taskview .tv-new:hover { background-color: alpha(white, 0.16); }
.desk-osd { padding: 12px 22px; border-radius: 16px; background-color: alpha(@window_bg_color, 0.92);
  border: 1px solid alpha(@window_fg_color, 0.1); box-shadow: 0 8px 28px rgba(0,0,0,0.35); }
.desk-osd .osd-name { font-weight: 700; font-size: 1.15em; }
.desk-osd .osd-dot { min-width: 8px; min-height: 8px; border-radius: 99px; background-color: alpha(@window_fg_color, 0.3); }
.desk-osd .osd-dot.on { background-color: @accent_bg_color; min-width: 22px; }
"#;

const THUMB_H: i32 = 118;

pub struct TaskView {
    shell: Weak<Shell>,
    window: gtk::Window,
    root: gtk::Box,
    wallpaper: RefCell<Option<gtk::cairo::ImageSurface>>,
    osd: gtk::Window,
    osd_box: gtk::Box,
    osd_gen: Cell<u32>,
}

/// Fenêtres avec position (y compris celles des autres bureaux), lues au compositeur.
fn windows_with_geometry() -> Vec<WindowInfo> {
    match IpcClient::request(&Request::GetWindows) {
        Ok(noos_core::ipc::Event::Windows { windows }) => windows,
        _ => vec![],
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

impl TaskView {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-taskview"));
        window.set_layer(Layer::Overlay);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("taskview");
        window.set_child(Some(&root));

        let osd = gtk::Window::builder().application(&shell.app).decorated(false).build();
        osd.add_css_class("noos-layer");
        osd.init_layer_shell();
        osd.set_namespace(Some("noos-desktop-osd"));
        osd.set_layer(Layer::Overlay);
        osd.set_anchor(Edge::Top, true);
        osd.set_margin(Edge::Top, 72);
        osd.set_keyboard_mode(KeyboardMode::None);
        let osd_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        osd_box.add_css_class("desk-osd");
        osd.set_child(Some(&osd_box));

        let tv = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            root,
            wallpaper: RefCell::new(None),
            osd,
            osd_box,
            osd_gen: Cell::new(0),
        });
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(&tv);
            keys.connect_key_pressed(move |_, key, _, mods| {
                let Some(tv) = w.upgrade() else { return glib::Propagation::Proceed };
                match key {
                    gdk::Key::Escape => tv.hide(),
                    gdk::Key::Tab if mods.contains(gdk::ModifierType::SUPER_MASK) => tv.hide(),
                    gdk::Key::Left | gdk::Key::Right => {
                        let delta = if key == gdk::Key::Right { 1 } else { -1 };
                        apps::request(Request::SwitchDesktopRelative { delta });
                    }
                    _ => return glib::Propagation::Proceed,
                }
                glib::Propagation::Stop
            });
            tv.window.add_controller(keys);
        }
        // Clic dans le vide : fermer.
        {
            let click = gtk::GestureClick::new();
            let w = Rc::downgrade(&tv);
            click.connect_released(move |g, _, x, y| {
                let Some(tv) = w.upgrade() else { return };
                if let Some(target) = g.widget().and_then(|wd| wd.pick(x, y, gtk::PickFlags::DEFAULT)) {
                    if target == tv.root.clone().upcast::<gtk::Widget>() {
                        tv.hide();
                    }
                }
            });
            tv.root.add_controller(click);
        }
        if std::env::var_os("NOOS_TASKVIEW_DEMO").is_some() {
            let w = Rc::downgrade(&tv);
            glib::timeout_add_local_once(Duration::from_millis(2500), move || {
                if let Some(tv) = w.upgrade() {
                    tv.toggle();
                }
            });
        }
        tv
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.hide();
        } else {
            self.fill();
            self.window.present();
        }
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn refresh_if_open(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.fill();
        }
    }

    /// Fond d'écran réduit (miniatures), converti une fois en surface cairo.
    fn wallpaper(&self) -> Option<gtk::cairo::ImageSurface> {
        if self.wallpaper.borrow().is_none() {
            let cfg = self.shell.upgrade()?.config();
            let p = crate::wallpaper::wallpaper_path(&cfg)?;
            let t = gdk::Texture::from_filename(&p).ok()?;
            let (tw, th) = (t.width(), t.height());
            let mut data = vec![0u8; (tw * th * 4) as usize];
            t.download(&mut data, tw as usize * 4);
            let full = gtk::cairo::ImageSurface::create_for_data(data, gtk::cairo::Format::ARgb32, tw, th, tw * 4).ok()?;
            // Version réduite (640 px de large) : dessin rapide des miniatures.
            let k = 640.0 / tw.max(1) as f64;
            let (w, h) = (640, ((th as f64 * k) as i32).max(1));
            let small = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, w, h).ok()?;
            {
                let cr = gtk::cairo::Context::new(&small).ok()?;
                cr.scale(k, k);
                cr.set_source_surface(&full, 0.0, 0.0).ok()?;
                cr.paint().ok()?;
            }
            *self.wallpaper.borrow_mut() = Some(small);
        }
        self.wallpaper.borrow().clone()
    }

    /// Reconstruit le contenu (fenêtres lues au compositeur dans un fil à part).
    fn fill(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(windows_with_geometry());
        });
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let windows = rx.recv().await.unwrap_or_default();
            if let Some(tv) = w.upgrade() {
                tv.build(windows);
            }
        });
    }

    fn build(self: &Rc<Self>, windows: Vec<WindowInfo>) {
        let Some(shell) = self.shell.upgrade() else { return };
        while let Some(c) = self.root.first_child() {
            self.root.remove(&c);
        }
        let (names, current) = shell.desktops.borrow().clone();
        let outputs = shell.outputs.borrow().clone();

        // ── Fenêtres du bureau actif ──
        let top = gtk::Box::new(gtk::Orientation::Vertical, 14);
        top.set_vexpand(true);
        top.set_valign(gtk::Align::Center);
        top.set_halign(gtk::Align::Center);
        top.set_margin_top(48);
        let title = gtk::Label::new(Some(names.get(current).map(|s| s.as_str()).unwrap_or("Bureau")));
        title.add_css_class("tv-title");
        top.append(&title);
        let here: Vec<&WindowInfo> = windows.iter().filter(|w| w.desktop == current || w.sticky).collect();
        let sub = gtk::Label::new(Some(&match here.len() {
            0 => "Aucune fenêtre sur ce bureau".to_string(),
            1 => "1 fenêtre · glissez-la sur un bureau pour l'y envoyer".to_string(),
            n => format!("{n} fenêtres · glissez-en une sur un bureau pour l'y envoyer"),
        }));
        sub.add_css_class("tv-sub");
        top.append(&sub);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_max_children_per_line(5);
        flow.set_column_spacing(16);
        flow.set_row_spacing(16);
        flow.set_halign(gtk::Align::Center);
        flow.set_margin_top(12);
        for w in here {
            flow.append(&self.window_card(w));
        }
        top.append(&flow);
        self.root.append(&top);

        // ── Bande des bureaux ──
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        strip.add_css_class("tv-strip");
        strip.set_halign(gtk::Align::Center);
        strip.set_margin_bottom(64);
        // Rectangle englobant les écrans (pour placer les fenêtres dans les miniatures).
        let (bx, by, bw, bh) = outputs.iter().fold((i32::MAX, i32::MAX, i32::MIN, i32::MIN), |(x0, y0, x1, y1), o| {
            (x0.min(o.x), y0.min(o.y), x1.max(o.x + o.logical_width.max(1)), y1.max(o.y + o.logical_height.max(1)))
        });
        let bounds = if outputs.is_empty() { (0, 0, 1920, 1080) } else { (bx, by, (bw - bx).max(1), (bh - by).max(1)) };
        let thumb_w = (THUMB_H as f64 * bounds.2 as f64 / bounds.3 as f64).round() as i32;
        for (i, name) in names.iter().enumerate() {
            let rects: Vec<[i32; 4]> = windows.iter().filter(|w| (w.desktop == i || w.sticky) && !w.minimized).filter_map(|w| w.geometry).collect();
            strip.append(&self.desktop_card(i, name, i == current, names.len() > 1, rects, bounds, &outputs, thumb_w));
        }
        let add = gtk::Button::new();
        add.add_css_class("tv-new");
        let ab = gtk::Box::new(gtk::Orientation::Vertical, 6);
        ab.set_valign(gtk::Align::Center);
        let plus = gtk::Image::from_icon_name("list-add-symbolic");
        plus.set_pixel_size(28);
        ab.append(&plus);
        ab.append(&gtk::Label::new(Some("Nouveau bureau")));
        add.set_child(Some(&ab));
        add.set_size_request(thumb_w.min(200), THUMB_H);
        add.set_valign(gtk::Align::Start);
        add.connect_clicked(|_| apps::request(Request::AddDesktop { name: None, switch: false }));
        strip.append(&add);
        self.root.append(&strip);
    }

    fn window_card(self: &Rc<Self>, w: &WindowInfo) -> gtk::Widget {
        let overlay = gtk::Overlay::new();
        let b = gtk::Button::new();
        b.add_css_class("tv-window");
        if w.minimized {
            b.add_css_class("minimized");
        }
        let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let icon = gtk::Image::from_gicon(&apps::icon(&w.app_id));
        icon.set_pixel_size(64);
        v.append(&icon);
        let l = gtk::Label::new(Some(if w.title.is_empty() { &w.app_id } else { &w.title }));
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        l.set_max_width_chars(22);
        v.append(&l);
        b.set_child(Some(&v));
        let (id, me) = (w.id, Rc::downgrade(self));
        b.connect_clicked(move |_| {
            apps::request(Request::Activate { id });
            if let Some(tv) = me.upgrade() {
                tv.hide();
            }
        });
        // Glisser vers un bureau.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::MOVE);
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&format!("noos-window:{id}").to_value())));
        {
            let icon = icon.clone();
            drag.connect_drag_begin(move |src, _| src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&icon))), 32, 32));
        }
        b.add_controller(drag);
        overlay.set_child(Some(&b));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("tv-close");
        close.add_css_class("circular");
        close.set_halign(gtk::Align::End);
        close.set_valign(gtk::Align::Start);
        close.set_margin_top(6);
        close.set_margin_end(6);
        close.set_tooltip_text(Some("Fermer la fenêtre"));
        close.connect_clicked(move |_| apps::request(Request::Close { id }));
        overlay.add_overlay(&close);
        overlay.upcast()
    }

    #[allow(clippy::too_many_arguments)]
    fn desktop_card(
        self: &Rc<Self>,
        index: usize,
        name: &str,
        current: bool,
        closable: bool,
        rects: Vec<[i32; 4]>,
        bounds: (i32, i32, i32, i32),
        outputs: &[noos_core::ipc::OutputInfo],
        thumb_w: i32,
    ) -> gtk::Widget {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let b = gtk::Button::new();
        b.add_css_class("tv-desk");
        if current {
            b.add_css_class("current");
        }
        let area = gtk::DrawingArea::new();
        area.add_css_class("tv-thumb");
        area.set_content_width(thumb_w);
        area.set_content_height(THUMB_H);
        let wall = self.wallpaper();
        let outs: Vec<[i32; 4]> = outputs.iter().map(|o| [o.x, o.y, o.logical_width, o.logical_height]).collect();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let sx = w / bounds.2 as f64;
            let sy = h / bounds.3 as f64;
            let outs = if outs.is_empty() { vec![[bounds.0, bounds.1, bounds.2, bounds.3]] } else { outs.clone() };
            // Un fond d'écran par écran.
            for o in &outs {
                let (ox, oy, ow, oh) = ((o[0] - bounds.0) as f64 * sx, (o[1] - bounds.1) as f64 * sy, o[2] as f64 * sx, o[3] as f64 * sy);
                cr.save().ok();
                rounded(cr, ox + 1.0, oy + 1.0, ow - 2.0, oh - 2.0, 8.0);
                cr.clip();
                match &wall {
                    Some(surf) => {
                        let tw = surf.width() as f64;
                        let th = surf.height() as f64;
                        let s = (ow / tw).max(oh / th);
                        cr.translate(ox + (ow - tw * s) / 2.0, oy + (oh - th * s) / 2.0);
                        cr.scale(s, s);
                        let _ = cr.set_source_surface(surf, 0.0, 0.0);
                        let _ = cr.paint();
                    }
                    None => {
                        cr.set_source_rgb(0.12, 0.14, 0.22);
                        let _ = cr.paint();
                    }
                }
                cr.restore().ok();
            }
            // Fenêtres : rectangles clairs à leur place.
            for r in &rects {
                let (x, y, rw, rh) = ((r[0] - bounds.0) as f64 * sx, (r[1] - bounds.1) as f64 * sy, r[2] as f64 * sx, r[3] as f64 * sy);
                rounded(cr, x, y, rw.max(6.0), rh.max(4.0), 3.0);
                cr.set_source_rgba(0.95, 0.96, 1.0, 0.82);
                let _ = cr.fill_preserve();
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
                // Barre de titre.
                rounded(cr, x, y, rw.max(6.0), (rh * 0.12).clamp(3.0, 6.0), 2.0);
                cr.set_source_rgba(0.5, 0.6, 0.9, 0.9);
                let _ = cr.fill();
            }
        });
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&area));
        if closable {
            let x = gtk::Button::from_icon_name("window-close-symbolic");
            x.add_css_class("tv-close");
            x.add_css_class("circular");
            x.set_halign(gtk::Align::End);
            x.set_valign(gtk::Align::Start);
            x.set_margin_top(4);
            x.set_margin_end(4);
            x.set_tooltip_text(Some("Fermer ce bureau (ses fenêtres rejoignent le bureau voisin)"));
            x.connect_clicked(move |_| apps::request(Request::RemoveDesktop { index }));
            overlay.add_overlay(&x);
        }
        b.set_child(Some(&overlay));
        let me = Rc::downgrade(self);
        b.connect_clicked(move |_| {
            apps::request(Request::SwitchDesktop { index });
            if let Some(tv) = me.upgrade() {
                tv.hide();
            }
        });
        // Déposer une fenêtre : l'envoyer sur ce bureau.
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        drop.connect_drop(move |_, value, _, _| {
            let Ok(s) = value.get::<String>() else { return false };
            let Some(id) = s.strip_prefix("noos-window:").and_then(|x| x.parse::<u64>().ok()) else { return false };
            apps::request(Request::MoveWindowToDesktop { id, index, follow: false });
            true
        });
        b.add_controller(drop);
        v.append(&b);
        // Nom (double-clic : renommer).
        let stack = gtk::Stack::new();
        let label = gtk::Label::new(Some(name));
        label.add_css_class("tv-desk-name");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let entry = gtk::Entry::new();
        entry.set_text(name);
        stack.add_named(&label, Some("label"));
        stack.add_named(&entry, Some("edit"));
        let dbl = gtk::GestureClick::new();
        {
            let (stack, entry) = (stack.clone(), entry.clone());
            dbl.connect_pressed(move |_, n, _, _| {
                if n == 2 {
                    stack.set_visible_child_name("edit");
                    entry.grab_focus();
                }
            });
        }
        label.add_controller(dbl);
        entry.connect_activate(move |e| {
            apps::request(Request::RenameDesktop { index, name: e.text().to_string() });
        });
        label.set_tooltip_text(Some("Double-clic pour renommer"));
        v.append(&stack);
        v.upcast()
    }

    /// Indicateur bref : nom du bureau et points de position.
    pub fn show_osd(self: &Rc<Self>) {
        let Some(shell) = self.shell.upgrade() else { return };
        let (names, current) = shell.desktops.borrow().clone();
        while let Some(c) = self.osd_box.first_child() {
            self.osd_box.remove(&c);
        }
        let l = gtk::Label::new(Some(names.get(current).map(|s| s.as_str()).unwrap_or("")));
        l.add_css_class("osd-name");
        self.osd_box.append(&l);
        let dots = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        dots.set_halign(gtk::Align::Center);
        for i in 0..names.len() {
            let d = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            d.add_css_class("osd-dot");
            if i == current {
                d.add_css_class("on");
            }
            dots.append(&d);
        }
        self.osd_box.append(&dots);
        self.osd.present();
        let generation = self.osd_gen.get() + 1;
        self.osd_gen.set(generation);
        let w = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(1100), move || {
            if let Some(tv) = w.upgrade() {
                if tv.osd_gen.get() == generation {
                    tv.osd.set_visible(false);
                }
            }
        });
    }
}

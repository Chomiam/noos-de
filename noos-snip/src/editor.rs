//! Fenêtre de l'Outil Capture d'écran : annotation (stylo, surligneur, gomme), rognage,
//! copie, enregistrement, nouvelle capture (avec délai).

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use adap::prelude::*;
use gtk::{cairo, gdk, gio, glib};

use crate::{overlay::Mode, shot::Shot};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Pen,
    Highlighter,
    Eraser,
    Crop,
}

#[derive(Clone)]
struct Stroke {
    highlighter: bool,
    color: (f64, f64, f64),
    width: f64,
    points: Vec<(f64, f64)>,
}

const COLORS: &[(&str, (f64, f64, f64))] = &[
    ("Noir", (0.08, 0.08, 0.08)),
    ("Blanc", (1.0, 1.0, 1.0)),
    ("Rouge", (0.91, 0.22, 0.22)),
    ("Jaune", (1.0, 0.82, 0.10)),
    ("Vert", (0.18, 0.72, 0.36)),
    ("Bleu", (0.20, 0.52, 0.96)),
];

pub struct Editor {
    pub window: adap::ApplicationWindow,
    area: gtk::DrawingArea,
    title: adap::WindowTitle,
    toasts: adap::ToastOverlay,
    base: RefCell<Shot>,
    base_surface: RefCell<cairo::ImageSurface>,
    strokes: RefCell<Vec<Stroke>>,
    redo: RefCell<Vec<Stroke>>,
    /// États avant rognage (annulables).
    history: RefCell<Vec<(Shot, Vec<Stroke>)>>,
    current: RefCell<Option<Stroke>>,
    crop: Cell<Option<[f64; 4]>>,
    tool: Cell<Tool>,
    color: Cell<(f64, f64, f64)>,
    size: Cell<f64>,
    /// Transformation affichage → image : (échelle, décalage x, décalage y).
    view: Cell<(f64, f64, f64)>,
    drag_start: Cell<Option<(f64, f64)>>,
    saved: RefCell<Option<PathBuf>>,
    delay: Cell<u32>,
}

impl Editor {
    pub fn new(app: &adap::Application, shot: Shot, saved: Option<PathBuf>) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Outil Capture d'écran")
            .default_width((shot.w + 48).clamp(560, 1280))
            .default_height((shot.h + 160).clamp(420, 860))
            .build();
        let title = adap::WindowTitle::new("Outil Capture d'écran", "");
        let header = adap::HeaderBar::new();
        header.set_title_widget(Some(&title));

        let area = gtk::DrawingArea::new();
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.add_css_class("snip-canvas");
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&area));

        let ed = Rc::new(Self {
            window,
            area,
            title,
            toasts,
            base_surface: RefCell::new(shot.surface()),
            base: RefCell::new(shot),
            strokes: RefCell::new(Vec::new()),
            redo: RefCell::new(Vec::new()),
            history: RefCell::new(Vec::new()),
            current: RefCell::new(None),
            crop: Cell::new(None),
            tool: Cell::new(Tool::Pen),
            color: Cell::new(COLORS[2].1),
            size: Cell::new(4.0),
            view: Cell::new((1.0, 0.0, 0.0)),
            drag_start: Cell::new(None),
            saved: RefCell::new(saved),
            delay: Cell::new(0),
        });

        header.pack_start(&ed.new_menu());
        for (icon, tip, action) in [
            ("folder-open-symbolic", "Ouvrir le dossier des captures", "win.open-folder"),
            ("document-save-as-symbolic", "Enregistrer sous… (Ctrl+S)", "win.save-as"),
            ("edit-copy-symbolic", "Copier (Ctrl+C)", "win.copy"),
        ] {
            let b = gtk::Button::from_icon_name(icon);
            b.set_tooltip_text(Some(tip));
            b.set_action_name(Some(action));
            header.pack_end(&b);
        }

        let view = adap::ToolbarView::new();
        view.add_top_bar(&header);
        view.add_top_bar(&ed.tools_bar());
        view.set_content(Some(&ed.toasts));
        ed.window.set_content(Some(&view));

        ed.setup_actions(app);
        ed.setup_canvas();
        ed.update_title();
        ed
    }

    fn update_title(&self) {
        let b = self.base.borrow();
        let name = self
            .saved
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_default();
        self.title.set_subtitle(&format!("{} × {}{}{}", b.w, b.h, if name.is_empty() { "" } else { " · " }, name));
    }

    fn new_menu(self: &Rc<Self>) -> gtk::MenuButton {
        let button = gtk::MenuButton::new();
        button.set_child(Some(&adap::ButtonContent::builder().icon_name("list-add-symbolic").label("Nouveau").build()));
        button.set_tooltip_text(Some("Nouvelle capture (Ctrl+N)"));
        let pop = gtk::Popover::new();
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for (mode, icon, label) in [
            (Mode::Rect, "edit-select-all-symbolic", "Rectangle"),
            (Mode::Window, "focus-windows-symbolic", "Fenêtre"),
            (Mode::Full, "view-fullscreen-symbolic", "Plein écran"),
            (Mode::Free, "document-edit-symbolic", "Forme libre"),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let c = adap::ButtonContent::builder().icon_name(icon).label(label).build();
            c.set_halign(gtk::Align::Start);
            b.set_child(Some(&c));
            let (ed, pop2) = (self.clone(), pop.clone());
            b.connect_clicked(move |_| {
                pop2.popdown();
                ed.new_capture(mode);
            });
            vbox.append(&b);
        }
        vbox.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let delays = gtk::DropDown::from_strings(&["Pas de délai", "Délai de 3 s", "Délai de 5 s", "Délai de 10 s"]);
        {
            let ed = self.clone();
            delays.connect_selected_notify(move |d| ed.delay.set([0, 3, 5, 10][d.selected().min(3) as usize]));
        }
        vbox.append(&delays);
        pop.set_child(Some(&vbox));
        button.set_popover(Some(&pop));
        button
    }

    /// Nouvelle capture : la fenêtre se cache, un nouveau processus prend le relais
    /// (il reprend aussi le presse-papiers).
    pub fn new_capture(&self, mode: Mode) {
        self.window.set_visible(false);
        let mode = match mode {
            Mode::Rect => "rect",
            Mode::Window => "window",
            Mode::Full => "full",
            Mode::Free => "free",
        };
        let exe = std::env::current_exe().unwrap_or_else(|_| "noos-snip".into());
        let delay = self.delay.get().to_string();
        // Laisse au compositeur le temps de retirer la fenêtre de l'écran.
        let window = self.window.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            let _ = std::process::Command::new(exe).args(["--mode", mode, "--delay", &delay]).spawn();
            window.close();
        });
    }

    fn tools_bar(self: &Rc<Self>) -> gtk::Widget {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bar.add_css_class("toolbar");
        bar.set_halign(gtk::Align::Center);
        let mut group: Option<gtk::ToggleButton> = None;
        for (tool, icon, tip) in [
            (Tool::Pen, "document-edit-symbolic", "Stylo"),
            (Tool::Highlighter, "format-text-highlight-symbolic", "Surligneur"),
            (Tool::Eraser, "edit-clear-symbolic", "Gomme (efface un trait)"),
            (Tool::Crop, "image-crop-symbolic", "Rogner"),
        ] {
            let b = gtk::ToggleButton::new();
            b.set_child(Some(&adap::ButtonContent::builder().icon_name(icon).label(tip.split(' ').next().unwrap_or(tip)).build()));
            b.set_tooltip_text(Some(tip));
            b.add_css_class("flat");
            b.set_group(group.as_ref());
            if group.is_none() {
                group = Some(b.clone());
            }
            b.set_active(tool == Tool::Pen);
            let ed = self.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    ed.tool.set(tool);
                    ed.crop.set(None);
                    ed.area.queue_draw();
                }
            });
            bar.append(&b);
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));

        let mut cgroup: Option<gtk::ToggleButton> = None;
        for (name, rgb) in COLORS {
            let b = gtk::ToggleButton::new();
            b.set_tooltip_text(Some(name));
            b.add_css_class("color-swatch");
            b.set_valign(gtk::Align::Center);
            b.set_size_request(22, 22);
            b.add_css_class(&format!("swatch-{}", name.to_lowercase()));
            b.set_group(cgroup.as_ref());
            if cgroup.is_none() {
                cgroup = Some(b.clone());
            }
            b.set_active(*rgb == self.color.get());
            let (ed, rgb) = (self.clone(), *rgb);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    ed.color.set(rgb);
                }
            });
            bar.append(&b);
        }
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let size = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 24.0, 1.0);
        size.set_value(self.size.get());
        size.set_size_request(110, -1);
        size.set_tooltip_text(Some("Épaisseur"));
        {
            let ed = self.clone();
            size.connect_value_changed(move |s| ed.size.set(s.value()));
        }
        bar.append(&size);
        bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        for (icon, tip, action) in [("edit-undo-symbolic", "Annuler (Ctrl+Z)", "win.undo"), ("edit-redo-symbolic", "Rétablir (Ctrl+Y)", "win.redo")] {
            let b = gtk::Button::from_icon_name(icon);
            b.set_tooltip_text(Some(tip));
            b.add_css_class("flat");
            b.set_action_name(Some(action));
            bar.append(&b);
        }
        bar.upcast()
    }

    fn setup_actions(self: &Rc<Self>, app: &adap::Application) {
        let add = |name: &str, accels: &[&str], f: Box<dyn Fn()>| {
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| f());
            self.window.add_action(&action);
            app.set_accels_for_action(&format!("win.{name}"), accels);
        };
        let ed = self.clone();
        add("copy", &["<Control>c"], Box::new(move || ed.copy(true)));
        let ed = self.clone();
        add("save-as", &["<Control>s"], Box::new(move || ed.save_as()));
        let ed = self.clone();
        add("undo", &["<Control>z"], Box::new(move || ed.undo()));
        let ed = self.clone();
        add("redo", &["<Control>y", "<Control><Shift>z"], Box::new(move || ed.redo()));
        let ed = self.clone();
        add("new", &["<Control>n"], Box::new(move || ed.new_capture(Mode::Rect)));
        let ed = self.clone();
        add(
            "open-folder",
            &[],
            Box::new(move || {
                let dir = ed.saved.borrow().as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf()));
                if let Some(dir) = dir {
                    let uri = gio::File::for_path(dir).uri();
                    let _ = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE);
                }
            }),
        );
    }

    // ───────────── Rendu ─────────────

    fn setup_canvas(self: &Rc<Self>) {
        {
            let ed = self.clone();
            self.area.set_draw_func(move |_, cr, w, h| ed.draw(cr, w, h));
        }
        let drag = gtk::GestureDrag::new();
        {
            let ed = self.clone();
            drag.connect_drag_begin(move |_, x, y| {
                let p = ed.to_image(x, y);
                ed.drag_start.set(Some((x, y)));
                match ed.tool.get() {
                    Tool::Pen | Tool::Highlighter => {
                        let hl = ed.tool.get() == Tool::Highlighter;
                        *ed.current.borrow_mut() = Some(Stroke {
                            highlighter: hl,
                            color: ed.color.get(),
                            width: if hl { ed.size.get() * 3.0 + 6.0 } else { ed.size.get() },
                            points: vec![p],
                        });
                    }
                    Tool::Eraser => ed.erase_at(p),
                    Tool::Crop => ed.crop.set(Some([p.0, p.1, 0.0, 0.0])),
                }
                ed.area.queue_draw();
            });
        }
        {
            let ed = self.clone();
            drag.connect_drag_update(move |_, dx, dy| {
                let Some((sx, sy)) = ed.drag_start.get() else { return };
                let p = ed.to_image(sx + dx, sy + dy);
                match ed.tool.get() {
                    Tool::Pen | Tool::Highlighter => {
                        if let Some(s) = ed.current.borrow_mut().as_mut() {
                            s.points.push(p);
                        }
                    }
                    Tool::Eraser => ed.erase_at(p),
                    Tool::Crop => {
                        let a = ed.to_image(sx, sy);
                        ed.crop.set(Some([a.0.min(p.0), a.1.min(p.1), (a.0 - p.0).abs(), (a.1 - p.1).abs()]));
                    }
                }
                ed.area.queue_draw();
            });
        }
        {
            let ed = self.clone();
            drag.connect_drag_end(move |_, _, _| {
                ed.drag_start.set(None);
                if let Some(s) = ed.current.borrow_mut().take() {
                    ed.strokes.borrow_mut().push(s);
                    ed.redo.borrow_mut().clear();
                }
                if ed.tool.get() == Tool::Crop {
                    if let Some(r) = ed.crop.take() {
                        if r[2] >= 4.0 && r[3] >= 4.0 {
                            ed.apply_crop(r);
                        }
                    }
                }
                ed.area.queue_draw();
            });
        }
        self.area.add_controller(drag);
    }

    fn to_image(&self, x: f64, y: f64) -> (f64, f64) {
        let (s, ox, oy) = self.view.get();
        ((x - ox) / s, (y - oy) / s)
    }

    fn draw(&self, cr: &cairo::Context, w: i32, h: i32) {
        let b = self.base.borrow();
        let margin = 24.0;
        let s = ((w as f64 - margin * 2.0) / b.w as f64).min((h as f64 - margin * 2.0) / b.h as f64).clamp(0.05, 1.0);
        let ox = ((w as f64 - b.w as f64 * s) / 2.0).round();
        let oy = ((h as f64 - b.h as f64 * s) / 2.0).round();
        self.view.set((s, ox, oy));

        cr.save().ok();
        cr.translate(ox, oy);
        cr.scale(s, s);
        // Ombre légère sous l'image.
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
        cr.rectangle(2.0 / s, 3.0 / s, b.w as f64, b.h as f64);
        let _ = cr.fill();
        let _ = cr.set_source_surface(&*self.base_surface.borrow(), 0.0, 0.0);
        let _ = cr.paint();
        for stroke in self.strokes.borrow().iter().chain(self.current.borrow().iter()) {
            draw_stroke(cr, stroke);
        }
        if let Some(r) = self.crop.get() {
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(0.0, 0.0, b.w as f64, b.h as f64);
            cr.rectangle(r[0], r[1], r[2], r[3]);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
            let _ = cr.fill();
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.set_line_width(1.5 / s);
            cr.rectangle(r[0], r[1], r[2], r[3]);
            let _ = cr.stroke();
        }
        cr.restore().ok();
    }

    fn erase_at(&self, p: (f64, f64)) {
        let (s, _, _) = self.view.get();
        let radius = 10.0 / s;
        let before = self.strokes.borrow().len();
        self.strokes.borrow_mut().retain(|st| {
            !st.points.iter().any(|q| (q.0 - p.0).hypot(q.1 - p.1) < radius + st.width / 2.0)
        });
        if self.strokes.borrow().len() != before {
            self.redo.borrow_mut().clear();
        }
    }

    /// Image + annotations, à la taille réelle.
    fn flatten(&self) -> Shot {
        let b = self.base.borrow();
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, b.w, b.h).expect("surface");
        {
            let cr = cairo::Context::new(&surface).expect("contexte");
            let _ = cr.set_source_surface(&*self.base_surface.borrow(), 0.0, 0.0);
            let _ = cr.paint();
            for stroke in self.strokes.borrow().iter() {
                draw_stroke(&cr, stroke);
            }
        }
        Shot::from_surface(surface)
    }

    fn apply_crop(&self, r: [f64; 4]) {
        let flat = self.flatten();
        let cropped = flat.crop(r[0].round() as i32, r[1].round() as i32, r[2].round() as i32, r[3].round() as i32);
        self.history.borrow_mut().push((self.base.borrow().clone(), self.strokes.borrow().clone()));
        self.set_base(cropped);
        self.strokes.borrow_mut().clear();
        self.redo.borrow_mut().clear();
    }

    fn set_base(&self, shot: Shot) {
        *self.base_surface.borrow_mut() = shot.surface();
        *self.base.borrow_mut() = shot;
        self.update_title();
        self.area.queue_draw();
    }

    fn undo(&self) {
        let last = self.strokes.borrow_mut().pop();
        if let Some(s) = last {
            self.redo.borrow_mut().push(s);
        } else if let Some((base, strokes)) = self.history.borrow_mut().pop() {
            self.set_base(base);
            *self.strokes.borrow_mut() = strokes;
        }
        self.area.queue_draw();
    }

    fn redo(&self) {
        if let Some(s) = self.redo.borrow_mut().pop() {
            self.strokes.borrow_mut().push(s);
            self.area.queue_draw();
        }
    }

    // ───────────── Sorties ─────────────

    pub fn copy(&self, toast: bool) {
        let tex = self.flatten().texture();
        WidgetExt::display(&self.window).clipboard().set_texture(&tex);
        if toast {
            self.toasts.add_toast(adap::Toast::new("Copiée dans le Presse-papiers"));
        }
    }

    fn save_as(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Enregistrer la capture").modal(true).build();
        let name = self
            .saved
            .borrow()
            .as_ref()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "Capture d'écran.png".into());
        dialog.set_initial_name(Some(&name));
        if let Some(dir) = self.saved.borrow().as_ref().and_then(|p| p.parent()) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let ed = self.clone();
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(file) = res else { return };
            let Some(mut path) = file.path() else { return };
            if path.extension().is_none() {
                path.set_extension("png");
            }
            match ed.flatten().save_png(&path) {
                Ok(()) => {
                    ed.toasts.add_toast(adap::Toast::new("Capture enregistrée"));
                    *ed.saved.borrow_mut() = Some(path);
                    ed.update_title();
                }
                Err(e) => ed.toasts.add_toast(adap::Toast::new(&format!("Échec de l'enregistrement : {e}"))),
            }
        });
    }
}

fn draw_stroke(cr: &cairo::Context, s: &Stroke) {
    if s.points.is_empty() {
        return;
    }
    let (r, g, b) = s.color;
    cr.set_source_rgba(r, g, b, if s.highlighter { 0.38 } else { 1.0 });
    cr.set_line_width(s.width);
    cr.set_line_cap(if s.highlighter { cairo::LineCap::Square } else { cairo::LineCap::Round });
    cr.set_line_join(cairo::LineJoin::Round);
    cr.move_to(s.points[0].0, s.points[0].1);
    if s.points.len() == 1 {
        cr.line_to(s.points[0].0 + 0.1, s.points[0].1);
    }
    for p in s.points.iter().skip(1) {
        cr.line_to(p.0, p.1);
    }
    let _ = cr.stroke();
}

/// Feuille de style de l'outil (barre de sélection, pastilles de couleur).
pub fn css() -> String {
    let mut css = String::from(
        r#"
window.snip-overlay { background: transparent; }
.snip-toolbar {
  background-color: alpha(@window_bg_color, 0.96);
  border: 1px solid alpha(@window_fg_color, 0.12);
  border-radius: 10px;
  padding: 6px;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.4);
}
.snip-toolbar button { border-radius: 6px; }
.snip-canvas { background-color: alpha(@window_fg_color, 0.04); }
.color-swatch { min-width: 22px; min-height: 22px; padding: 0; margin: 0 1px; border-radius: 99px; border: 2px solid alpha(@window_fg_color, 0.25); }
.color-swatch:checked { border: 2px solid @accent_color; box-shadow: 0 0 0 2px alpha(@accent_color, 0.35); }
"#,
    );
    for (name, (r, g, b)) in COLORS {
        css.push_str(&format!(
            ".swatch-{} {{ background: rgb({}, {}, {}); }}\n",
            name.to_lowercase(),
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8
        ));
    }
    css
}

pub fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css());
    gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

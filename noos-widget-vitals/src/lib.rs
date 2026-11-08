//! Widget « Ressources » de la barre des tâches (coin inférieur gauche, comme les widgets
//! de Windows 11) : processeur, carte graphique, mémoire, disque et réseau, chacun avec sa
//! petite icône, sa valeur et une jauge. Un clic ouvre le détail avec l'historique des
//! deux dernières minutes.
//!
//! ```ignore
//! let w = noos_widget_vitals::Vitals::new();
//! barre.append(w.widget());
//! ```

mod sample;

use std::{
    cell::RefCell,
    collections::VecDeque,
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};
pub use sample::{Sampler, Snapshot};
use sample::{gib, rate};

/// Intervalle entre deux mesures.
const PERIOD: Duration = Duration::from_secs(2);
/// Points d'historique (2 min).
const HISTORY: usize = 60;

const ICONS: &[(&str, &str)] = &[
    ("noos-vitals-cpu-symbolic.svg", include_str!("icons/noos-vitals-cpu-symbolic.svg")),
    ("noos-vitals-gpu-symbolic.svg", include_str!("icons/noos-vitals-gpu-symbolic.svg")),
    ("noos-vitals-ram-symbolic.svg", include_str!("icons/noos-vitals-ram-symbolic.svg")),
    ("noos-vitals-disk-symbolic.svg", include_str!("icons/noos-vitals-disk-symbolic.svg")),
    ("noos-vitals-net-symbolic.svg", include_str!("icons/noos-vitals-net-symbolic.svg")),
    ("noos-vitals-temp-symbolic.svg", include_str!("icons/noos-vitals-temp-symbolic.svg")),
];

const CSS: &str = r#"
button.vitals { padding: 2px 8px; border-radius: 8px; }
.vitals .vital { min-width: 46px; }
.vitals .vital label { font-size: 0.82em; font-feature-settings: "tnum"; }
.vitals .vital.net { min-width: 64px; }
.vital-cpu { color: #4c9aff; }
.vital-gpu { color: #3ec27f; }
.vital-ram { color: #a970ff; }
.vital-disk { color: #ffa348; }
.vital-net { color: #33c7de; }
.vitals-pop { padding: 14px 16px; min-width: 360px; }
.vitals-pop .vitals-title { font-weight: 700; font-size: 1.1em; }
.vitals-pop .vitals-sub { font-size: 0.82em; opacity: 0.6; }
.vitals-pop .vital-card {
  padding: 10px 12px; border-radius: 10px;
  background-color: alpha(@window_fg_color, 0.05);
}
.vitals-pop .vital-badge {
  min-width: 30px; min-height: 30px; border-radius: 8px;
  background-color: alpha(currentColor, 0.16);
}
.vitals-pop .vital-name { font-weight: 600; color: @window_fg_color; }
.vitals-pop .vital-detail { font-size: 0.8em; opacity: 0.65; color: @window_fg_color; }
.vitals-pop .vital-value { font-size: 1.25em; font-weight: 700; font-feature-settings: "tnum"; color: @window_fg_color; }
"#;

/// Composants suivis.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cpu,
    Gpu,
    Ram,
    Disk,
    Net,
}

impl Kind {
    const ALL: [Kind; 5] = [Kind::Cpu, Kind::Gpu, Kind::Ram, Kind::Disk, Kind::Net];
    fn id(self) -> &'static str {
        match self {
            Kind::Cpu => "cpu",
            Kind::Gpu => "gpu",
            Kind::Ram => "ram",
            Kind::Disk => "disk",
            Kind::Net => "net",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Kind::Cpu => "Processeur",
            Kind::Gpu => "Carte graphique",
            Kind::Ram => "Mémoire",
            Kind::Disk => "Disque",
            Kind::Net => "Réseau",
        }
    }
    fn rgb(self) -> (f64, f64, f64) {
        match self {
            Kind::Cpu => (0.30, 0.60, 1.0),
            Kind::Gpu => (0.24, 0.76, 0.50),
            Kind::Ram => (0.66, 0.44, 1.0),
            Kind::Disk => (1.0, 0.64, 0.28),
            Kind::Net => (0.20, 0.78, 0.87),
        }
    }
    fn icon(self) -> String {
        format!("noos-vitals-{}-symbolic", self.id())
    }
}

/// Historique en pourcentage (le réseau est ramené à son maximum récent).
#[derive(Default)]
struct Series(VecDeque<f64>);

impl Series {
    fn push(&mut self, v: f64) {
        self.0.push_back(v);
        while self.0.len() > HISTORY {
            self.0.pop_front();
        }
    }
    fn max(&self) -> f64 {
        self.0.iter().copied().fold(0.0, f64::max)
    }
}

struct Item {
    kind: Kind,
    // Barre des tâches.
    small: gtk::Box,
    small_label: gtk::Label,
    small_bar: gtk::DrawingArea,
    // Détail.
    card: gtk::Box,
    name: gtk::Label,
    detail: gtk::Label,
    value: gtk::Label,
    graph: gtk::DrawingArea,
    level: RefCell<f64>,
    series: Rc<RefCell<Series>>,
}

pub struct Vitals {
    button: gtk::MenuButton,
    items: Vec<Rc<Item>>,
    subtitle: gtk::Label,
}

/// Icônes : écrites une fois dans le cache et ajoutées au thème d'icônes (recoloriées
/// comme les autres icônes symboliques).
fn install_icons() {
    let dir = glib::user_cache_dir().join("noos/vitals-icons");
    let _ = std::fs::create_dir_all(&dir);
    for (name, svg) in ICONS {
        let p = dir.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(*svg) {
            let _ = std::fs::write(&p, svg);
        }
    }
    if let Some(d) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&d).add_search_path(&dir);
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    }
}

/// Petite jauge horizontale (2 px) sous la valeur.
fn draw_level(item: &Rc<Item>) {
    let (kind, weak) = (item.kind, Rc::downgrade(item));
    item.small_bar.set_draw_func(move |_, cr, w, h| {
        let Some(it) = weak.upgrade() else { return };
        let v = (*it.level.borrow() / 100.0).clamp(0.0, 1.0);
        let (r, g, b) = if v > 0.9 { (1.0, 0.37, 0.37) } else { kind.rgb() };
        rounded(cr, 0.0, 0.0, w as f64, h as f64, h as f64 / 2.0);
        cr.set_source_rgba(r, g, b, 0.22);
        let _ = cr.fill();
        if v > 0.0 {
            rounded(cr, 0.0, 0.0, (w as f64 * v).max(h as f64), h as f64, h as f64 / 2.0);
            cr.set_source_rgba(r, g, b, 1.0);
            let _ = cr.fill();
        }
    });
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::PI;
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Courbe de l'historique (aire remplie) ; `scale` = valeur du haut du graphique.
fn draw_graph(item: &Rc<Item>) {
    let (kind, series) = (item.kind, item.series.clone());
    item.graph.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let s = series.borrow();
        let (r, g, b) = kind.rgb();
        // Lignes de repère.
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.15);
        cr.set_line_width(1.0);
        for k in 1..4 {
            let y = (h * k as f64 / 4.0).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(w, y);
        }
        let _ = cr.stroke();
        if s.0.len() < 2 {
            return;
        }
        let scale = if kind == Kind::Net { s.max().max(1.0) } else { 100.0 };
        let step = w / (HISTORY - 1) as f64;
        let x0 = w - step * (s.0.len() - 1) as f64;
        let pt = |i: usize, v: f64| (x0 + step * i as f64, h - (v / scale).clamp(0.0, 1.0) * (h - 2.0) - 1.0);
        cr.move_to(x0, h);
        for (i, v) in s.0.iter().enumerate() {
            let (x, y) = pt(i, *v);
            cr.line_to(x, y);
        }
        cr.line_to(w, h);
        cr.close_path();
        let grad = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
        grad.add_color_stop_rgba(0.0, r, g, b, 0.45);
        grad.add_color_stop_rgba(1.0, r, g, b, 0.03);
        let _ = cr.set_source(&grad);
        let _ = cr.fill();
        for (i, v) in s.0.iter().enumerate() {
            let (x, y) = pt(i, *v);
            if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
        }
        cr.set_source_rgba(r, g, b, 1.0);
        cr.set_line_width(1.6);
        let _ = cr.stroke();
    });
}

impl Vitals {
    pub fn new() -> Rc<Self> {
        install_icons();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        pop_box.add_css_class("vitals-pop");
        let head = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(Some("Ressources"));
        title.add_css_class("vitals-title");
        title.set_xalign(0.0);
        let subtitle = gtk::Label::new(Some(""));
        subtitle.add_css_class("vitals-sub");
        subtitle.set_xalign(0.0);
        head.append(&title);
        head.append(&subtitle);
        pop_box.append(&head);

        let mut items = vec![];
        for kind in Kind::ALL {
            // ── Version compacte (barre) ──
            let small = gtk::Box::new(gtk::Orientation::Vertical, 2);
            small.add_css_class("vital");
            small.add_css_class(kind.id());
            small.set_valign(gtk::Align::Center);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let icon = gtk::Image::from_icon_name(&kind.icon());
            icon.set_pixel_size(14);
            icon.add_css_class(&format!("vital-{}", kind.id()));
            let small_label = gtk::Label::new(Some("–"));
            small_label.set_xalign(0.0);
            small_label.set_hexpand(true);
            line.append(&icon);
            line.append(&small_label);
            let small_bar = gtk::DrawingArea::new();
            small_bar.set_content_height(3);
            small.append(&line);
            small.append(&small_bar);
            row.append(&small);

            // ── Carte du détail ──
            let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
            card.add_css_class("vital-card");
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let badge = gtk::Image::from_icon_name(&kind.icon());
            badge.add_css_class("vital-badge");
            badge.add_css_class(&format!("vital-{}", kind.id()));
            top.append(&badge);
            let texts = gtk::Box::new(gtk::Orientation::Vertical, 1);
            texts.set_hexpand(true);
            texts.set_valign(gtk::Align::Center);
            let name = gtk::Label::new(Some(kind.title()));
            name.add_css_class("vital-name");
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let detail = gtk::Label::new(None);
            detail.add_css_class("vital-detail");
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            detail.set_max_width_chars(34);
            texts.append(&name);
            texts.append(&detail);
            top.append(&texts);
            let value = gtk::Label::new(Some("–"));
            value.add_css_class("vital-value");
            value.set_valign(gtk::Align::Center);
            top.append(&value);
            card.append(&top);
            let graph = gtk::DrawingArea::new();
            graph.set_content_height(34);
            card.append(&graph);
            pop_box.append(&card);

            let item = Rc::new(Item {
                kind,
                small,
                small_label,
                small_bar,
                card,
                name,
                detail,
                value,
                graph,
                level: RefCell::new(0.0),
                series: Rc::new(RefCell::new(Series::default())),
            });
            draw_level(&item);
            draw_graph(&item);
            items.push(item);
        }

        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.set_child(Some(&pop_box));
        let button = gtk::MenuButton::new();
        button.set_always_show_arrow(false);
        button.add_css_class("vitals");
        button.add_css_class("flat");
        button.set_child(Some(&row));
        button.set_popover(Some(&popover));
        button.set_tooltip_text(Some("Ressources de l'ordinateur"));

        let v = Rc::new(Self { button, items, subtitle });
        v.start();
        // Captures : NOOS_VITALS_OPEN=1 ouvre le détail après quelques mesures.
        if std::env::var_os("NOOS_VITALS_OPEN").is_some() {
            let b = v.button.clone();
            glib::timeout_add_local_once(Duration::from_secs(7), move || b.popup());
        }
        v
    }

    pub fn widget(&self) -> &gtk::MenuButton {
        &self.button
    }

    /// Mesures dans un fil à part ; s'arrête quand le widget est détruit.
    fn start(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded::<Snapshot>(1);
        std::thread::spawn(move || {
            let mut s = Sampler::new();
            std::thread::sleep(Duration::from_millis(400));
            loop {
                if tx.send_blocking(s.sample()).is_err() {
                    break;
                }
                std::thread::sleep(PERIOD);
            }
        });
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(snap) = rx.recv().await {
                let Some(v) = w.upgrade() else { break };
                v.update(&snap);
            }
        });
    }

    fn update(&self, s: &Snapshot) {
        let pct = |v: f64| format!("{:.0} %", v);
        for it in &self.items {
            let (level, small, value, name, detail): (f64, String, String, String, String) = match it.kind {
                Kind::Cpu => (
                    s.cpu,
                    pct(s.cpu),
                    pct(s.cpu),
                    s.cpu_name.clone(),
                    [
                        Some(format!("{} cœurs logiques", s.cpu_threads)),
                        s.cpu_mhz.map(|m| format!("{:.2} GHz", m as f64 / 1000.0).replace('.', ",")),
                        s.cpu_temp.map(|t| format!("{t:.0} °C")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · "),
                ),
                Kind::Gpu => {
                    let g = s.gpu.unwrap_or(0.0);
                    it.small.set_visible(s.gpu.is_some());
                    it.card.set_visible(s.gpu.is_some());
                    (
                        g,
                        pct(g),
                        pct(g),
                        s.gpu_name.clone(),
                        [
                            Some(s.gpu_note.to_string()),
                            s.gpu_mem.map(|(u, t)| format!("Mémoire {} / {}", gib(u), gib(t))),
                            s.gpu_temp.map(|t| format!("{t:.0} °C")),
                        ]
                        .into_iter()
                        .flatten()
                        .filter(|x| !x.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · "),
                    )
                }
                Kind::Ram => (
                    s.ram_pct(),
                    pct(s.ram_pct()),
                    pct(s.ram_pct()),
                    "Mémoire".into(),
                    format!(
                        "{} utilisés sur {}{}",
                        gib(s.ram_used),
                        gib(s.ram_total),
                        if s.swap_total > 0 { format!(" · échange {}", gib(s.swap_used)) } else { String::new() }
                    ),
                ),
                Kind::Disk => (
                    s.disk_busy,
                    pct(s.disk_busy),
                    pct(s.disk_busy),
                    "Disque".into(),
                    format!(
                        "Lecture {} · écriture {}\nSystème : {} libres sur {}",
                        rate(s.disk_read),
                        rate(s.disk_write),
                        gib((s.root_total - s.root_used.min(s.root_total)) >> 20),
                        gib(s.root_total >> 20)
                    ),
                ),
                Kind::Net => {
                    let total = s.net_rx + s.net_tx;
                    let peak = it.series.borrow().max().max(total).max(1.0);
                    (total * 100.0 / peak, rate(s.net_rx), rate(total), "Réseau".into(), format!("↓ {} · ↑ {}", rate(s.net_rx), rate(s.net_tx)))
                }
            };
            let point = if it.kind == Kind::Net { s.net_rx + s.net_tx } else { level };
            it.series.borrow_mut().push(point);
            *it.level.borrow_mut() = level;
            it.small_label.set_text(&small);
            it.value.set_text(&value);
            it.name.set_text(&name);
            it.detail.set_text(&detail);
            it.small_bar.queue_draw();
            it.graph.queue_draw();
        }
        self.subtitle.set_text(&format!("Mis à jour toutes les {} s · historique de 2 min", PERIOD.as_secs()));
        self.button.set_tooltip_text(Some(&format!(
            "Processeur {:.0} % · Mémoire {:.0} %{} · Disque {:.0} % · Réseau ↓ {}",
            s.cpu,
            s.ram_pct(),
            s.gpu.map(|g| format!(" · Carte graphique {g:.0} %")).unwrap_or_default(),
            s.disk_busy,
            rate(s.net_rx)
        )));
    }
}

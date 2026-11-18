//! Ressources : petit = quatre jauges rondes (processeur, carte graphique, mémoire, disque) ;
//! moyen = la barre compacte de la barre des tâches (`noos-widget-vitals`) ; grand = une carte
//! par composant (nom du matériel, valeur, jauge, détail) et le débit réseau.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};
use noos_core::config::DesktopWidget;
use noos_widget_vitals::{Sampler, Snapshot, gib, rate};

use super::{Ctx, ring};

pub const CSS: &str = r#"
.desk-vit-gauges { padding: 2px; }
.desk-vit-gauge label.value { font-weight: 800; font-size: 0.95em; font-feature-settings: "tnum"; }
.desk-vit-gauge label.name { font-size: 0.75em; opacity: 0.7; }
.desk-vit-row { padding: 8px 10px; border-radius: 12px; background-color: alpha(currentColor, 0.06); }
.desk-vit-row .name { font-weight: 700; }
.desk-vit-row .detail { font-size: 0.8em; opacity: 0.65; }
.desk-vit-row .value { font-size: 1.35em; font-weight: 800; font-feature-settings: "tnum"; }
.desk-vit-row progressbar trough, .desk-vit-row progressbar progress { min-height: 5px; border-radius: 99px; }
.desk-vit-badge { min-width: 30px; min-height: 30px; border-radius: 9px; color: white; }
.vit-cpu .desk-vit-badge, .vit-cpu progressbar progress { background-color: #4c9aff; }
.vit-gpu .desk-vit-badge, .vit-gpu progressbar progress { background-color: #3ec27f; }
.vit-ram .desk-vit-badge, .vit-ram progressbar progress { background-color: #a970ff; }
.vit-disk .desk-vit-badge, .vit-disk progressbar progress { background-color: #ffa348; }
.vit-net .desk-vit-badge { background-color: #33c7de; }
"#;

/// Couleurs des composants (les mêmes que dans la barre).
const COLORS: [(f64, f64, f64); 4] = [(0.30, 0.60, 1.0), (0.24, 0.76, 0.50), (0.66, 0.44, 1.0), (1.0, 0.64, 0.28)];

/// Mesures toutes les 2 s dans un thread ; `f` reçoit chaque mesure tant que `widget` existe.
fn watch(widget: &impl IsA<gtk::Widget>, f: impl Fn(&Snapshot) + 'static) {
    let (tx, rx) = async_channel::bounded::<Snapshot>(1);
    std::thread::spawn(move || {
        let mut s = Sampler::new();
        std::thread::sleep(Duration::from_millis(300));
        // S'arrête quand le widget (donc le récepteur) disparaît.
        while tx.send_blocking(s.sample()).is_ok() {
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    let w = widget.as_ref().downgrade();
    glib::spawn_future_local(async move {
        while let Ok(snap) = rx.recv().await {
            if w.upgrade().is_none() {
                break;
            }
            f(&snap);
        }
    });
}

/// (pourcentage, valeur affichée) des quatre jauges.
fn values(s: &Snapshot) -> [(f64, String); 4] {
    let disk = if s.root_total > 0 { s.root_used as f64 * 100.0 / s.root_total as f64 } else { 0.0 };
    [
        (s.cpu, format!("{:.0} %", s.cpu)),
        (s.gpu.unwrap_or(0.0), s.gpu.map(|g| format!("{g:.0} %")).unwrap_or_else(|| "—".into())),
        (s.ram_pct(), format!("{:.0} %", s.ram_pct())),
        (disk, format!("{disk:.0} %")),
    ]
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    match w.size() {
        "s" => small(),
        "l" => large(),
        _ => medium(),
    }
}

fn medium() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let t = gtk::Label::new(Some("Ressources"));
    t.add_css_class("desk-title");
    t.set_xalign(0.0);
    b.append(&t);
    let v = noos_widget_vitals::Vitals::new();
    v.widget().set_sensitive(true);
    b.append(v.widget());
    let keep = Rc::new(v);
    b.connect_destroy(move |_| {
        let _ = &keep;
    });
    b.upcast()
}

fn small() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("desk-vit-gauges");
    let state: Rc<RefCell<[(f64, String); 4]>> = Rc::new(RefCell::new(Default::default()));
    let mut parts = vec![];
    for (i, name) in ["Proc.", "Graph.", "Mém.", "Disque"].into_iter().enumerate() {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.add_css_class("desk-vit-gauge");
        let o = gtk::Overlay::new();
        let a = gtk::DrawingArea::new();
        a.set_content_width(54);
        a.set_content_height(54);
        let st = state.clone();
        a.set_draw_func(move |a, cr, w, h| {
            let fg = a.color();
            let p = st.borrow()[i].0 / 100.0;
            ring(cr, w as f64, h as f64, p, COLORS[i], (fg.red() as f64, fg.green() as f64, fg.blue() as f64));
        });
        o.set_child(Some(&a));
        let v = gtk::Label::new(Some("…"));
        v.add_css_class("value");
        o.add_overlay(&v);
        col.append(&o);
        let n = gtk::Label::new(Some(name));
        n.add_css_class("name");
        col.append(&n);
        b.append(&col);
        parts.push((a, v));
    }
    watch(&b, move |s| {
        *state.borrow_mut() = values(s);
        for (i, (a, v)) in parts.iter().enumerate() {
            v.set_text(&state.borrow()[i].1.replace(" %", "%"));
            a.queue_draw();
        }
    });
    b.upcast()
}

fn large() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_size_request(380, -1);
    let t = gtk::Label::new(Some("Ressources"));
    t.add_css_class("desk-title");
    t.set_xalign(0.0);
    b.append(&t);
    let mut rows = vec![];
    for (id, name, icon) in [
        ("cpu", "Processeur", "noos-vitals-cpu-symbolic"),
        ("gpu", "Carte graphique", "noos-vitals-gpu-symbolic"),
        ("ram", "Mémoire", "noos-vitals-ram-symbolic"),
        ("disk", "Disque système", "noos-vitals-disk-symbolic"),
        ("net", "Réseau", "noos-vitals-net-symbolic"),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("desk-vit-row");
        row.add_css_class(&format!("vit-{id}"));
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("desk-vit-badge");
        badge.set_valign(gtk::Align::Center);
        badge.set_hexpand(false);
        let img = gtk::Image::from_icon_name(icon);
        img.set_hexpand(true);
        badge.append(&img);
        row.append(&badge);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 3);
        col.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let n = gtk::Label::new(Some(name));
        n.add_css_class("name");
        n.set_xalign(0.0);
        n.set_hexpand(true);
        top.append(&n);
        let v = gtk::Label::new(Some("…"));
        v.add_css_class("value");
        top.append(&v);
        col.append(&top);
        let bar = gtk::ProgressBar::new();
        bar.set_visible(id != "net");
        col.append(&bar);
        let d = gtk::Label::new(None);
        d.add_css_class("detail");
        d.set_xalign(0.0);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&d);
        row.append(&col);
        b.append(&row);
        rows.push((id, v, bar, d, row));
    }
    watch(&b, move |s| {
        let vals = values(s);
        for (id, v, bar, d, row) in &rows {
            let (pct, text, detail) = match *id {
                "cpu" => (vals[0].0, vals[0].1.clone(), {
                    let mut t = format!("{} · {} fils", if s.cpu_name.is_empty() { "Processeur" } else { &s.cpu_name }, s.cpu_threads);
                    if let Some(c) = s.cpu_temp {
                        t.push_str(&format!(" · {c:.0} °C"));
                    }
                    t
                }),
                "gpu" => {
                    row.set_visible(s.gpu.is_some() || !s.gpu_name.is_empty());
                    let mut t = if s.gpu_name.is_empty() { "Carte graphique".to_string() } else { s.gpu_name.clone() };
                    if let Some((used, total)) = s.gpu_mem {
                        t.push_str(&format!(" · {} / {}", gib(used), gib(total)));
                    }
                    (vals[1].0, vals[1].1.clone(), t)
                }
                "ram" => (vals[2].0, vals[2].1.clone(), format!("{} utilisés sur {}", gib(s.ram_used), gib(s.ram_total))),
                "disk" => (vals[3].0, vals[3].1.clone(), format!("{} libres sur {} · lecture {} · écriture {}", gib((s.root_total - s.root_used.min(s.root_total)) >> 20), gib(s.root_total >> 20), rate(s.disk_read), rate(s.disk_write))),
                _ => (0.0, format!("↓ {}", rate(s.net_rx)), format!("Envoi ↑ {}", rate(s.net_tx))),
            };
            v.set_text(&text);
            bar.set_fraction((pct / 100.0).clamp(0.0, 1.0));
            d.set_text(&detail);
        }
    });
    b.upcast()
}

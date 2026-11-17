//! Énergie (zone de notification, élément `power` de la barre).
//!
//! - **Portable** : batterie dessinée selon le niveau et l'état (en charge, branchée, faible,
//!   critique) ; info-bulle avec le pourcentage et l'autonomie. Données d'UPower
//!   (`DisplayDevice`) par D-Bus, mises à jour par le signal `PropertiesChanged` ; sans UPower,
//!   lecture de `/sys/class/power_supply` toutes les 60 s.
//! - **PC fixe** : jauge dont l'aiguille montre le mode d'alimentation actif.
//! - Clic : panneau (batterie en grand, santé, consommation, modes Économie / Équilibré /
//!   Performances, raccourci vers Paramètres › Alimentation). Molette : mode voisin.
//!
//! Modes : power-profiles-daemon (`org.freedesktop.UPower.PowerProfiles`, ancien nom
//! `net.hadess.PowerProfiles`), ou auto-cpufreq sur les portables installés par NOOS
//! (voir `noos_core::power`). Un changement fait ailleurs (Paramètres) est suivi : signal
//! D-Bus pour power-profiles-daemon, surveillance de `/opt/auto-cpufreq` pour auto-cpufreq.
//!
//! `NOOS_POWER_DEMO=battery|charging|low|critical|full|plugged|desktop` : données fictives
//! (captures) ; `NOOS_POWER_OPEN=1` ouvre le panneau après 2 s.

use std::{
    cell::{Cell, RefCell},
    f64::consts::PI,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::power::{self as pw, Battery, ChargeState};

use crate::{Shell, apps};

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const PP_IFACES: &[(&str, &str)] = &[
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

const GREEN: (f64, f64, f64) = (0.235, 0.812, 0.447);
const ORANGE: (f64, f64, f64) = (1.0, 0.62, 0.18);
const RED: (f64, f64, f64) = (1.0, 0.32, 0.32);

const CSS: &str = r#"
.power-flyout { padding-top: 20px; }
.power-flyout .power-big { font-size: 2.6em; font-weight: 300; font-feature-settings: "tnum"; }
.power-flyout .power-status { opacity: 0.75; }
.power-flyout .power-title { font-size: 1.25em; font-weight: 600; }
.power-flyout progressbar > trough { min-height: 6px; border-radius: 99px; background-color: alpha(@window_fg_color, 0.10); }
.power-flyout progressbar > trough > progress { min-height: 6px; border-radius: 99px; background-color: @accent_bg_color;
  transition: background-color 300ms ease; }
.power-flyout progressbar.charging > trough > progress { background-color: #3ccf72; }
.power-flyout progressbar.low > trough > progress { background-color: #ff9e2e; }
.power-flyout progressbar.critical > trough > progress { background-color: #ff5252; }
.power-flyout .power-details { background-color: alpha(@window_fg_color, 0.04); border-radius: 8px;
  border: 1px solid alpha(@window_fg_color, 0.06); padding: 4px 12px; }
.power-flyout .power-detail { padding: 6px 0; }
.power-flyout .power-detail + .power-detail { border-top: 1px solid alpha(@window_fg_color, 0.06); }
.power-flyout .power-detail .value { font-feature-settings: "tnum"; opacity: 0.75; }
.power-flyout .tile { transition: background-color 180ms ease, color 180ms ease; padding: 8px 4px; }
.power-flyout .tile image { -gtk-icon-size: 20px; }
.power-flyout .power-desc { font-size: 0.9em; opacity: 0.7; }
.power-flyout .power-warn { font-size: 0.9em; color: #ff9e2e; }
.power-flyout .power-backend { font-size: 0.8em; opacity: 0.55; }
@keyframes noos-power-rise { from { opacity: 0; transform: translateY(10px); } to { opacity: 1; transform: none; } }
.power-flyout.opening > * { animation: noos-power-rise 260ms cubic-bezier(0.2, 0.8, 0.2, 1); }
"#;

// ───────────────────────────── Icône dessinée ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum Look {
    /// Batterie : `alert` 0 normal, 1 faible, 2 critique.
    Battery { charging: bool, plugged: bool, alert: u8 },
    /// Jauge du mode d'alimentation (PC fixe).
    Gauge,
}

/// Icône animée : batterie dont le remplissage suit le niveau, ou jauge dont l'aiguille
/// suit le mode. La valeur glisse vers sa cible (≈ 0,4 s, décélération douce).
struct Glyph {
    area: gtk::DrawingArea,
    look: Cell<Look>,
    /// Batterie : fraction 0–1 ; jauge : 0 (économie), 1 (équilibré), 2 (performances).
    value: Cell<f64>,
    from: Cell<f64>,
    target: Cell<f64>,
    started: Cell<Option<i64>>,
    ticking: Cell<bool>,
}

impl Glyph {
    fn new(w: i32, h: i32) -> Rc<Self> {
        let area = gtk::DrawingArea::new();
        area.set_content_width(w);
        area.set_content_height(h);
        area.set_valign(gtk::Align::Center);
        area.set_halign(gtk::Align::Center);
        area.set_can_target(false);
        let g = Rc::new(Self {
            area,
            look: Cell::new(Look::Gauge),
            value: Cell::new(1.0),
            from: Cell::new(1.0),
            target: Cell::new(1.0),
            started: Cell::new(None),
            ticking: Cell::new(false),
        });
        let w = Rc::downgrade(&g);
        g.area.set_draw_func(move |a, cr, width, height| {
            if let Some(g) = w.upgrade() {
                let fg = a.color();
                let fg = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);
                match g.look.get() {
                    Look::Battery { charging, plugged, alert } => draw_battery(cr, width as f64, height as f64, g.value.get(), fg, charging, plugged, alert),
                    Look::Gauge => draw_gauge(cr, width as f64, height as f64, g.value.get(), fg),
                }
            }
        });
        g
    }

    /// Nouvelle cible ; `animate` = glisser depuis la valeur actuelle.
    fn set(self: &Rc<Self>, look: Look, target: f64, animate: bool) {
        let look_changed = self.look.replace(look) != look;
        if (self.target.get() - target).abs() < 1e-6 && !look_changed {
            return;
        }
        self.target.set(target);
        if !animate || !self.area.is_mapped() {
            self.value.set(target);
            self.area.queue_draw();
            return;
        }
        self.from.set(self.value.get());
        self.started.set(None);
        if self.ticking.replace(true) {
            return;
        }
        let w = Rc::downgrade(self);
        self.area.add_tick_callback(move |_, clock| {
            let Some(g) = w.upgrade() else { return glib::ControlFlow::Break };
            let now = clock.frame_time();
            let t0 = *g.started.get().get_or_insert(now);
            g.started.set(Some(t0));
            let t = ((now - t0) as f64 / 420_000.0).clamp(0.0, 1.0);
            let e = 1.0 - (1.0 - t).powi(3);
            g.value.set(g.from.get() + (g.target.get() - g.from.get()) * e);
            g.area.queue_draw();
            if t >= 1.0 {
                g.ticking.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Rejoue le remplissage depuis zéro (ouverture du panneau).
    fn replay(self: &Rc<Self>) {
        let t = self.target.get();
        self.value.set(0.0);
        self.target.set(-1.0);
        self.set(self.look.get(), t, true);
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Éclair centré en (cx, cy), hauteur h.
fn bolt(cr: &gtk::cairo::Context, cx: f64, cy: f64, h: f64) {
    let s = h / 10.0;
    cr.move_to(cx + 1.2 * s, cy - 5.0 * s);
    cr.line_to(cx - 2.6 * s, cy + 0.8 * s);
    cr.line_to(cx - 0.1 * s, cy + 0.8 * s);
    cr.line_to(cx - 1.2 * s, cy + 5.0 * s);
    cr.line_to(cx + 2.6 * s, cy - 0.8 * s);
    cr.line_to(cx + 0.1 * s, cy - 0.8 * s);
    cr.close_path();
}

/// Prise (branchée, pas en charge) centrée en (cx, cy), hauteur h : deux broches, corps, câble.
fn plug(cr: &gtk::cairo::Context, cx: f64, cy: f64, h: f64) {
    let (bw, bh) = (0.64 * h, 0.40 * h);
    let pw = (0.17 * h).max(1.2);
    let top = cy - 0.5 * h;
    let body_y = top + 0.30 * h;
    rounded(cr, cx - bw / 2.0, body_y, bw, bh, bh * 0.35);
    cr.rectangle(cx - bw * 0.30 - pw / 2.0, top, pw, 0.32 * h);
    cr.rectangle(cx + bw * 0.30 - pw / 2.0, top, pw, 0.32 * h);
    cr.rectangle(cx - pw / 2.0, body_y + bh - 0.02 * h, pw, 0.32 * h);
}

#[allow(clippy::too_many_arguments)]
fn draw_battery(cr: &gtk::cairo::Context, w: f64, h: f64, frac: f64, fg: (f64, f64, f64), charging: bool, plugged: bool, alert: u8) {
    let frac = frac.clamp(0.0, 1.0);
    // Corps horizontal (proportions d'une icône Windows 11) + borne à droite.
    let bh = (h * 0.72).min(w * 0.5);
    let lw = (bh / 9.0).max(1.2);
    let nub = (bh * 0.16).max(1.5);
    let bw = w - nub - lw;
    let (x, y) = (lw / 2.0, (h - bh) / 2.0);
    let r = bh * 0.22;
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.9);
    cr.set_line_width(lw);
    rounded(cr, x, y, bw, bh, r);
    let _ = cr.stroke();
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.55);
    rounded(cr, x + bw + lw * 0.2, y + bh * 0.3, nub, bh * 0.4, nub * 0.45);
    let _ = cr.fill();
    // Remplissage : au moins un trait visible au-dessus de 0 %.
    let pad = lw * 1.25;
    let inner = bw - 2.0 * pad;
    let fill = if frac > 0.0 { (inner * frac).max(lw * 0.9) } else { 0.0 };
    let color = match (charging, alert) {
        (true, _) => GREEN,
        (_, 2) => RED,
        (_, 1) => ORANGE,
        _ => fg,
    };
    cr.set_source_rgb(color.0, color.1, color.2);
    rounded(cr, x + pad, y + pad, fill, bh - 2.0 * pad, (r - pad / 2.0).max(0.5));
    let _ = cr.fill();
    if plugged && bh < 20.0 {
        // Petite icône : la prise, trop fine à l'intérieur, déborde en badge sur la droite
        // (détourée pour se détacher de la batterie).
        let ph = h * 0.86;
        let (px, py) = (w - ph * 0.34, h / 2.0);
        cr.set_operator(gtk::cairo::Operator::Clear);
        cr.set_line_width(lw * 1.3);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        plug(cr, px, py, ph);
        let _ = cr.stroke_preserve();
        let _ = cr.fill();
        cr.set_operator(gtk::cairo::Operator::Over);
        cr.set_source_rgb(fg.0, fg.1, fg.2);
        plug(cr, px, py, ph);
        let _ = cr.fill();
    } else if charging || plugged {
        // Pictogramme découpé dans le remplissage (contour transparent), puis dessiné.
        let (cx, cy) = (x + bw / 2.0, h / 2.0);
        let size = bh * 1.05;
        let shape = |cr: &gtk::cairo::Context| if charging { bolt(cr, cx, cy, size) } else { plug(cr, cx, cy, bh * 0.92) };
        cr.set_operator(gtk::cairo::Operator::Clear);
        cr.set_line_width(lw * if charging { 1.6 } else { 1.1 });
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        shape(cr);
        let _ = cr.stroke_preserve();
        let _ = cr.fill();
        cr.set_operator(gtk::cairo::Operator::Over);
        // Sur un remplissage clair (couleur du texte), la prise prend une teinte contrastée.
        let light = 0.2126 * fg.0 + 0.7152 * fg.1 + 0.0722 * fg.2 > 0.5;
        let ink = if charging { fg } else if light { (0.11, 0.11, 0.13) } else { (0.96, 0.96, 0.97) };
        cr.set_source_rgb(ink.0, ink.1, ink.2);
        shape(cr);
        let _ = cr.fill();
    }
}

/// Jauge : arc gradué, partie colorée jusqu'à l'aiguille (vert, couleur du texte, orange).
fn draw_gauge(cr: &gtk::cairo::Context, w: f64, h: f64, pos: f64, fg: (f64, f64, f64)) {
    let pos = pos.clamp(0.0, 2.0);
    let r = (w / 2.0).min(h * 0.82) * 0.86;
    let lw = (r / 4.2).max(1.4);
    let (cx, cy) = (w / 2.0, h / 2.0 + r * 0.42);
    let a0 = PI * 1.12;
    let a1 = PI * 1.88;
    let angle = a0 + (a1 - a0) * pos / 2.0;
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_width(lw);
    cr.set_source_rgba(fg.0, fg.1, fg.2, 0.28);
    cr.arc(cx, cy, r, a0, a1);
    let _ = cr.stroke();
    // Couleur interpolée : vert → texte → orange.
    let mix = |a: (f64, f64, f64), b: (f64, f64, f64), t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, a.2 + (b.2 - a.2) * t);
    let c = if pos <= 1.0 { mix(GREEN, fg, pos) } else { mix(fg, ORANGE, pos - 1.0) };
    cr.set_source_rgb(c.0, c.1, c.2);
    cr.arc(cx, cy, r, a0, angle.max(a0 + 0.01));
    let _ = cr.stroke();
    // Aiguille et pivot.
    cr.set_source_rgb(fg.0, fg.1, fg.2);
    cr.set_line_width((lw * 0.75).max(1.2));
    cr.move_to(cx, cy);
    cr.line_to(cx + angle.cos() * r * 0.78, cy + angle.sin() * r * 0.78);
    let _ = cr.stroke();
    cr.arc(cx, cy, lw * 0.85, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

fn profile_pos(p: &str) -> f64 {
    match p {
        "power-saver" => 0.0,
        "performance" => 2.0,
        _ => 1.0,
    }
}

fn battery_look(b: &Battery) -> Look {
    Look::Battery {
        charging: b.state == ChargeState::Charging,
        plugged: matches!(b.state, ChargeState::Full | ChargeState::PendingCharge),
        alert: if b.critical() { 2 } else if b.low() { 1 } else { 0 },
    }
}

// ───────────────────────────── Modèle ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Backend {
    /// Aucun gestionnaire de modes.
    None,
    Ppd { dest: &'static str, path: &'static str },
    AutoCpufreq,
    /// Démonstration (captures) : rien n'est appliqué.
    Demo,
}

impl Backend {
    fn name(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Ppd { .. } => "power-profiles-daemon",
            Self::AutoCpufreq => "auto-cpufreq",
            Self::Demo => "démonstration",
        }
    }
}

/// Panneau (construit une fois).
struct Flyout {
    window: gtk::Window,
    root: gtk::Box,
    glyph: Rc<Glyph>,
    big: gtk::Label,
    status: gtk::Label,
    bar: gtk::ProgressBar,
    details: gtk::Box,
    tiles: Vec<(String, gtk::ToggleButton)>,
    tiles_box: gtk::Box,
    desc: gtk::Label,
    warn: gtk::Label,
    unavailable: gtk::Label,
    backend: gtk::Label,
    hidden_at: Cell<Option<Instant>>,
}

pub struct Power {
    shell: Weak<Shell>,
    conn: Option<gio::DBusConnection>,
    demo: bool,
    battery: RefCell<Option<Battery>>,
    /// Détails de la batterie physique (santé, modèle, cycles), lus à l'ouverture du panneau.
    details: RefCell<Option<Battery>>,
    backend: RefCell<Backend>,
    profiles: RefCell<Vec<String>>,
    active: RefCell<String>,
    degraded: RefCell<String>,
    buttons: RefCell<Vec<(gtk::Button, Rc<Glyph>)>>,
    flyout: RefCell<Option<Rc<Flyout>>>,
    /// Alerte « batterie faible » déjà envoyée pour cette décharge.
    warned: Cell<bool>,
    _monitor: RefCell<Option<gio::FileMonitor>>,
    syncing: Cell<bool>,
}

fn dict(v: &glib::Variant) -> glib::VariantDict {
    glib::VariantDict::new(Some(&v.child_value(0)))
}

fn get<T: glib::variant::FromVariant>(d: &glib::VariantDict, k: &str) -> Option<T> {
    d.lookup_value(k, None).and_then(|v| v.get())
}

fn get_all(conn: &gio::DBusConnection, dest: &str, path: &str, iface: &str, f: impl FnOnce(Option<glib::VariantDict>) + 'static) {
    conn.call(
        Some(dest),
        path,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(iface,).to_variant()),
        Some(glib::VariantTy::new("(a{sv})").unwrap()),
        gio::DBusCallFlags::NONE,
        3000,
        gio::Cancellable::NONE,
        move |r| f(r.ok().map(|v| dict(&v))),
    );
}

/// Abonnement à `PropertiesChanged` d'un objet (relecture regroupée après 150 ms).
fn watch(conn: &gio::DBusConnection, dest: &str, path: &str, f: impl Fn() + 'static) {
    let f = Rc::new(f);
    let pending = Rc::new(Cell::new(false));
    #[allow(deprecated)]
    let id = conn.signal_subscribe(Some(dest), Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged"), Some(path), None, gio::DBusSignalFlags::NONE, move |_, _, _, _, _, _| {
        if pending.replace(true) {
            return;
        }
        let (f, pending) = (f.clone(), pending.clone());
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            pending.set(false);
            f();
        });
    });
    std::mem::forget(id);
}

fn battery_from_upower(d: &glib::VariantDict) -> Option<Battery> {
    // Type 2 = batterie ; IsPresent faux sur un PC fixe (DisplayDevice vide).
    if get::<u32>(d, "Type") != Some(2) || get::<bool>(d, "IsPresent") != Some(true) {
        return None;
    }
    Some(Battery {
        percent: get(d, "Percentage").unwrap_or(0.0),
        state: ChargeState::from_upower(get(d, "State").unwrap_or(0)),
        time_to_empty: get(d, "TimeToEmpty").unwrap_or(0),
        time_to_full: get(d, "TimeToFull").unwrap_or(0),
        rate: get(d, "EnergyRate").unwrap_or(0.0),
        ..Default::default()
    })
}

/// Notification par le serveur du shell (appel asynchrone).
fn notify(summary: &str, body: &str, icon: &str, critical: bool) {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    let hints = glib::VariantDict::new(None);
    hints.insert("urgency", if critical { 2u8 } else { 1u8 });
    let args = ("Énergie", 0u32, icon, summary, body, Vec::<String>::new(), hints.end(), 10000i32).to_variant();
    conn.call(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
        "Notify",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        5000,
        gio::Cancellable::NONE,
        |_| {},
    );
}

impl Power {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            // Au-dessus du style de la barre et des panneaux (USER + 10).
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 20);
        }
        let demo = pw::demo();
        let conn = if demo.is_some() { None } else { gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok() };
        // GIO ne garde qu'une référence faible sur le bus système : sans celle-ci, la connexion
        // se ferme et les signaux n'arrivent plus, sans erreur.
        if let Some(c) = &conn {
            std::mem::forget(c.clone());
        }
        let p = Rc::new(Self {
            shell: Rc::downgrade(shell),
            conn,
            demo: demo.is_some(),
            battery: RefCell::new(demo.clone().flatten()),
            details: RefCell::new(demo.flatten()),
            backend: RefCell::new(Backend::None),
            profiles: RefCell::new(vec![]),
            active: RefCell::new("balanced".into()),
            degraded: RefCell::new(String::new()),
            buttons: RefCell::new(vec![]),
            flyout: RefCell::new(None),
            warned: Cell::new(false),
            _monitor: RefCell::new(None),
            syncing: Cell::new(false),
        });
        if p.demo {
            *p.backend.borrow_mut() = Backend::Demo;
            *p.profiles.borrow_mut() = pw::PROFILES.iter().map(|s| s.to_string()).collect();
            if let Ok(v) = std::env::var("NOOS_POWER_DEMO") {
                if let Some(prof) = v.split([';', ',']).find(|s| pw::PROFILES.contains(s)) {
                    *p.active.borrow_mut() = prof.to_string();
                }
            }
        } else {
            p.start_battery();
            p.start_profiles(0);
        }
        if std::env::var_os("NOOS_POWER_OPEN").is_some() {
            let w = Rc::downgrade(&p);
            glib::timeout_add_local_once(Duration::from_millis(2000), move || {
                if let Some(p) = w.upgrade() {
                    p.toggle();
                }
            });
        }
        p
    }

    // ── Batterie ──

    fn start_battery(self: &Rc<Self>) {
        let Some(conn) = self.conn.clone() else {
            self.sysfs_fallback();
            return;
        };
        let w = Rc::downgrade(self);
        get_all(&conn, UPOWER, DISPLAY_DEVICE, "org.freedesktop.UPower.Device", move |d| {
            let Some(p) = w.upgrade() else { return };
            match d {
                Some(d) => {
                    p.set_battery(battery_from_upower(&d));
                    let w = Rc::downgrade(&p);
                    if let Some(conn) = &p.conn {
                        watch(conn, UPOWER, DISPLAY_DEVICE, move || {
                            if let Some(p) = w.upgrade() {
                                p.reload_battery();
                            }
                        });
                    }
                }
                None => p.sysfs_fallback(),
            }
        });
    }

    fn reload_battery(self: &Rc<Self>) {
        let Some(conn) = &self.conn else { return };
        let w = Rc::downgrade(self);
        get_all(conn, UPOWER, DISPLAY_DEVICE, "org.freedesktop.UPower.Device", move |d| {
            if let (Some(p), Some(d)) = (w.upgrade(), d) {
                p.set_battery(battery_from_upower(&d));
            }
        });
    }

    /// UPower absent : lecture du noyau, relue toutes les 60 s seulement s'il y a une batterie.
    fn sysfs_fallback(self: &Rc<Self>) {
        let b = pw::read_sysfs();
        let has = b.is_some();
        self.set_battery(b.clone());
        *self.details.borrow_mut() = b;
        if has {
            let w = Rc::downgrade(self);
            glib::timeout_add_seconds_local(60, move || {
                let Some(p) = w.upgrade() else { return glib::ControlFlow::Break };
                let b = pw::read_sysfs();
                *p.details.borrow_mut() = b.clone();
                p.set_battery(b);
                glib::ControlFlow::Continue
            });
        }
    }

    /// Santé, modèle et cycles : batterie physique d'UPower (le DisplayDevice ne les a pas).
    fn load_details(self: &Rc<Self>) {
        let Some(conn) = self.conn.clone() else { return };
        let w = Rc::downgrade(self);
        conn.call(
            Some(UPOWER),
            "/org/freedesktop/UPower",
            UPOWER,
            "EnumerateDevices",
            None,
            Some(glib::VariantTy::new("(ao)").unwrap()),
            gio::DBusCallFlags::NONE,
            3000,
            gio::Cancellable::NONE,
            move |r| {
                let Ok(v) = r else { return };
                let paths: Vec<String> = v.child_value(0).iter().filter_map(|p| p.str().map(String::from)).collect();
                let Some(conn) = w.upgrade().and_then(|p| p.conn.clone()) else { return };
                for path in paths.into_iter().filter(|p| p.contains("battery_")) {
                    let w = w.clone();
                    get_all(&conn, UPOWER, &path, "org.freedesktop.UPower.Device", move |d| {
                        let (Some(p), Some(d)) = (w.upgrade(), d) else { return };
                        if get::<u32>(&d, "Type") != Some(2) || get::<bool>(&d, "PowerSupply") != Some(true) {
                            return;
                        }
                        let model = [get::<String>(&d, "Vendor"), get::<String>(&d, "Model")].into_iter().flatten().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" ");
                        let b = Battery {
                            capacity: get(&d, "Capacity").unwrap_or(0.0),
                            cycles: get::<i32>(&d, "ChargeCycles").filter(|c| *c > 0).unwrap_or(0) as u32,
                            model,
                            ..Default::default()
                        };
                        *p.details.borrow_mut() = Some(b);
                        p.update_flyout(false);
                    });
                }
            },
        );
    }

    fn set_battery(self: &Rc<Self>, b: Option<Battery>) {
        // Alerte à 10 % (une fois par décharge), comme Windows.
        if let Some(b) = &b {
            if b.critical() && !self.warned.replace(true) {
                notify(
                    "Batterie presque vide",
                    &format!("{:.0} % restants ({}). Branchez votre ordinateur.", b.percent, b.status_text().to_lowercase()),
                    "battery-level-10-symbolic",
                    true,
                );
            } else if b.state.plugged() {
                self.warned.set(false);
            }
        }
        *self.battery.borrow_mut() = b;
        self.refresh(true);
    }

    // ── Modes d'alimentation ──

    fn start_profiles(self: &Rc<Self>, i: usize) {
        let Some(conn) = self.conn.clone() else { return };
        let Some(&(dest, path)) = PP_IFACES.get(i) else {
            self.start_acf();
            return;
        };
        let w = Rc::downgrade(self);
        get_all(&conn, dest, path, dest, move |d| {
            let Some(p) = w.upgrade() else { return };
            match d {
                Some(d) => {
                    *p.backend.borrow_mut() = Backend::Ppd { dest, path };
                    p.apply_ppd(&d);
                    let w = Rc::downgrade(&p);
                    let Some(conn) = p.conn.clone() else { return };
                    watch(&conn, dest, path, move || {
                        let Some(p) = w.upgrade() else { return };
                        let Some(conn) = p.conn.clone() else { return };
                        let w = Rc::downgrade(&p);
                        get_all(&conn, dest, path, dest, move |d| {
                            if let (Some(p), Some(d)) = (w.upgrade(), d) {
                                p.apply_ppd(&d);
                            }
                        });
                    });
                }
                None => p.start_profiles(i + 1),
            }
        });
    }

    fn apply_ppd(self: &Rc<Self>, d: &glib::VariantDict) {
        let list: Vec<String> = d
            .lookup_value("Profiles", None)
            .map(|v| v.iter().filter_map(|e| glib::VariantDict::new(Some(&e)).lookup_value("Profile", None).and_then(|s| s.get::<String>())).collect())
            .unwrap_or_default();
        *self.profiles.borrow_mut() = pw::PROFILES.iter().filter(|p| list.iter().any(|l| l == *p)).map(|s| s.to_string()).collect();
        *self.active.borrow_mut() = get(d, "ActiveProfile").unwrap_or_else(|| "balanced".into());
        *self.degraded.borrow_mut() = get(d, "PerformanceDegraded").unwrap_or_default();
        self.refresh(true);
    }

    /// Sans power-profiles-daemon : auto-cpufreq (portables installés par NOOS).
    fn start_acf(self: &Rc<Self>) {
        if !pw::acf_running() {
            self.refresh(false);
            return;
        }
        *self.backend.borrow_mut() = Backend::AutoCpufreq;
        *self.profiles.borrow_mut() = pw::PROFILES.iter().map(|s| s.to_string()).collect();
        self.read_acf();
        // Changement fait ailleurs (Paramètres, terminal) : le fichier de choix forcé change.
        if let Ok(m) = gio::File::for_path(pw::ACF_DIR).monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            let w = Rc::downgrade(self);
            m.connect_changed(move |_, _, _, _| {
                if let Some(p) = w.upgrade() {
                    p.read_acf();
                }
            });
            *self._monitor.borrow_mut() = Some(m);
        }
    }

    fn read_acf(self: &Rc<Self>) {
        let bytes = std::fs::read(pw::ACF_OVERRIDE).ok();
        *self.active.borrow_mut() = pw::acf_profile(bytes.as_deref()).to_string();
        self.refresh(true);
    }

    fn set_profile(self: &Rc<Self>, profile: &str) {
        if *self.active.borrow() == profile {
            return;
        }
        let previous = self.active.replace(profile.to_string());
        self.refresh(true);
        let backend = self.backend.borrow().clone();
        match backend {
            Backend::Ppd { dest, path } => {
                let Some(conn) = &self.conn else { return };
                let args = (dest, "ActiveProfile", profile.to_variant()).to_variant();
                let w = Rc::downgrade(self);
                conn.call(
                    Some(dest),
                    path,
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    Some(&args),
                    None,
                    gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                    10000,
                    gio::Cancellable::NONE,
                    move |r| {
                        if let (Err(e), Some(p)) = (r, w.upgrade()) {
                            eprintln!("noos-shell : mode d'alimentation : {e}");
                            *p.active.borrow_mut() = previous;
                            p.refresh(true);
                        }
                    },
                );
            }
            Backend::AutoCpufreq => {
                let profile = profile.to_string();
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let ok = std::process::Command::new("pkexec").args([pw::HELPER, "profile", &profile]).status().is_ok_and(|s| s.success());
                    let _ = tx.send_blocking(ok);
                });
                let w = Rc::downgrade(self);
                glib::spawn_future_local(async move {
                    let ok = rx.recv().await.unwrap_or(false);
                    if let Some(p) = w.upgrade() {
                        if !ok {
                            eprintln!("noos-shell : auto-cpufreq : changement de mode refusé");
                        }
                        // Le fichier fait foi (réussite ou non).
                        p.read_acf();
                    }
                });
            }
            Backend::Demo | Backend::None => {}
        }
    }

    /// Mode voisin (molette sur l'icône).
    fn step_profile(self: &Rc<Self>, delta: i32) {
        let list = self.profiles.borrow().clone();
        let Some(i) = list.iter().position(|p| *p == *self.active.borrow()) else { return };
        let j = (i as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
        if j != i {
            self.set_profile(&list[j]);
        }
    }

    // ── Barre ──

    /// Bouton pour une barre des tâches (un par barre / écran).
    pub fn button(self: &Rc<Self>) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("status-icon");
        b.add_css_class("power-icon");
        b.set_valign(gtk::Align::Center);
        let glyph = Glyph::new(24, 16);
        b.set_child(Some(&glyph.area));
        {
            let w = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    if let Some(s) = p.shell.upgrade() {
                        s.close_flyouts_except("power");
                    }
                    p.toggle();
                }
            });
        }
        {
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
            let w = Rc::downgrade(self);
            scroll.connect_scroll(move |_, _, dy| {
                if let Some(p) = w.upgrade() {
                    p.step_profile(if dy < 0.0 { 1 } else { -1 });
                }
                glib::Propagation::Stop
            });
            b.add_controller(scroll);
        }
        self.buttons.borrow_mut().retain(|(b, _)| b.root().is_some());
        self.buttons.borrow_mut().push((b.clone(), glyph));
        self.refresh(false);
        b
    }

    fn profile_tip(&self) -> String {
        let active = self.active.borrow();
        let mut s = format!("Mode d'alimentation : {}", pw::profile_label(&active));
        if !self.profiles.borrow().is_empty() {
            s.push_str("\nMolette : changer de mode");
        }
        s
    }

    /// Met à jour les icônes des barres et le panneau.
    fn refresh(self: &Rc<Self>, animate: bool) {
        let bat = self.battery.borrow().clone();
        let has_profiles = *self.backend.borrow() != Backend::None && !self.profiles.borrow().is_empty();
        for (b, g) in self.buttons.borrow().iter() {
            match &bat {
                Some(bat) => {
                    g.area.set_content_width(24);
                    g.set(battery_look(bat), bat.percent / 100.0, animate);
                    let mut tip = bat.tooltip();
                    if has_profiles {
                        tip.push_str(&format!("\nMode : {}", pw::profile_label(&self.active.borrow())));
                    }
                    b.set_tooltip_text(Some(&tip));
                    b.set_visible(true);
                }
                None => {
                    g.area.set_content_width(20);
                    g.set(Look::Gauge, profile_pos(&self.active.borrow()), animate);
                    b.set_tooltip_text(Some(&self.profile_tip()));
                    // Rien à montrer sans batterie ni modes d'alimentation.
                    b.set_visible(has_profiles);
                }
            }
        }
        self.update_flyout(animate);
    }

    // ── Panneau ──

    fn build_flyout(self: &Rc<Self>) -> Rc<Flyout> {
        let shell = self.shell.upgrade().expect("shell");
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-quicksettings"));
        window.set_layer(Layer::Top);
        crate::editmode::anchor_to_bar(&window, &shell.config());
        shell.flyouts.borrow_mut().push(window.downgrade());
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Right, 12);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
        root.add_css_class("noos-flyout");
        root.add_css_class("quick");
        root.add_css_class("power-flyout");
        root.set_size_request(380, -1);

        // En-tête : grande icône, pourcentage (ou mode), état.
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let glyph = Glyph::new(76, 44);
        head.append(&glyph.area);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        texts.set_valign(gtk::Align::Center);
        let big = gtk::Label::new(None);
        big.set_xalign(0.0);
        let status = gtk::Label::new(None);
        status.add_css_class("power-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        texts.append(&big);
        texts.append(&status);
        head.append(&texts);
        root.append(&head);

        let bar = gtk::ProgressBar::new();
        root.append(&bar);

        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.add_css_class("power-details");
        root.append(&details);

        let title = gtk::Label::new(Some("Mode d'alimentation"));
        title.add_css_class("section-title");
        title.set_xalign(0.0);
        root.append(&title);
        let tiles_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tiles_box.set_homogeneous(true);
        let mut tiles = vec![];
        let mut first: Option<gtk::ToggleButton> = None;
        for p in pw::PROFILES {
            let b = gtk::ToggleButton::new();
            b.add_css_class("tile");
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            v.append(&gtk::Image::from_icon_name(&format!("power-profile-{p}-symbolic")));
            v.append(&gtk::Label::new(Some(pw::profile_label(p))));
            b.set_child(Some(&v));
            b.set_hexpand(true);
            match &first {
                Some(f) => b.set_group(Some(f)),
                None => first = Some(b.clone()),
            }
            let (w, name) = (Rc::downgrade(self), p.to_string());
            b.connect_toggled(move |b| {
                if let Some(pw_) = w.upgrade() {
                    if b.is_active() && !pw_.syncing.get() {
                        pw_.set_profile(&name);
                    }
                }
            });
            tiles_box.append(&b);
            tiles.push((p.to_string(), b));
        }
        root.append(&tiles_box);
        let desc = gtk::Label::new(None);
        desc.add_css_class("power-desc");
        desc.set_xalign(0.0);
        desc.set_wrap(true);
        root.append(&desc);
        let warn = gtk::Label::new(None);
        warn.add_css_class("power-warn");
        warn.set_xalign(0.0);
        warn.set_wrap(true);
        root.append(&warn);
        let unavailable = gtk::Label::new(Some("Modes d'alimentation indisponibles : le service power-profiles-daemon ne répond pas."));
        unavailable.add_css_class("power-desc");
        unavailable.set_xalign(0.0);
        unavailable.set_wrap(true);
        root.append(&unavailable);

        // Pied : raccourci vers Paramètres › Alimentation, gestionnaire utilisé.
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("footer");
        let settings = gtk::Button::new();
        let sc = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        sc.append(&gtk::Image::from_icon_name("emblem-system-symbolic"));
        sc.append(&gtk::Label::new(Some("Paramètres d'alimentation")));
        settings.set_child(Some(&sc));
        {
            let w = Rc::downgrade(self);
            settings.connect_clicked(move |_| {
                if let Some(p) = w.upgrade() {
                    p.hide();
                }
                apps::spawn("noos-settings power");
            });
        }
        footer.append(&settings);
        let backend = gtk::Label::new(None);
        backend.add_css_class("power-backend");
        backend.set_hexpand(true);
        backend.set_xalign(1.0);
        footer.append(&backend);
        root.append(&footer);
        window.set_child(Some(&root));

        let f = Rc::new(Flyout {
            window,
            root,
            glyph,
            big,
            status,
            bar,
            details,
            tiles,
            tiles_box,
            desc,
            warn,
            unavailable,
            backend,
            hidden_at: Cell::new(None),
        });
        {
            let w = Rc::downgrade(self);
            crate::close_on_focus_loss(&f.window, move || {
                if let Some(p) = w.upgrade() {
                    p.hide();
                }
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(self);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    if let Some(p) = w.upgrade() {
                        p.hide();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            f.window.add_controller(keys);
        }
        f
    }

    fn update_flyout(self: &Rc<Self>, animate: bool) {
        let Some(f) = self.flyout.borrow().clone() else { return };
        let bat = self.battery.borrow().clone();
        let active = self.active.borrow().clone();
        let backend = self.backend.borrow().clone();
        let profiles = self.profiles.borrow().clone();
        match &bat {
            Some(b) => {
                f.glyph.area.set_content_width(76);
                f.glyph.set(battery_look(b), b.percent / 100.0, animate);
                f.big.set_text(&format!("{:.0} %", b.percent));
                f.big.remove_css_class("power-title");
                f.big.add_css_class("power-big");
                f.status.set_text(&b.status_text());
                f.bar.set_fraction(b.percent / 100.0);
                for c in ["charging", "low", "critical"] {
                    f.bar.remove_css_class(c);
                }
                if b.state == ChargeState::Charging {
                    f.bar.add_css_class("charging");
                } else if b.critical() {
                    f.bar.add_css_class("critical");
                } else if b.low() {
                    f.bar.add_css_class("low");
                }
                f.bar.set_visible(true);
            }
            None => {
                f.glyph.area.set_content_width(64);
                f.glyph.set(Look::Gauge, profile_pos(&active), animate);
                f.big.set_text(if backend == Backend::None { "Alimentation secteur" } else { pw::profile_label(&active) });
                f.big.remove_css_class("power-big");
                f.big.add_css_class("power-title");
                f.status.set_text(if backend == Backend::None { "Aucune batterie détectée" } else { "Sur secteur · aucune batterie" });
                f.bar.set_visible(false);
            }
        }

        // Détails de la batterie.
        while let Some(c) = f.details.first_child() {
            f.details.remove(&c);
        }
        let mut rows: Vec<(&str, String, String)> = vec![];
        if let Some(b) = &bat {
            let det = self.details.borrow().clone().unwrap_or_default();
            if det.capacity > 0.0 {
                let verdict = match det.capacity {
                    c if c >= 80.0 => "bonne",
                    c if c >= 60.0 => "correcte",
                    _ => "usée",
                };
                rows.push(("battery-good-symbolic", "Santé de la batterie".into(), format!("{:.0} % · {verdict}", det.capacity)));
            }
            if b.rate > 0.1 && b.state != ChargeState::Full {
                let label = if b.state == ChargeState::Charging { "Puissance de charge" } else { "Consommation" };
                rows.push(("utilities-system-monitor-symbolic", label.into(), format!("{:.1} W", b.rate).replace('.', ",")));
            }
            if det.cycles > 0 {
                rows.push(("view-refresh-symbolic", "Cycles de charge".into(), det.cycles.to_string()));
            }
            if !det.model.is_empty() {
                rows.push(("drive-harddisk-symbolic", "Batterie".into(), det.model.clone()));
            }
        }
        for (icon, label, value) in &rows {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            r.add_css_class("power-detail");
            let i = gtk::Image::from_icon_name(icon);
            i.add_css_class("dim-label");
            r.append(&i);
            let l = gtk::Label::new(Some(label));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            r.append(&l);
            let v = gtk::Label::new(Some(value));
            v.add_css_class("value");
            v.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.set_max_width_chars(22);
            r.append(&v);
            f.details.append(&r);
        }
        f.details.set_visible(!rows.is_empty());

        // Modes.
        self.syncing.set(true);
        for (name, b) in &f.tiles {
            b.set_visible(profiles.contains(name));
            b.set_active(*name == active);
        }
        self.syncing.set(false);
        let available = backend != Backend::None && !profiles.is_empty();
        f.tiles_box.set_visible(available);
        f.desc.set_visible(available);
        f.desc.set_text(pw::profile_description(&active, backend == Backend::AutoCpufreq));
        let degraded = self.degraded.borrow().clone();
        f.warn.set_visible(available && active == "performance" && !degraded.is_empty());
        f.warn.set_text(match degraded.as_str() {
            "lap-detected" => "Performances réduites : l'ordinateur est posé sur les genoux.",
            "high-operating-temperature" => "Performances réduites : température élevée.",
            _ => "Performances réduites par le système.",
        });
        f.unavailable.set_visible(!available);
        f.backend.set_text(&match backend {
            Backend::Demo => "Démonstration".to_string(),
            _ if available => format!("Géré par {}", backend.name()),
            _ => String::new(),
        });
        f.window.set_default_size(1, 1);
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.flyout.borrow().is_none() {
            let f = self.build_flyout();
            *self.flyout.borrow_mut() = Some(f);
        }
        let Some(f) = self.flyout.borrow().clone() else { return };
        if f.hidden_at.get().is_some_and(|t| t.elapsed().as_millis() < 250) {
            return;
        }
        if f.window.is_visible() {
            self.hide();
            return;
        }
        if !self.demo && self.battery.borrow().is_some() {
            self.load_details();
        }
        self.update_flyout(false);
        // Apparition : contenu qui monte en fondu, icône qui se remplit.
        f.root.remove_css_class("opening");
        f.window.present();
        f.root.add_css_class("opening");
        let g = f.glyph.clone();
        glib::idle_add_local_once(move || g.replay());
        let r = f.root.clone();
        glib::timeout_add_local_once(Duration::from_millis(400), move || r.remove_css_class("opening"));
    }

    pub fn hide(&self) {
        if let Some(f) = self.flyout.borrow().as_ref() {
            if f.window.is_visible() {
                f.window.set_visible(false);
                f.hidden_at.set(Some(Instant::now()));
            }
        }
    }
}

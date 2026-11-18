//! Catalogue des widgets du bureau : description (galerie), contenu et réglages propres à
//! chaque widget. L'hôte (cadre, déplacement, taille, style) est dans `desk.rs`.
//!
//! Chaque widget construit son contenu à partir de sa configuration (`DesktopWidget`) ; un
//! réglage modifié passe par [`Ctx::edit`] (annulable) puis le contenu est reconstruit, dans le
//! même cadre.

mod calendar;
mod clock;
mod countdown;
mod folders;
mod media;
mod note;
mod photo;
mod weather;
mod worldclock;

use std::rc::{Rc, Weak};

use gtk::{glib, prelude::*};
use noos_core::config::DesktopWidget;

use crate::Shell;

pub struct Spec {
    pub kind: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub category: &'static str,
    /// Style quand la configuration n'en précise pas (`glass`, `solid`, `clear`).
    pub default_style: &'static str,
    /// Taille approximative (moyenne), pour placer un nouveau widget sans chevauchement.
    pub approx: (f64, f64),
}

pub const CATEGORIES: &[(&str, &str)] = &[("all", "Tous"), ("time", "Heure et dates"), ("info", "Infos"), ("media", "Multimédia"), ("tools", "Pratique"), ("system", "Système")];

pub const SPECS: &[Spec] = &[
    Spec { kind: "clock", name: "Horloge", description: "L'heure et la date, en chiffres ou sur un cadran", icon: "preferences-system-time-symbolic", category: "time", default_style: "clear", approx: (260.0, 130.0) },
    Spec { kind: "worldclock", name: "Horloges du monde", description: "L'heure de plusieurs villes, de jour comme de nuit", icon: "web-browser-symbolic", category: "time", default_style: "glass", approx: (300.0, 220.0) },
    Spec { kind: "calendar", name: "Calendrier", description: "Le mois en cours, semaines et jours fériés", icon: "x-office-calendar-symbolic", category: "time", default_style: "glass", approx: (300.0, 320.0) },
    Spec { kind: "countdown", name: "Compte à rebours", description: "Les jours qui restent avant un événement", icon: "alarm-symbolic", category: "time", default_style: "glass", approx: (320.0, 150.0) },
    Spec { kind: "weather", name: "Météo", description: "Conditions actuelles et prévisions sur 5 jours", icon: "weather-few-clouds-symbolic", category: "info", default_style: "glass", approx: (330.0, 200.0) },
    Spec { kind: "media", name: "Lecteur multimédia", description: "Ce qui joue en ce moment : pochette, titre, commandes", icon: "audio-x-generic-symbolic", category: "media", default_style: "glass", approx: (340.0, 150.0) },
    Spec { kind: "photo", name: "Cadre photo", description: "Les images d'un dossier, en fondu enchaîné", icon: "image-x-generic-symbolic", category: "media", default_style: "glass", approx: (320.0, 210.0) },
    Spec { kind: "note", name: "Note", description: "Un pense-bête toujours sous les yeux", icon: "accessories-text-editor-symbolic", category: "tools", default_style: "glass", approx: (240.0, 200.0) },
    Spec { kind: "folders", name: "Dossiers", description: "Vos dossiers favoris à portée de clic", icon: "folder-symbolic", category: "tools", default_style: "glass", approx: (300.0, 200.0) },
    Spec { kind: "vitals", name: "Ressources", description: "Processeur, carte graphique, mémoire, disque, réseau", icon: "power-profile-performance-symbolic", category: "system", default_style: "glass", approx: (380.0, 100.0) },
];

pub fn spec(kind: &str) -> &'static Spec {
    SPECS.iter().find(|s| s.kind == kind).unwrap_or(&SPECS[7])
}

/// Style effectif d'un widget (celui de la configuration, sinon celui du widget).
pub fn style_of(w: &DesktopWidget) -> &str {
    if w.style.is_empty() { spec(&w.kind).default_style } else { w.style() }
}

/// Accès d'un widget à sa configuration.
#[derive(Clone)]
pub struct Ctx {
    pub shell: Weak<Shell>,
    pub id: String,
}

impl Ctx {
    /// Configuration actuelle du widget.
    pub fn conf(&self) -> Option<DesktopWidget> {
        self.shell.upgrade()?.config().desktop_widgets.into_iter().find(|w| w.id == self.id)
    }

    /// Modifie le widget (annulable, message `label`).
    pub fn edit(&self, label: &str, f: impl FnOnce(&mut DesktopWidget)) {
        let Some(s) = self.shell.upgrade() else { return };
        let id = self.id.clone();
        s.edit_layout(label, move |c| {
            if let Some(w) = c.desktop_widgets.iter_mut().find(|w| w.id == id) {
                f(w);
            }
        });
    }

    pub fn set_option(&self, label: &str, key: &str, value: &str) {
        let (k, v) = (key.to_string(), value.to_string());
        self.edit(label, move |w| {
            w.options.insert(k, v);
        });
    }

    /// Enregistre sans historique ni message (texte d'une note, saisie en cours).
    pub fn save_quietly(&self, f: impl FnOnce(&mut DesktopWidget)) {
        let Some(s) = self.shell.upgrade() else { return };
        let mut cfg = s.config();
        if let Some(w) = cfg.desktop_widgets.iter_mut().find(|w| w.id == self.id) {
            f(w);
            let _ = cfg.save();
        }
    }

    /// Ouvre les réglages du widget (bouton « Choisir une ville »…).
    pub fn open_settings(&self) {
        if let Some(s) = self.shell.upgrade() {
            crate::desk::open_settings(&s, &self.id);
        }
    }
}

/// Contenu du widget.
pub fn build(ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    match w.kind.as_str() {
        "clock" => clock::build(ctx, w),
        "worldclock" => worldclock::build(ctx, w),
        "calendar" => calendar::build(ctx, w),
        "countdown" => countdown::build(ctx, w),
        "weather" => weather::build(ctx, w),
        "media" => media::build(ctx, w),
        "photo" => photo::build(ctx, w),
        "folders" => folders::build(ctx, w),
        "vitals" => vitals(),
        _ => note::build(ctx, w),
    }
}

/// Réglages propres au widget, ajoutés au panneau ⚙.
pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    match w.kind.as_str() {
        "clock" => clock::settings(ctx, w, b),
        "worldclock" => worldclock::settings(ctx, w, b),
        "calendar" => calendar::settings(ctx, w, b),
        "countdown" => countdown::settings(ctx, w, b),
        "weather" => weather::settings(ctx, w, b),
        "photo" => photo::settings(ctx, w, b),
        "folders" => folders::settings(ctx, w, b),
        "note" => note::settings(ctx, w, b),
        _ => {}
    }
}

/// Options par défaut d'un widget neuf (compte à rebours : prochain nouvel an…).
pub fn defaults(w: &mut DesktopWidget) {
    if w.kind == "countdown" {
        countdown::defaults(w);
    }
}

fn vitals() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("desk-vitals");
    let t = gtk::Label::new(Some("Ressources"));
    t.add_css_class("desk-title");
    t.set_xalign(0.0);
    b.append(&t);
    let v = noos_widget_vitals::Vitals::new();
    v.widget().set_sensitive(true);
    b.append(v.widget());
    // Le widget vit tant que la boîte existe (ses minuteries vérifient leur widget).
    let keep = Rc::new(v);
    b.connect_destroy(move |_| {
        let _ = &keep;
    });
    b.upcast()
}

// ───────────────────────────── Petits éléments communs ─────────────────────────────

/// Titre de section du panneau de réglages.
pub fn section(b: &gtk::Box, title: &str) {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("desk-set-title");
    l.set_xalign(0.0);
    b.append(&l);
}

/// Boutons liés (choix exclusif) : `(valeur, libellé)`, `current` coché ; `f(valeur)` au clic.
pub fn choice(b: &gtk::Box, items: &[(&'static str, &'static str)], current: &str, f: impl Fn(&'static str) + 'static) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("linked");
    row.add_css_class("desk-choice");
    let f = Rc::new(f);
    let mut first: Option<gtk::ToggleButton> = None;
    for (value, label) in items {
        let t = gtk::ToggleButton::with_label(label);
        t.set_hexpand(true);
        if let Some(g) = &first {
            t.set_group(Some(g));
        } else {
            first = Some(t.clone());
        }
        t.set_active(*value == current);
        let (f, value) = (f.clone(), *value);
        t.connect_toggled(move |t| {
            if t.is_active() {
                f(value);
            }
        });
        row.append(&t);
    }
    b.append(&row);
}

/// Interrupteur avec libellé.
pub fn switch(b: &gtk::Box, label: &str, on: bool, f: impl Fn(bool) + 'static) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = gtk::Label::new(Some(label));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    row.append(&l);
    let s = gtk::Switch::new();
    s.set_active(on);
    s.set_valign(gtk::Align::Center);
    s.connect_active_notify(move |s| f(s.is_active()));
    row.append(&s);
    b.append(&row);
}

/// Première lettre en majuscule (« mercredi 18 novembre » → « Mercredi 18 novembre »).
pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Minuterie liée à un widget : s'arrête quand il est détruit.
pub fn every(widget: &impl IsA<gtk::Widget>, secs: u32, f: impl Fn() + 'static) {
    let w = widget.as_ref().downgrade();
    glib::timeout_add_seconds_local(secs, move || {
        match w.upgrade() {
            Some(_) => {
                f();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }
    });
}

/// Ouvre un fichier ou un dossier avec l'application par défaut.
pub fn open_path(path: &std::path::Path) {
    let uri = gtk::gio::File::for_path(path).uri();
    let _ = gtk::gio::AppInfo::launch_default_for_uri(&uri, gtk::gio::AppLaunchContext::NONE);
}

/// Taille selon `s` / `m` / `l`.
pub fn by_size<T: Copy>(w: &DesktopWidget, s: T, m: T, l: T) -> T {
    match w.size() {
        "s" => s,
        "l" => l,
        _ => m,
    }
}

pub const CSS: &str = r#"
.desk-title { font-weight: 700; font-size: 1.05em; }
.desk-dim { opacity: 0.7; }
.desk-set-title { font-weight: 700; font-size: 0.85em; opacity: 0.75; margin-top: 6px; }
.desk-choice button { padding: 4px 10px; }
.desk-choice button:checked { background-color: @accent_bg_color; color: @accent_fg_color; }
"#;

/// Feuilles de style de tous les widgets.
pub fn css() -> String {
    [CSS, clock::CSS, calendar::CSS, countdown::CSS, weather::CSS, worldclock::CSS, media::CSS, photo::CSS, folders::CSS, note::CSS].concat()
}

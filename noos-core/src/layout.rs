//! Mises en page du bureau (mode édition) : instantané de la disposition (barres, coin des
//! notifications, widgets), historique annuler / rétablir, modèles prédéfinis, mises en page
//! enregistrées (`~/.config/noos/layouts/*.toml`) et aimantation des widgets.

use std::{fs, path::PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    config::{DesktopWidget, NoosConfig, TaskbarAlignment, new_widget_id},
    paths,
};

/// Disposition complète du bureau. Les modèles laissent `widgets` vide (`None`) : ils ne
/// touchent pas aux widgets posés par l'utilisateur.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    pub name: String,
    /// Bord de la barre des tâches : `bottom`, `top`, `left`, `right`.
    pub position: String,
    pub alignment: TaskbarAlignment,
    pub zone_left: Vec<String>,
    pub zone_center: Vec<String>,
    pub zone_right: Vec<String>,
    pub hidden: Vec<String>,
    pub show_search: bool,
    pub show_vitals: bool,
    pub show_power: bool,
    pub taskview_button: bool,
    /// Barre d'état (bord opposé).
    pub statusbar: bool,
    pub status_left: Vec<String>,
    pub status_center: Vec<String>,
    pub status_right: Vec<String>,
    /// Coin des notifications.
    pub notifications: String,
    #[serde(rename = "widget", skip_serializing_if = "Option::is_none")]
    pub widgets: Option<Vec<DesktopWidget>>,
}

impl Default for Layout {
    fn default() -> Self {
        Layout::capture(&NoosConfig::default(), false)
    }
}

impl Layout {
    /// Instantané de la configuration (avec ou sans les widgets).
    pub fn capture(cfg: &NoosConfig, with_widgets: bool) -> Self {
        let t = &cfg.taskbar;
        Self {
            name: String::new(),
            position: t.position.clone(),
            alignment: t.alignment,
            zone_left: t.zone_left.clone(),
            zone_center: t.zone_center.clone(),
            zone_right: t.zone_right.clone(),
            hidden: t.hidden.clone(),
            show_search: t.show_search,
            show_vitals: t.show_vitals,
            show_power: t.show_power,
            taskview_button: cfg.desktops.taskbar_button,
            statusbar: cfg.topbar.enabled,
            status_left: cfg.topbar.zone_left.clone(),
            status_center: cfg.topbar.zone_center.clone(),
            status_right: cfg.topbar.zone_right.clone(),
            notifications: cfg.notifications.position.clone(),
            widgets: with_widgets.then(|| cfg.desktop_widgets.clone()),
        }
    }

    /// Applique la disposition à la configuration (les widgets seulement s'il y en a).
    pub fn apply(&self, cfg: &mut NoosConfig) {
        let t = &mut cfg.taskbar;
        t.position = match self.position.as_str() {
            "top" | "left" | "right" => self.position.clone(),
            _ => "bottom".into(),
        };
        t.alignment = self.alignment;
        t.zone_left = self.zone_left.clone();
        t.zone_center = self.zone_center.clone();
        t.zone_right = self.zone_right.clone();
        t.hidden = self.hidden.clone();
        t.show_search = self.show_search;
        t.show_vitals = self.show_vitals;
        t.show_power = self.show_power;
        cfg.desktops.taskbar_button = self.taskview_button;
        cfg.topbar.enabled = self.statusbar;
        cfg.topbar.zone_left = self.status_left.clone();
        cfg.topbar.zone_center = self.status_center.clone();
        cfg.topbar.zone_right = self.status_right.clone();
        if matches!(self.notifications.as_str(), "top-left" | "top-right" | "bottom-left" | "bottom-right") {
            cfg.notifications.position = self.notifications.clone();
        }
        if let Some(widgets) = &self.widgets {
            cfg.desktop_widgets = widgets.clone();
            for w in &mut cfg.desktop_widgets {
                if w.id.is_empty() {
                    w.id = new_widget_id();
                }
            }
        }
    }

    /// Même disposition des barres (sans comparer le nom ni les widgets) : modèle actif.
    pub fn same_bars(&self, other: &Layout) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.name.clear();
        b.name.clear();
        a.widgets = None;
        b.widgets = None;
        // Les zones ne comptent que par les éléments affichés.
        let (ca, cb) = (a.config(), b.config());
        a.position == b.position && a.alignment == b.alignment && ca.bar_zones() == cb.bar_zones() && a.notifications == b.notifications
    }

    fn config(&self) -> NoosConfig {
        let mut c = NoosConfig::default();
        self.apply(&mut c);
        c
    }

    /// Éléments masqués d'après la liste `hidden` (modèles).
    fn sync_shown(mut self) -> Self {
        let off = |id: &str| self.hidden.iter().any(|h| h == id);
        self.show_search = !off("search");
        self.show_vitals = !off("vitals");
        self.show_power = !off("power");
        self.taskview_button = !off("taskview");
        self
    }

    pub fn to_toml(&self) -> anyhow::Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        let l: Self = toml::from_str(text).context("fichier de mise en page illisible")?;
        Ok(l)
    }
}

// ───────────────────────────── Annuler / rétablir ─────────────────────────────

/// Pile d'instantanés : chaque action du mode édition enregistre l'état d'avant.
#[derive(Default)]
pub struct History {
    undo: Vec<(String, Layout)>,
    redo: Vec<(String, Layout)>,
    /// Dernière action enregistrée et son heure : la même action répétée aussitôt (flèches
    /// du clavier) ne fait qu'une étape d'annulation.
    last: Option<(String, std::time::Instant)>,
}

impl History {
    pub const MAX: usize = 50;
    /// Délai sous lequel la même action répétée est fusionnée.
    pub const MERGE: std::time::Duration = std::time::Duration::from_millis(1200);

    /// Action `label` faite, `before` = état d'avant.
    pub fn push(&mut self, label: &str, before: Layout) {
        if self.undo.last().is_some_and(|(_, l)| *l == before) && self.redo.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let repeat = self.last.as_ref().is_some_and(|(l, t)| l == label && now.duration_since(*t) < Self::MERGE);
        self.last = Some((label.to_string(), now));
        if repeat && self.redo.is_empty() && self.undo.last().is_some_and(|(l, _)| l == label) {
            return;
        }
        self.undo.push((label.to_string(), before));
        if self.undo.len() > Self::MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Annule la dernière action : état à appliquer et nom de l'action.
    pub fn undo(&mut self, current: Layout) -> Option<(String, Layout)> {
        self.last = None;
        let (label, before) = self.undo.pop()?;
        self.redo.push((label.clone(), current));
        Some((label, before))
    }

    pub fn redo(&mut self, current: Layout) -> Option<(String, Layout)> {
        self.last = None;
        let (label, after) = self.redo.pop()?;
        self.undo.push((label.clone(), current));
        Some((label, after))
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(l, _)| l.as_str())
    }
}

// ───────────────────────────── Modèles ─────────────────────────────

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub layout: Layout,
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Dispositions toutes faites (barres et notifications ; les widgets ne bougent pas).
pub fn presets() -> Vec<Preset> {
    let base = Layout::default();
    let mk = |id, name, description, f: &dyn Fn(&mut Layout)| {
        let mut l = base.clone();
        f(&mut l);
        l.name = String::from(name);
        Preset { id, name, description, layout: l.sync_shown() }
    };
    vec![
        mk("windows", "Windows 11", "Barre des tâches centrée en bas, notifications en bas à droite", &|_| {}),
        mk("classic", "Classique", "Démarrer et applications à gauche, comme Windows 10", &|l| {
            l.alignment = TaskbarAlignment::Left;
            l.zone_left = ids(&["start", "search", "taskview", "tasks"]);
            l.zone_center = vec![];
            l.zone_right = ids(&["vitals", "tray", "status", "power", "clock", "notifications", "showdesktop"]);
        }),
        mk("statusbar", "Barre d'état", "Barre d'état en haut, applications seules en bas (façon macOS)", &|l| {
            l.statusbar = true;
            l.status_left = ids(&["start", "desktops"]);
            l.status_center = vec![];
            l.status_right = ids(&["tray", "status", "power", "clock", "notifications"]);
            l.zone_left = vec![];
            l.zone_center = ids(&["tasks", "taskview"]);
            l.zone_right = vec![];
            l.hidden = ids(&["search", "vitals", "showdesktop"]);
            l.notifications = "top-right".into();
        }),
        mk("gnome", "GNOME", "Horloge au centre de la barre d'état, bureaux à gauche", &|l| {
            l.statusbar = true;
            l.status_left = ids(&["taskview", "desktops"]);
            l.status_center = ids(&["clock"]);
            l.status_right = ids(&["tray", "status", "power", "notifications"]);
            l.zone_left = vec![];
            l.zone_center = ids(&["start", "search", "tasks"]);
            l.zone_right = ids(&["showdesktop"]);
            l.hidden = ids(&["vitals"]);
            l.notifications = "top-right".into();
        }),
        mk("vertical", "Verticale", "Barre des tâches à gauche, barre d'état en haut (écrans larges)", &|l| {
            l.position = "left".into();
            l.statusbar = true;
            l.status_left = ids(&["desktops"]);
            l.status_center = ids(&["clock"]);
            l.status_right = ids(&["vitals", "tray", "status", "power", "notifications"]);
            l.zone_left = ids(&["start", "search", "taskview"]);
            l.zone_center = ids(&["tasks"]);
            l.zone_right = ids(&["showdesktop"]);
            l.hidden = vec![];
            l.notifications = "top-right".into();
        }),
        mk("minimal", "Épurée", "L'essentiel : Démarrer, applications, horloge", &|l| {
            l.zone_left = vec![];
            l.zone_center = ids(&["start", "tasks"]);
            l.zone_right = ids(&["tray", "status", "power", "clock"]);
            l.hidden = ids(&["vitals", "search", "taskview", "notifications", "showdesktop"]);
        }),
    ]
}

// ───────────────────────────── Mises en page enregistrées ─────────────────────────────

pub fn layouts_dir() -> PathBuf {
    paths::config_dir().join("layouts")
}

/// Nom de fichier tiré du nom : minuscules, sans accents ni espaces.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "mise-en-page".into() } else { s }
}

/// Enregistre une mise en page nommée (remplace celle du même nom).
pub fn save_named(layout: &Layout) -> anyhow::Result<PathBuf> {
    let dir = layouts_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.toml", slug(&layout.name)));
    fs::write(&path, layout.to_toml()?)?;
    Ok(path)
}

/// Mises en page enregistrées, par nom.
pub fn list_saved() -> Vec<(PathBuf, Layout)> {
    let mut v: Vec<(PathBuf, Layout)> = fs::read_dir(layouts_dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| {
            let mut l = Layout::from_toml(&fs::read_to_string(&p).ok()?).ok()?;
            if l.name.is_empty() {
                l.name = p.file_stem()?.to_string_lossy().to_string();
            }
            Some((p, l))
        })
        .collect();
    v.sort_by_key(|(_, l)| l.name.to_lowercase());
    v
}

// ───────────────────────────── Aimantation ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    fn overlaps(&self, o: &Rect, gap: f64) -> bool {
        self.x < o.x + o.w + gap && o.x < self.x + self.w + gap && self.y < o.y + o.h + gap && o.y < self.y + self.h + gap
    }
}

/// Repère d'alignement à dessiner : ligne verticale (`x = pos`) ou horizontale (`y = pos`),
/// de `from` à `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub pos: f64,
    pub from: f64,
    pub to: f64,
}

/// Paramètres de l'aimantation : écran, zone utile (sans les barres), pas de la grille.
#[derive(Debug, Clone, Copy)]
pub struct SnapArea {
    pub width: f64,
    pub height: f64,
    /// Zone utile (sans les barres) : rectangle où les marges s'appliquent.
    pub work: Rect,
    pub margin: f64,
    pub gap: f64,
    pub grid: f64,
    pub threshold: f64,
}

impl SnapArea {
    pub fn new(width: f64, height: f64, work: Rect) -> Self {
        Self { width, height, work, margin: 24.0, gap: 16.0, grid: 16.0, threshold: 10.0 }
    }
}

/// Position aimantée d'un widget de taille `m.w × m.h` lâché en `(m.x, m.y)` : alignements
/// avec les bords et le centre de la zone utile et avec les autres widgets (bords, centres,
/// côte à côte), sinon la grille. Renvoie la position et les repères à afficher.
pub fn snap(m: Rect, others: &[Rect], area: &SnapArea) -> (f64, f64, Vec<Guide>) {
    let wk = area.work;
    // Candidats sur un axe : (position du widget, position de la ligne, étendue de la ligne).
    let axis = |vertical: bool| -> Option<(f64, Guide)> {
        let (pos, size) = if vertical { (m.x, m.w) } else { (m.y, m.h) };
        let (w0, wlen) = if vertical { (wk.x, wk.w) } else { (wk.y, wk.h) };
        let (full0, full1) = if vertical { (0.0, area.height) } else { (0.0, area.width) };
        let mut cands: Vec<(f64, f64, f64, f64)> = vec![
            (w0 + area.margin, w0 + area.margin, full0, full1),
            (w0 + wlen - area.margin - size, w0 + wlen - area.margin, full0, full1),
            (w0 + (wlen - size) / 2.0, w0 + wlen / 2.0, full0, full1),
        ];
        for o in others {
            let (op, os, oc0, oc1) = if vertical { (o.x, o.w, o.y, o.y + o.h) } else { (o.y, o.h, o.x, o.x + o.w) };
            let (mc0, mc1) = if vertical { (m.y, m.y + m.h) } else { (m.x, m.x + m.w) };
            let (a, b) = (oc0.min(mc0), oc1.max(mc1));
            cands.extend([
                (op, op, a, b),                                // bords de début alignés
                (op + os - size, op + os, a, b),               // bords de fin alignés
                (op + (os - size) / 2.0, op + os / 2.0, a, b), // centres alignés
                (op + os + area.gap, op + os + area.gap, a, b), // juste après, avec un écart
                (op - size - area.gap, op - area.gap, a, b),   // juste avant
            ]);
        }
        cands
            .into_iter()
            .map(|(p, line, from, to)| ((p - pos).abs(), p, Guide { vertical, pos: line, from, to }))
            .filter(|(d, _, _)| *d <= area.threshold)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p, g)| (p, g))
    };
    let mut guides = vec![];
    let grid = |v: f64| if area.grid > 1.0 { (v / area.grid).round() * area.grid } else { v };
    let x = match axis(true) {
        Some((p, g)) => {
            guides.push(g);
            p
        }
        None => grid(m.x),
    };
    let y = match axis(false) {
        Some((p, g)) => {
            guides.push(g);
            p
        }
        None => grid(m.y),
    };
    let x = x.clamp(0.0, (area.width - m.w).max(0.0));
    let y = y.clamp(0.0, (area.height - m.h).max(0.0));
    (x, y, guides)
}

/// Premier emplacement libre pour un widget de taille `(w, h)` : en colonnes depuis le coin
/// supérieur gauche de la zone utile, comme les icônes d'un bureau. S'il n'y en a pas, la
/// position qui recouvre le moins les autres.
pub fn free_spot(size: (f64, f64), others: &[Rect], area: &SnapArea) -> (f64, f64) {
    let wk = area.work;
    let step = area.grid.max(8.0) * 2.0;
    let (w, h) = size;
    let mut best: Option<(f64, f64, f64)> = None;
    let mut x = wk.x + area.margin;
    while x + w <= wk.x + wk.w - area.margin + 0.5 {
        let mut y = wk.y + area.margin;
        while y + h <= wk.y + wk.h - area.margin + 0.5 {
            let r = Rect::new(x, y, w, h);
            if !others.iter().any(|o| r.overlaps(o, area.gap - 1.0)) {
                return (x, y);
            }
            let cover: f64 = others.iter().map(|o| r.intersection(o)).sum();
            if best.is_none_or(|(_, _, c)| cover < c) {
                best = Some((x, y, cover));
            }
            y += step;
        }
        x += step;
    }
    best.map(|(x, y, _)| (x, y)).unwrap_or((wk.x + area.margin, wk.y + area.margin))
}

impl Rect {
    /// Surface commune.
    pub fn intersection(&self, o: &Rect) -> f64 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantane_aller_retour() {
        let mut cfg = NoosConfig::default();
        cfg.taskbar.position = "left".into();
        cfg.desktop_widgets.push(DesktopWidget::new("clock", 0.1, 0.2));
        let l = Layout::capture(&cfg, true);
        let mut other = NoosConfig::default();
        l.apply(&mut other);
        assert_eq!(other.taskbar.position, "left");
        assert_eq!(other.desktop_widgets, cfg.desktop_widgets);
        // Fichier TOML : relu à l'identique.
        let back = Layout::from_toml(&l.to_toml().unwrap()).unwrap();
        assert_eq!(back, l);
        // Sans widgets : ceux de la configuration restent.
        let mut l2 = Layout::capture(&NoosConfig::default(), false);
        l2.position = "top".into();
        l2.apply(&mut other);
        assert_eq!(other.desktop_widgets.len(), 1);
        assert_eq!(other.taskbar.position, "top");
    }

    #[test]
    fn annuler_retablir() {
        let mut h = History::default();
        let a = Layout::default();
        let mut b = a.clone();
        b.position = "top".into();
        let mut c = b.clone();
        c.statusbar = true;
        h.push("Bord", a.clone());
        h.push("Barre d'état", b.clone());
        // La même action répétée aussitôt (flèches) : une seule étape.
        h.push("Barre d'état", c.clone());
        assert_eq!(h.undo_label(), Some("Barre d'état"));
        assert_eq!(h.undo_label(), Some("Barre d'état"));
        let (l, s) = h.undo(c.clone()).unwrap();
        assert_eq!((l.as_str(), &s), ("Barre d'état", &b));
        let (_, s) = h.undo(b.clone()).unwrap();
        assert_eq!(s, a);
        assert!(h.undo(a.clone()).is_none());
        let (_, s) = h.redo(a.clone()).unwrap();
        assert_eq!(s, b);
        // Nouvelle action : on ne peut plus rétablir.
        h.push("Autre", b.clone());
        assert!(h.redo(b).is_none());
    }

    #[test]
    fn modeles_valides() {
        let p = presets();
        assert!(p.len() >= 6);
        for preset in &p {
            let mut cfg = NoosConfig::default();
            preset.layout.apply(&mut cfg);
            let z = cfg.bar_zones();
            // Chaque élément affiché une seule fois ; Démarrer et les applications toujours là.
            let all: Vec<&String> = z.iter().flatten().collect();
            for id in ["start", "tasks"] {
                assert_eq!(all.iter().filter(|x| **x == id).count(), 1, "{} : {id}", preset.id);
            }
            assert!(preset.layout.same_bars(&Layout::capture(&cfg, false)), "{}", preset.id);
        }
        // Le premier modèle correspond à la configuration par défaut.
        assert!(p[0].layout.same_bars(&Layout::default()));
        assert!(!p[1].layout.same_bars(&Layout::default()));
    }

    #[test]
    fn noms_de_fichier() {
        assert_eq!(slug("Mon bureau à moi !"), "mon-bureau-a-moi");
        assert_eq!(slug("  "), "mise-en-page");
        assert_eq!(slug("Été 2026"), "ete-2026");
    }

    #[test]
    fn aimantation() {
        let area = SnapArea::new(1920.0, 1080.0, Rect::new(0.0, 0.0, 1920.0, 1032.0));
        // Près de la marge gauche : collé à 24 px, repère vertical.
        let (x, y, g) = snap(Rect::new(30.0, 101.0, 200.0, 100.0), &[], &area);
        assert_eq!(x, 24.0);
        assert_eq!(y, 96.0); // grille de 16 px
        assert_eq!(g.len(), 1);
        assert!(g[0].vertical && g[0].pos == 24.0);
        // Aligné sur le bord gauche d'un autre widget.
        let other = Rect::new(400.0, 500.0, 300.0, 200.0);
        let (x, _, g) = snap(Rect::new(405.0, 200.0, 100.0, 100.0), &[other], &area);
        assert_eq!(x, 400.0);
        assert!(g.iter().any(|g| g.vertical && g.pos == 400.0 && g.from == 200.0 && g.to == 700.0));
        // Juste sous l'autre, avec l'écart de 16 px.
        let (_, y, _) = snap(Rect::new(800.0, 712.0, 100.0, 100.0), &[other], &area);
        assert_eq!(y, 716.0);
        // Centré sur l'écran.
        let (x, _, _) = snap(Rect::new(855.0, 300.0, 200.0, 100.0), &[], &area);
        assert_eq!(x, 860.0);
        // Jamais hors de l'écran.
        let (x, y, _) = snap(Rect::new(1900.0, 1070.0, 200.0, 100.0), &[], &area);
        assert_eq!((x, y), (1720.0, 980.0));
    }

    #[test]
    fn emplacement_libre() {
        let area = SnapArea::new(1920.0, 1080.0, Rect::new(0.0, 0.0, 1920.0, 1032.0));
        assert_eq!(free_spot((200.0, 100.0), &[], &area), (24.0, 24.0));
        let taken = [Rect::new(24.0, 24.0, 300.0, 300.0)];
        let (x, y) = free_spot((200.0, 100.0), &taken, &area);
        assert!(!Rect::new(x, y, 200.0, 100.0).overlaps(&taken[0], 0.0));
        assert_eq!(x, 24.0);
        assert!(y >= 324.0);
        // Bureau plein : la position la moins recouverte, pas le coin.
        let full: Vec<Rect> = (0..6).flat_map(|i| (0..4).map(move |j| Rect::new(i as f64 * 320.0, j as f64 * 258.0, 300.0, 240.0))).filter(|r| !(r.x == 1600.0 && r.y == 774.0)).collect();
        let (x, y) = free_spot((200.0, 200.0), &full, &area);
        let r = Rect::new(x, y, 200.0, 200.0);
        let cover: f64 = full.iter().map(|o| r.intersection(o)).sum();
        assert!(cover < 200.0 * 200.0 * 0.5, "{x},{y} recouvre {cover}");
        assert!((x, y) != (24.0, 24.0));
    }
}

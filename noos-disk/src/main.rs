//! « Disques » — gestion des disques de NOOS (GTK4 + libAdapta, udisks2).
//!
//! Disques internes, clés USB, cartes mémoire et images disque : carte des partitions,
//! formatage (exFAT, NTFS, FAT32, ext4, Btrfs, XFS, F2FS), création / suppression de
//! partitions, initialisation (GPT / MBR), renommage, vérification et réparation, volumes
//! chiffrés (LUKS), santé SMART (ATA / NVMe) et autotest, test de vitesse, création et
//! restauration d'images disque, éjection en toute sécurité.
//!
//! `noos-disk [/dev/sdX | /dev/sdX1]` ouvre un disque (ou un volume) ; `--format /dev/sdX1`
//! ouvre directement le formatage (fenêtre « Lecture automatique » du shell).
//! Captures : `NOOS_DISK_DIALOG=format|init|bench|smart|rename`.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

mod selftest;

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_disk::{
    ops, rawio,
    udisks::{self, Drive, Model},
};

const APP_ID: &str = "org.noos.Disks";

const CSS: &str = r#"
.disk-hero { padding: 18px 20px; border-radius: 14px; background-color: alpha(@window_fg_color, 0.04); }
.disk-hero-icon { min-width: 72px; min-height: 72px; border-radius: 18px; color: @accent_color;
  background: linear-gradient(135deg, alpha(@accent_bg_color, 0.28), alpha(@accent_bg_color, 0.08)); }
.chip { border-radius: 99px; padding: 2px 10px; font-size: 0.82em; font-weight: 700; background-color: alpha(@window_fg_color, 0.08); }
.chip.good { background-color: alpha(@success_color, 0.18); color: @success_color; }
.chip.warn { background-color: alpha(@warning_color, 0.2); color: @warning_color; }
.chip.bad { background-color: alpha(@error_color, 0.2); color: @error_color; }
.chip.system { background-color: alpha(@accent_bg_color, 0.22); color: @accent_color; }
.disk-map { border-radius: 12px; }
.vol-dot { min-width: 12px; min-height: 12px; border-radius: 4px; }
.fs-card { padding: 10px 12px; border-radius: 10px; }
.bench-big { font-size: 2.2em; font-weight: 800; font-feature-settings: "tnum"; }
.bench-unit { font-size: 0.9em; opacity: 0.7; }
.drive-row-sub { font-size: 0.82em; opacity: 0.65; }
.actions-flow button { border-radius: 10px; padding: 8px 14px; }
"#;

/// Couleur d'un type de système de fichiers (carte des partitions).
fn fs_color(t: &str) -> (f64, f64, f64) {
    match t {
        "ntfs" => (0.29, 0.56, 0.96),
        "exfat" => (0.18, 0.76, 0.80),
        "vfat" => (0.98, 0.69, 0.25),
        "ext4" | "ext3" | "ext2" => (0.36, 0.78, 0.45),
        "btrfs" => (0.66, 0.47, 0.98),
        "xfs" => (0.95, 0.45, 0.60),
        "f2fs" => (0.55, 0.85, 0.25),
        "crypto_LUKS" => (0.85, 0.35, 0.35),
        "swap" => (0.55, 0.55, 0.62),
        "iso9660" | "udf" => (0.90, 0.55, 0.20),
        "" => (0.45, 0.45, 0.50),
        _ => (0.60, 0.60, 0.70),
    }
}

/// Élément de la carte : un volume (bloc) ou un espace libre.
#[derive(Clone, Debug, PartialEq)]
enum Sel {
    Block(String),
    Free { table: String, offset: u64, size: u64 },
}

struct App {
    win: adap::ApplicationWindow,
    conn: gio::DBusConnection,
    model: RefCell<Model>,
    drive: RefCell<Option<String>>,
    sel: RefCell<Option<Sel>>,
    sidebar: gtk::ListBox,
    content: gtk::Box,
    title: adap::WindowTitle,
    toasts: adap::ToastOverlay,
    banner: adap::Banner,
    syncing: Cell<bool>,
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

fn chip(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("chip");
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

fn group(title: &str) -> adap::PreferencesGroup {
    let g = adap::PreferencesGroup::new();
    if !title.is_empty() {
        g.set_title(title);
    }
    g
}

fn info_row(title: &str, value: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_title(title);
    r.set_use_markup(false);
    let v = gtk::Label::new(Some(value));
    v.add_css_class("dim-label");
    v.set_selectable(true);
    v.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    v.set_max_width_chars(48);
    r.add_suffix(&v);
    r
}

/// Segments de la carte des partitions (début, taille, sélection) — trous de plus de 4 Mio
/// affichés comme « espace libre ».
fn segments(m: &Model, drive: &Drive) -> Vec<(u64, u64, Sel)> {
    let Some(whole) = m.whole(&drive.path) else { return vec![] };
    let size = whole.size.max(drive.size);
    if whole.table.is_none() {
        // Pas de table : tout le disque est un volume (ou vide).
        return vec![(0, size, Sel::Block(whole.path.clone()))];
    }
    let mut out = vec![];
    let mut pos = 0u64;
    const MIB: u64 = 1 << 20;
    for p in m.partitions(&drive.path) {
        let part = p.partition.as_ref().unwrap();
        if part.container {
            continue; // partition étendue : ses partitions logiques sont listées
        }
        if part.offset > pos + 4 * MIB {
            out.push((pos, part.offset - pos, Sel::Free { table: whole.path.clone(), offset: pos.max(MIB), size: part.offset - pos.max(MIB) }));
        }
        out.push((part.offset, part.size, Sel::Block(p.path.clone())));
        pos = pos.max(part.offset + part.size);
    }
    if size > pos + 4 * MIB {
        // GPT garde 33 secteurs à la fin : on laisse 1 Mio.
        out.push((pos, size - pos, Sel::Free { table: whole.path.clone(), offset: pos.max(MIB), size: size - pos.max(MIB) - MIB }));
    }
    out
}

impl App {
    fn toast(&self, t: &str) {
        let toast = adap::Toast::new(t);
        toast.set_timeout(4);
        self.toasts.add_toast(toast);
    }

    fn error(&self, title: &str, body: &str) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_response("ok", "OK");
        d.present(Some(&self.win));
    }

    /// Termine une opération : message d'erreur, ou toast, puis relecture.
    fn finish<T>(self: &Rc<Self>, what: &str, ok: &str) -> impl FnOnce(Result<T, String>) + 'static {
        let (a, what, ok) = (self.clone(), what.to_string(), ok.to_string());
        move |r| {
            match r {
                Ok(_) => {
                    if !ok.is_empty() {
                        a.toast(&ok)
                    }
                }
                Err(e) => a.error(&what, &e),
            }
            a.reload();
        }
    }

    fn reload(self: &Rc<Self>) {
        match udisks::load(&self.conn) {
            Ok(m) => *self.model.borrow_mut() = m,
            Err(e) => self.toast(&format!("udisks : {e}")),
        }
        {
            let m = self.model.borrow();
            let cur = self.drive.borrow().clone();
            if cur.as_ref().is_none_or(|d| m.drive(d).is_none()) {
                *self.drive.borrow_mut() = m.drives.first().map(|d| d.path.clone());
                *self.sel.borrow_mut() = None;
            }
        }
        self.fill_sidebar();
        self.fill_content();
        self.update_jobs();
    }

    fn update_jobs(self: &Rc<Self>) {
        let m = self.model.borrow();
        if let Some(j) = m.jobs.first() {
            let pct = j.progress.map(|p| format!(" — {:.0} %", p * 100.0)).unwrap_or_default();
            self.banner.set_title(&format!("{} en cours{pct}…", j.label()));
            self.banner.set_revealed(true);
            // La progression des tâches n'émet pas toujours de signal : relecture régulière.
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                if let Some(a) = w.upgrade() {
                    a.reload();
                }
            });
        } else {
            self.banner.set_revealed(false);
        }
    }

    // ───────────────────────────── Barre latérale ─────────────────────────────

    fn fill_sidebar(self: &Rc<Self>) {
        self.syncing.set(true);
        while let Some(r) = self.sidebar.first_child() {
            self.sidebar.remove(&r);
        }
        let m = self.model.borrow();
        let cur = self.drive.borrow().clone();
        for d in &m.drives {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            h.set_margin_top(6);
            h.set_margin_bottom(6);
            let icon = gtk::Image::from_icon_name(d.icon());
            icon.set_pixel_size(24);
            h.append(&icon);
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            v.set_hexpand(true);
            let n = label(&d.name(), &[]);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&n);
            let sub = format!("{} · {}{}", d.kind(), noos_disk::human(d.size), if m.is_system(&d.path) { " · système" } else { "" });
            v.append(&label(&sub, &["drive-row-sub"]));
            h.append(&v);
            if d.health.as_ref().is_some_and(|h| h.failing) {
                h.append(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            }
            if m.drives.iter().any(|x| x.path == d.path) && m.jobs.iter().any(|j| j.objects.iter().any(|o| m.block(o).is_some_and(|b| b.drive == d.path))) {
                let sp = gtk::Spinner::new();
                sp.start();
                h.append(&sp);
            }
            row.set_child(Some(&h));
            unsafe { row.set_data("drive", d.path.clone()) };
            let section = if d.connection == "loop" {
                "Images disque"
            } else if d.is_external() {
                "Périphériques externes"
            } else {
                "Disques internes"
            };
            unsafe { row.set_data("section", section) };
            self.sidebar.append(&row);
            if cur.as_deref() == Some(&d.path) {
                self.sidebar.select_row(Some(&row));
            }
        }
        self.syncing.set(false);
    }

    // ───────────────────────────── Contenu ─────────────────────────────

    fn fill_content(self: &Rc<Self>) {
        while let Some(c) = self.content.first_child() {
            self.content.remove(&c);
        }
        let m = self.model.borrow().clone();
        let Some(path) = self.drive.borrow().clone() else {
            let s = adap::StatusPage::new();
            s.set_icon_name(Some("drive-harddisk-symbolic"));
            s.set_title("Aucun disque");
            self.content.append(&s);
            return;
        };
        let Some(d) = m.drive(&path).cloned() else { return };
        self.title.set_title(&d.name());
        self.title.set_subtitle(&format!("{} · {}", d.kind(), noos_disk::human(d.size)));
        let system = m.is_system(&d.path);
        self.content.append(&self.hero(&m, &d, system));

        let segs = segments(&m, &d);
        if self.sel.borrow().as_ref().is_none_or(|s| !segs.iter().any(|x| &x.2 == s)) {
            // Sélection par défaut : le premier volume monté, sinon le premier élément.
            // Sélection par défaut : le plus grand volume monté, sinon le plus grand élément.
            let mounted = |s: &&(u64, u64, Sel)| matches!(&s.2, Sel::Block(b) if m.block(b).is_some_and(|b| m.content(b).mounted_at().is_some()));
            let pick = segs.iter().filter(mounted).max_by_key(|s| s.1).or_else(|| segs.iter().max_by_key(|s| s.1)).map(|s| s.2.clone());
            *self.sel.borrow_mut() = pick;
        }
        if segs.is_empty() {
            let s = adap::StatusPage::new();
            s.set_icon_name(Some("media-removable-symbolic"));
            s.set_title("Pas de support");
            s.set_description(Some("Insérez un disque ou une carte."));
            self.content.append(&s);
            return;
        }
        let g = group("Volumes");
        let whole = m.whole(&d.path);
        g.set_description(Some(&match whole.and_then(|w| w.table.clone()) {
            Some(t) => format!("Table de partitions {}", if t == "gpt" { "GPT (moderne)" } else { "MBR (ancienne)" }),
            None => "Sans table de partitions".into(),
        }));
        self.content.append(&self.map(&m, &segs, &d));
        for (_, size, s) in &segs {
            g.add(&self.volume_row(&m, s, *size));
        }
        self.content.append(&g);
        if let Some(s) = self.sel.borrow().clone() {
            self.content.append(&self.details(&m, &d, &s, system));
        }
    }

    fn hero(self: &Rc<Self>, m: &Model, d: &Drive, system: bool) -> gtk::Box {
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        hero.add_css_class("disk-hero");
        let icon = gtk::Image::from_icon_name(d.icon());
        icon.set_pixel_size(40);
        icon.add_css_class("disk-hero-icon");
        icon.set_valign(gtk::Align::Center);
        hero.append(&icon);
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        v.set_hexpand(true);
        v.set_valign(gtk::Align::Center);
        let t = label(&d.name(), &["title-2"]);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        v.append(&t);
        let bus = match d.connection.as_str() {
            "usb" => "USB",
            "sdio" => "lecteur de cartes",
            "loop" => "fichier image",
            _ if d.nvme => "PCIe NVMe",
            _ => "SATA",
        };
        let dev = m.whole(&d.path).map(|w| w.device.clone()).unwrap_or_default();
        v.append(&label(&format!("{} · {} · {} · {dev}", d.kind(), noos_disk::human(d.size), bus), &["dim-label"]));
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if system {
            chips.append(&chip("Disque du système", "system"));
        }
        match &d.health {
            Some(h) if h.failing => {
                chips.append(&chip("Santé : défaillant, sauvegardez vos données !", "bad"));
            }
            Some(h) => {
                let warn = h.attributes_failing > 0 || h.bad_sectors.unwrap_or(0) > 0 || h.percent_used.unwrap_or(0) >= 90;
                let mut txt = if warn { "Santé : à surveiller".to_string() } else { "Santé : bonne".to_string() };
                if let Some(t) = h.temperature_c {
                    txt.push_str(&format!(" · {t:.0} °C"));
                }
                chips.append(&chip(&txt, if warn { "warn" } else { "good" }));
            }
            None => {}
        }
        if !d.serial.is_empty() && d.connection != "loop" {
            let s = chip(&format!("N° {}", if d.serial.len() > 20 { &d.serial[..20] } else { &d.serial }), "");
            s.set_tooltip_text(Some(&d.serial));
            chips.append(&s);
        }
        v.append(&chips);
        hero.append(&v);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_valign(gtk::Align::Center);
        if (d.is_external() || d.connection == "loop") && !system {
            let ej = gtk::Button::with_label(if d.connection == "loop" { "Détacher" } else { "Éjecter" });
            ej.add_css_class("pill");
            ej.add_css_class("suggested-action");
            let (a, path) = (self.clone(), d.path.clone());
            ej.connect_clicked(move |_| a.safely_remove(&path));
            actions.append(&ej);
        }
        let menu = gio::Menu::new();
        let tools = gio::Menu::new();
        tools.append(Some("Test de vitesse…"), Some("disk.bench"));
        if d.health.is_some() {
            tools.append(Some("Santé du disque (SMART)…"), Some("disk.smart"));
        }
        menu.append_section(None, &tools);
        let images = gio::Menu::new();
        images.append(Some("Créer une image du disque…"), Some("disk.backup"));
        if !system {
            images.append(Some("Restaurer une image sur ce disque…"), Some("disk.restore"));
        }
        menu.append_section(None, &images);
        if !system {
            let danger = gio::Menu::new();
            danger.append(Some("Initialiser le disque…"), Some("disk.init"));
            menu.append_section(None, &danger);
        }
        let mb = gtk::MenuButton::new();
        mb.set_icon_name("view-more-symbolic");
        mb.add_css_class("circular");
        mb.set_menu_model(Some(&menu));
        mb.set_tooltip_text(Some("Outils du disque"));
        actions.append(&mb);
        hero.append(&actions);
        hero
    }

    /// Carte des partitions : barre proportionnelle (les petites partitions gardent une
    /// largeur lisible), remplissage = espace occupé ; un clic sélectionne.
    fn map(self: &Rc<Self>, m: &Model, segs: &[(u64, u64, Sel)], d: &Drive) -> gtk::DrawingArea {
        let area = gtk::DrawingArea::new();
        area.set_content_height(68);
        area.add_css_class("disk-map");
        let total: u64 = segs.iter().map(|s| s.1).sum::<u64>().max(1);
        // Largeurs : proportionnelles, minimum 6 % par élément.
        let mut w: Vec<f64> = segs.iter().map(|s| (s.1 as f64 / total as f64).max(0.06)).collect();
        let sum: f64 = w.iter().sum();
        w.iter_mut().for_each(|x| *x /= sum);
        let info: Vec<(Sel, String, f64)> = segs
            .iter()
            .map(|(_, _, s)| match s {
                Sel::Block(p) => {
                    let b = m.block(p).map(|b| m.content(b));
                    let used = b.and_then(|b| b.mounted_at()).and_then(udisks::usage).map(|(u, t)| u as f64 / t.max(1) as f64).unwrap_or(-1.0);
                    (s.clone(), b.map(|b| b.id_type.clone()).unwrap_or_default(), used)
                }
                Sel::Free { .. } => (s.clone(), "free".into(), -1.0),
            })
            .collect();
        let sel = self.sel.borrow().clone();
        let widths = Rc::new(w);
        {
            let widths = widths.clone();
            let info = info.clone();
            area.set_draw_func(move |_, cr, ww, hh| {
                let (ww, hh) = (ww as f64, hh as f64);
                let gap = 4.0;
                let mut x = 0.0;
                for (k, frac) in widths.iter().enumerate() {
                    let segw = (ww - gap * (widths.len() as f64 - 1.0)) * frac;
                    let (s, kind, used) = &info[k];
                    rounded(cr, x, 0.0, segw, hh, 9.0);
                    if kind == "free" {
                        cr.set_source_rgba(0.5, 0.5, 0.55, 0.12);
                        let _ = cr.fill_preserve();
                        cr.set_source_rgba(0.6, 0.6, 0.65, 0.5);
                        cr.set_dash(&[5.0, 4.0], 0.0);
                        cr.set_line_width(1.5);
                        let _ = cr.stroke();
                        cr.set_dash(&[], 0.0);
                    } else {
                        let (r, g, b) = fs_color(kind);
                        cr.set_source_rgba(r, g, b, 0.35);
                        let _ = cr.fill();
                        if *used >= 0.0 {
                            cr.save().ok();
                            rounded(cr, x, 0.0, segw, hh, 9.0);
                            cr.clip();
                            cr.rectangle(x, 0.0, segw * used.clamp(0.0, 1.0), hh);
                            cr.set_source_rgba(r, g, b, 0.85);
                            let _ = cr.fill();
                            cr.restore().ok();
                        } else {
                            rounded(cr, x, hh - 6.0, segw, 6.0, 3.0);
                            cr.set_source_rgba(r, g, b, 0.9);
                            let _ = cr.fill();
                        }
                    }
                    if sel.as_ref() == Some(s) {
                        rounded(cr, x + 1.5, 1.5, segw - 3.0, hh - 3.0, 8.0);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
                        cr.set_line_width(3.0);
                        let _ = cr.stroke();
                    }
                    x += segw + gap;
                }
            });
        }
        let click = gtk::GestureClick::new();
        let (a, info2) = (self.clone(), info);
        let _ = d;
        click.connect_released(move |g, _, x, _| {
            let ww = g.widget().map(|w| w.width() as f64).unwrap_or(1.0);
            let gap = 4.0;
            let mut pos = 0.0;
            for (k, frac) in widths.iter().enumerate() {
                let segw = (ww - gap * (widths.len() as f64 - 1.0)) * frac;
                if x >= pos && x <= pos + segw + gap {
                    a.select(info2[k].0.clone());
                    return;
                }
                pos += segw + gap;
            }
        });
        area.add_controller(click);
        area
    }

    fn select(self: &Rc<Self>, s: Sel) {
        *self.sel.borrow_mut() = Some(s);
        self.fill_content();
    }

    fn volume_row(self: &Rc<Self>, m: &Model, s: &Sel, size: u64) -> adap::ActionRow {
        let r = adap::ActionRow::new();
        r.set_use_markup(false);
        r.set_activatable(true);
        let dot = gtk::DrawingArea::new();
        dot.set_content_width(14);
        dot.set_content_height(14);
        dot.set_valign(gtk::Align::Center);
        let color: Rc<Cell<Option<(f64, f64, f64)>>> = Rc::new(Cell::new(None));
        {
            let color = color.clone();
            dot.set_draw_func(move |_, cr, w, h| {
                rounded(cr, 1.0, 1.0, w as f64 - 2.0, h as f64 - 2.0, 4.0);
                match color.get() {
                    Some((r, g, b)) => {
                        cr.set_source_rgb(r, g, b);
                        let _ = cr.fill();
                    }
                    None => {
                        cr.set_source_rgba(0.6, 0.6, 0.65, 0.8);
                        cr.set_dash(&[2.0, 2.0], 0.0);
                        cr.set_line_width(1.5);
                        let _ = cr.stroke();
                    }
                }
            });
        }
        match s {
            Sel::Block(p) => {
                let b = m.block(p).cloned().unwrap_or_default();
                let c = m.content(&b).clone();
                color.set(Some(fs_color(&c.id_type)));
                r.set_title(&c.title());
                let mut sub = vec![noos_disk::fs_name(&c.id_type), noos_disk::human(size)];
                if b.encrypted.is_some() {
                    sub.push(if c.path != b.path { "chiffré, déverrouillé".into() } else { "chiffré, verrouillé".into() });
                }
                match c.mounted_at() {
                    Some(mp) => sub.push(format!("monté sur {mp}")),
                    None if c.filesystem.is_some() => sub.push("non monté".into()),
                    None => {}
                }
                r.set_subtitle(&sub.join(" · "));
                if let Some((u, t)) = c.mounted_at().and_then(udisks::usage) {
                    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
                    bar.set_value(u as f64 / t.max(1) as f64);
                    bar.set_size_request(120, -1);
                    bar.set_valign(gtk::Align::Center);
                    bar.add_offset_value("full-low", 0.75);
                    bar.add_offset_value("full-high", 0.9);
                    r.add_suffix(&bar);
                    r.add_suffix(&label(&format!("{} libres", noos_disk::human(t - u.min(t))), &["dim-label", "caption"]));
                }
                if let Some(j) = m.job_for(p).or_else(|| m.job_for(&c.path)) {
                    let sp = gtk::Spinner::new();
                    sp.start();
                    sp.set_tooltip_text(Some(&j.label()));
                    r.add_suffix(&sp);
                }
            }
            Sel::Free { size, .. } => {
                r.set_title("Espace libre");
                r.set_subtitle(&format!("{} non alloués · cliquez pour créer une partition", noos_disk::human(*size)));
            }
        }
        r.add_prefix(&dot);
        if self.sel.borrow().as_ref() == Some(s) {
            r.add_suffix(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        let (a, s) = (self.clone(), s.clone());
        r.connect_activated(move |_| a.select(s.clone()));
        r
    }

    fn details(self: &Rc<Self>, m: &Model, d: &Drive, s: &Sel, system: bool) -> gtk::Box {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.add_css_class("actions-flow");
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        flow.set_max_children_per_line(6);
        let action = |icon: &str, text: &str, class: &str, f: Box<dyn Fn()>| {
            let b = gtk::Button::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            h.append(&gtk::Image::from_icon_name(icon));
            h.append(&gtk::Label::new(Some(text)));
            b.set_child(Some(&h));
            if !class.is_empty() {
                b.add_css_class(class);
            }
            b.connect_clicked(move |_| f());
            b
        };
        match s {
            Sel::Free { table, offset, size } => {
                let g = group("Espace libre");
                g.add(&info_row("Taille", &noos_disk::human(*size)));
                g.add(&info_row("Position", &format!("à {} du début", noos_disk::human(*offset))));
                v.append(&g);
                let (a, t, o, sz) = (self.clone(), table.clone(), *offset, *size);
                flow.append(&action("list-add-symbolic", "Créer une partition…", "suggested-action", Box::new(move || a.format_dialog(FormatTarget::Free { table: t.clone(), offset: o, size: sz }))));
            }
            Sel::Block(p) => {
                let Some(b) = m.block(p).cloned() else { return v };
                let c = m.content(&b).clone();
                let g = group("Détails");
                g.add(&info_row("Appareil", &c.device));
                if !c.id_type.is_empty() {
                    let name = noos_disk::fs_name(&c.id_type);
                    let ver = if c.id_version.is_empty() || name.eq_ignore_ascii_case(&c.id_version) { String::new() } else { format!(" (version {})", c.id_version) };
                    g.add(&info_row("Système de fichiers", &format!("{}{ver}", noos_disk::fs_name(&c.id_type))));
                }
                g.add(&info_row("Taille", &format!("{} ({} octets)", noos_disk::human(b.size), group_digits(b.size))));
                if let Some((u, t)) = c.mounted_at().and_then(udisks::usage) {
                    let r = adap::ActionRow::new();
                    r.set_title("Espace utilisé");
                    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
                    bar.set_value(u as f64 / t.max(1) as f64);
                    bar.set_size_request(220, -1);
                    bar.set_valign(gtk::Align::Center);
                    r.add_suffix(&bar);
                    r.add_suffix(&label(&format!("{} sur {} ({:.0} %)", noos_disk::human(u), noos_disk::human(t), u as f64 * 100.0 / t.max(1) as f64), &["dim-label"]));
                    g.add(&r);
                }
                if let Some(mp) = c.mounted_at() {
                    let r = info_row("Monté sur", mp);
                    r.set_activatable(true);
                    r.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
                    let mp = mp.to_string();
                    r.connect_activated(move |_| open_folder(&mp));
                    g.add(&r);
                }
                if !c.id_uuid.is_empty() {
                    g.add(&info_row("UUID", &c.id_uuid));
                }
                if let Some(part) = &b.partition {
                    g.add(&info_row("Partition", &format!("n° {} · {}", part.number, partition_type_name(&part.type_))));
                }
                if b.read_only {
                    g.add(&info_row("Protection", "Lecture seule (verrou d'écriture ?)"));
                }
                v.append(&g);
                let critical_fs = c.filesystem.iter().flatten().any(|mp| mp == "/" || mp.starts_with("/boot") || mp == "/home");
                if c.filesystem.is_some() && !critical_fs && d.connection != "loop" {
                    v.append(&self.mount_group(&c, d));
                }

                let critical = c.filesystem.iter().flatten().any(|mp| mp == "/" || mp.starts_with("/boot") || mp == "/home");
                if let Some(mp) = c.mounted_at().map(String::from) {
                    flow.append(&action("folder-open-symbolic", "Ouvrir", "suggested-action", Box::new(move || open_folder(&mp))));
                    if !critical {
                        let (a, cp) = (self.clone(), c.path.clone());
                        flow.append(&action("media-eject-symbolic", "Démonter", "", Box::new(move || ops::unmount(&a.conn, &cp, a.finish("Démontage impossible", "Volume démonté")))));
                    }
                } else if c.filesystem.is_some() {
                    // Disque interne : monté « pour de bon » dans /mnt, droits réglés.
                    if !d.is_external() && d.connection != "loop" && c.fstab.is_none() {
                        let (a, c2) = (self.clone(), c.clone());
                        flow.append(&action("folder-new-symbolic", "Monter dans /mnt…", "suggested-action", Box::new(move || a.mnt_dialog(&c2))));
                    }
                    let (a, cp) = (self.clone(), c.path.clone());
                    let primary = if d.is_external() || c.fstab.is_some() || d.connection == "loop" { "suggested-action" } else { "" };
                    flow.append(&action(
                        "media-mount-symbolic",
                        if primary.is_empty() { "Monter (temporaire)" } else { "Monter" },
                        primary,
                        Box::new(move || {
                            let a2 = a.clone();
                            ops::mount(&a.conn, &cp, move |r| match r {
                                Ok(mp) => {
                                    a2.toast(&format!("Monté sur {mp}"));
                                    a2.reload();
                                }
                                Err(e) => a2.error("Montage impossible", &e),
                            })
                        }),
                    ));
                }
                if b.encrypted.is_some() {
                    if c.path == b.path {
                        let (a, bp) = (self.clone(), b.path.clone());
                        flow.append(&action("changes-allow-symbolic", "Déverrouiller…", "suggested-action", Box::new(move || a.unlock_dialog(&bp))));
                    } else if c.mounted_at().is_none() {
                        let (a, bp) = (self.clone(), b.path.clone());
                        flow.append(&action("changes-prevent-symbolic", "Verrouiller", "", Box::new(move || ops::lock(&a.conn, &bp, a.finish("Verrouillage impossible", "Volume verrouillé")))));
                    }
                }
                if c.filesystem.is_some() && !critical {
                    let (a, cp, cur) = (self.clone(), c.path.clone(), c.id_label.clone());
                    flow.append(&action("document-edit-symbolic", "Renommer…", "", Box::new(move || a.rename_dialog(&cp, &cur))));
                    let (a, cp, mounted) = (self.clone(), c.path.clone(), c.mounted_at().is_some());
                    flow.append(&action("emblem-ok-symbolic", "Vérifier", "", Box::new(move || a.check(&cp, false, mounted))));
                    let (a, cp, mounted) = (self.clone(), c.path.clone(), c.mounted_at().is_some());
                    flow.append(&action("applications-engineering-symbolic", "Réparer", "", Box::new(move || a.check(&cp, true, mounted))));
                }
                if !critical && !system {
                    let (a, bp) = (self.clone(), b.path.clone());
                    flow.append(&action("edit-clear-all-symbolic", "Formater…", "", Box::new(move || a.format_dialog(FormatTarget::Block(bp.clone())))));
                    if b.partition.is_some() {
                        let (a, bp, name) = (self.clone(), b.path.clone(), c.title());
                        flow.append(&action("user-trash-symbolic", "Supprimer…", "destructive-action", Box::new(move || a.delete_partition(&bp, &name))));
                    }
                } else if !critical {
                    let (a, bp) = (self.clone(), b.path.clone());
                    flow.append(&action("edit-clear-all-symbolic", "Formater…", "", Box::new(move || a.format_dialog(FormatTarget::Block(bp.clone())))));
                }
                if critical {
                    let note = label("Ce volume fait fonctionner NOOS : il ne peut être ni démonté, ni formaté, ni supprimé.", &["dim-label", "caption"]);
                    note.set_wrap(true);
                    v.append(&note);
                }
                let _ = d;
            }
        }
        v.append(&flow);
        v
    }

    // ───────────────────────────── Montage et droits ─────────────────────────────

    /// « Montage et droits » : montage automatique dans /mnt (fstab) et propriété du volume.
    fn mount_group(self: &Rc<Self>, c: &noos_disk::Block, d: &Drive) -> adap::PreferencesGroup {
        let g = group("Montage et droits");
        let (me, _) = ops::ids();
        // Montage automatique.
        let auto = adap::ActionRow::new();
        auto.set_use_markup(false);
        auto.add_prefix(&gtk::Image::from_icon_name("media-mount-symbolic"));
        match &c.fstab {
            Some((dir, opts)) => {
                auto.set_title("Monté automatiquement au démarrage");
                auto.set_subtitle(&format!("Toujours dans {dir}{}", if opts.contains("uid=") { " · fichiers à vous" } else { "" }));
                let b = gtk::Button::with_label("Désactiver");
                b.set_valign(gtk::Align::Center);
                let (a, uuid) = (self.clone(), c.id_uuid.clone());
                b.connect_clicked(move |_| a.run_helper(vec!["remove".into(), uuid.clone()], "Montage automatique désactivé"));
                auto.add_suffix(&b);
            }
            None => {
                auto.set_title("Montage automatique au démarrage");
                auto.set_subtitle(&format!(
                    "Toujours au même endroit : /mnt/{}{}",
                    ops::mount_name(&if c.id_label.is_empty() { c.title() } else { c.id_label.clone() }),
                    if d.is_external() { " (s'il est branché)" } else { "" }
                ));
                let b = gtk::Button::with_label("Configurer…");
                b.set_valign(gtk::Align::Center);
                let (a, c2) = (self.clone(), c.clone());
                b.connect_clicked(move |_| a.mnt_dialog(&c2));
                auto.add_suffix(&b);
            }
        }
        g.add(&auto);
        // Propriété.
        let own = adap::ActionRow::new();
        own.set_use_markup(false);
        own.add_prefix(&gtk::Image::from_icon_name("system-users-symbolic"));
        if !c.unix_fs() {
            own.set_title("Droits");
            own.set_subtitle(&format!("{} n'a pas de droits Linux : NOOS monte le volume avec vos droits, tout vous appartient.", noos_disk::fs_name(&c.id_type)));
        } else if let Some(mp) = c.mounted_at() {
            match ops::owner(mp) {
                Some(uid) if uid == me => {
                    own.set_title("Propriétaire : vous");
                    own.set_subtitle("Vous pouvez créer, modifier et supprimer des fichiers à la racine du volume");
                    let ok = gtk::Image::from_icon_name("object-select-symbolic");
                    ok.add_css_class("success");
                    own.add_suffix(&ok);
                }
                other => {
                    own.set_title(&format!("Propriétaire : {}", other.map(ops::user_name).unwrap_or_else(|| "inconnu".into())));
                    own.set_subtitle("Vous ne pouvez pas écrire à la racine de ce volume.");
                    let b = gtk::Button::with_label("M'en donner la propriété…");
                    b.add_css_class("suggested-action");
                    b.set_valign(gtk::Align::Center);
                    let (a, mp) = (self.clone(), mp.to_string());
                    b.connect_clicked(move |_| a.own_dialog(&mp));
                    own.add_suffix(&b);
                }
            }
        } else {
            own.set_title("Droits");
            own.set_subtitle("Montez le volume pour voir à qui il appartient");
        }
        g.add(&own);
        g
    }

    /// Lance le script privilégié hors du fil principal, puis relit l'état.
    fn run_helper(self: &Rc<Self>, args: Vec<String>, ok: &str) {
        let (a, ok) = (self.clone(), ok.to_string());
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || ops::helper(&args)).await.unwrap_or_else(|_| Err("échec du script".into()));
            match r {
                Ok(_) => a.toast(&ok),
                Err(e) => a.error("Opération impossible", &e),
            }
            a.reload();
        });
    }

    fn mnt_dialog(self: &Rc<Self>, c: &noos_disk::Block) {
        let base = if c.id_label.is_empty() { c.title() } else { c.id_label.clone() };
        let d = adap::AlertDialog::new(
            Some("Monter dans /mnt"),
            Some("Le volume sera monté maintenant et à chaque démarrage, toujours au même endroit, avec vos droits. Si le disque est absent, le démarrage n'est pas bloqué."),
        );
        let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let e = gtk::Entry::new();
        e.set_text(&ops::mount_name(&base));
        e.set_activates_default(true);
        let path = label("", &["dim-label", "caption"]);
        let upd = {
            let path = path.clone();
            move |e: &gtk::Entry| path.set_text(&format!("Emplacement : /mnt/{}", ops::mount_name(&e.text())))
        };
        upd(&e);
        e.connect_changed(upd);
        v.append(&e);
        v.append(&path);
        let own = gtk::CheckButton::with_label("Me donner la propriété du volume");
        own.set_active(true);
        if c.unix_fs() {
            v.append(&own);
        } else {
            v.append(&label(&format!("{} : fichiers attribués à votre compte automatiquement.", noos_disk::fs_name(&c.id_type)), &["dim-label", "caption"]));
        }
        d.set_extra_child(Some(&v));
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Monter")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let (a, c) = (self.clone(), c.clone());
        d.connect_response(None, move |_, r| {
            if r != "ok" {
                return;
            }
            let (uid, gid) = ops::ids();
            let dir = format!("/mnt/{}", ops::mount_name(&e.text()));
            a.toast(&format!("Montage dans {dir}…"));
            a.run_helper(
                vec!["setup".into(), c.id_uuid.clone(), c.id_type.clone(), dir.clone(), uid.to_string(), gid.to_string(), if own.is_active() { "1".into() } else { "0".into() }],
                &format!("Monté dans {dir}, aussi à chaque démarrage"),
            );
        });
        d.present(Some(&self.win));
    }

    fn own_dialog(self: &Rc<Self>, mp: &str) {
        let d = adap::AlertDialog::new(Some("Prendre possession du volume"), Some(&format!("Les fichiers de {mp} vous appartiendront : plus de « permission refusée »."))
        );
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let all = gtk::CheckButton::with_label("Tout le contenu (conseillé pour un disque de données)");
        let top = gtk::CheckButton::with_label("Seulement le dossier racine (les fichiers existants gardent leur propriétaire)");
        top.set_group(Some(&all));
        all.set_active(true);
        v.append(&all);
        v.append(&top);
        d.set_extra_child(Some(&v));
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Prendre possession")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        let (a, mp) = (self.clone(), mp.to_string());
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                let (uid, gid) = ops::ids();
                a.toast("Attribution des fichiers…");
                a.run_helper(vec!["own".into(), mp.clone(), uid.to_string(), gid.to_string(), if all.is_active() { "1".into() } else { "0".into() }], "Le volume vous appartient");
            }
        });
        d.present(Some(&self.win));
    }

    // ───────────────────────────── Actions ─────────────────────────────

    /// Démonte tous les volumes du disque, verrouille les volumes chiffrés, puis coupe
    /// l'alimentation (ou éjecte le support / détache l'image).
    fn safely_remove(self: &Rc<Self>, drive: &str) {
        let a = self.clone();
        let name = self.model.borrow().drive(drive).map(|d| d.name()).unwrap_or_default();
        ops::safely_remove(&self.conn, &self.model.borrow(), drive, move |r| {
            match r {
                Ok(()) => a.toast(&format!("Vous pouvez retirer « {name} » en toute sécurité")),
                Err(e) => a.error("Impossible de retirer le périphérique", &e),
            }
            a.reload();
        });
    }

    fn check(self: &Rc<Self>, block: &str, repair: bool, mounted: bool) {
        let run = {
            let (a, block) = (self.clone(), block.to_string());
            move || {
                let a2 = a.clone();
                a.toast(if repair { "Réparation en cours…" } else { "Vérification en cours…" });
                ops::check(&a.conn, &block, repair, move |r| {
                    match r {
                        Ok(true) => a2.toast(if repair { "Réparation terminée : le volume est sain" } else { "Aucune erreur trouvée" }),
                        Ok(false) => a2.error(
                            if repair { "Réparation incomplète" } else { "Erreurs trouvées" },
                            if repair { "Certaines erreurs n'ont pas pu être corrigées. Sauvegardez vos données." } else { "Le système de fichiers contient des erreurs. Utilisez « Réparer »." },
                        ),
                        Err(e) => a2.error(if repair { "Réparation impossible" } else { "Vérification impossible" }, &e),
                    }
                    a2.reload();
                });
            }
        };
        if mounted {
            // Vérifier / réparer demande un volume démonté : on démonte d'abord.
            let (a, block2) = (self.clone(), block.to_string());
            ops::unmount(&self.conn, block, move |r| match r {
                Ok(_) => run(),
                Err(e) => a.error("Impossible de démonter le volume", &e),
            });
            let _ = block2;
        } else {
            run();
        }
    }

    fn delete_partition(self: &Rc<Self>, block: &str, name: &str) {
        let d = adap::AlertDialog::new(Some(&format!("Supprimer « {name} » ?")), Some("La partition et toutes ses données seront définitivement effacées. L'espace deviendra libre."));
        d.add_responses(&[("cancel", "Annuler"), ("delete", "Supprimer")]);
        d.set_response_appearance("delete", adap::ResponseAppearance::Destructive);
        let (a, block) = (self.clone(), block.to_string());
        d.connect_response(None, move |_, r| {
            if r == "delete" {
                let (a2, b2) = (a.clone(), block.clone());
                // Démonter d'abord si besoin (sinon udisks refuse).
                ops::unmount(&a.conn, &block, move |_| ops::delete_partition(&a2.conn, &b2, a2.finish("Suppression impossible", "Partition supprimée")));
            }
        });
        d.present(Some(&self.win));
    }

    fn rename_dialog(self: &Rc<Self>, block: &str, current: &str) {
        let d = adap::AlertDialog::new(Some("Renommer le volume"), Some("Nouveau nom (étiquette) du volume :"));
        let e = gtk::Entry::new();
        e.set_text(current);
        e.set_activates_default(true);
        d.set_extra_child(Some(&e));
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Renommer")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let (a, block) = (self.clone(), block.to_string());
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                ops::set_label(&a.conn, &block, e.text().trim(), a.finish("Renommage impossible", "Volume renommé"));
            }
        });
        d.present(Some(&self.win));
    }

    fn unlock_dialog(self: &Rc<Self>, block: &str) {
        let d = adap::AlertDialog::new(Some("Déverrouiller le volume chiffré"), Some("Saisissez la phrase secrète de ce volume :"));
        let e = gtk::PasswordEntry::new();
        e.set_show_peek_icon(true);
        e.set_activates_default(true);
        d.set_extra_child(Some(&e));
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Déverrouiller")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("ok"));
        let (a, block) = (self.clone(), block.to_string());
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                let a2 = a.clone();
                ops::unlock(&a.conn, &block, &e.text(), move |r| {
                    match r {
                        Ok(_) => a2.toast("Volume déverrouillé"),
                        Err(e) => a2.error("Déverrouillage impossible", &e),
                    }
                    a2.reload();
                });
            }
        });
        d.present(Some(&self.win));
    }

    fn format_dialog(self: &Rc<Self>, target: FormatTarget) {
        let m = self.model.borrow().clone();
        let drive = self.drive.borrow().clone().and_then(|d| m.drive(&d).cloned()).unwrap_or_default();
        let (title, current_name, size, old_desc) = match &target {
            FormatTarget::Block(p) => {
                let b = m.block(p).cloned().unwrap_or_default();
                let c = m.content(&b).clone();
                let whole = b.partition.is_none();
                (
                    if whole { "Formater le disque entier" } else { "Formater le volume" },
                    if c.id_label.is_empty() { default_label(&drive) } else { c.id_label.clone() },
                    b.size,
                    format!("« {} » ({}, {})", c.title(), noos_disk::fs_name(&c.id_type), noos_disk::human(b.size)),
                )
            }
            FormatTarget::Free { size, .. } => ("Créer une partition", default_label(&drive), *size, String::new()),
        };
        let dlg = adap::Dialog::new();
        dlg.set_title(title);
        dlg.set_content_width(560);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        let page = adap::PreferencesPage::new();
        let g = group("");
        let name = adap::EntryRow::new();
        name.set_title("Nom du volume");
        name.set_text(&current_name);
        g.add(&name);
        let size_row = adap::SpinRow::with_range(0.01, (size as f64 / 1e9).max(0.01), 0.1);
        if matches!(target, FormatTarget::Free { .. }) {
            size_row.set_title("Taille (Go)");
            size_row.set_subtitle(&format!("{} disponibles", noos_disk::human(size)));
            size_row.set_digits(2);
            size_row.set_value(size as f64 / 1e9);
            g.add(&size_row);
        }
        page.add(&g);

        let types = group("Système de fichiers");
        let recommended = if drive.is_external() { "exfat" } else { "ext4" };
        let choices: Vec<(&str, &str, &str, &str)> = vec![
            ("exfat", "exFAT", "Compatible partout : Windows, macOS, Linux, TV, consoles. Idéal pour les clés USB et disques externes.", "network-workgroup-symbolic"),
            ("ntfs", "NTFS", "Pour Windows : fichiers de plus de 4 Go, droits Windows, journalisation. Lu et écrit par NOOS (ntfs-3g).", "computer-symbolic"),
            ("vfat", "FAT32", "Anciens appareils (autoradios, appareils photo, BIOS). Fichiers de 4 Go maximum.", "media-flash-symbolic"),
            ("ext4", "ext4", "Le classique de Linux, éprouvé et rapide. Garde les droits Unix.", "drive-harddisk-symbolic"),
            ("btrfs", "Btrfs", "Linux moderne : instantanés, compression transparente, sommes de contrôle.", "emblem-synchronizing-symbolic"),
            ("xfs", "XFS", "Linux, très performant pour les gros fichiers et gros volumes.", "folder-videos-symbolic"),
            ("f2fs", "F2FS", "Linux et Android, optimisé pour la mémoire flash (cartes SD).", "media-flash-symbolic"),
        ];
        let supported = m.supported.clone();
        let chosen = Rc::new(RefCell::new(recommended.to_string()));
        let mut first: Option<gtk::CheckButton> = None;
        for (id, t, desc, icon) in choices.into_iter().filter(|c| supported.is_empty() || supported.iter().any(|s| s == c.0)) {
            let r = adap::ActionRow::new();
            r.set_title(&if id == recommended { format!("{t}  · conseillé") } else { t.to_string() });
            r.set_subtitle(desc);
            r.add_prefix(&gtk::Image::from_icon_name(icon));
            let ck = gtk::CheckButton::new();
            if let Some(f) = &first {
                ck.set_group(Some(f));
            } else {
                first = Some(ck.clone());
            }
            ck.set_active(id == recommended);
            r.add_suffix(&ck);
            r.set_activatable_widget(Some(&ck));
            let chosen = chosen.clone();
            ck.connect_toggled(move |c| {
                if c.is_active() {
                    *chosen.borrow_mut() = id.to_string();
                }
            });
            types.add(&r);
        }
        page.add(&types);
        let opts_g = group("Options");
        let erase = adap::SwitchRow::new();
        erase.set_title("Effacer les données en profondeur");
        erase.set_subtitle("Écrit des zéros sur tout l'espace : les anciennes données ne pourront plus être récupérées (beaucoup plus long)");
        if matches!(target, FormatTarget::Block(_)) {
            opts_g.add(&erase);
            page.add(&opts_g);
        }
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bottom.set_halign(gtk::Align::End);
        bottom.set_margin_end(18);
        bottom.set_margin_bottom(18);
        bottom.set_margin_top(6);
        let go = gtk::Button::with_label(if matches!(target, FormatTarget::Free { .. }) { "Créer" } else { "Formater…" });
        go.add_css_class("pill");
        go.add_css_class(if matches!(target, FormatTarget::Free { .. }) { "suggested-action" } else { "destructive-action" });
        bottom.append(&go);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&page);
        page.set_vexpand(true);
        body.append(&bottom);
        tv.set_content(Some(&body));
        dlg.set_child(Some(&tv));
        dlg.set_content_height(720);
        {
            let (a, dlg2) = (self.clone(), dlg.clone());
            go.connect_clicked(move |_| {
                let kind = chosen.borrow().clone();
                let label = name.text().trim().to_string();
                let label = fit_label(&kind, &label);
                match &target {
                    FormatTarget::Free { table, offset, size } => {
                        dlg2.close();
                        let want = ((size_row.value() * 1e9) as u64).min(*size);
                        a.toast("Création de la partition…");
                        let tt = a.model.borrow().block(table).and_then(|b| b.table.clone()).unwrap_or_else(|| "gpt".into());
                        ops::create_partition(&a.conn, table, &tt, *offset, want, "", &kind, &label, a.finish("Création impossible", "Partition créée et formatée"));
                    }
                    FormatTarget::Block(p) => {
                        let confirm = adap::AlertDialog::new(
                            Some("Formater ?"),
                            Some(&format!("Toutes les données de {old_desc} seront effacées. Cette action est irréversible.")),
                        );
                        confirm.add_responses(&[("cancel", "Annuler"), ("format", "Formater")]);
                        confirm.set_response_appearance("format", adap::ResponseAppearance::Destructive);
                        let (a2, p2, dlg3, er) = (a.clone(), p.clone(), dlg2.clone(), erase.is_active());
                        confirm.connect_response(None, move |_, r| {
                            if r != "format" {
                                return;
                            }
                            dlg3.close();
                            let (a3, p3, kind, label) = (a2.clone(), p2.clone(), kind.clone(), label.clone());
                            a2.toast("Formatage…");
                            // Démonter d'abord (ignoré si déjà démonté).
                            ops::unmount(&a2.conn, &p2, move |_| ops::format(&a3.conn, &p3, &kind, &label, er, a3.finish("Formatage impossible", "Formatage terminé")));
                        });
                        confirm.present(Some(&a.win));
                    }
                }
            });
        }
        dlg.present(Some(&self.win));
    }

    /// Nouvelle table de partitions (GPT / MBR) puis, au choix, un volume sur tout le disque.
    fn init_dialog(self: &Rc<Self>) {
        let m = self.model.borrow().clone();
        let Some(d) = self.drive.borrow().clone().and_then(|p| m.drive(&p).cloned()) else { return };
        let Some(whole) = m.whole(&d.path).cloned() else { return };
        let dlg = adap::AlertDialog::new(
            Some("Initialiser le disque ?"),
            Some(&format!("Tout le contenu de « {} » ({}) sera effacé et le disque repartira de zéro.", d.name(), noos_disk::human(d.size))),
        );
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let gpt = gtk::CheckButton::with_label("GPT — moderne, conseillé (disques de plus de 2 To, UEFI)");
        let mbr = gtk::CheckButton::with_label("MBR — compatibilité avec les très vieux PC et appareils");
        mbr.set_group(Some(&gpt));
        gpt.set_active(true);
        let vol = gtk::CheckButton::with_label(&format!("Créer un volume {} sur tout le disque", if d.is_external() { "exFAT" } else { "ext4" }));
        vol.set_active(true);
        vol.set_margin_top(6);
        box_.append(&gpt);
        box_.append(&mbr);
        box_.append(&vol);
        dlg.set_extra_child(Some(&box_));
        dlg.add_responses(&[("cancel", "Annuler"), ("init", "Initialiser")]);
        dlg.set_response_appearance("init", adap::ResponseAppearance::Destructive);
        let a = self.clone();
        dlg.connect_response(None, move |_, r| {
            if r != "init" {
                return;
            }
            let table = if gpt.is_active() { "gpt" } else { "dos" };
            let (a2, wp, size, ext, with_vol, label) = (a.clone(), whole.path.clone(), whole.size, d.is_external(), vol.is_active(), default_label(&d));
            a.toast("Initialisation…");
            // Démonter tous les volumes du disque d'abord.
            let mounted: Vec<String> = a.model.borrow().volumes(&d.path).iter().filter(|v| v.mounted_at().is_some()).map(|v| v.path.clone()).collect();
            ops::unmount_all(&a.conn, mounted, move |r| {
                if let Err(e) = r {
                    return a2.error("Initialisation impossible", &e);
                }
                let a3 = a2.clone();
                let wp_fmt = wp.clone();
                ops::format(&a2.conn, &wp_fmt, table, "", false, move |r| {
                    if let Err(e) = r {
                        a3.error("Initialisation impossible", &e);
                        return a3.reload();
                    }
                    if !with_vol {
                        a3.toast("Disque initialisé");
                        return a3.reload();
                    }
                    const MIB: u64 = 1 << 20;
                    let fs = if ext { "exfat" } else { "ext4" };
                    ops::create_partition(&a3.conn, &wp, table, MIB, size - 2 * MIB, "", fs, &fit_label(fs, &label), a3.finish("Création du volume impossible", "Disque initialisé et prêt"));
                });
            });
        });
        dlg.present(Some(&self.win));
    }

    // ───────────────────────────── Outils ─────────────────────────────

    fn bench_dialog(self: &Rc<Self>) {
        let m = self.model.borrow().clone();
        let Some(d) = self.drive.borrow().clone().and_then(|p| m.drive(&p).cloned()) else { return };
        let Some(whole) = m.whole(&d.path).cloned() else { return };
        let dlg = adap::Dialog::new();
        dlg.set_title("Test de vitesse");
        dlg.set_content_width(620);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        let v = gtk::Box::new(gtk::Orientation::Vertical, 14);
        for s in [18] {
            v.set_margin_start(s);
            v.set_margin_end(s);
            v.set_margin_bottom(s);
        }
        let intro = label(&format!("Mesure la vitesse de lecture de « {} » en 24 points répartis sur tout le disque, puis le temps d'accès. Lecture seule : aucune donnée n'est modifiée.", d.name()), &["dim-label"]);
        intro.set_wrap(true);
        v.append(&intro);
        let numbers = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        numbers.set_homogeneous(true);
        let big = |title: &str| {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let n = label("–", &["bench-big"]);
            b.append(&n);
            b.append(&label(title, &["bench-unit"]));
            (b, n)
        };
        let (b1, avg) = big("Lecture moyenne (Mo/s)");
        let (b2, minmax) = big("Min – max (Mo/s)");
        let (b3, access) = big("Accès aléatoire (ms)");
        numbers.append(&b1);
        numbers.append(&b2);
        numbers.append(&b3);
        v.append(&numbers);
        let samples: Rc<RefCell<Vec<(f64, f64)>>> = Rc::new(RefCell::new(vec![]));
        let chart = gtk::DrawingArea::new();
        chart.set_content_height(150);
        {
            let samples = samples.clone();
            chart.set_draw_func(move |_, cr, w, h| {
                let (w, h) = (w as f64, h as f64);
                let s = samples.borrow();
                cr.set_source_rgba(0.5, 0.5, 0.5, 0.15);
                for k in 0..=4 {
                    let y = (h - 1.0) * k as f64 / 4.0 + 0.5;
                    cr.move_to(0.0, y);
                    cr.line_to(w, y);
                }
                let _ = cr.stroke();
                if s.len() < 2 {
                    return;
                }
                let max = s.iter().map(|x| x.1).fold(1.0, f64::max) * 1.15;
                cr.move_to(0.0, h);
                for (x, r) in s.iter() {
                    cr.line_to(x * w, h - r / max * h);
                }
                cr.line_to(s.last().unwrap().0 * w, h);
                cr.close_path();
                cr.set_source_rgba(0.35, 0.6, 1.0, 0.25);
                let _ = cr.fill();
                for (k, (x, r)) in s.iter().enumerate() {
                    if k == 0 { cr.move_to(x * w, h - r / max * h) } else { cr.line_to(x * w, h - r / max * h) }
                }
                cr.set_source_rgba(0.35, 0.6, 1.0, 1.0);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            });
        }
        v.append(&chart);
        let verdict = label("", &["heading"]);
        verdict.set_wrap(true);
        v.append(&verdict);
        let bar = gtk::ProgressBar::new();
        v.append(&bar);
        let start = gtk::Button::with_label("Démarrer");
        start.add_css_class("pill");
        start.add_css_class("suggested-action");
        start.set_halign(gtk::Align::Center);
        v.append(&start);
        tv.set_content(Some(&v));
        dlg.set_child(Some(&tv));
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let cancel = cancel.clone();
            dlg.connect_closed(move |_| cancel.store(true, std::sync::atomic::Ordering::Relaxed));
        }
        let a = self.clone();
        let start2 = start.clone();
        start2.connect_clicked(move |b| {
            b.set_sensitive(false);
            samples.borrow_mut().clear();
            let (avg, minmax, access, chart, verdict, bar, samples, cancel, size, b) =
                (avg.clone(), minmax.clone(), access.clone(), chart.clone(), verdict.clone(), bar.clone(), samples.clone(), cancel.clone(), whole.size, b.clone());
            let a2 = a.clone();
            ops::open_raw(&a.conn, &whole.path, "OpenForBenchmark", move |r| {
                let fd = match r {
                    Ok(fd) => fd,
                    Err(e) => {
                        b.set_sensitive(true);
                        return a2.error("Test impossible", &e);
                    }
                };
                let (ptx, prx) = async_channel::unbounded();
                let (rtx, rrx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = rtx.send_blocking(rawio::benchmark(fd, size, ptx, cancel));
                });
                let (bar2, avg2) = (bar.clone(), avg.clone());
                glib::spawn_future_local(async move {
                    while let Ok((d, t, rate)) = prx.recv().await {
                        bar2.set_fraction(d as f64 / t.max(1) as f64);
                        avg2.set_text(&format!("{:.0}", rate / 1e6));
                    }
                });
                glib::spawn_future_local(async move {
                    let Ok(res) = rrx.recv().await else { return };
                    b.set_sensitive(true);
                    b.set_label("Recommencer");
                    match res {
                        Ok(r) => {
                            avg.set_text(&format!("{:.0}", r.seq_avg / 1e6));
                            minmax.set_text(&format!("{:.0} – {:.0}", r.seq_min / 1e6, r.seq_max / 1e6));
                            access.set_text(&format!("{:.2}", r.access_ms));
                            *samples.borrow_mut() = r.samples.clone();
                            chart.queue_draw();
                            bar.set_fraction(1.0);
                            let mbs = r.seq_avg / 1e6;
                            verdict.set_text(match mbs {
                                x if x < 30.0 => "≈ USB 2.0 ou carte mémoire lente : bien pour des documents, lent pour de gros fichiers.",
                                x if x < 120.0 => "≈ clé USB 3 courante ou disque dur portable.",
                                x if x < 250.0 => "≈ disque dur rapide ou clé USB 3 haut de gamme.",
                                x if x < 650.0 => "≈ SSD SATA : très réactif.",
                                _ => "≈ SSD NVMe : la catégorie la plus rapide !",
                            });
                        }
                        Err(e) => verdict.set_text(&format!("Test interrompu : {e}")),
                    }
                });
            });
        });
        dlg.present(Some(&self.win));
        if std::env::var("NOOS_DISK_DIALOG").as_deref() == Ok("bench-run") {
            // (captures) le test se lance seul.
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || start.emit_clicked());
        }
    }

    fn smart_dialog(self: &Rc<Self>) {
        let m = self.model.borrow().clone();
        let Some(d) = self.drive.borrow().clone().and_then(|p| m.drive(&p).cloned()) else { return };
        let Some(h) = d.health.clone() else { return };
        let dlg = adap::Dialog::new();
        dlg.set_title("Santé du disque");
        dlg.set_content_width(640);
        dlg.set_content_height(640);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        let page = adap::PreferencesPage::new();
        let g = group(&format!("{} ({})", d.name(), if h.kind == "NVMe" { "NVMe" } else { "SMART" }));
        let status = if h.failing {
            "Défaillant : sauvegardez vos données au plus vite"
        } else if h.attributes_failing > 0 || h.bad_sectors.unwrap_or(0) > 0 {
            "À surveiller"
        } else {
            "Bon"
        };
        g.add(&info_row("État général", status));
        if let Some(t) = h.temperature_c {
            g.add(&info_row("Température", &format!("{t:.0} °C")));
        }
        if let Some(p) = h.power_on_hours {
            g.add(&info_row("Durée de fonctionnement", &format!("{p} h (≈ {:.1} ans)", p as f64 / 8766.0).replace('.', ",")));
        }
        if let Some(b) = h.bad_sectors {
            g.add(&info_row("Secteurs défectueux", &b.to_string()));
        }
        if let Some(u) = h.percent_used {
            g.add(&info_row("Usure estimée", &format!("{u} %")));
        }
        let st = match h.selftest.as_str() {
            "success" => "Réussi".to_string(),
            "inprogress" => format!("En cours ({} % restants)", h.selftest_remaining),
            "" => "Jamais lancé".to_string(),
            s => s.to_string(),
        };
        let test = info_row("Dernier autotest", &st);
        let run = gtk::Button::with_label("Lancer un autotest court");
        run.set_valign(gtk::Align::Center);
        {
            let (a, path, nvme) = (self.clone(), d.path.clone(), d.nvme);
            run.connect_clicked(move |b| {
                b.set_sensitive(false);
                ops::smart_selftest(&a.conn, &path, nvme, a.finish("Autotest impossible", "Autotest lancé (environ 2 minutes)"));
            });
        }
        test.add_suffix(&run);
        g.add(&test);
        page.add(&g);
        if h.kind == "ATA" {
            let attrs = group("Attributs SMART");
            attrs.set_description(Some("Valeur normalisée (plus haut = mieux) ; un attribut sous son seuil signale une défaillance."));
            page.add(&attrs);
            let attrs2 = attrs.clone();
            ops::smart_attributes(&self.conn, &d.path, move |r| {
                if let Err(e) = &r {
                    eprintln!("noos-disk : attributs SMART : {e}");
                    attrs2.set_description(Some(&format!("Attributs indisponibles : {e}")));
                }
                if let Ok(list) = r {
                    for (id, name, value, worst, thr, pretty) in list {
                        let r = adap::ActionRow::new();
                        r.set_use_markup(false);
                        r.set_title(&format!("{id} · {name}"));
                        r.set_subtitle(&format!("valeur {value} · pire {worst} · seuil {thr}"));
                        let l = label(&pretty, &["dim-label"]);
                        r.add_suffix(&l);
                        if thr > 0 && value >= 0 && value <= thr {
                            r.add_suffix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
                        }
                        attrs2.add(&r);
                    }
                }
            });
        }
        tv.set_content(Some(&page));
        dlg.set_child(Some(&tv));
        dlg.present(Some(&self.win));
    }

    /// Image disque : `backup` (disque → fichier) ou restauration (fichier → disque).
    fn image_dialog(self: &Rc<Self>, backup: bool) {
        let m = self.model.borrow().clone();
        let Some(d) = self.drive.borrow().clone().and_then(|p| m.drive(&p).cloned()) else { return };
        let Some(whole) = m.whole(&d.path).cloned() else { return };
        let fd_dialog = gtk::FileDialog::builder().title(if backup { "Enregistrer l'image disque" } else { "Choisir l'image à restaurer" }).build();
        let initial = format!("{}.img", d.name().replace([' ', '/'], "-"));
        let a = self.clone();
        let then = move |file: gio::File| {
            let Some(path) = file.path() else { return };
            let start = {
                let (a, whole, path) = (a.clone(), whole.clone(), path.clone());
                move || {
                    let a2 = a.clone();
                    let (whole2, path2) = (whole.clone(), path.clone());
                    ops::open_raw(&a.conn, &whole.path, if backup { "OpenForBackup" } else { "OpenForRestore" }, move |r| match r {
                        Ok(fd) => a2.copy_progress(fd, whole2.size, path2.clone(), backup),
                        Err(e) => a2.error("Accès au disque refusé", &e),
                    });
                }
            };
            if backup {
                start();
            } else {
                let c = adap::AlertDialog::new(
                    Some("Restaurer l'image ?"),
                    Some(&format!("« {} » remplacera tout le contenu de « {} ». Cette action est irréversible.", path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), d.name())),
                );
                c.add_responses(&[("cancel", "Annuler"), ("go", "Restaurer")]);
                c.set_response_appearance("go", adap::ResponseAppearance::Destructive);
                c.connect_response(None, move |_, r| {
                    if r == "go" {
                        start();
                    }
                });
                c.present(Some(&a.win));
            }
        };
        if backup {
            fd_dialog.set_initial_name(Some(&initial));
            fd_dialog.save(Some(&self.win), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    then(f)
                }
            });
        } else {
            fd_dialog.open(Some(&self.win), gio::Cancellable::NONE, move |r| {
                if let Ok(f) = r {
                    then(f)
                }
            });
        }
    }

    fn copy_progress(self: &Rc<Self>, fd: std::os::fd::OwnedFd, size: u64, path: std::path::PathBuf, backup: bool) {
        let dlg = adap::AlertDialog::new(Some(if backup { "Création de l'image…" } else { "Restauration de l'image…" }), Some(&path.display().to_string()));
        let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let bar = gtk::ProgressBar::new();
        bar.set_show_text(true);
        let info = label("", &["dim-label"]);
        v.append(&bar);
        v.append(&info);
        dlg.set_extra_child(Some(&v));
        dlg.add_response("cancel", "Annuler");
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let cancel = cancel.clone();
            dlg.connect_response(None, move |_, _| cancel.store(true, std::sync::atomic::Ordering::Relaxed));
        }
        dlg.present(Some(&self.win));
        let (ptx, prx) = async_channel::unbounded();
        let (rtx, rrx) = async_channel::bounded(1);
        let p2 = path.clone();
        std::thread::spawn(move || {
            let r = if backup { rawio::backup(fd, size, &p2, ptx, cancel) } else { rawio::restore(fd, &p2, ptx, cancel) };
            let _ = rtx.send_blocking(r);
        });
        glib::spawn_future_local(async move {
            while let Ok((done, total, rate)) = prx.recv().await {
                bar.set_fraction(done as f64 / total.max(1) as f64);
                let eta = if rate > 0.0 { (total.saturating_sub(done)) as f64 / rate } else { 0.0 };
                info.set_text(&format!("{} sur {} · {}/s · encore {}", noos_disk::human(done), noos_disk::human(total), noos_disk::human(rate as u64), duration(eta)));
            }
        });
        let a = self.clone();
        glib::spawn_future_local(async move {
            let Ok(r) = rrx.recv().await else { return };
            dlg.force_close();
            match r {
                Ok(()) => a.toast(if backup { "Image disque créée" } else { "Image restaurée" }),
                Err(e) => a.error(if backup { "Création de l'image interrompue" } else { "Restauration interrompue" }, &e),
            }
            a.reload();
        });
    }
}

enum FormatTarget {
    Block(String),
    Free { table: String, offset: u64, size: u64 },
}

fn duration(s: f64) -> String {
    let s = s.max(0.0) as u64;
    if s >= 3600 { format!("{} h {:02} min", s / 3600, s / 60 % 60) } else if s >= 60 { format!("{} min {:02} s", s / 60, s % 60) } else { format!("{s} s") }
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202f}');
        }
        out.push(c);
    }
    out
}

/// Longueur maximale des étiquettes : FAT 11 car. majuscules, NTFS 32, ext 16, exFAT 15.
fn fit_label(kind: &str, l: &str) -> String {
    let max = match kind {
        "vfat" => 11,
        "exfat" => 15,
        "ext4" | "ext3" | "ext2" => 16,
        "xfs" => 12,
        _ => 32,
    };
    let s: String = l.chars().take(max).collect();
    if kind == "vfat" { s.to_uppercase() } else { s }
}

fn default_label(d: &Drive) -> String {
    match d.kind() {
        "Clé USB" => "Clé USB".into(),
        "Carte mémoire" => "Carte".into(),
        _ => "Données".into(),
    }
}

fn partition_type_name(t: &str) -> String {
    match t.to_lowercase().as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" | "0xef" => "Partition système EFI".into(),
        "0fc63daf-8483-4772-8e79-3d69d8477de4" | "0x83" => "Données Linux".into(),
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" | "0x82" => "Échange Linux".into(),
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7" | "0x07" | "0x0c" | "0x0b" => "Données de base (Windows)".into(),
        "e3c9e316-0b5c-4db8-817d-f92df00215ae" => "Réservée Microsoft".into(),
        "de94bba4-06d1-4d40-a16a-bfd50179d6ac" => "Récupération Windows".into(),
        "bc13c2ff-59e6-4262-a352-b275fd6f7172" => "Démarrage étendu (/boot)".into(),
        "21686148-6449-6e6f-744e-656564454649" => "Démarrage BIOS".into(),
        "4f68bce3-e8cd-4db1-96e7-fbcaf984b709" => "Racine Linux (x86-64)".into(),
        "48465300-0000-11aa-aa11-00306543ecac" => "Apple HFS+".into(),
        "7c3457ef-0000-11aa-aa11-00306543ecac" => "Apple APFS".into(),
        "" => "inconnue".into(),
        other => other.to_string(),
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

fn open_folder(path: &str) {
    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>);
}

fn build(app: &adap::Application, start: Option<String>, format_target: Option<String>) {
    let conn = match udisks::system_bus() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("noos-disk : bus système : {e}");
            return;
        }
    };
    let win = adap::ApplicationWindow::builder().application(app).title("Disques").default_width(1120).default_height(780).build();
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    sidebar.set_header_func(|row, before| {
        let sec: &str = unsafe { row.data::<&str>("section").map(|p| *p.as_ref()).unwrap_or("") };
        let prev: &str = before.map(|b| unsafe { b.data::<&str>("section").map(|p| *p.as_ref()).unwrap_or("") }).unwrap_or("");
        if sec != prev {
            let l = label(sec, &["heading", "dim-label"]);
            l.set_margin_start(12);
            l.set_margin_top(if before.is_some() { 14 } else { 6 });
            l.set_margin_bottom(4);
            row.set_header(Some(&l));
        } else {
            row.set_header(gtk::Widget::NONE);
        }
    });
    let side_tv = adap::ToolbarView::new();
    let side_header = adap::HeaderBar::new();
    side_header.set_title_widget(Some(&adap::WindowTitle::new("Disques", "")));
    side_tv.add_top_bar(&side_header);
    side_tv.set_content(Some(&gtk::ScrolledWindow::builder().child(&sidebar).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 20);
    content.set_margin_top(18);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let clamp = adap::Clamp::new();
    clamp.set_maximum_size(980);
    clamp.set_child(Some(&content));
    let title = adap::WindowTitle::new("Disques", "");
    let main_tv = adap::ToolbarView::new();
    let header = adap::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Actualiser"));
    header.pack_start(&refresh);
    main_tv.add_top_bar(&header);
    let banner = adap::Banner::new("");
    main_tv.add_top_bar(&banner);
    let toasts = adap::ToastOverlay::new();
    toasts.set_child(Some(&gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build()));
    main_tv.set_content(Some(&toasts));

    let split = adap::NavigationSplitView::new();
    split.set_sidebar(Some(&adap::NavigationPage::new(&side_tv, "Disques")));
    split.set_content(Some(&adap::NavigationPage::new(&main_tv, "Disque")));
    split.set_min_sidebar_width(270.0);
    win.set_content(Some(&split));

    let a = Rc::new(App {
        win: win.clone(),
        conn: conn.clone(),
        model: RefCell::new(Model::default()),
        drive: RefCell::new(None),
        sel: RefCell::new(None),
        sidebar: sidebar.clone(),
        content,
        title,
        toasts,
        banner,
        syncing: Cell::new(false),
    });
    {
        let a2 = a.clone();
        sidebar.connect_row_selected(move |_, row| {
            if a2.syncing.get() {
                return;
            }
            let Some(row) = row else { return };
            let p: Option<String> = unsafe { row.data::<String>("drive").map(|p| p.as_ref().clone()) };
            if p != *a2.drive.borrow() {
                *a2.drive.borrow_mut() = p;
                *a2.sel.borrow_mut() = None;
                a2.fill_content();
            }
        });
    }
    {
        let a2 = a.clone();
        refresh.connect_clicked(move |_| a2.reload());
    }
    // Actions du menu « ⋮ ».
    let group = gio::SimpleActionGroup::new();
    let add = |name: &str, f: Rc<dyn Fn(&Rc<App>)>| {
        let act = gio::SimpleAction::new(name, None);
        let a2 = a.clone();
        act.connect_activate(move |_, _| f(&a2));
        group.add_action(&act);
    };
    add("bench", Rc::new(|a| a.bench_dialog()));
    add("smart", Rc::new(|a| a.smart_dialog()));
    add("backup", Rc::new(|a| a.image_dialog(true)));
    add("restore", Rc::new(|a| a.image_dialog(false)));
    add("init", Rc::new(|a| a.init_dialog()));
    win.insert_action_group("disk", Some(&group));
    {
        let w = Rc::downgrade(&a);
        let ids = udisks::watch(&conn, move || {
            if let Some(a) = w.upgrade() {
                a.reload();
            }
        });
        std::mem::forget(ids);
    }
    a.reload();

    // Disque / volume demandé en argument (/dev/sdb, /dev/sdb1).
    let want = format_target.clone().or(start);
    if let Some(dev) = want {
        let m = a.model.borrow().clone();
        if let Some(b) = m.blocks.iter().find(|b| b.device == dev || b.path == dev) {
            *a.drive.borrow_mut() = Some(b.drive.clone());
            if b.partition.is_some() || m.whole(&b.drive).is_some_and(|w| w.path == b.path) {
                *a.sel.borrow_mut() = Some(Sel::Block(b.path.clone()));
            }
            a.fill_sidebar();
            a.fill_content();
            if format_target.is_some() {
                let p = b.path.clone();
                let a2 = a.clone();
                glib::idle_add_local_once(move || a2.format_dialog(FormatTarget::Block(p)));
            }
        }
    }
    win.present();
    if let Ok(which) = std::env::var("NOOS_DISK_DIALOG") {
        let a2 = a.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || match which.as_str() {
            "format" => {
                let s = a2.sel.borrow().clone();
                match s {
                    Some(Sel::Block(p)) => a2.format_dialog(FormatTarget::Block(p)),
                    Some(Sel::Free { table, offset, size }) => a2.format_dialog(FormatTarget::Free { table, offset, size }),
                    None => {}
                }
            }
            "free" | "free-format" => {
                let m = a2.model.borrow().clone();
                let d = a2.drive.borrow().clone().and_then(|p| m.drive(&p).cloned());
                if let Some(seg) = d.map(|d| segments(&m, &d)).and_then(|s| s.into_iter().find(|x| matches!(x.2, Sel::Free { .. }))) {
                    a2.select(seg.2.clone());
                    if which == "free-format" {
                        if let Sel::Free { table, offset, size } = seg.2 {
                            a2.format_dialog(FormatTarget::Free { table, offset, size });
                        }
                    }
                }
            }
            "init" => a2.init_dialog(),
            "bench" | "bench-run" => a2.bench_dialog(),
            "smart" => a2.smart_dialog(),
            _ => {}
        });
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--selftest") {
        let code = selftest::run(args.get(i + 1).map(|s| s.as_str()).unwrap_or(""));
        return glib::ExitCode::from(code as u8);
    }
    let format_target = args.iter().position(|a| a == "--format").and_then(|i| args.get(i + 1).cloned());
    let start = args.iter().skip(1).find(|a| a.starts_with("/dev/") || a.starts_with("/org/")).cloned();
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    app.connect_activate(move |app| build(app, start.clone(), format_target.clone()));
    app.run_with_args::<&str>(&[])
}

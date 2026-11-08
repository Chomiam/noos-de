//! Périphériques amovibles : icône « Retirer le périphérique en toute sécurité » de la barre
//! (comme Windows), et fenêtre « Lecture automatique » quand on branche une clé USB, un disque
//! externe ou une carte mémoire (monter, ouvrir, importer les photos, gérer, formater…).
//! Les données viennent d'udisks2 (`noos_disk`), surveillé en continu sur le bus système.

use std::{
    cell::RefCell,
    collections::HashSet,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

use adap::prelude::*;
use gtk::{gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_disk::{Drive, Model, ops, udisks};

use crate::{Shell, apps};

const CSS: &str = r#"
.drives-menu { padding: 6px; min-width: 320px; }
.drives-menu .drives-title { font-weight: 700; margin: 6px 8px 8px 8px; }
.drives-menu .drive-item { border-radius: 8px; padding: 8px; }
.drives-menu .drive-sub { font-size: 0.82em; opacity: 0.65; }
.autoplay {
  padding: 18px; min-width: 400px; border-radius: 14px;
  background-color: @window_bg_color; color: @window_fg_color;
  border: 1px solid alpha(@window_fg_color, 0.10);
  box-shadow: 0 10px 34px rgba(0, 0, 0, 0.38);
}
.autoplay .ap-icon { min-width: 52px; min-height: 52px; border-radius: 14px; color: @accent_color;
  background: linear-gradient(135deg, alpha(@accent_bg_color, 0.30), alpha(@accent_bg_color, 0.08)); }
.autoplay row image.accent { color: @accent_color; }
"#;

pub struct Drives {
    shell: Weak<Shell>,
    conn: Option<gio::DBusConnection>,
    model: RefCell<Model>,
    buttons: RefCell<Vec<(gtk::MenuButton, gtk::Box)>>,
    /// Disques déjà vus (pour reconnaître un branchement).
    known: RefCell<HashSet<String>>,
    /// Disques branchés en attente de la fenêtre (on laisse les volumes apparaître).
    pending: RefCell<Vec<String>>,
    autoplay: RefCell<Option<gtk::Window>>,
    /// Disque de la fenêtre de lecture automatique ouverte.
    autoplay_drive: RefCell<Option<String>>,
}

/// Joue un son de NOOS (`data/sounds`) par PipeWire, sans bloquer.
pub fn play(name: &str) {
    let candidates = [noos_core::paths::data_dir().join("sounds").join(name), std::path::PathBuf::from("/opt/noos/share/noos/sounds").join(name)];
    let Some(file) = candidates.into_iter().find(|p| p.exists()) else { return };
    std::thread::spawn(move || {
        let ok = std::process::Command::new("pw-play").arg("--volume=0.7").arg(&file).status().is_ok_and(|s| s.success());
        if !ok {
            let _ = std::process::Command::new("paplay").arg(&file).status();
        }
    });
}

/// Notification par le serveur de notifications du shell (appel asynchrone : pas de blocage).
pub fn notify(summary: &str, body: &str, icon: &str) {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    let hints = glib::VariantDict::new(None).end();
    let args = ("Disques", 0u32, icon, summary, body, Vec::<String>::new(), hints, 6000i32).to_variant();
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

fn volume_desc(m: &Model, d: &Drive) -> String {
    let vols = m.volumes(&d.path);
    let fs: Vec<String> = vols.iter().map(|v| noos_disk::fs_name(&v.id_type)).collect::<Vec<_>>();
    let mut fs_unique: Vec<String> = vec![];
    for f in fs {
        if !fs_unique.contains(&f) {
            fs_unique.push(f);
        }
    }
    let mut parts = vec![d.kind().to_string(), noos_disk::human(d.size)];
    if !fs_unique.is_empty() {
        parts.push(fs_unique.join(", "));
    }
    parts.join(" · ")
}

impl Drives {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }
        let conn = udisks::system_bus().ok();
        let model = conn.as_ref().and_then(|c| udisks::load(c).ok()).unwrap_or_default();
        let known = model.drives.iter().map(|d| d.path.clone()).collect();
        let d = Rc::new(Self {
            shell: Rc::downgrade(shell),
            conn: conn.clone(),
            model: RefCell::new(model),
            buttons: RefCell::new(vec![]),
            known: RefCell::new(known),
            pending: RefCell::new(vec![]),
            autoplay: RefCell::new(None),
            autoplay_drive: RefCell::new(None),
        });
        if let Some(c) = &conn {
            let w = Rc::downgrade(&d);
            let ids = udisks::watch(c, move || {
                if let Some(d) = w.upgrade() {
                    d.changed();
                }
            });
            std::mem::forget(ids);
        }
        if std::env::var("NOOS_AUTOPLAY_DEMO").is_ok() {
            // Captures : fenêtre de lecture automatique pour le premier disque externe.
            let w = Rc::downgrade(&d);
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                if let Some(d) = w.upgrade() {
                    let ext = d.model.borrow().removable_drives().first().map(|x| x.path.clone());
                    if let Some(p) = ext {
                        d.show_autoplay(&p);
                    }
                }
            });
        }
        d
    }

    /// Bouton de la barre des tâches (un par barre / écran).
    pub fn button(self: &Rc<Self>) -> gtk::MenuButton {
        let b = gtk::MenuButton::new();
        b.set_valign(gtk::Align::Center);
        b.set_icon_name("media-removable-symbolic");
        b.set_always_show_arrow(false);
        b.set_direction(gtk::ArrowType::Up);
        b.add_css_class("status-icon");
        b.set_tooltip_text(Some("Retirer le périphérique en toute sécurité et éjecter le média"));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("drives-menu");
        let pop = gtk::Popover::new();
        pop.set_has_arrow(false);
        pop.set_child(Some(&list));
        b.set_popover(Some(&pop));
        noos_theme::fit_popover(&pop);
        self.buttons.borrow_mut().push((b.clone(), list.clone()));
        self.refresh_button(&b, &list);
        if std::env::var("NOOS_DRIVES_OPEN").is_ok() {
            let b2 = b.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || b2.popup());
        }
        b
    }

    fn changed(self: &Rc<Self>) {
        let Some(conn) = &self.conn else { return };
        let Ok(m) = udisks::load(conn) else { return };
        // Nouveaux disques externes (pas les nouveaux volumes d'un disque déjà là : un
        // formatage dans Disques ne doit pas rouvrir la fenêtre ; pas les images disque).
        let mut fresh_drives = vec![];
        let old = self.model.borrow().clone();
        let removed: Vec<String> = old
            .drives
            .iter()
            .filter(|d| d.is_external() && d.connection != "loop" && !old.is_system(&d.path) && m.drive(&d.path).is_none())
            .map(|d| d.path.clone())
            .collect();
        {
            let mut known = self.known.borrow_mut();
            for d in &m.drives {
                if known.insert(d.path.clone()) && d.is_external() && d.connection != "loop" && !m.is_system(&d.path) {
                    fresh_drives.push(d.path.clone());
                }
            }
            known.retain(|p| m.drive(p).is_some());
        }
        let sounds = self.shell.upgrade().is_some_and(|s| s.config().autoplay.sounds);
        if !removed.is_empty() {
            if sounds {
                play("noos-device-removed.wav");
            }
            // Retrait du disque de la fenêtre de lecture automatique : on la ferme.
            if self.autoplay_drive.borrow().as_ref().is_some_and(|d| removed.contains(d)) {
                if let Some(w) = self.autoplay.borrow_mut().take() {
                    w.destroy();
                }
            }
        }
        if !fresh_drives.is_empty() && sounds {
            play("noos-device-added.wav");
        }
        *self.model.borrow_mut() = m;
        let buttons: Vec<_> = self.buttons.borrow().clone();
        for (b, l) in &buttons {
            self.refresh_button(b, l);
        }
        for d in fresh_drives {
            if self.pending.borrow().contains(&d) {
                continue;
            }
            self.pending.borrow_mut().push(d.clone());
            // Laisser une seconde aux partitions pour apparaître, puis agir une seule fois.
            let w = Rc::downgrade(self);
            glib::timeout_add_local_once(Duration::from_millis(1200), move || {
                if let Some(s) = w.upgrade() {
                    s.pending.borrow_mut().retain(|x| x != &d);
                    s.plugged(&d);
                }
            });
        }
    }

    fn refresh_button(self: &Rc<Self>, b: &gtk::MenuButton, list: &gtk::Box) {
        let m = self.model.borrow().clone();
        let drives: Vec<Drive> = m.removable_drives().into_iter().cloned().collect();
        b.set_visible(!drives.is_empty());
        if b.popover().is_some_and(|p| p.is_visible()) {
            return;
        }
        while let Some(c) = list.first_child() {
            list.remove(&c);
        }
        let t = gtk::Label::new(Some("Retirer en toute sécurité"));
        t.add_css_class("drives-title");
        t.set_xalign(0.0);
        list.append(&t);
        for d in drives {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("drive-item");
            let icon = gtk::Image::from_icon_name(d.icon());
            icon.set_pixel_size(24);
            row.append(&icon);
            let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
            v.set_hexpand(true);
            let n = gtk::Label::new(Some(&format!("Éjecter {}", d.name())));
            n.set_xalign(0.0);
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&n);
            let mounted: Vec<String> = m.volumes(&d.path).iter().filter_map(|v| v.mounted_at().map(|mp| Path::new(mp).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default())).collect();
            let sub = if mounted.is_empty() { volume_desc(&m, &d) } else { format!("{} · {}", volume_desc(&m, &d), mounted.join(", ")) };
            let s = gtk::Label::new(Some(&sub));
            s.add_css_class("drive-sub");
            s.set_xalign(0.0);
            s.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&s);
            row.append(&v);
            // Ouvrir le premier volume monté.
            if let Some(mp) = m.volumes(&d.path).iter().find_map(|v| v.mounted_at()).map(String::from) {
                let open = gtk::Button::from_icon_name("folder-open-symbolic");
                open.add_css_class("flat");
                open.set_valign(gtk::Align::Center);
                open.set_tooltip_text(Some("Ouvrir"));
                let b2 = b.clone();
                open.connect_clicked(move |_| {
                    b2.popdown();
                    open_folder(&mp);
                });
                row.append(&open);
            }
            let ej = gtk::Button::from_icon_name("media-eject-symbolic");
            ej.add_css_class("flat");
            ej.set_valign(gtk::Align::Center);
            ej.set_tooltip_text(Some("Retirer en toute sécurité"));
            let (s2, path, b2, name) = (self.clone(), d.path.clone(), b.clone(), d.name());
            ej.connect_clicked(move |btn| {
                btn.set_sensitive(false);
                b2.popdown();
                s2.eject(&path, &name);
            });
            row.append(&ej);
            list.append(&row);
        }
        let manage = gtk::Button::with_label("Ouvrir Disques…");
        manage.add_css_class("flat");
        manage.set_margin_top(4);
        let b2 = b.clone();
        manage.connect_clicked(move |_| {
            b2.popdown();
            apps::spawn("noos-disk");
        });
        list.append(&manage);
    }

    pub fn eject(self: &Rc<Self>, drive: &str, name: &str) {
        let Some(conn) = self.conn.clone() else { return };
        let name = name.to_string();
        let m = self.model.borrow().clone();
        ops::safely_remove(&conn, &m, drive, move |r| match r {
            Ok(()) => notify("Le matériel peut être retiré", &format!("« {name} » peut être retiré de l'ordinateur en toute sécurité."), "media-removable-symbolic"),
            Err(e) => notify("Impossible de retirer le périphérique", &format!("« {name} » : {e}"), "dialog-warning-symbolic"),
        });
    }

    /// Branchement : montage puis lecture automatique (selon les réglages).
    fn plugged(self: &Rc<Self>, drive: &str) {
        let Some(shell) = self.shell.upgrade() else { return };
        let cfg = shell.config().autoplay;
        let m = self.model.borrow().clone();
        let Some(d) = m.drive(drive).cloned() else { return };
        let to_mount: Vec<String> = m
            .volumes(drive)
            .iter()
            .filter(|v| v.mounted_at().is_none() && !v.hint_ignore && v.fstab.is_none() && v.id_type != "swap")
            .map(|v| v.path.clone())
            .collect();
        let after = {
            let (s, drive, action) = (self.clone(), drive.to_string(), cfg.action.clone());
            move || {
                if let Some(m) = s.conn.as_ref().and_then(|c| udisks::load(c).ok()) {
                    *s.model.borrow_mut() = m;
                }
                let m = s.model.borrow().clone();
                let mp = m.volumes(&drive).iter().find_map(|v| v.mounted_at()).map(String::from);
                match action.as_str() {
                    "open" if mp.is_some() => open_folder(mp.as_deref().unwrap()),
                    "import" if mp.is_some() => import_media(mp.as_deref().unwrap(), &d.name()),
                    "nothing" => notify(&format!("{} connecté", d.kind()), &format!("{} ({})", d.name(), noos_disk::human(d.size)), "media-removable-symbolic"),
                    _ => s.show_autoplay(&drive),
                }
            }
        };
        if !cfg.mount || to_mount.is_empty() {
            return after();
        }
        let Some(conn) = self.conn.clone() else { return };
        mount_each(conn, to_mount, Rc::new(after));
    }

    /// Fenêtre « Lecture automatique » (coin inférieur droit, au-dessus de la barre), dans le
    /// style des applications NOOS : en-tête, puis une liste de choix (lignes libAdapta).
    fn show_autoplay(self: &Rc<Self>, drive: &str) {
        if let Some(w) = self.autoplay.borrow_mut().take() {
            w.destroy();
        }
        let Some(shell) = self.shell.upgrade() else { return };
        let m = self.model.borrow().clone();
        let Some(d) = m.drive(drive).cloned() else { return };
        let mount = m.volumes(drive).iter().find_map(|v| v.mounted_at()).map(String::from);
        let blank = m.volumes(drive).is_empty() && m.partitions(drive).iter().all(|p| p.id_type.is_empty());
        let device = m.whole(drive).map(|w| w.device.clone()).unwrap_or_default();

        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-autoplay"));
        window.set_layer(Layer::Overlay);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Bottom, 12);
        window.set_margin(Edge::Right, 12);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
        root.add_css_class("autoplay");

        // En-tête : icône, nom du volume, description du périphérique.
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let icon = gtk::Image::from_icon_name(d.icon());
        icon.set_pixel_size(28);
        icon.add_css_class("ap-icon");
        icon.set_valign(gtk::Align::Center);
        head.append(&icon);
        let tv = gtk::Box::new(gtk::Orientation::Vertical, 2);
        tv.set_hexpand(true);
        tv.set_valign(gtk::Align::Center);
        let label_name = m.volumes(drive).iter().map(|v| v.id_label.clone()).find(|l| !l.is_empty());
        let t = gtk::Label::new(Some(&label_name.clone().unwrap_or_else(|| d.name())));
        t.add_css_class("title-3");
        t.set_xalign(0.0);
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        tv.append(&t);
        let sub = gtk::Label::new(Some(&format!("{}{}", if label_name.is_some() { format!("{} · ", d.name()) } else { String::new() }, volume_desc(&m, &d))));
        sub.add_css_class("dim-label");
        sub.add_css_class("caption");
        sub.set_xalign(0.0);
        sub.set_wrap(true);
        sub.set_max_width_chars(42);
        tv.append(&sub);
        head.append(&tv);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.add_css_class("circular");
        close.set_valign(gtk::Align::Start);
        close.set_tooltip_text(Some("Fermer"));
        head.append(&close);
        root.append(&head);

        let q = gtk::Label::new(Some(if blank { "Ce périphérique n'est pas formaté" } else { "Que voulez-vous faire ?" }));
        q.add_css_class("heading");
        q.set_xalign(0.0);
        root.append(&q);

        let always = adap::SwitchRow::new();
        always.set_title("Toujours faire ceci");
        always.set_subtitle("Pour les clés USB et disques externes");
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let add_choice = |icon: &str, title: &str, subtitle: &str, key: &'static str, accent: bool, f: Box<dyn Fn()>| {
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title(title);
            if !subtitle.is_empty() {
                r.set_subtitle(subtitle);
            }
            let i = gtk::Image::from_icon_name(icon);
            i.set_pixel_size(20);
            if accent {
                i.add_css_class("accent");
            }
            r.add_prefix(&i);
            r.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            r.set_activatable(true);
            let (w, always, shell) = (window.clone(), always.clone(), Rc::downgrade(&shell));
            r.connect_activated(move |_| {
                if always.is_active() && !key.is_empty() {
                    if let Some(s) = shell.upgrade() {
                        s.update_config(|c| c.autoplay.action = key.to_string());
                    }
                }
                w.destroy();
                f();
            });
            list.append(&r);
        };
        if blank {
            let dev = device.clone();
            add_choice("edit-clear-all-symbolic", "Formater", "exFAT, NTFS, FAT32, ext4… avec Disques", "", true, Box::new(move || apps::spawn(&format!("noos-disk --format {dev}"))));
        }
        if let Some(mp) = mount.clone() {
            let mp2 = mp.clone();
            add_choice("folder-open-symbolic", "Ouvrir dans l'Explorateur", "Afficher les fichiers", "open", true, Box::new(move || open_folder(&mp2)));
            let media = count_media(&mp);
            if media.0 > 0 {
                let (mp3, name) = (mp.clone(), d.name());
                add_choice(
                    "camera-photo-symbolic",
                    "Importer les photos et vidéos",
                    &format!("{} fichier{} vers Images › Importées", media.0, if media.0 > 1 { "s" } else { "" }),
                    "import",
                    false,
                    Box::new(move || import_media(&mp3, &name)),
                );
            }
            if media.1 > 0 {
                let mp4 = mp.clone();
                add_choice("media-playback-start-symbolic", "Lire la musique et les vidéos", "Avec l'application par défaut", "", false, Box::new(move || play_media(&mp4)));
            }
        } else if !blank {
            // Volume non monté (montage automatique coupé ou refusé) : le monter puis l'ouvrir.
            let vols: Vec<String> = m.volumes(drive).iter().map(|v| v.path.clone()).collect();
            if let (Some(conn), false) = (self.conn.clone(), vols.is_empty()) {
                add_choice(
                    "folder-open-symbolic",
                    "Ouvrir dans l'Explorateur",
                    "Monter le volume et afficher les fichiers",
                    "open",
                    true,
                    Box::new(move || {
                        let c2 = conn.clone();
                        ops::mount(&c2, &vols[0], |r| match r {
                            Ok(mp) => open_folder(&mp),
                            Err(e) => notify("Ouverture impossible", &e, "dialog-warning-symbolic"),
                        })
                    }),
                );
            }
        }
        let dev2 = device.clone();
        add_choice("drive-harddisk-symbolic", "Gérer le périphérique", "Partitions, formatage, santé, vitesse", "", false, Box::new(move || apps::spawn(&format!("noos-disk {dev2}"))));
        if mount.is_some() {
            let (s2, path, name) = (self.clone(), d.path.clone(), d.name());
            add_choice("media-eject-symbolic", "Éjecter", "Retirer en toute sécurité", "", false, Box::new(move || s2.eject(&path, &name)));
        }
        add_choice("action-unavailable-symbolic", "Ne rien faire", "", "nothing", false, Box::new(|| {}));
        root.append(&list);
        if !blank {
            let opts = gtk::ListBox::new();
            opts.add_css_class("boxed-list");
            opts.set_selection_mode(gtk::SelectionMode::None);
            opts.append(&always);
            root.append(&opts);
        }
        window.set_child(Some(&root));
        {
            let w = window.clone();
            close.connect_clicked(move |_| w.destroy());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = window.clone();
            keys.connect_key_pressed(move |_, k, _, _| {
                if k == gtk::gdk::Key::Escape {
                    w.destroy();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(keys);
        }
        {
            // Fermeture automatique après 30 s sans choix.
            let w = window.downgrade();
            glib::timeout_add_local_once(Duration::from_secs(30), move || {
                if let Some(w) = w.upgrade() {
                    w.destroy();
                }
            });
        }
        window.present();
        *self.autoplay.borrow_mut() = Some(window);
        *self.autoplay_drive.borrow_mut() = Some(drive.to_string());
    }
}

/// Monte les volumes l'un après l'autre (droits de l'utilisateur), puis `then`.
fn mount_each(conn: gio::DBusConnection, mut blocks: Vec<String>, then: Rc<dyn Fn()>) {
    let Some(b) = blocks.pop() else { return then() };
    let (c2, b2) = (conn.clone(), b.clone());
    ops::mount(&conn, &b2, move |r| {
        if let Err(e) = r {
            eprintln!("noos-shell : montage de {b} : {e}");
        }
        mount_each(c2, blocks, then);
    });
}

fn open_folder(path: &str) {
    apps::spawn(&format!("noos-file '{}'", path.replace('\'', "'\\''")));
}

const PHOTO: &[&str] = &["jpg", "jpeg", "png", "heic", "heif", "webp", "nef", "cr2", "cr3", "arw", "dng", "raf", "orf", "rw2"];
const VIDEO: &[&str] = &["mp4", "mov", "m4v", "mkv", "avi", "mts", "3gp"];
const AUDIO: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "wav", "aac"];

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Parcours limité (profondeur 4, 4000 entrées) : (photos/vidéos, musique/vidéos).
fn count_media(root: &str) -> (usize, usize) {
    let (mut photos, mut play, mut seen) = (0, 0, 0);
    let mut stack = vec![(PathBuf::from(root), 0)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 4000 {
                return (photos, play);
            }
            let p = e.path();
            if p.is_dir() {
                if depth < 4 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    stack.push((p, depth + 1));
                }
                continue;
            }
            let x = ext(&p);
            if PHOTO.contains(&x.as_str()) || VIDEO.contains(&x.as_str()) {
                photos += 1;
            }
            if AUDIO.contains(&x.as_str()) || VIDEO.contains(&x.as_str()) {
                play += 1;
            }
        }
    }
    (photos, play)
}

fn play_media(root: &str) {
    // Le premier fichier audio / vidéo trouvé, avec l'application par défaut.
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
            } else if AUDIO.contains(&ext(&p).as_str()) || VIDEO.contains(&ext(&p).as_str()) {
                let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&p).uri(), None::<&gio::AppLaunchContext>);
                return;
            }
        }
    }
}

/// Copie photos et vidéos dans ~/Images/Importées/<date> <nom> (sans doublons), en
/// arrière-plan, puis notifie et ouvre le dossier.
fn import_media(root: &str, name: &str) {
    let pictures = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(|| glib::home_dir().join("Images"));
    let date = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default();
    let dest = pictures.join("Importées").join(format!("{date} {}", name.replace('/', "-")));
    let root = PathBuf::from(root);
    notify("Importation en cours", &format!("Photos et vidéos de « {name} »…"), "camera-photo-symbolic");
    let (tx, rx) = async_channel::bounded(1);
    let dest2 = dest.clone();
    std::thread::spawn(move || {
        let mut copied = 0usize;
        let mut stack = vec![root];
        let _ = std::fs::create_dir_all(&dest2);
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let x = ext(&p);
                if !(PHOTO.contains(&x.as_str()) || VIDEO.contains(&x.as_str())) {
                    continue;
                }
                let target = dest2.join(p.file_name().unwrap_or_default());
                if target.exists() {
                    continue;
                }
                if std::fs::copy(&p, &target).is_ok() {
                    copied += 1;
                }
            }
        }
        let _ = tx.send_blocking(copied);
    });
    let name = name.to_string();
    glib::spawn_future_local(async move {
        let n = rx.recv().await.unwrap_or(0);
        notify("Importation terminée", &format!("{n} fichier{} de « {name} » copiés dans {}", if n > 1 { "s" } else { "" }, dest.display()), "camera-photo-symbolic");
        if n > 0 {
            open_folder(&dest.to_string_lossy());
        }
    });
}

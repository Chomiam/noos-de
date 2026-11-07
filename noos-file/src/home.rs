//! Page « Accueil » (comme l'Accueil de l'Explorateur de Windows 11) : accès rapide aux
//! dossiers personnels, disques avec leur espace libre, fichiers récents.

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gio, glib};

use crate::{human_size, thumbs};

pub type Nav = Rc<dyn Fn(gio::File)>;

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("home-section");
    l.set_xalign(0.0);
    l
}

/// Dossiers de l'accès rapide : (nom, icône couleur, chemin).
fn quick_folders() -> Vec<(String, &'static str, std::path::PathBuf)> {
    let mut v = Vec::new();
    for (dir, name, icon) in [
        (glib::UserDirectory::Desktop, "Bureau", "user-desktop"),
        (glib::UserDirectory::Documents, "Documents", "folder-documents"),
        (glib::UserDirectory::Downloads, "Téléchargements", "folder-download"),
        (glib::UserDirectory::Pictures, "Images", "folder-pictures"),
        (glib::UserDirectory::Music, "Musique", "folder-music"),
        (glib::UserDirectory::Videos, "Vidéos", "folder-videos"),
    ] {
        if let Some(p) = glib::user_special_dir(dir).filter(|p| p.exists() && *p != glib::home_dir()) {
            v.push((name.to_string(), icon, p));
        }
    }
    v.push(("Dossier personnel".into(), "user-home", glib::home_dir()));
    v
}

fn folder_card(name: &str, icon: &str, path: &std::path::Path, nav: &Nav) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("home-card");
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(44);
    h.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(name));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    v.append(&t);
    let short = path
        .strip_prefix(glib::home_dir())
        .map(|p| format!("~/{}", p.display()))
        .unwrap_or_else(|_| path.display().to_string());
    let s = gtk::Label::new(Some(short.trim_end_matches('/')));
    s.add_css_class("dim-label");
    s.add_css_class("caption");
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    v.append(&s);
    h.append(&v);
    b.set_child(Some(&h));
    let (nav, file) = (nav.clone(), gio::File::for_path(path));
    b.connect_clicked(move |_| nav(file.clone()));
    b
}

/// Disques : la racine, puis les volumes montés (un par système de fichiers).
fn disks() -> Vec<(String, gio::Icon, gio::File)> {
    let mut v = vec![("Système (NOOS)".to_string(), gio::ThemedIcon::new("drive-harddisk").upcast(), gio::File::for_path("/"))];
    for m in gio::VolumeMonitor::get().mounts() {
        v.push((m.name().to_string(), m.icon(), m.root()));
    }
    let mut seen = Vec::new();
    v.retain(|(_, _, f)| {
        let id = f
            .query_info("id::filesystem", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
            .ok()
            .and_then(|i| i.attribute_string("id::filesystem").map(|s| s.to_string()))
            .unwrap_or_else(|| f.uri().to_string());
        if seen.contains(&id) {
            false
        } else {
            seen.push(id);
            true
        }
    });
    v
}

fn disk_card(name: &str, icon: &gio::Icon, root: &gio::File, nav: &Nav) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("home-card");
    b.add_css_class("disk");
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let img = gtk::Image::from_gicon(icon);
    img.set_pixel_size(44);
    h.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    v.set_hexpand(true);
    v.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(name));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    v.append(&t);
    let bar = gtk::LevelBar::new();
    bar.set_min_value(0.0);
    bar.set_max_value(1.0);
    // Jauge orange puis rouge quand le disque se remplit.
    bar.add_offset_value("full-low", 0.75);
    bar.add_offset_value("full-high", 0.9);
    v.append(&bar);
    let s = gtk::Label::new(Some("…"));
    s.add_css_class("dim-label");
    s.add_css_class("caption");
    s.set_xalign(0.0);
    v.append(&s);
    h.append(&v);
    b.set_child(Some(&h));
    if let Ok(fs) = root.query_filesystem_info("filesystem::size,filesystem::free,filesystem::type", gio::Cancellable::NONE) {
        let (size, free) = (fs.attribute_uint64("filesystem::size"), fs.attribute_uint64("filesystem::free"));
        if size > 0 {
            bar.set_value(1.0 - free as f64 / size as f64);
            let kind = fs.attribute_string("filesystem::type").map(|t| format!(" · {t}")).unwrap_or_default();
            s.set_text(&format!("{} libres sur {}{kind}", human_size(free as i64), human_size(size as i64)));
        }
    }
    let (nav, root) = (nav.clone(), root.clone());
    b.connect_clicked(move |_| nav(root.clone()));
    b
}

/// « il y a 5 min », « hier », « 3 nov. »…
fn relative(dt: &glib::DateTime) -> String {
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let secs = now.difference(dt).as_seconds();
    match secs {
        s if s < 60 => "à l'instant".into(),
        s if s < 3600 => format!("il y a {} min", s / 60),
        s if s < 86_400 => format!("il y a {} h", s / 3600),
        s if s < 2 * 86_400 => "hier".into(),
        _ => dt.to_local().ok().and_then(|d| d.format("%-d %b %Y").ok()).map(|s| s.to_string()).unwrap_or_default(),
    }
}

fn recent_list(nav: &Nav, open: &Rc<dyn Fn(gio::File)>) -> gtk::Widget {
    let items: Vec<gtk::RecentInfo> = gtk::RecentManager::default()
        .items()
        .into_iter()
        .filter(|r| r.is_local() && r.exists())
        .take(12)
        .collect();
    if items.is_empty() {
        let l = gtk::Label::new(Some("Les fichiers que vous ouvrez apparaîtront ici."));
        l.add_css_class("dim-label");
        l.set_xalign(0.0);
        return l.upcast();
    }
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    for r in items {
        let file = gio::File::for_uri(&r.uri());
        let row = adap::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&r.display_name());
        let folder = file
            .parent()
            .and_then(|p| p.path())
            .map(|p| {
                p.strip_prefix(glib::home_dir()).map(|q| format!("~/{}", q.display())).unwrap_or_else(|_| p.display().to_string())
            })
            .unwrap_or_default();
        row.set_subtitle(&format!("{} · {}", folder.trim_end_matches('/'), relative(&r.modified())));
        row.set_activatable(true);
        let img = gtk::Image::from_gicon(&r.gicon().unwrap_or_else(|| gio::ThemedIcon::new("text-x-generic").upcast()));
        img.set_pixel_size(36);
        row.add_prefix(&img);
        // Miniature si possible.
        {
            let (file, img) = (file.clone(), img.clone());
            glib::spawn_future_local(async move {
                let attrs = "standard::content-type,standard::size,time::modified";
                let Ok(info) = file.query_info_future(attrs, gio::FileQueryInfoFlags::NONE, glib::Priority::LOW).await else { return };
                if let Some(path) = thumbs::thumbnail(&file, &info).await {
                    img.set_from_file(Some(&path));
                    img.add_css_class("thumb");
                }
            });
        }
        let to_folder = gtk::Button::from_icon_name("folder-open-symbolic");
        to_folder.add_css_class("flat");
        to_folder.set_valign(gtk::Align::Center);
        to_folder.set_tooltip_text(Some("Afficher dans le dossier"));
        if let Some(parent) = file.parent() {
            let nav = nav.clone();
            to_folder.connect_clicked(move |_| nav(parent.clone()));
        }
        row.add_suffix(&to_folder);
        let (open, f) = (open.clone(), file.clone());
        row.connect_activated(move |_| open(f.clone()));
        list.append(&row);
    }
    list.upcast()
}

/// Construit (ou reconstruit) la page d'accueil dans `holder`.
pub fn fill(holder: &gtk::Box, nav: &Nav, open: &Rc<dyn Fn(gio::File)>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
    let hello = if (5..18).contains(&hour) { "Bonjour" } else { "Bonsoir" };
    let name = glib::real_name().to_string_lossy().to_string();
    let name = if name.is_empty() || name == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { name };
    let first = name.split_whitespace().next().unwrap_or(&name).to_string();
    let title = gtk::Label::new(Some(&format!("{hello}, {first}")));
    title.add_css_class("home-title");
    title.set_xalign(0.0);
    holder.append(&title);

    holder.append(&section("Accès rapide"));
    let quick = gtk::FlowBox::new();
    quick.set_selection_mode(gtk::SelectionMode::None);
    quick.set_min_children_per_line(2);
    quick.set_max_children_per_line(4);
    quick.set_column_spacing(10);
    quick.set_row_spacing(10);
    quick.set_homogeneous(true);
    for (name, icon, path) in quick_folders() {
        quick.append(&folder_card(&name, icon, &path, nav));
    }
    holder.append(&quick);

    holder.append(&section("Disques"));
    let drives = gtk::FlowBox::new();
    drives.set_selection_mode(gtk::SelectionMode::None);
    drives.set_min_children_per_line(1);
    drives.set_max_children_per_line(3);
    drives.set_column_spacing(10);
    drives.set_row_spacing(10);
    drives.set_homogeneous(true);
    for (name, icon, root) in disks() {
        drives.append(&disk_card(&name, &icon, &root, nav));
    }
    holder.append(&drives);

    holder.append(&section("Récents"));
    holder.append(&recent_list(nav, open));
}

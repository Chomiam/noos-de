//! Dossiers : raccourcis vers des dossiers (Documents, Téléchargements… par défaut), ouverts
//! dans l'Explorateur d'un clic.

use std::path::{Path, PathBuf};

use gtk::{gio, glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, open_path, section};

pub const CSS: &str = r#"
.desk-folder { padding: 8px 4px; border-radius: 12px; min-width: 72px; }
.desk-folder:hover { background-color: alpha(currentColor, 0.08); }
.desk-folder-tile { min-width: 44px; min-height: 44px; border-radius: 12px; color: white; }
.desk-folder-tile.c0 { background-image: linear-gradient(135deg, #4f8ef7, #2f6ae0); }
.desk-folder-tile.c1 { background-image: linear-gradient(135deg, #2ec27e, #1a9a5f); }
.desk-folder-tile.c2 { background-image: linear-gradient(135deg, #f6a33a, #e07b1a); }
.desk-folder-tile.c3 { background-image: linear-gradient(135deg, #c061cb, #9141ac); }
.desk-folder-tile.c4 { background-image: linear-gradient(135deg, #f66151, #c01c28); }
.desk-folder-tile.c5 { background-image: linear-gradient(135deg, #33c7de, #1c8fa8); }
.desk-folder-row { padding: 6px 8px; border-radius: 12px; }
.desk-folder-row:hover { background-color: alpha(currentColor, 0.08); }
.size-s .desk-folder { min-width: 48px; padding: 6px; }
.size-s .desk-folder-tile { min-width: 40px; min-height: 40px; }
.desk-folder-name { font-size: 0.85em; }
"#;

/// Pastille colorée du dossier n° `i`.
fn tile(p: &Path, i: usize, px: i32) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("desk-folder-tile");
    tile.add_css_class(&format!("c{}", i % 6));
    tile.set_halign(gtk::Align::Center);
    tile.set_valign(gtk::Align::Center);
    tile.set_hexpand(false);
    let img = gtk::Image::from_icon_name(icon(p));
    img.set_pixel_size(px);
    img.set_hexpand(true);
    img.set_halign(gtk::Align::Center);
    tile.append(&img);
    tile
}

fn defaults() -> Vec<PathBuf> {
    let home = glib::home_dir();
    // Dossiers XDG, sinon les noms habituels s'ils existent.
    let mut v: Vec<PathBuf> = [
        (glib::UserDirectory::Documents, ["Documents", "Documents"]),
        (glib::UserDirectory::Downloads, ["Téléchargements", "Downloads"]),
        (glib::UserDirectory::Pictures, ["Images", "Pictures"]),
        (glib::UserDirectory::Music, ["Musique", "Music"]),
        (glib::UserDirectory::Videos, ["Vidéos", "Videos"]),
    ]
    .into_iter()
    .filter_map(|(d, names)| glib::user_special_dir(d).filter(|p| *p != home).or_else(|| names.iter().map(|n| home.join(n)).find(|p| p.is_dir())))
    .collect();
    v.push(glib::home_dir());
    v
}

fn paths(w: &DesktopWidget) -> Vec<PathBuf> {
    match w.option("paths") {
        Some(p) => p.split(';').filter(|s| !s.is_empty()).map(PathBuf::from).collect(),
        None => defaults(),
    }
}

/// Icône du dossier (spéciaux reconnus).
fn icon(p: &Path) -> &'static str {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is = |d, names: [&str; 2]| glib::user_special_dir(d).as_deref() == Some(p) || names.contains(&n);
    if p == glib::home_dir() {
        "user-home-symbolic"
    } else if is(glib::UserDirectory::Documents, ["Documents", "Documents"]) {
        "folder-documents-symbolic"
    } else if is(glib::UserDirectory::Downloads, ["Téléchargements", "Downloads"]) {
        "folder-download-symbolic"
    } else if is(glib::UserDirectory::Pictures, ["Images", "Pictures"]) {
        "folder-pictures-symbolic"
    } else if is(glib::UserDirectory::Music, ["Musique", "Music"]) {
        "folder-music-symbolic"
    } else if is(glib::UserDirectory::Videos, ["Vidéos", "Videos"]) {
        "folder-videos-symbolic"
    } else if is(glib::UserDirectory::Desktop, ["Bureau", "Desktop"]) {
        "user-desktop-symbolic"
    } else {
        "folder-symbolic"
    }
}

fn name(p: &Path) -> String {
    if p == glib::home_dir() {
        return "Personnel".into();
    }
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if w.size() != "s" {
        let t = gtk::Label::new(Some("Dossiers"));
        t.add_css_class("desk-title");
        t.set_xalign(0.0);
        b.append(&t);
    }
    // Grande taille : une liste (nom et nombre d'éléments). Petite : les icônes seules.
    if w.size() == "l" {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.set_size_request(300, -1);
        for (i, p) in paths(w).into_iter().enumerate() {
            let btn = gtk::Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("desk-folder-row");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.append(&tile(&p, i, 20));
            let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
            col.set_valign(gtk::Align::Center);
            let l = gtk::Label::new(Some(&name(&p)));
            l.add_css_class("heading");
            l.set_xalign(0.0);
            col.append(&l);
            let n = std::fs::read_dir(&p).map(|d| d.take(10_000).filter_map(Result::ok).filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count()).unwrap_or(0);
            let c = gtk::Label::new(Some(&match n {
                0 => "Vide".to_string(),
                1 => "1 élément".to_string(),
                n => format!("{n} éléments"),
            }));
            c.add_css_class("desk-dim");
            c.add_css_class("caption");
            c.set_xalign(0.0);
            col.append(&c);
            row.append(&col);
            btn.set_child(Some(&row));
            btn.set_tooltip_text(Some(&p.to_string_lossy()));
            btn.connect_clicked(move |_| open_path(&p));
            list.append(&btn);
        }
        b.append(&list);
        return b.upcast();
    }
    let small = w.size() == "s";
    let grid = gtk::FlowBox::new();
    grid.set_selection_mode(gtk::SelectionMode::None);
    let cols = 3;
    grid.set_min_children_per_line(cols);
    grid.set_max_children_per_line(cols);
    grid.set_homogeneous(true);
    grid.set_column_spacing(4);
    grid.set_row_spacing(4);
    for (i, p) in paths(w).into_iter().enumerate() {
        let btn = gtk::Button::new();
        btn.add_css_class("flat");
        btn.add_css_class("desk-folder");
        let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
        v.append(&tile(&p, i, if small { 20 } else { 22 }));
        if !small {
            let l = gtk::Label::new(Some(&name(&p)));
            l.add_css_class("desk-folder-name");
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_max_width_chars(15);
            v.append(&l);
        }
        btn.set_child(Some(&v));
        btn.set_tooltip_text(Some(&format!("{} — {}", name(&p), p.to_string_lossy())));
        btn.connect_clicked(move |_| open_path(&p));
        grid.insert(&btn, -1);
    }
    b.append(&grid);
    b.upcast()
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Dossiers affichés");
    let list = paths(w);
    for (i, p) in list.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Image::from_icon_name(icon(p)));
        let l = gtk::Label::new(Some(&name(p)));
        l.set_hexpand(true);
        l.set_xalign(0.0);
        l.set_tooltip_text(Some(&p.to_string_lossy()));
        row.append(&l);
        let x = gtk::Button::from_icon_name("window-close-symbolic");
        x.add_css_class("flat");
        x.add_css_class("circular");
        x.set_tooltip_text(Some("Retirer"));
        let (ctx, list, n) = (ctx.clone(), list.clone(), name(p));
        x.connect_clicked(move |_| {
            let mut l = list.clone();
            l.remove(i);
            ctx.set_option(&format!("« {n} » retiré des dossiers"), "paths", &join(&l));
        });
        row.append(&x);
        b.append(&row);
    }
    let add = gtk::Button::with_label("Ajouter un dossier…");
    let (ctx, list) = (ctx.clone(), list.clone());
    add.connect_clicked(move |btn| {
        let d = gtk::FileDialog::builder().title("Ajouter un dossier").build();
        let (ctx, list) = (ctx.clone(), list.clone());
        let win = btn.root().and_downcast::<gtk::Window>();
        d.select_folder(win.as_ref(), gio::Cancellable::NONE, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                let mut l = list.clone();
                if !l.contains(&p) {
                    l.push(p.clone());
                }
                ctx.set_option(&format!("« {} » ajouté aux dossiers", name(&p)), "paths", &join(&l));
            }
        });
    });
    b.append(&add);
}

fn join(l: &[PathBuf]) -> String {
    l.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(";")
}

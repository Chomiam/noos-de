//! Applications › Applications par défaut.
//!
//! - **Tuiles** des utilisations courantes : l'application actuelle en grand, un clic ouvre un
//!   sélecteur (applications recommandées, actuelle cochée, recherche dans Logiciels) ; le choix
//!   vaut pour tous les types de la catégorie ;
//! - **par application** (comme Windows 11) : les types qu'une application sait ouvrir, ceux
//!   pour lesquels elle est déjà par défaut, « par défaut pour tout » ;
//! - **par type de fichier**, groupés par famille, avec les extensions réelles (base MIME
//!   `globs2`) et une recherche par extension ;
//! - **liens** (http, mailto, magnet…).
//!
//! Les choix sont écrits par GIO dans `~/.config/mimeapps.list`.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};

use crate::ui::{self, App, group};

/// (titre, icône, description, types MIME — le premier sert de référence).
pub const CATEGORIES: &[(&str, &str, &str, &[&str])] = &[
    (
        "Navigateur web",
        "web-browser-symbolic",
        "Liens et pages web",
        &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"],
    ),
    (
        "Images",
        "image-x-generic-symbolic",
        "JPEG, PNG, WebP, HEIC, RAW…",
        &[
            "image/jpeg", "image/png", "image/webp", "image/gif", "image/bmp", "image/tiff", "image/svg+xml", "image/avif",
            "image/heif", "image/heic", "image/x-icon", "image/jxl", "image/x-portable-pixmap", "image/x-nikon-nef",
            "image/x-canon-cr2", "image/x-canon-cr3", "image/x-sony-arw", "image/x-adobe-dng", "image/x-fuji-raf",
        ],
    ),
    (
        "Musique",
        "audio-x-generic-symbolic",
        "MP3, FLAC, Ogg, WAV…",
        &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/wav", "audio/aac", "audio/mp4", "audio/opus", "audio/x-vorbis+ogg"],
    ),
    (
        "Vidéos",
        "video-x-generic-symbolic",
        "MP4, MKV, WebM, AVI…",
        &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo", "video/mpeg", "video/ogg"],
    ),
    ("Documents PDF", "x-office-document-symbolic", "Fichiers .pdf", &["application/pdf"]),
    (
        "Texte et code",
        "text-x-generic-symbolic",
        "Texte, Markdown, scripts, code source",
        &[
            "text/plain", "text/markdown", "text/x-log", "application/x-shellscript", "application/json", "application/toml",
            "application/xml", "application/x-yaml", "text/x-python", "text/x-rust", "text/x-csrc", "text/x-c++src",
            "text/x-chdr", "text/javascript", "text/css", "text/x-java", "text/x-go", "text/x-csharp", "text/x-php",
            "text/x-sql", "text/x-makefile", "text/x-cmake",
        ],
    ),
    (
        "Archives",
        "package-x-generic-symbolic",
        "ZIP, TAR, 7z, RAR…",
        &[
            "application/zip", "application/x-tar", "application/x-compressed-tar", "application/x-xz-compressed-tar",
            "application/x-zstd-compressed-tar", "application/x-bzip-compressed-tar", "application/x-7z-compressed", "application/vnd.rar",
            "application/x-rar", "application/gzip", "application/x-cd-image", "application/x-iso9660-image", "application/x-apple-diskimage",
        ],
    ),
    ("Courriel", "mail-unread-symbolic", "Liens « mailto: »", &["x-scheme-handler/mailto"]),
    ("Dossiers", "folder-symbolic", "Gestionnaire de fichiers", &["inode/directory"]),
];

/// Recherche proposée dans Logiciels quand une catégorie n'a aucune application.
fn suggestion(category: &str) -> &'static str {
    match category {
        "Navigateur web" => "navigateur",
        "Courriel" => "thunderbird",
        "Images" => "image",
        "Musique" => "musique",
        "Vidéos" => "vlc",
        "Documents PDF" => "pdf",
        "Texte et code" => "éditeur de texte",
        "Archives" => "archive",
        _ => "",
    }
}

/// Familles de types de fichiers (préfixe MIME ou types précis).
const FAMILIES: &[(&str, &str)] = &[
    ("Images", "image-x-generic-symbolic"),
    ("Audio", "audio-x-generic-symbolic"),
    ("Vidéo", "video-x-generic-symbolic"),
    ("Documents", "x-office-document-symbolic"),
    ("Texte et code", "text-x-script-symbolic"),
    ("Archives", "package-x-generic-symbolic"),
    ("Polices", "font-x-generic-symbolic"),
    ("Autres", "application-x-executable-symbolic"),
];

fn family(mime: &str) -> usize {
    let is = |p: &str| mime.starts_with(p);
    if is("image/") {
        0
    } else if is("audio/") {
        1
    } else if is("video/") {
        2
    } else if is("application/vnd.oasis") || is("application/vnd.openxmlformats") || is("application/msword") || is("application/vnd.ms-")
        || mime == "application/pdf" || mime == "application/rtf" || is("application/epub") || is("application/x-mobipocket")
    {
        3
    } else if is("text/") || mime.ends_with("+xml") || mime.ends_with("json") || mime.contains("script") || mime.contains("yaml") || mime.contains("toml") || mime == "application/xml" || mime == "application/sql" {
        4
    } else if mime.contains("zip") || mime.contains("tar") || mime.contains("compressed") || mime.contains("rar") || mime.contains("archive") || mime == "application/gzip" || mime.contains("xz") || mime.contains("bzip") || mime.contains("zstd") {
        5
    } else if is("font/") || mime.contains("font") {
        6
    } else {
        7
    }
}

/// Extensions connues par type MIME (base MIME partagée, `globs2`).
fn globs() -> Rc<HashMap<String, Vec<String>>> {
    thread_local! {
        static GLOBS: RefCell<Option<Rc<HashMap<String, Vec<String>>>>> = const { RefCell::new(None) };
    }
    GLOBS.with(|g| {
        g.borrow_mut()
            .get_or_insert_with(|| {
                let mut map: HashMap<String, Vec<String>> = HashMap::new();
                let dirs = glib::system_data_dirs().into_iter().chain(std::iter::once(glib::user_data_dir()));
                for dir in dirs {
                    let Ok(text) = std::fs::read_to_string(dir.join("mime/globs2")) else { continue };
                    for line in text.lines().filter(|l| !l.starts_with('#')) {
                        let mut f = line.splitn(3, ':');
                        let (_, Some(mime), Some(glob)) = (f.next(), f.next(), f.next()) else { continue };
                        let Some(ext) = glob.strip_prefix("*.") else { continue };
                        if ext.contains(['*', '[', '?']) {
                            continue;
                        }
                        let v = map.entry(mime.to_string()).or_default();
                        let e = format!(".{}", ext.split(':').next().unwrap_or(ext).to_lowercase());
                        if !v.contains(&e) {
                            v.push(e);
                        }
                    }
                }
                Rc::new(map)
            })
            .clone()
    })
}

fn extensions(mime: &str) -> Vec<String> {
    let mut v = globs().get(mime).cloned().unwrap_or_default();
    v.sort_by_key(|e| e.len());
    v
}

pub fn app_icon(app: &gio::AppInfo, size: i32) -> gtk::Image {
    let img = match app.icon() {
        Some(icon) => gtk::Image::from_gicon(&icon),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    img.set_pixel_size(size);
    img
}

/// Applications capables d'ouvrir `mime`, sans doublons, les recommandées d'abord.
pub fn apps_for(mime: &str) -> Vec<gio::AppInfo> {
    let mut list = gio::AppInfo::recommended_for_type(mime);
    for a in gio::AppInfo::all_for_type(mime) {
        if !list.iter().any(|b| b.id() == a.id()) {
            list.push(a);
        }
    }
    list.retain(|a| a.should_show() || a.id().is_some_and(|id| id.starts_with("org.noos.")));
    list
}

fn same(a: &gio::AppInfo, b: &gio::AppInfo) -> bool {
    a.id() == b.id()
}

fn set_default(app: &Rc<App>, info: &gio::AppInfo, mimes: &[&str], quiet: bool) -> bool {
    let mut failed = Vec::new();
    for m in mimes {
        if let Err(e) = info.set_as_default_for_type(m) {
            failed.push(format!("{m} : {e}"));
        }
    }
    if !failed.is_empty() {
        app.error("Association impossible", &failed.join("\n"));
        return false;
    }
    if !quiet {
        app.toast(&format!("{} est maintenant l'application par défaut", info.display_name()));
    }
    true
}

fn chips(exts: &[String], max: usize) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for e in exts.iter().take(max) {
        let l = gtk::Label::new(Some(e));
        l.add_css_class("da-ext");
        b.append(&l);
    }
    if exts.len() > max {
        let l = gtk::Label::new(Some(&format!("+{}", exts.len() - max)));
        l.add_css_class("da-ext");
        l.add_css_class("dim-label");
        b.append(&l);
    }
    b.set_valign(gtk::Align::Center);
    b
}

/// Rouvre la page (après une modification qui change plusieurs éléments).
fn reload(app: &Rc<App>) {
    app.nav.pop();
    let p = page(app);
    app.push(&p);
}

// ───────────────────────────── Page ─────────────────────────────

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = ui::page(app, &["Applications"], "Applications par défaut");
    content.append(&tiles(app));
    content.append(&by_app(app));
    content.append(&by_type(app));
    content.append(&links(app));

    let g = group("");
    let reset = ui::card("edit-undo-symbolic", "Réinitialiser", "Effacer tous vos choix : le système reprend ses associations (NOOS : Visionneuse, Explorateur…)");
    let b = gtk::Button::with_label("Réinitialiser");
    b.add_css_class("destructive-action");
    b.set_valign(gtk::Align::Center);
    {
        let app = app.clone();
        b.connect_clicked(move |_| {
            let app2 = app.clone();
            app.confirm("Réinitialiser les applications par défaut ?", "Tous les choix faits ici seront effacés.", "Réinitialiser", true, move || {
                for ct in gio::content_types_get_registered() {
                    if let Some(m) = gio::content_type_get_mime_type(&ct) {
                        gio::AppInfo::reset_type_associations(&m);
                    }
                }
                for (_, _, _, mimes) in CATEGORIES {
                    for m in *mimes {
                        gio::AppInfo::reset_type_associations(m);
                    }
                }
                app2.toast("Applications par défaut réinitialisées");
                reload(&app2);
            });
        });
    }
    reset.add_suffix(&b);
    g.add(&reset);
    content.append(&g);
    p
}

// ───────────────────────────── Tuiles ─────────────────────────────

fn tiles(app: &Rc<App>) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let t = gtk::Label::new(Some("Utilisations courantes"));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    v.append(&t);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_min_children_per_line(2);
    flow.set_max_children_per_line(5);
    flow.set_column_spacing(10);
    flow.set_row_spacing(10);
    for (title, icon, desc, mimes) in CATEGORIES {
        let current = gio::AppInfo::default_for_type(mimes[0], false);
        let b = gtk::Button::new();
        b.add_css_class("da-tile");
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let ci = gtk::Image::from_icon_name(icon);
        ci.add_css_class("dim-label");
        top.append(&ci);
        let tl = gtk::Label::new(Some(title));
        tl.add_css_class("caption-heading");
        tl.set_xalign(0.0);
        tl.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&tl);
        col.append(&top);
        let big = match &current {
            Some(a) => app_icon(a, 52),
            None => {
                let i = gtk::Image::from_icon_name("list-add-symbolic");
                i.set_pixel_size(36);
                i.add_css_class("dim-label");
                i
            }
        };
        big.set_margin_top(6);
        big.set_margin_bottom(4);
        col.append(&big);
        let name = gtk::Label::new(Some(&current.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| "Choisir…".into())));
        name.add_css_class("da-tile-app");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&name);
        let d = gtk::Label::new(Some(desc));
        d.add_css_class("dim-label");
        d.add_css_class("caption");
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&d);
        b.set_child(Some(&col));
        let (app, title, mimes) = (app.clone(), *title, *mimes);
        b.connect_clicked(move |_| chooser(&app, title, mimes));
        flow.append(&b);
    }
    v.append(&flow);
    v
}

/// Sélecteur d'application pour une catégorie (ou un type) : grandes lignes avec icône.
fn chooser(app: &Rc<App>, title: &str, mimes: &'static [&'static str]) {
    let apps = apps_for(mimes[0]);
    let recommended: Vec<gio::AppInfo> = gio::AppInfo::recommended_for_type(mimes[0]);
    let current = gio::AppInfo::default_for_type(mimes[0], false);
    let dialog = adap::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(460);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
    v.set_margin_start(18);
    v.set_margin_end(18);
    v.set_margin_bottom(18);
    let exts: Vec<String> = mimes.iter().flat_map(|m| extensions(m)).take(14).collect();
    if !exts.is_empty() {
        let l = gtk::Label::new(Some(&format!("Pour {}", exts.join(" "))));
        l.add_css_class("dim-label");
        l.set_wrap(true);
        l.set_xalign(0.0);
        v.append(&l);
    }
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    if apps.is_empty() {
        list.append(&ui::card("dialog-information-symbolic", "Aucune application installée", "Trouvez-en une dans Logiciels"));
    }
    for a in &apps {
        let r = adap::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&a.display_name());
        let rec = recommended.iter().any(|b| same(a, b));
        let is_current = current.as_ref().is_some_and(|c| same(a, c));
        r.set_subtitle(match (is_current, rec) {
            (true, _) => "Actuellement par défaut",
            (false, true) => "Recommandée pour ce type",
            _ => "Peut aussi ouvrir ce type",
        });
        r.add_prefix(&app_icon(a, 40));
        if is_current {
            let c = gtk::Image::from_icon_name("object-select-symbolic");
            c.add_css_class("accent");
            r.add_suffix(&c);
        }
        r.set_activatable(true);
        let (app, info, d) = (app.clone(), a.clone(), dialog.clone());
        r.connect_activated(move |_| {
            d.close();
            if set_default(&app, &info, mimes, false) {
                reload(&app);
            }
        });
        list.append(&r);
    }
    let sc = gtk::ScrolledWindow::builder().child(&list).propagate_natural_height(true).max_content_height(460).hscrollbar_policy(gtk::PolicyType::Never).build();
    v.append(&sc);
    let more = gtk::Button::with_label("Trouver d'autres applications dans Logiciels");
    more.add_css_class("flat");
    let query = suggestion(title);
    let d2 = dialog.clone();
    more.connect_clicked(move |_| {
        d2.close();
        let _ = std::process::Command::new("noos-software").env("NOOS_SOFTWARE_SEARCH", query).spawn();
    });
    v.append(&more);
    tv.set_content(Some(&v));
    dialog.set_child(Some(&tv));
    dialog.present(Some(&app.window));
}

// ───────────────────────────── Par application ─────────────────────────────

fn by_app(app: &Rc<App>) -> adap::PreferencesGroup {
    let g = group("Par application");
    g.set_description(Some("Choisissez une application pour voir tous les types de fichiers qu'elle sait ouvrir."));
    let mut apps: Vec<(gio::AppInfo, usize, usize)> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let types: Vec<glib::GString> = a.supported_types().into_iter().filter(|t| !t.starts_with("x-scheme-handler/")).collect();
            if types.is_empty() {
                return None;
            }
            let defaults = types.iter().filter(|t| gio::AppInfo::default_for_type(t, false).is_some_and(|d| same(&d, &a))).count();
            Some((a, types.len(), defaults))
        })
        .collect();
    apps.sort_by_key(|(a, _, _)| a.display_name().to_lowercase());
    let exp = adap::ExpanderRow::new();
    exp.set_title(&format!("{} applications", apps.len()));
    exp.set_subtitle("Ouvrez une application pour définir ses associations");
    let icon = gtk::Image::from_icon_name("view-app-grid-symbolic");
    icon.add_css_class("win11-row-icon");
    exp.add_prefix(&icon);
    for (a, n, d) in apps {
        let r = adap::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&a.display_name());
        r.set_subtitle(&if d > 0 { format!("{n} types de fichiers · par défaut pour {d}") } else { format!("{n} types de fichiers") });
        r.add_prefix(&app_icon(&a, 32));
        r.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        r.set_activatable(true);
        let (app, info) = (app.clone(), a.clone());
        r.connect_activated(move |_| {
            let p = app_page(&app, &info);
            app.push(&p);
        });
        exp.add_row(&r);
    }
    g.add(&exp);
    g
}

fn app_page(app: &Rc<App>, info: &gio::AppInfo) -> adap::NavigationPage {
    let (p, content) = ui::page(app, &["Applications", "Applications par défaut"], &info.display_name());
    let types: Vec<String> = info.supported_types().into_iter().map(|t| t.to_string()).filter(|t| !t.starts_with("x-scheme-handler/")).collect();
    let big = app_icon(info, 72);
    let (h, actions) = ui::hero(
        &big,
        &info.display_name(),
        &info.description().map(|d| d.to_string()).unwrap_or_else(|| format!("Peut ouvrir {} types de fichiers", types.len())),
    );
    let all = gtk::Button::with_label("Par défaut pour tout");
    all.add_css_class("suggested-action");
    all.add_css_class("pill");
    {
        let (app, info, types) = (app.clone(), info.clone(), types.clone());
        all.connect_clicked(move |_| {
            let refs: Vec<&str> = types.iter().map(String::as_str).collect();
            if set_default(&app, &info, &refs, true) {
                app.toast(&format!("{} ouvre maintenant ses {} types de fichiers", info.display_name(), refs.len()));
                app.nav.pop();
                let p = app_page(&app, &info);
                app.push(&p);
            }
        });
    }
    actions.append(&all);
    content.append(&h);

    let g = group("Types de fichiers");
    for t in &types {
        let desc = gio::content_type_get_description(t).to_string();
        let r = adap::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&desc);
        let current = gio::AppInfo::default_for_type(t, false);
        let mine = current.as_ref().is_some_and(|c| same(c, info));
        r.set_subtitle(&if mine { "Par défaut".to_string() } else { format!("Actuellement : {}", current.map(|c| c.display_name().to_string()).unwrap_or_else(|| "aucune".into())) });
        let icon = gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(t));
        icon.add_css_class("win11-row-icon");
        r.add_prefix(&icon);
        r.add_suffix(&chips(&extensions(t), 3));
        let sw = gtk::Switch::new();
        sw.set_active(mine);
        sw.set_sensitive(!mine);
        sw.set_valign(gtk::Align::Center);
        sw.set_tooltip_text(Some(if mine { "Déjà par défaut : choisissez une autre application pour changer" } else { "Ouvrir ce type avec cette application" }));
        let (app, info, t, r2) = (app.clone(), info.clone(), t.clone(), r.clone());
        sw.connect_state_set(move |sw, on| {
            if on && set_default(&app, &info, &[&t], true) {
                r2.set_subtitle("Par défaut");
                sw.set_sensitive(false);
            }
            glib::Propagation::Proceed
        });
        r.add_suffix(&sw);
        g.add(&r);
    }
    content.append(&g);
    p
}

// ───────────────────────────── Par type ─────────────────────────────

fn by_type(app: &Rc<App>) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let g = group("Par type de fichier");
    g.set_description(Some("Recherchez une extension (« .odt », « mkv ») ou un nom (« vidéo »), ou parcourez par famille."));
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher un type ou une extension"));
    let results = gtk::ListBox::new();
    results.add_css_class("boxed-list");
    results.set_selection_mode(gtk::SelectionMode::None);
    results.set_visible(false);
    let families = gtk::ListBox::new();
    families.add_css_class("boxed-list");
    families.set_selection_mode(gtk::SelectionMode::None);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    holder.append(&search);
    holder.append(&results);
    holder.append(&families);
    g.add(&holder);
    v.append(&g);

    // Types connus (description, type MIME), regroupés par famille.
    let types: Rc<Vec<(String, String)>> = Rc::new({
        let mut v: Vec<(String, String)> = gio::content_types_get_registered()
            .into_iter()
            .filter_map(|ct| {
                let mime = gio::content_type_get_mime_type(&ct)?.to_string();
                if mime.starts_with("x-scheme-handler/") || mime.contains("x-content/") || mime.starts_with("inode/") && mime != "inode/directory" {
                    return None;
                }
                Some((gio::content_type_get_description(&ct).to_string(), mime))
            })
            .collect();
        v.sort_by_key(|(d, _)| d.to_lowercase());
        v.dedup_by(|a, b| a.1 == b.1);
        v
    });
    for (i, (name, icon)) in FAMILIES.iter().enumerate() {
        let members: Vec<(String, String)> = types.iter().filter(|(_, m)| family(m) == i).cloned().collect();
        let exp = adap::ExpanderRow::new();
        exp.set_title(name);
        exp.set_subtitle(&format!("{} types connus", members.len()));
        let ic = gtk::Image::from_icon_name(icon);
        ic.add_css_class("win11-row-icon");
        exp.add_prefix(&ic);
        // Remplissage au premier dépliage (des centaines de types).
        let filled = std::cell::Cell::new(false);
        let app = app.clone();
        exp.connect_expanded_notify(move |e| {
            if !e.is_expanded() || filled.replace(true) {
                return;
            }
            let mut n = 0;
            for (desc, mime) in &members {
                let apps = apps_for(mime);
                if apps.is_empty() {
                    continue;
                }
                e.add_row(&type_row(&app, desc, mime, &apps));
                n += 1;
            }
            e.set_subtitle(&format!("{n} types avec une application"));
        });
        families.append(&exp);
    }

    let app = app.clone();
    search.connect_search_changed(move |s| {
        while let Some(c) = results.first_child() {
            results.remove(&c);
        }
        let q = s.text().trim().to_lowercase();
        let q = q.trim_start_matches('.').to_string();
        let searching = q.len() >= 2;
        results.set_visible(searching);
        families.set_visible(!searching);
        if !searching {
            return;
        }
        let dot = format!(".{q}");
        let mut shown = 0;
        // Extension exacte d'abord, puis le reste.
        let mut matches: Vec<&(String, String)> = types
            .iter()
            .filter(|(d, m)| d.to_lowercase().contains(&q) || m.contains(&q) || extensions(m).iter().any(|e| e.starts_with(&dot)))
            .collect();
        matches.sort_by_key(|(_, m)| !extensions(m).iter().any(|e| *e == dot));
        for (desc, mime) in matches {
            let apps = apps_for(mime);
            if apps.is_empty() {
                continue;
            }
            results.append(&type_row(&app, desc, mime, &apps));
            shown += 1;
            if shown >= 40 {
                break;
            }
        }
        if shown == 0 {
            results.append(&ui::card("edit-find-symbolic", "Aucun type de fichier trouvé", "Aucune application installée ne l'ouvre, ou le type est inconnu"));
        }
    });
    v
}

/// Ligne « type de fichier » : extensions, application actuelle, menu pour en choisir une autre.
fn type_row(app: &Rc<App>, desc: &str, mime: &str, apps: &[gio::AppInfo]) -> adap::ActionRow {
    let row = adap::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(desc);
    let current = gio::AppInfo::default_for_type(mime, false);
    let current_name = current.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| "Aucune".into());
    row.set_subtitle(&current_name);
    let icon = gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(mime));
    icon.set_pixel_size(20);
    icon.add_css_class("win11-row-icon");
    row.add_prefix(&icon);
    row.add_suffix(&chips(&extensions(mime), 3));
    let choose = gtk::MenuButton::new();
    choose.set_valign(gtk::Align::Center);
    let face = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    match &current {
        Some(c) => face.append(&app_icon(c, 20)),
        None => face.append(&gtk::Image::from_icon_name("list-add-symbolic")),
    }
    face.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    choose.set_child(Some(&face));
    choose.set_tooltip_text(Some("Changer d'application"));
    let pop = gtk::Popover::new();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for a in apps {
        let b = gtk::Button::new();
        b.add_css_class("flat");
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        h.append(&app_icon(a, 24));
        let l = gtk::Label::new(Some(&a.display_name()));
        l.set_xalign(0.0);
        h.append(&l);
        if current.as_ref().is_some_and(|c| same(c, a)) {
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.set_hexpand(true);
            check.set_halign(gtk::Align::End);
            h.append(&check);
        }
        b.set_child(Some(&h));
        let (app, info, mime, pop2, row2, face2) = (app.clone(), a.clone(), mime.to_string(), pop.clone(), row.clone(), face.clone());
        b.connect_clicked(move |_| {
            pop2.popdown();
            if set_default(&app, &info, &[&mime], false) {
                row2.set_subtitle(&info.display_name());
                if let Some(first) = face2.first_child() {
                    face2.remove(&first);
                }
                face2.prepend(&app_icon(&info, 20));
            }
        });
        box_.append(&b);
    }
    pop.set_child(Some(&box_));
    noos_theme::fit_popover(&pop);
    choose.set_popover(Some(&pop));
    row.add_suffix(&choose);
    row
}

// ───────────────────────────── Liens ─────────────────────────────

const SCHEMES: &[(&str, &str, &str)] = &[
    ("https", "Pages web", "web-browser-symbolic"),
    ("mailto", "Adresses de courriel", "mail-unread-symbolic"),
    ("magnet", "Liens magnet (torrent)", "folder-download-symbolic"),
    ("tel", "Numéros de téléphone", "call-start-symbolic"),
    ("irc", "Salons IRC", "user-available-symbolic"),
    ("ssh", "Connexions SSH", "utilities-terminal-symbolic"),
    ("steam", "Liens Steam", "input-gaming-symbolic"),
    ("discord", "Liens Discord", "user-available-symbolic"),
    ("spotify", "Liens Spotify", "audio-x-generic-symbolic"),
    ("zoommtg", "Réunions Zoom", "camera-video-symbolic"),
    ("vscode", "Liens Visual Studio Code", "text-x-script-symbolic"),
];

fn links(app: &Rc<App>) -> adap::PreferencesGroup {
    let g = group("Liens");
    g.set_description(Some("Applications qui s'ouvrent quand vous cliquez sur un lien d'un type particulier."));
    for (scheme, desc, icon) in SCHEMES {
        let mime = format!("x-scheme-handler/{scheme}");
        let apps = apps_for(&mime);
        if apps.is_empty() {
            continue;
        }
        let r = type_row(app, &format!("{scheme}: — {desc}"), &mime, &apps);
        // Icône parlante plutôt que l'icône générique du type « lien ».
        let i = gtk::Image::from_icon_name(icon);
        i.add_css_class("win11-row-icon");
        r.add_prefix(&i);
        g.add(&r);
    }
    g
}

//! Applications › Applications par défaut (comme Windows 11).
//!
//! - Utilisations courantes : un choix par catégorie (navigateur, images, musique…), appliqué à
//!   tous les types MIME de la catégorie ;
//! - par type de fichier : recherche parmi les types connus et choix de l'application ;
//! - réinitialisation : retour aux associations du système (`mimeapps.list` de l'utilisateur).
//!
//! Les choix sont écrits par GIO dans `~/.config/mimeapps.list` : ils valent pour toutes les
//! applications (Explorateur, navigateurs, liens…).

use std::rc::Rc;

use adap::prelude::*;
use gtk::gio;

use crate::ui::{App, card, group, hero, link, page};

/// (titre, icône, description, types MIME — le premier sert de référence).
const CATEGORIES: &[(&str, &str, &str, &[&str])] = &[
    (
        "Navigateur web",
        "web-browser-symbolic",
        "Liens et pages web",
        &["x-scheme-handler/http", "x-scheme-handler/https", "text/html", "application/xhtml+xml"],
    ),
    ("Courriel", "mail-unread-symbolic", "Liens « mailto: »", &["x-scheme-handler/mailto"]),
    (
        "Images",
        "image-x-generic-symbolic",
        "JPEG, PNG, WebP, GIF, HEIC…",
        &[
            "image/jpeg", "image/png", "image/webp", "image/gif", "image/bmp", "image/tiff", "image/svg+xml", "image/avif",
            "image/heif", "image/heic", "image/x-icon", "image/jxl", "image/x-portable-pixmap",
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
        "Texte",
        "text-x-generic-symbolic",
        "Fichiers texte, Markdown, configuration",
        &["text/plain", "text/markdown", "text/x-log", "application/x-shellscript", "application/json", "application/toml"],
    ),
    (
        "Archives",
        "package-x-generic-symbolic",
        "ZIP, TAR, 7z, RAR…",
        &[
            "application/zip", "application/x-tar", "application/x-compressed-tar", "application/x-xz-compressed-tar",
            "application/x-7z-compressed", "application/vnd.rar", "application/x-rar", "application/gzip",
        ],
    ),
    ("Dossiers", "folder-symbolic", "Gestionnaire de fichiers", &["inode/directory"]),
];

/// Recherche proposée dans Logiciels quand une catégorie n'a aucune application.
fn suggestion(category: &str) -> Option<&'static str> {
    Some(match category {
        "Navigateur web" => "firefox",
        "Courriel" => "thunderbird",
        "Images" => "image",
        "Musique" => "rhythmbox",
        "Vidéos" => "vlc",
        "Documents PDF" => "papers",
        "Texte" => "text editor",
        "Archives" => "file-roller",
        _ => return None,
    })
}

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Applications");
    let g = group("");
    g.add(&link(
        app,
        "preferences-system-symbolic",
        "Applications par défaut",
        "Applications qui ouvrent vos fichiers et vos liens",
        default_apps,
    ));
    let installed = card("view-app-grid-symbolic", "Applications installées", "Installer, mettre à jour ou supprimer des applications");
    installed.set_activatable(true);
    installed.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    installed.connect_activated(|_| {
        let _ = std::process::Command::new("noos-software").env("NOOS_SOFTWARE_PAGE", "installed").spawn();
    });
    g.add(&installed);
    content.append(&g);
    p
}

fn app_icon(app: &gio::AppInfo, size: i32) -> gtk::Image {
    let img = match app.icon() {
        Some(icon) => gtk::Image::from_gicon(&icon),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    img.set_pixel_size(size);
    img
}

/// Applications capables d'ouvrir `mime`, sans doublons, la recommandée d'abord.
fn apps_for(mime: &str) -> Vec<gio::AppInfo> {
    let mut list = gio::AppInfo::recommended_for_type(mime);
    for a in gio::AppInfo::all_for_type(mime) {
        if !list.iter().any(|b| b.id() == a.id()) {
            list.push(a);
        }
    }
    list.retain(|a| a.should_show() || a.id().is_some_and(|id| id.starts_with("org.noos.")));
    list
}

/// Liste déroulante d'applications (icône + nom).
fn app_combo(title: &str, subtitle: &str, icon: &str, apps: &[gio::AppInfo], current: Option<&gio::AppInfo>) -> adap::ComboRow {
    let store = gio::ListStore::new::<gio::AppInfo>();
    for a in apps {
        store.append(a);
    }
    let row = adap::ComboRow::new();
    row.set_use_markup(false);
    row.set_title(title);
    row.set_subtitle(subtitle);
    let prefix = gtk::Image::from_icon_name(icon);
    prefix.set_pixel_size(20);
    prefix.add_css_class("win11-row-icon");
    row.add_prefix(&prefix);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        h.append(&gtk::Image::new());
        let l = gtk::Label::new(None);
        l.set_xalign(0.0);
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        h.append(&l);
        item.set_child(Some(&h));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(a) = item.item().and_downcast::<gio::AppInfo>() else { return };
        let h = item.child().and_downcast::<gtk::Box>().unwrap();
        let img = h.first_child().and_downcast::<gtk::Image>().unwrap();
        match a.icon() {
            Some(icon) => img.set_from_gicon(&icon),
            None => img.set_icon_name(Some("application-x-executable")),
        }
        img.set_pixel_size(20);
        h.last_child().and_downcast::<gtk::Label>().unwrap().set_text(&a.display_name());
    });
    row.set_factory(Some(&factory));
    row.set_model(Some(&store));
    let pos = current.and_then(|c| apps.iter().position(|a| a.id() == c.id())).unwrap_or(0);
    row.set_selected(pos as u32);
    row
}

fn set_default(app: &Rc<App>, info: &gio::AppInfo, mimes: &[&str]) {
    let mut failed = Vec::new();
    for m in mimes {
        if let Err(e) = info.set_as_default_for_type(m) {
            failed.push(format!("{m} : {e}"));
        }
    }
    if failed.is_empty() {
        app.toast(&format!("{} est maintenant l'application par défaut", info.display_name()));
    } else {
        app.error("Association impossible", &failed.join("\n"));
    }
}

pub fn default_apps(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Applications"], "Applications par défaut");
    let img = gtk::Image::from_icon_name("preferences-system-symbolic");
    img.set_pixel_size(72);
    let (h, actions) = hero(
        &img,
        "Choisissez les applications qui ouvrent vos fichiers",
        "Le choix s'applique partout : Explorateur, liens, pièces jointes et autres applications.",
    );
    let reset = gtk::Button::with_label("Réinitialiser");
    reset.add_css_class("pill");
    {
        let app = app.clone();
        reset.connect_clicked(move |_| {
            let app2 = app.clone();
            app.confirm(
                "Réinitialiser les applications par défaut ?",
                "Les choix faits ici pour les utilisations courantes seront effacés : le système reprend ses associations.",
                "Réinitialiser",
                true,
                move || {
                    for (_, _, _, mimes) in CATEGORIES {
                        for m in *mimes {
                            gio::AppInfo::reset_type_associations(m);
                        }
                    }
                    app2.toast("Applications par défaut réinitialisées");
                    app2.nav.pop();
                    let page = default_apps(&app2);
                    app2.push(&page);
                },
            );
        });
    }
    actions.append(&reset);
    content.append(&h);

    // ── Utilisations courantes ──
    let common = group("Utilisations courantes");
    for (title, icon, desc, mimes) in CATEGORIES {
        let apps = apps_for(mimes[0]);
        let current = gio::AppInfo::default_for_type(mimes[0], false);
        if apps.is_empty() {
            let r = card(icon, title, &format!("{desc} — aucune application installée"));
            // Proposer d'en installer une depuis Logiciels.
            if let Some(query) = suggestion(title) {
                let b = gtk::Button::with_label("Trouver une application");
                b.set_valign(gtk::Align::Center);
                b.add_css_class("pill");
                b.connect_clicked(move |_| {
                    let _ = std::process::Command::new("noos-software").env("NOOS_SOFTWARE_SEARCH", query).spawn();
                });
                r.add_suffix(&b);
            }
            common.add(&r);
            continue;
        }
        let row = app_combo(title, desc, icon, &apps, current.as_ref());
        let (app, mimes) = (app.clone(), *mimes);
        row.connect_selected_notify(move |r| {
            if let Some(info) = r.selected_item().and_downcast::<gio::AppInfo>() {
                set_default(&app, &info, mimes);
            }
        });
        common.add(&row);
    }
    content.append(&common);

    // ── Par type de fichier ──
    let by_type = group("Par type de fichier");
    by_type.set_description(Some("Recherchez un type (« jpeg », « .odt », « vidéo »…) pour choisir son application."));
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher un type de fichier"));
    search.set_margin_bottom(6);
    by_type.set_header_suffix(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 6);
    holder.append(&search);
    holder.append(&list);
    by_type.add(&holder);
    content.append(&by_type);

    // Types connus ayant au moins une application (description, type MIME).
    let types: Rc<Vec<(String, String)>> = Rc::new({
        let mut v: Vec<(String, String)> = gio::content_types_get_registered()
            .into_iter()
            .filter_map(|ct| {
                let mime = gio::content_type_get_mime_type(&ct)?.to_string();
                if mime.starts_with("x-scheme-handler/") || mime.contains("x-content/") {
                    return None;
                }
                Some((gio::content_type_get_description(&ct).to_string(), mime))
            })
            .collect();
        v.sort_by_key(|(d, _)| d.to_lowercase());
        v.dedup_by(|a, b| a.1 == b.1);
        v
    });
    let refill = {
        let (app, list, types) = (app.clone(), list.clone(), types.clone());
        move |query: &str| {
            while let Some(c) = list.first_child() {
                list.remove(&c);
            }
            let q = query.trim().trim_start_matches('.').to_lowercase();
            if q.len() < 2 {
                let r = card("", "Tapez au moins deux lettres", "");
                r.add_css_class("dim-label");
                list.append(&r);
                return;
            }
            let mut shown = 0;
            for (desc, mime) in types.iter() {
                if !(desc.to_lowercase().contains(&q) || mime.contains(&q)) {
                    continue;
                }
                let apps = apps_for(mime);
                if apps.is_empty() {
                    continue;
                }
                list.append(&type_row(&app, desc, mime, &apps));
                shown += 1;
                if shown >= 40 {
                    break;
                }
            }
            if shown == 0 {
                list.append(&card("", "Aucun type de fichier trouvé", ""));
            }
        }
    };
    refill("");
    search.connect_search_changed(move |s| refill(&s.text()));
    p
}

/// Ligne « type de fichier » : application actuelle, bouton pour en choisir une autre.
fn type_row(app: &Rc<App>, desc: &str, mime: &str, apps: &[gio::AppInfo]) -> adap::ActionRow {
    let row = adap::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(desc);
    let current = gio::AppInfo::default_for_type(mime, false);
    let current_name = current.as_ref().map(|a| a.display_name().to_string()).unwrap_or_else(|| "Aucune".into());
    row.set_subtitle(&format!("{mime} — {current_name}"));
    let icon = gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(mime));
    icon.set_pixel_size(20);
    icon.add_css_class("win11-row-icon");
    row.add_prefix(&icon);
    if let Some(c) = &current {
        let i = app_icon(c, 24);
        i.set_valign(gtk::Align::Center);
        row.add_suffix(&i);
    }
    let choose = gtk::MenuButton::new();
    choose.set_label("Changer");
    choose.set_valign(gtk::Align::Center);
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
        if current.as_ref().is_some_and(|c| c.id() == a.id()) {
            let check = gtk::Image::from_icon_name("object-select-symbolic");
            check.set_hexpand(true);
            check.set_halign(gtk::Align::End);
            h.append(&check);
        }
        b.set_child(Some(&h));
        let (app, info, mime, pop2, row2) = (app.clone(), a.clone(), mime.to_string(), pop.clone(), row.clone());
        b.connect_clicked(move |_| {
            pop2.popdown();
            set_default(&app, &info, &[&mime]);
            row2.set_subtitle(&format!("{mime} — {}", info.display_name()));
        });
        box_.append(&b);
    }
    pop.set_child(Some(&box_));
    noos_theme::fit_popover(&pop);
    choose.set_popover(Some(&pop));
    row.add_suffix(&choose);
    row
}

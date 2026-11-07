//! Volet de détails (à droite) et aperçu rapide (Espace) : image, texte, dossier, propriétés.

use adap::prelude::*;
use gtk::{gdk_pixbuf, gio, glib};

use crate::{file_of, human_size, is_dir, thumbs};

/// Texte affiché au plus (aperçu des fichiers texte).
const TEXT_PREVIEW_BYTES: usize = 64 * 1024;

pub fn kind_label(info: &gio::FileInfo) -> String {
    if is_dir(info) {
        return "Dossier de fichiers".into();
    }
    info.content_type().map(|t| gio::content_type_get_description(&t).to_string()).unwrap_or_else(|| "Fichier".into())
}

pub fn date_label(info: &gio::FileInfo) -> String {
    info.modification_date_time()
        .and_then(|d| d.to_local().ok())
        .and_then(|d| d.format("%-d %B %Y à %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

fn mime(info: &gio::FileInfo) -> String {
    info.content_type().and_then(|t| gio::content_type_get_mime_type(&t)).map(|m| m.to_string()).unwrap_or_default()
}

fn is_text(info: &gio::FileInfo) -> bool {
    info.content_type().is_some_and(|t| gio::content_type_is_a(&t, "text/plain")) && info.size() > 0
}

fn row(title: &str, value: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.set_subtitle(value);
    r.set_subtitle_selectable(true);
    r.add_css_class("property");
    r
}

/// Grande icône (ou miniature, chargée en arrière-plan) d'un fichier.
fn visual(file: &gio::File, info: &gio::FileInfo, size: i32, full_image: bool) -> gtk::Widget {
    let m = mime(info);
    if full_image && m.starts_with("image/") {
        // Aperçu rapide : l'image elle-même, ajustée à la fenêtre.
        let pic = gtk::Picture::for_file(file);
        pic.set_content_fit(gtk::ContentFit::Contain);
        pic.set_can_shrink(true);
        pic.set_vexpand(true);
        pic.add_css_class("preview-image");
        return pic.upcast();
    }
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    crate::set_icon(info, &image);
    if thumbs::supported(info).is_some() {
        let (file, info, img) = (file.clone(), info.clone(), image.clone());
        glib::spawn_future_local(async move {
            if let Some(path) = thumbs::thumbnail(&file, &info).await {
                img.set_from_file(Some(&path));
                img.add_css_class("thumb");
            }
        });
    }
    image.upcast()
}

/// Nombre d'éléments d'un dossier (en arrière-plan, limité à 100 000).
fn count_children(file: &gio::File, row: &adap::ActionRow) {
    let (file, row) = (file.clone(), row.clone());
    glib::spawn_future_local(async move {
        let n = gio::spawn_blocking(move || {
            file.enumerate_children("standard::name", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                .map(|e| e.take(100_000).count())
                .ok()
        })
        .await
        .ok()
        .flatten();
        row.set_subtitle(&match n {
            Some(0) => "Vide".into(),
            Some(1) => "1 élément".into(),
            Some(n) => format!("{n} éléments"),
            None => "Contenu illisible".into(),
        });
    });
}

/// Début d'un fichier texte, en police à chasse fixe.
fn text_view(file: &gio::File, max_lines: usize) -> gtk::Widget {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.add_css_class("text-preview");
    view.set_top_margin(8);
    view.set_bottom_margin(8);
    view.set_left_margin(10);
    view.set_right_margin(10);
    let buffer = view.buffer();
    let file = file.clone();
    glib::spawn_future_local(async move {
        let text = gio::spawn_blocking(move || {
            let path = file.path()?;
            let mut f = std::fs::File::open(path).ok()?;
            let mut bytes = vec![0u8; TEXT_PREVIEW_BYTES];
            let n = std::io::Read::read(&mut f, &mut bytes).ok()?;
            bytes.truncate(n);
            let text = String::from_utf8_lossy(&bytes).to_string();
            Some(text.lines().take(max_lines).collect::<Vec<_>>().join("\n"))
        })
        .await
        .ok()
        .flatten();
        buffer.set_text(&text.unwrap_or_else(|| "(aperçu indisponible)".into()));
    });
    gtk::ScrolledWindow::builder().child(&view).vexpand(true).min_content_height(160).build().upcast()
}

/// Contenu du volet de détails pour la sélection (ou le dossier courant si rien n'est
/// sélectionné).
pub fn details(selected: &[gio::FileInfo], current: &gio::File, folder_name: &str) -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.add_css_class("details-pane");
    page.set_margin_top(18);
    page.set_margin_bottom(18);
    page.set_margin_start(16);
    page.set_margin_end(16);

    match selected {
        [] => {
            let img = gtk::Image::from_icon_name("folder-open-symbolic");
            img.set_pixel_size(96);
            img.add_css_class("dim-label");
            page.append(&img);
            let t = gtk::Label::new(Some(folder_name));
            t.add_css_class("title-3");
            t.set_wrap(true);
            page.append(&t);
            let hint = gtk::Label::new(Some("Sélectionnez un fichier pour afficher son aperçu et ses propriétés."));
            hint.add_css_class("dim-label");
            hint.set_wrap(true);
            hint.set_justify(gtk::Justification::Center);
            page.append(&hint);
            let g = adap::PreferencesGroup::new();
            let r = row("Contenu", "…");
            count_children(current, &r);
            g.add(&r);
            if let Some(p) = current.path() {
                g.add(&row("Emplacement", &p.to_string_lossy()));
            }
            page.append(&g);
        }
        [info] => {
            let Some(file) = file_of(info.upcast_ref()) else { return page.upcast() };
            let v = visual(&file, info, 160, false);
            v.set_halign(gtk::Align::Center);
            v.add_css_class("details-visual");
            page.append(&v);
            let name = gtk::Label::new(Some(&info.display_name()));
            name.add_css_class("title-3");
            name.set_wrap(true);
            name.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            name.set_justify(gtk::Justification::Center);
            name.set_selectable(true);
            page.append(&name);
            let kind = gtk::Label::new(Some(&kind_label(info)));
            kind.add_css_class("dim-label");
            page.append(&kind);

            let g = adap::PreferencesGroup::new();
            if is_dir(info) {
                let r = row("Contenu", "…");
                count_children(&file, &r);
                g.add(&r);
            } else {
                g.add(&row("Taille", &format!("{} ({} octets)", human_size(info.size()), info.size())));
            }
            if let Some(p) = file.path().filter(|_| mime(info).starts_with("image/")) {
                if let Some((_, w, h)) = gdk_pixbuf::Pixbuf::file_info(&p) {
                    g.add(&row("Dimensions", &format!("{w} × {h} pixels")));
                }
            }
            let date = date_label(info);
            if !date.is_empty() {
                g.add(&row("Modifié le", &date));
            }
            if let Some(orig) = info.attribute_byte_string("trash::orig-path") {
                g.add(&row("Emplacement d'origine", orig.as_str()));
            } else if let Some(p) = file.parent().and_then(|p| p.path()) {
                g.add(&row("Emplacement", &p.to_string_lossy()));
            }
            page.append(&g);
            if is_text(info) {
                page.append(&text_view(&file, 60));
            }
        }
        many => {
            let img = gtk::Image::from_icon_name("edit-copy-symbolic");
            img.set_pixel_size(96);
            img.add_css_class("dim-label");
            page.append(&img);
            let t = gtk::Label::new(Some(&format!("{} éléments sélectionnés", many.len())));
            t.add_css_class("title-3");
            page.append(&t);
            let g = adap::PreferencesGroup::new();
            let dirs = many.iter().filter(|i| is_dir(i)).count();
            let files = many.len() - dirs;
            let size: i64 = many.iter().filter(|i| !is_dir(i)).map(|i| i.size()).sum();
            if files > 0 {
                g.add(&row("Fichiers", &format!("{files} — {}", human_size(size))));
            }
            if dirs > 0 {
                g.add(&row("Dossiers", &dirs.to_string()));
            }
            page.append(&g);
        }
    }
    gtk::ScrolledWindow::builder()
        .child(&page)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build()
        .upcast()
}

/// Aperçu rapide (Espace) : image en grand, texte, ou grande icône et propriétés.
pub fn quick_look(parent: &impl IsA<gtk::Widget>, info: &gio::FileInfo) {
    let Some(file) = file_of(info.upcast_ref()) else { return };
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    if mime(info).starts_with("image/") {
        content.append(&visual(&file, info, 256, true));
    } else if is_text(info) {
        content.append(&text_view(&file, 2000));
    } else {
        let v = visual(&file, info, 256, false);
        v.set_vexpand(true);
        v.set_valign(gtk::Align::Center);
        content.append(&v);
    }
    let sub = gtk::Label::new(Some(&format!(
        "{} · {}{}",
        kind_label(info),
        if is_dir(info) { String::new() } else { human_size(info.size()) + " · " },
        date_label(info)
    )));
    sub.add_css_class("dim-label");
    content.append(&sub);

    let open = gtk::Button::with_label("Ouvrir");
    open.add_css_class("suggested-action");
    let header = adap::HeaderBar::new();
    header.pack_end(&open);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&content));
    let dialog = adap::Dialog::builder()
        .title(info.display_name().as_str())
        .content_width(900)
        .content_height(640)
        .child(&tv)
        .build();
    {
        let (dialog, file) = (dialog.clone(), file.clone());
        open.connect_clicked(move |_| {
            dialog.close();
            let ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
            let _ = gio::AppInfo::launch_default_for_uri(&file.uri(), ctx.as_ref());
        });
    }
    // Espace referme l'aperçu, comme sur macOS.
    let keys = gtk::EventControllerKey::new();
    {
        let dialog = dialog.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::space {
                dialog.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    dialog.add_controller(keys);
    dialog.present(Some(parent));
}

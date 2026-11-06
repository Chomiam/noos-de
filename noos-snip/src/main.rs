//! noos-snip — Outil Capture d'écran de NOOS (équivalent de Win+Maj+S).
//!
//! `noos-snip [--mode rect|window|full|free] [--delay N]` : capture puis sélection.
//! `noos-snip --app` : fenêtre de l'outil (lancée depuis le menu Démarrer).
//! `noos-snip --edit image.png` : annoter une image existante.
//!
//! Après la sélection, la capture est copiée dans le presse-papiers, enregistrée dans
//! « Images/Captures d'écran » et ouverte dans l'éditeur (annotation, rognage).

mod editor;
mod overlay;
mod shot;

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::{gio, glib};
use overlay::Mode;

struct Args {
    mode: Mode,
    delay: u32,
    app: bool,
    edit: Option<std::path::PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args { mode: Mode::Rect, delay: 0, app: false, edit: None };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mode" => args.mode = it.next().and_then(|m| Mode::parse(&m)).unwrap_or(Mode::Rect),
            "--delay" => args.delay = it.next().and_then(|d| d.parse().ok()).unwrap_or(0),
            "--app" => args.app = true,
            "--edit" => args.edit = it.next().map(Into::into),
            "-h" | "--help" => {
                println!("noos-snip [--mode rect|window|full|free] [--delay SECONDES] [--app] [--edit IMAGE]");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    args
}

/// Notification « capture copiée », avec la miniature (centre de notifications NOOS).
fn notify(path: &std::path::Path) {
    let path = path.to_path_buf();
    glib::spawn_future_local(async move {
        let Ok(conn) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let mut hints: HashMap<String, glib::Variant> = HashMap::new();
        hints.insert("image-path".into(), path.to_string_lossy().to_string().to_variant());
        hints.insert("desktop-entry".into(), "org.noos.Snip".to_variant());
        hints.insert("transient".into(), true.to_variant());
        let body = format!("Copiée dans le Presse-papiers et enregistrée dans « {} ».",
            path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let args = ("Outil Capture d'écran", 0u32, "applets-screenshooter", "Capture d'écran", body, Vec::<String>::new(), hints, 4000i32).to_variant();
        let _ = conn
            .call_future(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                "Notify",
                Some(&args),
                None,
                gio::DBusCallFlags::NONE,
                2000,
            )
            .await;
    });
}

fn start_capture(app: &adap::Application, mode: Mode, delay: u32) {
    let hold = Rc::new(RefCell::new(Some(app.hold())));
    let app = app.clone();
    glib::timeout_add_local_once(Duration::from_millis(delay as u64 * 1000 + 50), move || {
        let shot = match shot::grab() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("noos-snip : {e:#}");
                hold.borrow_mut().take();
                return;
            }
        };
        let app2 = app.clone();
        overlay::Overlay::start(&app, shot, mode, move |result| {
            if let Some(shot) = result {
                let path = shot::auto_path();
                let saved = match shot.save_png(&path) {
                    Ok(()) => Some(path),
                    Err(e) => {
                        eprintln!("noos-snip : {e:#}");
                        None
                    }
                };
                let editor = editor::Editor::new(&app2, shot, saved.clone());
                // Presse-papiers tant que la fenêtre reste ouverte (le clic de sélection
                // fournit le « serial » exigé par Wayland).
                editor.copy(false);
                editor.window.present();
                if let Some(p) = &saved {
                    notify(p);
                }
            }
            // L'éditeur (s'il est ouvert) garde l'application en vie.
            hold.borrow_mut().take();
        });
    });
}

/// Fenêtre de démarrage (menu Démarrer) : choix du mode et du délai.
fn launcher(app: &adap::Application) {
    let window = adap::ApplicationWindow::builder()
        .application(app)
        .title("Outil Capture d'écran")
        .default_width(520)
        .default_height(380)
        .build();
    let header = adap::HeaderBar::new();
    let page = adap::StatusPage::builder()
        .icon_name("applets-screenshooter")
        .title("Outil Capture d'écran")
        .description("Raccourcis : Win + Maj + S ou Impr. écran")
        .build();
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 12);
    box_.set_halign(gtk::Align::Center);
    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let delay = gtk::DropDown::from_strings(&["Pas de délai", "Délai de 3 s", "Délai de 5 s", "Délai de 10 s"]);
    for (mode, icon, label) in [
        (Mode::Rect, "edit-select-all-symbolic", "Rectangle"),
        (Mode::Window, "focus-windows-symbolic", "Fenêtre"),
        (Mode::Full, "view-fullscreen-symbolic", "Plein écran"),
        (Mode::Free, "document-edit-symbolic", "Forme libre"),
    ] {
        let b = gtk::Button::new();
        b.set_child(Some(&adap::ButtonContent::builder().icon_name(icon).label(label).build()));
        if mode == Mode::Rect {
            b.add_css_class("suggested-action");
        }
        let (app, window, delay) = (app.clone(), window.clone(), delay.clone());
        b.connect_clicked(move |_| {
            let secs = [0, 3, 5, 10][delay.selected().min(3) as usize];
            let hold = app.hold();
            window.set_visible(false);
            let (app, window) = (app.clone(), window.clone());
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                start_capture(&app, mode, secs);
                window.close();
                drop(hold);
            });
        });
        modes.append(&b);
    }
    box_.append(&modes);
    delay.set_halign(gtk::Align::Center);
    box_.append(&delay);
    page.set_child(Some(&box_));
    let view = adap::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    window.set_content(Some(&view));
    window.present();
}

fn main() -> glib::ExitCode {
    let args = parse_args();
    let app = adap::Application::builder()
        .application_id("org.noos.Snip")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        editor::install_css();
        if let Some(path) = &args.edit {
            match shot::Shot::load(path) {
                Ok(shot) => editor::Editor::new(app, shot, Some(path.clone())).window.present(),
                Err(e) => eprintln!("noos-snip : {e:#}"),
            }
        } else if args.app {
            launcher(app);
        } else {
            start_capture(app, args.mode, args.delay);
        }
    });
    app.run_with_args::<&str>(&[])
}

//! noos-lock — écran de verrouillage de NOOS.
//!
//! Verrouille la session avec `ext-session-lock-v1` (gtk4-session-lock) : une fenêtre par
//! écran avec l'animation choisie (Paramètres › Personnalisation › Écran de verrouillage :
//! animation NOOS ou de la bibliothèque, sinon le fond d'écran), une grande horloge, et au premier
//! appui (touche ou clic) la carte de connexion : avatar, nom, mot de passe vérifié par PAM
//! (service `noos-lock`).
//!
//! Lancé par le compositeur (Super+L, inactivité, avant la mise en veille, IPC `Lock`).
//! `noos-lock --preview [animation]` : même écran dans une fenêtre, sans verrouiller (tests,
//! aperçu des Paramètres) ; le mot de passe n'y est pas vérifié.
//! `noos-lock --screensaver` : écran de veille (`[lock] mode = "screensaver"`) — animation et
//! horloge seulement ; la première touche, un clic ou un mouvement de souris le retire.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::NoosConfig;

const APP_ID: &str = "org.noos.Lock";

const CSS: &str = "
window.noos-lock { background: black; }
.lock-dim { background-color: black; }
.lock-time { font-size: 112px; font-weight: 300; color: white; text-shadow: 0 2px 18px rgba(0,0,0,0.45); }
.lock-date { font-size: 22px; font-weight: 500; color: rgba(255,255,255,0.92); text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.lock-hint { color: rgba(255,255,255,0.75); font-size: 14px; text-shadow: 0 1px 6px rgba(0,0,0,0.6); }
.lock-shade { background: linear-gradient(to bottom, rgba(0,0,0,0.25), rgba(0,0,0,0) 35%, rgba(0,0,0,0) 60%, rgba(0,0,0,0.45)); }
.lock-shade.auth { background-color: rgba(0,0,0,0.35); }
.lock-card { padding: 28px 34px; border-radius: 22px; background-color: rgba(20,21,29,0.62); border: 1px solid rgba(255,255,255,0.10); }
.lock-name { font-size: 22px; font-weight: 700; color: white; }
.lock-card entry, .lock-card passwordentry { min-height: 40px; min-width: 280px; border-radius: 10px; }
.lock-error { color: #ff8b97; font-weight: 600; }
.lock-card.shake { animation: lock-shake 380ms ease-in-out; }
@keyframes lock-shake { 0% { margin-left: 0; } 20% { margin-left: -18px; } 40% { margin-left: 16px; } 60% { margin-left: -10px; } 80% { margin-left: 6px; } 100% { margin-left: 0; } }
";

/// Vérifie le mot de passe de l'utilisateur courant (PAM, service `noos-lock`).
fn authenticate(password: String) -> Result<(), String> {
    let user = glib::user_name().to_string_lossy().into_owned();
    let conv = pam_client2::conv_mock::Conversation::with_credentials(user.clone(), password);
    let mut ctx = pam_client2::Context::new("noos-lock", Some(&user), conv).map_err(|e| e.to_string())?;
    ctx.authenticate(pam_client2::Flag::NONE).map_err(|_| "Mot de passe incorrect".to_string())?;
    // Compte expiré ou verrouillé : on déverrouille quand même (le mot de passe est bon).
    let _ = ctx.acct_mgmt(pam_client2::Flag::NONE);
    Ok(())
}

fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("fil d'authentification") }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Avatar de l'utilisateur : ~/.face, sinon AccountsService, sinon initiales.
fn avatar(name: &str) -> adap::Avatar {
    let a = adap::Avatar::new(96, Some(name), true);
    let user = glib::user_name().to_string_lossy().into_owned();
    for p in [glib::home_dir().join(".face"), std::path::PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))] {
        if let Ok(t) = gdk::Texture::from_filename(&p) {
            a.set_custom_image(Some(&t));
            break;
        }
    }
    a
}

struct Screen {
    window: gtk::Window,
    shade: gtk::Box,
    card_rev: gtk::Revealer,
    card: gtk::Box,
    hint: gtk::Label,
    entry: gtk::PasswordEntry,
    error: gtk::Label,
    spinner: gtk::Spinner,
    last_input: Cell<std::time::Instant>,
}

/// Ce qu'il faut faire quand le mot de passe est bon.
type OnUnlock = Rc<dyn Fn()>;

fn build_screen(cfg: &NoosConfig, animation_override: Option<&str>, preview: bool, screensaver: bool, on_unlock: OnUnlock) -> Rc<Screen> {
    let window = gtk::Window::new();
    window.add_css_class("noos-lock");
    window.set_title(Some("Écran de verrouillage"));
    let overlay = gtk::Overlay::new();

    // ── Fond : animation, sinon fond d'écran ──
    let bg = gtk::Stack::new();
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    if !cfg.appearance.wallpaper.is_empty() {
        picture.set_filename(Some(&cfg.appearance.wallpaper));
    }
    bg.add_named(&picture, Some("wallpaper"));
    let spec = animation_override.map(String::from).unwrap_or_else(|| cfg.lock.animation.clone());
    if let Some(src) = noos_shader::resolve(&spec) {
        let view = noos_shader::view::ShaderView::new();
        view.set_fps(cfg.lock.fps.max(1));
        view.set_render_scale(cfg.lock.render_scale);
        view.set_source(Some(src));
        let bg2 = bg.clone();
        view.connect_error(move |_| bg2.set_visible_child_name("wallpaper"));
        bg.add_named(view.widget(), Some("animation"));
        bg.set_visible_child_name("animation");
    }
    overlay.set_child(Some(&bg));
    // Assombrissement réglable de l'animation (Paramètres › Écran de verrouillage).
    let dim = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dim.add_css_class("lock-dim");
    dim.set_opacity(cfg.lock.animation_dim.clamp(0.0, 0.85) as f64);
    dim.set_can_target(false);
    overlay.add_overlay(&dim);
    let shade = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shade.add_css_class("lock-shade");
    shade.set_can_target(false);
    overlay.add_overlay(&shade);

    // ── Horloge ──
    let clock = gtk::Box::new(gtk::Orientation::Vertical, 0);
    clock.set_halign(gtk::Align::Center);
    clock.set_valign(gtk::Align::Start);
    clock.set_margin_top(90);
    clock.set_can_target(false);
    let time = gtk::Label::new(None);
    time.add_css_class("lock-time");
    time.add_css_class("numeric");
    let date = gtk::Label::new(None);
    date.add_css_class("lock-date");
    clock.append(&time);
    clock.append(&date);
    clock.set_visible(cfg.lock.show_clock);
    overlay.add_overlay(&clock);
    let tick = {
        let (time, date) = (time.clone(), date.clone());
        move || {
            if let Ok(now) = glib::DateTime::now_local() {
                time.set_text(&now.format("%H:%M").map(|s| s.to_string()).unwrap_or_default());
                date.set_text(&capitalize(&now.format("%A %-d %B").map(|s| s.to_string()).unwrap_or_default()));
            }
        }
    };
    tick();
    glib::timeout_add_seconds_local(1, move || {
        tick();
        glib::ControlFlow::Continue
    });

    // ── Carte de connexion ──
    let real = glib::real_name().to_string_lossy().into_owned();
    let name = if real.is_empty() || real == "Unknown" { glib::user_name().to_string_lossy().into_owned() } else { real };
    let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
    card.add_css_class("lock-card");
    card.set_halign(gtk::Align::Center);
    card.append(&avatar(&name));
    let n = gtk::Label::new(Some(&name));
    n.add_css_class("lock-name");
    card.append(&n);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_property("placeholder-text", "Mot de passe");
    entry.set_hexpand(true);
    let go = gtk::Button::from_icon_name("go-next-symbolic");
    go.add_css_class("suggested-action");
    go.add_css_class("circular");
    go.set_valign(gtk::Align::Center);
    go.set_tooltip_text(Some("Déverrouiller"));
    let spinner = gtk::Spinner::new();
    row.append(&entry);
    row.append(&go);
    row.append(&spinner);
    card.append(&row);
    let error = gtk::Label::new(None);
    error.add_css_class("lock-error");
    error.set_visible(false);
    card.append(&error);
    if preview {
        let p = gtk::Label::new(Some("Aperçu : le mot de passe n'est pas vérifié (Échap pour fermer)"));
        p.add_css_class("lock-hint");
        card.append(&p);
    }
    let card_rev = gtk::Revealer::new();
    card_rev.set_transition_type(gtk::RevealerTransitionType::Crossfade);
    card_rev.set_transition_duration(220);
    card_rev.set_child(Some(&card));
    card_rev.set_valign(gtk::Align::End);
    card_rev.set_halign(gtk::Align::Center);
    card_rev.set_margin_bottom(120);
    overlay.add_overlay(&card_rev);
    let hint = gtk::Label::new(Some("Appuyez sur une touche ou cliquez pour déverrouiller"));
    hint.add_css_class("lock-hint");
    hint.set_valign(gtk::Align::End);
    hint.set_margin_bottom(48);
    hint.set_can_target(false);
    overlay.add_overlay(&hint);
    window.set_child(Some(&overlay));

    let s = Rc::new(Screen {
        window,
        shade,
        card_rev,
        card,
        hint,
        entry,
        error,
        spinner,
        last_input: Cell::new(std::time::Instant::now()),
    });

    // Validation du mot de passe.
    let submit: Rc<dyn Fn()> = {
        let (s, on_unlock) = (s.clone(), on_unlock.clone());
        Rc::new(move || {
            let pw = s.entry.text().to_string();
            if pw.is_empty() {
                return;
            }
            if preview {
                on_unlock();
                return;
            }
            s.entry.set_sensitive(false);
            s.spinner.start();
            s.error.set_visible(false);
            let task = background(move || authenticate(pw));
            let (s, on_unlock) = (s.clone(), on_unlock.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                s.spinner.stop();
                s.entry.set_sensitive(true);
                match res {
                    Ok(()) => on_unlock(),
                    Err(e) => {
                        s.entry.set_text("");
                        s.error.set_text(&e);
                        s.error.set_visible(true);
                        s.card.remove_css_class("shake");
                        s.card.add_css_class("shake");
                        s.entry.grab_focus();
                    }
                }
            });
        })
    };
    {
        let submit = submit.clone();
        s.entry.connect_activate(move |_| submit());
    }
    go.connect_clicked(move |_| submit());

    if screensaver {
        // Écran de veille : ni carte ni invite ; toute activité le retire.
        s.hint.set_visible(false);
        let started = std::time::Instant::now();
        let grace = move || started.elapsed() > Duration::from_millis(1200);
        let keys = gtk::EventControllerKey::new();
        {
            let on_unlock = on_unlock.clone();
            keys.connect_key_pressed(move |_, _, _, _| {
                if grace() {
                    on_unlock();
                }
                glib::Propagation::Stop
            });
        }
        s.window.add_controller(keys);
        let click = gtk::GestureClick::new();
        {
            let on_unlock = on_unlock.clone();
            click.connect_pressed(move |_, _, _, _| on_unlock());
        }
        s.window.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let origin: Rc<Cell<Option<(f64, f64)>>> = Rc::new(Cell::new(None));
        motion.connect_motion(move |_, x, y| match origin.get() {
            None => origin.set(Some((x, y))),
            // Un vrai mouvement (pas une vibration du bureau) après le délai de grâce.
            Some((ox, oy)) if grace() && ((x - ox).powi(2) + (y - oy).powi(2)).sqrt() > 12.0 => on_unlock(),
            _ => {}
        });
        s.window.add_controller(motion);
        return s;
    }

    // Première touche / clic : la carte apparaît (la touche tapée est gardée).
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let s = s.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            s.last_input.set(std::time::Instant::now());
            if key == gdk::Key::Escape {
                if preview {
                    s.window.close();
                } else {
                    show_card(&s, false);
                }
                return glib::Propagation::Stop;
            }
            if !s.card_rev.reveals_child() {
                show_card(&s, true);
                if let Some(c) = key.to_unicode().filter(|c| !c.is_control()) {
                    s.entry.set_text(&c.to_string());
                    s.entry.set_position(-1);
                    return glib::Propagation::Stop;
                }
            }
            glib::Propagation::Proceed
        });
    }
    s.window.add_controller(keys);
    let click = gtk::GestureClick::new();
    {
        let s = s.clone();
        click.connect_pressed(move |_, _, _, _| {
            s.last_input.set(std::time::Instant::now());
            if !s.card_rev.reveals_child() {
                show_card(&s, true);
            }
        });
    }
    s.window.add_controller(click);
    // Retour à l'horloge après 30 s sans saisie.
    {
        let s2 = Rc::downgrade(&s);
        glib::timeout_add_seconds_local(5, move || {
            let Some(s) = s2.upgrade() else { return glib::ControlFlow::Break };
            if s.card_rev.reveals_child() && s.last_input.get().elapsed() > Duration::from_secs(30) && s.entry.is_sensitive() {
                show_card(&s, false);
            }
            glib::ControlFlow::Continue
        });
    }
    if std::env::var_os("NOOS_LOCK_DEMO_CARD").is_some() {
        let s = s.clone();
        glib::timeout_add_local_once(Duration::from_millis(800), move || show_card(&s, true));
    }
    s
}

fn show_card(s: &Screen, show: bool) {
    s.card_rev.set_reveal_child(show);
    s.hint.set_visible(!show);
    if show {
        s.shade.add_css_class("auth");
        s.entry.grab_focus();
    } else {
        s.shade.remove_css_class("auth");
        s.entry.set_text("");
        s.error.set_visible(false);
    }
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let preview = args.iter().any(|a| a == "--preview");
    let screensaver = args.iter().any(|a| a == "--screensaver");
    let animation = args.iter().skip_while(|a| *a != "--preview").nth(1).cloned();
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
        adap::StyleManager::default().set_color_scheme(adap::ColorScheme::ForceDark);
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5);
        }
    });
    let screens: Rc<RefCell<Vec<Rc<Screen>>>> = Rc::new(RefCell::new(Vec::new()));
    app.connect_activate(move |app| {
        let cfg = NoosConfig::load();
        if preview {
            let a = app.clone();
            let s = build_screen(&cfg, animation.as_deref(), true, screensaver, Rc::new(move || a.quit()));
            s.window.set_application(Some(app));
            s.window.set_default_size(1280, 800);
            s.window.present();
            screens.borrow_mut().push(s);
            return;
        }
        if !gtk4_session_lock::is_supported() {
            eprintln!("noos-lock : le compositeur ne gère pas ext-session-lock-v1");
            app.quit();
            return;
        }
        let guard = app.hold();
        let instance = gtk4_session_lock::Instance::new();
        let unlock: OnUnlock = {
            let instance = instance.clone();
            Rc::new(move || instance.unlock())
        };
        {
            let (screens, cfg, unlock) = (screens.clone(), cfg.clone(), unlock.clone());
            instance.connect_monitor(move |inst, monitor| {
                let s = build_screen(&cfg, None, false, screensaver, unlock.clone());
                inst.assign_window_to_monitor(&s.window, monitor);
                s.window.present();
                screens.borrow_mut().push(s);
            });
        }
        {
            let app = app.clone();
            let guard = RefCell::new(Some(guard));
            instance.connect_unlocked(move |_| {
                guard.borrow_mut().take();
                app.quit();
            });
        }
        {
            let app = app.clone();
            instance.connect_failed(move |_| {
                eprintln!("noos-lock : verrouillage refusé (un autre écran de verrouillage est-il actif ?)");
                app.quit();
            });
        }
        if !instance.lock() {
            eprintln!("noos-lock : verrouillage impossible");
        }
        std::mem::forget(instance);
    });
    app.run_with_args::<&str>(&[])
}

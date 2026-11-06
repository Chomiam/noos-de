//! noos-login : écran de connexion de NOOS, façon Windows 11.
//!
//! Greeter pour greetd, lancé en plein écran par `noos-comp --kiosk noos-login` (voir
//! `noos-login/noos-greeter`). Écran de verrouillage (grande horloge) puis formulaire de
//! connexion sur le fond flouté : avatar, mot de passe, **sessions affichées en cartes**
//! (pas cachées derrière une roue dentée), comptes en bas à gauche, accessibilité et
//! alimentation en bas à droite.
//!
//! Hors de greetd (`GREETD_SOCK` absent), mode aperçu : le mot de passe `noos` « réussit ».
//! Thème : `/etc/noos/login.toml` (Paramètres → Personnalisation → Écran de connexion).

mod greetd;
mod osk;
mod system;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use adap::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use noos_core::{config::Appearance, login::LoginConfig};

use greetd::{AuthKind, Greetd, Request, Response};
use system::{Session, State, User};

const APP_ID: &str = "org.noos.Login";
/// Retour à l'écran de verrouillage après une minute sans activité.
const IDLE_LOCK: Duration = Duration::from_secs(60);

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(|app| {
        let ui = Ui::new(app);
        ui.window.present();
        // Les rappels ne gardent que des références faibles : l'interface vit aussi
        // longtemps que l'application.
        app.connect_shutdown(move |_| {
            let _ = &ui;
        });
    });
    app.run_with_args::<&str>(&[])
}

// ───────────────────────────── Authentification ─────────────────────────────

enum Event {
    /// Question PAM supplémentaire (nouveau mot de passe, code…).
    Prompt(AuthKind, String),
    Info(String),
    Failed(String),
    Done,
}

/// Dialogue greetd dans un fil séparé (PAM peut bloquer plusieurs secondes).
fn authenticate(
    username: String,
    password: String,
    session: Session,
    events: async_channel::Sender<Event>,
    answers: async_channel::Receiver<Option<String>>,
) {
    std::thread::spawn(move || {
        let send = |e| {
            let _ = events.send_blocking(e);
        };
        let mut g = match Greetd::connect() {
            None => {
                // Aperçu hors greetd.
                std::thread::sleep(Duration::from_millis(700));
                if password == "noos" {
                    let (cmd, env) = session.command();
                    println!("[aperçu] session : {cmd:?} {env:?}");
                    send(Event::Done);
                } else {
                    send(Event::Failed("Mot de passe incorrect (aperçu : essayez « noos »).".into()));
                }
                return;
            }
            Some(Err(e)) => return send(Event::Failed(format!("{e:#}"))),
            Some(Ok(g)) => g,
        };
        let mut first_secret = Some(password);
        let mut resp = g.call(&Request::CreateSession { username });
        loop {
            match resp {
                Err(e) => return send(Event::Failed(format!("{e:#}"))),
                Ok(Response::Success) => {
                    let (cmd, env) = session.command();
                    match g.call(&Request::StartSession { cmd, env }) {
                        Ok(Response::Success) => send(Event::Done),
                        Ok(Response::Error { description, .. }) => {
                            let _ = g.call(&Request::CancelSession);
                            send(Event::Failed(format!("Impossible de démarrer la session : {description}")));
                        }
                        Ok(_) => send(Event::Failed("Réponse inattendue de greetd.".into())),
                        Err(e) => send(Event::Failed(format!("{e:#}"))),
                    }
                    return;
                }
                Ok(Response::Error { error_type, description }) => {
                    let _ = g.call(&Request::CancelSession);
                    let msg = if error_type == "auth_error" {
                        "Mot de passe incorrect. Réessayez.".to_string()
                    } else {
                        description
                    };
                    return send(Event::Failed(msg));
                }
                Ok(Response::AuthMessage { auth_message_type, auth_message }) => match auth_message_type {
                    AuthKind::Secret | AuthKind::Visible => {
                        let answer = match (auth_message_type, first_secret.take()) {
                            (AuthKind::Secret, Some(p)) => Some(p),
                            _ => {
                                send(Event::Prompt(auth_message_type, auth_message));
                                answers.recv_blocking().ok().flatten()
                            }
                        };
                        if answer.is_none() {
                            let _ = g.call(&Request::CancelSession);
                            return send(Event::Failed(String::new()));
                        }
                        resp = g.call(&Request::PostAuthMessageResponse { response: answer });
                    }
                    AuthKind::Info | AuthKind::Error => {
                        send(Event::Info(auth_message));
                        resp = g.call(&Request::PostAuthMessageResponse { response: None });
                    }
                },
            }
        }
    });
}

// ───────────────────────────── Interface ─────────────────────────────

struct Ui {
    app: adap::Application,
    window: adap::ApplicationWindow,
    cfg: LoginConfig,
    state: RefCell<State>,
    users: Vec<User>,
    sessions: Vec<Session>,
    stack: gtk::Stack,
    background: gtk::Stack,
    avatar: adap::Avatar,
    name: gtk::Label,
    username: gtk::Entry,
    password: gtk::PasswordEntry,
    submit: gtk::Button,
    spinner: gtk::Spinner,
    message: gtk::Label,
    session_cards: gtk::FlowBox,
    user_list: gtk::ListBox,
    user: RefCell<Option<User>>,
    session: RefCell<Option<Session>>,
    busy: Cell<bool>,
    /// Réponse attendue à une question PAM supplémentaire.
    answer: RefCell<Option<async_channel::Sender<Option<String>>>>,
    last_activity: Cell<Instant>,
    osk: Rc<osk::Osk>,
    orca: RefCell<Option<std::process::Child>>,
    base_dpi: i32,
}

impl Ui {
    fn new(app: &adap::Application) -> Rc<Self> {
        let cfg = LoginConfig::load();
        let state = State::load();
        let users = system::users();
        let sessions = system::sessions();
        apply_theme(&cfg);

        let window = adap::ApplicationWindow::builder().application(app).title("Connexion").build();
        window.add_css_class("noos-login");
        window.fullscreen();

        // Fond : image nette (verrouillage) et floutée (connexion), en fondu.
        let background = gtk::Stack::new();
        background.set_transition_type(gtk::StackTransitionType::Crossfade);
        background.set_transition_duration(400);
        let sharp = background_picture(&cfg, false);
        let blurred = background_picture(&cfg, cfg.blur);
        background.add_named(&sharp, Some("lock"));
        background.add_named(&blurred, Some("login"));

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideUp);
        stack.set_transition_duration(350);

        let avatar = adap::Avatar::new(168, None, true);
        avatar.add_css_class("login-avatar");
        let name = gtk::Label::new(None);
        name.add_css_class("login-name");
        let username = gtk::Entry::new();
        username.set_placeholder_text(Some("Nom d'utilisateur"));
        username.add_css_class("login-entry");
        let password = gtk::PasswordEntry::new();
        password.set_show_peek_icon(true);
        password.set_placeholder_text(Some("Mot de passe"));
        password.set_hexpand(true);
        password.add_css_class("login-entry");
        let submit = gtk::Button::from_icon_name("go-next-symbolic");
        submit.set_tooltip_text(Some("Se connecter"));
        submit.add_css_class("suggested-action");
        submit.add_css_class("login-submit");
        let spinner = gtk::Spinner::new();
        let message = gtk::Label::new(None);
        message.add_css_class("login-message");
        message.set_wrap(true);
        message.set_justify(gtk::Justification::Center);
        message.set_max_width_chars(48);
        let session_cards = gtk::FlowBox::new();
        session_cards.set_selection_mode(gtk::SelectionMode::None);
        session_cards.set_homogeneous(true);
        session_cards.set_max_children_per_line(4);
        session_cards.set_min_children_per_line(1);
        session_cards.set_column_spacing(10);
        session_cards.set_row_spacing(10);
        session_cards.set_halign(gtk::Align::Center);
        let user_list = gtk::ListBox::new();
        user_list.add_css_class("login-users");
        user_list.set_selection_mode(gtk::SelectionMode::Single);

        let osk = osk::Osk::new(&system::keyboard_layout());
        let base_dpi = gtk::Settings::default().map(|s| s.gtk_xft_dpi()).unwrap_or(-1);

        let ui = Rc::new(Self {
            app: app.clone(),
            window,
            cfg,
            state: RefCell::new(state),
            users,
            sessions,
            stack,
            background,
            avatar,
            name,
            username,
            password,
            submit,
            spinner,
            message,
            session_cards,
            user_list,
            user: RefCell::new(None),
            session: RefCell::new(None),
            busy: Cell::new(false),
            answer: RefCell::new(None),
            last_activity: Cell::new(Instant::now()),
            osk,
            orca: RefCell::new(None),
            base_dpi,
        });
        ui.build();
        ui
    }

    fn build(self: &Rc<Self>) {
        let root = gtk::Overlay::new();
        root.set_child(Some(&self.background));
        let dim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dim.add_css_class("login-dim");
        dim.set_can_target(false);
        let provider = gtk::CssProvider::new();
        provider.load_from_string(&format!(".login-dim {{ background-color: rgba(0,0,0,{:.2}); }}", self.cfg.dim.clamp(0.0, 0.8)));
        gtk::style_context_add_provider_for_display(&WidgetExt::display(&self.window), &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
        root.add_overlay(&dim);
        root.add_overlay(&self.stack);

        self.stack.add_named(&self.lock_page(), Some("lock"));
        self.stack.add_named(&self.login_page(), Some("login"));
        self.window.set_content(Some(&root));

        // Activité : clavier et clics (retour au verrouillage après une minute d'inactivité).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(ui) = weak.upgrade() else { return glib::Propagation::Proceed };
            ui.last_activity.set(Instant::now());
            if ui.stack.visible_child_name().as_deref() == Some("lock") {
                ui.show_login();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::Escape && !ui.busy.get() && ui.cfg.lock_screen {
                ui.show_lock();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.last_activity.set(Instant::now());
            }
        });
        self.window.add_controller(click);
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(5, move || {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            if ui.cfg.lock_screen
                && !ui.busy.get()
                && ui.stack.visible_child_name().as_deref() == Some("login")
                && ui.last_activity.get().elapsed() > IDLE_LOCK
            {
                ui.show_lock();
            }
            glib::ControlFlow::Continue
        });

        // Réglages d'accessibilité mémorisés.
        let st = self.state.borrow().clone();
        self.set_large_text(st.large_text);
        self.set_high_contrast(st.high_contrast);
        self.osk.widget.set_reveal_child(st.osk);

        let initial = {
            let st = self.state.borrow();
            self.users.iter().find(|u| u.name == st.last_user).or(self.users.first()).cloned()
        };
        self.select_user(initial);
        // NOOS_LOGIN_PAGE=login : formulaire directement (tests, captures).
        let direct = std::env::var("NOOS_LOGIN_PAGE").is_ok_and(|p| p == "login");
        if self.cfg.lock_screen && !direct {
            self.stack.set_visible_child_name("lock");
            self.background.set_visible_child_name("lock");
        } else {
            self.show_login();
        }
    }

    // ── Écran de verrouillage ──

    fn lock_page(self: &Rc<Self>) -> gtk::Widget {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("lock-page");
        let time = gtk::Label::new(None);
        time.add_css_class("lock-time");
        let date = gtk::Label::new(None);
        date.add_css_class("lock-date");
        let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top.set_margin_top(110);
        top.append(&time);
        top.append(&date);
        page.append(&top);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        page.append(&spacer);
        let hint = gtk::Label::new(Some("Appuyez sur une touche ou cliquez pour vous connecter"));
        hint.add_css_class("lock-hint");
        hint.set_margin_bottom(48);
        page.append(&hint);

        let h24 = self.cfg.clock_24h;
        let update = move || {
            let now = glib::DateTime::now_local().ok();
            if let Some(now) = now {
                time.set_label(&clock(&now, h24));
                date.set_label(&french_date(&now));
            }
        };
        update();
        glib::timeout_add_seconds_local(1, move || {
            update();
            glib::ControlFlow::Continue
        });

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_released(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.show_login();
            }
        });
        page.add_controller(click);
        page.upcast()
    }

    fn show_lock(&self) {
        self.stack.set_transition_type(gtk::StackTransitionType::SlideDown);
        self.stack.set_visible_child_name("lock");
        self.background.set_visible_child_name("lock");
        self.password.set_text("");
        self.message.set_label("");
    }

    fn show_login(&self) {
        self.stack.set_transition_type(gtk::StackTransitionType::SlideUp);
        self.stack.set_visible_child_name("login");
        self.background.set_visible_child_name("login");
        self.last_activity.set(Instant::now());
        self.focus_input();
    }

    fn focus_input(&self) {
        if WidgetExt::is_visible(&self.username) && EditableExt::text(&self.username).is_empty() {
            self.username.grab_focus();
            self.osk.set_target(&self.username);
        } else {
            self.password.grab_focus();
            self.osk.set_target(&self.password);
        }
    }

    // ── Formulaire de connexion ──

    fn login_page(self: &Rc<Self>) -> gtk::Widget {
        let overlay = gtk::Overlay::new();

        let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
        card.add_css_class("login-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_width_request(420);
        card.append(&self.avatar);
        self.avatar.set_halign(gtk::Align::Center);
        card.append(&self.name);
        card.append(&self.username);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("linked");
        row.append(&self.password);
        row.append(&self.submit);
        card.append(&row);
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        status.set_halign(gtk::Align::Center);
        status.append(&self.spinner);
        status.append(&self.message);
        card.append(&status);

        // Sessions : toujours visibles, en cartes.
        let title = gtk::Label::new(Some("Session"));
        title.add_css_class("login-section");
        title.set_margin_top(10);
        card.append(&title);
        card.append(&self.session_cards);
        overlay.set_child(Some(&card));

        // Bas de l'écran : comptes (gauche), clavier visuel (centre), accessibilité et
        // alimentation (droite).
        let bottom = gtk::CenterBox::new();
        bottom.set_valign(gtk::Align::End);
        bottom.set_margin_start(24);
        bottom.set_margin_end(24);
        bottom.set_margin_bottom(24);
        if self.cfg.show_user_list && !self.users.is_empty() {
            bottom.set_start_widget(Some(&self.users_panel()));
        }
        bottom.set_center_widget(Some(&self.osk.widget));
        bottom.set_end_widget(Some(&self.tray()));
        overlay.add_overlay(&bottom);

        let host = system::hostname();
        if !host.is_empty() {
            let h = gtk::Label::new(Some(&host));
            h.add_css_class("login-host");
            h.set_halign(gtk::Align::End);
            h.set_valign(gtk::Align::Start);
            h.set_margin_top(18);
            h.set_margin_end(24);
            overlay.add_overlay(&h);
        }

        let weak = Rc::downgrade(self);
        self.password.connect_activate(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.submit();
            }
        });
        let weak = Rc::downgrade(self);
        self.submit.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.submit();
            }
        });
        let weak = Rc::downgrade(self);
        self.username.connect_activate(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.password.grab_focus();
                ui.osk.set_target(&ui.password);
            }
        });
        let weak = Rc::downgrade(self);
        self.osk.connect_enter(move || {
            if let Some(ui) = weak.upgrade() {
                ui.submit();
            }
        });
        for e in [self.username.clone().upcast::<gtk::Widget>(), self.password.clone().upcast()] {
            let focus = gtk::EventControllerFocus::new();
            let weak = Rc::downgrade(self);
            let target = e.clone();
            focus.connect_enter(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(ed) = target.dynamic_cast_ref::<gtk::Editable>() {
                        ui.osk.set_target(ed);
                    }
                }
            });
            e.add_controller(focus);
        }
        overlay.upcast()
    }

    fn users_panel(self: &Rc<Self>) -> gtk::Widget {
        for u in &self.users {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("login-user-row");
            let a = adap::Avatar::new(40, Some(u.display_name()), true);
            if let Some(tex) = u.avatar.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok()) {
                a.set_custom_image(Some(&tex));
            }
            row.append(&a);
            let l = gtk::Label::new(Some(u.display_name()));
            l.set_xalign(0.0);
            row.append(&l);
            self.user_list.append(&row);
        }
        let other = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        other.add_css_class("login-user-row");
        other.append(&adap::Avatar::new(40, None, false));
        other.append(&gtk::Label::new(Some("Autre utilisateur")));
        self.user_list.append(&other);
        let weak = Rc::downgrade(self);
        self.user_list.connect_row_activated(move |_, row| {
            let Some(ui) = weak.upgrade() else { return };
            let user = ui.users.get(row.index() as usize).cloned();
            ui.select_user(user);
            ui.focus_input();
        });
        self.user_list.set_valign(gtk::Align::End);
        self.user_list.clone().upcast()
    }

    fn select_user(self: &Rc<Self>, user: Option<User>) {
        let others = !self.cfg.show_user_list || self.users.is_empty();
        match &user {
            Some(u) => {
                self.avatar.set_text(Some(u.display_name()));
                self.avatar.set_custom_image(u.avatar.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok()).as_ref());
                self.name.set_label(u.display_name());
                self.name.set_visible(true);
                self.username.set_visible(false);
                self.username.set_text(&u.name);
                if let Some(i) = self.users.iter().position(|x| x == u) {
                    self.user_list.select_row(self.user_list.row_at_index(i as i32).as_ref());
                }
            }
            None => {
                self.avatar.set_text(None);
                self.avatar.set_custom_image(None::<&gdk::Paintable>);
                self.name.set_visible(!others);
                self.name.set_label("Autre utilisateur");
                self.username.set_visible(true);
                self.username.set_text("");
                if !others {
                    self.user_list.select_row(self.user_list.row_at_index(self.users.len() as i32).as_ref());
                }
            }
        }
        self.password.set_text("");
        self.message.set_label("");
        // Session : dernière de ce compte, sinon celle par défaut, sinon la première.
        let remembered = user.as_ref().and_then(|u| self.state.borrow().sessions.get(&u.name).cloned());
        let session = remembered
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .or_else(|| self.sessions.iter().find(|s| s.id == self.cfg.default_session))
            .or(self.sessions.first())
            .cloned();
        *self.user.borrow_mut() = user;
        *self.session.borrow_mut() = session;
        self.rebuild_sessions();
    }

    fn rebuild_sessions(self: &Rc<Self>) {
        self.session_cards.remove_all();
        if self.sessions.is_empty() {
            let l = gtk::Label::new(Some("Aucune session installée"));
            l.add_css_class("login-message");
            self.session_cards.insert(&l, -1);
            return;
        }
        // Une seule ligne (centrée) jusqu'à 4 sessions.
        self.session_cards.set_max_children_per_line(self.sessions.len().clamp(1, 4) as u32);
        let current = self.session.borrow().as_ref().map(|s| s.id.clone());
        for s in &self.sessions {
            let b = gtk::Button::new();
            b.add_css_class("session-card");
            if current.as_deref() == Some(&s.id) {
                b.add_css_class("selected");
            }
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let icon = gtk::Image::from_icon_name(s.icon());
            icon.set_pixel_size(28);
            v.append(&icon);
            let n = gtk::Label::new(Some(&s.name));
            n.add_css_class("session-name");
            n.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&n);
            let kind = gtk::Label::new(Some(if s.wayland { "Wayland" } else { "X11" }));
            kind.add_css_class("session-kind");
            v.append(&kind);
            b.set_child(Some(&v));
            if !s.comment.is_empty() {
                b.set_tooltip_text(Some(&s.comment));
            }
            b.set_focus_on_click(false);
            let (weak, s) = (Rc::downgrade(self), s.clone());
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    *ui.session.borrow_mut() = Some(s.clone());
                    ui.rebuild_sessions();
                    ui.focus_input();
                }
            });
            self.session_cards.insert(&b, -1);
        }
    }

    // ── Accessibilité et alimentation ──

    fn tray(self: &Rc<Self>) -> gtk::Widget {
        let tray = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tray.set_valign(gtk::Align::End);
        let layout = gtk::Label::new(Some(&system::keyboard_layout().to_uppercase()));
        layout.add_css_class("login-layout");
        layout.set_tooltip_text(Some("Disposition du clavier"));
        tray.append(&layout);

        // Accessibilité.
        let a11y = gtk::Box::new(gtk::Orientation::Vertical, 4);
        a11y.set_margin_top(8);
        a11y.set_margin_bottom(8);
        a11y.set_margin_start(8);
        a11y.set_margin_end(8);
        let title = gtk::Label::new(Some("Accessibilité"));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_margin_bottom(6);
        a11y.append(&title);
        let st = self.state.borrow().clone();
        let weak = Rc::downgrade(self);
        a11y.append(&toggle("Grand texte", "format-text-larger-symbolic", st.large_text, move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_large_text(on);
                ui.state.borrow_mut().large_text = on;
                ui.state.borrow().save();
            }
        }));
        let weak = Rc::downgrade(self);
        a11y.append(&toggle("Contraste élevé", "display-brightness-symbolic", st.high_contrast, move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.set_high_contrast(on);
                ui.state.borrow_mut().high_contrast = on;
                ui.state.borrow().save();
            }
        }));
        let weak = Rc::downgrade(self);
        a11y.append(&toggle("Clavier visuel", "input-keyboard-symbolic", st.osk, move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.osk.widget.set_reveal_child(on);
                ui.state.borrow_mut().osk = on;
                ui.state.borrow().save();
                ui.focus_input();
            }
        }));
        if which("orca") {
            let weak = Rc::downgrade(self);
            a11y.append(&toggle("Lecteur d'écran (Orca)", "audio-speakers-symbolic", false, move |on| {
                if let Some(ui) = weak.upgrade() {
                    ui.set_screen_reader(on);
                }
            }));
        }
        let pop = gtk::Popover::new();
        pop.set_child(Some(&a11y));
        let btn = gtk::MenuButton::new();
        btn.set_icon_name("preferences-desktop-accessibility-symbolic");
        btn.set_tooltip_text(Some("Accessibilité"));
        btn.add_css_class("login-tray-button");
        btn.set_popover(Some(&pop));
        btn.set_direction(gtk::ArrowType::Up);
        tray.append(&btn);

        // Alimentation.
        let power = gtk::Box::new(gtk::Orientation::Vertical, 2);
        power.set_margin_top(6);
        power.set_margin_bottom(6);
        for (label, icon, action) in [
            ("Mettre en veille", "weather-clear-night-symbolic", "suspend"),
            ("Redémarrer", "system-reboot-symbolic", "reboot"),
            ("Arrêter", "system-shutdown-symbolic", "poweroff"),
        ] {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            r.append(&gtk::Image::from_icon_name(icon));
            r.append(&gtk::Label::new(Some(label)));
            b.set_child(Some(&r));
            b.connect_clicked(move |_| system::power(action));
            power.append(&b);
        }
        let pop = gtk::Popover::new();
        pop.set_child(Some(&power));
        let btn = gtk::MenuButton::new();
        btn.set_icon_name("system-shutdown-symbolic");
        btn.set_tooltip_text(Some("Alimentation"));
        btn.add_css_class("login-tray-button");
        btn.set_popover(Some(&pop));
        btn.set_direction(gtk::ArrowType::Up);
        tray.append(&btn);
        tray.upcast()
    }

    fn set_large_text(&self, on: bool) {
        if let Some(s) = gtk::Settings::default() {
            let base = if self.base_dpi > 0 { self.base_dpi } else { 96 * 1024 };
            s.set_gtk_xft_dpi(if on { (base as f64 * 1.35) as i32 } else { base });
        }
    }

    fn set_high_contrast(&self, on: bool) {
        if on {
            self.window.add_css_class("hc");
        } else {
            self.window.remove_css_class("hc");
        }
    }

    fn set_screen_reader(&self, on: bool) {
        let mut orca = self.orca.borrow_mut();
        if let Some(mut child) = orca.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if on {
            *orca = std::process::Command::new("orca").arg("--replace").spawn().ok();
        }
    }

    // ── Connexion ──

    fn submit(self: &Rc<Self>) {
        self.last_activity.set(Instant::now());
        // Réponse à une question PAM en cours.
        if let Some(tx) = self.answer.borrow_mut().take() {
            let _ = tx.send_blocking(Some(self.password.text().to_string()));
            self.password.set_text("");
            self.password.set_placeholder_text(Some("Mot de passe"));
            self.set_busy(true);
            return;
        }
        if self.busy.get() {
            return;
        }
        let username = self.username.text().trim().to_string();
        if username.is_empty() {
            self.message.set_label("Saisissez votre nom d'utilisateur.");
            self.username.grab_focus();
            return;
        }
        let Some(session) = self.session.borrow().clone() else {
            self.message.set_label("Aucune session disponible.");
            return;
        };
        self.set_busy(true);
        self.message.set_label("");
        let (ev_tx, ev_rx) = async_channel::unbounded();
        let (ans_tx, ans_rx) = async_channel::unbounded();
        authenticate(username.clone(), self.password.text().to_string(), session.clone(), ev_tx, ans_rx);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while let Ok(ev) = ev_rx.recv().await {
                let Some(ui) = weak.upgrade() else { return };
                match ev {
                    Event::Prompt(kind, text) => {
                        ui.set_busy(false);
                        ui.message.set_label(text.trim_end_matches([':', ' ']));
                        ui.password.set_text("");
                        ui.password.set_placeholder_text(Some(if kind == AuthKind::Secret { "Réponse (masquée)" } else { "Réponse" }));
                        ui.password.grab_focus();
                        *ui.answer.borrow_mut() = Some(ans_tx.clone());
                    }
                    Event::Info(text) => ui.message.set_label(&text),
                    Event::Failed(text) => {
                        ui.set_busy(false);
                        ui.answer.borrow_mut().take();
                        ui.password.set_placeholder_text(Some("Mot de passe"));
                        ui.password.set_text("");
                        ui.message.set_label(&text);
                        ui.message.add_css_class("error");
                        ui.password.grab_focus();
                        shake(&ui.password);
                        return;
                    }
                    Event::Done => {
                        {
                            let mut st = ui.state.borrow_mut();
                            st.last_user = username.clone();
                            st.sessions.insert(username.clone(), session.id.clone());
                            st.save();
                        }
                        ui.set_screen_reader(false);
                        ui.message.remove_css_class("error");
                        ui.message.set_label("Ouverture de la session…");
                        // greetd lance la session quand le greeter se termine.
                        let app = ui.app.clone();
                        glib::timeout_add_local_once(Duration::from_millis(150), move || app.quit());
                        return;
                    }
                }
            }
        });
    }

    fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
        self.spinner.set_spinning(busy);
        self.spinner.set_visible(busy);
        self.password.set_sensitive(!busy);
        self.submit.set_sensitive(!busy);
        self.username.set_sensitive(!busy);
        if busy {
            self.message.remove_css_class("error");
        }
    }
}

// ───────────────────────────── Utilitaires ─────────────────────────────

fn toggle(label: &str, icon: &str, active: bool, f: impl Fn(bool) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    row.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::new(Some(label));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    row.append(&l);
    let sw = gtk::Switch::new();
    sw.set_active(active);
    sw.set_valign(gtk::Align::Center);
    sw.connect_active_notify(move |s| f(s.is_active()));
    row.append(&sw);
    row
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Petite secousse horizontale du champ (mot de passe refusé).
fn shake(w: &impl IsA<gtk::Widget>) {
    let w = w.clone().upcast::<gtk::Widget>();
    let start = Instant::now();
    w.add_tick_callback(move |w, _| {
        let t = start.elapsed().as_secs_f64();
        if t > 0.4 {
            w.set_margin_start(0);
            w.set_margin_end(0);
            return glib::ControlFlow::Break;
        }
        let dx = ((t * 40.0).sin() * 10.0 * (1.0 - t / 0.4)) as i32;
        w.set_margin_start(dx.max(0));
        w.set_margin_end((-dx).max(0));
        glib::ControlFlow::Continue
    });
}

fn clock(now: &glib::DateTime, h24: bool) -> String {
    if h24 {
        format!("{:02}:{:02}", now.hour(), now.minute())
    } else {
        let h = now.hour() % 12;
        format!("{}:{:02} {}", if h == 0 { 12 } else { h }, now.minute(), if now.hour() < 12 { "AM" } else { "PM" })
    }
}

fn french_date(now: &glib::DateTime) -> String {
    const DAYS: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
    const MONTHS: [&str; 12] =
        ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    let day = DAYS[(now.day_of_week() as usize).clamp(1, 7) - 1];
    let mut s = format!("{day} {} {}", now.day_of_month(), MONTHS[(now.month() as usize).clamp(1, 12) - 1]);
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

/// Fond : image du thème (nette ou floutée), sinon dégradé NOOS (CSS).
fn background_picture(cfg: &LoginConfig, blur: bool) -> gtk::Widget {
    let path = std::path::Path::new(&cfg.background);
    if !cfg.background.is_empty() && path.is_file() {
        // Flou : image réduite à 48 px de large, agrandie avec filtrage linéaire.
        let texture = if blur {
            gdk_pixbuf::Pixbuf::from_file_at_scale(path, 48, -1, true).ok().map(|p| gdk::Texture::for_pixbuf(&p))
        } else {
            gdk::Texture::from_filename(path).ok()
        };
        if let Some(t) = texture {
            let pic = gtk::Picture::for_paintable(&t);
            pic.set_content_fit(gtk::ContentFit::Cover);
            pic.set_can_shrink(true);
            if blur {
                pic.add_css_class("blurred");
            }
            return pic.upcast();
        }
    }
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class(if blur { "login-gradient-blur" } else { "login-gradient" });
    b.set_hexpand(true);
    b.set_vexpand(true);
    b.upcast()
}

fn apply_theme(cfg: &LoginConfig) {
    let app = Appearance {
        color_scheme: cfg.color_scheme,
        palette: cfg.palette.clone(),
        accent: cfg.accent.clone(),
        ..Default::default()
    };
    let style = adap::StyleManager::default();
    style.set_color_scheme(noos_theme::adap_color_scheme(cfg.color_scheme));
    let css = format!("{}{}{}", noos_theme::palette::colors_css(&app, style.is_dark()), noos_theme::overrides_css(&app), CSS);
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

const CSS: &str = r#"
window.noos-login { background: #11111b; color: white; }
.login-gradient { background: radial-gradient(circle at 25% 30%, alpha(@accent_bg_color, 0.55), transparent 55%),
  radial-gradient(circle at 80% 75%, alpha(#cba6f7, 0.40), transparent 50%), #11111b; }
.login-gradient-blur { background: radial-gradient(circle at 25% 30%, alpha(@accent_bg_color, 0.35), transparent 65%),
  radial-gradient(circle at 80% 75%, alpha(#cba6f7, 0.25), transparent 60%), #14141f; }
.lock-time { font-size: 112px; font-weight: 600; color: white; text-shadow: 0 2px 12px rgba(0,0,0,0.45); }
.lock-date { font-size: 26px; font-weight: 500; color: white; text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.lock-hint { color: alpha(white, 0.85); font-size: 15px; text-shadow: 0 1px 6px rgba(0,0,0,0.6); }
.login-avatar { border-radius: 9999px; box-shadow: 0 6px 30px rgba(0,0,0,0.35); }
.login-name { font-size: 28px; font-weight: 600; color: white; text-shadow: 0 1px 8px rgba(0,0,0,0.5); }
.login-entry { min-height: 44px; font-size: 15px; border-radius: 8px; background-color: alpha(black, 0.35); color: white;
  border: 1px solid alpha(white, 0.18); }
.linked .login-entry { border-top-right-radius: 0; border-bottom-right-radius: 0; }
.login-submit { min-width: 48px; min-height: 44px; border-radius: 0 8px 8px 0; }
.login-message { color: alpha(white, 0.9); }
.login-message.error { color: #ffb4b4; font-weight: 600; }
.login-section { color: alpha(white, 0.75); font-weight: 600; font-size: 13px; letter-spacing: 1px; }
button.session-card { min-width: 132px; padding: 12px 10px; border-radius: 10px; background-color: alpha(black, 0.30);
  color: white; border: 2px solid alpha(white, 0.10); }
button.session-card:hover { background-color: alpha(white, 0.12); }
button.session-card.selected { border-color: @accent_bg_color; background-color: alpha(@accent_bg_color, 0.28); }
.session-name { font-weight: 700; font-size: 14px; }
.session-kind { font-size: 11px; color: alpha(white, 0.65); }
list.login-users { background: transparent; }
list.login-users > row { border-radius: 8px; padding: 6px 10px; margin: 2px 0; color: white; background: transparent; }
list.login-users > row:hover { background-color: alpha(white, 0.10); }
list.login-users > row:selected { background-color: alpha(white, 0.18); }
.login-tray-button { min-width: 44px; min-height: 44px; border-radius: 8px; color: white; background-color: alpha(black, 0.25); }
.login-tray-button:hover { background-color: alpha(white, 0.15); }
.login-layout { color: white; font-weight: 700; padding: 0 10px; }
.login-host { color: alpha(white, 0.7); font-size: 13px; }
.osk { background-color: alpha(#1e1e2e, 0.92); border-radius: 14px; padding: 12px; margin-bottom: 8px;
  box-shadow: 0 8px 30px rgba(0,0,0,0.4); }
button.osk-key { min-height: 44px; font-size: 16px; border-radius: 8px; background-color: alpha(white, 0.10); color: white; }
button.osk-key:hover { background-color: alpha(white, 0.20); }
window.hc .login-dim { background-color: rgba(0,0,0,0.85); }
window.hc label { color: white; text-shadow: none; }
window.hc .login-entry { border: 2px solid white; background-color: black; }
window.hc button.session-card { border: 2px solid white; background-color: black; }
window.hc button.session-card.selected { border: 4px solid yellow; }
window.hc .login-tray-button { border: 2px solid white; background-color: black; }
"#;

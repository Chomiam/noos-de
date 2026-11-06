//! Agent d'authentification polkit de NOOS : la fenêtre « Contrôle de compte d'utilisateur ».
//!
//! Sans agent, polkit refuse d'office toute action d'administration demandée depuis la
//! session (écrire une clé USB avec Popsicle via udisks, pkexec, installer un paquet…).
//! L'agent s'enregistre auprès de polkitd (bus système) pour la session ; à chaque demande
//! (`BeginAuthentication`), il affiche une fenêtre façon Windows 11 par-dessus l'écran
//! assombri, puis confie le mot de passe à l'assistant setuid de polkit
//! (`polkit-agent-helper-1`), qui vérifie via PAM et répond lui-même à polkitd.
//!
//! Protocole de l'assistant : `polkit-agent-helper-1 <utilisateur>`, cookie sur l'entrée
//! standard, puis échanges ligne à ligne (`PAM_PROMPT_ECHO_OFF <invite>` → réponse, …),
//! terminés par `SUCCESS` ou `FAILURE`.
//!
//! Test sans polkit : `NOOS_POLKIT_TEST=1 noos-shell` affiche la fenêtre avec une demande
//! d'exemple.

use std::{
    cell::RefCell,
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::Shell;

const AGENT_PATH: &str = "/org/noos/PolicyKit1/AuthenticationAgent";
const HELPERS: &[&str] = &[
    "/usr/lib/polkit-1/polkit-agent-helper-1",
    "/usr/libexec/polkit-agent-helper-1",
    "/usr/lib/policykit-1/polkit-agent-helper-1",
];

const XML: &str = r#"<node>
  <interface name="org.freedesktop.PolicyKit1.AuthenticationAgent">
    <method name="BeginAuthentication">
      <arg type="s" name="action_id" direction="in"/>
      <arg type="s" name="message" direction="in"/>
      <arg type="s" name="icon_name" direction="in"/>
      <arg type="a{ss}" name="details" direction="in"/>
      <arg type="s" name="cookie" direction="in"/>
      <arg type="a(sa{sv})" name="identities" direction="in"/>
    </method>
    <method name="CancelAuthentication">
      <arg type="s" name="cookie" direction="in"/>
    </method>
  </interface>
</node>"#;

/// Une demande d'autorisation en cours.
struct Request {
    action_id: String,
    message: String,
    icon: String,
    details: HashMap<String, String>,
    cookie: String,
    /// Comptes autorisés à valider (nom d'utilisateur, nom affiché).
    users: Vec<(String, String)>,
    invocation: Option<gio::DBusMethodInvocation>,
}

pub struct PolkitAgent {
    shell: Rc<Shell>,
    current: RefCell<Option<Request>>,
    /// Demandes arrivées pendant qu'une autre est affichée.
    queue: RefCell<Vec<Request>>,
    window: RefCell<Option<gtk::Window>>,
    /// Connexion au bus système : gio n'en garde qu'une référence faible ; si elle se
    /// ferme, polkitd oublie l'agent (« No authentication agent found »).
    conn: RefCell<Option<gio::DBusConnection>>,
    /// Assistant en cours (pour l'annuler).
    helper: RefCell<Option<std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>>>,
}

impl PolkitAgent {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let agent = Rc::new(Self {
            shell: shell.clone(),
            current: RefCell::new(None),
            queue: RefCell::new(Vec::new()),
            window: RefCell::new(None),
            conn: RefCell::new(None),
            helper: RefCell::new(None),
        });
        if std::env::var_os("NOOS_POLKIT_TEST").is_some() {
            let a = agent.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || a.demo());
        } else if crate::own_session_names() {
            agent.register();
        }
        agent
    }

    // ───────────── D-Bus ─────────────

    fn register(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let conn = match gio::bus_get_future(gio::BusType::System).await {
                Ok(c) => c,
                Err(e) => return eprintln!("noos-shell : polkit : bus système : {e}"),
            };
            *this.conn.borrow_mut() = Some(conn.clone());
            let node = gio::DBusNodeInfo::for_xml(XML).expect("XML polkit");
            let iface = node.lookup_interface("org.freedesktop.PolicyKit1.AuthenticationAgent").expect("interface polkit");
            let this2 = this.clone();
            let res = conn
                .register_object(AGENT_PATH, &iface)
                .method_call(move |_, _sender, _, _, method, params, invocation| {
                    this2.dbus_call(method, &params, invocation);
                })
                .build();
            if let Err(e) = res {
                return eprintln!("noos-shell : polkit : objet D-Bus : {e}");
            }
            let Some(session) = session_id(&conn).await else {
                return eprintln!("noos-shell : polkit : session logind introuvable, pas d'agent d'authentification");
            };
            let mut subject_details = HashMap::new();
            subject_details.insert("session-id".to_string(), session.to_variant());
            let subject = ("unix-session".to_string(), subject_details);
            let locale = std::env::var("LANG").unwrap_or_else(|_| "fr_FR.UTF-8".into());
            let args = (subject, locale, AGENT_PATH.to_string()).to_variant();
            match conn
                .call_future(
                    Some("org.freedesktop.PolicyKit1"),
                    "/org/freedesktop/PolicyKit1/Authority",
                    "org.freedesktop.PolicyKit1.Authority",
                    "RegisterAuthenticationAgent",
                    Some(&args),
                    None,
                    gio::DBusCallFlags::NONE,
                    -1,
                )
                .await
            {
                Ok(_) => eprintln!("noos-shell : agent polkit enregistré (session {session})"),
                Err(e) => eprintln!("noos-shell : polkit : enregistrement de l'agent refusé : {e}"),
            }
        });
    }

    fn dbus_call(self: &Rc<Self>, method: &str, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        match method {
            "BeginAuthentication" => {
                type Ids = Vec<(String, HashMap<String, glib::Variant>)>;
                type Args = (String, String, String, HashMap<String, String>, String, Ids);
                let Some((action_id, message, icon, details, cookie, identities)) = params.get::<Args>() else {
                    invocation.return_error(gio::DBusError::InvalidArgs, "arguments invalides");
                    return;
                };
                let users = identities_to_users(&identities);
                if users.is_empty() {
                    invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.Failed", "aucun compte utilisable");
                    return;
                }
                let req = Request { action_id, message, icon, details, cookie, users, invocation: Some(invocation) };
                if self.current.borrow().is_some() {
                    self.queue.borrow_mut().push(req);
                } else {
                    self.show(req);
                }
            }
            "CancelAuthentication" => {
                let cookie = params.get::<(String,)>().map(|c| c.0).unwrap_or_default();
                invocation.return_value(None);
                let current = self.current.borrow().as_ref().map(|r| r.cookie.clone());
                if current.as_deref() == Some(cookie.as_str()) {
                    self.finish(Err("Cancelled"));
                } else {
                    let mut q = self.queue.borrow_mut();
                    if let Some(i) = q.iter().position(|r| r.cookie == cookie) {
                        let mut r = q.remove(i);
                        if let Some(inv) = r.invocation.take() {
                            inv.return_dbus_error("org.freedesktop.PolicyKit1.Error.Cancelled", "annulé");
                        }
                    }
                }
            }
            _ => invocation.return_error(gio::DBusError::UnknownMethod, "méthode inconnue"),
        }
    }

    /// Termine la demande courante (Ok = authentifiée) et passe à la suivante.
    fn finish(self: &Rc<Self>, result: Result<(), &str>) {
        if let Some(h) = self.helper.borrow_mut().take() {
            if let Some(mut child) = h.lock().unwrap().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(w) = self.window.borrow_mut().take() {
            w.close();
        }
        if let Some(mut req) = self.current.borrow_mut().take() {
            if let Some(inv) = req.invocation.take() {
                match result {
                    Ok(()) => inv.return_value(None),
                    Err(kind) => inv.return_dbus_error(&format!("org.freedesktop.PolicyKit1.Error.{kind}"), "authentification non effectuée"),
                }
            }
        }
        let next = {
            let mut q = self.queue.borrow_mut();
            if q.is_empty() { None } else { Some(q.remove(0)) }
        };
        if let Some(req) = next {
            self.show(req);
        }
    }

    // ───────────── Fenêtre ─────────────

    fn demo(self: &Rc<Self>) {
        let me = whoami();
        let mut details = HashMap::new();
        details.insert("drive".into(), "SanDisk Ultra (/dev/sdb)".into());
        details.insert("program".into(), "/usr/bin/popsicle-gtk".into());
        self.show(Request {
            action_id: "org.freedesktop.udisks2.open-device".into(),
            message: "L'authentification est nécessaire pour ouvrir le périphérique SanDisk Ultra (/dev/sdb).".into(),
            icon: "drive-removable-media-symbolic".into(),
            details,
            cookie: "demo".into(),
            users: vec![(me.clone(), display_name(&me))],
            invocation: None,
        });
    }

    fn show(self: &Rc<Self>, req: Request) {
        let window = gtk::Window::builder().application(&self.shell.app).decorated(false).build();
        window.add_css_class("uac-window");
        window.init_layer_shell();
        window.set_namespace(Some("noos-polkit"));
        window.set_layer(Layer::Overlay);
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("uac-card");
        card.set_halign(gtk::Align::Center);
        card.set_valign(gtk::Align::Center);
        card.set_width_request(480);

        // Bandeau d'en-tête (comme le contrôle de compte de Windows).
        let band = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        band.add_css_class("uac-band");
        let shield = gtk::Image::from_icon_name("security-high-symbolic");
        shield.set_pixel_size(18);
        band.append(&shield);
        band.append(&gtk::Label::new(Some("Contrôle de compte d'utilisateur")));
        card.append(&band);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.add_css_class("uac-body");
        let title = gtk::Label::new(Some("Voulez-vous autoriser cette action à modifier votre appareil ?"));
        title.add_css_class("uac-title");
        title.set_wrap(true);
        title.set_xalign(0.0);
        body.append(&title);

        // Ce qui est demandé.
        let what = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let icon = gtk::Image::from_icon_name(if req.icon.is_empty() { "dialog-password-symbolic" } else { &req.icon });
        icon.set_pixel_size(40);
        icon.set_valign(gtk::Align::Start);
        what.append(&icon);
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let program = req
            .details
            .get("program")
            .or_else(|| req.details.get("command_line"))
            .map(|p| program_name(p))
            // Sinon le processus demandeur (polkitd fournit son PID) : « popsicle-gtk »…
            .or_else(|| {
                ["polkit.subject-pid", "polkit.caller-pid"].iter().find_map(|k| {
                    let pid = req.details.get(*k)?;
                    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
                    Some(comm.trim().to_string()).filter(|c| !c.is_empty())
                })
            })
            .unwrap_or_else(|| "Service système".into());
        let prog = gtk::Label::new(Some(&program));
        prog.add_css_class("uac-program");
        prog.set_xalign(0.0);
        texts.append(&prog);
        let msg = gtk::Label::new(Some(&req.message));
        msg.set_wrap(true);
        msg.set_xalign(0.0);
        msg.add_css_class("dim-label");
        texts.append(&msg);
        what.append(&texts);
        body.append(&what);

        // Détails (action polkit et paramètres).
        let more = gtk::Expander::new(Some("Afficher plus de détails"));
        let grid = gtk::Grid::new();
        grid.set_column_spacing(12);
        grid.set_row_spacing(4);
        grid.set_margin_top(6);
        let mut rows: Vec<(String, String)> = vec![("Action".into(), req.action_id.clone())];
        let mut keys: Vec<_> = req.details.iter().collect();
        keys.sort();
        rows.extend(keys.into_iter().map(|(k, v)| (k.clone(), v.clone())));
        for (i, (k, v)) in rows.iter().enumerate() {
            let kl = gtk::Label::new(Some(k));
            kl.set_xalign(0.0);
            kl.add_css_class("dim-label");
            kl.add_css_class("caption");
            let vl = gtk::Label::new(Some(v));
            vl.set_xalign(0.0);
            vl.set_selectable(true);
            vl.set_wrap(true);
            vl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            vl.add_css_class("caption");
            grid.attach(&kl, 0, i as i32, 1, 1);
            grid.attach(&vl, 1, i as i32, 1, 1);
        }
        more.set_child(Some(&grid));
        body.append(&more);

        // Compte administrateur.
        let account = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        account.add_css_class("uac-account");
        let avatar = adap::Avatar::new(44, Some(&req.users[0].1), true);
        account.append(&avatar);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_valign(gtk::Align::Center);
        let user_combo = gtk::DropDown::from_strings(&req.users.iter().map(|u| u.1.as_str()).collect::<Vec<_>>());
        if req.users.len() > 1 {
            names.append(&user_combo);
        } else {
            let n = gtk::Label::new(Some(&req.users[0].1));
            n.set_xalign(0.0);
            n.add_css_class("heading");
            names.append(&n);
        }
        let sub = gtk::Label::new(Some("Administrateur"));
        sub.set_xalign(0.0);
        sub.add_css_class("caption");
        sub.add_css_class("dim-label");
        names.append(&sub);
        account.append(&names);
        body.append(&account);

        let password = gtk::PasswordEntry::new();
        password.set_show_peek_icon(true);
        password.set_placeholder_text(Some("Mot de passe"));
        password.add_css_class("uac-password");
        body.append(&password);
        let error = gtk::Label::new(None);
        error.add_css_class("uac-error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        body.append(&error);
        card.append(&body);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        buttons.add_css_class("uac-buttons");
        buttons.set_homogeneous(true);
        let yes = gtk::Button::with_label("Oui");
        yes.add_css_class("suggested-action");
        let no = gtk::Button::with_label("Non");
        let spinner = gtk::Spinner::new();
        buttons.append(&yes);
        buttons.append(&no);
        card.append(&buttons);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("uac-dim");
        root.append(&card);
        card.set_vexpand(true);
        window.set_child(Some(&root));

        let users: Vec<String> = req.users.iter().map(|u| u.0.clone()).collect();
        let cookie = req.cookie.clone();
        let demo = req.invocation.is_none();
        *self.current.borrow_mut() = Some(req);

        {
            let names_for_avatar: Vec<String> = self.current.borrow().as_ref().unwrap().users.iter().map(|u| u.1.clone()).collect();
            let avatar = avatar.clone();
            user_combo.connect_selected_notify(move |d| {
                if let Some(n) = names_for_avatar.get(d.selected() as usize) {
                    avatar.set_text(Some(n));
                }
            });
        }
        let submit = {
            let this = self.clone();
            let (password, error, yes, no, user_combo, spinner) =
                (password.clone(), error.clone(), yes.clone(), no.clone(), user_combo.clone(), spinner.clone());
            Rc::new(move || {
                let pw = password.text().to_string();
                if pw.is_empty() {
                    password.grab_focus();
                    return;
                }
                let user = users.get(user_combo.selected() as usize).or(users.first()).cloned().unwrap_or_default();
                if demo {
                    this.finish(Ok(()));
                    return;
                }
                for w in [password.upcast_ref::<gtk::Widget>(), yes.upcast_ref(), no.upcast_ref()] {
                    w.set_sensitive(false);
                }
                spinner.start();
                error.set_visible(false);
                let rx = run_helper(&user, &cookie, pw, &this);
                let (this, password, error, yes, no, spinner) =
                    (this.clone(), password.clone(), error.clone(), yes.clone(), no.clone(), spinner.clone());
                glib::spawn_future_local(async move {
                    let outcome = rx.recv().await.unwrap_or(HelperOutcome::Failed("assistant polkit interrompu".into()));
                    spinner.stop();
                    match outcome {
                        HelperOutcome::Success => this.finish(Ok(())),
                        HelperOutcome::Failed(msg) => {
                            for w in [password.upcast_ref::<gtk::Widget>(), yes.upcast_ref(), no.upcast_ref()] {
                                w.set_sensitive(true);
                            }
                            password.set_text("");
                            password.grab_focus();
                            error.set_label(&msg);
                            error.set_visible(true);
                        }
                    }
                });
            })
        };
        {
            let s = submit.clone();
            yes.connect_clicked(move |_| s());
        }
        {
            let s = submit.clone();
            password.connect_activate(move |_| s());
        }
        {
            let this = self.clone();
            no.connect_clicked(move |_| this.finish(Err("Cancelled")));
        }
        let keys = gtk::EventControllerKey::new();
        {
            let this = self.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    this.finish(Err("Cancelled"));
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        window.add_controller(keys);
        window.present();
        password.grab_focus();
        *self.window.borrow_mut() = Some(window);
    }
}

// ───────────── Assistant polkit ─────────────

enum HelperOutcome {
    Success,
    Failed(String),
}

/// Lance `polkit-agent-helper-1` dans un fil : le mot de passe répond à la première
/// question PAM masquée ; le résultat arrive sur le canal.
fn run_helper(user: &str, cookie: &str, password: String, agent: &Rc<PolkitAgent>) -> async_channel::Receiver<HelperOutcome> {
    let (tx, rx) = async_channel::bounded(1);
    let Some(helper) = HELPERS.iter().find(|p| std::path::Path::new(p).exists()) else {
        let _ = tx.send_blocking(HelperOutcome::Failed("polkit-agent-helper-1 introuvable".into()));
        return rx;
    };
    let child = Command::new(helper)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send_blocking(HelperOutcome::Failed(format!("impossible de lancer l'assistant polkit : {e}")));
            return rx;
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(Some(child)));
    *agent.helper.borrow_mut() = Some(shared.clone());
    let cookie = cookie.to_string();
    std::thread::spawn(move || {
        let outcome = (|| {
            let mut stdin = stdin?;
            let stdout = stdout?;
            writeln!(stdin, "{cookie}").ok()?;
            let mut password = Some(password);
            let mut last_error = String::new();
            for line in BufReader::new(stdout).lines() {
                let line = line.ok()?;
                if line == "SUCCESS" {
                    return Some(HelperOutcome::Success);
                }
                if line == "FAILURE" {
                    break;
                }
                if line.starts_with("PAM_PROMPT_ECHO_OFF") || line.starts_with("PAM_PROMPT_ECHO_ON") {
                    // Une seule réponse possible : le mot de passe saisi (pas de 2e facteur ici).
                    let answer = password.take().unwrap_or_default();
                    writeln!(stdin, "{answer}").ok()?;
                    stdin.flush().ok()?;
                } else if let Some(msg) = line.strip_prefix("PAM_ERROR_MSG ") {
                    last_error = msg.to_string();
                }
            }
            let msg = if last_error.is_empty() { "Mot de passe incorrect. Réessayez.".to_string() } else { last_error };
            Some(HelperOutcome::Failed(msg))
        })()
        .unwrap_or(HelperOutcome::Failed("échange avec l'assistant polkit interrompu".into()));
        if let Some(mut c) = shared.lock().unwrap().take() {
            let _ = c.wait();
        }
        let _ = tx.send_blocking(outcome);
    });
    rx
}

// ───────────── Utilitaires ─────────────

/// Session logind de ce processus : $XDG_SESSION_ID, sinon logind (GetSessionByPID).
async fn session_id(conn: &gio::DBusConnection) -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if !id.is_empty() {
            return Some(id);
        }
    }
    let reply = conn
        .call_future(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "GetSessionByPID",
            Some(&(std::process::id(),).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            -1,
        )
        .await
        .ok()?;
    let path: glib::variant::ObjectPath = reply.child_value(0).get()?;
    let prop = conn
        .call_future(
            Some("org.freedesktop.login1"),
            path.as_str(),
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.freedesktop.login1.Session", "Id").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            -1,
        )
        .await
        .ok()?;
    prop.child_value(0).as_variant()?.get::<String>()
}

fn passwd() -> Vec<Vec<String>> {
    std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .map(|l| l.split(':').map(String::from).collect())
        .collect()
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "utilisateur".into())
}

fn display_name(user: &str) -> String {
    passwd()
        .into_iter()
        .find(|f| f.first().map(String::as_str) == Some(user))
        .and_then(|f| f.get(4).map(|g| g.split(',').next().unwrap_or("").to_string()))
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| user.to_string())
}

/// Identités polkit (« unix-user » {uid}) → comptes ; l'utilisateur courant en premier.
fn identities_to_users(ids: &[(String, HashMap<String, glib::Variant>)]) -> Vec<(String, String)> {
    let pw = passwd();
    let me = whoami();
    let mut users: Vec<(String, String)> = ids
        .iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, d)| d.get("uid").and_then(|v| v.get::<u32>()))
        .filter_map(|uid| {
            pw.iter()
                .find(|f| f.get(2).and_then(|u| u.parse::<u32>().ok()) == Some(uid))
                .and_then(|f| f.first().cloned())
                .or_else(|| {
                    // Comptes hors /etc/passwd (systemd-homed, LDAP) : getent.
                    Command::new("getent")
                        .args(["passwd", &uid.to_string()])
                        .output()
                        .ok()
                        .and_then(|o| String::from_utf8_lossy(&o.stdout).split(':').next().map(String::from))
                        .filter(|n| !n.is_empty())
                })
        })
        .map(|name| {
            let display = display_name(&name);
            (name, display)
        })
        .collect();
    users.sort_by_key(|(n, _)| *n != me);
    users.dedup();
    users
}

/// « /usr/bin/popsicle-gtk --x » → « popsicle-gtk ».
fn program_name(p: &str) -> String {
    let first = p.split_whitespace().next().unwrap_or(p);
    first.rsplit('/').next().unwrap_or(first).to_string()
}

pub const CSS: &str = r#"
window.uac-window { background: transparent; }
.uac-dim { background-color: rgba(0, 0, 0, 0.55); }
/* Base opaque sous la couleur (souvent translucide) du thème : texte toujours lisible. */
.uac-card { background: linear-gradient(@window_bg_color, @window_bg_color), #1c1c26; border-radius: 12px; border: 1px solid alpha(@window_fg_color, 0.12);
  box-shadow: 0 18px 60px rgba(0,0,0,0.55); }
.uac-band { background-color: @accent_bg_color; color: @accent_fg_color; padding: 10px 18px; border-radius: 12px 12px 0 0;
  font-weight: 600; font-size: 13px; }
.uac-body { padding: 22px 26px 12px 26px; }
.uac-title { font-size: 19px; font-weight: 700; }
.uac-program { font-weight: 700; font-size: 15px; }
.uac-account { padding: 8px 0; }
.uac-password { min-height: 40px; }
.uac-error { color: @error_color; font-weight: 600; }
.uac-buttons { padding: 14px 26px 22px 26px; background-color: alpha(@window_fg_color, 0.04); border-radius: 0 0 12px 12px; }
.uac-buttons button { min-height: 38px; }
"#;

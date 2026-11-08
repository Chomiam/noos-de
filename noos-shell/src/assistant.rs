//! Panneau Assistant (bord droit, style Copilot) : conversation avec l'assistant local
//! `noos-ai` (D-Bus `org.noos.Assistant1`, démarré à la demande). Super+A ou le bouton de la
//! barre l'ouvrent. Les actions qui touchent au système s'affichent en carte de confirmation.

use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;

use crate::{Shell, apps};

const BUS: &str = "org.noos.Assistant1";
const PATH: &str = "/org/noos/Assistant";

const SUGGESTIONS: &[(&str, &str)] = &[
    ("audio-volume-high-symbolic", "Monte le son à 50 %"),
    ("network-wireless-symbolic", "Ouvre les paramètres réseau"),
    ("software-update-available-symbolic", "Mets à jour mon système"),
    ("weather-clear-night-symbolic", "Passe en mode sombre"),
    ("preferences-system-time-symbolic", "Quelle heure est-il ?"),
    ("utilities-terminal-symbolic", "Ouvre le terminal"),
];

const CSS: &str = r#"
.assistant { padding: 0; min-width: 400px; }
.assistant .ai-head { padding: 16px 16px 12px 16px; border-bottom: 1px solid alpha(@window_fg_color, 0.08); }
.assistant .ai-avatar {
  min-width: 34px; min-height: 34px; border-radius: 99px;
  background-image: linear-gradient(135deg, #7c5cff, #e2549b 55%, #ff9f43);
  color: white;
}
.assistant .ai-title { font-weight: 700; font-size: 1.05em; }
.assistant .ai-sub { font-size: 0.82em; opacity: 0.65; }
.assistant .ai-chat { padding: 16px; }
.assistant .ai-hello { font-size: 1.35em; font-weight: 700; margin-top: 18px; }
.assistant .ai-hello-sub { opacity: 0.7; margin-bottom: 8px; }
.assistant .ai-chip {
  border-radius: 10px; padding: 8px 12px;
  background-color: alpha(@window_fg_color, 0.05);
  border: 1px solid alpha(@window_fg_color, 0.08);
}
.assistant .ai-chip:hover { background-color: alpha(@accent_bg_color, 0.18); }
.assistant .bubble { border-radius: 14px; padding: 9px 13px; }
.assistant .bubble.user {
  background-color: @accent_bg_color; color: @accent_fg_color;
  border-bottom-right-radius: 4px;
}
.assistant .bubble.bot {
  background-color: alpha(@window_fg_color, 0.06);
  border-bottom-left-radius: 4px;
}
.assistant .bubble.bot.error { background-color: alpha(@warning_bg_color, 0.18); }
.assistant .bubble.bot.done image.ok { color: @success_color; }
.assistant .ai-meta { font-size: 0.75em; opacity: 0.55; margin: 2px 6px 0 6px; }
.assistant .ai-confirm {
  border-radius: 14px; padding: 12px 14px;
  background-color: alpha(@accent_bg_color, 0.10);
  border: 1px solid alpha(@accent_bg_color, 0.35);
}
.assistant .ai-confirm .ai-confirm-title { font-weight: 700; }
.assistant .ai-input {
  margin: 12px 16px 16px 16px; padding: 4px 4px 4px 14px; border-radius: 99px;
  background-color: alpha(@window_fg_color, 0.06);
  border: 1px solid alpha(@window_fg_color, 0.10);
}
.assistant .ai-input:focus-within { border-color: alpha(@accent_bg_color, 0.8); }
.assistant .ai-input entry, .assistant .ai-input entry:focus {
  background: none; border: none; box-shadow: none; outline: none; min-height: 34px;
}
.assistant .ai-input button { border-radius: 99px; min-width: 34px; min-height: 34px; padding: 0; }
.assistant .ai-privacy { font-size: 0.75em; opacity: 0.5; margin: -8px 0 12px 0; }
"#;

pub struct Assistant {
    shell: std::rc::Weak<Shell>,
    window: gtk::Window,
    chat: gtk::Box,
    scroller: gtk::ScrolledWindow,
    entry: gtk::Entry,
    send: gtk::Button,
    busy: std::cell::Cell<bool>,
    conn: RefCell<Option<gio::DBusConnection>>,
}

/// Appel D-Bus asynchrone du démon ; `done` reçoit le JSON (ou une erreur mise en forme).
fn call(conn: &gio::DBusConnection, method: &str, args: glib::Variant, done: impl FnOnce(Value) + 'static) {
    conn.call(
        Some(BUS),
        PATH,
        BUS,
        method,
        Some(&args),
        Some(glib::VariantTy::new("(s)").unwrap()),
        gio::DBusCallFlags::NONE,
        200_000,
        gio::Cancellable::NONE,
        move |res| {
            let v = match res {
                Ok(v) => v.get::<(String,)>().and_then(|(s,)| serde_json::from_str(&s).ok()).unwrap_or(Value::Null),
                Err(e) => serde_json::json!({"ok": false, "kind": "error", "message": format!("L'assistant ne répond pas ({}).", e.message())}),
            };
            done(v);
        },
    );
}

fn label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_max_width_chars(36);
    l.set_selectable(true);
    l
}

impl Assistant {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }

        let window = gtk::Window::builder().application(&shell.app).decorated(false).default_width(420).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-assistant"));
        window.set_layer(Layer::Top);
        for e in [Edge::Top, Edge::Bottom, Edge::Right] {
            window.set_anchor(e, true);
            window.set_margin(e, 12);
        }
        window.set_keyboard_mode(KeyboardMode::OnDemand);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("noos-flyout");
        root.add_css_class("assistant");

        // ── En-tête ──
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        head.add_css_class("ai-head");
        let avatar = gtk::Image::from_icon_name("starred-symbolic");
        avatar.add_css_class("ai-avatar");
        head.append(&avatar);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 0);
        titles.set_hexpand(true);
        titles.set_valign(gtk::Align::Center);
        let t = gtk::Label::new(Some("Assistant"));
        t.add_css_class("ai-title");
        t.set_xalign(0.0);
        let s = gtk::Label::new(Some("IA locale · rien ne quitte ce PC"));
        s.add_css_class("ai-sub");
        s.set_xalign(0.0);
        titles.append(&t);
        titles.append(&s);
        head.append(&titles);
        let new_btn = gtk::Button::from_icon_name("list-add-symbolic");
        new_btn.set_tooltip_text(Some("Nouvelle conversation"));
        new_btn.add_css_class("flat");
        let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings_btn.set_tooltip_text(Some("Paramètres de l'IA"));
        settings_btn.add_css_class("flat");
        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        close_btn.set_tooltip_text(Some("Fermer (Super+A)"));
        close_btn.add_css_class("flat");
        head.append(&new_btn);
        head.append(&settings_btn);
        head.append(&close_btn);
        root.append(&head);

        // ── Conversation ──
        let chat = gtk::Box::new(gtk::Orientation::Vertical, 10);
        chat.add_css_class("ai-chat");
        chat.set_valign(gtk::Align::End);
        let scroller = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&chat).build();
        root.append(&scroller);

        // ── Saisie ──
        let input = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        input.add_css_class("ai-input");
        let entry = gtk::Entry::builder().placeholder_text("Demandez quelque chose…").hexpand(true).build();
        let mic = gtk::Button::from_icon_name("audio-input-microphone-symbolic");
        mic.add_css_class("flat");
        mic.set_sensitive(false);
        mic.set_tooltip_text(Some("Commande vocale : bientôt (Super+H)"));
        let send = gtk::Button::from_icon_name("go-up-symbolic");
        send.add_css_class("suggested-action");
        send.set_tooltip_text(Some("Envoyer (Entrée)"));
        input.append(&entry);
        input.append(&mic);
        input.append(&send);
        root.append(&input);
        let privacy = gtk::Label::new(Some("Les actions sur le système demandent toujours votre accord."));
        privacy.add_css_class("ai-privacy");
        root.append(&privacy);
        window.set_child(Some(&root));

        let a = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            chat,
            scroller,
            entry,
            send,
            busy: std::cell::Cell::new(false),
            conn: RefCell::new(None),
        });
        a.welcome();

        {
            let w = Rc::downgrade(&a);
            let go = move || {
                if let Some(a) = w.upgrade() {
                    let t = a.entry.text().trim().to_string();
                    if !t.is_empty() && !a.busy.get() {
                        a.entry.set_text("");
                        a.ask(&t);
                    }
                }
            };
            let g = go.clone();
            a.entry.connect_activate(move |_| g());
            a.send.connect_clicked(move |_| go());
        }
        {
            let w = Rc::downgrade(&a);
            close_btn.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    a.hide();
                }
            });
        }
        {
            let w = Rc::downgrade(&a);
            settings_btn.connect_clicked(move |_| {
                apps::spawn("noos-settings ai");
                if let Some(a) = w.upgrade() {
                    a.hide();
                }
            });
        }
        {
            let w = Rc::downgrade(&a);
            new_btn.connect_clicked(move |_| {
                let Some(a) = w.upgrade() else { return };
                if let Some(c) = a.conn() {
                    call(&c, "Reset", ().to_variant(), |_| {});
                }
                a.clear();
                a.welcome();
            });
        }
        {
            let keys = gtk::EventControllerKey::new();
            let w = Rc::downgrade(&a);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    if let Some(a) = w.upgrade() {
                        a.hide();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            a.window.add_controller(keys);
        }
        a
    }

    fn conn(&self) -> Option<gio::DBusConnection> {
        if self.conn.borrow().is_none() {
            *self.conn.borrow_mut() = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
        }
        self.conn.borrow().clone()
    }

    fn clear(&self) {
        while let Some(c) = self.chat.first_child() {
            self.chat.remove(&c);
        }
    }

    /// Accueil : salutation et suggestions cliquables.
    fn welcome(self: &Rc<Self>) {
        let hour = glib::DateTime::now_local().map(|d| d.hour()).unwrap_or(12);
        let hello = gtk::Label::new(Some(if !(5..18).contains(&hour) { "Bonsoir 👋" } else { "Bonjour 👋" }));
        hello.add_css_class("ai-hello");
        hello.set_xalign(0.0);
        let sub = label("Je peux régler l'ordinateur, ouvrir des applications et répondre à vos questions. Essayez :");
        sub.add_css_class("ai-hello-sub");
        sub.set_selectable(false);
        self.chat.append(&hello);
        self.chat.append(&sub);
        for (icon, text) in SUGGESTIONS {
            let b = gtk::Button::new();
            b.add_css_class("ai-chip");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(text));
            l.set_xalign(0.0);
            row.append(&l);
            b.set_child(Some(&row));
            let (w, text) = (Rc::downgrade(self), text.to_string());
            b.connect_clicked(move |_| {
                if let Some(a) = w.upgrade() {
                    if !a.busy.get() {
                        a.ask(&text);
                    }
                }
            });
            self.chat.append(&b);
        }
    }

    fn scroll_down(&self) {
        let adj = self.scroller.vadjustment();
        glib::timeout_add_local_once(Duration::from_millis(60), move || adj.set_value(adj.upper()));
    }

    fn bubble(&self, text: &str, user: bool) -> gtk::Box {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        b.add_css_class("bubble");
        b.add_css_class(if user { "user" } else { "bot" });
        b.set_halign(if user { gtk::Align::End } else { gtk::Align::Start });
        b.append(&label(text));
        self.chat.append(&b);
        self.scroll_down();
        b
    }

    fn meta(&self, text: &str) {
        let m = gtk::Label::new(Some(text));
        m.add_css_class("ai-meta");
        m.set_xalign(0.0);
        self.chat.append(&m);
    }

    pub fn ask(self: &Rc<Self>, text: &str) {
        // Première question : on retire l'accueil.
        if self.chat.first_child().is_some_and(|c| c.has_css_class("ai-hello")) {
            self.clear();
        }
        self.bubble(text, true);
        let thinking = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        thinking.add_css_class("bubble");
        thinking.add_css_class("bot");
        thinking.set_halign(gtk::Align::Start);
        let sp = gtk::Spinner::new();
        sp.start();
        thinking.append(&sp);
        let think_label = gtk::Label::new(Some("Je réfléchis…"));
        thinking.append(&think_label);
        {
            // Première question : le modèle se charge (jusqu'à une minute sur processeur).
            let l = think_label.downgrade();
            glib::timeout_add_local_once(Duration::from_secs(6), move || {
                if let Some(l) = l.upgrade() {
                    l.set_text("Chargement du modèle… la première réponse est plus lente.");
                }
            });
        }
        self.chat.append(&thinking);
        self.scroll_down();
        self.busy.set(true);
        self.send.set_sensitive(false);

        let Some(conn) = self.conn() else {
            self.chat.remove(&thinking);
            self.show(&serde_json::json!({"kind": "error", "message": "Bus de session indisponible."}));
            return;
        };
        let w = Rc::downgrade(self);
        call(&conn, "Ask", (text,).to_variant(), move |v| {
            let Some(a) = w.upgrade() else { return };
            a.chat.remove(&thinking);
            a.busy.set(false);
            a.send.set_sensitive(true);
            a.show(&v);
        });
    }

    /// Affiche une réponse du démon.
    fn show(self: &Rc<Self>, v: &Value) {
        let kind = v["kind"].as_str().unwrap_or("error");
        let msg = v["message"].as_str().unwrap_or("Réponse vide.");
        match kind {
            "confirm" => self.confirm_card(v),
            _ => {
                let b = self.bubble(msg, false);
                b.add_css_class(kind);
                let icon = match kind {
                    "done" => Some("object-select-symbolic"),
                    "error" => Some("dialog-warning-symbolic"),
                    _ => None,
                };
                if let Some(i) = icon {
                    let img = gtk::Image::from_icon_name(i);
                    img.set_valign(gtk::Align::Start);
                    if kind == "done" {
                        img.add_css_class("ok");
                    }
                    b.prepend(&img);
                }
                if kind == "error" && msg.contains("Paramètres › IA") {
                    let open = gtk::Button::with_label("Ouvrir Paramètres › IA");
                    open.set_halign(gtk::Align::Start);
                    open.add_css_class("pill");
                    let w = Rc::downgrade(self);
                    open.connect_clicked(move |_| {
                        apps::spawn("noos-settings ai");
                        if let Some(a) = w.upgrade() {
                            a.hide();
                        }
                    });
                    self.chat.append(&open);
                }
                match v["via"].as_str() {
                    Some("rules") => self.meta("Commande rapide · sans modèle"),
                    Some("model") => self.meta(&format!("Modèle local {}", v["model"].as_str().unwrap_or(""))),
                    _ => {}
                }
            }
        }
        self.scroll_down();
    }

    fn confirm_card(self: &Rc<Self>, v: &Value) {
        let id = v["pending"].as_u64().unwrap_or(0) as u32;
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("ai-confirm");
        let title = gtk::Label::new(Some(&format!("{} ?", v["title"].as_str().unwrap_or("Confirmer"))));
        title.add_css_class("ai-confirm-title");
        title.set_xalign(0.0);
        card.append(&title);
        let desc = label(v["message"].as_str().unwrap_or("").split_once(" ? ").map(|(_, d)| d).unwrap_or(""));
        desc.set_opacity(0.8);
        card.append(&desc);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let no = gtk::Button::with_label("Annuler");
        let yes = gtk::Button::with_label("Confirmer");
        let risky = matches!(v["action"].as_str(), Some("session.poweroff" | "session.reboot" | "session.logout"));
        yes.add_css_class(if risky { "destructive-action" } else { "suggested-action" });
        no.add_css_class("pill");
        yes.add_css_class("pill");
        buttons.append(&no);
        buttons.append(&yes);
        card.append(&buttons);
        self.chat.append(&card);
        for (btn, accept) in [(yes, true), (no, false)] {
            let (w, buttons) = (Rc::downgrade(self), buttons.clone());
            btn.connect_clicked(move |_| {
                let Some(a) = w.upgrade() else { return };
                buttons.set_sensitive(false);
                let Some(conn) = a.conn() else { return };
                let (w, buttons) = (w.clone(), buttons.clone());
                call(&conn, "Confirm", (id, accept).to_variant(), move |v| {
                    buttons.set_visible(false);
                    if let Some(a) = w.upgrade() {
                        a.show(&v);
                    }
                });
            });
        }
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.window.is_visible() {
            self.hide();
        } else {
            if let Some(s) = self.shell.upgrade() {
                s.close_flyouts();
            }
            self.window.present();
            self.entry.grab_focus();
            // Le démon charge le modèle pendant que l'on tape la question.
            if let Some(c) = self.conn() {
                call(&c, "Warm", ().to_variant(), |_| {});
            }
        }
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }
}

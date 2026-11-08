//! Notifications façon Windows 11 : serveur `org.freedesktop.Notifications`, bannières en
//! bas à droite et centre de notifications (cloche de la barre des tâches) avec calendrier.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use adap::prelude::*;
use gtk::{gdk, gio, glib, pango};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::{Shell, apps};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const XML: &str = r#"
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities"><arg type="as" direction="out"/></method>
    <method name="Notify">
      <arg type="s" direction="in"/><arg type="u" direction="in"/><arg type="s" direction="in"/>
      <arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/>
      <arg type="a{sv}" direction="in"/><arg type="i" direction="in"/>
      <arg type="u" direction="out"/>
    </method>
    <method name="CloseNotification"><arg type="u" direction="in"/></method>
    <method name="GetServerInformation">
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
      <arg type="s" direction="out"/><arg type="s" direction="out"/>
    </method>
    <signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>
    <signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>
  </interface>
</node>"#;

/// Raisons de fermeture (spécification freedesktop).
const EXPIRED: u32 = 1;
const DISMISSED: u32 = 2;
const CLOSED_BY_CALL: u32 = 3;

#[derive(Clone)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: Option<gio::Icon>,
    pub image: Option<gdk::Texture>,
    pub summary: String,
    pub body: String,
    /// (clé, libellé) ; la clé « default » est l'action du clic sur la notification.
    pub actions: Vec<(String, String)>,
    pub critical: bool,
    pub transient: bool,
    pub resident: bool,
    pub desktop_entry: Option<String>,
    pub time: glib::DateTime,
}

pub struct Notifications {
    shell: Weak<Shell>,
    scroll: gtk::ScrolledWindow,
    list: RefCell<Vec<Notification>>,
    /// Notifications affichées en bannière (ids), la plus récente en bas.
    banners: RefCell<Vec<u32>>,
    next_id: Cell<u32>,
    conn: RefCell<Option<gio::DBusConnection>>,
    banner_window: gtk::Window,
    banner_box: gtk::Box,
    center: gtk::Window,
    center_list: gtk::Box,
    empty_label: gtk::Label,
    dnd_button: gtk::ToggleButton,
    date_label: gtk::Label,
    calendar: gtk::Calendar,
    hidden_at: Cell<Option<Instant>>,
    syncing: Cell<bool>,
}

fn icon_from_string(s: &str) -> Option<gio::Icon> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(path) = s.strip_prefix("file://") {
        return Some(gio::FileIcon::new(&gio::File::for_path(path)).upcast());
    }
    if s.starts_with('/') {
        return Some(gio::FileIcon::new(&gio::File::for_path(s)).upcast());
    }
    Some(gio::ThemedIcon::new(s).upcast())
}

/// Image brute `(iiibiiay)` des indices image-data / icon_data.
fn texture_from_data(v: &glib::Variant) -> Option<gdk::Texture> {
    let (w, h, stride, alpha, _bits, channels, data) = v.get::<(i32, i32, i32, bool, i32, i32, Vec<u8>)>()?;
    if w <= 0 || h <= 0 || data.len() < (stride * (h - 1) + w * channels) as usize {
        return None;
    }
    let format = if alpha { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    Some(gdk::MemoryTexture::new(w, h, format, &glib::Bytes::from_owned(data), stride as usize).upcast())
}

fn texture_from_path(s: &str) -> Option<gdk::Texture> {
    let path = s.strip_prefix("file://").unwrap_or(s);
    if !path.starts_with('/') {
        return None;
    }
    gdk::Texture::from_filename(path).ok()
}

fn play_sound() {
    let file = "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga";
    if std::path::Path::new(file).exists() {
        let _ = std::process::Command::new("pw-play")
            .arg(file)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

fn layer_window(shell: &Shell, namespace: &str, keyboard: KeyboardMode) -> gtk::Window {
    let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
    window.add_css_class("noos-layer");
    window.init_layer_shell();
    window.set_namespace(Some(namespace));
    crate::editmode::anchor_to_bar(&window, &shell.config());
        shell.flyouts.borrow_mut().push(window.downgrade());
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Bottom, 12);
    window.set_margin(Edge::Right, 12);
    window.set_keyboard_mode(keyboard);
    window
}

impl Notifications {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let cfg = shell.config();

        // ── Bannières ──
        let banner_window = layer_window(shell, "noos-notifications", KeyboardMode::None);
        banner_window.set_layer(Layer::Overlay);
        // Les bannières suivent [notifications] position (coin choisi en mode édition).
        shell.flyouts.borrow_mut().retain(|w| w.upgrade().is_some_and(|w| w != banner_window));
        crate::editmode::place_notif(&banner_window, &cfg.notifications.position);
        let banner_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        banner_box.set_valign(gtk::Align::End);
        banner_window.set_child(Some(&banner_box));

        // ── Centre de notifications ──
        let center = layer_window(shell, "noos-notification-center", KeyboardMode::Exclusive);
        center.set_layer(Layer::Top);
        // Un seul panneau (notifications + calendrier) : le flou du compositeur couvre
        // toute la surface, sans bande vide entre deux cartes.
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_size_request(380, -1);
        root.add_css_class("noos-flyout");
        root.add_css_class("notif-center");

        let notif_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = gtk::Label::new(Some("Notifications"));
        title.add_css_class("section-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let dnd_button = gtk::ToggleButton::new();
        dnd_button.set_icon_name("notifications-disabled-symbolic");
        dnd_button.set_tooltip_text(Some("Ne pas déranger"));
        dnd_button.add_css_class("flat");
        dnd_button.set_active(cfg.notifications.do_not_disturb);
        header.append(&dnd_button);
        let clear = gtk::Button::with_label("Tout effacer");
        clear.add_css_class("flat");
        header.append(&clear);
        notif_panel.append(&header);

        let center_list = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let empty_label = gtk::Label::new(Some("Aucune nouvelle notification"));
        empty_label.add_css_class("dim-label");
        empty_label.set_margin_top(24);
        empty_label.set_margin_bottom(24);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(460);
        let list_wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_wrap.append(&empty_label);
        list_wrap.append(&center_list);
        scroll.set_child(Some(&list_wrap));
        notif_panel.append(&scroll);
        root.append(&notif_panel);

        // Calendrier (comme le panneau date/heure de Windows 11).
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let cal_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        cal_panel.add_css_class("calendar-panel");
        let date_label = gtk::Label::new(None);
        date_label.add_css_class("section-title");
        date_label.set_xalign(0.0);
        cal_panel.append(&date_label);
        let calendar = gtk::Calendar::new();
        cal_panel.append(&calendar);
        root.append(&cal_panel);
        center.set_child(Some(&root));

        let n = Rc::new(Self {
            shell: Rc::downgrade(shell),
            scroll: scroll.clone(),
            list: RefCell::new(Vec::new()),
            banners: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            conn: RefCell::new(None),
            banner_window,
            banner_box,
            center,
            center_list,
            empty_label,
            dnd_button,
            date_label,
            calendar,
            hidden_at: Cell::new(None),
            syncing: Cell::new(false),
        });

        {
            let n2 = n.clone();
            clear.connect_clicked(move |_| n2.clear_all());
        }
        {
            let n2 = n.clone();
            n.dnd_button.connect_toggled(move |b| {
                if n2.syncing.get() {
                    return;
                }
                let on = b.is_active();
                if let Some(shell) = n2.shell.upgrade() {
                    shell.update_config(|c| c.notifications.do_not_disturb = on);
                }
                n2.changed();
            });
        }
        {
            let n2 = n.clone();
            crate::close_on_focus_loss(&n.center, move || n2.hide());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let n2 = n.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    n2.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            n.center.add_controller(keys);
        }
        n.start_dbus();
        n.rebuild_center();
        n
    }

    // ───────────── D-Bus ─────────────

    fn start_dbus(self: &Rc<Self>) {
        if !crate::own_session_names() {
            return;
        }
        let this = self.clone();
        let _ = gio::bus_own_name(
            gio::BusType::Session,
            BUS_NAME,
            gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
            move |conn, _| {
                let node = gio::DBusNodeInfo::for_xml(XML).expect("XML notifications");
                let iface = node.lookup_interface(BUS_NAME).expect("interface notifications");
                let this2 = this.clone();
                let res = conn
                    .register_object(PATH, &iface)
                    .method_call(move |_, _sender, _, _, method, params, invocation| {
                        this2.dbus_call(method, &params, invocation);
                    })
                    .build();
                if let Err(e) = res {
                    eprintln!("noos-shell : notifications D-Bus : {e}");
                }
                *this.conn.borrow_mut() = Some(conn);
            },
            |_, _| {},
            |_, name| eprintln!("noos-shell : nom D-Bus {name} indisponible (autre serveur de notifications ?)"),
        );
    }

    fn dbus_call(self: &Rc<Self>, method: &str, params: &glib::Variant, invocation: gio::DBusMethodInvocation) {
        match method {
            "GetCapabilities" => {
                let caps: Vec<String> = ["actions", "body", "body-markup", "body-hyperlinks", "icon-static", "persistence"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                invocation.return_value(Some(&(caps,).to_variant()));
            }
            "GetServerInformation" => {
                invocation.return_value(Some(&("NOOS", "NOOS", env!("CARGO_PKG_VERSION"), "1.2").to_variant()));
            }
            "CloseNotification" => {
                if let Some((id,)) = params.get::<(u32,)>() {
                    self.close(id, CLOSED_BY_CALL);
                }
                invocation.return_value(None);
            }
            "Notify" => {
                type Args = (String, u32, String, String, String, Vec<String>, HashMap<String, glib::Variant>, i32);
                let Some(args) = params.get::<Args>() else {
                    invocation.return_error(gio::DBusError::InvalidArgs, "arguments invalides");
                    return;
                };
                let id = self.notify(args);
                invocation.return_value(Some(&(id,).to_variant()));
            }
            _ => invocation.return_error(gio::DBusError::UnknownMethod, "méthode inconnue"),
        }
    }

    fn emit(&self, signal: &str, args: glib::Variant) {
        if let Some(conn) = self.conn.borrow().as_ref() {
            let _ = conn.emit_signal(None, PATH, BUS_NAME, signal, Some(&args));
        }
    }

    // ───────────── Logique ─────────────

    #[allow(clippy::type_complexity)]
    fn notify(
        self: &Rc<Self>,
        (app_name, replaces, app_icon, summary, body, actions, hints, timeout): (
            String,
            u32,
            String,
            String,
            String,
            Vec<String>,
            HashMap<String, glib::Variant>,
            i32,
        ),
    ) -> u32 {
        let hint_str = |k: &str| hints.get(k).and_then(|v| v.get::<String>()).filter(|s| !s.is_empty());
        let hint_bool = |k: &str| hints.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false);
        let critical = hints.get("urgency").and_then(|v| v.get::<u8>()).unwrap_or(1) >= 2;
        let desktop_entry = hint_str("desktop-entry");

        let image = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|k| hints.get(*k).and_then(texture_from_data))
            .or_else(|| hint_str("image-path").or_else(|| hint_str("image_path")).and_then(|p| texture_from_path(&p)));
        let app_icon = icon_from_string(&app_icon)
            .or_else(|| desktop_entry.as_deref().filter(|d| apps::exists(d)).map(apps::icon));

        let id = if replaces != 0 && self.list.borrow().iter().any(|n| n.id == replaces) {
            replaces
        } else {
            let id = self.next_id.get();
            self.next_id.set(id.wrapping_add(1).max(1));
            id
        };
        let app_name = if app_name.is_empty() {
            desktop_entry.as_deref().map(apps::display_name).unwrap_or_else(|| "Application".into())
        } else {
            app_name
        };
        let notif = Notification {
            id,
            app_name,
            app_icon,
            image,
            summary,
            body,
            actions: actions.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect(),
            critical,
            transient: hint_bool("transient"),
            resident: hint_bool("resident"),
            desktop_entry,
            time: glib::DateTime::now_local().unwrap_or_else(|_| glib::DateTime::now_utc().unwrap()),
        };

        {
            let mut list = self.list.borrow_mut();
            list.retain(|n| n.id != id);
            list.push(notif.clone());
        }
        let cfg = self.shell.upgrade().map(|s| s.config()).unwrap_or_default();
        let dnd = cfg.notifications.do_not_disturb;
        if !dnd || critical {
            {
                let mut banners = self.banners.borrow_mut();
                banners.retain(|b| *b != id);
                banners.push(id);
                // Trois bannières au maximum à l'écran.
                let excess = banners.len().saturating_sub(3);
                banners.drain(..excess);
            }
            if cfg.notifications.sound && replaces == 0 {
                play_sound();
            }
            if !critical {
                let secs = if timeout > 0 { (timeout as u64).div_ceil(1000).max(2) } else { cfg.notifications.banner_seconds.max(2) as u64 };
                let n2 = self.clone();
                glib::timeout_add_local_once(Duration::from_secs(secs), move || n2.banner_expired(id));
            }
        }
        self.changed();
        id
    }

    fn banner_expired(self: &Rc<Self>, id: u32) {
        let before = self.banners.borrow().len();
        self.banners.borrow_mut().retain(|b| *b != id);
        if before != self.banners.borrow().len() {
            // Les notifications « transitoires » ne vont pas au centre.
            let transient = self.list.borrow().iter().any(|n| n.id == id && n.transient);
            if transient {
                self.list.borrow_mut().retain(|n| n.id != id);
                self.emit("NotificationClosed", (id, EXPIRED).to_variant());
            }
            self.changed();
        }
    }

    pub fn close(self: &Rc<Self>, id: u32, reason: u32) {
        let existed = self.list.borrow().iter().any(|n| n.id == id);
        self.list.borrow_mut().retain(|n| n.id != id);
        self.banners.borrow_mut().retain(|b| *b != id);
        if existed {
            self.emit("NotificationClosed", (id, reason).to_variant());
            self.changed();
        }
    }

    fn clear_all(self: &Rc<Self>) {
        let ids: Vec<u32> = self.list.borrow().iter().map(|n| n.id).collect();
        for id in ids {
            self.close(id, DISMISSED);
        }
    }

    /// Clic sur une notification ou un de ses boutons.
    fn invoke(self: &Rc<Self>, id: u32, action: &str) {
        let Some(n) = self.list.borrow().iter().find(|n| n.id == id).cloned() else { return };
        if n.actions.iter().any(|(k, _)| k == action) {
            self.emit("ActionInvoked", (id, action).to_variant());
        } else if action == "default" {
            // Pas d'action par défaut : on ouvre l'application.
            if let Some(entry) = n.desktop_entry.as_deref().filter(|d| apps::exists(d)) {
                apps::launch(entry);
            }
        }
        if !n.resident {
            self.close(id, DISMISSED);
        }
        self.hide();
    }

    /// Nombre de notifications au centre et état « ne pas déranger » (pour la cloche).
    pub fn state(&self) -> (usize, bool) {
        (self.list.borrow().iter().filter(|n| !n.transient).count(), self.dnd_button.is_active())
    }

    fn changed(self: &Rc<Self>) {
        self.rebuild_banners();
        self.rebuild_center();
        if let Some(shell) = self.shell.upgrade() {
            shell.refresh_indicators();
        }
    }

    // ───────────── Affichage ─────────────

    fn card(self: &Rc<Self>, n: &Notification, banner: bool) -> gtk::Widget {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("notif-card");
        if banner {
            card.add_css_class("banner");
        }
        if n.critical {
            card.add_css_class("critical");
        }
        card.set_size_request(if banner { 364 } else { -1 }, -1);

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon = match &n.app_icon {
            Some(i) => gtk::Image::from_gicon(i),
            None => gtk::Image::from_icon_name("preferences-system-notifications-symbolic"),
        };
        icon.set_pixel_size(16);
        head.append(&icon);
        let app = gtk::Label::new(Some(&n.app_name));
        app.add_css_class("notif-app");
        app.set_xalign(0.0);
        app.set_hexpand(true);
        app.set_ellipsize(pango::EllipsizeMode::End);
        head.append(&app);
        let time = gtk::Label::new(n.time.format("%H:%M").ok().as_deref());
        time.add_css_class("notif-time");
        head.append(&time);
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.add_css_class("notif-close");
        close.set_tooltip_text(Some(if banner { "Masquer" } else { "Effacer" }));
        {
            let (n2, id) = (self.clone(), n.id);
            close.connect_clicked(move |_| {
                if banner {
                    // Masquer la bannière : la notification reste dans le centre.
                    n2.banners.borrow_mut().retain(|b| *b != id);
                    n2.changed();
                } else {
                    n2.close(id, DISMISSED);
                }
            });
        }
        head.append(&close);
        card.append(&head);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        if let Some(tex) = &n.image {
            let pic = gtk::Image::from_paintable(Some(tex));
            pic.set_pixel_size(48);
            pic.set_valign(gtk::Align::Start);
            pic.add_css_class("notif-image");
            content.append(&pic);
        }
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        let summary = gtk::Label::new(Some(&n.summary));
        summary.add_css_class("notif-summary");
        summary.set_xalign(0.0);
        summary.set_wrap(true);
        summary.set_wrap_mode(pango::WrapMode::WordChar);
        text.append(&summary);
        if !n.body.is_empty() {
            let body = gtk::Label::new(None);
            if pango::parse_markup(&n.body, '\0').is_ok() {
                body.set_markup(&n.body);
            } else {
                body.set_text(&n.body);
            }
            body.add_css_class("notif-body");
            body.set_xalign(0.0);
            body.set_wrap(true);
            body.set_wrap_mode(pango::WrapMode::WordChar);
            body.set_lines(if banner { 3 } else { 6 });
            body.set_ellipsize(pango::EllipsizeMode::End);
            text.append(&body);
        }
        content.append(&text);
        card.append(&content);

        let buttons: Vec<&(String, String)> = n.actions.iter().filter(|(k, _)| k != "default").collect();
        if !buttons.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.set_homogeneous(true);
            for (key, label) in buttons {
                let b = gtk::Button::with_label(label);
                b.add_css_class("notif-action");
                let (n2, id, key) = (self.clone(), n.id, key.clone());
                b.connect_clicked(move |_| n2.invoke(id, &key));
                row.append(&b);
            }
            card.append(&row);
        }

        let click = gtk::GestureClick::new();
        click.set_button(1);
        {
            let (n2, id) = (self.clone(), n.id);
            // Les boutons de la carte réclament leurs propres clics : ce geste n'est
            // déclenché que par un clic sur le reste de la carte.
            click.connect_released(move |_, _, _, _| n2.invoke(id, "default"));
        }
        card.add_controller(click);
        card.upcast()
    }

    fn rebuild_banners(self: &Rc<Self>) {
        while let Some(c) = self.banner_box.first_child() {
            self.banner_box.remove(&c);
        }
        let list = self.list.borrow().clone();
        let ids = self.banners.borrow().clone();
        for id in &ids {
            if let Some(n) = list.iter().find(|n| n.id == *id) {
                self.banner_box.append(&self.card(n, true));
            }
        }
        if ids.is_empty() || self.center.is_visible() {
            self.banner_window.set_visible(false);
        } else {
            if let Some(shell) = self.shell.upgrade() {
                let cfg = shell.config();
                crate::editmode::place_notif(&self.banner_window, &cfg.notifications.position);
                let h = cfg.taskbar.height.clamp(28, 96) as i32;
                // Contre la barre (zone exclusive) + petite marge.
                let e = if cfg.notifications.position.starts_with("top") { Edge::Top } else { Edge::Bottom };
                self.banner_window.set_margin(e, 12.max(h / 4));
            }
            // Taille minimale : la surface reprend sa taille naturelle à chaque changement.
            self.banner_window.set_default_size(1, 1);
            self.banner_window.present();
        }
    }

    /// Nouveau coin des bannières (mode édition).
    pub fn apply_position(&self, corner: &str) {
        crate::editmode::place_notif(&self.banner_window, corner);
    }

    fn rebuild_center(self: &Rc<Self>) {
        while let Some(c) = self.center_list.first_child() {
            self.center_list.remove(&c);
        }
        let list: Vec<Notification> = self.list.borrow().iter().filter(|n| !n.transient).cloned().collect();
        self.empty_label.set_visible(list.is_empty());
        // Regroupées par application, les plus récentes en haut.
        let mut groups: Vec<(String, Vec<Notification>)> = Vec::new();
        for n in list.into_iter().rev() {
            match groups.iter_mut().find(|(app, _)| *app == n.app_name) {
                Some((_, v)) => v.push(n),
                None => groups.push((n.app_name.clone(), vec![n])),
            }
        }
        for (_, items) in groups {
            for n in items {
                self.center_list.append(&self.card(&n, false));
            }
        }
        // Hauteur de la liste : celle du contenu (libellés repliés à la largeur du panneau),
        // jusqu'à 460 px, puis défilement. Le ScrolledWindow sous-estime seul cette hauteur.
        let (_, natural, _, _) = self.center_list.measure(gtk::Orientation::Vertical, 348);
        let empty = if self.empty_label.is_visible() { 70 } else { 0 };
        self.scroll.set_min_content_height((natural + empty).clamp(0, 460));
        self.syncing.set(true);
        if let Some(shell) = self.shell.upgrade() {
            self.dnd_button.set_active(shell.config().notifications.do_not_disturb);
        }
        self.syncing.set(false);
        self.center.set_default_size(1, 1);
    }

    pub fn toggle(self: &Rc<Self>) {
        if let Some(t) = self.hidden_at.get() {
            if t.elapsed().as_millis() < 250 {
                return;
            }
        }
        if self.center.is_visible() {
            self.hide();
        } else {
            if let Ok(now) = glib::DateTime::now_local() {
                let text = now.format("%A %e %B").map(|s| s.to_string()).unwrap_or_default();
                let mut c = text.chars();
                let cap = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                self.date_label.set_text(&cap);
                self.calendar.select_day(&now);
            }
            self.banners.borrow_mut().clear();
            self.rebuild_banners();
            self.rebuild_center();
            self.center.present();
        }
    }

    pub fn hide(&self) {
        if self.center.is_visible() {
            self.center.set_visible(false);
            self.hidden_at.set(Some(Instant::now()));
        }
    }
}

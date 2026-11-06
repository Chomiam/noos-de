//! Zone de notification (icônes des applications actives : Steam, Discord, Nextcloud…).
//!
//! NOOS est à la fois le « watcher » et l'hôte du protocole StatusNotifierItem
//! (`org.kde.StatusNotifierWatcher`), utilisé par les applis Qt, Electron, GTK
//! (libappindicator/ayatana). Les menus passent par `com.canonical.dbusmenu`.

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::Shell;

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
    <method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>"#;

#[derive(Clone)]
pub enum TrayIcon {
    Named(String),
    Pixmap(gdk::Texture),
}

#[derive(Clone)]
pub struct TrayItem {
    /// Clé publiée aux clients : `bus/chemin`.
    pub key: String,
    pub bus: String,
    /// Nom unique (`:1.42`) : c'est lui qui apparaît comme émetteur des signaux.
    pub owner: String,
    pub path: String,
    pub id: String,
    pub title: String,
    pub tooltip: String,
    pub status: String,
    pub icon: Option<TrayIcon>,
    pub menu: Option<String>,
    pub item_is_menu: bool,
}

pub struct Tray {
    shell: Weak<Shell>,
    items: RefCell<Vec<TrayItem>>,
    conn: RefCell<Option<gio::DBusConnection>>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
}

/// Icônes ARGB32 (ordre réseau) → texture RGBA, la plus proche de 32 px.
fn pixmap_texture(pixmaps: Vec<(i32, i32, Vec<u8>)>) -> Option<gdk::Texture> {
    let (w, h, data) = pixmaps
        .into_iter()
        .filter(|(w, h, d)| *w > 0 && *h > 0 && d.len() >= (*w * *h * 4) as usize)
        .min_by_key(|(w, _, _)| (w - 32).abs())?;
    let mut rgba = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4).take((w * h) as usize) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    Some(gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(rgba), w as usize * 4).upcast())
}

fn prop_str(props: &HashMap<String, glib::Variant>, key: &str) -> String {
    props.get(key).and_then(|v| v.get::<String>()).unwrap_or_default()
}

impl Tray {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let tray = Rc::new(Self {
            shell: Rc::downgrade(shell),
            items: RefCell::new(Vec::new()),
            conn: RefCell::new(None),
            subscriptions: RefCell::new(Vec::new()),
        });
        tray.start_dbus();
        tray
    }

    /// Icônes à afficher (les éléments « Passive » sont masqués, comme le prévoit le protocole).
    pub fn items(&self) -> Vec<TrayItem> {
        self.items.borrow().iter().filter(|i| i.status != "Passive").cloned().collect()
    }

    fn start_dbus(self: &Rc<Self>) {
        if !crate::own_session_names() {
            return;
        }
        let this = self.clone();
        let _ = gio::bus_own_name(
            gio::BusType::Session,
            WATCHER,
            gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
            move |conn, _| this.on_bus(conn),
            |_, _| {},
            |_, _| eprintln!("noos-shell : {WATCHER} déjà pris : zone de notification inactive"),
        );
    }

    fn on_bus(self: &Rc<Self>, conn: gio::DBusConnection) {
        let node = gio::DBusNodeInfo::for_xml(XML).expect("XML tray");
        let iface = node.lookup_interface(WATCHER).expect("interface tray");
        let (t1, t2) = (self.clone(), self.clone());
        let res = conn
            .register_object(WATCHER_PATH, &iface)
            .method_call(move |_, sender, _, _, method, params, invocation| {
                if method == "RegisterStatusNotifierItem" {
                    if let (Some((service,)), Some(sender)) = (params.get::<(String,)>(), sender) {
                        t1.register(service, sender.to_string());
                    }
                }
                invocation.return_value(None);
            })
            .property(move |_, _, _, _, name| match name {
                "RegisteredStatusNotifierItems" => {
                    t2.items.borrow().iter().map(|i| i.key.clone()).collect::<Vec<_>>().to_variant()
                }
                "IsStatusNotifierHostRegistered" => true.to_variant(),
                _ => 0i32.to_variant(),
            })
            .build();
        if let Err(e) = res {
            eprintln!("noos-shell : watcher D-Bus : {e}");
            return;
        }
        // Nous sommes aussi l'hôte : on l'annonce pour les applis qui l'attendent.
        let _ = conn.emit_signal(None, WATCHER_PATH, WATCHER, "StatusNotifierHostRegistered", None);
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = gio::bus_own_name_on_connection(&conn, &host_name, gio::BusNameOwnerFlags::NONE, |_, _| {}, |_, _| {});

        // Changements d'icône / d'état / de titre.
        let this = self.clone();
        let sub = conn.subscribe_to_signal(None, Some(ITEM_IFACE), None, None, None, gio::DBusSignalFlags::NONE, move |sig| {
            let (sender, path) = (sig.sender_name.to_string(), sig.object_path.to_string());
            let key = this.items.borrow().iter().find(|i| i.owner == sender && i.path == path).map(|i| i.key.clone());
            if let Some(key) = key {
                this.refresh(key);
            }
        });
        // Appli fermée : son nom disparaît du bus.
        let this = self.clone();
        let sub2 = conn.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |sig| {
                if let Some((name, _old, new)) = sig.parameters.get::<(String, String, String)>() {
                    if new.is_empty() {
                        this.remove_bus(&name);
                    }
                }
            },
        );
        self.subscriptions.borrow_mut().extend([sub, sub2]);
        *self.conn.borrow_mut() = Some(conn);
    }

    fn register(self: &Rc<Self>, service: String, sender: String) {
        // « service » est soit un nom de bus (chemin par défaut), soit un chemin d'objet.
        let (bus, path) = if service.starts_with('/') {
            (sender.clone(), service)
        } else {
            (service, "/StatusNotifierItem".to_string())
        };
        let key = format!("{bus}{path}");
        if self.items.borrow().iter().any(|i| i.key == key) {
            return;
        }
        let owner = if bus.starts_with(':') { bus.clone() } else { sender };
        self.items.borrow_mut().push(TrayItem {
            key: key.clone(),
            bus,
            owner,
            path,
            id: String::new(),
            title: String::new(),
            tooltip: String::new(),
            status: "Active".into(),
            icon: None,
            menu: None,
            item_is_menu: false,
        });
        if let Some(conn) = self.conn.borrow().as_ref() {
            let _ = conn.emit_signal(None, WATCHER_PATH, WATCHER, "StatusNotifierItemRegistered", Some(&(key.as_str(),).to_variant()));
        }
        self.refresh(key);
    }

    fn remove_bus(self: &Rc<Self>, name: &str) {
        let removed: Vec<String> = self
            .items
            .borrow()
            .iter()
            .filter(|i| i.bus == name || i.owner == name)
            .map(|i| i.key.clone())
            .collect();
        if removed.is_empty() {
            return;
        }
        self.items.borrow_mut().retain(|i| !removed.contains(&i.key));
        if let Some(conn) = self.conn.borrow().as_ref() {
            for key in &removed {
                let _ = conn.emit_signal(None, WATCHER_PATH, WATCHER, "StatusNotifierItemUnregistered", Some(&(key.as_str(),).to_variant()));
            }
        }
        self.changed();
    }

    /// Relit toutes les propriétés d'un élément.
    fn refresh(self: &Rc<Self>, key: String) {
        let Some(conn) = self.conn.borrow().clone() else { return };
        let Some(item) = self.items.borrow().iter().find(|i| i.key == key).cloned() else { return };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let reply = conn
                .call_future(
                    Some(&item.bus),
                    &item.path,
                    "org.freedesktop.DBus.Properties",
                    "GetAll",
                    Some(&(ITEM_IFACE,).to_variant()),
                    Some(glib::VariantTy::new("(a{sv})").unwrap()),
                    gio::DBusCallFlags::NONE,
                    3000,
                )
                .await;
            let Ok(reply) = reply else { return };
            let Some(props) = reply.child_value(0).get::<HashMap<String, glib::Variant>>() else { return };

            let theme_path = prop_str(&props, "IconThemePath");
            if !theme_path.is_empty() {
                let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
                if !theme.search_path().iter().any(|p| p.to_string_lossy() == theme_path) {
                    theme.add_search_path(&theme_path);
                }
            }
            let status = prop_str(&props, "Status");
            let (name_key, pix_key) = if status == "NeedsAttention" {
                ("AttentionIconName", "AttentionIconPixmap")
            } else {
                ("IconName", "IconPixmap")
            };
            let icon_name = prop_str(&props, name_key);
            let icon = if !icon_name.is_empty() {
                if icon_name.starts_with('/') {
                    gdk::Texture::from_filename(&icon_name).ok().map(TrayIcon::Pixmap)
                } else {
                    Some(TrayIcon::Named(icon_name))
                }
            } else {
                props.get(pix_key).and_then(|v| v.get::<Vec<(i32, i32, Vec<u8>)>>()).and_then(pixmap_texture).map(TrayIcon::Pixmap)
            };
            let tooltip = props
                .get("ToolTip")
                .and_then(|v| v.get::<(String, Vec<(i32, i32, Vec<u8>)>, String, String)>())
                .map(|t| if t.2.is_empty() { t.3 } else { t.2 })
                .unwrap_or_default();
            let menu = props
                .get("Menu")
                .and_then(|v| v.str().map(|s| s.to_string()).or_else(|| v.get::<String>()))
                .filter(|m| !m.is_empty() && m != "/");
            let mut owner = None;
            if !item.bus.starts_with(':') {
                // Nom unique, pour reconnaître l'émetteur des signaux.
                if let Ok(r) = conn
                    .call_future(
                        Some("org.freedesktop.DBus"),
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "GetNameOwner",
                        Some(&(item.bus.as_str(),).to_variant()),
                        Some(glib::VariantTy::new("(s)").unwrap()),
                        gio::DBusCallFlags::NONE,
                        1000,
                    )
                    .await
                {
                    owner = r.get::<(String,)>().map(|o| o.0);
                }
            }

            if let Some(it) = this.items.borrow_mut().iter_mut().find(|i| i.key == key) {
                it.id = prop_str(&props, "Id");
                it.title = prop_str(&props, "Title");
                it.status = if status.is_empty() { "Active".into() } else { status };
                it.icon = icon;
                it.tooltip = tooltip;
                it.menu = menu;
                it.item_is_menu = props.get("ItemIsMenu").and_then(|v| v.get::<bool>()).unwrap_or(false);
                if let Some(o) = owner {
                    it.owner = o;
                }
            }
            this.changed();
        });
    }

    fn changed(&self) {
        if let Some(shell) = self.shell.upgrade() {
            shell.refresh_indicators();
        }
    }

    fn call(&self, item: &TrayItem, method: &str, args: glib::Variant) -> impl std::future::Future<Output = bool> + 'static {
        let conn = self.conn.borrow().clone();
        let (bus, path, method) = (item.bus.clone(), item.path.clone(), method.to_string());
        async move {
            let Some(conn) = conn else { return false };
            conn.call_future(Some(&bus), &path, ITEM_IFACE, &method, Some(&args), None, gio::DBusCallFlags::NONE, 2000)
                .await
                .is_ok()
        }
    }

    /// Bouton d'icône pour la barre ou le menu « ^ ». `on_done` : appelé après une action
    /// (pour refermer le menu « ^ »).
    pub fn button(self: &Rc<Self>, item: &TrayItem, size: i32, on_done: Rc<dyn Fn()>) -> gtk::Button {
        let button = gtk::Button::new();
        button.add_css_class("tray-icon");
        let image = match &item.icon {
            Some(TrayIcon::Named(name)) => gtk::Image::from_icon_name(name),
            Some(TrayIcon::Pixmap(tex)) => gtk::Image::from_paintable(Some(tex)),
            None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
        };
        image.set_pixel_size(size);
        button.set_child(Some(&image));
        let tip = [&item.tooltip, &item.title, &item.id].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_default();
        if !tip.is_empty() {
            button.set_tooltip_text(Some(&tip));
        }

        // Clic gauche : activer (ou menu pour les éléments « menu seulement »).
        {
            let (tray, item, done) = (self.clone(), item.clone(), on_done.clone());
            button.connect_clicked(move |b| {
                let (tray, item, b, done) = (tray.clone(), item.clone(), b.clone(), done.clone());
                glib::spawn_future_local(async move {
                    if item.item_is_menu || !tray.call(&item, "Activate", (0i32, 0i32).to_variant()).await {
                        tray.show_menu(&item, &b).await;
                    } else {
                        done();
                    }
                });
            });
        }
        // Clic droit : menu contextuel.
        let right = gtk::GestureClick::new();
        right.set_button(3);
        {
            let (tray, item, button) = (self.clone(), item.clone(), button.clone());
            right.connect_pressed(move |_, _, _, _| {
                let (tray, item, button) = (tray.clone(), item.clone(), button.clone());
                glib::spawn_future_local(async move { tray.show_menu(&item, &button).await });
            });
        }
        button.add_controller(right);
        // Clic du milieu : action secondaire.
        let middle = gtk::GestureClick::new();
        middle.set_button(2);
        {
            let (tray, item) = (self.clone(), item.clone());
            middle.connect_pressed(move |_, _, _, _| {
                let fut = tray.call(&item, "SecondaryActivate", (0i32, 0i32).to_variant());
                glib::spawn_future_local(async move {
                    fut.await;
                });
            });
        }
        button.add_controller(middle);
        button
    }

    async fn show_menu(self: &Rc<Self>, item: &TrayItem, anchor: &gtk::Button) {
        let Some(conn) = self.conn.borrow().clone() else { return };
        let Some(menu_path) = item.menu.clone() else {
            // Pas de dbusmenu : l'appli affiche son propre menu.
            self.call(item, "ContextMenu", (0i32, 0i32).to_variant()).await;
            return;
        };
        let _ = conn
            .call_future(Some(&item.bus), &menu_path, MENU_IFACE, "AboutToShow", Some(&(0i32,).to_variant()), None, gio::DBusCallFlags::NONE, 1000)
            .await;
        let layout = conn
            .call_future(
                Some(&item.bus),
                &menu_path,
                MENU_IFACE,
                "GetLayout",
                Some(&(0i32, -1i32, Vec::<String>::new()).to_variant()),
                Some(glib::VariantTy::new("(u(ia{sv}av))").unwrap()),
                gio::DBusCallFlags::NONE,
                2000,
            )
            .await;
        let Ok(layout) = layout else { return };
        let root = parse_menu(&layout.child_value(1));

        let popover = gtk::Popover::new();
        popover.add_css_class("tray-menu");
        popover.set_has_arrow(false);
        popover.set_parent(anchor);
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let activate: Rc<dyn Fn(i32)> = {
            let (conn, bus, path, popover) = (conn.clone(), item.bus.clone(), menu_path.clone(), popover.clone());
            Rc::new(move |id| {
                popover.popdown();
                let args = (id, "clicked", 0i32.to_variant(), 0u32).to_variant();
                let (conn, bus, path) = (conn.clone(), bus.clone(), path.clone());
                glib::spawn_future_local(async move {
                    let _ = conn.call_future(Some(&bus), &path, MENU_IFACE, "Event", Some(&args), None, gio::DBusCallFlags::NONE, 2000).await;
                });
            })
        };
        build_menu(&vbox, &root.children, 0, &activate);
        popover.set_child(Some(&vbox));
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
    }
}

struct MenuNode {
    id: i32,
    label: String,
    enabled: bool,
    visible: bool,
    separator: bool,
    /// Case à cocher / bouton radio : état coché.
    toggle: Option<bool>,
    children: Vec<MenuNode>,
}

fn parse_menu(v: &glib::Variant) -> MenuNode {
    let id = v.try_child_value(0).and_then(|c| c.get::<i32>()).unwrap_or(0);
    let props = v.try_child_value(1).and_then(|c| c.get::<HashMap<String, glib::Variant>>()).unwrap_or_default();
    let mut children = Vec::new();
    if let Some(list) = v.try_child_value(2) {
        for i in 0..list.n_children() {
            if let Some(inner) = list.child_value(i).as_variant() {
                children.push(parse_menu(&inner));
            }
        }
    }
    let label = prop_str(&props, "label").replace("__", "\u{1}").replace('_', "").replace('\u{1}', "_");
    MenuNode {
        id,
        label,
        enabled: props.get("enabled").and_then(|v| v.get::<bool>()).unwrap_or(true),
        visible: props.get("visible").and_then(|v| v.get::<bool>()).unwrap_or(true),
        separator: prop_str(&props, "type") == "separator",
        toggle: (!prop_str(&props, "toggle-type").is_empty())
            .then(|| props.get("toggle-state").and_then(|v| v.get::<i32>()).unwrap_or(0) == 1),
        children,
    }
}

fn build_menu(vbox: &gtk::Box, nodes: &[MenuNode], depth: i32, activate: &Rc<dyn Fn(i32)>) {
    for node in nodes.iter().filter(|n| n.visible) {
        if node.separator {
            let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
            sep.set_margin_top(4);
            sep.set_margin_bottom(4);
            vbox.append(&sep);
            continue;
        }
        if !node.children.is_empty() {
            // Sous-menu : titre puis éléments en retrait.
            let title = gtk::Label::new(Some(&node.label));
            title.add_css_class("dim-label");
            title.set_xalign(0.0);
            title.set_margin_start(10 + depth * 12);
            title.set_margin_top(4);
            vbox.append(&title);
            build_menu(vbox, &node.children, depth + 1, activate);
            continue;
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.set_opacity(if node.toggle == Some(true) { 1.0 } else { 0.0 });
        if node.toggle.is_some() || depth > 0 {
            row.append(&check);
        }
        let label = gtk::Label::new(Some(&node.label));
        label.set_xalign(0.0);
        row.append(&label);
        let b = gtk::Button::new();
        b.add_css_class("flat");
        b.set_child(Some(&row));
        b.set_sensitive(node.enabled);
        b.set_margin_start(depth * 12);
        let (activate, id) = (activate.clone(), node.id);
        b.connect_clicked(move |_| activate(id));
        vbox.append(&b);
    }
}

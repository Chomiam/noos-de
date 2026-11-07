//! Réseau (bouton de la barre) : état Ethernet / Wi-Fi, recherche et connexion Wi-Fi
//! (NetworkManager via `nmcli`), Bluetooth (BlueZ via `bluetoothctl`) avec recherche,
//! appairage et niveau de batterie des appareils.

use std::{
    cell::{Cell, RefCell},
    process::Command,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use adap::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::Shell;

// ───────────────────────────── Données ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Link {
    Wired(String),
    Wifi { ssid: String, signal: u8 },
    #[default]
    None,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetStatus {
    pub link: Link,
    pub has_wifi: bool,
    pub wifi_enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AccessPoint {
    pub ssid: String,
    pub signal: u8,
    pub secure: bool,
    pub in_use: bool,
    pub known: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub icon: String,
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtStatus {
    pub available: bool,
    pub powered: bool,
    pub devices: Vec<BtDevice>,
}

/// Découpe une ligne `nmcli -t` (champs séparés par « : », avec « \: » et « \\ » échappés).
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    fields.last_mut().unwrap().push(n);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().unwrap().push(c),
        }
    }
    fields
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `bluetoothctl` attend indéfiniment si le service bluetoothd est absent : délai maximal.
fn btctl(args: &[&str], secs: u32) -> Option<String> {
    let mut full = vec![secs.to_string(), "bluetoothctl".to_string()];
    full.extend(args.iter().map(|a| a.to_string()));
    let out = Command::new("timeout").args(&full).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

pub fn parse_status(devices: &str, radio: &str, wifi_list: &str) -> NetStatus {
    let mut st = NetStatus { wifi_enabled: radio.trim() == "enabled", ..Default::default() };
    let mut wifi_conn = None;
    for line in devices.lines() {
        let f = split_terse(line);
        if f.len() < 3 {
            continue;
        }
        match (f[0].as_str(), f[1].as_str()) {
            ("wifi", state) => {
                st.has_wifi = true;
                if state == "connected" {
                    wifi_conn = Some(f[2].clone());
                }
            }
            ("ethernet", "connected") if !matches!(st.link, Link::Wired(_)) => st.link = Link::Wired(f[2].clone()),
            _ => {}
        }
    }
    if !matches!(st.link, Link::Wired(_)) {
        if let Some(conn) = wifi_conn {
            let (ssid, signal) = parse_wifi_list(wifi_list, "")
                .into_iter()
                .find(|a| a.in_use)
                .map(|a| (a.ssid, a.signal))
                .unwrap_or((conn, 70));
            st.link = Link::Wifi { ssid, signal };
        }
    }
    st
}

/// `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list` ; `known` : connexions Wi-Fi
/// enregistrées (`nmcli -t -f NAME,TYPE connection show`).
pub fn parse_wifi_list(list: &str, known: &str) -> Vec<AccessPoint> {
    let known: Vec<String> = known
        .lines()
        .map(split_terse)
        .filter(|f| f.len() >= 2 && f[1].contains("wireless"))
        .map(|f| f[0].clone())
        .collect();
    let mut aps: Vec<AccessPoint> = Vec::new();
    for line in list.lines() {
        let f = split_terse(line);
        if f.len() < 4 || f[1].is_empty() {
            continue;
        }
        let ap = AccessPoint {
            in_use: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            secure: !f[3].is_empty() && f[3] != "--",
            known: known.contains(&f[1]),
        };
        match aps.iter_mut().find(|a| a.ssid == ap.ssid) {
            Some(existing) => {
                existing.in_use |= ap.in_use;
                existing.signal = existing.signal.max(ap.signal);
            }
            None => aps.push(ap),
        }
    }
    aps.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    aps
}

/// `bluetoothctl info <MAC>`.
pub fn parse_bt_info(mac: &str, info: &str) -> BtDevice {
    let mut d = BtDevice { mac: mac.to_string(), ..Default::default() };
    for line in info.lines().map(str::trim) {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            match k {
                "Name" => d.name = v.to_string(),
                "Alias" if d.name.is_empty() => d.name = v.to_string(),
                "Icon" => d.icon = v.to_string(),
                "Paired" => d.paired = v == "yes",
                "Connected" => d.connected = v == "yes",
                // « Battery Percentage: 0x4b (75) »
                "Battery Percentage" => {
                    d.battery = v.split('(').nth(1).and_then(|s| s.trim_end_matches(')').parse().ok())
                }
                _ => {}
            }
        }
    }
    if d.name.is_empty() {
        d.name = mac.to_string();
    }
    d
}

/// Lignes « Device AA:BB:CC:DD:EE:FF Nom » de `bluetoothctl devices`.
pub fn parse_bt_devices(list: &str) -> Vec<(String, String)> {
    list.lines()
        .filter_map(|l| l.strip_prefix("Device "))
        .filter_map(|l| l.split_once(' ').map(|(m, n)| (m.to_string(), n.to_string())))
        .collect()
}

pub fn read_status() -> NetStatus {
    let devices = run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).unwrap_or_default();
    let radio = run("nmcli", &["-t", "radio", "wifi"]).unwrap_or_default();
    let list = if devices.contains("wifi:connected") {
        run("nmcli", &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "no"]).unwrap_or_default()
    } else {
        String::new()
    };
    parse_status(&devices, &radio, &list)
}

/// Profil VPN (WireGuard ou greffon VPN de NetworkManager).
#[derive(Clone, Debug, PartialEq)]
pub struct Vpn {
    pub name: String,
    pub uuid: String,
    pub active: bool,
}

pub fn parse_vpns(text: &str) -> Vec<Vpn> {
    text.lines()
        .map(split_terse)
        .filter(|f| f.len() >= 4 && (f[2] == "wireguard" || f[2] == "vpn"))
        .map(|f| Vpn { name: f[0].clone(), uuid: f[1].clone(), active: f[3] == "yes" })
        .collect()
}

pub fn read_vpns() -> Vec<Vpn> {
    if std::env::var_os("NOOS_VPN_DEMO").is_some() {
        return parse_vpns("Proton Suisse:a:wireguard:yes\nMullvad Paris:b:wireguard:no\nBureau:c:vpn:no\n");
    }
    parse_vpns(&run("nmcli", &["-t", "-f", "NAME,UUID,TYPE,ACTIVE", "connection", "show"]).unwrap_or_default())
}

/// Active le profil `uuid` (en coupant les autres VPN), ou coupe tout si `None`.
pub fn switch_vpn(uuid: Option<&str>) -> Result<(), String> {
    for v in read_vpns().into_iter().filter(|v| v.active && Some(v.uuid.as_str()) != uuid) {
        let _ = run("nmcli", &["connection", "down", &v.uuid]);
    }
    if let Some(u) = uuid {
        let out = Command::new("nmcli").args(["connection", "up", u]).output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().trim_start_matches("Error: ").to_string());
        }
    }
    Ok(())
}

pub fn scan_wifi(rescan: bool) -> Vec<AccessPoint> {
    let list = run(
        "nmcli",
        &["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", if rescan { "yes" } else { "auto" }],
    )
    .unwrap_or_default();
    let known = run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"]).unwrap_or_default();
    parse_wifi_list(&list, &known)
}

pub fn read_bt() -> BtStatus {
    let Some(show) = btctl(&["show"], 3) else { return BtStatus::default() };
    let mut st = BtStatus { available: show.contains("Controller"), powered: show.contains("Powered: yes"), devices: vec![] };
    if !st.available {
        return st;
    }
    for (mac, _) in parse_bt_devices(&btctl(&["devices"], 3).unwrap_or_default()) {
        if let Some(info) = btctl(&["info", &mac], 3) {
            st.devices.push(parse_bt_info(&mac, &info));
        }
    }
    st.devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)).then(a.name.cmp(&b.name)));
    st
}

fn signal_icon(signal: u8) -> &'static str {
    match signal {
        75.. => "network-wireless-signal-excellent-symbolic",
        50..=74 => "network-wireless-signal-good-symbolic",
        25..=49 => "network-wireless-signal-ok-symbolic",
        _ => "network-wireless-signal-weak-symbolic",
    }
}

/// Icône de la barre des tâches.
pub fn status_icon(st: &NetStatus) -> &'static str {
    match &st.link {
        Link::Wired(_) => "network-wired-symbolic",
        Link::Wifi { signal, .. } => signal_icon(*signal),
        Link::None if st.has_wifi && st.wifi_enabled => "network-wireless-offline-symbolic",
        Link::None => "network-offline-symbolic",
    }
}

pub fn status_tooltip(st: &NetStatus) -> String {
    match &st.link {
        Link::Wired(c) => format!("Ethernet : {c}\nAccès Internet"),
        Link::Wifi { ssid, signal } => format!("Wi-Fi : {ssid} ({signal} %)"),
        Link::None => "Non connecté".into(),
    }
}

fn battery_icon(level: u8) -> &'static str {
    match level {
        90.. => "battery-level-100-symbolic",
        70..=89 => "battery-level-80-symbolic",
        50..=69 => "battery-level-60-symbolic",
        30..=49 => "battery-level-40-symbolic",
        15..=29 => "battery-level-20-symbolic",
        _ => "battery-level-10-symbolic",
    }
}

/// Exécute une commande dans un thread puis appelle `done` sur le thread GTK.
fn background<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(work());
    });
    glib::spawn_future_local(async move {
        if let Ok(v) = rx.recv().await {
            done(v);
        }
    });
}

// ───────────────────────────── Panneau ─────────────────────────────

pub struct NetFlyout {
    shell: Weak<Shell>,
    window: gtk::Window,
    status_icon: gtk::Image,
    status_title: gtk::Label,
    status_sub: gtk::Label,
    wifi_box: gtk::Box,
    wifi_switch: gtk::Switch,
    wifi_list: gtk::ListBox,
    wifi_spinner: gtk::Spinner,
    bt_box: gtk::Box,
    bt_switch: gtk::Switch,
    bt_list: gtk::ListBox,
    bt_spinner: gtk::Spinner,
    bt_scan: gtk::Button,
    syncing: Cell<bool>,
    hidden_at: Cell<Option<Instant>>,
    /// Réseau dont on saisit le mot de passe.
    pending_ssid: RefCell<Option<String>>,
    hw_note: gtk::Label,
    /// Matériel détecté : (Wi-Fi, Bluetooth) ; None = pas encore connu.
    hardware: Cell<(Option<bool>, Option<bool>)>,
}

fn header(title: &str, switch: &gtk::Switch, extra: &[&gtk::Widget]) -> gtk::Box {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-title");
    l.set_xalign(0.0);
    l.set_hexpand(true);
    h.append(&l);
    for w in extra {
        h.append(*w);
    }
    switch.set_valign(gtk::Align::Center);
    h.append(switch);
    h
}

impl NetFlyout {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-quicksettings"));
        window.set_layer(Layer::Top);
        window.set_anchor(Edge::Bottom, true);
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Bottom, 12);
        window.set_margin(Edge::Right, 12);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("noos-flyout");
        root.add_css_class("quick");
        root.add_css_class("net-flyout");
        root.set_size_request(400, -1);

        // État actuel.
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        status.add_css_class("net-status");
        let status_icon = gtk::Image::from_icon_name("network-wired-symbolic");
        status_icon.set_pixel_size(32);
        status.append(&status_icon);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_valign(gtk::Align::Center);
        let status_title = gtk::Label::new(None);
        status_title.add_css_class("heading");
        status_title.set_xalign(0.0);
        let status_sub = gtk::Label::new(None);
        status_sub.add_css_class("dim-label");
        status_sub.set_xalign(0.0);
        col.append(&status_title);
        col.append(&status_sub);
        status.append(&col);
        root.append(&status);

        // Wi-Fi.
        let wifi_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let wifi_switch = gtk::Switch::new();
        let wifi_spinner = gtk::Spinner::new();
        let wifi_refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        wifi_refresh.add_css_class("flat");
        wifi_refresh.set_tooltip_text(Some("Rechercher les réseaux"));
        wifi_box.append(&header("Wi-Fi", &wifi_switch, &[wifi_spinner.upcast_ref(), wifi_refresh.upcast_ref()]));
        let wifi_list = gtk::ListBox::new();
        wifi_list.add_css_class("boxed-list");
        wifi_list.set_selection_mode(gtk::SelectionMode::None);
        let wifi_scroll = gtk::ScrolledWindow::new();
        wifi_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        wifi_scroll.set_propagate_natural_height(true);
        wifi_scroll.set_max_content_height(260);
        wifi_scroll.set_child(Some(&wifi_list));
        wifi_box.append(&wifi_scroll);
        root.append(&wifi_box);

        // Bluetooth.
        let bt_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let bt_switch = gtk::Switch::new();
        let bt_spinner = gtk::Spinner::new();
        let bt_scan = gtk::Button::with_label("Rechercher");
        bt_scan.add_css_class("flat");
        bt_scan.set_tooltip_text(Some("Rechercher des appareils Bluetooth à proximité"));
        bt_box.append(&header("Bluetooth", &bt_switch, &[bt_spinner.upcast_ref(), bt_scan.upcast_ref()]));
        let bt_list = gtk::ListBox::new();
        bt_list.add_css_class("boxed-list");
        bt_list.set_selection_mode(gtk::SelectionMode::None);
        let bt_scroll = gtk::ScrolledWindow::new();
        bt_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        bt_scroll.set_propagate_natural_height(true);
        bt_scroll.set_max_content_height(240);
        bt_scroll.set_child(Some(&bt_list));
        bt_box.append(&bt_scroll);
        root.append(&bt_box);
        // Sections affichées seulement une fois le matériel détecté.
        wifi_box.set_visible(false);
        bt_box.set_visible(false);
        let hw_note = gtk::Label::new(Some("Aucun adaptateur Wi-Fi ni Bluetooth détecté sur ce PC"));
        hw_note.add_css_class("dim-label");
        hw_note.set_xalign(0.0);
        hw_note.set_wrap(true);
        hw_note.set_visible(false);
        hw_note.set_margin_bottom(14);
        root.append(&hw_note);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("footer");
        let settings = gtk::Button::with_label("Paramètres réseau avancés");
        settings.add_css_class("flat");
        settings.connect_clicked(|_| {
            let _ = Command::new("nm-connection-editor").spawn();
        });
        footer.append(&settings);
        footer.set_visible(glib::find_program_in_path("nm-connection-editor").is_some());
        root.append(&footer);
        window.set_child(Some(&root));

        let f = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            status_icon,
            status_title,
            status_sub,
            wifi_box,
            wifi_switch,
            wifi_list,
            wifi_spinner,
            bt_box,
            bt_switch,
            bt_list,
            bt_spinner,
            bt_scan,
            syncing: Cell::new(false),
            hidden_at: Cell::new(None),
            pending_ssid: RefCell::new(None),
            hw_note,
            hardware: Cell::new((None, None)),
        });

        {
            let w = Rc::downgrade(&f);
            wifi_refresh.connect_clicked(move |_| {
                if let Some(f) = w.upgrade() {
                    f.load_wifi(true);
                }
            });
        }
        {
            let w = Rc::downgrade(&f);
            f.wifi_switch.connect_active_notify(move |s| {
                let Some(f) = w.upgrade() else { return };
                if f.syncing.get() {
                    return;
                }
                let on = s.is_active();
                let w = Rc::downgrade(&f);
                background(
                    move || run("nmcli", &["radio", "wifi", if on { "on" } else { "off" }]),
                    move |_| {
                        if let Some(f) = w.upgrade() {
                            f.refresh();
                        }
                    },
                );
            });
        }
        {
            let w = Rc::downgrade(&f);
            f.bt_switch.connect_active_notify(move |s| {
                let Some(f) = w.upgrade() else { return };
                if f.syncing.get() {
                    return;
                }
                let on = s.is_active();
                let w = Rc::downgrade(&f);
                background(
                    move || btctl(&["power", if on { "on" } else { "off" }], 5),
                    move |_| {
                        if let Some(f) = w.upgrade() {
                            f.load_bt();
                        }
                    },
                );
            });
        }
        {
            let w = Rc::downgrade(&f);
            f.bt_scan.connect_clicked(move |b| {
                let Some(f) = w.upgrade() else { return };
                b.set_sensitive(false);
                f.bt_spinner.start();
                let w = Rc::downgrade(&f);
                background(
                    // La recherche dure 10 s ; les appareils trouvés restent connus de BlueZ.
                    || btctl(&["--timeout", "10", "scan", "on"], 13),
                    move |_| {
                        if let Some(f) = w.upgrade() {
                            f.bt_scan.set_sensitive(true);
                            f.load_bt();
                        }
                    },
                );
            });
        }
        {
            let f2 = f.clone();
            crate::close_on_focus_loss(&f.window, move || f2.hide());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let f2 = f.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    f2.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            f.window.add_controller(keys);
        }
        f
    }

    fn refresh(self: &Rc<Self>) {
        let w = Rc::downgrade(self);
        background(read_status, move |st| {
            let Some(f) = w.upgrade() else { return };
            f.apply_status(&st);
            if st.has_wifi && st.wifi_enabled {
                f.load_wifi(false);
            }
            if let Some(shell) = f.shell.upgrade() {
                shell.refresh_status_soon();
            }
        });
        self.load_bt();
    }

    fn apply_status(&self, st: &NetStatus) {
        self.status_icon.set_icon_name(Some(status_icon(st)));
        let (title, sub) = match &st.link {
            Link::Wired(c) => ("Ethernet".to_string(), format!("{c} · Connecté")),
            Link::Wifi { ssid, signal } => (ssid.clone(), format!("Wi-Fi · Connecté · {signal} %")),
            Link::None => ("Non connecté".to_string(), "Aucune connexion réseau".to_string()),
        };
        self.status_title.set_text(&title);
        self.status_sub.set_text(&sub);
        self.syncing.set(true);
        self.wifi_switch.set_active(st.wifi_enabled);
        self.syncing.set(false);
        self.wifi_box.set_visible(st.has_wifi);
        self.wifi_list.set_visible(st.wifi_enabled);
        let (_, bt) = self.hardware.get();
        self.hardware.set((Some(st.has_wifi), bt));
        self.update_note();
    }

    fn load_wifi(self: &Rc<Self>, rescan: bool) {
        self.wifi_spinner.start();
        let w = Rc::downgrade(self);
        background(
            move || scan_wifi(rescan),
            move |aps| {
                if let Some(f) = w.upgrade() {
                    f.wifi_spinner.stop();
                    f.fill_wifi(&aps);
                }
            },
        );
    }

    fn fill_wifi(self: &Rc<Self>, aps: &[AccessPoint]) {
        self.wifi_list.remove_all();
        if aps.is_empty() {
            let row = adap::ActionRow::new();
            row.set_title("Aucun réseau trouvé");
            self.wifi_list.append(&row);
        }
        for ap in aps {
            let row = adap::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&ap.ssid));
            let mut sub = Vec::new();
            if ap.in_use {
                sub.push("Connecté");
            } else if ap.known {
                sub.push("Enregistré");
            }
            sub.push(if ap.secure { "Sécurisé" } else { "Ouvert" });
            row.set_subtitle(&sub.join(" · "));
            let icon = gtk::Image::from_icon_name(signal_icon(ap.signal));
            row.add_prefix(&icon);
            if ap.secure {
                row.add_suffix(&gtk::Image::from_icon_name("channel-secure-symbolic"));
            }
            let button = gtk::Button::with_label(if ap.in_use { "Déconnecter" } else { "Se connecter" });
            button.set_valign(gtk::Align::Center);
            if !ap.in_use {
                button.add_css_class("suggested-action");
            }
            row.add_suffix(&button);
            let (w, target) = (Rc::downgrade(self), ap.clone());
            button.connect_clicked(move |b| {
                let Some(f) = w.upgrade() else { return };
                if target.in_use {
                    f.wifi_action(b, vec!["connection".into(), "down".into(), "id".into(), target.ssid.clone()]);
                } else if target.known || !target.secure {
                    f.wifi_action(b, vec!["device".into(), "wifi".into(), "connect".into(), target.ssid.clone()]);
                } else {
                    f.ask_password(&target.ssid);
                }
            });
            self.wifi_list.append(&row);
            if self.pending_ssid.borrow().as_deref() == Some(ap.ssid.as_str()) {
                self.wifi_list.append(&self.password_row(&ap.ssid));
            }
        }
    }

    fn password_row(self: &Rc<Self>, ssid: &str) -> gtk::Widget {
        let row = adap::PasswordEntryRow::new();
        row.set_title(&format!("Clé de sécurité de « {} »", glib::markup_escape_text(ssid)));
        row.set_show_apply_button(true);
        let (w, ssid) = (Rc::downgrade(self), ssid.to_string());
        row.connect_apply(move |r| {
            let Some(f) = w.upgrade() else { return };
            let password = r.text().to_string();
            *f.pending_ssid.borrow_mut() = None;
            r.set_sensitive(false);
            f.wifi_spinner.start();
            let (ssid, w) = (ssid.clone(), Rc::downgrade(&f));
            background(
                move || {
                    Command::new("nmcli")
                        .args(["device", "wifi", "connect", &ssid, "password", &password])
                        .output()
                        .map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).to_string()))
                        .unwrap_or((false, "nmcli introuvable".into()))
                },
                move |(ok, err)| {
                    if let Some(f) = w.upgrade() {
                        f.wifi_spinner.stop();
                        if !ok {
                            f.status_sub.set_text(&format!("Échec : {}", err.lines().last().unwrap_or("connexion refusée")));
                        }
                        f.refresh();
                    }
                },
            );
        });
        row.upcast()
    }

    fn ask_password(self: &Rc<Self>, ssid: &str) {
        *self.pending_ssid.borrow_mut() = Some(ssid.to_string());
        self.load_wifi(false);
    }

    fn wifi_action(self: &Rc<Self>, button: &gtk::Button, args: Vec<String>) {
        button.set_sensitive(false);
        self.wifi_spinner.start();
        let w = Rc::downgrade(self);
        background(
            move || Command::new("nmcli").args(&args).status().map(|s| s.success()).unwrap_or(false),
            move |_| {
                if let Some(f) = w.upgrade() {
                    f.wifi_spinner.stop();
                    f.refresh();
                }
            },
        );
    }

    fn load_bt(self: &Rc<Self>) {
        self.bt_spinner.start();
        let w = Rc::downgrade(self);
        background(read_bt, move |st| {
            if let Some(f) = w.upgrade() {
                f.bt_spinner.stop();
                f.fill_bt(&st);
            }
        });
    }

    fn update_note(&self) {
        self.hw_note.set_visible(self.hardware.get() == (Some(false), Some(false)));
    }

    fn fill_bt(self: &Rc<Self>, st: &BtStatus) {
        self.bt_box.set_visible(st.available);
        let (wifi, _) = self.hardware.get();
        self.hardware.set((wifi, Some(st.available)));
        self.update_note();
        self.syncing.set(true);
        self.bt_switch.set_active(st.powered);
        self.syncing.set(false);
        self.bt_scan.set_sensitive(st.powered);
        self.bt_list.set_visible(st.powered);
        self.bt_list.remove_all();
        if st.powered && st.devices.is_empty() {
            let row = adap::ActionRow::new();
            row.set_title("Aucun appareil : lancez une recherche");
            self.bt_list.append(&row);
        }
        for d in &st.devices {
            let row = adap::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&d.name));
            let status = if d.connected {
                "Connecté"
            } else if d.paired {
                "Appairé"
            } else {
                "Disponible"
            };
            row.set_subtitle(status);
            let icon = if d.icon.is_empty() { "bluetooth-symbolic".to_string() } else { format!("{}-symbolic", d.icon) };
            row.add_prefix(&gtk::Image::from_icon_name(&icon));
            if let Some(level) = d.battery {
                let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                b.append(&gtk::Image::from_icon_name(battery_icon(level)));
                let l = gtk::Label::new(Some(&format!("{level} %")));
                l.add_css_class("numeric");
                b.append(&l);
                b.set_tooltip_text(Some("Batterie de l'appareil"));
                row.add_suffix(&b);
            }
            let label = if d.connected {
                "Déconnecter"
            } else if d.paired {
                "Connecter"
            } else {
                "Associer"
            };
            let button = gtk::Button::with_label(label);
            button.set_valign(gtk::Align::Center);
            row.add_suffix(&button);
            let (w, d) = (Rc::downgrade(self), d.clone());
            button.connect_clicked(move |b| {
                let Some(f) = w.upgrade() else { return };
                b.set_sensitive(false);
                f.bt_spinner.start();
                let d = d.clone();
                let w = Rc::downgrade(&f);
                background(
                    move || {
                        if d.connected {
                            btctl(&["disconnect", &d.mac], 10);
                        } else {
                            if !d.paired {
                                btctl(&["pair", &d.mac], 30);
                                btctl(&["trust", &d.mac], 5);
                            }
                            btctl(&["connect", &d.mac], 20);
                        }
                    },
                    move |_| {
                        if let Some(f) = w.upgrade() {
                            f.load_bt();
                        }
                    },
                );
            });
            self.bt_list.append(&row);
        }
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.hidden_at.get().is_some_and(|t| t.elapsed().as_millis() < 250) {
            return;
        }
        if self.window.is_visible() {
            self.hide();
        } else {
            self.refresh();
            self.window.set_default_size(1, 1);
            self.window.present();
        }
    }

    pub fn hide(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
            self.hidden_at.set(Some(Instant::now()));
            *self.pending_ssid.borrow_mut() = None;
        }
    }
}

/// Rafraîchissement périodique (icônes de la barre).
pub const POLL: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terse_escapes() {
        assert_eq!(split_terse(r"*:Box\:Maison:72:WPA2"), vec!["*", "Box:Maison", "72", "WPA2"]);
    }

    #[test]
    fn wired_wins() {
        let st = parse_status("ethernet:connected:Connexion filaire 1\nwifi:connected:Livebox\nloopback:connected (externally):lo", "enabled", "*:Livebox:80:WPA2");
        assert_eq!(st.link, Link::Wired("Connexion filaire 1".into()));
        assert!(st.has_wifi);
        assert_eq!(status_icon(&st), "network-wired-symbolic");
    }

    #[test]
    fn wifi_status_and_list() {
        let st = parse_status("wifi:connected:Livebox-1234\nethernet:unavailable:", "enabled", " :Voisin:30:WPA2\n*:Livebox-1234:68:WPA2 WPA3\n :Livebox-1234:40:WPA2");
        assert_eq!(st.link, Link::Wifi { ssid: "Livebox-1234".into(), signal: 68 });
        let aps = parse_wifi_list(" :Voisin:30:WPA2\n*:Livebox-1234:68:WPA2\n :Livebox-1234:40:WPA2\n :Gratuit:55:\n :::", "Livebox-1234:802-11-wireless\nFilaire:802-3-ethernet");
        assert_eq!(aps.len(), 3);
        assert!(aps[0].in_use && aps[0].known && aps[0].secure);
        assert!(!aps.iter().find(|a| a.ssid == "Gratuit").unwrap().secure);
    }

    #[test]
    fn bluetooth_battery() {
        let info = "Device 11:22:33:44:55:66 (public)\n\tName: WH-1000XM4\n\tAlias: WH-1000XM4\n\tIcon: audio-headset\n\tPaired: yes\n\tConnected: yes\n\tBattery Percentage: 0x4b (75)\n";
        let d = parse_bt_info("11:22:33:44:55:66", info);
        assert_eq!(d.name, "WH-1000XM4");
        assert!(d.paired && d.connected);
        assert_eq!(d.battery, Some(75));
        assert_eq!(parse_bt_devices("Device 11:22:33:44:55:66 WH-1000XM4\nDevice AA:BB:CC:DD:EE:FF Souris MX"), vec![
            ("11:22:33:44:55:66".into(), "WH-1000XM4".into()),
            ("AA:BB:CC:DD:EE:FF".into(), "Souris MX".into())
        ]);
    }
}

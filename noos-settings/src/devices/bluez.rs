//! Bluetooth par BlueZ, sur le bus système (`org.bluez`) avec gio : adaptateurs et appareils
//! (`GetManagedObjects`, suivi par les signaux), découverte, appairage, connexion, oubli, et
//! **agent d'appairage** (`org.bluez.Agent1`) pour les codes à confirmer ou à saisir.
//!
//! Un seul contrôleur par processus ([`Bluetooth::get`]) : l'agent reste enregistré tant que
//! Paramètres vit, et chaque page s'abonne aux changements.
//!
//! Démo sans BlueZ ni adaptateur : `NOOS_BT_DEMO=1` (appareils fictifs, appairage simulé).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

const BUS: &str = "org.bluez";
const AGENT_PATH: &str = "/org/noos/settings/bluetooth_agent";
type Props = HashMap<String, glib::Variant>;

pub fn demo() -> bool {
    std::env::var_os("NOOS_BT_DEMO").is_some()
}

pub fn demo_mode() -> String {
    std::env::var("NOOS_BT_DEMO").unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Availability {
    /// Pas encore lu.
    #[default]
    Unknown,
    Ready,
    /// bluetoothd absent ou arrêté (service désactivé, BlueZ non installé).
    NoService,
    /// Service présent mais aucun adaptateur (pas de puce Bluetooth, clé USB débranchée).
    NoAdapter,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Adapter {
    pub path: String,
    pub name: String,
    pub address: String,
    pub powered: bool,
    pub discoverable: bool,
    pub discovering: bool,
    /// `PowerState` (BlueZ ≥ 5.66) : « off-blocked » = bloqué par rfkill (mode Avion).
    pub power_state: String,
}

impl Adapter {
    pub fn blocked(&self) -> bool {
        self.power_state == "off-blocked"
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    pub path: String,
    pub address: String,
    /// `Alias` (nom choisi, ou nom annoncé, ou adresse).
    pub name: String,
    /// Vrai si l'appareil annonce un nom (sinon `name` = adresse).
    pub named: bool,
    pub icon: String,
    pub class: u32,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery: Option<u8>,
    pub rssi: Option<i16>,
}

impl Device {
    pub fn kind(&self) -> Kind {
        Kind::from_icon(&self.icon, self.class)
    }

    /// Libellé d'état sous le nom : « Connecté · 80 % », « Associé », « Non connecté »…
    pub fn status(&self) -> String {
        let mut s = if self.connected {
            "Connecté".to_string()
        } else if self.paired {
            "Associé, non connecté".to_string()
        } else {
            self.kind().label.to_string()
        };
        if let (true, Some(b)) = (self.connected, self.battery) {
            s.push_str(&format!(" · Batterie {b} %"));
        }
        s
    }
}

/// Type d'appareil : libellé et icône symbolique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    pub label: &'static str,
    pub icon: &'static str,
    /// Ordre dans les listes (audio et entrées d'abord).
    pub rank: u8,
}

impl Kind {
    /// D'après la propriété `Icon` de BlueZ (noms freedesktop), à défaut d'après la classe.
    pub fn from_icon(icon: &str, class: u32) -> Kind {
        let k = |label, icon, rank| Kind { label, icon, rank };
        match icon {
            "audio-headset" => k("Casque-micro", "audio-headset-symbolic", 0),
            "audio-headphones" => k("Casque", "audio-headphones-symbolic", 0),
            "audio-card" | "audio-speakers" => k("Enceinte", "audio-speakers-symbolic", 1),
            "input-mouse" => k("Souris", "input-mouse-symbolic", 2),
            "input-keyboard" => k("Clavier", "input-keyboard-symbolic", 2),
            "input-gaming" => k("Manette de jeu", "input-gaming-symbolic", 2),
            "input-tablet" => k("Tablette graphique", "input-tablet-symbolic", 3),
            "phone" => k("Téléphone", "phone-symbolic", 4),
            "computer" => k("Ordinateur", "computer-symbolic", 5),
            "video-display" => k("Écran ou téléviseur", "video-display-symbolic", 5),
            "camera-photo" => k("Appareil photo", "camera-photo-symbolic", 5),
            "camera-video" => k("Caméra", "camera-video-symbolic", 5),
            "printer" => k("Imprimante", "printer-symbolic", 5),
            "scanner" => k("Scanner", "scanner-symbolic", 5),
            "multimedia-player" => k("Lecteur multimédia", "multimedia-player-symbolic", 3),
            "network-wireless" => k("Point d'accès", "network-wireless-symbolic", 6),
            _ => match (class >> 8) & 0x1f {
                // Classe « majeure » du Bluetooth classique.
                0x01 => k("Ordinateur", "computer-symbolic", 5),
                0x02 => k("Téléphone", "phone-symbolic", 4),
                0x04 => k("Appareil audio", "audio-speakers-symbolic", 1),
                0x05 => k("Périphérique d'entrée", "input-keyboard-symbolic", 2),
                0x06 => k("Imprimante ou caméra", "printer-symbolic", 5),
                0x07 => k("Objet connecté", "watch-symbolic", 6),
                _ => k("Appareil Bluetooth", "bluetooth-symbolic", 7),
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub availability: Availability,
    pub adapter: Option<Adapter>,
    pub devices: Vec<Device>,
}

impl Model {
    pub fn paired(&self) -> Vec<Device> {
        let mut v: Vec<Device> = self.devices.iter().filter(|d| d.paired).cloned().collect();
        v.sort_by(|a, b| (!a.connected, a.kind().rank, a.name.to_lowercase()).cmp(&(!b.connected, b.kind().rank, b.name.to_lowercase())));
        v
    }

    /// Appareils trouvés par la recherche (non associés), les plus proches d'abord ; les
    /// appareils sans nom à la fin.
    pub fn nearby(&self) -> Vec<Device> {
        let mut v: Vec<Device> = self.devices.iter().filter(|d| !d.paired).cloned().collect();
        v.sort_by(|a, b| (!a.named, std::cmp::Reverse(a.rssi.unwrap_or(-127)), a.name.to_lowercase()).cmp(&(!b.named, std::cmp::Reverse(b.rssi.unwrap_or(-127)), b.name.to_lowercase())));
        v
    }

    pub fn device(&self, path: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.path == path)
    }
}

fn s(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.str().map(str::to_string)).unwrap_or_default()
}
fn b(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false)
}

/// Analyse la réponse de `GetManagedObjects` (`a{oa{sa{sv}}}`).
fn parse(reply: &glib::Variant) -> Model {
    let mut m = Model { availability: Availability::NoAdapter, ..Default::default() };
    let objects = reply.child_value(0);
    let mut adapters = vec![];
    let mut batteries: HashMap<String, u8> = HashMap::new();
    for n in 0..objects.n_children() {
        let entry = objects.child_value(n);
        let path = entry.child_value(0).str().unwrap_or("").to_string();
        let ifaces_v = entry.child_value(1);
        for k in 0..ifaces_v.n_children() {
            let e = ifaces_v.child_value(k);
            let name = e.child_value(0).str().unwrap_or("").to_string();
            let p: Props = e.child_value(1).get().unwrap_or_default();
            match name.as_str() {
                "org.bluez.Adapter1" => adapters.push(Adapter {
                    path: path.clone(),
                    name: s(&p, "Alias"),
                    address: s(&p, "Address"),
                    powered: b(&p, "Powered"),
                    discoverable: b(&p, "Discoverable"),
                    discovering: b(&p, "Discovering"),
                    power_state: s(&p, "PowerState"),
                }),
                "org.bluez.Device1" => {
                    let address = s(&p, "Address");
                    let alias = s(&p, "Alias");
                    let named = p.contains_key("Name") || (!alias.is_empty() && alias.replace('-', ":") != address);
                    m.devices.push(Device {
                        path: path.clone(),
                        name: if alias.is_empty() { address.clone() } else { alias },
                        address,
                        named,
                        icon: s(&p, "Icon"),
                        class: p.get("Class").and_then(|v| v.get::<u32>()).unwrap_or(0),
                        paired: b(&p, "Paired") || b(&p, "Bonded"),
                        trusted: b(&p, "Trusted"),
                        connected: b(&p, "Connected"),
                        battery: None,
                        rssi: p.get("RSSI").and_then(|v| v.get::<i16>()),
                    })
                }
                "org.bluez.Battery1" => {
                    if let Some(v) = p.get("Percentage").and_then(|v| v.get::<u8>()) {
                        batteries.insert(path.clone(), v);
                    }
                }
                _ => {}
            }
        }
    }
    for d in &mut m.devices {
        d.battery = batteries.get(&d.path).copied();
    }
    // Adaptateur par défaut : le premier allumé, sinon le premier (hci0).
    adapters.sort_by(|a, b| (!a.powered, &a.path).cmp(&(!b.powered, &b.path)));
    if let Some(a) = adapters.into_iter().next() {
        let prefix = format!("{}/", a.path);
        m.devices.retain(|d| d.path.starts_with(&prefix));
        m.adapter = Some(a);
        m.availability = Availability::Ready;
    }
    m
}

/// Message clair pour une erreur BlueZ.
pub fn explain(e: &glib::Error) -> String {
    let remote = gio::DBusError::remote_error(e).map(|s| s.to_string()).unwrap_or_default();
    let name = remote.rsplit('.').next().unwrap_or("");
    let msg = e.message();
    match name {
        "AuthenticationFailed" => "Le code n'a pas été accepté. Réessayez en vérifiant le code affiché sur l'appareil.".into(),
        "AuthenticationCanceled" | "Canceled" => "L'association a été annulée.".into(),
        "AuthenticationRejected" | "Rejected" => "L'appareil a refusé l'association.".into(),
        "AuthenticationTimeout" => "L'appareil n'a pas répondu à temps. Rapprochez-le et réessayez.".into(),
        "ConnectionAttemptFailed" | "PageTimeout" => {
            "Impossible de joindre l'appareil : il est peut-être éteint, hors de portée ou n'est plus en mode d'association.".into()
        }
        "AlreadyExists" => "Cet appareil est déjà associé.".into(),
        "InProgress" => "Une opération est déjà en cours avec cet appareil, patientez quelques secondes.".into(),
        "NotReady" => "Le Bluetooth est désactivé.".into(),
        "Blocked" => "Le Bluetooth est bloqué (mode Avion ou interrupteur du PC).".into(),
        "NotAvailable" | "NotSupported" => "Cet appareil ne propose aucun service utilisable par ce PC.".into(),
        "DoesNotExist" => "L'appareil n'est plus visible. Relancez la recherche.".into(),
        _ if msg.contains("br-connection-profile-unavailable") => {
            "Aucun profil compatible : pour un casque ou une enceinte, le module audio Bluetooth de PipeWire (libspa-0.2-bluetooth) est nécessaire.".into()
        }
        _ if msg.contains("Timeout") || msg.contains("timed out") => "L'appareil n'a pas répondu à temps. Rapprochez-le et réessayez.".into(),
        _ if msg.contains("AccessDenied") || msg.contains("not allowed") => {
            "Accès refusé par BlueZ : votre compte doit faire partie du groupe « bluetooth » (reconnectez-vous après l'avoir ajouté).".into()
        }
        _ => msg.split(": ").last().unwrap_or(msg).to_string(),
    }
}

// ───────────────────────────── Agent d'appairage ─────────────────────────────

/// Réponse différée à une demande de l'agent (BlueZ attend tant qu'on n'a pas répondu).
/// En démo, `demo` est rappelée à la place (vrai = accepté).
pub struct Reply {
    inv: RefCell<Option<gio::DBusMethodInvocation>>,
    demo: RefCell<Option<Box<dyn FnOnce(bool)>>>,
}

impl Reply {
    fn new(inv: Option<gio::DBusMethodInvocation>) -> Rc<Self> {
        Rc::new(Self { inv: RefCell::new(inv), demo: RefCell::new(None) })
    }
    fn simulated(f: impl FnOnce(bool) + 'static) -> Rc<Self> {
        Rc::new(Self { inv: RefCell::new(None), demo: RefCell::new(Some(Box::new(f))) })
    }
    fn done(&self, ok: bool) {
        if let Some(f) = self.demo.borrow_mut().take() {
            f(ok);
        }
    }
    pub fn accept(&self) {
        if let Some(i) = self.inv.borrow_mut().take() {
            i.return_value(None);
        }
        self.done(true);
    }
    pub fn pin(&self, pin: &str) {
        if let Some(i) = self.inv.borrow_mut().take() {
            i.return_value(Some(&(pin.to_string(),).to_variant()));
        }
        self.done(true);
    }
    pub fn passkey(&self, key: u32) {
        if let Some(i) = self.inv.borrow_mut().take() {
            i.return_value(Some(&(key,).to_variant()));
        }
        self.done(true);
    }
    pub fn reject(&self) {
        if let Some(i) = self.inv.borrow_mut().take() {
            i.return_dbus_error("org.bluez.Error.Rejected", "refusé par l'utilisateur");
        }
        self.done(false);
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        // Jamais de demande laissée sans réponse (BlueZ attendrait jusqu'à son délai).
        if let Some(i) = self.inv.get_mut().take() {
            i.return_dbus_error("org.bluez.Error.Canceled", "annulé");
        }
        if let Some(f) = self.demo.get_mut().take() {
            f(false);
        }
    }
}

/// Demandes de l'agent, transmises à l'interface.
pub enum AgentRequest {
    /// « Le code 123456 s'affiche-t-il sur l'appareil ? »
    Confirm { device: String, passkey: u32, reply: Rc<Reply> },
    /// Code PIN à saisir (anciens appareils, 0000 ou 1234 le plus souvent).
    PinCode { device: String, reply: Rc<Reply> },
    /// Code numérique (6 chiffres) à saisir.
    Passkey { device: String, reply: Rc<Reply> },
    /// Code à taper sur l'appareil (clavier) ; `entered` = chiffres déjà tapés.
    Display { device: String, code: String, entered: Option<u16> },
    /// Autoriser une connexion entrante (appareil non associé).
    Authorize { device: String, service: Option<String>, reply: Rc<Reply> },
    /// BlueZ annule la demande en cours.
    Cancel,
}

const AGENT_XML: &str = r#"<node>
  <interface name="org.bluez.Agent1">
    <method name="Release"/>
    <method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
    <method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
    <method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/><arg type="q" direction="in"/></method>
    <method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
    <method name="RequestAuthorization"><arg type="o" direction="in"/></method>
    <method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
    <method name="Cancel"/>
  </interface>
</node>"#;

/// Nom lisible d'un profil (UUID 128 bits « 0000xxxx-0000-1000-8000-00805f9b34fb »).
pub fn service_name(uuid: &str) -> Option<&'static str> {
    let short = uuid.get(4..8)?;
    Some(match short.to_ascii_lowercase().as_str() {
        "110a" | "110b" | "110d" => "audio (A2DP)",
        "110e" | "110c" => "télécommande (AVRCP)",
        "1108" | "1112" | "111e" | "111f" => "micro et appels",
        "1124" => "clavier / souris (HID)",
        "1105" => "transfert de fichiers (OBEX)",
        "1106" => "transfert de fichiers (FTP)",
        "112f" => "répertoire téléphonique",
        "1132" => "messages (SMS)",
        "1115" | "1116" => "partage de connexion",
        _ => return None,
    })
}

// ───────────────────────────── Contrôleur ─────────────────────────────

type Listener = Box<dyn Fn() -> bool>;

pub struct Bluetooth {
    conn: RefCell<Option<gio::DBusConnection>>,
    model: RefCell<Model>,
    listeners: RefCell<Vec<Listener>>,
    agent_ui: RefCell<Option<Rc<dyn Fn(AgentRequest)>>>,
    pending: Cell<bool>,
    agent_registered: Cell<bool>,
}

thread_local! {
    static INSTANCE: RefCell<Option<Rc<Bluetooth>>> = const { RefCell::new(None) };
}

impl Bluetooth {
    /// Contrôleur unique (créé au premier appel : connexion au bus, agent, abonnements).
    pub fn get() -> Rc<Bluetooth> {
        if let Some(b) = INSTANCE.with(|i| i.borrow().clone()) {
            return b;
        }
        let bt = Rc::new(Bluetooth {
            conn: RefCell::new(None),
            model: RefCell::new(Model::default()),
            listeners: RefCell::new(vec![]),
            agent_ui: RefCell::new(None),
            pending: Cell::new(false),
            agent_registered: Cell::new(false),
        });
        INSTANCE.with(|i| *i.borrow_mut() = Some(bt.clone()));
        if demo() {
            *bt.model.borrow_mut() = demo_model();
        } else {
            bt.connect();
        }
        bt
    }

    pub fn model(&self) -> Model {
        self.model.borrow().clone()
    }

    /// `f` est rappelée à chaque changement ; elle renvoie `false` pour se désabonner
    /// (page fermée).
    pub fn subscribe(&self, f: impl Fn() -> bool + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    /// Interface qui répond aux demandes de l'agent (codes, autorisations).
    pub fn set_agent_ui(&self, f: impl Fn(AgentRequest) + 'static) {
        *self.agent_ui.borrow_mut() = Some(Rc::new(f));
    }

    fn notify(&self) {
        let list = std::mem::take(&mut *self.listeners.borrow_mut());
        let kept: Vec<Listener> = list.into_iter().filter(|f| f()).collect();
        let mut l = self.listeners.borrow_mut();
        let added = std::mem::take(&mut *l);
        *l = kept;
        l.extend(added);
    }

    fn agent_request(&self, req: AgentRequest) {
        let ui = self.agent_ui.borrow().clone();
        match ui {
            Some(f) => f(req),
            // Aucune interface : les `Reply` abandonnées répondent « annulé ».
            None => drop(req),
        }
    }

    fn connect(self: &Rc<Self>) {
        let this = self.clone();
        glib::spawn_future_local(async move {
            let conn = match gio::bus_get_future(gio::BusType::System).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("noos-settings : Bluetooth : bus système : {e}");
                    this.model.borrow_mut().availability = Availability::NoService;
                    this.notify();
                    return;
                }
            };
            // GIO ne garde qu'une référence faible sur la connexion : on la conserve.
            *this.conn.borrow_mut() = Some(conn.clone());
            this.watch(&conn);
            this.reload().await;
        });
    }

    fn watch(self: &Rc<Self>, conn: &gio::DBusConnection) {
        let weak = Rc::downgrade(self);
        let changed = move || {
            let Some(this) = weak.upgrade() else { return };
            if this.pending.replace(true) {
                return;
            }
            let weak = Rc::downgrade(&this);
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if let Some(this) = weak.upgrade() {
                    this.pending.set(false);
                    glib::spawn_future_local(async move { this.reload().await });
                }
            });
        };
        let changed = Rc::new(changed);
        for (sender, iface, member) in [
            (Some(BUS), Some("org.freedesktop.DBus.ObjectManager"), None),
            (Some(BUS), Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged")),
            // bluetoothd démarré ou arrêté.
            (Some("org.freedesktop.DBus"), Some("org.freedesktop.DBus"), Some("NameOwnerChanged")),
        ] {
            let c = changed.clone();
            #[allow(deprecated)]
            conn.signal_subscribe(sender, iface, member, None, if member == Some("NameOwnerChanged") { Some(BUS) } else { None }, gio::DBusSignalFlags::NONE, move |_, _, _, _, _, _| c());
        }
    }

    async fn reload(self: &Rc<Self>) {
        let Some(conn) = self.conn.borrow().clone() else { return };
        let reply = conn
            .call_future(Some(BUS), "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None, None, gio::DBusCallFlags::NONE, 5000)
            .await;
        let m = match reply {
            Ok(r) => parse(&r),
            Err(e) => {
                eprintln!("noos-settings : Bluetooth : {e}");
                self.agent_registered.set(false);
                Model { availability: Availability::NoService, ..Default::default() }
            }
        };
        if m.availability == Availability::Ready && !self.agent_registered.get() {
            self.agent_registered.set(true);
            self.register_agent(&conn).await;
        }
        let changed = {
            let old = self.model.borrow();
            old.availability != m.availability || old.adapter != m.adapter || old.devices != m.devices
        };
        *self.model.borrow_mut() = m;
        if changed {
            self.notify();
        }
    }

    async fn register_agent(self: &Rc<Self>, conn: &gio::DBusConnection) {
        thread_local! { static OBJECT: Cell<bool> = const { Cell::new(false) }; }
        if !OBJECT.with(|o| o.get()) {
            let node = gio::DBusNodeInfo::for_xml(AGENT_XML).expect("XML agent BlueZ");
            let iface = node.lookup_interface("org.bluez.Agent1").expect("interface Agent1");
            let weak = Rc::downgrade(self);
            let res = conn
                .register_object(AGENT_PATH, &iface)
                .method_call(move |_, _, _, _, method, params, inv| match weak.upgrade() {
                    Some(this) => this.agent_call(method, &params, inv),
                    None => inv.return_dbus_error("org.bluez.Error.Rejected", "Paramètres fermé"),
                })
                .build();
            match res {
                Ok(_) => OBJECT.with(|o| o.set(true)),
                Err(e) => return eprintln!("noos-settings : Bluetooth : agent : {e}"),
            }
        }
        let args = (glib::variant::ObjectPath::try_from(AGENT_PATH.to_string()).unwrap(), "KeyboardDisplay").to_variant();
        let r = conn
            .call_future(Some(BUS), "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", Some(&args), None, gio::DBusCallFlags::NONE, 5000)
            .await;
        if let Err(e) = r {
            if !e.message().contains("AlreadyExists") {
                return eprintln!("noos-settings : Bluetooth : RegisterAgent : {e}");
            }
        }
        // Agent par défaut : c'est lui que BlueZ sollicite aussi pour les demandes venues de
        // l'appareil (un téléphone qui s'associe au PC) tant que Paramètres est ouvert.
        let args = (glib::variant::ObjectPath::try_from(AGENT_PATH.to_string()).unwrap(),).to_variant();
        let _ = conn
            .call_future(Some(BUS), "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent", Some(&args), None, gio::DBusCallFlags::NONE, 5000)
            .await;
    }

    fn device_name(&self, path: &str) -> String {
        self.model.borrow().device(path).map(|d| d.name.clone()).unwrap_or_else(|| "l'appareil".into())
    }

    fn agent_call(self: &Rc<Self>, method: &str, params: &glib::Variant, inv: gio::DBusMethodInvocation) {
        let dev = params.try_child_value(0).and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
        let device = self.device_name(&dev);
        match method {
            "Release" => inv.return_value(None),
            "Cancel" => {
                inv.return_value(None);
                self.agent_request(AgentRequest::Cancel);
            }
            "RequestPinCode" => self.agent_request(AgentRequest::PinCode { device, reply: Reply::new(Some(inv)) }),
            "RequestPasskey" => self.agent_request(AgentRequest::Passkey { device, reply: Reply::new(Some(inv)) }),
            "RequestConfirmation" => {
                let passkey = params.try_child_value(1).and_then(|v| v.get::<u32>()).unwrap_or(0);
                self.agent_request(AgentRequest::Confirm { device, passkey, reply: Reply::new(Some(inv)) })
            }
            "DisplayPinCode" => {
                let code = params.try_child_value(1).and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
                inv.return_value(None);
                self.agent_request(AgentRequest::Display { device, code, entered: None });
            }
            "DisplayPasskey" => {
                let key = params.try_child_value(1).and_then(|v| v.get::<u32>()).unwrap_or(0);
                let entered = params.try_child_value(2).and_then(|v| v.get::<u16>());
                inv.return_value(None);
                self.agent_request(AgentRequest::Display { device, code: format!("{key:06}"), entered });
            }
            "RequestAuthorization" => self.agent_request(AgentRequest::Authorize { device, service: None, reply: Reply::new(Some(inv)) }),
            "AuthorizeService" => {
                // Appareil associé et de confiance : accepté sans question (comme Windows).
                let trusted = self.model.borrow().device(&dev).is_some_and(|d| d.paired && d.trusted);
                if trusted {
                    inv.return_value(None);
                    return;
                }
                let uuid = params.try_child_value(1).and_then(|v| v.str().map(str::to_string)).unwrap_or_default();
                let service = service_name(&uuid).map(str::to_string);
                self.agent_request(AgentRequest::Authorize { device, service, reply: Reply::new(Some(inv)) })
            }
            _ => inv.return_error(gio::DBusError::UnknownMethod, "méthode inconnue"),
        }
    }

    // ───────────── Actions ─────────────

    async fn call(&self, path: &str, iface: &str, method: &str, args: Option<glib::Variant>, timeout_ms: i32) -> Result<(), String> {
        let conn = self.conn.borrow().clone().ok_or_else(|| "Service Bluetooth indisponible.".to_string())?;
        conn.call_future(Some(BUS), path, iface, method, args.as_ref(), None, gio::DBusCallFlags::NONE, timeout_ms)
            .await
            .map(|_| ())
            .map_err(|e| explain(&e))
    }

    async fn set_prop(&self, path: &str, iface: &str, name: &str, value: glib::Variant) -> Result<(), String> {
        let args = (iface, name, value).to_variant();
        self.call(path, "org.freedesktop.DBus.Properties", "Set", Some(args), 10_000).await
    }

    /// Modifie le modèle de démo puis prévient les pages.
    fn demo_edit(&self, f: impl FnOnce(&mut Model)) {
        f(&mut self.model.borrow_mut());
        self.notify();
    }

    pub async fn set_powered(self: &Rc<Self>, on: bool) -> Result<(), String> {
        if demo() {
            glib::timeout_future(Duration::from_millis(300)).await;
            self.demo_edit(|m| {
                if let Some(a) = &mut m.adapter {
                    a.powered = on;
                }
                for d in &mut m.devices {
                    d.connected &= on;
                }
            });
            return Ok(());
        }
        let Some(a) = self.model.borrow().adapter.clone() else { return Err("Aucun adaptateur Bluetooth.".into()) };
        if on && a.blocked() {
            unblock_rfkill()?;
            glib::timeout_future(Duration::from_millis(400)).await;
        }
        self.set_prop(&a.path, "org.bluez.Adapter1", "Powered", on.to_variant()).await
    }

    pub async fn set_discoverable(self: &Rc<Self>, on: bool) -> Result<(), String> {
        if demo() {
            self.demo_edit(|m| {
                if let Some(a) = &mut m.adapter {
                    a.discoverable = on;
                }
            });
            return Ok(());
        }
        let Some(a) = self.model.borrow().adapter.clone() else { return Err("Aucun adaptateur Bluetooth.".into()) };
        self.set_prop(&a.path, "org.bluez.Adapter1", "Discoverable", on.to_variant()).await
    }

    pub async fn discovery(self: &Rc<Self>, on: bool) -> Result<(), String> {
        if demo() {
            self.demo_edit(|m| {
                if let Some(a) = &mut m.adapter {
                    a.discovering = on;
                }
            });
            if on {
                self.demo_discover();
            }
            return Ok(());
        }
        let Some(a) = self.model.borrow().adapter.clone() else { return Err("Aucun adaptateur Bluetooth.".into()) };
        if on == a.discovering {
            return Ok(());
        }
        let r = self.call(&a.path, "org.bluez.Adapter1", if on { "StartDiscovery" } else { "StopDiscovery" }, None, 10_000).await;
        // Une recherche lancée par un autre programme (barre des tâches) : pas une erreur.
        match r {
            Err(e) if e.contains("InProgress") || e.contains("déjà en cours") || e.contains("NotAuthorized") => Ok(()),
            r => r,
        }
    }

    /// Associe l'appareil (codes demandés par l'agent), le marque de confiance puis le connecte.
    pub async fn pair(self: &Rc<Self>, path: &str) -> Result<(), String> {
        if demo() {
            return self.demo_pair(path).await;
        }
        match self.call(path, "org.bluez.Device1", "Pair", None, 90_000).await {
            Ok(()) => {}
            Err(e) if e.contains("déjà associé") => {}
            Err(e) => return Err(e),
        }
        let _ = self.set_prop(path, "org.bluez.Device1", "Trusted", true.to_variant()).await;
        // Certains appareils (souris BLE) se connectent d'eux-mêmes après l'association.
        let connected = self.model.borrow().device(path).is_some_and(|d| d.connected);
        if !connected {
            // Échec de connexion après une association réussie : l'appareil reste associé,
            // on ne le signale pas comme une erreur (Connecter depuis sa carte).
            if let Err(e) = self.call(path, "org.bluez.Device1", "Connect", None, 30_000).await {
                eprintln!("noos-settings : Bluetooth : connexion après association : {e}");
            }
        }
        Ok(())
    }

    pub async fn cancel_pairing(self: &Rc<Self>, path: &str) {
        if !demo() {
            let _ = self.call(path, "org.bluez.Device1", "CancelPairing", None, 5000).await;
        }
    }

    pub async fn connect_device(self: &Rc<Self>, path: &str, on: bool) -> Result<(), String> {
        if demo() {
            glib::timeout_future(Duration::from_millis(900)).await;
            let p = path.to_string();
            self.demo_edit(|m| {
                if let Some(d) = m.devices.iter_mut().find(|d| d.path == p) {
                    d.connected = on;
                }
            });
            return Ok(());
        }
        self.call(path, "org.bluez.Device1", if on { "Connect" } else { "Disconnect" }, None, 30_000).await
    }

    pub async fn rename(self: &Rc<Self>, path: &str, name: &str) -> Result<(), String> {
        if demo() {
            let (p, n) = (path.to_string(), name.to_string());
            self.demo_edit(|m| {
                if let Some(d) = m.devices.iter_mut().find(|d| d.path == p) {
                    d.name = n;
                }
            });
            return Ok(());
        }
        self.set_prop(path, "org.bluez.Device1", "Alias", name.to_variant()).await
    }

    /// Oublie l'appareil (`Adapter1.RemoveDevice`) : il faudra l'associer de nouveau.
    pub async fn forget(self: &Rc<Self>, path: &str) -> Result<(), String> {
        if demo() {
            glib::timeout_future(Duration::from_millis(400)).await;
            let p = path.to_string();
            self.demo_edit(|m| m.devices.retain(|d| d.path != p));
            return Ok(());
        }
        let Some(a) = self.model.borrow().adapter.clone() else { return Err("Aucun adaptateur Bluetooth.".into()) };
        let dev = glib::variant::ObjectPath::try_from(path.to_string()).map_err(|e| e.to_string())?;
        self.call(&a.path, "org.bluez.Adapter1", "RemoveDevice", Some((dev,).to_variant()), 10_000).await
    }

    // ───────────── Démo ─────────────

    fn demo_discover(self: &Rc<Self>) {
        let found = [
            ("/org/bluez/hci0/dev_D0_81_7A_11_22_33", "iPhone de Camille", "phone", -48, 1200),
            ("/org/bluez/hci0/dev_F4_4E_FD_44_55_66", "JBL Flip 6", "audio-card", -62, 2200),
            ("/org/bluez/hci0/dev_E8_AB_FA_77_88_99", "Keychron K3", "input-keyboard", -55, 3200),
            ("/org/bluez/hci0/dev_40_8E_2C_AA_BB_CC", "Galaxy Buds2 Pro", "audio-headphones", -70, 4300),
            ("/org/bluez/hci0/dev_5C_F3_70_12_34_56", "5C-F3-70-12-34-56", "", -88, 5000),
        ];
        for (path, name, icon, rssi, ms) in found {
            let this = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                if !this.model.borrow().adapter.as_ref().is_some_and(|a| a.discovering) {
                    return;
                }
                this.demo_edit(|m| {
                    if m.device(path).is_none() {
                        m.devices.push(Device {
                            path: path.into(),
                            address: path.rsplit("dev_").next().unwrap_or("").replace('_', ":"),
                            name: name.into(),
                            named: !icon.is_empty(),
                            icon: icon.into(),
                            rssi: Some(rssi),
                            ..Default::default()
                        });
                    }
                });
            });
        }
    }

    async fn demo_pair(self: &Rc<Self>, path: &str) -> Result<(), String> {
        glib::timeout_future(Duration::from_millis(1200)).await;
        let kind = self.model.borrow().device(path).map(|d| d.icon.clone()).unwrap_or_default();
        let device = self.device_name(path);
        let (tx, rx) = async_channel::bounded::<bool>(1);
        let answer = Rc::new(move |ok: bool| {
            let _ = tx.try_send(ok);
        });
        let a1 = answer.clone();
        // Le clavier demande de taper un code, le téléphone une confirmation, l'enceinte rien.
        match kind.as_str() {
            "phone" | "audio-headphones" => {
                self.agent_request(AgentRequest::Confirm { device, passkey: 482_913, reply: Reply::simulated(move |ok| a1(ok)) });
            }
            "input-keyboard" => {
                self.agent_request(AgentRequest::Display { device, code: "275 019".into(), entered: Some(3) });
                let a = a1.clone();
                glib::timeout_add_local_once(Duration::from_millis(4000), move || a(true));
            }
            _ if demo_mode() == "pin" => {
                self.agent_request(AgentRequest::PinCode { device, reply: Reply::simulated(move |ok| a1(ok)) });
            }
            _ => a1(true),
        }
        let ok = rx.recv().await.unwrap_or(false);
        self.agent_request(AgentRequest::Cancel);
        if !ok {
            return Err("L'association a été annulée.".into());
        }
        glib::timeout_future(Duration::from_millis(900)).await;
        let p = path.to_string();
        self.demo_edit(|m| {
            if let Some(d) = m.devices.iter_mut().find(|d| d.path == p) {
                d.paired = true;
                d.trusted = true;
                d.connected = true;
                if d.icon.starts_with("audio") || d.icon.starts_with("input") {
                    d.battery = Some(90);
                }
            }
        });
        Ok(())
    }
}

fn demo_model() -> Model {
    let mode = demo_mode();
    let adapter = Adapter {
        path: "/org/bluez/hci0".into(),
        name: "PC-de-Alex".into(),
        address: "9C:B6:D0:0A:1B:2C".into(),
        powered: mode != "off",
        discoverable: false,
        discovering: false,
        power_state: if mode == "off" { "off".into() } else { "on".into() },
    };
    if mode == "none" {
        return Model { availability: Availability::NoAdapter, ..Default::default() };
    }
    if mode == "service" {
        return Model { availability: Availability::NoService, ..Default::default() };
    }
    let on = adapter.powered;
    let dev = |id: &str, name: &str, icon: &str, connected: bool, battery: Option<u8>| Device {
        path: format!("/org/bluez/hci0/dev_{id}"),
        address: id.replace('_', ":"),
        name: name.into(),
        named: true,
        icon: icon.into(),
        paired: true,
        trusted: true,
        connected: connected && on,
        battery,
        ..Default::default()
    };
    Model {
        availability: Availability::Ready,
        adapter: Some(adapter),
        devices: vec![
            dev("AC_80_0A_01_02_03", "WH-1000XM5", "audio-headphones", true, Some(80)),
            dev("D4_3A_2C_04_05_06", "MX Master 3S", "input-mouse", true, Some(55)),
            dev("98_7A_14_07_08_09", "Manette Xbox", "input-gaming", false, Some(20)),
            dev("A4_C1_38_0A_0B_0C", "Pixel 9", "phone", false, None),
        ],
    }
}

/// Débloque le Bluetooth bloqué par rfkill (mode Avion) : écriture d'un `rfkill_event`
/// « CHANGE_ALL » dans /dev/rfkill (accessible à l'utilisateur de la session active).
pub fn unblock_rfkill() -> Result<(), String> {
    use std::io::Write;
    // struct rfkill_event { u32 idx; u8 type; u8 op; u8 soft; u8 hard; }
    // type 2 = BLUETOOTH, op 3 = CHANGE_ALL, soft 0 = débloqué.
    let ev: [u8; 8] = [0, 0, 0, 0, 2, 3, 0, 0];
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/rfkill")
        .and_then(|mut f| f.write_all(&ev))
        .map_err(|e| format!("Impossible de débloquer le Bluetooth (rfkill) : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(Kind::from_icon("audio-headphones", 0).label, "Casque");
        assert_eq!(Kind::from_icon("input-gaming", 0).icon, "input-gaming-symbolic");
        // Classe 0x240404 : audio (majeure 0x04).
        assert_eq!(Kind::from_icon("", 0x240404).label, "Appareil audio");
        assert_eq!(Kind::from_icon("", 0).label, "Appareil Bluetooth");
    }

    #[test]
    fn services() {
        assert_eq!(service_name("0000110b-0000-1000-8000-00805f9b34fb"), Some("audio (A2DP)"));
        assert_eq!(service_name("00001124-0000-1000-8000-00805f9b34fb"), Some("clavier / souris (HID)"));
        assert_eq!(service_name("abc"), None);
    }

    #[test]
    fn order() {
        let m = demo_model();
        let p = m.paired();
        assert_eq!(p[0].name, "WH-1000XM5");
        assert!(p.last().unwrap().name == "Pixel 9" || !p.last().unwrap().connected);
        assert_eq!(p[0].status(), "Connecté · Batterie 80 %");
    }
}

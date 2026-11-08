//! Lecture d'udisks2 (bus système) : disques, volumes, partitions, santé (SMART ATA / NVMe) et
//! tâches en cours, en un seul appel `GetManagedObjects`. Les opérations (formater, monter,
//! éjecter…) sont dans [`crate::ops`] ; udisks demande lui-même l'autorisation (polkit).

use std::collections::HashMap;

use gtk::{gio, glib};

pub const BUS: &str = "org.freedesktop.UDisks2";
pub const ROOT: &str = "/org/freedesktop/UDisks2";
const I: &str = "org.freedesktop.UDisks2.";

type Props = HashMap<String, glib::Variant>;

fn s(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.get::<String>()).unwrap_or_default()
}
fn u(p: &Props, k: &str) -> u64 {
    p.get(k).and_then(|v| v.get::<u64>().or_else(|| v.get::<u32>().map(u64::from))).unwrap_or(0)
}
fn i(p: &Props, k: &str) -> i64 {
    p.get(k).and_then(|v| v.get::<i64>().or_else(|| v.get::<i32>().map(i64::from))).unwrap_or(0)
}
fn b(p: &Props, k: &str) -> bool {
    p.get(k).and_then(|v| v.get::<bool>()).unwrap_or(false)
}
fn d(p: &Props, k: &str) -> f64 {
    p.get(k).and_then(|v| v.get::<f64>()).unwrap_or(0.0)
}
/// Chaîne d'octets terminée par NUL (`ay`).
fn ay(v: &glib::Variant) -> String {
    let bytes: Vec<u8> = v.get::<Vec<u8>>().unwrap_or_default();
    String::from_utf8_lossy(bytes.split(|c| *c == 0).next().unwrap_or(&[])).into_owned()
}
fn ays(p: &Props, k: &str) -> String {
    p.get(k).map(ay).unwrap_or_default()
}
fn aay(p: &Props, k: &str) -> Vec<String> {
    let Some(v) = p.get(k) else { return vec![] };
    (0..v.n_children()).map(|n| ay(&v.child_value(n))).filter(|s| !s.is_empty()).collect()
}
fn o(p: &Props, k: &str) -> String {
    p.get(k).and_then(|v| v.get::<glib::variant::ObjectPath>()).map(|o| o.as_str().to_string()).filter(|s| s != "/").unwrap_or_default()
}

/// Santé du disque.
#[derive(Debug, Clone, Default)]
pub struct Health {
    /// « ATA » (SMART) ou « NVMe ».
    pub kind: &'static str,
    pub failing: bool,
    pub temperature_c: Option<f64>,
    pub power_on_hours: Option<u64>,
    pub bad_sectors: Option<i64>,
    pub attributes_failing: i64,
    /// Usure (NVMe, %).
    pub percent_used: Option<i64>,
    pub selftest: String,
    pub selftest_remaining: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Drive {
    pub path: String,
    pub vendor: String,
    pub model: String,
    pub serial: String,
    pub revision: String,
    pub size: u64,
    pub removable: bool,
    pub ejectable: bool,
    pub can_power_off: bool,
    pub media_removable: bool,
    pub optical: bool,
    pub media: String,
    /// `usb`, `sdio`, `ieee1394`, `` (interne)…
    pub connection: String,
    /// Tours/min : 0 = SSD, -1 = inconnu (disque dur), > 0 = disque dur.
    pub rotation: i32,
    pub sort_key: String,
    pub nvme: bool,
    pub health: Option<Health>,
}

impl Drive {
    pub fn name(&self) -> String {
        let n = format!("{} {}", self.vendor.trim(), self.model.trim()).trim().to_string();
        if n.is_empty() { "Disque".into() } else { n }
    }
    pub fn is_external(&self) -> bool {
        self.removable || self.ejectable || self.connection == "usb" || self.connection == "sdio"
    }
    /// Nature lisible : « Clé USB », « SSD NVMe », « Disque dur »…
    pub fn kind(&self) -> &'static str {
        if self.connection == "loop" {
            "Image disque"
        } else if self.optical {
            "Lecteur optique"
        } else if self.connection == "sdio" || self.media.starts_with("flash_sd") {
            "Carte mémoire"
        } else if self.connection == "usb" && (self.removable || self.media_removable) {
            "Clé USB"
        } else if self.connection == "usb" {
            "Disque externe USB"
        } else if self.nvme {
            "SSD NVMe"
        } else if self.rotation == 0 {
            "SSD"
        } else {
            "Disque dur"
        }
    }
    pub fn icon(&self) -> &'static str {
        if self.connection == "loop" {
            "media-optical-cd-audio-symbolic"
        } else if self.optical {
            "media-optical-symbolic"
        } else if self.connection == "sdio" || self.media.starts_with("flash_sd") {
            "media-flash-symbolic"
        } else if self.is_external() {
            "drive-removable-media-symbolic"
        } else if self.rotation == 0 || self.nvme {
            "drive-harddisk-solidstate-symbolic"
        } else {
            "drive-harddisk-symbolic"
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Partition {
    pub number: u32,
    pub offset: u64,
    pub size: u64,
    pub type_: String,
    pub name: String,
    pub table: String,
    pub container: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub path: String,
    pub device: String,
    pub size: u64,
    pub drive: String,
    pub id_type: String,
    pub id_usage: String,
    pub id_label: String,
    pub id_uuid: String,
    pub id_version: String,
    pub read_only: bool,
    pub hint_system: bool,
    pub hint_ignore: bool,
    pub hint_name: String,
    /// Volume déchiffré : chemin du bloc chiffré d'origine.
    pub crypto_backing: String,
    /// Table de partitions portée par ce bloc (`gpt`, `dos`).
    pub table: Option<String>,
    pub partition: Option<Partition>,
    /// Système de fichiers : points de montage (vide = non monté).
    pub filesystem: Option<Vec<String>>,
    /// Volume chiffré (LUKS) : bloc déchiffré s'il est déverrouillé.
    pub encrypted: Option<String>,
    pub loop_file: Option<String>,
    pub swap_active: Option<bool>,
    /// Ligne /etc/fstab du volume : (point de montage, options, gérée par NOOS).
    pub fstab: Option<(String, String)>,
}

impl Block {
    /// Système de fichiers Linux (droits Unix : la propriété du dossier compte).
    pub fn unix_fs(&self) -> bool {
        matches!(self.id_type.as_str(), "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs" | "nilfs2")
    }
}

impl Block {
    pub fn mounted_at(&self) -> Option<&str> {
        self.filesystem.as_ref()?.first().map(|s| s.as_str())
    }
    pub fn is_partition_of_table(&self) -> bool {
        self.partition.is_some()
    }
    /// Nom à afficher : étiquette, sinon nom de partition, sinon type.
    pub fn title(&self) -> String {
        if !self.id_label.is_empty() {
            return self.id_label.clone();
        }
        if !self.hint_name.is_empty() {
            return self.hint_name.clone();
        }
        if let Some(p) = &self.partition {
            if !p.name.is_empty() {
                return p.name.clone();
            }
        }
        match self.id_usage.as_str() {
            "filesystem" => format!("Volume {}", fs_name(&self.id_type)),
            "crypto" => "Volume chiffré".into(),
            "other" if self.id_type == "swap" => "Mémoire d'échange".into(),
            _ if self.partition.as_ref().is_some_and(|p| p.container) => "Partition étendue".into(),
            _ => "Partition inconnue".into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub operation: String,
    pub progress: Option<f64>,
    pub objects: Vec<String>,
    pub cancelable: bool,
    pub path: String,
}

impl Job {
    pub fn label(&self) -> String {
        match self.operation.as_str() {
            o if o.starts_with("format") => "Formatage",
            "filesystem-mount" => "Montage",
            "filesystem-unmount" => "Démontage",
            "filesystem-check" => "Vérification",
            "filesystem-repair" => "Réparation",
            "partition-create" => "Création de la partition",
            "partition-delete" => "Suppression de la partition",
            "partition-modify" => "Modification de la partition",
            o if o.starts_with("ata-secure-erase") => "Effacement sécurisé",
            "ata-smart-selftest" => "Autotest SMART",
            "drive-eject" => "Éjection",
            "cleanup" => "Nettoyage",
            o if o.starts_with("encrypted") => "Chiffrement",
            _ => "Opération",
        }
        .to_string()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Model {
    pub drives: Vec<Drive>,
    pub blocks: Vec<Block>,
    pub jobs: Vec<Job>,
    pub supported: Vec<String>,
}

impl Model {
    pub fn drive(&self, path: &str) -> Option<&Drive> {
        self.drives.iter().find(|d| d.path == path)
    }
    pub fn block(&self, path: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.path == path)
    }
    /// Bloc du disque entier.
    pub fn whole(&self, drive: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.drive == drive && b.partition.is_none() && b.crypto_backing.is_empty())
    }
    /// Partitions du disque, dans l'ordre.
    pub fn partitions(&self, drive: &str) -> Vec<&Block> {
        let mut v: Vec<&Block> = self.blocks.iter().filter(|b| b.drive == drive && b.partition.is_some()).collect();
        v.sort_by_key(|b| b.partition.as_ref().map(|p| p.offset).unwrap_or(0));
        v
    }
    /// Bloc déchiffré d'un volume chiffré déverrouillé.
    pub fn cleartext(&self, encrypted: &Block) -> Option<&Block> {
        encrypted.encrypted.as_deref().filter(|s| !s.is_empty()).and_then(|p| self.block(p))
    }
    /// Volume « utilisable » : lui-même, ou son contenu déchiffré.
    pub fn content<'a>(&'a self, b: &'a Block) -> &'a Block {
        self.cleartext(b).unwrap_or(b)
    }
    /// Le disque porte-t-il le système (/, /boot, /home…) ?
    pub fn is_system(&self, drive: &str) -> bool {
        self.blocks.iter().filter(|b| b.drive == drive).any(|b| {
            let c = self.content(b);
            c.filesystem.iter().flatten().any(|m| m == "/" || m.starts_with("/boot") || m == "/home" || m == "/sysroot" || m == "/var")
                || c.swap_active == Some(true)
        })
    }
    /// Disques à éjecter depuis la barre des tâches (amovibles, branchés).
    pub fn removable_drives(&self) -> Vec<&Drive> {
        self.drives.iter().filter(|d| d.is_external() && !self.is_system(&d.path) && self.whole(&d.path).is_some()).collect()
    }
    /// Volumes (systèmes de fichiers) d'un disque, contenu déchiffré compris.
    pub fn volumes(&self, drive: &str) -> Vec<&Block> {
        let mut v = vec![];
        for b in self.blocks.iter().filter(|b| b.drive == drive) {
            let c = self.content(b);
            if c.filesystem.is_some() && !v.iter().any(|x: &&Block| x.path == c.path) {
                v.push(c);
            }
        }
        v
    }
    pub fn job_for(&self, object: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.objects.iter().any(|o| o == object))
    }
}

/// Connexion au bus système (partagée par GIO).
pub fn system_bus() -> Result<gio::DBusConnection, String> {
    gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).map_err(|e| e.to_string())
}

fn parse(reply: &glib::Variant) -> Model {
    let mut m = Model::default();
    let objects = reply.child_value(0);
    for n in 0..objects.n_children() {
        let entry = objects.child_value(n);
        let path = entry.child_value(0).str().unwrap_or("").to_string();
        let ifaces_v = entry.child_value(1);
        let mut ifaces: HashMap<String, Props> = HashMap::new();
        for k in 0..ifaces_v.n_children() {
            let e = ifaces_v.child_value(k);
            let name = e.child_value(0).str().unwrap_or("").to_string();
            let props: Props = e.child_value(1).get().unwrap_or_default();
            ifaces.insert(name.trim_start_matches(I).to_string(), props);
        }
        if let Some(p) = ifaces.get("Manager") {
            m.supported = p.get("SupportedFilesystems").and_then(|v| v.get::<Vec<String>>()).unwrap_or_default();
        }
        if let Some(p) = ifaces.get("Drive") {
            let mut dr = Drive {
                path: path.clone(),
                vendor: s(p, "Vendor"),
                model: s(p, "Model"),
                serial: s(p, "Serial"),
                revision: s(p, "Revision"),
                size: u(p, "Size"),
                removable: b(p, "Removable"),
                ejectable: b(p, "Ejectable"),
                can_power_off: b(p, "CanPowerOff"),
                media_removable: b(p, "MediaRemovable"),
                optical: b(p, "Optical"),
                media: s(p, "Media"),
                connection: s(p, "ConnectionBus"),
                rotation: i(p, "RotationRate") as i32,
                sort_key: s(p, "SortKey"),
                nvme: ifaces.contains_key("NVMe.Controller"),
                health: None,
            };
            if let Some(a) = ifaces.get("Drive.Ata").filter(|a| b(a, "SmartSupported") && b(a, "SmartEnabled")) {
                let t = d(a, "SmartTemperature");
                let pos = u(a, "SmartPowerOnSeconds");
                dr.health = Some(Health {
                    kind: "ATA",
                    failing: b(a, "SmartFailing"),
                    temperature_c: (t > 0.0).then(|| t - 273.15),
                    power_on_hours: (pos > 0).then(|| pos / 3600),
                    bad_sectors: Some(i(a, "SmartNumBadSectors")).filter(|n| *n >= 0),
                    attributes_failing: i(a, "SmartNumAttributesFailing"),
                    percent_used: None,
                    selftest: s(a, "SmartSelftestStatus"),
                    selftest_remaining: i(a, "SmartSelftestPercentRemaining"),
                });
            }
            if let Some(nv) = ifaces.get("NVMe.Controller") {
                let t = u(nv, "SmartTemperature") as f64;
                let warn = u(nv, "SmartCriticalWarning");
                dr.health = Some(Health {
                    kind: "NVMe",
                    failing: warn != 0,
                    temperature_c: (t > 0.0).then(|| t - 273.15),
                    power_on_hours: Some(u(nv, "SmartPowerOnHours")).filter(|h| *h > 0),
                    bad_sectors: None,
                    attributes_failing: warn as i64,
                    percent_used: Some(i(nv, "SmartPercentUsed")),
                    selftest: s(nv, "SmartSelftestStatus"),
                    selftest_remaining: i(nv, "SmartSelftestPercentRemaining"),
                });
            }
            m.drives.push(dr);
        }
        if let Some(p) = ifaces.get("Block") {
            let mut bl = Block {
                path: path.clone(),
                device: ays(p, "PreferredDevice"),
                size: u(p, "Size"),
                drive: o(p, "Drive"),
                id_type: s(p, "IdType"),
                id_usage: s(p, "IdUsage"),
                id_label: s(p, "IdLabel"),
                id_uuid: s(p, "IdUUID"),
                id_version: s(p, "IdVersion"),
                read_only: b(p, "ReadOnly"),
                hint_system: b(p, "HintSystem"),
                hint_ignore: b(p, "HintIgnore"),
                hint_name: s(p, "HintName"),
                crypto_backing: o(p, "CryptoBackingDevice"),
                ..Default::default()
            };
            if bl.device.is_empty() {
                bl.device = ays(p, "Device");
            }
            if let Some(t) = ifaces.get("PartitionTable") {
                bl.table = Some(s(t, "Type"));
            }
            if let Some(pt) = ifaces.get("Partition") {
                bl.partition = Some(Partition {
                    number: u(pt, "Number") as u32,
                    offset: u(pt, "Offset"),
                    size: u(pt, "Size"),
                    type_: s(pt, "Type"),
                    name: s(pt, "Name"),
                    table: o(pt, "Table"),
                    container: b(pt, "IsContainer"),
                });
            }
            if let Some(f) = ifaces.get("Filesystem") {
                bl.filesystem = Some(aay(f, "MountPoints"));
            }
            if let Some(e) = ifaces.get("Encrypted") {
                bl.encrypted = Some(o(e, "CleartextDevice"));
            }
            if let Some(l) = ifaces.get("Loop") {
                bl.loop_file = Some(ays(l, "BackingFile"));
            }
            if let Some(cfg) = p.get("Configuration") {
                for k in 0..cfg.n_children() {
                    let item = cfg.child_value(k);
                    if item.child_value(0).str() == Some("fstab") {
                        let d: Props = item.child_value(1).get().unwrap_or_default();
                        bl.fstab = Some((ays(&d, "dir"), ays(&d, "opts")));
                    }
                }
            }
            if let Some(sw) = ifaces.get("Swapspace") {
                bl.swap_active = Some(b(sw, "Active"));
            }
            m.blocks.push(bl);
        }
        if let Some(p) = ifaces.get("Job") {
            let objs = p.get("Objects").map(|v| (0..v.n_children()).filter_map(|n| v.child_value(n).str().map(String::from)).collect()).unwrap_or_default();
            m.jobs.push(Job {
                operation: s(p, "Operation"),
                progress: b(p, "ProgressValid").then(|| d(p, "Progress")),
                objects: objs,
                cancelable: b(p, "Cancelable"),
                path: path.clone(),
            });
        }
    }
    // Volumes déchiffrés : rattachés au disque de leur volume chiffré.
    let backing: Vec<(String, String)> = m.blocks.iter().filter(|b| !b.crypto_backing.is_empty()).map(|b| (b.path.clone(), b.crypto_backing.clone())).collect();
    for (clear, enc) in backing {
        let drive = m.block(&enc).map(|b| b.drive.clone()).unwrap_or_default();
        if let Some(b) = m.blocks.iter_mut().find(|b| b.path == clear) {
            b.drive = drive;
        }
    }
    // Images disque (loop) : un « disque » virtuel par périphérique loop utilisé.
    let loops: Vec<Block> = m.blocks.iter().filter(|b| b.drive.is_empty() && b.loop_file.as_deref().is_some_and(|f| !f.is_empty()) && b.partition.is_none()).cloned().collect();
    for l in loops {
        let path = format!("loop:{}", l.path);
        let file = l.loop_file.clone().unwrap_or_default();
        m.drives.push(Drive {
            path: path.clone(),
            vendor: String::new(),
            model: std::path::Path::new(&file).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "Image disque".into()),
            serial: file,
            size: l.size,
            connection: "loop".into(),
            rotation: 0,
            sort_key: format!("zz{}", l.device),
            ejectable: true,
            ..Default::default()
        });
        let table = l.path.clone();
        for b in m.blocks.iter_mut() {
            if b.path == table || b.partition.as_ref().is_some_and(|p| p.table == table) {
                b.drive = path.clone();
            }
        }
    }
    m.drives.sort_by(|a, b| (a.is_external(), &a.sort_key).cmp(&(b.is_external(), &b.sort_key)));
    m
}

/// Lecture synchrone (≈ quelques ms).
pub fn load(conn: &gio::DBusConnection) -> Result<Model, String> {
    let reply = conn
        .call_sync(Some(BUS), ROOT, "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None, None, gio::DBusCallFlags::NONE, 10_000, gio::Cancellable::NONE)
        .map_err(|e| e.message().to_string())?;
    Ok(parse(&reply))
}

/// Appelle `f` (avec un délai de regroupement de 250 ms) à chaque changement côté udisks :
/// disque branché / retiré, montage, formatage, progression d'une tâche…
pub fn watch(conn: &gio::DBusConnection, f: impl Fn() + 'static) -> Vec<gio::SignalSubscriptionId> {
    // GIO ne garde qu'une référence faible sur la connexion au bus système : si l'appelant
    // la relâche, elle est fermée et les abonnements meurent en silence. On la garde.
    std::mem::forget(conn.clone());
    let f = std::rc::Rc::new(f);
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut ids = vec![];
    for (iface, member) in [
        (Some("org.freedesktop.DBus.ObjectManager"), None),
        (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged")),
    ] {
        let (f, pending) = (f.clone(), pending.clone());
        #[allow(deprecated)]
        ids.push(conn.signal_subscribe(Some(BUS), iface, member, None, None, gio::DBusSignalFlags::NONE, move |_, _, _, _, _, _| {
            if pending.replace(true) {
                return;
            }
            let (f, pending) = (f.clone(), pending.clone());
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                pending.set(false);
                f();
            });
        }));
    }
    ids
}

/// Nom lisible d'un système de fichiers.
pub fn fs_name(t: &str) -> String {
    match t {
        "vfat" => "FAT32".into(),
        "exfat" => "exFAT".into(),
        "ntfs" => "NTFS".into(),
        "ext4" | "ext3" | "ext2" => t.to_uppercase().replace("EXT", "ext"),
        "btrfs" => "Btrfs".into(),
        "xfs" => "XFS".into(),
        "f2fs" => "F2FS".into(),
        "crypto_LUKS" => "LUKS (chiffré)".into(),
        "swap" => "Échange".into(),
        "iso9660" => "ISO 9660".into(),
        "udf" => "UDF".into(),
        "hfsplus" => "HFS+".into(),
        "apfs" => "APFS".into(),
        "" => "Aucun".into(),
        t => t.to_string(),
    }
}

/// « 29,8 Go » (puissances de 1000, comme les fabricants et Windows les affichent… à peu près).
pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    let (v, unit) = if b >= 1e12 {
        (b / 1e12, "To")
    } else if b >= 1e9 {
        (b / 1e9, "Go")
    } else if b >= 1e6 {
        (b / 1e6, "Mo")
    } else if b >= 1e3 {
        (b / 1e3, "Ko")
    } else {
        (b, "o")
    };
    if v >= 100.0 || unit == "o" { format!("{v:.0} {unit}") } else { format!("{v:.1} {unit}").replace('.', ",") }
}

/// Espace occupé / total d'un point de montage.
pub fn usage(mount: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(mount).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: chemin C valide, structure initialisée.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let total = s.f_blocks as u64 * s.f_frsize as u64;
    let free = s.f_bavail as u64 * s.f_frsize as u64;
    Some((total.saturating_sub(free), total))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sizes() {
        assert_eq!(super::human(30_752_000_000), "30,8 Go");
        assert_eq!(super::human(250_059_350_016), "250 Go");
        assert_eq!(super::fs_name("vfat"), "FAT32");
    }
}

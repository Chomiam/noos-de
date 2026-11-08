//! Opérations udisks2 (asynchrones, sur le fil principal GTK). udisks vérifie les droits
//! (polkit : la fenêtre d'élévation de noos-shell apparaît si besoin).

use std::os::fd::OwnedFd;

use gtk::{gio, glib, prelude::*};

use crate::udisks::BUS;

const I: &str = "org.freedesktop.UDisks2.";
pub type Done = Box<dyn FnOnce(Result<glib::Variant, String>)>;

/// Message d'erreur udisks rendu lisible.
pub fn explain(e: &glib::Error) -> String {
    let m = gio::DBusError::strip_remote_error(&mut e.clone()).then(|| e.message().to_string()).unwrap_or_else(|| e.message().to_string());
    let low = m.to_lowercase();
    if low.contains("not authorized") || low.contains("dismissed") {
        "Opération annulée (autorisation refusée).".into()
    } else if low.contains("target is busy") || low.contains("device or resource busy") || low.contains("is busy") {
        "Le volume est en cours d'utilisation : fermez les fichiers et fenêtres ouverts dessus, puis réessayez.".into()
    } else if low.contains("not mounted") {
        "Le volume n'est pas monté.".into()
    } else if low.contains("wrong passphrase") || low.contains("no key available") || low.contains("incorrect passphrase") {
        "Phrase secrète incorrecte.".into()
    } else {
        m.trim_start_matches("GDBus.Error:").to_string()
    }
}

/// Options `a{sv}`.
pub fn opts(pairs: &[(&str, glib::Variant)]) -> glib::Variant {
    let d = glib::VariantDict::new(None);
    for (k, v) in pairs {
        d.insert_value(k, v);
    }
    d.end()
}

pub fn call(conn: &gio::DBusConnection, path: &str, iface: &str, method: &str, args: glib::Variant, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    conn.call(Some(BUS), path, &format!("{I}{iface}"), method, Some(&args), None, gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION, i32::MAX, gio::Cancellable::NONE, move |r| {
        done(r.map_err(|e| explain(&e)))
    });
}

/// Arguments d'un appel : tuple des valeurs telles quelles (un `a{sv}` reste un `a{sv}`,
/// au lieu d'être emballé dans un `v` comme le ferait `(x,).to_variant()`).
pub fn tup(items: &[glib::Variant]) -> glib::Variant {
    glib::Variant::tuple_from_iter(items.iter().cloned())
}

fn no_opts() -> glib::Variant {
    opts(&[])
}

pub fn mount(conn: &gio::DBusConnection, block: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    call(conn, block, "Filesystem", "Mount", tup(&[no_opts()]), move |r| done(r.map(|v| v.child_value(0).str().unwrap_or("").to_string())));
}

pub fn unmount(conn: &gio::DBusConnection, block: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, block, "Filesystem", "Unmount", tup(&[no_opts()]), done);
}

/// Formate un bloc (partition ou disque entier) : `kind` = `ext4`, `ntfs`, `exfat`, `vfat`,
/// `btrfs`, `xfs`, `f2fs`… ou `gpt` / `dos` (nouvelle table de partitions), `empty`.
pub fn format(conn: &gio::DBusConnection, block: &str, kind: &str, label: &str, erase: bool, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    let mut o: Vec<(&str, glib::Variant)> = vec![("update-partition-type", true.to_variant())];
    if !label.is_empty() {
        o.push(("label", label.to_variant()));
    }
    if erase {
        o.push(("erase", "zero".to_variant()));
    }
    // Systèmes de fichiers Linux : le dossier racine appartient à l'utilisateur.
    if matches!(kind, "ext4" | "ext3" | "ext2" | "btrfs" | "xfs" | "f2fs") {
        o.push(("take-ownership", true.to_variant()));
    }
    call(conn, block, "Block", "Format", tup(&[kind.to_variant(), opts(&o)]), done);
}

/// Type de partition adapté au système de fichiers (Windows voit ses volumes NTFS / exFAT /
/// FAT comme « données de base », Linux les siens comme « données Linux »).
pub fn partition_type(table: &str, fstype: &str) -> &'static str {
    let windows = matches!(fstype, "ntfs" | "exfat" | "vfat");
    match (table, windows) {
        ("gpt", true) => "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
        ("gpt", false) => "0fc63daf-8483-4772-8e79-3d69d8477de4",
        (_, true) if fstype == "vfat" => "0x0c",
        (_, true) => "0x07",
        _ => "0x83",
    }
}

/// Crée une partition dans l'espace libre puis la formate.
#[allow(clippy::too_many_arguments)]
pub fn create_partition(conn: &gio::DBusConnection, table: &str, table_type: &str, offset: u64, size: u64, name: &str, fstype: &str, label: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    let mut fo: Vec<(&str, glib::Variant)> = vec![];
    if !label.is_empty() {
        fo.push(("label", label.to_variant()));
    }
    if matches!(fstype, "ext4" | "btrfs" | "xfs" | "f2fs") {
        fo.push(("take-ownership", true.to_variant()));
    }
    call(
        conn,
        table,
        "PartitionTable",
        "CreatePartitionAndFormat",
        tup(&[offset.to_variant(), size.to_variant(), partition_type(table_type, fstype).to_variant(), name.to_variant(), no_opts(), fstype.to_variant(), opts(&fo)]),
        done,
    );
}

pub fn delete_partition(conn: &gio::DBusConnection, block: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, block, "Partition", "Delete", tup(&[no_opts()]), done);
}

pub fn set_label(conn: &gio::DBusConnection, block: &str, label: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, block, "Filesystem", "SetLabel", tup(&[label.to_variant(), no_opts()]), done);
}

/// Vérifie (`repair = false`) ou répare le système de fichiers (démonté).
pub fn check(conn: &gio::DBusConnection, block: &str, repair: bool, done: impl FnOnce(Result<bool, String>) + 'static) {
    call(conn, block, "Filesystem", if repair { "Repair" } else { "Check" }, tup(&[no_opts()]), move |r| {
        done(r.map(|v| v.child_value(0).get::<bool>().unwrap_or(false)))
    });
}

pub fn unlock(conn: &gio::DBusConnection, block: &str, passphrase: &str, done: impl FnOnce(Result<String, String>) + 'static) {
    call(conn, block, "Encrypted", "Unlock", tup(&[passphrase.to_variant(), no_opts()]), move |r| {
        done(r.map(|v| v.child_value(0).str().unwrap_or("").to_string()))
    });
}

pub fn lock(conn: &gio::DBusConnection, block: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, block, "Encrypted", "Lock", tup(&[no_opts()]), done);
}

pub fn power_off(conn: &gio::DBusConnection, drive: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, drive, "Drive", "PowerOff", tup(&[no_opts()]), done);
}

pub fn eject(conn: &gio::DBusConnection, drive: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, drive, "Drive", "Eject", tup(&[no_opts()]), done);
}

pub fn smart_selftest(conn: &gio::DBusConnection, drive: &str, nvme: bool, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    if nvme {
        call(conn, drive, "NVMe.Controller", "SelftestStart", tup(&["short".to_variant(), no_opts()]), done);
    } else {
        call(conn, drive, "Drive.Ata", "SmartSelftestStart", tup(&["short".to_variant(), no_opts()]), done);
    }
}

/// Script privilégié `noos-disk-helper` (pkexec ; polkit garde l'autorisation quelques
/// minutes). Bloquant : à lancer hors du fil principal.
pub fn helper(args: &[String]) -> Result<String, String> {
    let path = std::env::var("NOOS_DISK_HELPER").unwrap_or_else(|_| "/opt/noos/libexec/noos-disk-helper".into());
    let out = std::process::Command::new("pkexec").arg(&path).args(args).output().map_err(|e| format!("pkexec : {e}"))?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        Some(126) => Err("Opération annulée (autorisation refusée).".into()),
        Some(127) => Err("Aucun agent d'authentification (polkit) disponible.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Utilisateur courant (uid, gid).
pub fn ids() -> (u32, u32) {
    // SAFETY: appels sans effet de bord.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// Propriétaire du dossier racine d'un volume monté.
pub fn owner(path: &str) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.uid())
}

/// Nom d'un utilisateur (uid).
pub fn user_name(uid: u32) -> String {
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|t| t.lines().find(|l| l.split(':').nth(2) == Some(&uid.to_string())).map(|l| l.split(':').next().unwrap_or("").to_string()))
        .unwrap_or_else(|| format!("utilisateur n° {uid}"))
}

/// « Données NTFS » → « Donnees-NTFS » (nom de dossier sous /mnt).
pub fn mount_name(label: &str) -> String {
    let mut s = String::new();
    for c in label.chars() {
        let c = match c {
            'à' | 'â' | 'ä' | 'À' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' => c,
            _ => '-',
        };
        if !(c == '-' && s.ends_with('-')) {
            s.push(c);
        }
    }
    let s: String = s.trim_matches(['-', '.']).chars().take(48).collect();
    if s.is_empty() { "Disque".into() } else { s }
}

pub fn loop_delete(conn: &gio::DBusConnection, block: &str, done: impl FnOnce(Result<glib::Variant, String>) + 'static) {
    call(conn, block, "Loop", "Delete", tup(&[no_opts()]), done);
}

/// Attributs SMART (ATA) : (id, nom, valeur normalisée, pire, seuil, valeur brute lisible).
pub fn smart_attributes(conn: &gio::DBusConnection, drive: &str, done: impl FnOnce(Result<Vec<(u8, String, i32, i32, i32, String)>, String>) + 'static) {
    call(conn, drive, "Drive.Ata", "SmartGetAttributes", tup(&[no_opts()]), move |r| {
        done(r.map(|v| {
            let list = v.child_value(0);
            (0..list.n_children())
                .map(|n| {
                    let a = list.child_value(n);
                    let id = a.child_value(0).get::<u8>().unwrap_or(0);
                    let name = a.child_value(1).str().unwrap_or("").to_string();
                    let value = a.child_value(3).get::<i32>().unwrap_or(-1);
                    let worst = a.child_value(4).get::<i32>().unwrap_or(-1);
                    let thr = a.child_value(5).get::<i32>().unwrap_or(-1);
                    let pretty = a.child_value(6).get::<i64>().unwrap_or(0);
                    let unit = a.child_value(7).get::<i32>().unwrap_or(0);
                    let pretty = match unit {
                        2 => format!("{} h", pretty / 3_600_000),
                        3 => format!("{} secteurs", pretty),
                        4 => format!("{:.0} °C", pretty as f64 / 1000.0 - 273.15),
                        _ => pretty.to_string(),
                    };
                    (id, name, value, worst, thr, pretty)
                })
                .collect()
        }))
    });
}

/// Ouvre le périphérique brut (`OpenForBenchmark`, `OpenForBackup`, `OpenForRestore`) :
/// udisks demande l'autorisation et renvoie un descripteur de fichier.
pub fn open_raw(conn: &gio::DBusConnection, block: &str, method: &str, done: impl FnOnce(Result<OwnedFd, String>) + 'static) {
    let args = if method == "OpenForBenchmark" { tup(&[opts(&[("writable", false.to_variant())])]) } else { tup(&[no_opts()]) };
    conn.call_with_unix_fd_list(
        Some(BUS),
        block,
        &format!("{I}Block"),
        method,
        Some(&args),
        None,
        gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
        i32::MAX,
        gio::UnixFDList::NONE,
        gio::Cancellable::NONE,
        move |r| {
            let r = r.map_err(|e| explain(&e)).and_then(|(v, list)| {
                let idx = v.child_value(0).get::<glib::variant::Handle>().map(|h| h.0).unwrap_or(0);
                let list = list.ok_or("aucun descripteur reçu")?;
                list.get(idx).map_err(|e| e.to_string())
            });
            done(r)
        },
    );
}

/// Démonte une liste de volumes l'un après l'autre.
pub fn unmount_all(conn: &gio::DBusConnection, mut blocks: Vec<String>, done: impl FnOnce(Result<(), String>) + 'static) {
    let Some(b) = blocks.pop() else { return done(Ok(())) };
    let conn2 = conn.clone();
    unmount(conn, &b, move |r| match r {
        Ok(_) => unmount_all(&conn2, blocks, done),
        Err(e) => done(Err(e)),
    });
}

/// Retrait en toute sécurité (voir aussi le shell) : démonter, verrouiller, éteindre.
pub fn safely_remove(conn: &gio::DBusConnection, m: &crate::udisks::Model, drive: &str, done: impl FnOnce(Result<(), String>) + 'static) {
    let mounted: Vec<String> = m.volumes(drive).iter().filter(|v| v.mounted_at().is_some()).map(|v| v.path.clone()).collect();
    let locks: Vec<String> = m.blocks.iter().filter(|b| b.drive == drive && b.encrypted.as_deref().is_some_and(|c| !c.is_empty())).map(|b| b.path.clone()).collect();
    let d = m.drive(drive).cloned().unwrap_or_default();
    let loop_block = drive.strip_prefix("loop:").map(String::from);
    let conn2 = conn.clone();
    unmount_all(conn, mounted, move |r| {
        if let Err(e) = r {
            return done(Err(e));
        }
        for l in &locks {
            lock(&conn2, l, |_| {});
        }
        let finish = move |r: Result<glib::Variant, String>| done(r.map(|_| ()));
        if let Some(lb) = loop_block {
            loop_delete(&conn2, &lb, finish);
        } else if d.can_power_off {
            power_off(&conn2, &d.path, finish);
        } else if d.ejectable {
            eject(&conn2, &d.path, finish);
        } else {
            finish(Ok(().to_variant()));
        }
    });
}


//! Disques (lsblk), plan de partitionnement et barre de visualisation.

use std::rc::Rc;

use gtk::prelude::*;
use serde::Deserialize;

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
/// Taille minimale du disque (système + quelques applications).
pub const MIN_DISK: u64 = 20 * GIB;

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub path: String,
    pub size: u64,
    pub fstype: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub path: String,
    pub model: String,
    pub size: u64,
    pub transport: String,
    pub removable: bool,
    pub rotational: bool,
    pub parts: Vec<Part>,
}

impl Disk {
    pub fn title(&self) -> String {
        let model = if self.model.trim().is_empty() { "Disque".to_string() } else { self.model.trim().to_string() };
        format!("{model} — {}", human(self.size))
    }

    pub fn kind(&self) -> &'static str {
        match self.transport.as_str() {
            "nvme" => "SSD NVMe",
            "usb" => "Clé ou disque USB",
            "mmc" => "Carte mémoire",
            _ if self.rotational => "Disque dur",
            _ => "SSD",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self.transport.as_str() {
            "usb" => "drive-removable-media-symbolic",
            "mmc" => "media-flash-symbolic",
            _ => "drive-harddisk-symbolic",
        }
    }

    /// Chemin de la partition n (nvme0n1 → nvme0n1p1, sda → sda1).
    pub fn part_path(&self, n: u32) -> String {
        let last = self.path.chars().last().unwrap_or('a');
        if last.is_ascii_digit() { format!("{}p{n}", self.path) } else { format!("{}{n}", self.path) }
    }
}

#[derive(Deserialize)]
struct Lsblk {
    blockdevices: Vec<Dev>,
}

#[derive(Deserialize)]
struct Dev {
    path: String,
    #[serde(default)]
    model: Option<String>,
    size: u64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    tran: Option<String>,
    #[serde(default)]
    rm: bool,
    #[serde(default)]
    rota: bool,
    #[serde(default)]
    ro: bool,
    #[serde(default)]
    fstype: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Dev>,
}

/// Disques installables : ni le support d'installation (monté sur /run/initramfs/live),
/// ni les périphériques en lecture seule, loop, zram ou lecteurs optiques.
pub fn disks() -> anyhow::Result<Vec<Disk>> {
    let out = std::process::Command::new("lsblk")
        .args(["-J", "-b", "-o", "PATH,MODEL,SIZE,TYPE,TRAN,RM,ROTA,RO,FSTYPE,LABEL,MOUNTPOINTS"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("lsblk : {}", String::from_utf8_lossy(&out.stderr));
    }
    parse(&out.stdout)
}

pub fn parse(json: &[u8]) -> anyhow::Result<Vec<Disk>> {
    let l: Lsblk = serde_json::from_slice(json)?;
    fn mounted_live(d: &Dev) -> bool {
        d.mountpoints.iter().flatten().any(|m| m.starts_with("/run/initramfs/live") || m == "/")
            || d.children.iter().any(mounted_live)
    }
    Ok(l.blockdevices
        .into_iter()
        .filter(|d| d.kind == "disk" && !d.ro && d.size >= GIB && !mounted_live(d) && !d.path.contains("zram"))
        .map(|d| Disk {
            parts: d
                .children
                .iter()
                .filter(|c| c.kind == "part")
                .map(|c| Part {
                    path: c.path.clone(),
                    size: c.size,
                    fstype: c.fstype.clone().unwrap_or_default(),
                    label: c.label.clone().unwrap_or_default(),
                })
                .collect(),
            model: d.model.unwrap_or_default(),
            transport: d.tran.unwrap_or_default(),
            removable: d.rm,
            rotational: d.rota,
            size: d.size,
            path: d.path,
        })
        .collect())
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e12 {
        format!("{:.1} To", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.0} Go", b / 1e9)
    } else {
        format!("{:.0} Mo", b / 1e6)
    }
}

pub fn uefi() -> bool {
    std::path::Path::new("/sys/firmware/efi").exists()
}

pub fn ram_bytes() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("MemTotal:").map(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())))
        .flatten()
        .map(|kb| kb * 1024)
        .unwrap_or(8 * GIB)
}

/// Swap par défaut : la taille de la mémoire (hibernation possible), entre 2 et 16 Gio.
pub fn default_swap_gib() -> u32 {
    ((ram_bytes() as f64 / GIB as f64).round() as u32).clamp(2, 16)
}

// ───────────────────────────── Plan ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    BiosBoot,
    Efi,
    Boot,
    Root,
    Swap,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::BiosBoot => "Amorçage BIOS",
            Role::Efi => "EFI",
            Role::Boot => "/boot",
            Role::Root => "Btrfs : @ (/) et @home (/home)",
            Role::Swap => "Swap",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Role::BiosBoot => "GRUB (BIOS)",
            Role::Efi => "FAT32, /boot/efi",
            Role::Boot => "ext4, noyaux et GRUB",
            Role::Root => "compression zstd",
            Role::Swap => "mémoire d'échange",
        }
    }

    pub fn color(self) -> (f64, f64, f64) {
        match self {
            Role::BiosBoot => (0.55, 0.55, 0.62),
            Role::Efi => (0.98, 0.70, 0.53),
            Role::Boot => (0.98, 0.89, 0.60),
            Role::Root => (0.54, 0.71, 0.98),
            Role::Swap => (0.80, 0.65, 0.97),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedPart {
    pub number: u32,
    pub role: Role,
    pub size: u64,
}

/// Disque entier : [BIOS boot 1 Mio | EFI 600 Mio] + /boot 1 Gio + Btrfs + [swap].
pub fn plan(disk_size: u64, uefi: bool, swap: Option<u32>) -> Vec<PlannedPart> {
    let mut parts = Vec::new();
    let mut n = 1;
    let mut push = |role, size| {
        parts.push(PlannedPart { number: n, role, size });
        n += 1;
    };
    if uefi {
        push(Role::Efi, 600 * MIB);
    } else {
        push(Role::BiosBoot, MIB);
    }
    push(Role::Boot, GIB);
    let reserved = 2 * MIB; // alignement + GPT de secours
    let used: u64 = (if uefi { 600 * MIB } else { MIB }) + GIB + reserved;
    let swap_bytes = swap.map(|g| g as u64 * GIB).unwrap_or(0);
    let root = disk_size.saturating_sub(used + swap_bytes);
    push(Role::Root, root);
    if swap.is_some() {
        push(Role::Swap, swap_bytes);
    }
    parts
}

// ───────────────────────────── Barre ─────────────────────────────

/// Segments affichés : (libellé, taille, couleur).
pub type Segment = (String, u64, (f64, f64, f64));

pub fn plan_segments(plan: &[PlannedPart]) -> Vec<Segment> {
    plan.iter().map(|p| (format!("{} ({})", p.role.label(), p.role.detail()), p.size, p.role.color())).collect()
}

pub fn current_segments(disk: &Disk) -> Vec<Segment> {
    const PALETTE: [(f64, f64, f64); 6] =
        [(0.58, 0.89, 0.84), (0.95, 0.55, 0.66), (0.65, 0.89, 0.63), (0.98, 0.70, 0.53), (0.54, 0.71, 0.98), (0.80, 0.65, 0.97)];
    let mut segs: Vec<Segment> = disk
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let name = p.path.rsplit('/').next().unwrap_or(&p.path).to_string();
            let what = match (p.label.is_empty(), p.fstype.is_empty()) {
                (false, _) => format!("{name} · {} ({})", p.label, p.fstype),
                (true, false) => format!("{name} · {}", p.fstype),
                _ => name,
            };
            (what, p.size, PALETTE[i % PALETTE.len()])
        })
        .collect();
    let used: u64 = disk.parts.iter().map(|p| p.size).sum();
    if disk.size > used + 64 * MIB {
        segs.push(("Espace libre".into(), disk.size - used, (0.27, 0.28, 0.35)));
    }
    segs
}

/// Barre horizontale proportionnelle + légende sous la barre.
pub fn bar(segments: &[Segment]) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let total: u64 = segments.iter().map(|s| s.1).sum::<u64>().max(1);
    let area = gtk::DrawingArea::new();
    area.set_content_height(34);
    area.set_hexpand(true);
    let segs: Rc<Vec<Segment>> = Rc::new(segments.to_vec());
    let s2 = segs.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let r = 8.0;
        // Coins arrondis : découpe globale.
        cr.new_sub_path();
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(r, r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        cr.clip();
        let mut x = 0.0;
        for (i, (_, size, (cr_, cg, cb))) in s2.iter().enumerate() {
            // Largeur minimale visible pour les petites partitions.
            let mut sw = (*size as f64 / total as f64 * w).max(6.0);
            if i == s2.len() - 1 {
                sw = w - x;
            }
            cr.set_source_rgb(*cr_, *cg, *cb);
            cr.rectangle(x, 0.0, sw, h);
            let _ = cr.fill();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            cr.rectangle(x + sw - 1.0, 0.0, 1.5, h);
            let _ = cr.fill();
            x += sw;
        }
    });
    v.append(&area);
    let legend = gtk::FlowBox::new();
    legend.set_selection_mode(gtk::SelectionMode::None);
    legend.set_max_children_per_line(4);
    legend.set_column_spacing(18);
    legend.set_row_spacing(4);
    for (label, size, (r, g, b)) in segs.iter() {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let dot = gtk::DrawingArea::new();
        dot.set_content_width(12);
        dot.set_content_height(12);
        dot.set_valign(gtk::Align::Center);
        let (r, g, b) = (*r, *g, *b);
        dot.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(r, g, b);
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, 6.0, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
        });
        item.append(&dot);
        let l = gtk::Label::new(Some(&format!("{label} — {}", human(*size))));
        l.add_css_class("caption");
        l.set_xalign(0.0);
        item.append(&l);
        legend.insert(&item, -1);
    }
    v.append(&legend);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_sizes() {
        let p = plan(256 * GIB, true, Some(8));
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].role, Role::Efi);
        assert_eq!(p[3].size, 8 * GIB);
        let total: u64 = p.iter().map(|x| x.size).sum();
        assert!(total <= 256 * GIB);
        let p = plan(64 * GIB, false, None);
        assert_eq!(p[0].role, Role::BiosBoot);
        assert_eq!(p.len(), 3);
    }

    #[test]
    fn lsblk_filter() {
        let j = br#"{"blockdevices":[
          {"path":"/dev/sda","model":"USB","size":16000000000,"type":"disk","tran":"usb","rm":true,"rota":false,"ro":false,"fstype":"iso9660","label":"NOOS","mountpoints":["/run/initramfs/live"],"children":[]},
          {"path":"/dev/nvme0n1","model":"Samsung 980","size":512110190592,"type":"disk","tran":"nvme","rm":false,"rota":false,"ro":false,"fstype":null,"label":null,"mountpoints":[null],
           "children":[{"path":"/dev/nvme0n1p1","size":629145600,"type":"part","fstype":"vfat","label":"EFI","mountpoints":[null]}]},
          {"path":"/dev/zram0","size":8000000000,"type":"disk","ro":false,"mountpoints":["[SWAP]"]}
        ]}"#;
        let d = parse(j).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].part_path(2), "/dev/nvme0n1p2");
        assert_eq!(d[0].parts.len(), 1);
    }
}

//! Détection du matériel graphique et plan « pilotes et codecs » de l'installation.
//!
//! | Matériel                                   | Installé (avec RPM Fusion)                               |
//! |--------------------------------------------|----------------------------------------------------------|
//! | tous                                       | codecs complets : ffmpeg, GStreamer bad-freeworld / ugly / libav, OpenH264, x264, x265, LAME, libdvdcss (tainted) |
//! | AMD                                        | VA-API complet (`mesa-va-drivers-freeworld`)              |
//! | Intel                                      | VA-API (`intel-media-driver`, `libva-intel-driver`)       |
//! | NVIDIA Turing et + (GTX 16xx, RTX…)        | pilote NVIDIA actuel (`akmod-nvidia`), CUDA, VA-API       |
//! | NVIDIA Maxwell / Pascal / Volta            | branche 580 (`akmod-nvidia-580xx`)                         |
//! | NVIDIA Kepler et plus ancien               | pilote libre nouveau (la branche 470 n'a pas GBM : pas de Wayland) |
//! | machine virtuelle                          | outils invités (QEMU/KVM, VMware, VirtualBox, Hyper-V)   |
//!
//! Sans RPM Fusion : seulement les codecs et outils disponibles dans Fedora.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaGen {
    /// Turing (TU1xx, GTX 1650 et +) et plus récentes : pilote actuel.
    Modern,
    /// Maxwell, Pascal, Volta : branche 580.
    Legacy580,
    /// Kepler et plus ancien : pas de pilote propriétaire compatible Wayland/GBM.
    TooOld,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Vendor {
    Amd,
    Intel,
    Nvidia(NvidiaGen),
    /// Carte graphique virtuelle (virtio, QXL, VMware SVGA, VirtualBox, Hyper-V).
    Virtual,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub vendor: Vendor,
    pub vendor_id: u16,
    pub device_id: u16,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct Hardware {
    pub gpus: Vec<Gpu>,
    /// Hyperviseur (`kvm`, `qemu`, `vmware`, `oracle`, `microsoft`…) si machine virtuelle.
    pub vm: Option<String>,
}

/// Génération d'une carte NVIDIA d'après l'identifiant de sa puce (plages PCI par architecture).
pub fn nvidia_gen(device: u16) -> NvidiaGen {
    match device {
        // Turing (TU102 0x1E0x…, TU116 0x21xx), Ampere (0x22xx-0x25xx), Ada (0x26xx-0x28xx), Blackwell…
        0x1E00.. => NvidiaGen::Modern,
        // Maxwell (GM107 0x138x, GM108 0x134x, GM20x 0x13Cx / 0x17xx), Pascal (0x15xx-0x1Dxx), Volta (0x1Dxx)
        0x1340..=0x1DFF => NvidiaGen::Legacy580,
        _ => NvidiaGen::TooOld,
    }
}

fn read_hex(path: &Path) -> Option<u32> {
    let s = std::fs::read_to_string(path).ok()?;
    u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

/// Nom commercial d'après la base PCI (`/usr/share/hwdata/pci.ids`), si elle est présente.
fn pci_name(vendor: u16, device: u16) -> Option<String> {
    let text = std::fs::read_to_string("/usr/share/hwdata/pci.ids").ok()?;
    let (v, d) = (format!("{vendor:04x}"), format!("\t{device:04x}"));
    let mut in_vendor = false;
    let mut vendor_name = String::new();
    for line in text.lines() {
        if !line.starts_with('\t') && !line.starts_with('#') {
            in_vendor = line.starts_with(&v);
            if in_vendor {
                vendor_name = line[4..].trim().to_string();
            }
        } else if in_vendor && line.starts_with(&d) && !line.starts_with("\t\t") {
            let name = line[d.len()..].trim();
            // « GA106 [GeForce RTX 3060] » → « GeForce RTX 3060 »
            let name = match (name.find('['), name.rfind(']')) {
                (Some(a), Some(b)) if b > a => &name[a + 1..b],
                _ => name,
            };
            let short = vendor_name.split_whitespace().next().unwrap_or("").replace(',', "");
            return Some(format!("{short} {name}"));
        }
    }
    None
}

pub fn classify(vendor: u16, device: u16) -> Vendor {
    match vendor {
        0x1002 | 0x1022 => Vendor::Amd,
        0x8086 => Vendor::Intel,
        0x10de => Vendor::Nvidia(nvidia_gen(device)),
        // virtio-gpu, QXL / bochs (Red Hat), VMware SVGA, VirtualBox, Hyper-V
        0x1af4 | 0x1b36 | 0x1234 | 0x15ad | 0x80ee | 0x1414 => Vendor::Virtual,
        _ => Vendor::Other,
    }
}

pub fn detect() -> Hardware {
    let mut gpus = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/bus/pci/devices") {
        for dev in rd.flatten().map(|e| e.path()) {
            // Classe 0x03xxxx : contrôleur d'affichage (VGA, 3D, autre).
            if read_hex(&dev.join("class")).is_none_or(|c| c >> 16 != 0x03) {
                continue;
            }
            let (Some(v), Some(d)) = (read_hex(&dev.join("vendor")), read_hex(&dev.join("device"))) else { continue };
            let (v, d) = (v as u16, d as u16);
            gpus.push(Gpu { vendor: classify(v, d), vendor_id: v, device_id: d, name: pci_name(v, d).unwrap_or_else(|| format!("{v:04x}:{d:04x}")) });
        }
    }
    gpus.sort_by_key(|g| (g.vendor_id, g.device_id));
    let vm = std::process::Command::new("systemd-detect-virt")
        .arg("--vm")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty() && s != "none");
    Hardware { gpus, vm }
}

/// Ce que l'installation fera pour ce matériel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// Paquets à installer (avec `--allowerasing` : ffmpeg remplace ffmpeg-free, etc.).
    pub packages: Vec<String>,
    /// Dépôts « tainted » de RPM Fusion (libdvdcss, firmwares restreints).
    pub tainted: bool,
    /// Module NVIDIA à compiler pendant l'installation (akmods).
    pub akmods: bool,
    /// Arguments du noyau en plus.
    pub kernel_args: Vec<String>,
    /// Résumé lisible (récapitulatif et journal).
    pub summary: Vec<String>,
}

fn push(v: &mut Vec<String>, items: &[&str]) {
    for i in items {
        if !v.iter().any(|x| x == i) {
            v.push(i.to_string());
        }
    }
}

pub fn plan(hw: &Hardware, rpmfusion: bool) -> Plan {
    let mut p = Plan::default();
    // Codecs : toujours ce que Fedora fournit ; avec RPM Fusion, la totale.
    push(&mut p.packages, &["gstreamer1-plugin-openh264", "mozilla-openh264", "gstreamer1-plugin-libav", "lame", "libva-utils"]);
    if rpmfusion {
        p.tainted = true;
        push(
            &mut p.packages,
            &["ffmpeg", "gstreamer1-plugins-bad-freeworld", "gstreamer1-plugins-ugly", "x264", "x265", "libdvdcss"],
        );
        p.summary.push("Codecs complets : H.264/H.265, AAC, MP3, DVD (ffmpeg, GStreamer, RPM Fusion)".into());
    } else {
        p.summary.push("Codecs libres de Fedora (RPM Fusion désactivé : pas de H.265 ni de pilote NVIDIA)".into());
    }

    let real: Vec<&Gpu> = hw.gpus.iter().filter(|g| g.vendor != Vendor::Virtual).collect();
    for g in &hw.gpus {
        match &g.vendor {
            Vendor::Amd => {
                if rpmfusion {
                    push(&mut p.packages, &["mesa-va-drivers-freeworld"]);
                }
                p.summary.push(format!("{} : pilote libre AMDGPU (Mesa){}", g.name, if rpmfusion { ", décodage vidéo complet" } else { "" }));
            }
            Vendor::Intel => {
                if rpmfusion {
                    push(&mut p.packages, &["intel-media-driver", "libva-intel-driver"]);
                }
                p.summary.push(format!("{} : pilote libre Intel (Mesa){}", g.name, if rpmfusion { ", décodage vidéo matériel" } else { "" }));
            }
            Vendor::Nvidia(generation) => match (generation, rpmfusion) {
                (NvidiaGen::Modern, true) => {
                    push(&mut p.packages, &["akmod-nvidia", "xorg-x11-drv-nvidia-cuda", "libva-nvidia-driver"]);
                    p.akmods = true;
                    p.summary.push(format!("{} : pilote NVIDIA propriétaire (branche actuelle), CUDA, décodage vidéo", g.name));
                }
                (NvidiaGen::Legacy580, true) => {
                    push(&mut p.packages, &["akmod-nvidia-580xx", "xorg-x11-drv-nvidia-580xx-cuda", "libva-nvidia-driver"]);
                    p.akmods = true;
                    p.summary.push(format!("{} : pilote NVIDIA propriétaire, branche 580 (Maxwell, Pascal, Volta)", g.name));
                }
                (NvidiaGen::TooOld, _) => {
                    p.summary.push(format!(
                        "{} : pilote libre nouveau (le pilote NVIDIA de cette génération ne prend pas en charge Wayland)",
                        g.name
                    ));
                }
                (_, false) => p.summary.push(format!("{} : pilote libre nouveau (activez RPM Fusion pour le pilote NVIDIA)", g.name)),
            },
            Vendor::Virtual => p.summary.push(format!("{} : carte graphique virtuelle", g.name)),
            Vendor::Other => p.summary.push(format!("{} : pilote libre (Mesa)", g.name)),
        }
    }
    if p.akmods {
        // KMS du pilote NVIDIA (indispensable au compositeur Wayland) ; nouveau est mis de côté.
        for a in ["rd.driver.blacklist=nouveau", "modprobe.blacklist=nouveau", "nvidia-drm.modeset=1", "nvidia-drm.fbdev=1"] {
            p.kernel_args.push(a.into());
        }
        push(&mut p.packages, &["kernel-devel"]);
    }
    // Portable hybride (Intel/AMD + NVIDIA) : bascule de carte graphique par application.
    if real.len() > 1 {
        push(&mut p.packages, &["switcheroo-control"]);
        p.summary.push("Plusieurs cartes graphiques : bascule par application (switcheroo-control)".into());
    }

    if let Some(vm) = &hw.vm {
        let (tools, label): (&[&str], &str) = match vm.as_str() {
            "kvm" | "qemu" => (&["qemu-guest-agent", "spice-vdagent", "spice-webdavd"], "QEMU/KVM"),
            "vmware" => (&["open-vm-tools", "open-vm-tools-desktop"], "VMware"),
            "oracle" => (&["virtualbox-guest-additions"], "VirtualBox"),
            "microsoft" => (&["hyperv-daemons"], "Hyper-V"),
            _ => (&[], "inconnue"),
        };
        push(&mut p.packages, tools);
        p.summary.push(if tools.is_empty() {
            format!("Machine virtuelle ({vm})")
        } else {
            format!("Machine virtuelle {label} : outils invités (presse-papiers, redimensionnement)")
        });
    }
    if hw.gpus.is_empty() && hw.vm.is_none() {
        p.summary.push("Aucune carte graphique détectée : pilotes libres (Mesa)".into());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_generations() {
        assert_eq!(nvidia_gen(0x1f82), NvidiaGen::Modern); // GTX 1650 (TU117)
        assert_eq!(nvidia_gen(0x2504), NvidiaGen::Modern); // RTX 3060
        assert_eq!(nvidia_gen(0x2684), NvidiaGen::Modern); // RTX 4090
        assert_eq!(nvidia_gen(0x1b80), NvidiaGen::Legacy580); // GTX 1080
        assert_eq!(nvidia_gen(0x1c82), NvidiaGen::Legacy580); // GTX 1050 Ti
        assert_eq!(nvidia_gen(0x1380), NvidiaGen::Legacy580); // GTX 750 Ti
        assert_eq!(nvidia_gen(0x13c2), NvidiaGen::Legacy580); // GTX 970
        assert_eq!(nvidia_gen(0x1184), NvidiaGen::TooOld); // GTX 770 (Kepler)
        assert_eq!(nvidia_gen(0x0dc4), NvidiaGen::TooOld); // GTS 450 (Fermi)
    }

    #[test]
    fn plans() {
        let gpu = |v, d| Gpu { vendor: classify(v, d), vendor_id: v, device_id: d, name: "carte".into() };
        let p = plan(&Hardware { gpus: vec![gpu(0x10de, 0x2504)], vm: None }, true);
        assert!(p.akmods && p.packages.contains(&"akmod-nvidia".to_string()));
        assert!(p.kernel_args.contains(&"nvidia-drm.modeset=1".to_string()));
        assert!(p.packages.contains(&"ffmpeg".to_string()) && p.tainted);

        let p = plan(&Hardware { gpus: vec![gpu(0x10de, 0x1b80)], vm: None }, true);
        assert!(p.packages.contains(&"akmod-nvidia-580xx".to_string()));

        let p = plan(&Hardware { gpus: vec![gpu(0x10de, 0x1184)], vm: None }, true);
        assert!(!p.akmods && p.kernel_args.is_empty());

        let p = plan(&Hardware { gpus: vec![gpu(0x8086, 0x3e91), gpu(0x10de, 0x1f82)], vm: None }, true);
        assert!(p.packages.contains(&"intel-media-driver".to_string()) && p.packages.contains(&"switcheroo-control".to_string()));

        let p = plan(&Hardware { gpus: vec![gpu(0x1002, 0x73bf)], vm: None }, false);
        assert!(!p.packages.contains(&"mesa-va-drivers-freeworld".to_string()) && !p.tainted);

        let p = plan(&Hardware { gpus: vec![gpu(0x1af4, 0x1050)], vm: Some("kvm".into()) }, true);
        assert!(p.packages.contains(&"qemu-guest-agent".to_string()) && !p.akmods);
    }
}

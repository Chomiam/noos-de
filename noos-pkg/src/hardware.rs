//! Paquets recommandés selon le matériel (bandeau d'Explorer) : microcode du processeur,
//! pilote NVIDIA, firmwares des cartes Wi-Fi / graphiques / réseau, règles udev des manettes,
//! lecteur d'empreintes, gestion d'énergie des portables. Sondage par `/sys` et `/proc`
//! (sans root), puis `isenkram-lookup` (Debian, modalias → paquets) s'il est installé.

use std::collections::HashSet;

use crate::exec::capture_codes;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    /// Fabricants PCI des cartes graphiques (0x10de NVIDIA, 0x1002 AMD, 0x8086 Intel).
    pub gpu_vendors: Vec<u16>,
    pub cpu_vendor: String,
    /// Modules du noyau chargés (`/sys/module`).
    pub modules: HashSet<String>,
    pub joystick: bool,
    pub laptop: bool,
    /// Périphériques USB (fabricant, produit).
    pub usb: Vec<(u16, u16)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    pub package: String,
    pub title: String,
    pub reason: String,
    /// Dans les composants non-free / contrib de Debian.
    pub nonfree: bool,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn hex(s: &str) -> Option<u16> {
    u16::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

pub fn probe() -> Probe {
    let mut p = Probe::default();
    if let Ok(rd) = std::fs::read_dir("/sys/bus/pci/devices") {
        for d in rd.filter_map(Result::ok).map(|e| e.path()) {
            let class = read(&format!("{}/class", d.display()));
            if class.trim().starts_with("0x03") {
                if let Some(v) = hex(&read(&format!("{}/vendor", d.display()))) {
                    p.gpu_vendors.push(v);
                }
            }
        }
    }
    p.cpu_vendor = read("/proc/cpuinfo").lines().find(|l| l.starts_with("vendor_id")).and_then(|l| l.split(':').nth(1)).unwrap_or("").trim().to_string();
    p.modules = std::fs::read_dir("/sys/module").map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    p.joystick = std::fs::read_dir("/sys/class/input").is_ok_and(|rd| rd.filter_map(Result::ok).any(|e| e.file_name().to_string_lossy().starts_with("js")));
    p.laptop = std::fs::read_dir("/sys/class/power_supply").is_ok_and(|rd| {
        rd.filter_map(Result::ok).any(|e| read(&format!("{}/type", e.path().display())).trim() == "Battery" && read(&format!("{}/scope", e.path().display())).trim() != "Device")
    });
    if let Ok(rd) = std::fs::read_dir("/sys/bus/usb/devices") {
        for d in rd.filter_map(Result::ok).map(|e| e.path()) {
            if let (Some(v), Some(pr)) = (hex(&read(&format!("{}/idVendor", d.display()))), hex(&read(&format!("{}/idProduct", d.display())))) {
                p.usb.push((v, pr));
            }
        }
    }
    p
}

/// Fabricants USB de lecteurs d'empreintes courants (Synaptics, Goodix, Validity, Elan, AuthenTec).
const FINGERPRINT_VENDORS: &[u16] = &[0x06cb, 0x27c6, 0x138a, 0x04f3, 0x08ff, 0x1c7a];

/// Recommandations possibles pour un matériel (avant de vérifier ce qui est installé).
pub fn candidates(p: &Probe) -> Vec<Recommendation> {
    let mut out = Vec::new();
    let mut add = |package: &str, title: &str, reason: &str, nonfree: bool| {
        if !out.iter().any(|r: &Recommendation| r.package == package) {
            out.push(Recommendation { package: package.into(), title: title.into(), reason: reason.into(), nonfree });
        }
    };
    match p.cpu_vendor.as_str() {
        "GenuineIntel" => add("intel-microcode", "Microcode du processeur Intel", "Corrections de sécurité et de stabilité du processeur", true),
        "AuthenticAMD" => add("amd64-microcode", "Microcode du processeur AMD", "Corrections de sécurité et de stabilité du processeur", true),
        _ => {}
    }
    if p.gpu_vendors.contains(&0x10de) {
        add("nvidia-driver", "Pilote NVIDIA propriétaire", "Carte graphique NVIDIA détectée : meilleures performances en jeu (Vulkan, CUDA)", true);
    }
    if p.gpu_vendors.contains(&0x1002) || p.modules.contains("amdgpu") {
        add("firmware-amd-graphics", "Firmware des cartes graphiques AMD", "Nécessaire à l'accélération graphique des cartes AMD", true);
    }
    if p.modules.contains("i915") || p.modules.contains("xe") {
        add("firmware-intel-graphics", "Firmware des cartes graphiques Intel", "Économie d'énergie (DMC) et décodage vidéo (GuC/HuC)", true);
    }
    for (module, package, title) in [
        ("iwlwifi", "firmware-iwlwifi", "Firmware Wi-Fi et Bluetooth Intel"),
        ("r8169", "firmware-realtek", "Firmware des cartes réseau Realtek"),
        ("rtw88_core", "firmware-realtek", "Firmware Wi-Fi Realtek"),
        ("rtw89_core", "firmware-realtek", "Firmware Wi-Fi Realtek"),
        ("ath10k_core", "firmware-atheros", "Firmware Wi-Fi Qualcomm Atheros"),
        ("ath11k", "firmware-atheros", "Firmware Wi-Fi Qualcomm Atheros"),
        ("ath12k", "firmware-atheros", "Firmware Wi-Fi Qualcomm Atheros"),
        ("brcmfmac", "firmware-brcm80211", "Firmware Wi-Fi Broadcom"),
        ("mt7921e", "firmware-mediatek", "Firmware Wi-Fi MediaTek"),
        ("mt7925e", "firmware-mediatek", "Firmware Wi-Fi MediaTek"),
    ] {
        if p.modules.contains(module) {
            add(package, title, "Matériel détecté : sans lui, il peut fonctionner mal ou pas du tout", true);
        }
    }
    if p.joystick {
        add("steam-devices", "Règles des manettes et casques de jeu", "Manette branchée : reconnue par Steam et les jeux sans droits root", true);
    }
    if p.usb.iter().any(|(v, _)| FINGERPRINT_VENDORS.contains(v)) {
        add("fprintd", "Lecteur d'empreintes digitales", "Lecteur d'empreintes détecté : déverrouillage par le doigt", false);
        add("libpam-fprintd", "Connexion par empreinte", "Utiliser l'empreinte pour la connexion et sudo", false);
    }
    if p.laptop {
        add("auto-cpufreq", "Économie d'énergie des portables", "Portable détecté : plus d'autonomie (réglage automatique du processeur)", false);
    }
    out
}

/// `isenkram-lookup` : paquets proposés par Debian pour le matériel (modalias).
fn isenkram() -> Vec<String> {
    capture_codes(&["isenkram-lookup"], &[0]).map(|o| o.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

/// Recommandations à proposer : pas encore installées et disponibles dans les dépôts actifs.
pub fn recommendations() -> Vec<Recommendation> {
    let mut recs = candidates(&probe());
    for pkg in isenkram() {
        if !recs.iter().any(|r| r.package == pkg) {
            recs.push(Recommendation { package: pkg.clone(), title: pkg, reason: "Proposé par Debian pour votre matériel (isenkram)".into(), nonfree: false });
        }
    }
    if recs.is_empty() {
        return recs;
    }
    let names: Vec<&str> = recs.iter().map(|r| r.package.as_str()).collect();
    let mut argv = vec!["apt-cache", "policy"];
    argv.extend(names.iter());
    let policy = capture_codes(&argv, &[0]).map(|o| crate::backends::apt::parse_policy(&o)).unwrap_or_default();
    recs.retain(|r| policy.get(&r.package).is_some_and(|p| p.installed.is_none() && p.candidate.is_some()));
    recs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_nvidia_intel() {
        let p = Probe {
            gpu_vendors: vec![0x8086, 0x10de],
            cpu_vendor: "GenuineIntel".into(),
            modules: ["i915", "iwlwifi", "snd_hda_intel"].iter().map(|s| s.to_string()).collect(),
            joystick: true,
            laptop: true,
            usb: vec![(0x27c6, 0x5110)],
        };
        let r: Vec<String> = candidates(&p).into_iter().map(|r| r.package).collect();
        assert_eq!(
            r,
            ["intel-microcode", "nvidia-driver", "firmware-intel-graphics", "firmware-iwlwifi", "steam-devices", "fprintd", "libpam-fprintd", "auto-cpufreq"]
        );
    }

    #[test]
    fn pc_amd_sans_rien() {
        let p = Probe { gpu_vendors: vec![0x1002], cpu_vendor: "AuthenticAMD".into(), ..Default::default() };
        let r: Vec<String> = candidates(&p).into_iter().map(|r| r.package).collect();
        assert_eq!(r, ["amd64-microcode", "firmware-amd-graphics"]);
    }
}

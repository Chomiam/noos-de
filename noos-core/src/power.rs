//! Énergie : batterie, profils d'alimentation et détection des portables, partagés par la
//! barre des tâches (`noos-shell`), Paramètres et l'installateur.
//!
//! - Batterie : le shell lit UPower par D-Bus ; à défaut (UPower absent), [`read_sysfs`] lit
//!   `/sys/class/power_supply`.
//! - Profils : power-profiles-daemon par D-Bus, ou **auto-cpufreq** sur les portables installés
//!   par NOOS (power-profiles-daemon y est masqué : les deux se disputeraient le gouverneur du
//!   processeur). auto-cpufreq se pilote par `auto-cpufreq --force=…` (root) : le shell passe
//!   par `pkexec` [`HELPER`] (action polkit `org.noos.power.helper`, sans mot de passe pour la
//!   session active) et lit le choix en cours dans [`ACF_OVERRIDE`].

use std::path::{Path, PathBuf};

/// Assistant root (pkexec) : `noos-power-helper profile <power-saver|balanced|performance>`.
pub const HELPER: &str = "/usr/libexec/noos-power-helper";
/// Dossier d'état d'auto-cpufreq (le paquet Debian ne le crée pas : l'assistant s'en charge).
pub const ACF_DIR: &str = "/opt/auto-cpufreq";
/// Choix forcé d'auto-cpufreq (pickle Python d'une chaîne) ; absent = mode automatique.
pub const ACF_OVERRIDE: &str = "/opt/auto-cpufreq/override.pickle";

// ───────────────────────────── Batterie ─────────────────────────────

/// État de charge (valeurs de `org.freedesktop.UPower.Device.State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChargeState {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    Full,
    /// Branché mais pas en charge (seuil de charge, batterie maintenue).
    PendingCharge,
    PendingDischarge,
}

impl ChargeState {
    pub fn from_upower(v: u32) -> Self {
        match v {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::Full,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }

    /// `status` de `/sys/class/power_supply/BAT*`.
    pub fn from_sysfs(s: &str) -> Self {
        match s.trim() {
            "Charging" => Self::Charging,
            "Discharging" => Self::Discharging,
            "Full" => Self::Full,
            "Not charging" => Self::PendingCharge,
            _ => Self::Unknown,
        }
    }

    /// Sur le secteur (en charge, chargée ou maintenue).
    pub fn plugged(self) -> bool {
        matches!(self, Self::Charging | Self::Full | Self::PendingCharge)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    /// Charge restante, 0 à 100.
    pub percent: f64,
    pub state: ChargeState,
    /// Secondes (0 = inconnu).
    pub time_to_empty: i64,
    pub time_to_full: i64,
    /// Santé : capacité actuelle / capacité d'origine, en % (0 = inconnue).
    pub capacity: f64,
    /// Puissance instantanée en watts (0 = inconnue).
    pub rate: f64,
    /// Fabricant et modèle de la batterie.
    pub model: String,
    /// Cycles de charge (0 = inconnu).
    pub cycles: u32,
}

/// Seuils de couleur de l'icône (comme Windows : orange sous 20 %, rouge sous 10 %).
pub const LOW: f64 = 20.0;
pub const CRITICAL: f64 = 10.0;

impl Battery {
    pub fn low(&self) -> bool {
        !self.state.plugged() && self.percent <= LOW
    }

    pub fn critical(&self) -> bool {
        !self.state.plugged() && self.percent <= CRITICAL
    }

    /// Autonomie ou temps de charge : « 2 h 15 restantes », « Pleine charge dans 40 min »…
    pub fn status_text(&self) -> String {
        match self.state {
            ChargeState::Charging if self.time_to_full > 0 => format!("Pleine charge dans {}", duration(self.time_to_full)),
            ChargeState::Charging => "En charge".into(),
            ChargeState::Full => "Chargée, sur secteur".into(),
            ChargeState::PendingCharge => "Branchée, pas en charge".into(),
            ChargeState::Discharging | ChargeState::PendingDischarge if self.time_to_empty > 0 => {
                let d = duration(self.time_to_empty);
                // « 1 h restante », « 2 h 15 restantes », « 40 min restantes »
                if self.time_to_empty < 7200 && self.time_to_empty >= 3600 && self.time_to_empty % 3600 < 60 {
                    format!("{d} restante")
                } else {
                    format!("{d} restantes")
                }
            }
            ChargeState::Discharging | ChargeState::PendingDischarge => "Sur batterie, estimation en cours…".into(),
            ChargeState::Empty => "Batterie vide".into(),
            ChargeState::Unknown => "Sur batterie".into(),
        }
    }

    /// Info-bulle de l'icône de la barre.
    pub fn tooltip(&self) -> String {
        format!("Batterie : {:.0} %\n{}", self.percent, self.status_text())
    }
}

/// Durée lisible : « 2 h 15 », « 1 h », « 40 min », « moins d'une minute ».
pub fn duration(secs: i64) -> String {
    let m = (secs + 30) / 60;
    match (m / 60, m % 60) {
        (0, 0) => "moins d'une minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02}"),
    }
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn num(p: &Path) -> Option<f64> {
    read(p)?.parse().ok()
}

/// Batteries du système (pas celles des souris, casques… : `scope` = `Device`).
pub fn battery_dirs(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| read(&p.join("type")).as_deref() == Some("Battery"))
        .filter(|p| read(&p.join("scope")).as_deref() != Some("Device"))
        .filter(|p| read(&p.join("present")).as_deref() != Some("0"))
        .collect();
    v.sort();
    v
}

/// Lecture directe de `/sys/class/power_supply` (repli sans UPower) : première batterie.
pub fn read_sysfs_in(root: &Path) -> Option<Battery> {
    let dir = battery_dirs(root).into_iter().next()?;
    let f = |k: &str| num(&dir.join(k));
    // Énergie (µWh) ou charge (µAh) selon le pilote.
    let (now, full, design, rate) = match (f("energy_now"), f("energy_full")) {
        (Some(n), Some(fu)) => (n, fu, f("energy_full_design"), f("power_now")),
        _ => (f("charge_now").unwrap_or(0.0), f("charge_full").unwrap_or(0.0), f("charge_full_design"), f("current_now")),
    };
    let percent = f("capacity").unwrap_or(if full > 0.0 { now / full * 100.0 } else { 0.0 }).clamp(0.0, 100.0);
    let state = ChargeState::from_sysfs(&read(&dir.join("status")).unwrap_or_default());
    let rate_raw = rate.unwrap_or(0.0).abs();
    let (mut tte, mut ttf) = (0, 0);
    if rate_raw > 0.0 {
        match state {
            ChargeState::Discharging => tte = (now / rate_raw * 3600.0) as i64,
            ChargeState::Charging => ttf = ((full - now).max(0.0) / rate_raw * 3600.0) as i64,
            _ => {}
        }
    }
    // Watts seulement pour energy/power (en charge/courant, il faudrait la tension).
    let watts = if f("power_now").is_some() { rate_raw / 1e6 } else { f("voltage_now").map(|v| rate_raw * v / 1e12).unwrap_or(0.0) };
    let capacity = design.filter(|d| *d > 0.0).map(|d| (full / d * 100.0).min(100.0)).unwrap_or(0.0);
    let model = [read(&dir.join("manufacturer")), read(&dir.join("model_name"))].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    Some(Battery {
        percent,
        state,
        time_to_empty: tte,
        time_to_full: ttf,
        capacity,
        rate: watts,
        model,
        cycles: f("cycle_count").unwrap_or(0.0) as u32,
    })
}

pub fn read_sysfs() -> Option<Battery> {
    read_sysfs_in(Path::new("/sys/class/power_supply"))
}

// ───────────────────────────── Portable ─────────────────────────────

/// Types de châssis SMBIOS des portables : 8 Portable, 9 Laptop, 10 Notebook, 14 Sub Notebook,
/// 31 Convertible, 32 Detachable.
pub fn chassis_is_portable(code: u32) -> bool {
    matches!(code, 8 | 9 | 10 | 14 | 31 | 32)
}

/// Portable : une batterie système, ou un châssis de portable (batterie retirée).
pub fn is_laptop_in(power_supply: &Path, chassis_type: &Path) -> bool {
    !battery_dirs(power_supply).is_empty() || read(chassis_type).and_then(|s| s.parse().ok()).is_some_and(chassis_is_portable)
}

pub fn is_laptop() -> bool {
    is_laptop_in(Path::new("/sys/class/power_supply"), Path::new("/sys/class/dmi/id/chassis_type"))
}

// ───────────────────────────── Profils ─────────────────────────────

/// Profils de power-profiles-daemon (mêmes noms côté auto-cpufreq dans NOOS).
pub const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];

pub fn profile_label(p: &str) -> &str {
    match p {
        "power-saver" => "Économie",
        "balanced" => "Équilibré",
        "performance" => "Performances",
        o => o,
    }
}

pub fn profile_description(p: &str, acf: bool) -> &'static str {
    match (p, acf) {
        ("power-saver", _) => "Fréquences réduites, autonomie maximale",
        ("balanced", true) => "auto-cpufreq ajuste le processeur selon la charge et le secteur",
        ("balanced", false) => "Bon compromis entre performances et consommation",
        ("performance", _) => "Processeur à pleine vitesse, consommation plus élevée",
        _ => "",
    }
}

/// Choix forcé d'auto-cpufreq d'après le contenu de `override.pickle` (pickle d'une chaîne
/// Python : on y cherche le mot, sans décoder le format).
pub fn acf_profile(pickle: Option<&[u8]>) -> &'static str {
    let Some(b) = pickle else { return "balanced" };
    let has = |w: &[u8]| b.windows(w.len()).any(|x| x == w);
    if has(b"powersave") {
        "power-saver"
    } else if has(b"performance") {
        "performance"
    } else {
        "balanced"
    }
}

/// Argument de `auto-cpufreq --force=` pour un profil.
pub fn acf_force(profile: &str) -> Option<&'static str> {
    match profile {
        "power-saver" => Some("powersave"),
        "balanced" => Some("reset"),
        "performance" => Some("performance"),
        _ => None,
    }
}

/// Le démon auto-cpufreq tourne-t-il ? (processus `auto-cpufreq --daemon`, lu dans /proc).
pub fn acf_running_in(proc_root: &Path) -> bool {
    std::fs::read_dir(proc_root).into_iter().flatten().flatten().any(|e| {
        e.file_name().to_string_lossy().bytes().all(|c| c.is_ascii_digit())
            && std::fs::read(e.path().join("cmdline")).is_ok_and(|c| {
                let args: Vec<&[u8]> = c.split(|b| *b == 0).collect();
                args.iter().any(|a| a.ends_with(b"auto-cpufreq")) && args.contains(&&b"--daemon"[..])
            })
    })
}

pub fn acf_running() -> bool {
    acf_running_in(Path::new("/proc"))
}

// ───────────────────────────── Démo ─────────────────────────────

/// `NOOS_POWER_DEMO` : `battery`, `charging`, `low`, `critical`, `full`, `plugged`,
/// `desktop` (pas de batterie). `None` sans la variable. `Some(None)` = PC fixe.
pub fn demo() -> Option<Option<Battery>> {
    let v = std::env::var("NOOS_POWER_DEMO").ok()?;
    let b = |percent, state, tte, ttf| Battery {
        percent,
        state,
        time_to_empty: tte,
        time_to_full: ttf,
        capacity: 91.4,
        rate: 9.8,
        model: "Sunwoda 5B10W51867".into(),
        cycles: 214,
    };
    Some(match v.split([';', ',']).next().unwrap_or("") {
        "desktop" | "fixe" => None,
        "charging" => Some(b(64.0, ChargeState::Charging, 0, 40 * 60)),
        "low" => Some(b(17.0, ChargeState::Discharging, 34 * 60, 0)),
        "critical" => Some(b(6.0, ChargeState::Discharging, 9 * 60, 0)),
        "full" => Some(b(100.0, ChargeState::Full, 0, 0)),
        "plugged" => Some(b(80.0, ChargeState::PendingCharge, 0, 0)),
        _ => Some(b(78.0, ChargeState::Discharging, 2 * 3600 + 15 * 60, 0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(files: &[(&str, &str)]) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("noos-power-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (p, v) in files {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, v).unwrap();
        }
        root
    }

    #[test]
    fn durees() {
        assert_eq!(duration(2 * 3600 + 15 * 60), "2 h 15");
        assert_eq!(duration(40 * 60), "40 min");
        assert_eq!(duration(3600), "1 h");
        assert_eq!(duration(3600 + 5 * 60), "1 h 05");
        assert_eq!(duration(10), "moins d'une minute");
    }

    #[test]
    fn textes() {
        let mut b = Battery { percent: 78.0, state: ChargeState::Discharging, time_to_empty: 8100, ..Default::default() };
        assert_eq!(b.status_text(), "2 h 15 restantes");
        assert_eq!(b.tooltip(), "Batterie : 78 %\n2 h 15 restantes");
        b.time_to_empty = 3600;
        assert_eq!(b.status_text(), "1 h restante");
        b.state = ChargeState::Charging;
        b.time_to_full = 2400;
        assert_eq!(b.status_text(), "Pleine charge dans 40 min");
        assert!(!b.low());
        let low = Battery { percent: 15.0, state: ChargeState::Discharging, ..Default::default() };
        assert!(low.low() && !low.critical());
        let crit = Battery { percent: 5.0, state: ChargeState::Charging, ..Default::default() };
        assert!(!crit.critical(), "en charge : pas d'alerte");
    }

    #[test]
    fn sysfs() {
        let root = fake(&[
            ("BAT0/type", "Battery"),
            ("BAT0/status", "Discharging"),
            ("BAT0/capacity", "50"),
            ("BAT0/energy_now", "25000000"),
            ("BAT0/energy_full", "50000000"),
            ("BAT0/energy_full_design", "55000000"),
            ("BAT0/power_now", "10000000"),
            ("BAT0/manufacturer", "LGC"),
            ("BAT0/model_name", "L19L3PD1"),
            ("AC/type", "Mains"),
            ("AC/online", "0"),
            ("hidpp_battery_0/type", "Battery"),
            ("hidpp_battery_0/scope", "Device"),
        ]);
        let b = read_sysfs_in(&root).unwrap();
        assert_eq!(b.percent, 50.0);
        assert_eq!(b.state, ChargeState::Discharging);
        assert_eq!(b.time_to_empty, 9000); // 25 Wh à 10 W : 2 h 30
        assert!((b.rate - 10.0).abs() < 0.01);
        assert!((b.capacity - 90.9).abs() < 0.1);
        assert_eq!(b.model, "LGC L19L3PD1");
        assert_eq!(battery_dirs(&root).len(), 1, "souris sans fil ignorée");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn portables() {
        let fixe = fake(&[("ps/AC/type", "Mains"), ("chassis", "35")]);
        assert!(!is_laptop_in(&fixe.join("ps"), &fixe.join("chassis")));
        let _ = std::fs::remove_dir_all(&fixe);
        let sans_batterie = fake(&[("ps/AC/type", "Mains"), ("chassis_type", "10\n")]);
        assert!(is_laptop_in(&sans_batterie.join("ps"), &sans_batterie.join("chassis_type")));
        let _ = std::fs::remove_dir_all(&sans_batterie);
        let avec = fake(&[("ps/BAT1/type", "Battery"), ("chassis_type", "3")]);
        assert!(is_laptop_in(&avec.join("ps"), &avec.join("chassis_type")));
        let _ = std::fs::remove_dir_all(&avec);
        for c in [8, 9, 10, 14, 31, 32] {
            assert!(chassis_is_portable(c));
        }
        for c in [3, 4, 6, 7, 13, 35] {
            assert!(!chassis_is_portable(c));
        }
    }

    #[test]
    fn auto_cpufreq() {
        // pickle.dumps("powersave") en protocole 4
        let p = b"\x80\x04\x95\r\x00\x00\x00\x00\x00\x00\x00\x8c\tpowersave\x94.";
        assert_eq!(acf_profile(Some(p)), "power-saver");
        assert_eq!(acf_profile(Some(b"\x80\x04\x95\x0f\x8c\x0bperformance\x94.")), "performance");
        assert_eq!(acf_profile(None), "balanced");
        assert_eq!(acf_force("balanced"), Some("reset"));
        assert_eq!(acf_force("autre"), None);
        let proc_ = fake(&[("123/cmdline", "/usr/bin/python3\0/usr/bin/auto-cpufreq\0--daemon\0"), ("self/cmdline", "x")]);
        assert!(acf_running_in(&proc_));
        let _ = std::fs::remove_dir_all(&proc_);
        let proc_ = fake(&[("77/cmdline", "/usr/bin/python3\0/usr/bin/auto-cpufreq\0--stats\0")]);
        assert!(!acf_running_in(&proc_));
        let _ = std::fs::remove_dir_all(&proc_);
    }
}

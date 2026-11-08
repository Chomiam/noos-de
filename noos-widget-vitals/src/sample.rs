//! Mesures des ressources, lues dans /proc et /sys (aucun outil externe, sauf `nvidia-smi`
//! pour les cartes NVIDIA). Un échantillon toutes les 2 s, dans un fil à part.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

/// Un relevé complet. Les débits sont en octets par seconde.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Processeur : occupation (0–100), fréquence moyenne (MHz), température (°C).
    pub cpu: f64,
    pub cpu_mhz: Option<u64>,
    pub cpu_temp: Option<f64>,
    pub cpu_name: String,
    pub cpu_threads: usize,
    /// Mémoire (Mio).
    pub ram_used: u64,
    pub ram_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    /// Carte graphique : occupation (0–100) si mesurable, et comment elle est estimée.
    pub gpu: Option<f64>,
    pub gpu_name: String,
    pub gpu_note: &'static str,
    pub gpu_temp: Option<f64>,
    pub gpu_mem: Option<(u64, u64)>,
    /// Disque : part du temps occupé (0–100), lecture / écriture.
    pub disk_busy: f64,
    pub disk_read: f64,
    pub disk_write: f64,
    /// Espace de la racine (octets).
    pub root_used: u64,
    pub root_total: u64,
    /// Réseau : réception / émission.
    pub net_rx: f64,
    pub net_tx: f64,
}

impl Snapshot {
    pub fn ram_pct(&self) -> f64 {
        if self.ram_total == 0 { 0.0 } else { self.ram_used as f64 * 100.0 / self.ram_total as f64 }
    }
}

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok()
}

fn read_u64(p: impl AsRef<Path>) -> Option<u64> {
    read(p)?.trim().parse().ok()
}

/// Comment mesurer la carte graphique.
enum Gpu {
    /// AMD : `gpu_busy_percent` (pilote amdgpu).
    Amd(PathBuf),
    /// Intel : fréquence réelle rapportée à la fréquence maximale (i915 / xe n'exposent pas
    /// de taux d'occupation simple ; la fréquence suit la charge).
    Intel { act: PathBuf, max: u64 },
    Nvidia,
    None,
}

fn detect_gpu() -> (Gpu, String) {
    let mut cards: Vec<PathBuf> = fs::read_dir("/sys/class/drm")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("card") && !n.contains('-')))
        .collect();
    cards.sort();
    // Carte dédiée d'abord (AMD / NVIDIA), puis Intel.
    for c in &cards {
        let busy = c.join("device/gpu_busy_percent");
        if busy.exists() {
            return (Gpu::Amd(busy), gpu_name(c).unwrap_or_else(|| "Carte graphique AMD".into()));
        }
    }
    if Path::new("/proc/driver/nvidia/version").exists() {
        let name = Command::new("nvidia-smi")
            .args(["--query-gpu=name", "--format=csv,noheader"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Carte graphique NVIDIA".into());
        return (Gpu::Nvidia, name);
    }
    for c in &cards {
        let act = c.join("gt_act_freq_mhz");
        let max = read_u64(c.join("gt_RP0_freq_mhz")).or_else(|| read_u64(c.join("gt_max_freq_mhz")));
        if let (true, Some(max)) = (act.exists(), max) {
            return (Gpu::Intel { act, max: max.max(1) }, gpu_name(c).unwrap_or_else(|| "Carte graphique Intel".into()));
        }
    }
    (Gpu::None, String::new())
}

/// Nom lisible depuis la base PCI (`/usr/share/hwdata/pci.ids`), sinon le fabricant.
fn gpu_name(card: &Path) -> Option<String> {
    let vendor = read(card.join("device/vendor"))?.trim().trim_start_matches("0x").to_lowercase();
    let device = read(card.join("device/device"))?.trim().trim_start_matches("0x").to_lowercase();
    let ids = read("/usr/share/hwdata/pci.ids")?;
    let mut in_vendor = false;
    let mut vname = String::new();
    for line in ids.lines() {
        if !line.starts_with('\t') && !line.starts_with('#') {
            if in_vendor {
                break;
            }
            if line.starts_with(&vendor) {
                in_vendor = true;
                vname = line[4..].trim().to_string();
            }
        } else if in_vendor && line.starts_with('\t') && !line.starts_with("\t\t") && line[1..].starts_with(&device) {
            let name = line[5..].trim();
            // « CoffeeLake-S GT2 [UHD Graphics 630] » → « UHD Graphics 630 ».
            let short = name.rsplit_once('[').map(|(_, b)| b.trim_end_matches(']')).unwrap_or(name);
            let brand = vname.split_whitespace().next().unwrap_or("");
            return Some(format!("{brand} {short}").trim().to_string());
        }
    }
    (!vname.is_empty()).then_some(vname)
}

fn cpu_name() -> String {
    read("/proc/cpuinfo")
        .and_then(|t| t.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim().to_string()))
        .map(|n| n.replace("(R)", "").replace("(TM)", "").replace(" CPU", "").split(" @ ").next().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "Processeur".into())
}

/// Températures : processeur (coretemp / k10temp / zenpower) et carte graphique (amdgpu…).
fn temps() -> (Option<f64>, Option<f64>) {
    let (mut cpu, mut gpu, mut acpi) = (None, None, None);
    for e in fs::read_dir("/sys/class/hwmon").into_iter().flatten().flatten() {
        let p = e.path();
        let name = read(p.join("name")).unwrap_or_default().trim().to_string();
        let t = read_u64(p.join("temp1_input")).map(|m| m as f64 / 1000.0);
        match name.as_str() {
            "coretemp" | "k10temp" | "zenpower" | "cpu_thermal" => cpu = cpu.or(t),
            "amdgpu" | "nouveau" | "radeon" => gpu = gpu.or(t),
            "acpitz" => acpi = acpi.or(t),
            _ => {}
        }
    }
    (cpu.or(acpi), gpu)
}

fn cpu_mhz() -> Option<u64> {
    let mut sum = 0u64;
    let mut n = 0u64;
    for e in fs::read_dir("/sys/devices/system/cpu/cpufreq").ok()?.flatten() {
        if let Some(k) = read_u64(e.path().join("scaling_cur_freq")) {
            sum += k;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n / 1000)
}

fn cpu_times() -> Option<(u64, u64)> {
    let t = read("/proc/stat")?;
    let v: Vec<u64> = t.lines().next()?.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
    let idle = v.get(3)? + v.get(4).unwrap_or(&0);
    Some((v.iter().take(8).sum(), idle))
}

/// Disques physiques : (temps occupé ms max, secteurs lus, secteurs écrits).
fn disk_stats() -> (u64, u64, u64) {
    let Some(t) = read("/proc/diskstats") else { return (0, 0, 0) };
    let (mut busy, mut r, mut w) = (0u64, 0u64, 0u64);
    for l in t.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 13 {
            continue;
        }
        let name = f[2];
        // Disques entiers seulement (pas de partitions, ni loop / ram / zram / dm).
        let whole = Path::new(&format!("/sys/block/{name}/device")).exists();
        if !whole {
            continue;
        }
        let n = |i: usize| f[i].parse::<u64>().unwrap_or(0);
        busy = busy.max(n(12));
        r += n(5);
        w += n(9);
    }
    (busy, r, w)
}

fn net_bytes() -> (u64, u64) {
    let Some(t) = read("/proc/net/dev") else { return (0, 0) };
    let (mut rx, mut tx) = (0, 0);
    for l in t.lines().skip(2) {
        let Some((iface, rest)) = l.split_once(':') else { continue };
        let iface = iface.trim();
        if iface == "lo" || ["veth", "docker", "br-", "virbr", "podman", "vnet"].iter().any(|p| iface.starts_with(p)) {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        rx += f.first().copied().unwrap_or(0);
        tx += f.get(8).copied().unwrap_or(0);
    }
    (rx, tx)
}

fn meminfo() -> (u64, u64, u64, u64) {
    let t = read("/proc/meminfo").unwrap_or_default();
    let get = |k: &str| t.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) / 1024;
    let total = get("MemTotal:");
    let (st, sf) = (get("SwapTotal:"), get("SwapFree:"));
    (total.saturating_sub(get("MemAvailable:")), total, st.saturating_sub(sf), st)
}

fn root_space() -> (u64, u64) {
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: chemin C valide terminé par NUL, structure initialisée.
    if unsafe { libc::statvfs(c"/".as_ptr(), &mut s) } != 0 {
        return (0, 0);
    }
    let total = s.f_blocks as u64 * s.f_frsize as u64;
    let free = s.f_bavail as u64 * s.f_frsize as u64;
    (total.saturating_sub(free), total)
}

pub struct Sampler {
    gpu: Gpu,
    gpu_name: String,
    cpu_name: String,
    prev_cpu: Option<(u64, u64)>,
    prev_disk: (u64, u64, u64),
    prev_net: (u64, u64),
    prev_at: Instant,
}

impl Sampler {
    pub fn new() -> Self {
        let (gpu, gpu_name) = detect_gpu();
        Self { gpu, gpu_name, cpu_name: cpu_name(), prev_cpu: cpu_times(), prev_disk: disk_stats(), prev_net: net_bytes(), prev_at: Instant::now() }
    }

    pub fn sample(&mut self) -> Snapshot {
        let dt = self.prev_at.elapsed().as_secs_f64().max(0.2);
        self.prev_at = Instant::now();
        let mut s = Snapshot { cpu_name: self.cpu_name.clone(), gpu_name: self.gpu_name.clone(), ..Default::default() };
        s.cpu_threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);

        if let (Some((t0, i0)), Some((t1, i1))) = (self.prev_cpu, cpu_times()) {
            let (dt_, di) = (t1.saturating_sub(t0), i1.saturating_sub(i0));
            s.cpu = if dt_ == 0 { 0.0 } else { (dt_ - di.min(dt_)) as f64 * 100.0 / dt_ as f64 };
            self.prev_cpu = Some((t1, i1));
        }
        s.cpu_mhz = cpu_mhz();
        let (ct, gt) = temps();
        s.cpu_temp = ct;
        s.gpu_temp = gt;

        (s.ram_used, s.ram_total, s.swap_used, s.swap_total) = meminfo();

        match &self.gpu {
            Gpu::Amd(p) => {
                s.gpu = read_u64(p).map(|v| v as f64);
                s.gpu_note = "Occupation";
                let dev = p.parent().unwrap();
                if let (Some(u), Some(t)) = (read_u64(dev.join("mem_info_vram_used")), read_u64(dev.join("mem_info_vram_total"))) {
                    s.gpu_mem = Some((u >> 20, t >> 20));
                }
            }
            Gpu::Intel { act, max } => {
                s.gpu = read_u64(act).map(|f| (f as f64 * 100.0 / *max as f64).min(100.0));
                s.gpu_note = "Activité (fréquence)";
            }
            Gpu::Nvidia => {
                let out = Command::new("nvidia-smi").args(["--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"]).output();
                if let Ok(o) = out {
                    let line = String::from_utf8_lossy(&o.stdout);
                    let f: Vec<f64> = line.lines().next().unwrap_or("").split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if f.len() == 4 {
                        s.gpu = Some(f[0]);
                        s.gpu_mem = Some((f[1] as u64, f[2] as u64));
                        s.gpu_temp = Some(f[3]);
                    }
                }
                s.gpu_note = "Occupation";
            }
            Gpu::None => {}
        }

        let (b1, r1, w1) = disk_stats();
        let (b0, r0, w0) = self.prev_disk;
        s.disk_busy = (b1.saturating_sub(b0) as f64 / (dt * 1000.0) * 100.0).min(100.0);
        s.disk_read = r1.saturating_sub(r0) as f64 * 512.0 / dt;
        s.disk_write = w1.saturating_sub(w0) as f64 * 512.0 / dt;
        self.prev_disk = (b1, r1, w1);
        (s.root_used, s.root_total) = root_space();

        let (rx1, tx1) = net_bytes();
        let (rx0, tx0) = self.prev_net;
        s.net_rx = rx1.saturating_sub(rx0) as f64 / dt;
        s.net_tx = tx1.saturating_sub(tx0) as f64 / dt;
        self.prev_net = (rx1, tx1);
        s
    }
}

/// « 1,2 Mo/s », « 340 Ko/s ».
pub fn rate(bps: f64) -> String {
    let (v, u) = if bps >= 1e9 {
        (bps / 1e9, "Go/s")
    } else if bps >= 1e6 {
        (bps / 1e6, "Mo/s")
    } else if bps >= 1e3 {
        (bps / 1e3, "Ko/s")
    } else {
        (bps, "o/s")
    };
    if v >= 100.0 || u == "o/s" { format!("{v:.0} {u}") } else { format!("{v:.1} {u}").replace('.', ",") }
}

/// Mio → « 5,2 Go ».
pub fn gib(mib: u64) -> String {
    format!("{:.1} Go", mib as f64 / 1024.0).replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling() {
        let mut s = Sampler::new();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let v = s.sample();
        assert!(v.ram_total > 0);
        assert!((0.0..=100.0).contains(&v.cpu));
        assert!(v.root_total > 0);
        assert_eq!(rate(1_234_567.0), "1,2 Mo/s");
        assert_eq!(rate(512.0), "512 o/s");
    }
}

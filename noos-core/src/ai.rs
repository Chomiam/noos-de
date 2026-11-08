//! IA locale : détection du matériel (RAM, GPU Vulkan, VRAM) et choix du modèle par palier.
//! Partagé par le démon `noos-ai` et Paramètres › IA. Tout reste sur le PC (Ollama local).

use std::process::Command;

/// Carte graphique vue par Vulkan.
#[derive(Debug, Clone, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub driver: String,
    pub discrete: bool,
    /// Mémoire dédiée (Mo), si connue (AMD : sysfs ; NVIDIA : nvidia-smi).
    pub vram_mb: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hardware {
    pub ram_mb: u64,
    pub ram_available_mb: u64,
    pub cpu_threads: usize,
    /// GPU utilisables par Vulkan (le rendu logiciel llvmpipe est exclu).
    pub gpus: Vec<Gpu>,
    pub vulkan_tools: bool,
    pub ollama: bool,
    pub ollama_vulkan: bool,
    pub whisper: bool,
    pub on_battery: bool,
}

impl Hardware {
    pub fn best_gpu(&self) -> Option<&Gpu> {
        self.gpus.iter().max_by_key(|g| (g.discrete, g.vram_mb.unwrap_or(0)))
    }
    /// Description courte : « Intel UHD 630 (Vulkan) · 7,6 Go de RAM ».
    pub fn summary(&self) -> String {
        let gpu = match self.best_gpu() {
            Some(g) => format!("{} (Vulkan{})", g.name, g.vram_mb.map(|v| format!(", {:.0} Go", v as f64 / 1024.0)).unwrap_or_default()),
            None if !self.vulkan_tools => "Carte graphique non vérifiée (outil vulkaninfo absent)".into(),
            None => "Processeur seul (pas de GPU Vulkan)".into(),
        };
        format!("{gpu} · {} Go de RAM", format!("{:.1}", self.ram_mb as f64 / 1024.0).replace('.', ","))
    }
}

fn meminfo(key: &str) -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

/// Analyse `vulkaninfo --summary` (blocs « GPUn: » avec deviceType / deviceName / driverName).
pub fn parse_vulkan_summary(text: &str) -> Vec<Gpu> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String, String)> = None;
    let flush = |cur: &mut Option<(String, String, String)>, out: &mut Vec<Gpu>| {
        if let Some((ty, name, driver)) = cur.take() {
            if !ty.contains("CPU") && !name.is_empty() {
                out.push(Gpu { name, driver, discrete: ty.contains("DISCRETE"), vram_mb: None });
            }
        }
    };
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("GPU") && l.ends_with(':') {
            flush(&mut cur, &mut out);
            cur = Some(Default::default());
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        if let Some((k, v)) = l.split_once('=') {
            let v = v.trim().to_string();
            match k.trim() {
                "deviceType" => c.0 = v,
                "deviceName" => c.1 = v,
                "driverName" => c.2 = v,
                _ => {}
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Mémoire dédiée : AMD (sysfs) puis NVIDIA (nvidia-smi).
fn vram_mb() -> Option<u64> {
    let amd = std::fs::read_dir("/sys/class/drm").ok()?.flatten().filter_map(|e| std::fs::read_to_string(e.path().join("device/mem_info_vram_total")).ok()).filter_map(|s| s.trim().parse::<u64>().ok()).max();
    if let Some(b) = amd {
        return Some(b / 1024 / 1024);
    }
    let out = Command::new("nvidia-smi").args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<u64>().ok()).max()
}

fn on_battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply")
        .map(|rd| {
            rd.flatten().any(|e| {
                let p = e.path();
                std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Battery")
                    && std::fs::read_to_string(p.join("status")).is_ok_and(|s| s.trim() == "Discharging")
            })
        })
        .unwrap_or(false)
}

/// Détecte le matériel (quelques dizaines de ms : `vulkaninfo`).
pub fn detect() -> Hardware {
    let vulkan_tools = which("vulkaninfo");
    let mut gpus = if vulkan_tools {
        Command::new("vulkaninfo").arg("--summary").output().map(|o| parse_vulkan_summary(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
    } else {
        vec![]
    };
    if let Some(v) = vram_mb() {
        if let Some(g) = gpus.iter_mut().find(|g| g.discrete) {
            g.vram_mb = Some(v);
        }
    }
    Hardware {
        ram_mb: meminfo("MemTotal:"),
        ram_available_mb: meminfo("MemAvailable:"),
        cpu_threads: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        gpus,
        vulkan_tools,
        ollama: which("ollama"),
        // Paquet ollama-vulkan : bibliothèque ggml Vulkan à côté des autres moteurs d'Ollama.
        ollama_vulkan: ["/usr/lib/ollama/libggml-vulkan.so", "/usr/lib64/ollama/libggml-vulkan.so", "/usr/lib/ollama/vulkan", "/usr/lib64/ollama/vulkan"].iter().any(|p| std::path::Path::new(p).exists()),
        whisper: which("whisper-cli") || which("whisper-cpp"),
        on_battery: on_battery(),
    }
}

/// Palier de modèle de langage (Ollama).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier {
    pub id: &'static str,
    pub label: &'static str,
    /// Nom du modèle dans Ollama.
    pub model: &'static str,
    /// Taille téléchargée approximative (Mo) et mémoire nécessaire (Mo).
    pub download_mb: u64,
    pub memory_mb: u64,
    pub description: &'static str,
}

pub const TIERS: &[Tier] = &[
    Tier {
        id: "light",
        label: "Léger",
        model: "qwen2.5:1.5b",
        download_mb: 990,
        memory_mb: 1800,
        description: "Commandes et recherche simples ; pour les PC modestes ou sur batterie",
    },
    Tier {
        id: "balanced",
        label: "Équilibré",
        model: "qwen2.5:3b",
        download_mb: 1900,
        memory_mb: 3200,
        description: "Bonne compréhension du français ; conseillé à partir de 8 Go de RAM",
    },
    Tier {
        id: "advanced",
        label: "Avancé",
        model: "qwen2.5:7b",
        download_mb: 4700,
        memory_mb: 6500,
        description: "Le plus fin ; pour une carte graphique de 8 Go ou 16 Go de RAM",
    },
];

/// Palier conseillé pour ce matériel (None : seules les commandes rapides, sans modèle).
pub fn recommended(hw: &Hardware) -> Option<&'static Tier> {
    let vram = hw.best_gpu().and_then(|g| g.vram_mb).unwrap_or(0);
    let id = if vram >= 8000 || (hw.ram_mb >= 15000 && hw.best_gpu().is_some_and(|g| g.discrete)) {
        "advanced"
    } else if hw.ram_mb >= 7000 {
        "balanced"
    } else if hw.ram_mb >= 3500 {
        "light"
    } else {
        return None;
    };
    TIERS.iter().find(|t| t.id == id)
}

pub fn tier(id: &str) -> Option<&'static Tier> {
    TIERS.iter().find(|t| t.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vulkan() {
        let t = "GPU0:\n\tapiVersion = 1.4\n\tdeviceType = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\tdeviceName = Intel(R) UHD Graphics 630 (CFL GT2)\n\tdriverName = Intel open-source Mesa driver\nGPU1:\n\tdeviceType = PHYSICAL_DEVICE_TYPE_CPU\n\tdeviceName = llvmpipe\n";
        let g = parse_vulkan_summary(t);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "Intel(R) UHD Graphics 630 (CFL GT2)");
        assert!(!g[0].discrete);
    }

    #[test]
    fn tiers() {
        let mut hw = Hardware { ram_mb: 7800, ram_available_mb: 4000, cpu_threads: 4, gpus: vec![], vulkan_tools: true, ollama: true, ollama_vulkan: true, whisper: true, on_battery: false };
        assert_eq!(recommended(&hw).map(|t| t.id), Some("balanced"));
        hw.ram_mb = 3000;
        assert!(recommended(&hw).is_none());
        hw.ram_mb = 16000;
        hw.gpus.push(Gpu { name: "RX".into(), driver: String::new(), discrete: true, vram_mb: Some(12000) });
        assert_eq!(recommended(&hw).map(|t| t.id), Some("advanced"));
    }
}

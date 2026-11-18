//! Mode jeu Steam et composants pour le jeu : état du système, réglages de la session
//! gamescope, catalogue des composants. Logique pure (testée) + détection (bloquante, à
//! lancer hors de la boucle GTK).

use std::{collections::HashSet, path::PathBuf, process::Command};

/// Assistant root (pkexec, action `org.noos.gaming.helper`).
pub const HELPER: &str = "/usr/libexec/noos-gaming-helper";
/// Bascule bureau ↔ mode jeu (pkexec, action `org.noos.gaming.switch`, sans mot de passe).
pub const SWITCH: &str = "/usr/libexec/noos-session-switch";
/// Session proposée par l'écran de connexion, créée à l'activation.
pub const SESSION_FILE: &str = "/usr/local/share/wayland-sessions/noos-steam.desktop";
pub const GREETD_CONF: &str = "/etc/greetd/config.toml";
pub const DECKY_UNIT: &str = "/etc/systemd/system/plugin_loader.service";
/// Paquets du mode jeu (gamescope et Steam sont dans contrib ; mangoapp = statistiques).
pub const MODE_PACKAGES: &[&str] = &["gamescope", "steam-installer", "mangoapp"];
/// Raccourci « Mode jeu Steam » (menu Démarrer, bureau).
pub const SHORTCUT_ID: &str = "org.noos.SteamGameMode.desktop";

// ───────────────────────── Réglages de la session ─────────────────────────

/// `~/.config/noos/steam-session.conf`, lu par `/usr/libexec/noos-steam-session`.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionConf {
    /// Connecteur (`DP-1`), vide = automatique.
    pub output: String,
    /// Définition, `None` = native.
    pub resolution: Option<(u32, u32)>,
    /// Fréquence en Hz, 0 = automatique.
    pub refresh: u32,
    pub hdr: bool,
    pub vrr: bool,
    /// Images par seconde, 0 = aucune limite.
    pub fps_limit: u32,
    /// Mise à l'échelle gamescope : vide (auto), `fsr`, `nis`, `linear`, `nearest`, `pixel`.
    pub filter: String,
    pub mangoapp: bool,
    pub steamdeck: bool,
}

impl Default for SessionConf {
    fn default() -> Self {
        Self { output: String::new(), resolution: None, refresh: 0, hdr: false, vrr: true, fps_limit: 0, filter: String::new(), mangoapp: true, steamdeck: true }
    }
}

pub const FILTERS: &[(&str, &str)] = &[
    ("", "Automatique"),
    ("fsr", "AMD FidelityFX Super Resolution (FSR)"),
    ("nis", "NVIDIA Image Scaling (NIS)"),
    ("linear", "Linéaire (lissé)"),
    ("nearest", "Au plus proche"),
    ("pixel", "Pixel art (entier)"),
];

pub const FPS_LIMITS: &[u32] = &[0, 30, 40, 60, 90, 120, 144, 165];

impl SessionConf {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| noos_core::paths::home().join(".config"));
        base.join("noos/steam-session.conf")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = Self::path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, self.to_text())
    }

    pub fn parse(text: &str) -> Self {
        let mut c = Self::default();
        let flag = |v: &str| v == "1";
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "OUTPUT" if v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => c.output = v.into(),
                "RESOLUTION" => c.resolution = v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))),
                "REFRESH" => c.refresh = v.parse().unwrap_or(0),
                "HDR" => c.hdr = flag(v),
                "VRR" => c.vrr = flag(v),
                "FPS_LIMIT" => c.fps_limit = v.parse().unwrap_or(0),
                "FILTER" if FILTERS.iter().any(|(id, _)| *id == v) => c.filter = v.into(),
                "MANGOAPP" => c.mangoapp = flag(v),
                "STEAMDECK" => c.steamdeck = flag(v),
                _ => {}
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        let b = |v: bool| if v { 1 } else { 0 };
        format!(
            "# Mode jeu Steam (gamescope) — écrit par Paramètres › Jeux, lu par noos-steam-session.\n\
             OUTPUT={}\nRESOLUTION={}\nREFRESH={}\nHDR={}\nVRR={}\nFPS_LIMIT={}\nFILTER={}\nMANGOAPP={}\nSTEAMDECK={}\n",
            self.output,
            self.resolution.map(|(w, h)| format!("{w}x{h}")).unwrap_or_default(),
            if self.refresh > 0 { self.refresh.to_string() } else { String::new() },
            b(self.hdr),
            b(self.vrr),
            self.fps_limit,
            self.filter,
            b(self.mangoapp),
            b(self.steamdeck),
        )
    }

    /// Ligne de commande équivalente (affichée sous les réglages).
    pub fn command_line(&self) -> String {
        let mut a = vec!["gamescope".to_string(), "-e".into()];
        if !self.output.is_empty() {
            a.push(format!("--prefer-output {}", self.output));
        }
        if let Some((w, h)) = self.resolution {
            a.push(format!("-W {w} -H {h}"));
        }
        if self.refresh > 0 {
            a.push(format!("-r {}", self.refresh));
        }
        if self.vrr {
            a.push("--adaptive-sync".into());
        }
        if self.hdr {
            a.push("--hdr-enabled".into());
        }
        if self.fps_limit > 0 {
            a.push(format!("--framerate-limit {}", self.fps_limit));
        }
        if !self.filter.is_empty() {
            a.push(format!("-F {}", self.filter));
        }
        if self.mangoapp {
            a.push("--mangoapp".into());
        }
        a.push("-- steam -gamepadui -steamos3".into());
        if self.steamdeck {
            a.push("-steamdeck".into());
        }
        a.join(" ")
    }
}

// ───────────────────────── Démarrage du PC ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boot {
    Login,
    Desktop,
    Steam,
}

impl Boot {
    pub const ALL: [Boot; 3] = [Boot::Login, Boot::Desktop, Boot::Steam];
    pub fn arg(self) -> &'static str {
        match self {
            Boot::Login => "login",
            Boot::Desktop => "desktop",
            Boot::Steam => "steam",
        }
    }
}

/// Connexion automatique de `/etc/greetd/config.toml` (section `[initial_session]`).
pub fn parse_boot(greetd: &str) -> Boot {
    let mut inside = false;
    for line in greetd.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            inside = l == "[initial_session]";
            continue;
        }
        if inside && l.starts_with("command") {
            return if l.contains("noos-steam-session") {
                Boot::Steam
            } else {
                Boot::Desktop
            };
        }
    }
    Boot::Login
}

// ───────────────────────── Composants ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// Paquets Debian : le premier fait foi pour « installé » ; `:i386` = version 32 bits.
    Apt(&'static [&'static str]),
    Flatpak(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Component {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    /// Comment s'en servir (affiché en petit sous la description).
    pub usage: &'static str,
    /// Icônes à essayer, dans l'ordre (la dernière est symbolique, toujours présente).
    pub icons: &'static [&'static str],
    pub source: Source,
    /// Dans contrib (Debian) : contrib / non-free doivent être activés.
    pub contrib: bool,
}

pub const COMPONENTS: &[Component] = &[
    Component {
        id: "steam",
        name: "Steam",
        summary: "La boutique et la bibliothèque de Valve, avec Proton pour les jeux Windows.",
        usage: "",
        icons: &["steam", "com.valvesoftware.Steam", "input-gaming-symbolic"],
        source: Source::Apt(&["steam-installer"]),
        contrib: true,
    },
    Component {
        id: "gamemode",
        name: "GameMode",
        summary: "Met le processeur et la carte graphique en performances pendant le jeu.",
        usage: "Steam : gamemoderun %command%",
        icons: &["gamemode", "power-profile-performance-symbolic", "speedometer-symbolic"],
        source: Source::Apt(&["gamemode", "libgamemode0:i386"]),
        contrib: false,
    },
    Component {
        id: "mangohud",
        name: "MangoHud",
        summary: "Images par seconde, températures et charge par-dessus le jeu (64 et 32 bits).",
        usage: "Steam : MANGOHUD=1 %command%",
        icons: &["mangohud", "io.github.flightlessmango.mangohud", "utilities-system-monitor-symbolic"],
        source: Source::Apt(&["mangohud", "mangohud:i386"]),
        contrib: false,
    },
    Component {
        id: "goverlay",
        name: "GOverlay",
        summary: "Règle MangoHud, vkBasalt et OptiScaler sans éditer de fichier.",
        usage: "",
        icons: &["io.github.benjamimgois.goverlay", "goverlay", "preferences-system-symbolic"],
        source: Source::Flatpak("io.github.benjamimgois.goverlay"),
        contrib: false,
    },
    Component {
        id: "protonplus",
        name: "ProtonPlus",
        summary: "Installe Proton-GE, Proton-CachyOS, Wine-GE… pour Steam, Lutris et Heroic.",
        usage: "",
        icons: &["com.vysp3r.ProtonPlus", "protonplus", "system-software-install-symbolic"],
        source: Source::Flatpak("com.vysp3r.ProtonPlus"),
        contrib: false,
    },
    Component {
        id: "vkbasalt",
        name: "vkBasalt",
        summary: "Filtres Vulkan : netteté (CAS), SMAA, tables de couleurs (64 et 32 bits).",
        usage: "Steam : ENABLE_VKBASALT=1 %command%",
        icons: &["vkbasalt", "applications-graphics-symbolic"],
        source: Source::Apt(&["vkbasalt", "vkbasalt:i386"]),
        contrib: false,
    },
    Component {
        id: "gamescope",
        name: "Gamescope",
        summary: "Le compositeur du Steam Deck, pour un seul jeu : définition, FSR, limite d'images.",
        usage: "Steam : gamescope -W 2560 -H 1440 -f -- %command%",
        icons: &["gamescope", "video-display-symbolic"],
        source: Source::Apt(&["gamescope"]),
        contrib: true,
    },
    Component {
        id: "protonupqt",
        name: "ProtonUp-Qt",
        summary: "Gère les versions de Proton et de Wine, aussi pour les émulateurs.",
        usage: "",
        icons: &["net.davidotek.pupgui2", "system-software-install-symbolic"],
        source: Source::Flatpak("net.davidotek.pupgui2"),
        contrib: false,
    },
    Component {
        id: "heroic",
        name: "Heroic Games Launcher",
        summary: "Vos jeux Epic Games, GOG et Amazon Prime Gaming.",
        usage: "",
        icons: &["com.heroicgameslauncher.hgl", "heroic", "applications-games-symbolic"],
        source: Source::Flatpak("com.heroicgameslauncher.hgl"),
        contrib: false,
    },
    Component {
        id: "lutris",
        name: "Lutris",
        summary: "Battle.net, EA, Ubisoft, émulateurs : tous les jeux au même endroit.",
        usage: "",
        icons: &["lutris", "net.lutris.Lutris", "applications-games-symbolic"],
        source: Source::Apt(&["lutris"]),
        contrib: true,
    },
    Component {
        id: "bottles",
        name: "Bottles",
        summary: "Logiciels et jeux Windows dans des « bouteilles » Wine isolées.",
        usage: "",
        icons: &["com.usebottles.bottles", "bottles", "wine", "applications-other-symbolic"],
        source: Source::Flatpak("com.usebottles.bottles"),
        contrib: false,
    },
    Component {
        id: "protontricks",
        name: "Protontricks",
        summary: "Winetricks pour les jeux Proton de Steam (polices, DLL, correctifs).",
        usage: "",
        icons: &["protontricks", "wine", "applications-utilities-symbolic"],
        source: Source::Apt(&["protontricks"]),
        contrib: true,
    },
];

// ───────────────────────── État du système ─────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decky {
    pub installed: bool,
    pub active: bool,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub gamescope: bool,
    pub steam: bool,
    pub mangoapp: bool,
    /// Session « Steam (mode jeu) » présente : le mode jeu est activé.
    pub session: bool,
    /// contrib (et non-free) activés dans les sources APT de Debian.
    pub contrib: bool,
    pub i386: bool,
    pub boot: Boot,
    pub decky: Decky,
    /// Identifiants des composants installés.
    pub installed: HashSet<String>,
    /// Assistant root présent (paquet NOOS à jour).
    pub helper: bool,
    pub demo: bool,
}

impl Status {
    pub fn enabled(&self) -> bool {
        self.session
    }
    /// Activé mais il manque un morceau (gamescope ou Steam désinstallés depuis).
    pub fn broken(&self) -> bool {
        self.session && !(self.gamescope && self.steam)
    }
}

fn exists_bin(name: &str) -> bool {
    noos_pkg::exec::have(name) || std::path::Path::new("/usr/games").join(name).is_file()
}

/// Paquets installés parmi `names` (`nom` ou `nom:i386`).
fn dpkg_installed(names: &[&str]) -> HashSet<String> {
    let out = Command::new("dpkg-query")
        .args(["-W", "-f=${binary:Package} ${Architecture} ${db:Status-Status}\\n"])
        .args(names)
        .env("LC_ALL", "C.UTF-8")
        .output();
    let mut set = HashSet::new();
    let Ok(out) = out else { return set };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut w = line.split_whitespace();
        let (Some(pkg), Some(arch), Some(st)) = (w.next(), w.next(), w.next()) else { continue };
        if st != "installed" {
            continue;
        }
        let base = pkg.split(':').next().unwrap_or(pkg);
        set.insert(if arch == "i386" { format!("{base}:i386") } else { base.to_string() });
    }
    set
}

fn flatpak_installed() -> HashSet<String> {
    noos_pkg::exec::capture(&["flatpak", "list", "--app", "--columns=application"])
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

/// contrib activé pour un dépôt Debian ?
pub fn contrib_enabled() -> bool {
    use noos_pkg::backends::aptsources;
    aptsources::entries()
        .iter()
        .any(|e| e.enabled && e.uris.iter().any(|u| aptsources::is_debian_uri(u)) && e.components.iter().any(|c| c == "contrib"))
}

pub fn i386_enabled() -> bool {
    noos_pkg::exec::capture(&["dpkg", "--print-foreign-architectures"]).is_ok_and(|o| o.lines().any(|l| l.trim() == "i386"))
}

pub fn decky_home() -> PathBuf {
    noos_core::paths::home().join("homebrew")
}

pub fn detect() -> Status {
    if let Some(s) = demo() {
        return s;
    }
    let mut apt_names: Vec<&str> = MODE_PACKAGES.to_vec();
    for c in COMPONENTS {
        if let Source::Apt(p) = c.source {
            apt_names.extend(p);
        }
    }
    let debs = dpkg_installed(&apt_names);
    let flats = if noos_pkg::exec::have("flatpak") { flatpak_installed() } else { HashSet::new() };
    let installed = COMPONENTS
        .iter()
        .filter(|c| match c.source {
            Source::Apt(p) => debs.contains(p[0]),
            Source::Flatpak(id) => flats.contains(id),
        })
        .map(|c| c.id.to_string())
        .collect();
    let decky_installed = std::path::Path::new(DECKY_UNIT).exists();
    Status {
        gamescope: exists_bin("gamescope"),
        steam: exists_bin("steam"),
        mangoapp: exists_bin("mangoapp"),
        session: std::path::Path::new(SESSION_FILE).exists(),
        contrib: contrib_enabled(),
        i386: i386_enabled(),
        boot: std::fs::read_to_string(GREETD_CONF).map(|t| parse_boot(&t)).unwrap_or(Boot::Login),
        decky: Decky {
            installed: decky_installed,
            active: decky_installed
                && Command::new("systemctl").args(["is-active", "--quiet", "plugin_loader.service"]).status().is_ok_and(|s| s.success()),
            version: std::fs::read_to_string(decky_home().join("services/.loader.version")).map(|v| v.trim().to_string()).unwrap_or_default(),
        },
        installed,
        helper: std::path::Path::new(HELPER).exists(),
        demo: false,
    }
}

// ───────────────────────── Démo (captures) ─────────────────────────

/// `NOOS_GAMING_DEMO=1|off|on|installing` : état fictif, rien n'est exécuté.
pub fn demo_mode() -> Option<String> {
    std::env::var("NOOS_GAMING_DEMO").ok().filter(|v| !v.is_empty() && v != "0")
}

pub fn demo() -> Option<Status> {
    let mode = demo_mode()?;
    let on = mode == "on";
    let ids: &[&str] = if on { &["steam", "gamemode", "mangohud", "protonplus", "gamescope", "heroic"] } else { &["gamemode", "mangohud"] };
    Some(Status {
        gamescope: on,
        steam: on,
        mangoapp: on,
        session: on,
        contrib: on,
        i386: true,
        boot: if on { Boot::Steam } else { Boot::Login },
        decky: Decky { installed: on, active: on, version: if on { "v3.3.1".into() } else { String::new() } },
        installed: ids.iter().map(|s| s.to_string()).collect(),
        helper: true,
        demo: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_roundtrip() {
        let c = SessionConf {
            output: "DP-1".into(),
            resolution: Some((2560, 1440)),
            refresh: 165,
            hdr: true,
            vrr: false,
            fps_limit: 60,
            filter: "fsr".into(),
            mangoapp: false,
            steamdeck: true,
        };
        assert_eq!(SessionConf::parse(&c.to_text()), c);
        assert_eq!(SessionConf::parse(&SessionConf::default().to_text()), SessionConf::default());
    }

    #[test]
    fn conf_rejects_garbage() {
        let c = SessionConf::parse("OUTPUT=$(rm -rf ~)\nFILTER=evil\nRESOLUTION=abc\nHDR=1 # commentaire\n");
        assert_eq!(c.output, "");
        assert_eq!(c.filter, "");
        assert_eq!(c.resolution, None);
        assert!(c.hdr);
    }

    #[test]
    fn command_line() {
        let c = SessionConf { resolution: Some((1920, 1080)), filter: "fsr".into(), ..Default::default() };
        let l = c.command_line();
        assert!(l.starts_with("gamescope -e -W 1920 -H 1080 --adaptive-sync -F fsr --mangoapp -- steam"), "{l}");
    }

    #[test]
    fn boot_modes() {
        let base = "[terminal]\nvt = 1\n\n[default_session]\ncommand = \"/usr/bin/noos-greeter\"\nuser = \"_greetd\"\n";
        assert_eq!(parse_boot(base), Boot::Login);
        let steam = format!("{base}\n[initial_session]\nuser = \"axel\"\ncommand = \"env X=1 /usr/libexec/noos-steam-session\"\n");
        assert_eq!(parse_boot(&steam), Boot::Steam);
        let desk = format!("{base}\n[initial_session]\nuser = \"axel\"\ncommand = \"/usr/bin/noos-session\"\n");
        assert_eq!(parse_boot(&desk), Boot::Desktop);
    }

    #[test]
    fn catalogue() {
        let mut ids = HashSet::new();
        for c in COMPONENTS {
            assert!(ids.insert(c.id), "identifiant en double : {}", c.id);
            assert!(c.icons.last().unwrap().ends_with("-symbolic"), "{} : icône de repli symbolique", c.id);
            if let Source::Flatpak(id) = c.source {
                assert_eq!(id.split('.').count() >= 3, true, "{id}");
            }
        }
    }
}

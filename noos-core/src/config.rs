//! Configuration utilisateur de NOOS (`~/.config/noos/noos.toml`).
//!
//! Tous les champs ont une valeur par défaut : un fichier vide ou partiel est valide,
//! ce qui permet de faire évoluer le format sans casser les configs existantes.

use std::{fs, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct NoosConfig {
    pub appearance: Appearance,
    pub taskbar: Taskbar,
    pub gaming: Gaming,
    pub keyboard: Keyboard,
    pub input: Input,
    pub power: Power,
    pub notifications: Notifications,
    pub tiling: Tiling,
    /// Réglages par écran (nom du connecteur DRM, ex. `DP-1`, `HDMI-A-1`).
    #[serde(rename = "output")]
    pub outputs: Vec<OutputConfig>,
}

// ───────────────────────────── Apparence ─────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ColorScheme {
    Light,
    #[default]
    Dark,
    /// Suit la préférence système (portail) si disponible.
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Appearance {
    pub color_scheme: ColorScheme,
    /// Nom d'une palette prédéfinie (voir `noos-theme`) : `nuit-saphir`, `mica`, `aurore`, `graphite`…
    pub palette: String,
    /// Couleur d'accent au format `#rrggbb`.
    pub accent: String,
    /// Rayon des coins des fenêtres et des panneaux (px).
    pub corner_radius: u32,
    /// Opacité de la barre des tâches et du menu Démarrer (0.0 – 1.0).
    pub panel_opacity: f64,
    /// Fond d'écran (chemin absolu). Vide = fond par défaut de NOOS.
    pub wallpaper: String,
    /// Fond d'écran changé chaque jour avec l'image du jour de Bing.
    pub bing_daily: bool,
    /// Diaporama de fonds d'écran (prioritaire sur `wallpaper` quand il est actif).
    pub slideshow: Slideshow,
    /// Police d'interface optionnelle (ex. `Inter 10`).
    pub font: String,
    /// Écrit aussi les couleurs dans `~/.config/gtk-4.0/gtk.css` (applis libadwaita) et
    /// `~/.config/gtk-3.0/gtk.css` (applis GTK3, thème adw-gtk3) pour que les applis tierces
    /// suivent la palette NOOS. Activé par défaut. Ces fichiers sont aussi lus par vos
    /// autres sessions (GNOME…).
    pub apply_to_all_apps: bool,
    /// Style des boutons de fenêtre : `mac` (pastilles colorées), `windows` (rectangulaires,
    /// fermeture rouge au survol) ou `theme` (celui du thème GTK, librement modifiable).
    pub button_style: String,
    /// Disposition des boutons de titre (syntaxe GTK `gtk-decoration-layout`).
    pub button_layout: String,
    /// Surcharges fines de la palette (couleurs nommées libAdapta, ex. `window_bg_color`).
    pub overrides: std::collections::BTreeMap<String, String>,
    /// CSS libre ajouté à la fin de la feuille générée.
    pub custom_css: String,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::Dark,
            palette: "nuit-saphir".into(),
            accent: "#89b4fa".into(),
            corner_radius: 12,
            panel_opacity: 0.72,
            wallpaper: String::new(),
            // Nouvelle installation : l'image du jour de Bing (pas de fond fourni par NOOS).
            bing_daily: true,
            slideshow: Slideshow::default(),
            font: String::new(),
            apply_to_all_apps: true,
            button_style: "mac".into(),
            button_layout: "icon:minimize,maximize,close".into(),
            overrides: Default::default(),
            custom_css: String::new(),
        }
    }
}

// ─────────────────────────── Barre des tâches ───────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TaskbarAlignment {
    #[default]
    Center,
    Left,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Taskbar {
    pub alignment: TaskbarAlignment,
    pub height: u32,
    /// Applications épinglées : nom du fichier .desktop sans extension (ex. `firefox`,
    /// `org.mozilla.firefox` pour le Flatpak). Les entrées introuvables sont ignorées.
    pub pinned: Vec<String>,
    pub show_search: bool,
    /// Applications épinglées dans le menu Démarrer.
    pub start_pinned: Vec<String>,
    /// Icônes des applications actives (zone de notification) : toutes visibles dans la
    /// barre, au lieu d'être regroupées dans le menu « ^ » comme sur Windows 11.
    pub tray_expanded: bool,
}

impl Default for Taskbar {
    fn default() -> Self {
        Self {
            alignment: TaskbarAlignment::Center,
            height: 48,
            // Première installation : Terminal, Firefox et Logiciels.
            pinned: vec!["org.noos.Terminal".into(), "org.mozilla.firefox".into(), "org.noos.Software".into()],
            show_search: true,
            tray_expanded: false,
            start_pinned: [
                "org.noos.Files",
                "org.noos.Settings",
                "org.noos.Software",
                "org.noos.Snip",
                "org.noos.Terminal",
                "org.noos.TextEditor",
                "org.noos.PhotoView",
                "firefox",
                "org.mozilla.firefox",
                "steam",
                "com.valvesoftware.Steam",
                "org.gnome.Ptyxis",
                "org.gnome.TextEditor",
                "org.gnome.Calculator",
                "org.gnome.Loupe",
                "org.gnome.Software",
                "org.gnome.Calendar",
                "org.gnome.Weather",
                "org.gnome.clocks",
                "org.gnome.Music",
                "org.gnome.Showtime",
                "net.lutris.Lutris",
                "com.heroicgameslauncher.hgl",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

// ─────────────────────────────── Gaming ───────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VrrPolicy {
    /// Jamais de VRR.
    Off,
    /// VRR activé en permanence (si l'écran le supporte).
    Always,
    /// VRR activé uniquement quand une fenêtre est en plein écran (jeux, vidéo).
    #[default]
    Fullscreen,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Gaming {
    pub vrr: VrrPolicy,
    /// Demande la sortie HDR (BT.2020 / PQ) sur les écrans compatibles. Expérimental.
    pub hdr: bool,
    /// Préférer un framebuffer 10 bits (requis pour le HDR, réduit le banding).
    pub ten_bit: bool,
    /// Autoriser le scanout direct des fenêtres plein écran (latence minimale).
    pub direct_scanout: bool,
    /// Mode jeu : supprime les effets et la barre des tâches au-dessus du plein écran.
    pub game_mode: bool,
}

impl Default for Gaming {
    fn default() -> Self {
        Self {
            vrr: VrrPolicy::Fullscreen,
            hdr: false,
            ten_bit: true,
            direct_scanout: true,
            game_mode: false,
        }
    }
}

// ─────────────────────────────── Entrées ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Keyboard {
    /// Disposition XKB (vide = celle du système, lue dans /etc/vconsole.conf).
    pub layout: String,
    pub variant: String,
    pub options: String,
    pub repeat_delay: Option<i32>,
    pub repeat_rate: Option<i32>,
    /// Verr Num actif au démarrage (écran de connexion et session), comme sous Windows.
    pub numlock: bool,
}

impl Default for Keyboard {
    fn default() -> Self {
        Self {
            layout: String::new(),
            variant: String::new(),
            options: String::new(),
            repeat_delay: None,
            repeat_rate: None,
            numlock: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Input {
    pub natural_scroll: bool,
    pub tap_to_click: bool,
    /// Accélération de la souris (-1.0 … 1.0)
    pub pointer_accel: f64,
}

impl Default for Input {
    fn default() -> Self {
        Self { natural_scroll: false, tap_to_click: true, pointer_accel: 0.0 }
    }
}

// ─────────────────────────────── Alimentation ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Power {
    /// Éteindre les écrans après N minutes d'inactivité (0 = jamais).
    pub screen_off_minutes: u32,
    /// Mettre en veille après N minutes d'inactivité (0 = jamais).
    pub suspend_minutes: u32,
    /// Ne pas éteindre l'écran pendant qu'une fenêtre est en plein écran (jeux, vidéos).
    pub inhibit_fullscreen: bool,
}

impl Default for Power {
    fn default() -> Self {
        Self { screen_off_minutes: 10, suspend_minutes: 0, inhibit_fullscreen: true }
    }
}

// ─────────────────────────────── Diaporama ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Slideshow {
    pub enabled: bool,
    /// Chemins absolus des images.
    pub images: Vec<String>,
    /// Durée d'affichage de chaque image, en minutes.
    pub interval_minutes: u32,
    /// crossfade, slide-left, slide-up, over-up, rotate, none, random
    pub transition: String,
    pub shuffle: bool,
}

impl Default for Slideshow {
    fn default() -> Self {
        Self { enabled: false, images: vec![], interval_minutes: 30, transition: "crossfade".into(), shuffle: false }
    }
}

// ─────────────────────────────── Notifications ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Notifications {
    /// Ne pas déranger : pas de bannières (sauf urgentes), elles vont au centre de notifications.
    pub do_not_disturb: bool,
    /// Durée d'affichage des bannières, en secondes.
    pub banner_seconds: u32,
    /// Son à l'arrivée d'une notification.
    pub sound: bool,
}

impl Default for Notifications {
    fn default() -> Self {
        Self { do_not_disturb: false, banner_seconds: 6, sound: true }
    }
}

// ─────────────────────────────── Fenêtres ───────────────────────────────

/// Ancrage des fenêtres (« Snap ») et animations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tiling {
    /// Interrupteur général : désactivé, plus aucun ancrage (ni bords, ni barre, ni
    /// Super + ← / →). Super + ↑ agrandit toujours.
    pub enabled: bool,
    /// Ancrer en faisant glisser une fenêtre contre un bord ou un coin de l'écran.
    pub edges: bool,
    /// Barre de dispositions en haut de l'écran pendant un déplacement.
    pub layout_bar: bool,
    /// Rectangle d'aperçu de l'emplacement visé.
    pub preview: bool,
    /// Animation fluide des fenêtres lors de l'ancrage, de l'agrandissement et de la restauration.
    pub animations: bool,
}

impl Default for Tiling {
    fn default() -> Self {
        Self { enabled: true, edges: true, layout_bar: true, preview: true, animations: true }
    }
}

// ─────────────────────────────── Écrans ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct OutputConfig {
    pub name: String,
    pub scale: Option<f64>,
    /// Mode souhaité `LARGEURxHAUTEUR@HZ`, ex. `2560x1440@165`.
    pub mode: Option<String>,
    pub vrr: Option<VrrPolicy>,
    pub hdr: Option<bool>,
    pub enabled: Option<bool>,
    /// Orientation : `normal`, `90`, `180`, `270` (sens horaire).
    pub transform: Option<String>,
}

impl NoosConfig {
    pub fn load() -> Self {
        Self::load_from(&paths::config_file()).unwrap_or_else(|err| {
            eprintln!("noos: configuration invalide ({err:#}), valeurs par défaut utilisées");
            Self::default()
        })
    }

    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("lecture de {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("ouverture de {}", path.display())),
        }
    }

    /// Écriture atomique (fichier temporaire + rename) pour que les observateurs
    /// (compositeur, shell) ne lisent jamais un fichier à moitié écrit.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = paths::config_file();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn output(&self, name: &str) -> Option<&OutputConfig> {
        self.outputs.iter().find(|o| o.name == name)
    }

    pub fn output_mut(&mut self, name: &str) -> &mut OutputConfig {
        if let Some(i) = self.outputs.iter().position(|o| o.name == name) {
            &mut self.outputs[i]
        } else {
            self.outputs.push(OutputConfig { name: name.to_string(), ..Default::default() });
            self.outputs.last_mut().unwrap()
        }
    }

    /// Politique VRR effective pour un écran donné.
    pub fn vrr_for(&self, output: &str) -> VrrPolicy {
        self.output(output).and_then(|o| o.vrr).unwrap_or(self.gaming.vrr)
    }

    pub fn hdr_for(&self, output: &str) -> bool {
        self.output(output).and_then(|o| o.hdr).unwrap_or(self.gaming.hdr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut cfg = NoosConfig::default();
        cfg.output_mut("DP-1").vrr = Some(VrrPolicy::Always);
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: NoosConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, back);
        assert_eq!(back.vrr_for("DP-1"), VrrPolicy::Always);
        assert_eq!(back.vrr_for("HDMI-A-1"), VrrPolicy::Fullscreen);
    }

    #[test]
    fn partial_file_is_valid() {
        let cfg: NoosConfig = toml::from_str("[appearance]\naccent = \"#ff0000\"\n").unwrap();
        assert_eq!(cfg.appearance.accent, "#ff0000");
        assert_eq!(cfg.appearance.corner_radius, 12);
    }
}

//! Moteur de thème NOOS.
//!
//! Le thème est un **vrai thème libAdapta** installé dans
//! `~/.local/share/themes/NOOS/libadapta-1.5/` :
//!
//! - `defaults-dark.css` / `defaults-light.css` : couleurs nommées générées depuis la palette,
//!   l'accent et les surcharges de `noos.toml` ;
//! - `base.css` / `base-hc.css` : feuille de style de libAdapta + ajustements NOOS
//!   (rayon des coins, boutons façon Windows 11, CSS libre).
//!
//! La session NOOS exporte `ADAP_DEBUG_THEME_NAME=NOOS` : **toutes** les applis libAdapta
//! de la session suivent ce thème, sans toucher aux réglages de votre session GNOME.
//! Les applis NOOS rechargent en plus le thème à chaud (fichier surveillé).

pub mod color;
pub mod palette;

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::{
    NoosConfig,
    config::{Appearance, ColorScheme},
    paths,
};

pub use palette::{PALETTES, Palette};

pub const THEME_NAME: &str = "NOOS";
const RESOURCE_PREFIX: &str = "/org/gnome/Adapta/styles";
const GLOBAL_BEGIN: &str = "/* >>> NOOS : bloc généré, ne pas modifier >>> */";
const GLOBAL_END: &str = "/* <<< NOOS <<< */";

pub fn theme_dir() -> PathBuf {
    paths::xdg_data_home().join("themes").join(THEME_NAME)
}

/// Ajustements de style ajoutés après la feuille de libAdapta.
pub fn overrides_css(app: &Appearance) -> String {
    let r = app.corner_radius;
    let mut css = format!(
        "\n/* ───── NOOS ───── */\n\
         window.csd {{ border-radius: {r}px; }}\n\
         window.csd.maximized, window.csd.fullscreen, window.csd.tiled, window.csd.tiled-left,\n\
         window.csd.tiled-right, window.csd.tiled-top, window.csd.tiled-bottom {{ border-radius: 0; }}\n\
         popover > contents, popover.menu > contents {{ border-radius: {pr}px; }}\n\
         .card, row.card, list.boxed-list {{ border-radius: {cr}px; }}\n",
        pr = r.saturating_sub(2).max(4),
        cr = r.saturating_sub(4).max(4),
    );
    if app.button_style == "mac" {
        css.push_str(
            "/* Boutons de fenêtre façon macOS (pastilles), gardés à droite. La pastille est\n\
                dessinée sur l'icône (taille fixe) : la barre de titre étire les boutons. */\n\
             windowcontrols > button, headerbar windowcontrols > button {\n\
               min-width: 0; min-height: 0; padding: 0 5px; margin: 0;\n\
               background: none; box-shadow: none; border: none; }\n\
             windowcontrols > button:hover, windowcontrols > button:active,\n\
             headerbar windowcontrols > button:hover { background: none; box-shadow: none; }\n\
             windowcontrols > button > image, headerbar windowcontrols > button > image {\n\
               min-width: 14px; min-height: 14px; padding: 0; margin: 0; border-radius: 99px;\n\
               -gtk-icon-size: 9px; color: transparent;\n\
               box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.14); }\n\
             windowcontrols > button.close > image { background-color: #ff5f57; }\n\
             windowcontrols > button.minimize > image { background-color: #febc2e; }\n\
             windowcontrols > button.maximize > image { background-color: #28c840; }\n\
             windowcontrols:hover > button > image { color: rgba(0, 0, 0, 0.6); }\n\
             windowcontrols > button:active > image { filter: brightness(0.85); }\n\
             window:backdrop windowcontrols:not(:hover) > button > image {\n\
               background-color: alpha(@window_fg_color, 0.2); box-shadow: none; }\n\
             headerbar windowcontrols.end { margin-right: 6px; }\n",
        );
    } else if app.button_style == "windows" {
        css.push_str(
            "/* Boutons de fenêtre façon Windows 11 */\n\
             windowcontrols { border-spacing: 0; }\n\
             windowcontrols > button { min-width: 46px; min-height: 32px; padding: 0; margin: 0;\n\
               border-radius: 0; background: none; box-shadow: none; }\n\
             windowcontrols > button > image { background: none; padding: 0; border-radius: 0;\n\
               min-width: 16px; min-height: 16px; }\n\
             windowcontrols > button:hover { background-color: alpha(currentColor, 0.08); }\n\
             windowcontrols > button:active { background-color: alpha(currentColor, 0.05); }\n\
             windowcontrols > button.close:hover { background-color: #c42b1c; color: white; }\n\
             windowcontrols > button.close:active { background-color: #c8473b; color: white; }\n\
             headerbar { padding-right: 0; }\n\
             headerbar windowcontrols.end { margin-right: 0; margin-top: -6px; margin-bottom: -6px; }\n\
             window.csd:not(.maximized) headerbar windowcontrols.end > button.close {\n\
               border-top-right-radius: 8px; }\n",
        );
    }
    if !app.font.is_empty() {
        let (family, size) = split_font(&app.font);
        css.push_str(&format!("* {{ font-family: \"{family}\"; }}\n"));
        if let Some(size) = size {
            css.push_str(&format!("window {{ font-size: {size}pt; }}\n"));
        }
    }
    if !app.custom_css.is_empty() {
        css.push_str("\n/* CSS personnalisé (noos.toml) */\n");
        css.push_str(&app.custom_css);
        css.push('\n');
    }
    css
}

fn split_font(font: &str) -> (String, Option<f64>) {
    match font.rsplit_once(' ') {
        Some((family, size)) if size.parse::<f64>().is_ok() => (family.to_string(), size.parse().ok()),
        _ => (font.to_string(), None),
    }
}

fn resource(path: &str) -> anyhow::Result<glib::Bytes> {
    gio::resources_lookup_data(&format!("{RESOURCE_PREFIX}/{path}"), gio::ResourceLookupFlags::NONE)
        .map_err(|e| anyhow::anyhow!("ressource libAdapta {path} : {e}"))
}

/// Écrit (ou met à jour) le thème libAdapta « NOOS ». libAdapta doit être initialisé.
pub fn install_theme(app: &Appearance) -> anyhow::Result<PathBuf> {
    let dir = theme_dir().join("libadapta-1.5");
    std::fs::create_dir_all(dir.join("assets"))?;

    let extra = overrides_css(app);
    for base in ["base.css", "base-hc.css"] {
        let original = resource(base)?;
        let mut css = String::from_utf8_lossy(&original).into_owned();
        css.push_str(&extra);
        write_if_changed(&dir.join(base), &css)?;
    }
    write_if_changed(&dir.join("defaults-dark.css"), &palette::colors_css(app, true))?;
    write_if_changed(&dir.join("defaults-light.css"), &palette::colors_css(app, false))?;

    // Images référencées par base.css (cases à cocher, puces…).
    if let Ok(children) = gio::resources_enumerate_children(&format!("{RESOURCE_PREFIX}/assets/"), gio::ResourceLookupFlags::NONE) {
        for name in children {
            let target = dir.join("assets").join(name.as_str());
            if !target.exists() {
                if let Ok(data) = resource(&format!("assets/{name}")) {
                    std::fs::write(target, &*data)?;
                }
            }
        }
    }
    // index.theme : rend le thème visible des sélecteurs de thème GTK classiques.
    write_if_changed(
        &theme_dir().join("index.theme"),
        "[Desktop Entry]\nType=X-GNOME-Metatheme\nName=NOOS\nComment=Thème généré par NOOS\n\n[X-GNOME-Metatheme]\nGtkTheme=NOOS\n",
    )?;

    apply_desktop_settings(app);
    apply_global_gtk_css(app)?;
    apply_gtk3_css(app)?;
    Ok(theme_dir())
}

/// Le thème sombre est-il actif pour les applis tierces ? (`System` : réglage GNOME actuel.)
fn wants_dark(app: &Appearance) -> bool {
    match app.color_scheme {
        ColorScheme::Dark => true,
        ColorScheme::Light => false,
        ColorScheme::System => interface_settings()
            .is_some_and(|s| s.string("color-scheme") == "prefer-dark"),
    }
}

fn interface_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup("org.gnome.desktop.interface", true)?;
    Some(gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None))
}

fn theme_exists(kind: &str, name: &str) -> bool {
    let mut dirs = vec![paths::xdg_data_home()];
    dirs.extend(glib::system_data_dirs());
    dirs.iter().any(|d| d.join(kind).join(name).is_dir())
        || (kind == "themes" && paths::home().join(".themes").join(name).is_dir())
}

/// Réglages lus par toutes les applis GTK3/GTK4/libadwaita (directement ou via le portail
/// `org.freedesktop.appearance`) : mode sombre, thème d'icônes Papirus, thème GTK3.
/// Sans cela, une installation neuve reste en Adwaita clair.
pub fn apply_desktop_settings(app: &Appearance) {
    let Some(settings) = interface_settings() else { return };
    let dark = wants_dark(app);
    let set = |key: &str, value: &str| {
        if settings.settings_schema().is_some_and(|s| s.has_key(key)) && settings.string(key) != value {
            let _ = settings.set_string(key, value);
        }
    };
    match app.color_scheme {
        ColorScheme::Dark => set("color-scheme", "prefer-dark"),
        ColorScheme::Light => set("color-scheme", "prefer-light"),
        ColorScheme::System => {}
    }
    let icons = if dark { "Papirus-Dark" } else { "Papirus-Light" };
    if theme_exists("icons", icons) {
        set("icon-theme", icons);
    }
    // adw-gtk3 : thème GTK3 qui lit les couleurs nommées de libadwaita (voir apply_gtk3_css).
    let gtk3 = if dark { "adw-gtk3-dark" } else { "adw-gtk3" };
    if theme_exists("themes", gtk3) {
        set("gtk-theme", gtk3);
    } else {
        set("gtk-theme", if dark { "Adwaita-dark" } else { "Adwaita" });
    }
    gio::Settings::sync();
}

/// Applis GTK3 (thème adw-gtk3) : couleurs nommées dans `~/.config/gtk-3.0/gtk.css`.
pub fn apply_gtk3_css(app: &Appearance) -> anyhow::Result<()> {
    let path = paths::xdg_config_home().join("gtk-3.0").join("gtk.css");
    let block = app.apply_to_all_apps.then(|| {
        // Couleurs opaques : les fenêtres GTK3 ne sont pas floutées (Mica) par le compositeur.
        let opaque = Appearance { panel_opacity: 1.0, ..app.clone() };
        palette::named_colors(&opaque, wants_dark(app))
            .into_iter()
            .map(|(k, v)| format!("@define-color {k} {v};\n"))
            .collect::<String>()
    });
    write_block(&path, block)
}

/// Remplace (ou retire, avec `None`) le bloc généré par NOOS dans un fichier CSS.
fn write_block(path: &std::path::Path, block: Option<String>) -> anyhow::Result<()> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let cleaned = match (current.find(GLOBAL_BEGIN), current.find(GLOBAL_END)) {
        (Some(a), Some(b)) if b > a => {
            format!("{}{}", &current[..a], &current[b + GLOBAL_END.len()..]).trim().to_string()
        }
        _ => current.trim().to_string(),
    };
    let out = match block {
        Some(b) => format!("{cleaned}\n\n{GLOBAL_BEGIN}\n{b}{GLOBAL_END}\n"),
        None => cleaned,
    };
    if out.trim() != current.trim() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, out.trim_start())?;
    }
    Ok(())
}

fn write_if_changed(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }
    std::fs::write(path, content)
}

/// Applis libadwaita « classiques » : bloc de variables CSS dans `~/.config/gtk-4.0/gtk.css`
/// (option `apply_to_all_apps`). Le bloc est retiré quand l'option est désactivée.
pub fn apply_global_gtk_css(app: &Appearance) -> anyhow::Result<()> {
    let path = paths::xdg_config_home().join("gtk-4.0").join("gtk.css");
    let block = app.apply_to_all_apps.then(|| {
        let vars = |dark: bool| {
            palette::named_colors(app, dark)
                .into_iter()
                .filter(|(_, v)| !v.starts_with('@'))
                .map(|(k, v)| format!("  --{}: {};\n", k.replace('_', "-"), v))
                .collect::<String>()
        };
        format!(
            ":root {{\n{}  --window-radius: {}px;\n}}\n@media (prefers-color-scheme: dark) {{\n:root {{\n{}}}\n}}\n",
            vars(false),
            app.corner_radius,
            vars(true)
        )
    });
    write_block(&path, block)
}

/// Corrige la taille d'un menu contextuel (popover) à son ouverture.
///
/// GTK sous-estime parfois la première mesure d'un `PopoverMenu` (d'une dizaine de pixels
/// dans l'Explorateur et le Terminal) : le menu est alors créé trop petit, sa dernière entrée
/// est coupée et il faut le faire défiler. Une nouvelle mesure, juste après l'affichage, donne
/// la bonne taille et GTK repositionne le menu (xdg_popup.reposition).
pub fn fit_popover(pop: &impl IsA<gtk::Popover>) {
    let pop: &gtk::Popover = pop.as_ref();
    pop.connect_map(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.queue_resize());
    });
}

pub fn adap_color_scheme(scheme: ColorScheme) -> adap::ColorScheme {
    match scheme {
        ColorScheme::Light => adap::ColorScheme::ForceLight,
        ColorScheme::Dark => adap::ColorScheme::ForceDark,
        ColorScheme::System => adap::ColorScheme::Default,
    }
}

type Listener = Box<dyn Fn(&NoosConfig)>;

/// Thème vivant d'une application NOOS : suit `noos.toml` et le mode sombre.
pub struct Theming {
    provider: gtk::CssProvider,
    config: RefCell<NoosConfig>,
    listeners: RefCell<Vec<Listener>>,
    _monitor: RefCell<Option<gio::FileMonitor>>,
}

impl Theming {
    /// À appeler dans `startup` de l'application (libAdapta est alors initialisé).
    pub fn init() -> Rc<Self> {
        let theming = Rc::new(Self {
            provider: gtk::CssProvider::new(),
            config: RefCell::new(NoosConfig::load()),
            listeners: RefCell::new(Vec::new()),
            _monitor: RefCell::new(None),
        });
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &theming.provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }

        let style = adap::StyleManager::default();
        let weak = Rc::downgrade(&theming);
        style.connect_dark_notify(move |_| {
            if let Some(t) = weak.upgrade() {
                t.apply();
            }
        });

        // On surveille le dossier (sauvegarde atomique = renommage).
        let dir = gio::File::for_path(paths::config_dir());
        let _ = std::fs::create_dir_all(paths::config_dir());
        if let Ok(monitor) = dir.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            let weak = Rc::downgrade(&theming);
            monitor.connect_changed(move |_, file, other, _event| {
                let is_cfg = |f: Option<&gio::File>| {
                    f.and_then(|f| f.basename()).is_some_and(|n| n.to_str() == Some("noos.toml"))
                };
                if is_cfg(Some(file)) || is_cfg(other) {
                    if let Some(t) = weak.upgrade() {
                        t.reload();
                    }
                }
            });
            *theming._monitor.borrow_mut() = Some(monitor);
        }

        theming.apply();
        theming
    }

    pub fn config(&self) -> NoosConfig {
        self.config.borrow().clone()
    }

    pub fn connect_changed(&self, f: impl Fn(&NoosConfig) + 'static) {
        self.listeners.borrow_mut().push(Box::new(f));
    }

    pub fn reload(&self) {
        let new = NoosConfig::load();
        if *self.config.borrow() == new {
            return;
        }
        *self.config.borrow_mut() = new;
        self.apply();
    }

    fn apply(&self) {
        let cfg = self.config.borrow().clone();
        let app = &cfg.appearance;
        let style = adap::StyleManager::default();
        let scheme = adap_color_scheme(app.color_scheme);
        if style.color_scheme() != scheme {
            style.set_color_scheme(scheme);
        }
        let dark = style.is_dark();
        let css = format!("{}{}", palette::colors_css(app, dark), overrides_css(app));
        self.provider.load_from_string(&css);
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_decoration_layout(Some(&app.button_layout));
        }
        for l in self.listeners.borrow().iter() {
            l(&cfg);
        }
    }
}

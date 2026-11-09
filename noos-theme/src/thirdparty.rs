//! Applis externes à NOOS (GTK3, GTK4 / libadwaita, Qt 5 / 6) : boutons de fenêtre et
//! couleurs alignés sur Paramètres › Personnalisation.
//!
//! - **Boutons** : `org.gnome.desktop.wm.preferences button-layout`, lu par GTK3, GTK4,
//!   libadwaita (aussi en Flatpak, via le portail Settings) et par la barre de titre Qt
//!   « Adwaita » (`QT_WAYLAND_DECORATION=adwaita`). Fedora y met `appmenu:close` : sans cela
//!   les applis tierces n'ont que le bouton Fermer.
//! - **Accent libadwaita** : `org.gnome.desktop.interface accent-color` (couleur GNOME la
//!   plus proche de l'accent NOOS) ; c'est la seule couleur que voient les applis Flatpak.
//! - **Qt** : palette générée pour qt5ct / qt6ct (`~/.config/qt{5,6}ct/colors/NOOS.conf`), la
//!   session exportant `QT_QPA_PLATFORMTHEME=qt6ct` (les deux modules répondent aux noms
//!   `qt5ct` et `qt6ct`). qt5ct / qt6ct surveillent leur fichier : changement à chaud.

use std::path::Path;

use gtk::gio::{self, prelude::*};
use noos_core::{config::Appearance, paths};

use crate::{
    color::Rgb,
    palette::{self, variant},
};

fn settings(schema: &str) -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(schema, true)?;
    Some(gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None))
}

fn set_string(s: &gio::Settings, key: &str, value: &str) {
    if s.settings_schema().is_some_and(|sc| sc.has_key(key)) && s.string(key) != value {
        let _ = s.set_string(key, value);
    }
}

/// Disposition des boutons au format GNOME (`icon` de GTK → `appmenu`, compris de tous).
pub fn gnome_button_layout(layout: &str) -> String {
    let side = |s: &str| {
        s.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| if t == "icon" { "appmenu" } else { t })
            .collect::<Vec<_>>()
            .join(",")
    };
    match layout.split_once(':') {
        Some((l, r)) => format!("{}:{}", side(l), side(r)),
        None => format!(":{}", side(layout)),
    }
}

/// Couleurs d'accent de GNOME / libadwaita (`accent-color`).
const GNOME_ACCENTS: &[(&str, &str)] = &[
    ("blue", "#3584e4"),
    ("teal", "#2190a4"),
    ("green", "#3a944a"),
    ("yellow", "#c88800"),
    ("orange", "#ed5b00"),
    ("red", "#e62d42"),
    ("pink", "#d56199"),
    ("purple", "#9141ac"),
    ("slate", "#6f8396"),
];

fn hsv(c: Rgb) -> (f64, f64, f64) {
    let max = c.r.max(c.g).max(c.b);
    let min = c.r.min(c.g).min(c.b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == c.r {
        60.0 * (((c.g - c.b) / d).rem_euclid(6.0))
    } else if max == c.g {
        60.0 * ((c.b - c.r) / d + 2.0)
    } else {
        60.0 * ((c.r - c.g) / d + 4.0)
    };
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

/// Accent GNOME le plus proche (teinte d'abord ; gris bleuté si peu saturé).
pub fn nearest_gnome_accent(accent: Rgb) -> &'static str {
    let (h, s, _) = hsv(accent);
    if s < 0.18 {
        return "slate";
    }
    GNOME_ACCENTS
        .iter()
        .filter(|(n, _)| *n != "slate")
        .min_by(|a, b| {
            let dist = |hex: &str| {
                let (h2, s2, _) = hsv(Rgb::parse(hex).unwrap());
                let dh = (h - h2).abs().min(360.0 - (h - h2).abs()) / 180.0;
                dh * 3.0 + (s - s2).abs()
            };
            dist(a.1).total_cmp(&dist(b.1))
        })
        .map(|(n, _)| *n)
        .unwrap_or("blue")
}

/// Boutons de fenêtre et accent pour les applis GTK / libadwaita externes.
pub fn apply_gnome_settings(app: &Appearance) {
    if let Some(wm) = settings("org.gnome.desktop.wm.preferences") {
        set_string(&wm, "button-layout", &gnome_button_layout(&app.button_layout));
    }
    if app.apply_to_all_apps {
        if let (Some(iface), Some(accent)) = (settings("org.gnome.desktop.interface"), accent_of(app)) {
            set_string(&iface, "accent-color", nearest_gnome_accent(accent));
        }
    }
    gio::Settings::sync();
}

fn accent_of(app: &Appearance) -> Option<Rgb> {
    Rgb::parse(&app.accent).or_else(|| Rgb::parse(palette::palette(&app.palette).accent))
}

/// Palette Qt (rôles de `QPalette`, dans l'ordre) : 21 couleurs pour Qt 5, 22 pour Qt 6
/// (rôle `Accent`).
pub fn qt_palette(app: &Appearance, dark: bool, qt6: bool) -> String {
    let v = variant(app, dark);
    let p = |s: &str| Rgb::parse(s).unwrap_or(Rgb::BLACK);
    let (window, fg, view, popover) = (p(v.window_bg), p(v.window_fg), p(v.view_bg), p(v.popover_bg));
    let accent = accent_of(app).unwrap_or(Rgb { r: 0.21, g: 0.52, b: 0.89 });
    let accent_fg = accent.readable_fg();
    let link = accent.standalone_on(view);
    let button = window.mix(fg, if dark { 0.10 } else { 0.06 });
    let roles = |fg: Rgb, text: Rgb, highlight: Rgb, highlight_fg: Rgb| {
        let mut c = vec![
            fg,                                   // WindowText
            button,                               // Button
            button.mix(Rgb::WHITE, 0.15),         // Light
            button.mix(Rgb::WHITE, 0.07),         // Midlight
            button.mix(Rgb::BLACK, 0.35),         // Dark
            button.mix(Rgb::BLACK, 0.20),         // Mid
            text,                                 // Text
            Rgb::WHITE,                           // BrightText
            fg,                                   // ButtonText
            view,                                 // Base
            window,                               // Window
            Rgb::BLACK,                           // Shadow
            highlight,                            // Highlight
            highlight_fg,                         // HighlightedText
            link,                                 // Link
            link.mix(fg, 0.35),                   // LinkVisited
            view.mix(fg, 0.04),                   // AlternateBase
            window,                               // NoRole
            popover,                              // ToolTipBase
            fg,                                   // ToolTipText
            text.mix(view, 0.45),                 // PlaceholderText
        ];
        if qt6 {
            c.push(accent); // Accent
        }
        c.iter().map(|c| format!("#ff{}", &c.hex()[1..])).collect::<Vec<_>>().join(", ")
    };
    let active = roles(fg, fg, accent, accent_fg);
    let dim = fg.mix(window, 0.5);
    let disabled = roles(dim, fg.mix(view, 0.5), accent.mix(window, 0.5), accent_fg.mix(window, 0.3));
    format!("[ColorScheme]\nactive_colors={active}\ndisabled_colors={disabled}\ninactive_colors={active}\n")
}

/// Met à jour quelques clés d'une section d'un fichier INI sans toucher au reste.
fn set_ini_keys(path: &Path, section: &str, keys: &[(&str, String)]) -> std::io::Result<()> {
    let current = std::fs::read_to_string(path).unwrap_or_default();
    let mut out: Vec<String> = vec![];
    let mut in_section = false;
    let mut found_section = false;
    let mut done: Vec<String> = vec![];
    let flush = |out: &mut Vec<String>, done: &mut Vec<String>| {
        for (k, v) in keys {
            if !done.iter().any(|d| d == k) {
                out.push(format!("{k}={v}"));
                done.push(k.to_string());
            }
        }
    };
    for line in current.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_section {
                // Fin de la section : clés manquantes avant la ligne vide éventuelle.
                while out.last().is_some_and(|l| l.trim().is_empty()) {
                    out.pop();
                }
                flush(&mut out, &mut done);
                out.push(String::new());
            }
            in_section = &t[1..t.len() - 1] == section;
            found_section |= in_section;
            out.push(line.to_string());
            continue;
        }
        if in_section {
            if let Some((k, _)) = t.split_once('=') {
                if let Some((key, v)) = keys.iter().find(|(key, _)| *key == k.trim()) {
                    out.push(format!("{key}={v}"));
                    done.push(key.to_string());
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if in_section {
        flush(&mut out, &mut done);
    }
    if !found_section {
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push(format!("[{section}]"));
        flush(&mut out, &mut done);
    }
    let text = out.join("\n") + "\n";
    if text != current {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, text)?;
    }
    Ok(())
}

/// Applis Qt 5 / 6 : palette NOOS par qt5ct / qt6ct (style Fusion, icônes Papirus).
pub fn apply_qt(app: &Appearance, dark: bool) -> anyhow::Result<()> {
    for (name, qt6) in [("qt5ct", false), ("qt6ct", true)] {
        let dir = paths::xdg_config_home().join(name);
        let scheme = dir.join("colors").join("NOOS.conf");
        let conf = dir.join(format!("{name}.conf"));
        if app.apply_to_all_apps {
            std::fs::create_dir_all(scheme.parent().unwrap())?;
            let text = qt_palette(app, dark, qt6);
            if std::fs::read_to_string(&scheme).ok().as_deref() != Some(text.as_str()) {
                std::fs::write(&scheme, text)?;
            }
            set_ini_keys(
                &conf,
                "Appearance",
                &[
                    ("custom_palette", "true".into()),
                    ("color_scheme_path", scheme.display().to_string()),
                    ("style", "Fusion".into()),
                    ("icon_theme", if dark { "Papirus-Dark" } else { "Papirus-Light" }.into()),
                    ("standard_dialogs", "xdgdesktopportal".into()),
                ],
            )?;
        } else if std::fs::read_to_string(&conf).is_ok_and(|c| c.contains(&*scheme.display().to_string())) {
            // Option désactivée : palette Qt par défaut (le reste des réglages est gardé).
            set_ini_keys(&conf, "Appearance", &[("custom_palette", "false".into())])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_layout() {
        assert_eq!(gnome_button_layout("icon:minimize,maximize,close"), "appmenu:minimize,maximize,close");
        assert_eq!(gnome_button_layout("close,minimize,maximize:"), "close,minimize,maximize:");
        assert_eq!(gnome_button_layout("minimize,maximize,close"), ":minimize,maximize,close");
    }

    #[test]
    fn accents() {
        assert_eq!(nearest_gnome_accent(Rgb::parse("#89b4fa").unwrap()), "blue");
        assert_eq!(nearest_gnome_accent(Rgb::parse("#e64553").unwrap()), "red");
        assert_eq!(nearest_gnome_accent(Rgb::parse("#a6e3a1").unwrap()), "green");
        assert_eq!(nearest_gnome_accent(Rgb::parse("#cba6f7").unwrap()), "purple");
        assert_eq!(nearest_gnome_accent(Rgb::parse("#888899").unwrap()), "slate");
    }

    #[test]
    fn qt_palette_roles() {
        let app = Appearance::default();
        let count = |s: &str| s.lines().find(|l| l.starts_with("active_colors=")).unwrap().split(", ").count();
        assert_eq!(count(&qt_palette(&app, true, false)), 21);
        assert_eq!(count(&qt_palette(&app, true, true)), 22);
    }

    #[test]
    fn ini_merge() {
        let dir = std::env::temp_dir().join(format!("noos-ini-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("qt6ct.conf");
        std::fs::write(&p, "[Appearance]\nstyle=Breeze\ncustom_palette=false\n\n[Fonts]\nfixed=\"Mono,10\"\n").unwrap();
        set_ini_keys(&p, "Appearance", &[("custom_palette", "true".into()), ("icon_theme", "Papirus-Dark".into())]).unwrap();
        let t = std::fs::read_to_string(&p).unwrap();
        assert_eq!(t, "[Appearance]\nstyle=Breeze\ncustom_palette=true\nicon_theme=Papirus-Dark\n\n[Fonts]\nfixed=\"Mono,10\"\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

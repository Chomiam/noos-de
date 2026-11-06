//! Palettes prédéfinies et génération des couleurs nommées libAdapta (`@define-color`).
//!
//! libAdapta 1.5 lit, dans `<thème>/libadapta-1.5/`, un fichier `defaults-dark.css` et
//! `defaults-light.css` qui définissent ces couleurs nommées : c'est tout ce qu'il faut pour
//! recolorer entièrement les applications.

use noos_core::config::Appearance;

use crate::color::Rgb;

#[derive(Debug, Clone, Copy)]
pub struct Variant {
    pub window_bg: &'static str,
    pub window_fg: &'static str,
    pub view_bg: &'static str,
    pub headerbar_bg: &'static str,
    pub sidebar_bg: &'static str,
    pub secondary_sidebar_bg: &'static str,
    pub card_bg: &'static str,
    pub popover_bg: &'static str,
    /// Fond de la barre des tâches / menu Démarrer (opacité appliquée par le shell).
    pub panel_bg: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub label: &'static str,
    pub dark: Variant,
    pub light: Variant,
    /// Accent suggéré quand on choisit la palette.
    pub accent: &'static str,
}

pub const PALETTES: &[Palette] = &[
    Palette {
        id: "nuit-saphir",
        label: "Nuit saphir",
        accent: "#89b4fa",
        dark: Variant {
            window_bg: "#1e1e2e",
            window_fg: "#cdd6f4",
            view_bg: "#181825",
            headerbar_bg: "#1e1e2e",
            sidebar_bg: "#181825",
            secondary_sidebar_bg: "#1b1b2a",
            card_bg: "#262637",
            popover_bg: "#2a2a3c",
            panel_bg: "#11111b",
        },
        light: Variant {
            window_bg: "#eff1f5",
            window_fg: "#4c4f69",
            view_bg: "#ffffff",
            headerbar_bg: "#e6e9ef",
            sidebar_bg: "#e6e9ef",
            secondary_sidebar_bg: "#eaedf2",
            card_bg: "#ffffff",
            popover_bg: "#ffffff",
            panel_bg: "#dce0e8",
        },
    },
    Palette {
        id: "mica",
        label: "Mica (style Windows 11)",
        accent: "#4cc2ff",
        dark: Variant {
            window_bg: "#202020",
            window_fg: "#ffffff",
            view_bg: "#1c1c1c",
            headerbar_bg: "#202020",
            sidebar_bg: "#1c1c1c",
            secondary_sidebar_bg: "#1f1f1f",
            card_bg: "#2b2b2b",
            popover_bg: "#2c2c2c",
            panel_bg: "#1c1c1c",
        },
        light: Variant {
            window_bg: "#f3f3f3",
            window_fg: "#1b1b1b",
            view_bg: "#fbfbfb",
            headerbar_bg: "#f3f3f3",
            sidebar_bg: "#ebebeb",
            secondary_sidebar_bg: "#eeeeee",
            card_bg: "#ffffff",
            popover_bg: "#f9f9f9",
            panel_bg: "#eeeeee",
        },
    },
    Palette {
        id: "graphite",
        label: "Graphite",
        accent: "#a6adc8",
        dark: Variant {
            window_bg: "#18191c",
            window_fg: "#e6e6e9",
            view_bg: "#131417",
            headerbar_bg: "#1d1e22",
            sidebar_bg: "#141518",
            secondary_sidebar_bg: "#16171a",
            card_bg: "#222327",
            popover_bg: "#25262b",
            panel_bg: "#0f1012",
        },
        light: Variant {
            window_bg: "#f6f6f7",
            window_fg: "#2a2b2e",
            view_bg: "#ffffff",
            headerbar_bg: "#ececee",
            sidebar_bg: "#ececee",
            secondary_sidebar_bg: "#f0f0f2",
            card_bg: "#ffffff",
            popover_bg: "#ffffff",
            panel_bg: "#e4e4e7",
        },
    },
    Palette {
        id: "aurore",
        label: "Aurore",
        accent: "#f38ba8",
        dark: Variant {
            window_bg: "#221b26",
            window_fg: "#f2e6ee",
            view_bg: "#1b1520",
            headerbar_bg: "#2a2130",
            sidebar_bg: "#1d1722",
            secondary_sidebar_bg: "#201a25",
            card_bg: "#2c2332",
            popover_bg: "#302636",
            panel_bg: "#16111a",
        },
        light: Variant {
            window_bg: "#fbf6f4",
            window_fg: "#4a3b44",
            view_bg: "#ffffff",
            headerbar_bg: "#f5ebe8",
            sidebar_bg: "#f3e8e5",
            secondary_sidebar_bg: "#f7eeeb",
            card_bg: "#ffffff",
            popover_bg: "#ffffff",
            panel_bg: "#efe2de",
        },
    },
    Palette {
        id: "adapta",
        label: "libAdapta (par défaut)",
        accent: "#3584e4",
        dark: Variant {
            window_bg: "#242424",
            window_fg: "#ffffff",
            view_bg: "#1e1e1e",
            headerbar_bg: "#303030",
            sidebar_bg: "#303030",
            secondary_sidebar_bg: "#2a2a2a",
            card_bg: "#2e2e2e",
            popover_bg: "#383838",
            panel_bg: "#1a1a1a",
        },
        light: Variant {
            window_bg: "#fafafa",
            window_fg: "#323232",
            view_bg: "#ffffff",
            headerbar_bg: "#ebebeb",
            sidebar_bg: "#ebebeb",
            secondary_sidebar_bg: "#f3f3f3",
            card_bg: "#ffffff",
            popover_bg: "#ffffff",
            panel_bg: "#e6e6e6",
        },
    },
];

pub fn palette(id: &str) -> &'static Palette {
    PALETTES.iter().find(|p| p.id == id).unwrap_or(&PALETTES[0])
}

pub fn variant(app: &Appearance, dark: bool) -> Variant {
    let p = palette(&app.palette);
    if dark { p.dark } else { p.light }
}

fn rgb(s: &str) -> Rgb {
    Rgb::parse(s).unwrap_or(Rgb::BLACK)
}

/// Toutes les couleurs nommées, dans l'ordre de `defaults-*.css` de libAdapta 1.5.
pub fn named_colors(app: &Appearance, dark: bool) -> Vec<(&'static str, String)> {
    let v = variant(app, dark);
    let fg = v.window_fg;
    let window_bg = rgb(v.window_bg);
    let accent = Rgb::parse(&app.accent).unwrap_or_else(|| rgb(palette(&app.palette).accent));
    let accent_fg = accent.readable_fg();
    let accent_standalone = accent.standalone_on(window_bg);

    let shade = if dark { "rgba(0, 0, 0, 0.36)" } else { "rgba(0, 0, 0, 0.07)" };
    let light_shade = if dark { "rgba(0, 0, 0, 0.25)" } else { "rgba(0, 0, 0, 0.07)" };
    let card_shade = if dark { "rgba(0, 0, 0, 0.36)" } else { "rgba(0, 0, 0, 0.07)" };
    let sidebar_backdrop = rgb(v.sidebar_bg).mix(window_bg, 0.5).hex();
    let secondary_backdrop = rgb(v.secondary_sidebar_bg).mix(window_bg, 0.5).hex();

    let (red, green, yellow) = if dark {
        ("#c01c28", "#26a269", "#cd9309")
    } else {
        ("#e01b24", "#2ec27e", "#e5a50a")
    };
    let (red_text, green_text, yellow_text) = if dark {
        ("#ff7b63", "#8ff0a4", "#f8e45c")
    } else {
        ("#c30000", "#1b8553", "#9c6e03")
    };

    let mut colors: Vec<(&'static str, String)> = vec![
        ("accent_bg_color", accent.hex()),
        ("accent_fg_color", accent_fg.hex()),
        ("accent_color", accent_standalone.hex()),
        ("destructive_bg_color", red.into()),
        ("destructive_fg_color", "white".into()),
        ("destructive_color", red_text.into()),
        ("success_bg_color", green.into()),
        ("success_fg_color", "white".into()),
        ("success_color", green_text.into()),
        ("warning_bg_color", yellow.into()),
        ("warning_fg_color", "rgba(0, 0, 0, 0.8)".into()),
        ("warning_color", yellow_text.into()),
        ("error_bg_color", red.into()),
        ("error_fg_color", "white".into()),
        ("error_color", red_text.into()),
        ("window_bg_color", v.window_bg.into()),
        ("window_fg_color", fg.into()),
        ("view_bg_color", v.view_bg.into()),
        ("view_fg_color", fg.into()),
        ("headerbar_bg_color", v.headerbar_bg.into()),
        ("headerbar_fg_color", fg.into()),
        ("headerbar_border_color", fg.into()),
        ("headerbar_backdrop_color", "@window_bg_color".into()),
        ("headerbar_shade_color", shade.into()),
        ("headerbar_darker_shade_color", if dark { "rgba(0, 0, 0, 0.9)" } else { "rgba(0, 0, 0, 0.12)" }.into()),
        ("sidebar_bg_color", v.sidebar_bg.into()),
        ("sidebar_fg_color", fg.into()),
        ("sidebar_backdrop_color", sidebar_backdrop),
        ("sidebar_shade_color", light_shade.into()),
        ("sidebar_border_color", shade.into()),
        ("secondary_sidebar_bg_color", v.secondary_sidebar_bg.into()),
        ("secondary_sidebar_fg_color", fg.into()),
        ("secondary_sidebar_backdrop_color", secondary_backdrop),
        ("secondary_sidebar_shade_color", light_shade.into()),
        ("secondary_sidebar_border_color", shade.into()),
        ("card_bg_color", v.card_bg.into()),
        ("card_fg_color", fg.into()),
        ("card_shade_color", card_shade.into()),
        ("dialog_bg_color", v.popover_bg.into()),
        ("dialog_fg_color", fg.into()),
        ("popover_bg_color", v.popover_bg.into()),
        ("popover_fg_color", fg.into()),
        ("popover_shade_color", light_shade.into()),
        ("thumbnail_bg_color", v.popover_bg.into()),
        ("thumbnail_fg_color", fg.into()),
        ("shade_color", light_shade.into()),
        ("scrollbar_outline_color", if dark { "rgba(0, 0, 0, 0.5)" } else { "white" }.into()),
    ];

    // Effets de transparence (comme Windows 11) : fonds des fenêtres NOOS translucides ;
    // le compositeur affiche le fond d'écran flouté dessous (Mica). Les menus, dialogues et
    // cartes restent opaques pour la lisibilité.
    if app.panel_opacity < 0.99 {
        let a = (app.panel_opacity + 0.12).clamp(0.6, 0.95);
        let translucent = |hex: &str, alpha: f64| {
            let c = rgb(hex);
            format!(
                "rgba({}, {}, {}, {alpha:.2})",
                (c.r * 255.0).round(),
                (c.g * 255.0).round(),
                (c.b * 255.0).round()
            )
        };
        for (key, hex, alpha) in [
            ("window_bg_color", v.window_bg, a),
            ("headerbar_bg_color", v.headerbar_bg, a),
            ("sidebar_bg_color", v.sidebar_bg, (a - 0.08).max(0.5)),
            ("secondary_sidebar_bg_color", v.secondary_sidebar_bg, (a - 0.04).max(0.5)),
            ("view_bg_color", v.view_bg, (a + 0.08).min(0.97)),
        ] {
            if let Some(entry) = colors.iter_mut().find(|(k, _)| *k == key) {
                entry.1 = translucent(hex, alpha);
            }
        }
    }

    // Surcharges utilisateur : `window_bg_color` ou `window-bg-color`.
    for (key, value) in &app.overrides {
        let key = key.replace('-', "_");
        if let Some(entry) = colors.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value.clone();
        }
    }
    colors
}

pub fn colors_css(app: &Appearance, dark: bool) -> String {
    let mut css = format!(
        "/* Généré par NOOS — palette « {} » ({}) — ne pas modifier : utilisez noos-settings. */\n",
        palette(&app.palette).label,
        if dark { "sombre" } else { "clair" }
    );
    for (name, value) in named_colors(app, dark) {
        css.push_str(&format!("@define-color {name} {value};\n"));
    }
    css
}

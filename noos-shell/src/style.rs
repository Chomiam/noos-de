//! Feuille de style propre au shell (barre, menu Démarrer, panneau rapide),
//! régénérée à chaque changement de thème.

use noos_core::NoosConfig;
use noos_theme::{color::Rgb, palette};

pub fn css(cfg: &NoosConfig, dark: bool) -> String {
    let app = &cfg.appearance;
    let v = palette::variant(app, dark);
    let panel = Rgb::parse(v.panel_bg).unwrap_or(Rgb::BLACK);
    let alpha = app.panel_opacity.clamp(0.3, 1.0);
    let rgba = |c: Rgb, a: f64| {
        format!("rgba({}, {}, {}, {a:.3})", (c.r * 255.0).round(), (c.g * 255.0).round(), (c.b * 255.0).round())
    };
    let bg = rgba(panel, alpha);
    // Le compositeur floute le fond d'écran sous les panneaux (Mica) : les menus peuvent
    // être aussi translucides que la barre (un peu plus couvrants pour la lisibilité).
    let menu_bg = rgba(panel.mix(Rgb::parse(v.window_bg).unwrap_or(panel), 0.5), (alpha + 0.06).min(1.0));
    // Bannières de notification : toujours opaques (lisibles par-dessus n'importe quoi).
    let banner_bg = rgba(panel.mix(Rgb::parse(v.window_bg).unwrap_or(panel), 0.5), 1.0);
    let r = app.corner_radius;
    let height = cfg.taskbar.height.clamp(28, 96);
    // Les boutons suivent la hauteur de la barre (marge de 6 px en haut et en bas).
    let btn = height.saturating_sub(12).max(20);
    let task_w = btn + 8;
    let pad_v = if height < 40 { 1 } else { 4 };
    // Sous 40 px, l'horloge n'affiche que l'heure (comme la petite barre de Windows).
    let date_display = if height < 40 { "font-size: 0; min-height: 0; margin: 0;" } else { "" };

    format!(
        r#"
window.noos-layer {{ background: transparent; }}
/* Pas d'image de fond : dégradé aux couleurs du thème. */
picture.no-wallpaper {{ background: linear-gradient(155deg, shade(@accent_bg_color, 0.45) 0%, @window_bg_color 55%, shade(@window_bg_color, 0.8) 100%); }}

/* ── Barre des tâches ── */
.noos-taskbar {{
  background-color: {bg};
  border-top: 1px solid alpha(@window_fg_color, 0.08);
  min-height: {height}px;
  padding: 0 8px;
}}
.noos-taskbar button {{
  border-radius: 6px;
  padding: {pad_v}px 6px;
  min-height: {btn}px;
  background: none;
  box-shadow: none;
}}
.noos-taskbar button:hover {{ background-color: alpha(@window_fg_color, 0.08); }}
.noos-taskbar button:active {{ background-color: alpha(@window_fg_color, 0.05); }}
.noos-taskbar button.task {{ min-width: {task_w}px; padding: {pad_v}px 4px; }}
.noos-taskbar button.task.focused {{ background-color: alpha(@window_fg_color, 0.07); }}
.noos-taskbar .indicator {{
  min-width: 6px; min-height: 3px; border-radius: 2px;
  background-color: alpha(@window_fg_color, 0.45);
  margin-top: 2px;
}}
.noos-taskbar .indicator.focused {{ min-width: 16px; background-color: @accent_bg_color; }}
.noos-taskbar .indicator.hidden {{ background-color: transparent; }}
.noos-taskbar .clock-time {{ font-weight: 600; font-feature-settings: "tnum"; }}
.noos-taskbar .clock-date {{ font-size: 0.85em; font-feature-settings: "tnum"; }}
.noos-taskbar .search-pill {{
  background-color: alpha(@window_fg_color, 0.06);
  border: 1px solid alpha(@window_fg_color, 0.08);
  border-radius: 99px;
  padding: 0 14px;
  min-width: 180px;
}}
.noos-taskbar .clock-date {{ {date_display} }}
.noos-taskbar .show-desktop {{ min-width: 6px; padding: 0; border-radius: 0; border-left: 1px solid alpha(@window_fg_color, 0.15); }}

/* ── Menu Démarrer / panneau rapide ── */
.noos-flyout {{
  background-color: {menu_bg};
  border: 1px solid alpha(@window_fg_color, 0.10);
  border-radius: {r}px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
  padding: 24px 28px 0 28px;
}}
.noos-flyout .section-title {{ font-weight: 700; }}
.noos-flyout searchentry {{ border-radius: 99px; min-height: 36px; }}
.noos-flyout flowboxchild {{ border-radius: 6px; padding: 0; }}
.noos-flyout .app-tile {{ border-radius: 6px; padding: 10px 4px; background: none; }}
.noos-flyout .app-tile:hover {{ background-color: alpha(@window_fg_color, 0.07); }}
.noos-flyout .app-tile label {{ font-size: 0.85em; }}
.noos-flyout .footer {{
  margin: 16px -28px 0 -28px;
  padding: 10px 28px;
  background-color: alpha(@window_fg_color, 0.04);
  border-top: 1px solid alpha(@window_fg_color, 0.08);
  border-bottom-left-radius: {r}px; border-bottom-right-radius: {r}px;
}}
.noos-flyout .footer button {{ background: none; border-radius: 6px; }}
.noos-flyout .footer button:hover {{ background-color: alpha(@window_fg_color, 0.07); }}
.noos-flyout list {{ background: none; }}
.noos-flyout .tile {{ border-radius: 6px; min-height: 48px; min-width: 96px; }}
.noos-flyout .tile:checked {{ background-color: @accent_bg_color; color: @accent_fg_color; }}
.noos-flyout.quick {{ padding-bottom: 0; }}
.noos-flyout .power-row {{ margin: 8px 0 0 0; }}
.noos-flyout .power-action {{ border-radius: 8px; padding: 10px 4px; }}
.noos-flyout .power-action:hover {{ background-color: alpha(@window_fg_color, 0.08); }}
.noos-flyout .power-action.power-off:hover {{ background-color: alpha(@destructive_bg_color, 0.85); color: white; }}

/* ── Réseau, volume, réglages rapides ── */
.noos-taskbar button.status-icon {{ padding: {pad_v}px 7px; min-width: 0; }}
.noos-taskbar menubutton.status-icon > button {{ padding: {pad_v}px 7px; min-width: 0; }}
.noos-taskbar menubutton.vpn-active > button image {{ color: #3ccf72; }}
.vpn-menu {{ padding: 6px; min-width: 240px; }}
.vpn-menu .vpn-menu-title {{ font-weight: 700; opacity: 0.7; font-size: 0.85em; margin: 2px 8px 6px 8px; }}
.vpn-menu button.vpn-item {{ border-radius: 6px; padding: 8px 10px; }}
.vpn-menu button.vpn-item label {{ font-weight: normal; }}
.vpn-menu .vpn-check {{ color: #3ccf72; }}
.vpn-menu separator {{ margin: 4px 6px; }}
.audio-flyout .section-title, .net-flyout .section-title {{ margin-top: 4px; }}
.net-flyout .net-status {{ padding: 4px 0 8px 0; border-bottom: 1px solid alpha(@window_fg_color, 0.08); }}
.noos-flyout list.boxed-list {{ background-color: alpha(@window_fg_color, 0.04); }}

/* ── Zone de notification, cloche ── */
.noos-taskbar button.tray-icon, .noos-taskbar .tray-overflow > button {{ padding: {pad_v}px 5px; min-width: 0; }}
.noos-taskbar .bell-badge {{
  background-color: @accent_bg_color; color: @accent_fg_color;
  border-radius: 99px; font-size: 0.62em; font-weight: 700;
  min-width: 13px; min-height: 13px; padding: 0 2px;
  margin: -5px -7px 0 0;
}}
popover.tray-popover > contents {{ background-color: {menu_bg}; border-radius: {r}px; padding: 6px; }}
.tray-grid button.tray-icon {{ border-radius: 6px; padding: 8px; background: none; }}
.tray-grid button.tray-icon:hover {{ background-color: alpha(@window_fg_color, 0.08); }}
popover.tray-menu > contents {{ background-color: {menu_bg}; border-radius: 8px; padding: 4px; min-width: 200px; }}
popover.tray-menu button {{ border-radius: 4px; padding: 6px 10px; font-weight: normal; }}

/* ── Notifications ── */
.noos-flyout.notif-center {{ padding: 16px; }}
.notif-card {{
  background-color: alpha(@window_fg_color, 0.05);
  border: 1px solid alpha(@window_fg_color, 0.07);
  border-radius: {r}px;
  padding: 12px 14px;
}}
.notif-card:hover {{ background-color: alpha(@window_fg_color, 0.08); }}
.notif-card.banner:hover {{ background-color: shade({banner_bg}, 1.08); }}
.notif-card.banner {{
  background-color: {banner_bg};
  border: 1px solid alpha(@window_fg_color, 0.10);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35);
  margin: 6px;
}}
.notif-card.critical {{ border-left: 3px solid @error_color; }}
.notif-card .notif-app, .notif-card .notif-time {{ font-size: 0.8em; opacity: 0.7; }}
.notif-card .notif-summary {{ font-weight: 700; }}
.notif-card .notif-body {{ opacity: 0.85; }}
.notif-card .notif-close {{ min-width: 22px; min-height: 22px; padding: 0; border-radius: 4px; }}
.notif-card .notif-image {{ border-radius: 6px; }}
.notif-card .notif-action {{ border-radius: 6px; }}
.calendar-panel calendar {{ background: none; border: none; }}
"#
    )
}

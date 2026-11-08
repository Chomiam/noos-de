//! Briques d'interface « façon Windows 11 » : pages avec fil d'Ariane, cartes avec icône,
//! liens vers une sous-page, exécution d'actions système en arrière-plan.

use std::{cell::RefCell, future::Future, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_core::{
    NoosConfig,
    ipc::{Event, IpcClient, Request},
};

pub struct App {
    pub window: adap::ApplicationWindow,
    pub config: RefCell<NoosConfig>,
    pub toasts: adap::ToastOverlay,
    pub nav: adap::NavigationView,
}

impl App {
    pub fn cfg(&self) -> NoosConfig {
        self.config.borrow().clone()
    }

    /// Sauvegarde la configuration, régénère le thème si besoin, prévient le compositeur.
    pub fn update(&self, f: impl FnOnce(&mut NoosConfig)) {
        let mut cfg = self.config.borrow_mut();
        let before = cfg.clone();
        f(&mut cfg);
        if *cfg == before {
            return;
        }
        if let Err(e) = cfg.save() {
            self.toast(&format!("Sauvegarde impossible : {e}"));
            return;
        }
        if cfg.appearance != before.appearance {
            if let Err(e) = noos_theme::install_theme(&cfg.appearance) {
                eprintln!("noos-settings : thème : {e:#}");
            }
        }
        std::thread::spawn(|| {
            let _ = IpcClient::fire(&Request::ReloadConfig);
        });
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    /// Exécute un plan (commandes admin) puis appelle `done(succès)`.
    pub fn run_plan(self: &Rc<Self>, title: &str, plan: anyhow::Result<noos_pkg::Plan>, done: impl FnOnce(bool) + 'static) {
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                self.error(title, &format!("{e:#}"));
                done(false);
                return;
            }
        };
        let task = background(move || -> Result<(), String> {
            let log = std::sync::Mutex::new(Vec::<String>::new());
            let res: anyhow::Result<()> = noos_pkg::Plan::run(&plan, &|l: &str| log.lock().unwrap().push(l.to_string()));
            let log = log.into_inner().unwrap();
            res.map_err(|e| format!("{e:#}\n\n{}", log.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n")))
        });
        let (app, title) = (self.clone(), title.to_string());
        glib::spawn_future_local(async move {
            match task.await {
                Ok(()) => {
                    app.toast(&format!("{title} : terminé"));
                    done(true);
                }
                Err(e) => {
                    app.error(&title, &e);
                    done(false);
                }
            }
        });
    }

    pub fn error(&self, title: &str, body: &str) {
        let d = adap::AlertDialog::new(Some(&format!("Échec : {title}")), Some(body));
        d.add_response("ok", "Fermer");
        d.present(Some(&self.window));
    }

    pub fn confirm(&self, title: &str, body: &str, action: &str, destructive: bool, f: impl Fn() + 'static) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance("ok", if destructive { adap::ResponseAppearance::Destructive } else { adap::ResponseAppearance::Suggested });
        d.set_default_response(Some("cancel"));
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f()
            }
        });
        d.present(Some(&self.window));
    }

    pub fn push(&self, page: &adap::NavigationPage) {
        self.nav.push(page);
    }
}

/// Exécute `f` dans un thread, résultat sur la boucle GTK.
pub fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("thread interrompu") }
}

/// Requête IPC asynchrone vers le compositeur.
pub fn ipc(req: Request) -> impl Future<Output = Result<Event, String>> {
    background(move || IpcClient::request(&req).map_err(|e| e.to_string()))
}

// ───────────────────────────── Pages ─────────────────────────────

/// Page avec fil d'Ariane : `crumbs` = parents (cliquables pour revenir), `title` = page courante.
pub fn page(app: &Rc<App>, crumbs: &[&str], title: &str) -> (adap::NavigationPage, gtk::Box) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.set_margin_top(8);
    content.set_margin_bottom(36);
    content.set_margin_start(36);
    content.set_margin_end(36);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header.add_css_class("win11-crumbs");
    let depth = crumbs.len();
    for (i, c) in crumbs.iter().enumerate() {
        let b = gtk::Button::with_label(c);
        b.add_css_class("flat");
        b.add_css_class("win11-crumb");
        let (app, back) = (app.clone(), (depth - i) as u32);
        b.connect_clicked(move |_| {
            for _ in 0..back {
                app.nav.pop();
            }
        });
        header.append(&b);
        let sep = gtk::Label::new(Some("›"));
        sep.add_css_class("win11-crumb-sep");
        header.append(&sep);
    }
    let t = gtk::Label::new(Some(title));
    t.add_css_class("win11-title");
    t.set_xalign(0.0);
    header.append(&t);
    content.append(&header);

    let clamp = adap::Clamp::builder().maximum_size(1000).tightening_threshold(700).child(&content).build();
    let scroll = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    // Tests (captures) : `NOOS_SETTINGS_SCROLL=<px>` fait défiler la page après son affichage.
    if let Some(y) = std::env::var("NOOS_SETTINGS_SCROLL").ok().and_then(|v| v.parse::<f64>().ok()) {
        let s = scroll.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || s.vadjustment().set_value(y));
    }
    let tv = adap::ToolbarView::new();
    let hb = adap::HeaderBar::new();
    hb.set_show_title(false);
    hb.set_show_back_button(false);
    tv.add_top_bar(&hb);
    tv.set_content(Some(&scroll));
    let nav_page = adap::NavigationPage::new(&tv, title);
    (nav_page, content)
}

pub fn group(title: &str) -> adap::PreferencesGroup {
    let g = adap::PreferencesGroup::new();
    if !title.is_empty() {
        g.set_title(title);
    }
    g
}

fn icon(name: &str) -> gtk::Image {
    let i = gtk::Image::from_icon_name(name);
    i.set_pixel_size(20);
    i.add_css_class("win11-row-icon");
    i
}

/// Carte simple : icône, titre, sous-titre.
pub fn card(icon_name: &str, title: &str, subtitle: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    if !subtitle.is_empty() {
        r.set_subtitle(subtitle);
    }
    if !icon_name.is_empty() {
        r.add_prefix(&icon(icon_name));
    }
    r
}

/// Carte qui ouvre une sous-page (chevron à droite, comme Windows 11).
pub fn link(app: &Rc<App>, icon_name: &str, title: &str, subtitle: &str, build: impl Fn(&Rc<App>) -> adap::NavigationPage + 'static) -> adap::ActionRow {
    let r = card(icon_name, title, subtitle);
    r.set_activatable(true);
    r.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let app = app.clone();
    r.connect_activated(move |_| {
        let p = build(&app);
        app.push(&p);
    });
    r
}

pub fn combo(icon_name: &str, title: &str, subtitle: &str, items: &[String], selected: usize) -> adap::ComboRow {
    let row = adap::ComboRow::new();
    row.set_use_markup(false);
    row.set_title(title);
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk::StringList::new(&refs)));
    row.set_selected(selected.min(items.len().saturating_sub(1)) as u32);
    if !icon_name.is_empty() {
        row.add_prefix(&icon(icon_name));
    }
    row
}

pub fn switch(icon_name: &str, title: &str, subtitle: &str, active: bool) -> adap::SwitchRow {
    let row = adap::SwitchRow::new();
    row.set_use_markup(false);
    row.set_title(title);
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.set_active(active);
    if !icon_name.is_empty() {
        row.add_prefix(&icon(icon_name));
    }
    row
}

pub fn spin(icon_name: &str, title: &str, subtitle: &str, (min, max, step): (f64, f64, f64), value: f64) -> adap::SpinRow {
    let row = adap::SpinRow::with_range(min, max, step);
    row.set_use_markup(false);
    row.set_title(title);
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row.set_value(value);
    if !icon_name.is_empty() {
        row.add_prefix(&icon(icon_name));
    }
    row
}

/// Bloc « héros » en tête de page (grande icône/image + textes + actions).
pub fn hero(visual: &impl IsA<gtk::Widget>, title: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    h.add_css_class("win11-hero");
    h.append(visual);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("title-2");
    t.set_xalign(0.0);
    t.set_wrap(true);
    v.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.add_css_class("dim-label");
        s.set_xalign(0.0);
        s.set_wrap(true);
        v.append(&s);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    h.append(&v);
    h.append(&actions);
    actions.set_valign(gtk::Align::Center);
    (h, actions)
}

pub const CSS: &str = r#"
/* ── Barre latérale ── */
.win11-sidebar { background: transparent; }
.win11-sidebar row { border-radius: 6px; min-height: 38px; padding: 0 10px; margin: 1px 6px; }
.win11-sidebar row:selected { background-color: alpha(@window_fg_color, 0.08); color: @window_fg_color;
  box-shadow: inset 3px 0 0 @accent_bg_color; }
.win11-sidebar row:hover:not(:selected) { background-color: alpha(@window_fg_color, 0.04); }
.win11-user { padding: 8px 14px 14px 14px; }
.win11-user .name { font-weight: 600; font-size: 1.05em; }
.win11-search { margin: 0 12px 10px 12px; min-height: 34px; }
.nav-blue { color: #4cc2ff; } .nav-orange { color: #ff9f43; } .nav-green { color: #6ccb5f; }
.nav-purple { color: #c39bff; } .nav-grey { color: alpha(@window_fg_color, 0.75); } .nav-teal { color: #42d6c4; }
.nav-pink { color: #ff8fb1; }

/* ── Pages ── */
.win11-title { font-size: 28px; font-weight: 600; }
.win11-crumb { font-size: 28px; font-weight: 600; padding: 0 4px; color: alpha(@window_fg_color, 0.55); }
.win11-crumb:hover { color: @window_fg_color; }
.win11-crumb-sep { font-size: 22px; color: alpha(@window_fg_color, 0.5); }
.win11-hero { background-color: @card_bg_color; border-radius: 8px; padding: 22px 24px;
  border: 1px solid alpha(@card_fg_color, 0.06); }
.win11-row-icon { margin-right: 6px; }

/* Cartes séparées comme Windows 11 (au lieu d'une liste continue). */
.win11 list.boxed-list { background: transparent; box-shadow: none; border: none; }
.win11 list.boxed-list > row { background-color: @card_bg_color; border-radius: 6px; margin-bottom: 4px;
  min-height: 64px; border: 1px solid alpha(@card_fg_color, 0.06); border-bottom-width: 1px; }
.win11 list.boxed-list > row:hover { background-color: mix(@card_bg_color, @window_fg_color, 0.03); }
.win11 list.boxed-list > row.expander { padding: 0; }
.win11 preferencesgroup > box > box > label.title { font-weight: 600; font-size: 14px; }

/* ── Pare-feu ── */
.fw-hero { padding: 28px 28px; }
.fw-state-ok { color: #3ccf72; } .fw-state-bad { color: #f0564a; } .fw-state-warn { color: #ffb43c; }
.fw-backend { font-size: 0.85em; color: alpha(@window_fg_color, 0.55); }

/* ── Écran ── */
.monitor-box { background-color: alpha(@window_fg_color, 0.03); border-radius: 8px; padding: 24px; }
.monitor { background-color: @accent_bg_color; color: @accent_fg_color; border-radius: 4px; font-size: 1.6em; font-weight: 700; }
.monitor.inactive { background-color: alpha(@window_fg_color, 0.25); }

/* ── Personnalisation ── */
.palette-tile { border-radius: 8px; padding: 0; min-width: 120px; min-height: 76px; border: 2px solid transparent; }
.palette-tile.selected { border-color: @accent_bg_color; }
.wallpaper-preview { border-radius: 8px; }
.tiling-illustration { border-radius: 8px; }
.login-preview-time { font-size: 34px; font-weight: 600; color: white; text-shadow: 0 1px 6px rgba(0,0,0,0.5); }
.login-preview-date { font-size: 11px; color: white; text-shadow: 0 1px 4px rgba(0,0,0,0.6); }
.preview-taskbar { min-height: 14px; background-color: alpha(black, 0.45); border-bottom-left-radius: 8px; border-bottom-right-radius: 8px; }
button.current-wallpaper .wallpaper-preview { outline: 2px solid @accent_color; outline-offset: 2px; }
.selection-bar { padding: 8px 12px; border-radius: 8px; background-color: alpha(@window_fg_color, 0.05); }
checkbutton.selection-check check { background-color: alpha(black, 0.4); border: 2px solid white; min-width: 18px; min-height: 18px; }
checkbutton.selection-check check:checked { background-color: @accent_bg_color; border-color: @accent_bg_color; }
picture.no-wallpaper { background: linear-gradient(155deg, shade(@accent_bg_color, 0.45) 0%, @window_bg_color 55%, shade(@window_bg_color, 0.8) 100%); }
/* ── Écran de verrouillage ── */
.ls-preview { border-radius: 12px; background: black; }
.ls-clock { color: white; font-size: 44px; font-weight: 300; text-shadow: 0 1px 10px rgba(0,0,0,0.5); }
button.ls-tile { padding: 8px; border-radius: 12px; background: transparent; }
button.ls-tile:hover { background-color: alpha(@window_fg_color, 0.05); }
button.ls-tile .ls-thumb { border-radius: 10px; background: black; border: 2px solid transparent; }
button.ls-tile.active .ls-thumb { border-color: @accent_bg_color; }
button.ls-tile .ls-name { font-weight: 700; }
button.ls-tile.incompatible { opacity: 0.45; }
.ls-badge { background: @accent_bg_color; color: @accent_fg_color; border-radius: 999px; padding: 3px; }
.ls-error { color: white; background: rgba(0,0,0,0.6); padding: 16px; }

/* ── Applications par défaut ── */
button.da-tile { background-color: @card_bg_color; border: 1px solid alpha(@card_fg_color, 0.06); border-radius: 10px; padding: 12px 14px; }
button.da-tile:hover { background-color: mix(@card_bg_color, @window_fg_color, 0.05); }
button.da-tile .da-tile-app { font-weight: 700; }
.da-ext { font-family: monospace; font-size: 0.78em; padding: 1px 6px; border-radius: 5px; background: alpha(@window_fg_color, 0.07); }

/* ── Réseau ── */
.net-hero-icon { border-radius: 999px; min-width: 72px; min-height: 72px; }
.net-hero-icon.net-ok { background: alpha(#3ccf72, 0.15); }
.net-hero-icon.net-bad { background: alpha(#f0564a, 0.15); }
.net-hero-icon.net-idle { background: alpha(@window_fg_color, 0.08); }
.net-ok { color: #3ccf72; } .net-bad { color: #f0564a; } .net-idle { color: alpha(@window_fg_color, 0.6); }
.net-badge { border-radius: 999px; padding: 2px 10px; font-size: 0.8em; font-weight: 700; }
.net-badge.net-ok { background: alpha(#3ccf72, 0.16); }
.net-badge.net-mid { background: alpha(#ffb43c, 0.16); color: #ffb43c; }
.net-badge.net-bad { background: alpha(#f0564a, 0.16); }
.net-badge.net-filter { background: alpha(#c39bff, 0.18); color: #c39bff; }
.net-badge.net-vpn { background: alpha(@accent_bg_color, 0.2); color: @accent_color; }
.net-flag { font-size: 22px; min-width: 32px; }
.net-block { padding: 6px 10px; border-radius: 8px; font-size: 0.9em; }
.net-block.on { background: alpha(#3ccf72, 0.14); color: #3ccf72; font-weight: 600; }
.net-block.off { background: alpha(@window_fg_color, 0.05); color: alpha(@window_fg_color, 0.55); }
.net-flag-big { font-size: 52px; }
.net-speed { font-size: 0.85em; font-weight: 700; color: alpha(@window_fg_color, 0.65); }
.net-speed.net-fastest { color: #3ccf72; }
.net-down { color: #4cc2ff; } .net-up { color: #ff8fb1; }
.net-graph { background: alpha(@window_fg_color, 0.03); border-radius: 6px; }
.swatch { min-width: 30px; min-height: 30px; border-radius: 99px; padding: 0; }
"#;

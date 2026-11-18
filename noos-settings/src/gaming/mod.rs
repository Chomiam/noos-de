//! Paramètres › Jeux : **mode jeu Steam** (session gamescope façon Steam Deck), composants
//! pour le jeu (GameMode, MangoHud, GOverlay, ProtonPlus…), puis les réglages d'affichage du
//! bureau (VRR, scanout direct, HDR) et le mode Jeu du compositeur.
//!
//! Le mode jeu installe gamescope et Steam (contrib) par `noos-pkg`, puis
//! `noos-gaming-helper` (pkexec) crée la session « Steam (mode jeu) » que l'écran de connexion
//! propose. Le raccourci « Mode jeu Steam » et « Passer au bureau » dans Steam passent d'une
//! session à l'autre sans mot de passe (`noos-session-switch`, connexion automatique
//! ponctuelle de greetd). Démo sans rien exécuter : `NOOS_GAMING_DEMO=1|on|installing`.

pub mod model;

use std::{cell::RefCell, collections::BTreeSet, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::glib;
use noos_core::{config::VrrPolicy, ipc::OutputInfo};
use noos_pkg::{Backend, Plan};

use crate::ui::{App, background, card, combo, group, page as ui_page, switch};
use model::{Boot, COMPONENTS, Component, SessionConf, Source, Status};

/// Ce qui est en cours (un seul à la fois : apt n'aime pas deux installations en même temps).
#[derive(Clone, Debug, PartialEq)]
enum Busy {
    Idle,
    /// Activation du mode jeu : dernière ligne du journal.
    Mode(String),
    Component(&'static str),
    Decky,
    Helper,
}

struct Ctx {
    app: Rc<App>,
    holder: gtk::Box,
    status: RefCell<Option<Status>>,
    busy: RefCell<Busy>,
    outputs: RefCell<Vec<OutputInfo>>,
    conf: RefCell<SessionConf>,
    /// Ligne de journal de l'activation (mise à jour sans tout reconstruire).
    log_label: RefCell<Option<gtk::Label>>,
}

const STEAM_AGREEMENT: &str = "https://store.steampowered.com/subscriber_agreement/";
const DECKY_URL: &str = "https://decky.xyz";
/// Licence de Steam acceptée par l'utilisateur dans la fenêtre de confirmation.
const PRESEED: &str = "printf '%s\\n' 'steam-installer steam/question select I AGREE' 'steam-installer steam/license note ' | debconf-set-selections";

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = ui_page(app, &[], "Jeux");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    display_groups(app, &content);

    let ctx = Rc::new(Ctx {
        app: app.clone(),
        holder,
        status: RefCell::new(None),
        busy: RefCell::new(Busy::Idle),
        outputs: RefCell::new(vec![]),
        conf: RefCell::new(SessionConf::load()),
        log_label: RefCell::new(None),
    });
    if model::demo_mode().is_some() {
        *ctx.conf.borrow_mut() = SessionConf { resolution: Some((2560, 1440)), refresh: 165, ..Default::default() };
    }
    // Squelette pendant la détection (dpkg, flatpak, systemctl : ≈ 100 ms).
    let wait = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    wait.add_css_class("game-hero");
    let sp = gtk::Spinner::new();
    sp.start();
    wait.append(&sp);
    wait.append(&gtk::Label::new(Some("Mode jeu Steam…")));
    ctx.holder.append(&wait);
    refresh(&ctx);
    {
        let ctx = ctx.clone();
        let task = crate::ui::ipc(noos_core::ipc::Request::GetOutputs);
        glib::spawn_future_local(async move {
            if let Ok(noos_core::ipc::Event::Outputs { outputs }) = task.await {
                *ctx.outputs.borrow_mut() = outputs;
                if ctx.status.borrow().is_some() {
                    render(&ctx);
                }
            }
        });
    }
    if model::demo_mode().as_deref() == Some("installing") {
        let ctx = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(300), move || demo_install(&ctx, true));
    }
    if let Ok(d) = std::env::var("NOOS_GAMING_DIALOG") {
        let ctx = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(1200), move || match d.as_str() {
            "enable" => ask_enable(&ctx),
            "disable" => ask_disable(&ctx),
            "decky" => ask_decky(&ctx, true),
            "switch" => ask_switch(&ctx.app),
            _ => {}
        });
    }
    p
}

/// Ouvre la confirmation « Passer en mode jeu » (raccourci : `noos-settings steam-mode`).
pub fn ask_switch(app: &Rc<App>) {
    let d = adap::AlertDialog::new(
        Some("Passer en mode jeu Steam ?"),
        Some(
            "Le bureau se ferme : enregistrez votre travail, les applications ouvertes seront quittées. \
             Steam s'ouvre ensuite en plein écran, comme sur un Steam Deck.\n\n\
             Pour revenir : bouton Marche/Arrêt de Steam › Passer au bureau.",
        ),
    );
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Passer en mode jeu")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let app2 = app.clone();
    d.connect_response(None, move |_, r| {
        let app = &app2;
        if r != "ok" {
            return;
        }
        if model::demo_mode().is_some() {
            app.toast("Démo : bascule simulée, rien n'est exécuté");
            return;
        }
        let task = background(|| {
            std::process::Command::new("pkexec").args([model::SWITCH, "steam"]).output().map_err(|e| e.to_string()).and_then(|o| {
                if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
            })
        });
        let app = app.clone();
        glib::spawn_future_local(async move {
            match task.await {
                // La bascule attend la fin de la session : on quitte le bureau proprement.
                Ok(()) => {
                    let _ = noos_core::ipc::IpcClient::fire(&noos_core::ipc::Request::Quit);
                }
                Err(e) => app.error("Passage en mode jeu", &e),
            }
        });
    });
    d.present(Some(&app.window));
}

// ───────────────────────── État ─────────────────────────

fn refresh(ctx: &Rc<Ctx>) {
    let demo = ctx.status.borrow().as_ref().is_some_and(|s| s.demo);
    if demo {
        render(ctx);
        return;
    }
    let ctx = ctx.clone();
    let task = background(model::detect);
    glib::spawn_future_local(async move {
        *ctx.status.borrow_mut() = Some(task.await);
        render(&ctx);
    });
}

fn render(ctx: &Rc<Ctx>) {
    let Some(st) = ctx.status.borrow().clone() else { return };
    while let Some(c) = ctx.holder.first_child() {
        ctx.holder.remove(&c);
    }
    *ctx.log_label.borrow_mut() = None;
    ctx.holder.append(&hero(ctx, &st));
    if st.enabled() {
        ctx.holder.append(&session_group(ctx, &st));
        ctx.holder.append(&display_group(ctx));
        ctx.holder.append(&decky_group(ctx, &st));
    }
    ctx.holder.append(&components_group(ctx, &st));
}

fn icon_name(candidates: &[&'static str]) -> &'static str {
    let theme = gtk::gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
    for c in candidates {
        if theme.as_ref().is_some_and(|t| t.has_icon(c)) {
            return c;
        }
    }
    candidates.last().copied().unwrap_or("application-x-executable-symbolic")
}

fn chip(text: &str, ok: bool) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    b.add_css_class("game-chip");
    b.add_css_class(if ok { "ok" } else { "missing" });
    let i = gtk::Image::from_icon_name(if ok { "object-select-symbolic" } else { "list-add-symbolic" });
    i.set_pixel_size(12);
    b.append(&i);
    b.append(&gtk::Label::new(Some(text)));
    b
}

fn feature(icon: &str, title: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("game-feature");
    b.set_hexpand(true);
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(20);
    i.set_valign(gtk::Align::Start);
    b.append(&i);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    v.append(&t);
    let s = gtk::Label::new(Some(text));
    s.add_css_class("caption");
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_max_width_chars(28);
    v.append(&s);
    b.append(&v);
    b
}

// ───────────────────────── Bannière ─────────────────────────

fn hero(ctx: &Rc<Ctx>, st: &Status) -> gtk::Box {
    let busy = ctx.busy.borrow().clone();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 18);
    outer.add_css_class("game-hero");
    if st.enabled() {
        outer.add_css_class("on");
    }
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    let logo = gtk::Image::from_icon_name(icon_name(&["steam", "com.valvesoftware.Steam", "input-gaming-symbolic"]));
    logo.set_pixel_size(84);
    logo.add_css_class("game-hero-logo");
    logo.set_valign(gtk::Align::Center);
    top.append(&logo);

    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    v.set_hexpand(true);
    v.set_valign(gtk::Align::Center);
    let kicker = gtk::Label::new(Some(if st.enabled() { "ACTIVÉ" } else { "SESSION DE JEU" }));
    kicker.add_css_class("game-kicker");
    kicker.set_xalign(0.0);
    v.append(&kicker);
    let t = gtk::Label::new(Some("Mode jeu Steam"));
    t.add_css_class("game-hero-title");
    t.set_xalign(0.0);
    v.append(&t);
    let s = gtk::Label::new(Some(
        "L'interface du Steam Deck en plein écran sur votre PC, dans sa propre session gamescope : \
         tout à la manette, sans le bureau, et retour au bureau en un clic.",
    ));
    s.add_css_class("game-hero-sub");
    s.set_xalign(0.0);
    s.set_wrap(true);
    v.append(&s);
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.set_margin_top(6);
    chips.append(&chip("Gamescope", st.gamescope));
    chips.append(&chip("Steam", st.steam));
    chips.append(&chip("Session", st.session));
    chips.append(&chip("Decky", st.decky.installed));
    v.append(&chips);
    top.append(&v);

    let right = gtk::Box::new(gtk::Orientation::Vertical, 10);
    right.set_valign(gtk::Align::Center);
    right.set_halign(gtk::Align::End);
    if let Busy::Mode(line) = &busy {
        let sp = gtk::Spinner::new();
        sp.set_size_request(40, 40);
        sp.start();
        right.append(&sp);
        let l = gtk::Label::new(Some("Installation…"));
        l.add_css_class("heading");
        right.append(&l);
        let log = gtk::Label::new(Some(line));
        log.add_css_class("game-log");
        log.set_xalign(0.0);
        log.set_ellipsize(gtk::pango::EllipsizeMode::End);
        log.set_max_width_chars(80);
        *ctx.log_label.borrow_mut() = Some(log.clone());
        top.append(&right);
        outer.append(&top);
        outer.append(&log);
        return outer;
    }
    let sw_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    sw_box.set_halign(gtk::Align::End);
    let state = gtk::Label::new(Some(if st.enabled() { "Activé" } else { "Désactivé" }));
    state.add_css_class("game-switch-label");
    sw_box.append(&state);
    let sw = gtk::Switch::new();
    sw.add_css_class("game-switch");
    sw.set_active(st.enabled());
    sw.set_valign(gtk::Align::Center);
    sw.set_sensitive(busy == Busy::Idle && st.helper);
    {
        let ctx = ctx.clone();
        sw.connect_state_set(move |_, on| {
            if on { ask_enable(&ctx) } else { ask_disable(&ctx) }
            glib::Propagation::Stop
        });
    }
    sw_box.append(&sw);
    right.append(&sw_box);
    if st.enabled() {
        let go = gtk::Button::new();
        let c = adap::ButtonContent::builder().icon_name("media-playback-start-symbolic").label("Lancer le mode jeu").build();
        go.set_child(Some(&c));
        go.add_css_class("suggested-action");
        go.add_css_class("pill");
        go.set_sensitive(!st.broken());
        let app = ctx.app.clone();
        go.connect_clicked(move |_| ask_switch(&app));
        right.append(&go);
    }
    top.append(&right);
    outer.append(&top);

    if !st.helper {
        outer.append(&notice("dialog-warning-symbolic", "Le paquet NOOS installé est trop ancien pour le mode jeu (noos-gaming-helper absent) : mettez NOOS à jour."));
    } else if st.broken() {
        outer.append(&notice("dialog-warning-symbolic", "Gamescope ou Steam a été désinstallé : réactivez le mode jeu pour le réinstaller."));
    } else if !st.enabled() {
        let f = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        f.set_homogeneous(true);
        f.append(&feature("input-gaming-symbolic", "Tout à la manette", "Big Picture, menu rapide, clavier virtuel de Steam"));
        f.append(&feature("speedometer-symbolic", "Gamescope", "VRR, HDR, FSR, limite d'images et statistiques par jeu"));
        f.append(&feature("application-x-addon-symbolic", "Decky Loader", "Greffons : thèmes, réglages des performances, jaquettes"));
        f.append(&feature("go-home-symbolic", "Retour au bureau", "« Passer au bureau » rouvre NOOS, sans mot de passe"));
        outer.append(&f);
        if !st.contrib {
            outer.append(&notice(
                "dialog-information-symbolic",
                "Steam et gamescope sont dans les composants contrib et non-free de Debian : ils seront activés en même temps.",
            ));
        }
    }
    outer
}

fn notice(icon: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.add_css_class("game-notice");
    b.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    b.append(&l);
    b
}

// ───────────────────────── Activation ─────────────────────────

fn missing_mode_packages(st: &Status) -> Vec<&'static str> {
    let mut v = vec![];
    if !st.gamescope {
        v.push("gamescope");
    }
    if !st.steam {
        v.push("steam-installer");
    }
    if !st.mangoapp {
        v.push("mangoapp");
    }
    v
}

fn bullets(lines: &[String]) -> String {
    lines.iter().map(|l| format!("•  {l}")).collect::<Vec<_>>().join("\n")
}

fn ask_enable(ctx: &Rc<Ctx>) {
    let Some(st) = ctx.status.borrow().clone() else { return };
    rerender_later(ctx);
    let missing = missing_mode_packages(&st);
    let mut steps = vec![];
    if !st.contrib {
        steps.push("Activer contrib et non-free dans les sources de Debian (Steam et gamescope y sont)".to_string());
    }
    if !missing.is_empty() {
        steps.push(format!("Installer {} (apt)", missing.join(", ")));
    }
    steps.push("Ajouter la session « Steam (mode jeu) » à l'écran de connexion".into());
    steps.push("Ajouter le raccourci « Mode jeu Steam » au menu Démarrer et au bureau".into());
    let mut body = format!("{}\n\nLe mot de passe administrateur est demandé une fois.", bullets(&steps));
    if !st.steam {
        body.push_str(" Au premier lancement, Steam télécharge sa mise à jour (≈ 300 Mo).");
    }
    let d = adap::AlertDialog::new(Some("Activer le mode jeu Steam ?"), Some(&body));
    if !st.steam {
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let l = gtk::Label::new(Some("En installant Steam, vous acceptez le contrat d'abonné Steam de Valve."));
        l.set_wrap(true);
        l.add_css_class("caption");
        v.append(&l);
        let link = gtk::LinkButton::with_label(STEAM_AGREEMENT, "Lire le contrat d'abonné Steam");
        link.set_halign(gtk::Align::Center);
        v.append(&link);
        d.set_extra_child(Some(&v));
    }
    d.add_responses(&[("cancel", "Annuler"), ("ok", if st.steam { "Activer" } else { "Accepter et activer" })]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, r| {
        if r != "ok" {
            return;
        }
        let ctx = ctx2.clone();
        if st.demo {
            demo_install(&ctx, false);
            return;
        }
        let plan = enable_plan(&st);
        run_live(&ctx, "Activation du mode jeu Steam", plan, Busy::Mode("Préparation…".into()), move |ctx, ok| {
            if ok {
                set_shortcut(true);
                ctx.app.toast("Mode jeu Steam activé : choisissez « Steam (mode jeu) » à la connexion, ou « Lancer le mode jeu »");
            }
        });
    });
    d.present(Some(&ctx.app.window));
}

fn enable_plan(st: &Status) -> anyhow::Result<Plan> {
    let mut plan = Plan::new();
    if !st.contrib {
        plan = plan.extend(noos_pkg::backends::apt::add_preset_plan("debian-nonfree", None)?);
    }
    if !st.steam {
        plan = plan.root(&["sh", "-c", PRESEED]);
    }
    let missing = missing_mode_packages(st);
    if !missing.is_empty() {
        let mut args = vec!["install"];
        args.extend(&missing);
        plan = plan.root_owned(noos_pkg::backends::apt::apt_root(&args));
    }
    Ok(plan.root(&[model::HELPER, "session", "on"]))
}

fn ask_disable(ctx: &Rc<Ctx>) {
    let Some(st) = ctx.status.borrow().clone() else { return };
    rerender_later(ctx);
    let ctx2 = ctx.clone();
    ctx.app.confirm(
        "Désactiver le mode jeu Steam ?",
        "La session « Steam (mode jeu) », le raccourci et le démarrage direct dans Steam sont retirés. \
         Steam, gamescope, Decky Loader et vos jeux restent installés.",
        "Désactiver",
        true,
        move || {
            if st.demo {
                demo_set(&ctx2, |s| {
                    s.session = false;
                    s.boot = Boot::Login;
                });
                return;
            }
            helper(&ctx2, "Désactivation du mode jeu", &["session", "off"], Busy::Helper, |_, ok| {
                if ok {
                    set_shortcut(false);
                }
            });
        },
    );
}

/// L'interrupteur reste dans son état réel tant que l'opération n'a pas réussi (rendu
/// différé : on est dans son propre signal).
fn rerender_later(ctx: &Rc<Ctx>) {
    let ctx = ctx.clone();
    glib::idle_add_local_once(move || render(&ctx));
}

// ───────────────────────── Exécution ─────────────────────────

/// Plan (apt + assistant) avec le journal en direct dans la bannière.
fn run_live(ctx: &Rc<Ctx>, title: &str, plan: anyhow::Result<Plan>, busy: Busy, done: impl FnOnce(&Rc<Ctx>, bool) + 'static) {
    let plan = match plan {
        Ok(p) => p,
        Err(e) => {
            ctx.app.error(title, &format!("{e:#}"));
            return;
        }
    };
    *ctx.busy.borrow_mut() = busy;
    render(ctx);
    let (tx, rx) = async_channel::unbounded::<String>();
    let task = background(move || -> Result<(), String> {
        let tail = std::sync::Mutex::new(std::collections::VecDeque::<String>::new());
        let res = plan.run(&|l: &str| {
            let _ = tx.send_blocking(l.to_string());
            let mut t = tail.lock().unwrap();
            t.push_back(l.to_string());
            if t.len() > 8 {
                t.pop_front();
            }
        });
        res.map_err(|e| format!("{e:#}\n\n{}", tail.into_inner().unwrap().into_iter().collect::<Vec<_>>().join("\n")))
    });
    {
        let ctx = ctx.clone();
        glib::spawn_future_local(async move {
            while let Ok(line) = rx.recv().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if let Busy::Mode(l) = &mut *ctx.busy.borrow_mut() {
                    *l = line.clone();
                }
                if let Some(lbl) = ctx.log_label.borrow().as_ref() {
                    lbl.set_text(&line);
                }
            }
        });
    }
    let (ctx, title) = (ctx.clone(), title.to_string());
    glib::spawn_future_local(async move {
        let res = task.await;
        *ctx.busy.borrow_mut() = Busy::Idle;
        if let Err(e) = &res {
            ctx.app.error(&title, e);
        }
        done(&ctx, res.is_ok());
        refresh(&ctx);
    });
}

/// `pkexec noos-gaming-helper …` (action polkit dédiée, mot de passe gardé quelques minutes).
fn helper(ctx: &Rc<Ctx>, title: &str, args: &[&str], busy: Busy, done: impl FnOnce(&Rc<Ctx>, bool) + 'static) {
    *ctx.busy.borrow_mut() = busy;
    render(ctx);
    let argv: Vec<String> = std::iter::once(model::HELPER.to_string()).chain(args.iter().map(|s| s.to_string())).collect();
    let task = background(move || {
        std::process::Command::new("pkexec").args(&argv).output().map_err(|e| e.to_string()).and_then(|o| {
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if o.status.success() {
                Ok(out.lines().last().unwrap_or_default().to_string())
            } else {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                // 126 / 127 : authentification refusée ou annulée.
                Err(if err.is_empty() { format!("refusé (code {})", o.status.code().unwrap_or(-1)) } else { format!("{err}\n\n{out}") })
            }
        })
    });
    let (ctx, title) = (ctx.clone(), title.to_string());
    glib::spawn_future_local(async move {
        let res = task.await;
        *ctx.busy.borrow_mut() = Busy::Idle;
        match &res {
            Ok(msg) if !msg.is_empty() => ctx.app.toast(msg),
            Ok(_) => {}
            Err(e) => ctx.app.error(&title, e),
        }
        done(&ctx, res.is_ok());
        refresh(&ctx);
    });
}

/// Raccourci « Mode jeu Steam » : menu Démarrer (~/.local/share/applications) et bureau.
fn set_shortcut(on: bool) {
    if model::demo_mode().is_some() {
        return;
    }
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "noos-settings".into());
    let icon = icon_name(&["steam", "input-gaming"]);
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Mode jeu Steam\nName[en]=Steam Game Mode\n\
         Comment=Fermer le bureau et passer à Steam en plein écran (gamescope)\n\
         Comment[en]=Close the desktop and switch to Steam full screen (gamescope)\n\
         Exec={exe} steam-mode\nIcon={icon}\nCategories=Game;\nKeywords=steam;gamescope;big picture;deck;jeu;console;\n\
         StartupNotify=false\n"
    );
    let mut paths = vec![glib::user_data_dir().join("applications").join(model::SHORTCUT_ID)];
    if let Some(desk) = glib::user_special_dir(glib::UserDirectory::Desktop) {
        if desk.is_dir() && desk != noos_core::paths::home() {
            paths.push(desk.join(model::SHORTCUT_ID));
        }
    }
    for p in paths {
        if on {
            let _ = p.parent().map(std::fs::create_dir_all);
            if std::fs::write(&p, &text).is_ok() {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755));
            }
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn shortcut_present() -> bool {
    glib::user_data_dir().join("applications").join(model::SHORTCUT_ID).exists()
}

// ───────────────────────── Session de jeu ─────────────────────────

fn session_group(ctx: &Rc<Ctx>, st: &Status) -> adap::PreferencesGroup {
    let g = group("Session de jeu");
    let user = glib::user_name().to_string_lossy().to_string();
    let boot = combo(
        "system-reboot-symbolic",
        "Au démarrage du PC",
        &format!("Connexion automatique de {user}, sans mot de passe (sauf « Écran de connexion »)"),
        &["Écran de connexion", "Bureau NOOS", "Mode jeu Steam"].map(String::from),
        Boot::ALL.iter().position(|b| *b == st.boot).unwrap_or(0),
    );
    boot.set_sensitive(*ctx.busy.borrow() == Busy::Idle);
    {
        let ctx = ctx.clone();
        let current = st.boot;
        boot.connect_selected_notify(move |r| {
            let b = Boot::ALL[(r.selected() as usize).min(2)];
            if b == current {
                return;
            }
            if model::demo_mode().is_some() {
                let ctx = ctx.clone();
                glib::idle_add_local_once(move || demo_set(&ctx, |s| s.boot = b));
                return;
            }
            let ctx = ctx.clone();
            glib::idle_add_local_once(move || helper(&ctx, "Démarrage du PC", &["boot", b.arg()], Busy::Helper, |_, _| {}));
        });
    }
    g.add(&boot);
    let sc = switch(
        "user-desktop-symbolic",
        "Raccourci « Mode jeu Steam »",
        "Dans le menu Démarrer et sur le bureau : ferme le bureau et ouvre Steam en plein écran",
        st.demo || shortcut_present(),
    );
    sc.connect_active_notify(|r| set_shortcut(r.is_active()));
    g.add(&sc);
    g.add(&card(
        "go-home-symbolic",
        "Revenir au bureau",
        "Dans Steam : bouton Marche/Arrêt › Passer au bureau. NOOS rouvre votre bureau sans redemander le mot de passe.",
    ));
    g
}

fn primary_output(ctx: &Ctx) -> Option<OutputInfo> {
    let outs = ctx.outputs.borrow();
    let want = ctx.conf.borrow().output.clone();
    outs.iter().find(|o| !want.is_empty() && o.name == want).or_else(|| outs.iter().find(|o| o.primary)).or_else(|| outs.first()).cloned()
}

fn save_conf(ctx: &Rc<Ctx>, f: impl FnOnce(&mut SessionConf)) {
    let mut c = ctx.conf.borrow_mut();
    f(&mut c);
    if model::demo_mode().is_some() {
        return;
    }
    if let Err(e) = c.save() {
        ctx.app.toast(&format!("Enregistrement impossible : {e}"));
    }
}

fn display_group(ctx: &Rc<Ctx>) -> adap::PreferencesGroup {
    let g = group("Affichage du mode jeu");
    g.set_description(Some("Réglages de gamescope, appliqués au prochain lancement du mode jeu. Steam règle aussi la limite d'images et la mise à l'échelle jeu par jeu (menu rapide « … »)."));
    let conf = ctx.conf.borrow().clone();
    let outs = ctx.outputs.borrow().clone();
    let out = primary_output(ctx);

    if outs.len() > 1 {
        let mut items = vec!["Automatique".to_string()];
        items.extend(outs.iter().map(|o| format!("{} — {}", o.name, o.model)));
        let sel = outs.iter().position(|o| o.name == conf.output).map(|i| i + 1).unwrap_or(0);
        let r = combo("video-display-symbolic", "Écran", "Écran utilisé par le mode jeu", &items, sel);
        let (ctx2, names) = (ctx.clone(), outs.iter().map(|o| o.name.clone()).collect::<Vec<_>>());
        r.connect_selected_notify(move |r| {
            let v = r.selected() as usize;
            save_conf(&ctx2, |c| c.output = if v == 0 { String::new() } else { names[v - 1].clone() });
            let ctx3 = ctx2.clone();
            glib::idle_add_local_once(move || render(&ctx3));
        });
        g.add(&r);
    }

    // Définitions et fréquences proposées par l'écran (sinon des valeurs courantes).
    let mut sizes: BTreeSet<(u32, u32)> = out.as_ref().map(|o| o.modes.iter().map(|m| (m.width as u32, m.height as u32)).collect()).unwrap_or_default();
    if sizes.is_empty() {
        sizes = [(1280, 720), (1280, 800), (1920, 1080), (2560, 1440), (3840, 2160)].into_iter().collect();
    }
    if let Some(r) = conf.resolution {
        sizes.insert(r);
    }
    let sizes: Vec<(u32, u32)> = sizes.into_iter().rev().collect();
    let native = out.as_ref().map(|o| format!("Native ({} × {})", o.width, o.height)).unwrap_or_else(|| "Native".into());
    let mut items = vec![native];
    items.extend(sizes.iter().map(|(w, h)| format!("{w} × {h}")));
    let sel = conf.resolution.and_then(|r| sizes.iter().position(|s| *s == r)).map(|i| i + 1).unwrap_or(0);
    let res = combo("view-fullscreen-symbolic", "Définition", "Définition de sortie de gamescope ; les jeux peuvent tourner plus bas, mis à l'échelle", &items, sel);
    {
        let ctx2 = ctx.clone();
        res.connect_selected_notify(move |r| {
            let v = r.selected() as usize;
            save_conf(&ctx2, |c| c.resolution = if v == 0 { None } else { Some(sizes[v - 1]) });
            let ctx3 = ctx2.clone();
            glib::idle_add_local_once(move || render(&ctx3));
        });
    }
    g.add(&res);

    let mut rates: BTreeSet<u32> = out
        .as_ref()
        .map(|o| {
            o.modes
                .iter()
                .filter(|m| conf.resolution.is_none_or(|(w, h)| m.width as u32 == w && m.height as u32 == h))
                .map(|m| ((m.refresh as f64) / 1000.0).round() as u32)
                .filter(|r| *r > 0)
                .collect()
        })
        .unwrap_or_default();
    if rates.is_empty() {
        rates = [60, 120, 144].into_iter().collect();
    }
    if conf.refresh > 0 {
        rates.insert(conf.refresh);
    }
    let rates: Vec<u32> = rates.into_iter().rev().collect();
    let mut items = vec!["Automatique".to_string()];
    items.extend(rates.iter().map(|r| format!("{r} Hz")));
    let sel = rates.iter().position(|r| *r == conf.refresh).map(|i| i + 1).unwrap_or(0);
    let hz = combo("speedometer-symbolic", "Fréquence d'actualisation", "", &items, sel);
    {
        let ctx2 = ctx.clone();
        hz.connect_selected_notify(move |r| {
            let v = r.selected() as usize;
            save_conf(&ctx2, |c| c.refresh = if v == 0 { 0 } else { rates[v - 1] });
        });
    }
    g.add(&hz);

    let vrr_sub = match &out {
        Some(o) if o.vrr_capable => "FreeSync / G-Sync compatible : écran compatible".to_string(),
        Some(o) => crate::vrr::explain(o).detail.map(|d| format!("Indisponible sur cet écran : {d}")).unwrap_or_else(|| "FreeSync / G-Sync compatible, si l'écran le permet".into()),
        None => "FreeSync / G-Sync compatible, si l'écran le permet".into(),
    };
    let hdr_sub = match &out {
        Some(o) if o.hdr_capable => "HDR10 : écran compatible. Activez aussi le HDR dans Steam (Paramètres › Affichage)".to_string(),
        Some(o) if !o.hdr_unavailable.is_empty() => format!("Indisponible : {}", o.hdr_unavailable),
        _ => "HDR10 sur un écran compatible (AMD ou Intel récents)".into(),
    };
    for (icon, title, sub, get, set) in [
        ("weather-clear-symbolic", "Fréquence variable (VRR)", vrr_sub, conf.vrr, (|c: &mut SessionConf, v| c.vrr = v) as fn(&mut SessionConf, bool)),
        ("color-select-symbolic", "HDR", hdr_sub, conf.hdr, |c, v| c.hdr = v),
    ] {
        let r = switch(icon, title, &sub, get);
        let ctx2 = ctx.clone();
        r.connect_active_notify(move |r| {
            let v = r.is_active();
            save_conf(&ctx2, |c| set(c, v));
        });
        g.add(&r);
    }

    let items: Vec<String> = model::FPS_LIMITS.iter().map(|f| if *f == 0 { "Aucune".to_string() } else { format!("{f} images/s") }).collect();
    let sel = model::FPS_LIMITS.iter().position(|f| *f == conf.fps_limit).unwrap_or(0);
    let fps = combo("media-playlist-repeat-symbolic", "Limite d'images", "Moins de chaleur et de bruit ; Steam peut aussi la changer pendant le jeu", &items, sel);
    {
        let ctx2 = ctx.clone();
        fps.connect_selected_notify(move |r| save_conf(&ctx2, |c| c.fps_limit = model::FPS_LIMITS[(r.selected() as usize).min(model::FPS_LIMITS.len() - 1)]));
    }
    g.add(&fps);

    let items: Vec<String> = model::FILTERS.iter().map(|(_, l)| l.to_string()).collect();
    let sel = model::FILTERS.iter().position(|(id, _)| *id == conf.filter).unwrap_or(0);
    let flt = combo("zoom-in-symbolic", "Mise à l'échelle", "Quand le jeu tourne sous la définition de l'écran", &items, sel);
    {
        let ctx2 = ctx.clone();
        flt.connect_selected_notify(move |r| save_conf(&ctx2, |c| c.filter = model::FILTERS[(r.selected() as usize).min(model::FILTERS.len() - 1)].0.to_string()));
    }
    g.add(&flt);

    for (icon, title, sub, get, set) in [
        (
            "utilities-system-monitor-symbolic",
            "Statistiques de performances",
            "Images par seconde, températures, consommation (MangoHud par mangoapp), du niveau 1 à 4 dans le menu rapide",
            conf.mangoapp,
            (|c: &mut SessionConf, v| c.mangoapp = v) as fn(&mut SessionConf, bool),
        ),
        (
            "input-gaming-symbolic",
            "Se présenter comme un Steam Deck",
            "Options propres au Deck dans Steam (-steamdeck) ; à désactiver si un jeu se comporte mal",
            conf.steamdeck,
            |c, v| c.steamdeck = v,
        ),
    ] {
        let r = switch(icon, title, sub, get);
        let ctx2 = ctx.clone();
        r.connect_active_notify(move |r| {
            let v = r.is_active();
            save_conf(&ctx2, |c| set(c, v));
        });
        g.add(&r);
    }
    let cmd = adap::ActionRow::new();
    cmd.set_title("Commande lancée");
    cmd.set_subtitle(&ctx.conf.borrow().command_line());
    cmd.set_subtitle_selectable(true);
    cmd.add_css_class("game-cmd");
    let i = gtk::Image::from_icon_name("utilities-terminal-symbolic");
    i.add_css_class("win11-row-icon");
    cmd.add_prefix(&i);
    g.add(&cmd);
    g
}

// ───────────────────────── Decky Loader ─────────────────────────

fn decky_group(ctx: &Rc<Ctx>, st: &Status) -> adap::PreferencesGroup {
    let g = group("Greffons");
    let busy = *ctx.busy.borrow() == Busy::Decky;
    let row = adap::ActionRow::new();
    row.set_title("Decky Loader");
    row.set_subtitle(&if st.decky.installed {
        format!(
            "{} · {} — greffons dans le menu rapide de Steam (« … »)",
            if st.decky.version.is_empty() { "Installé".to_string() } else { format!("Version {}", st.decky.version) },
            if st.decky.active { "service actif" } else { "service arrêté" }
        )
    } else {
        "Greffons pour l'interface de Steam : thèmes (CSS Loader), performances (PowerTools), jaquettes (SteamGridDB)…".into()
    });
    let i = gtk::Image::from_icon_name(icon_name(&["decky", "application-x-addon-symbolic"]));
    i.set_pixel_size(20);
    i.add_css_class("win11-row-icon");
    row.add_prefix(&i);
    let web = gtk::Button::from_icon_name("web-browser-symbolic");
    web.set_tooltip_text(Some("Site de Decky Loader"));
    web.add_css_class("flat");
    web.set_valign(gtk::Align::Center);
    web.connect_clicked(|_| {
        let _ = gtk::gio::AppInfo::launch_default_for_uri(DECKY_URL, None::<&gtk::gio::AppLaunchContext>);
    });
    row.add_suffix(&web);
    if busy {
        let sp = gtk::Spinner::new();
        sp.start();
        row.add_suffix(&sp);
    } else if st.decky.installed {
        let badge = gtk::Label::new(Some(if st.decky.active { "Actif" } else { "Arrêté" }));
        badge.add_css_class("net-badge");
        badge.add_css_class(if st.decky.active { "net-ok" } else { "net-mid" });
        badge.set_valign(gtk::Align::Center);
        row.add_suffix(&badge);
        let up = gtk::Button::with_label("Mettre à jour");
        up.set_valign(gtk::Align::Center);
        let ctx2 = ctx.clone();
        up.connect_clicked(move |_| ask_decky(&ctx2, true));
        row.add_suffix(&up);
        let rm = gtk::Button::with_label("Désinstaller");
        rm.set_valign(gtk::Align::Center);
        rm.add_css_class("destructive-action");
        let ctx2 = ctx.clone();
        rm.connect_clicked(move |_| ask_decky(&ctx2, false));
        row.add_suffix(&rm);
    } else {
        let b = gtk::Button::with_label("Installer");
        b.set_valign(gtk::Align::Center);
        b.add_css_class("suggested-action");
        b.set_sensitive(*ctx.busy.borrow() == Busy::Idle);
        let ctx2 = ctx.clone();
        b.connect_clicked(move |_| ask_decky(&ctx2, true));
        row.add_suffix(&b);
    }
    g.add(&row);
    g
}

fn ask_decky(ctx: &Rc<Ctx>, install: bool) {
    let Some(st) = ctx.status.borrow().clone() else { return };
    let home = model::decky_home();
    let (title, body, action) = if install {
        (
            if st.decky.installed { "Mettre à jour Decky Loader ?" } else { "Installer Decky Loader ?" },
            format!(
                "NOOS fait ce que fait le script officiel, sans exécuter de script téléchargé :\n\n{}\n\n\
                 Decky et ses greffons s'exécutent en administrateur (root) : n'installez que des greffons \
                 de son magasin officiel.",
                bullets(&[
                    "Chercher la dernière version publiée sur GitHub (SteamDeckHomebrew/decky-loader)".into(),
                    "Télécharger PluginLoader et vérifier son empreinte SHA-256 publiée par GitHub".into(),
                    format!("L'installer dans {}/services", home.display()),
                    "Autoriser le débogage distant de l'interface de Steam (.cef-enable-remote-debugging)".into(),
                    "Créer et démarrer le service plugin_loader".into(),
                ])
            ),
            if st.decky.installed { "Mettre à jour" } else { "Installer" },
        )
    } else {
        (
            "Désinstaller Decky Loader ?",
            format!(
                "Le service plugin_loader et {}/services sont supprimés. Vos greffons et leurs réglages restent dans {} \
                 (supprimez ce dossier pour tout effacer).",
                home.display(),
                home.display()
            ),
            "Désinstaller",
        )
    };
    let ctx2 = ctx.clone();
    ctx.app.confirm(title, &body, action, !install, move || {
        if st.demo {
            let ctx = ctx2.clone();
            *ctx.busy.borrow_mut() = Busy::Decky;
            render(&ctx);
            glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                *ctx.busy.borrow_mut() = Busy::Idle;
                demo_set(&ctx, |s| s.decky = if install { model::Decky { installed: true, active: true, version: "v3.3.1".into() } } else { Default::default() });
            });
            return;
        }
        helper(&ctx2, "Decky Loader", &["decky", if install { "install" } else { "remove" }], Busy::Decky, |_, _| {});
    });
}

// ───────────────────────── Composants ─────────────────────────

fn components_group(ctx: &Rc<Ctx>, st: &Status) -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let t = gtk::Label::new(Some("Composants pour le jeu"));
    t.add_css_class("heading");
    t.add_css_class("game-section-title");
    t.set_xalign(0.0);
    v.append(&t);
    let d = gtk::Label::new(Some(
        "Outils recommandés pour Steam, Proton et les autres boutiques. Paquets Debian (versions 32 bits comprises) ou applications Flatpak de Flathub.",
    ));
    d.add_css_class("dim-label");
    d.set_xalign(0.0);
    d.set_wrap(true);
    v.append(&d);
    // Deux colonnes de tuiles de même largeur (comme les appareils Bluetooth).
    let grid = gtk::Grid::new();
    grid.set_column_homogeneous(true);
    grid.set_column_spacing(8);
    grid.set_row_spacing(8);
    for (i, c) in COMPONENTS.iter().enumerate() {
        let t = tile(ctx, st, c);
        t.set_hexpand(true);
        grid.attach(&t, (i % 2) as i32, (i / 2) as i32, 1, 1);
    }
    v.append(&grid);
    v
}

fn tile(ctx: &Rc<Ctx>, st: &Status, c: &'static Component) -> gtk::Box {
    let installed = st.installed.contains(c.id);
    let busy = ctx.busy.borrow().clone();
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    b.add_css_class("game-tile");
    if installed {
        b.add_css_class("installed");
    }
    let i = gtk::Image::from_icon_name(icon_name(c.icons));
    i.set_pixel_size(40);
    i.set_valign(gtk::Align::Start);
    i.add_css_class("game-tile-icon");
    b.append(&i);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 3);
    v.set_hexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let n = gtk::Label::new(Some(c.name));
    n.add_css_class("heading");
    n.set_xalign(0.0);
    head.append(&n);
    let kind = gtk::Label::new(Some(match c.source {
        Source::Apt(_) if c.contrib => "apt · contrib",
        Source::Apt(_) => "apt",
        Source::Flatpak(_) => "Flatpak",
    }));
    kind.add_css_class("game-badge");
    kind.set_valign(gtk::Align::Center);
    head.append(&kind);
    v.append(&head);
    let s = gtk::Label::new(Some(c.summary));
    s.add_css_class("caption");
    s.set_xalign(0.0);
    s.set_wrap(true);
    // Largeur naturelle modeste : deux tuiles par ligne dès que la page le permet.
    s.set_max_width_chars(30);
    v.append(&s);
    if installed && !c.usage.is_empty() {
        let u = gtk::Label::new(Some(c.usage));
        u.add_css_class("game-usage");
        u.set_xalign(0.0);
        u.set_selectable(true);
        u.set_ellipsize(gtk::pango::EllipsizeMode::End);
        u.set_max_width_chars(30);
        v.append(&u);
    }
    b.append(&v);
    let right = gtk::Box::new(gtk::Orientation::Vertical, 4);
    right.set_valign(gtk::Align::Center);
    if busy == Busy::Component(c.id) {
        let sp = gtk::Spinner::new();
        sp.start();
        sp.set_size_request(24, 24);
        right.append(&sp);
        let l = gtk::Label::new(Some(if installed { "Suppression…" } else { "Installation…" }));
        l.add_css_class("caption");
        right.append(&l);
    } else {
        if installed {
            let ok = gtk::Label::new(Some("✓ Installé"));
            ok.add_css_class("game-installed");
            right.append(&ok);
        }
        let btn = gtk::Button::with_label(if installed { "Supprimer" } else { "Installer" });
        if !installed {
            btn.add_css_class("suggested-action");
        } else {
            btn.add_css_class("flat");
        }
        btn.set_sensitive(busy == Busy::Idle);
        let ctx2 = ctx.clone();
        btn.connect_clicked(move |_| component_action(&ctx2, c, !installed));
        right.append(&btn);
    }
    b.append(&right);
    b
}

/// Installation Flatpak : système si Flathub y est (cas de NOOS), sinon pour l'utilisateur.
fn flatpak_scope() -> noos_pkg::Scope {
    let system = noos_pkg::exec::capture(&["flatpak", "remotes", "--system", "--columns=name"]).unwrap_or_default();
    if system.lines().any(|l| l.trim() == "flathub") { noos_pkg::Scope::System } else { noos_pkg::Scope::User }
}

fn component_plan(st: &Status, c: &Component, install: bool) -> anyhow::Result<Plan> {
    let mut plan = Plan::new();
    match c.source {
        Source::Apt(pkgs) => {
            let pkgs: Vec<&str> = pkgs.iter().copied().filter(|p| st.i386 || !p.ends_with(":i386")).collect();
            if install {
                if c.contrib && !st.contrib {
                    plan = plan.extend(noos_pkg::backends::apt::add_preset_plan("debian-nonfree", None)?);
                }
                if c.id == "steam" {
                    plan = plan.root(&["sh", "-c", PRESEED]);
                }
            }
            let mut args = vec![if install { "install" } else { "remove" }];
            args.extend(pkgs);
            plan = plan.root_owned(noos_pkg::backends::apt::apt_root(&args));
        }
        Source::Flatpak(id) => {
            let fp = noos_pkg::backends::flatpak::Flatpak::new(noos_pkg::SystemInfo::detect());
            let scope = if install {
                flatpak_scope()
            } else if noos_pkg::exec::capture(&["flatpak", "info", "--user", id]).is_ok() {
                noos_pkg::Scope::User
            } else {
                noos_pkg::Scope::System
            };
            if install && scope == noos_pkg::Scope::User {
                plan = plan.user(&["flatpak", "remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"]);
            }
            let pkg = noos_pkg::Package {
                backend: Some(noos_pkg::BackendKind::Flatpak),
                id: id.into(),
                name: c.name.into(),
                origin: if install { "flathub".into() } else { String::new() },
                scope: Some(scope),
                ..Default::default()
            };
            plan = plan.extend(if install { fp.install(&[pkg]) } else { fp.remove(&[pkg]) });
        }
    }
    Ok(plan)
}

fn component_action(ctx: &Rc<Ctx>, c: &'static Component, install: bool) {
    let Some(st) = ctx.status.borrow().clone() else { return };
    let go = {
        let ctx = ctx.clone();
        let st = st.clone();
        move || {
            if st.demo {
                *ctx.busy.borrow_mut() = Busy::Component(c.id);
                render(&ctx);
                let ctx = ctx.clone();
                glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                    *ctx.busy.borrow_mut() = Busy::Idle;
                    demo_set(&ctx, |s| {
                        if install {
                            s.installed.insert(c.id.into());
                        } else {
                            s.installed.remove(c.id);
                        }
                    });
                });
                return;
            }
            let title = format!("{} de {}", if install { "Installation" } else { "Suppression" }, c.name);
            let plan = component_plan(&st, c, install);
            *ctx.busy.borrow_mut() = Busy::Component(c.id);
            render(&ctx);
            let ctx2 = ctx.clone();
            ctx.app.run_plan(&title, plan, move |_| {
                *ctx2.busy.borrow_mut() = Busy::Idle;
                refresh(&ctx2);
            });
        }
    };
    if install && c.contrib && !st.contrib {
        ctx.app.confirm(
            &format!("Installer {} ?", c.name),
            &format!("{} est dans les composants contrib de Debian : NOOS active contrib et non-free dans les sources APT, puis installe le paquet.", c.name),
            "Activer et installer",
            false,
            go,
        );
    } else if install && c.id == "steam" {
        ctx.app.confirm(
            "Installer Steam ?",
            &format!("En installant Steam, vous acceptez le contrat d'abonné Steam de Valve ({STEAM_AGREEMENT})."),
            "Accepter et installer",
            false,
            go,
        );
    } else if !install {
        ctx.app.confirm(&format!("Supprimer {} ?", c.name), "Vos jeux et vos réglages ne sont pas supprimés.", "Supprimer", true, go);
    } else {
        go();
    }
}

// ───────────────────────── Démo ─────────────────────────

fn demo_set(ctx: &Rc<Ctx>, f: impl FnOnce(&mut Status)) {
    if let Some(s) = ctx.status.borrow_mut().as_mut() {
        f(s);
    }
    render(ctx);
}

/// Activation simulée (journal d'apt fictif). `hold` : reste en cours (capture).
fn demo_install(ctx: &Rc<Ctx>, hold: bool) {
    const LINES: &[&str] = &[
        "→ (admin) cat > /etc/apt/sources.list.d/debian.sources (écriture du fichier)",
        "Réception de : 1 http://deb.debian.org/debian sid/contrib amd64 Packages [61,2 kB]",
        "Réception de : 14 http://deb.debian.org/debian sid/contrib amd64 gamescope 3.16.24+ds-2 [1 412 kB]",
        "Réception de : 15 http://deb.debian.org/debian sid/contrib amd64 steam-installer 1:1.0.0.87~ds-3 [1 220 kB]",
        "Paramétrage de gamescope (3.16.24+ds-2) …",
        "→ (admin) /usr/libexec/noos-gaming-helper session on",
    ];
    if hold {
        *ctx.busy.borrow_mut() = Busy::Mode(LINES[2].into());
        if let Some(s) = ctx.status.borrow_mut().as_mut() {
            s.contrib = true;
        }
        render(ctx);
        return;
    }
    *ctx.busy.borrow_mut() = Busy::Mode(LINES[0].into());
    render(ctx);
    for (i, l) in LINES.iter().enumerate().skip(1) {
        let ctx = ctx.clone();
        glib::timeout_add_local_once(Duration::from_millis(600 * i as u64), move || {
            *ctx.busy.borrow_mut() = Busy::Mode(l.to_string());
            if let Some(lbl) = ctx.log_label.borrow().as_ref() {
                lbl.set_text(l);
            }
        });
    }
    let ctx = ctx.clone();
    glib::timeout_add_local_once(Duration::from_millis(600 * LINES.len() as u64), move || {
        *ctx.busy.borrow_mut() = Busy::Idle;
        demo_set(&ctx, |s| {
            s.session = true;
            s.gamescope = true;
            s.steam = true;
            s.mangoapp = true;
            s.contrib = true;
            s.installed.insert("steam".into());
            s.installed.insert("gamescope".into());
        });
        ctx.app.toast("Démo : mode jeu Steam activé (rien n'a été installé)");
    });
}

// ───────────────────────── Bureau : affichage et mode Jeu ─────────────────────────

/// Réglages du bureau NOOS pour le jeu (compositeur) : VRR, scanout direct, 10 bits, HDR,
/// et le mode Jeu (plein écran sans interruption).
fn display_groups(app: &Rc<App>, content: &gtk::Box) {
    let cfg = app.cfg();
    let gm = &cfg.gaming;

    let g = group("Jeux sur le bureau NOOS");
    let vrr = combo(
        "speedometer-symbolic",
        "Fréquence d'actualisation variable (VRR)",
        "« En plein écran » l'active seulement pour les jeux et vidéos",
        &["Désactivée", "En plein écran", "Toujours"].map(String::from),
        match gm.vrr {
            VrrPolicy::Off => 0,
            VrrPolicy::Fullscreen => 1,
            VrrPolicy::Always => 2,
        },
    );
    {
        let app = app.clone();
        vrr.connect_selected_notify(move |r| {
            let v = match r.selected() {
                0 => VrrPolicy::Off,
                2 => VrrPolicy::Always,
                _ => VrrPolicy::Fullscreen,
            };
            app.update(|c| c.gaming.vrr = v);
        });
    }
    g.add(&vrr);
    // Écrans sans VRR : on l'explique sous le réglage (carte graphique, menu de l'écran…).
    {
        let (g, vrr) = (g.clone(), vrr.clone());
        let task = crate::ui::ipc(noos_core::ipc::Request::GetOutputs);
        glib::spawn_future_local(async move {
            let Ok(noos_core::ipc::Event::Outputs { outputs }) = task.await else { return };
            let several = outputs.len() > 1;
            let mut after: gtk::Widget = vrr.upcast();
            for o in &outputs {
                let Some(detail) = crate::vrr::explain(o).detail else { continue };
                let title = if several {
                    format!("VRR indisponible sur {} ({})", o.name, o.model)
                } else {
                    "Pourquoi le VRR est-il indisponible ?".into()
                };
                let row = card("dialog-information-symbolic", &title, &detail);
                // Juste sous la ligne VRR, avant les autres réglages du groupe.
                if let Some(list) = after.parent() {
                    if let Some(lb) = list.downcast_ref::<gtk::ListBox>() {
                        let idx = after.downcast_ref::<gtk::ListBoxRow>().map(|r| r.index() + 1).unwrap_or(-1);
                        lb.insert(&row, idx);
                        after = row.upcast();
                        continue;
                    }
                }
                g.add(&row);
            }
        });
    }
    for (icon, title, sub, get, set) in [
        (
            "media-playback-start-symbolic",
            "Scanout direct",
            "La fenêtre plein écran va directement à l'écran, sans composition (latence minimale)",
            gm.direct_scanout,
            (|c: &mut noos_core::NoosConfig, v| c.gaming.direct_scanout = v) as fn(&mut noos_core::NoosConfig, bool),
        ),
        (
            "color-select-symbolic",
            "Framebuffer 10 bits",
            "Moins de banding, requis pour le HDR (au prochain démarrage)",
            gm.ten_bit,
            |c, v| c.gaming.ten_bit = v,
        ),
        (
            "weather-clear-symbolic",
            "HDR par défaut",
            "HDR10 sur les écrans compatibles qui n'ont pas leur propre réglage (Système › Écran)",
            gm.hdr,
            |c, v| c.gaming.hdr = v,
        ),
    ] {
        let row = switch(icon, title, sub, get);
        let app = app.clone();
        row.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| set(c, v));
        });
        g.add(&row);
    }
    let game = switch(
        "input-gaming-symbolic",
        "Mode Jeu du bureau",
        "En plein écran : pas de menu Démarrer par la touche Super, pas de barre ni d'effets par-dessus",
        gm.game_mode,
    );
    {
        let app = app.clone();
        game.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.gaming.game_mode = v);
        });
    }
    g.add(&game);
    g.add(&card(
        "dialog-information-symbolic",
        "Steam, Proton et jeux X11",
        "Pris en charge par Xwayland, géré directement par le compositeur. Les jeux peuvent verrouiller la souris (pointer-constraints).",
    ));
    content.append(&g);
}

pub const CSS: &str = r#"
/* ── Jeux : mode jeu Steam ── */
.game-hero { border-radius: 12px; padding: 26px 28px;
  background: linear-gradient(120deg, #0f1b33 0%, #1b2a52 45%, #2b1f4f 100%);
  color: white; border: 1px solid alpha(white, 0.08); }
.game-hero.on { background: linear-gradient(120deg, #0b2a3d 0%, #11425e 45%, #1d6b72 100%); }
.game-hero label { color: white; }
.game-hero-logo { border-radius: 22px; padding: 10px; background: alpha(white, 0.08); }
.game-kicker { font-size: 0.75em; font-weight: 800; letter-spacing: 2px; color: alpha(white, 0.65); }
.game-hero.on .game-kicker { color: #66e0b4; }
.game-hero-title { font-size: 26px; font-weight: 700; }
.game-hero-sub { color: alpha(white, 0.78); }
.game-chip { border-radius: 999px; padding: 3px 10px; font-size: 0.82em; font-weight: 600; }
.game-chip.ok { background: alpha(#66e0b4, 0.18); color: #8ef0c9; }
.game-chip.ok label, .game-chip.ok image { color: #8ef0c9; }
.game-chip.missing { background: alpha(white, 0.08); }
.game-chip.missing label, .game-chip.missing image { color: alpha(white, 0.6); }
switch.game-switch { transform: scale(1.35); margin: 0 10px; }
.game-switch-label { font-weight: 700; font-size: 1.05em; }
.game-feature { background: alpha(white, 0.06); border-radius: 10px; padding: 12px 14px; }
.game-feature label.caption { color: alpha(white, 0.7); }
.game-notice { background: alpha(white, 0.07); border-radius: 8px; padding: 10px 14px; }
.game-log { font-family: monospace; font-size: 0.85em; background: alpha(black, 0.3); border-radius: 6px; padding: 8px 12px; color: alpha(white, 0.85); }
.game-hero button.pill { padding: 8px 22px; font-weight: 700; }
.game-section-title { font-size: 14px; font-weight: 600; margin-top: 6px; }
.game-tile { background-color: @card_bg_color; border-radius: 8px; border: 1px solid alpha(@card_fg_color, 0.06); padding: 14px 14px 14px 16px; }
.game-tile:hover { background-color: mix(@card_bg_color, @window_fg_color, 0.03); }
.game-tile.installed { border-color: alpha(#3ccf72, 0.30); }
.game-tile-icon { margin-top: 2px; }
.game-badge { font-size: 0.72em; font-weight: 700; border-radius: 999px; padding: 1px 8px; background: alpha(@window_fg_color, 0.08); color: alpha(@window_fg_color, 0.7); }
.game-usage { font-family: monospace; font-size: 0.78em; color: alpha(@window_fg_color, 0.6); margin-top: 2px; }
.game-installed { color: #3ccf72; font-weight: 700; font-size: 0.85em; }
row.game-cmd .subtitle { font-family: monospace; font-size: 0.85em; }
"#;

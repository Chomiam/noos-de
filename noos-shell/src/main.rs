//! noos-shell — barre des tâches, menu Démarrer, réglages rapides et fond d'écran de NOOS.
//!
//! Le shell est un client Wayland « layer-shell » du compositeur : il ne gère aucune fenêtre
//! lui-même. Il reçoit la liste des fenêtres par l'IPC NOOS et demande au compositeur de les
//! activer / réduire / fermer.

mod apps;
mod audio;
mod drives;
mod net;
mod notify;
mod polkit;
mod quick;
mod reorder;
mod startmenu;
mod style;
mod taskbar;
mod tray;
mod updates;
mod wallpaper;

use std::{cell::RefCell, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::{
    NoosConfig,
    ipc::{Event, IpcClient, Request, WindowInfo},
};
use noos_theme::Theming;

/// Icônes d'état de la barre (réseau, volume).
#[derive(Clone, Debug, PartialEq)]
pub struct StatusIcons {
    pub net_icon: String,
    pub net_tooltip: String,
    pub vol_icon: String,
    pub vol_tooltip: String,
    /// Profils VPN (bouton affiché dans la barre quand l'un d'eux est actif).
    pub vpns: Vec<net::Vpn>,
}

impl Default for StatusIcons {
    fn default() -> Self {
        Self {
            net_icon: "network-wired-symbolic".into(),
            net_tooltip: "Réseau".into(),
            vol_icon: "audio-volume-high-symbolic".into(),
            vol_tooltip: "Volume".into(),
            vpns: vec![],
        }
    }
}

fn read_status_icons() -> StatusIcons {
    let net = net::read_status();
    let (vol_icon, vol_tooltip) = match audio::default_volume() {
        Some((v, mute)) => (
            audio::volume_icon(v, mute).to_string(),
            if mute { "Son coupé".to_string() } else { format!("Volume : {} %", (v * 100.0).round()) },
        ),
        None => ("audio-volume-muted-symbolic".into(), "Aucune sortie audio".into()),
    };
    StatusIcons { net_icon: net::status_icon(&net).into(), net_tooltip: net::status_tooltip(&net), vol_icon, vol_tooltip, vpns: net::read_vpns() }
}

pub struct Shell {
    pub app: adap::Application,
    pub theming: Rc<Theming>,
    pub windows: RefCell<Vec<WindowInfo>>,
    taskbars: RefCell<Vec<taskbar::Taskbar>>,
    wallpapers: RefCell<Vec<wallpaper::Wallpaper>>,
    pub start: RefCell<Option<Rc<startmenu::StartMenu>>>,
    pub quick: RefCell<Option<Rc<quick::QuickSettings>>>,
    pub notify: RefCell<Option<Rc<notify::Notifications>>>,
    /// Agent d'authentification polkit (fenêtre d'élévation de privilèges).
    pub polkit: RefCell<Option<Rc<polkit::PolkitAgent>>>,
    pub tray: RefCell<Option<Rc<tray::Tray>>>,
    pub net: RefCell<Option<Rc<net::NetFlyout>>>,
    pub audio: RefCell<Option<Rc<audio::AudioFlyout>>>,
    /// Périphériques amovibles : éjection, lecture automatique.
    pub drives: RefCell<Option<Rc<drives::Drives>>>,
    /// Icône « mises à jour disponibles ».
    pub updates: RefCell<Option<Rc<updates::UpdateIcon>>>,
    status: RefCell<StatusIcons>,
    pub slideshow: wallpaper::Slideshow,
    css: gtk::CssProvider,
}

impl Shell {
    pub fn config(&self) -> NoosConfig {
        self.theming.config()
    }

    /// Modifie la configuration, la sauvegarde et prévient le compositeur.
    pub fn update_config(&self, f: impl FnOnce(&mut NoosConfig)) {
        let mut cfg = self.config();
        f(&mut cfg);
        if let Err(e) = cfg.save() {
            eprintln!("noos-shell : sauvegarde impossible : {e:#}");
            return;
        }
        let _ = noos_theme::install_theme(&cfg.appearance);
        self.theming.reload();
        apps::request(Request::ReloadConfig);
    }

    fn rebuild_outputs(self: &Rc<Self>) {
        let display = gdk::Display::default().unwrap();
        let monitors = display.monitors();
        for t in self.taskbars.borrow_mut().drain(..) {
            t.window.destroy();
        }
        for w in self.wallpapers.borrow_mut().drain(..) {
            w.window.destroy();
        }
        for i in 0..monitors.n_items() {
            let Some(monitor) = monitors.item(i).and_downcast::<gdk::Monitor>() else { continue };
            self.wallpapers.borrow_mut().push(wallpaper::Wallpaper::new(self, &monitor));
            self.taskbars.borrow_mut().push(taskbar::Taskbar::new(self, &monitor));
        }
        self.refresh_tasks();
    }

    /// Relit l'état réseau / volume (dans un thread) et met à jour les icônes.
    pub fn refresh_status_soon(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(read_status_icons());
        });
        let shell = self.clone();
        glib::spawn_future_local(async move {
            if let Ok(st) = rx.recv().await {
                if *shell.status.borrow() != st {
                    *shell.status.borrow_mut() = st;
                    shell.apply_status_icons();
                }
            }
        });
    }

    fn apply_status_icons(&self) {
        let st = self.status.borrow().clone();
        for t in self.taskbars.borrow().iter() {
            t.update_status(&st);
        }
    }

    /// Zone de notification et cloche de toutes les barres.
    pub fn refresh_indicators(&self) {
        let cfg = self.config();
        let tray = self.tray.borrow().clone();
        let notifs = self.notify.borrow().as_ref().map(|n| n.state()).unwrap_or((0, false));
        for t in self.taskbars.borrow().iter() {
            t.update_indicators(tray.as_ref(), &cfg, notifs);
        }
    }

    pub fn refresh_tasks(&self) {
        let windows = self.windows.borrow().clone();
        let cfg = self.config();
        for t in self.taskbars.borrow().iter() {
            t.update(&windows, &cfg);
        }
    }

    /// Affiche une image du diaporama sur tous les écrans.
    pub fn show_wallpaper(&self, path: &str, transition: gtk::StackTransitionType) {
        for w in self.wallpapers.borrow().iter() {
            w.show(std::path::Path::new(path), transition);
        }
    }

    fn apply_style(self: &Rc<Self>, cfg: &NoosConfig) {
        let dark = adap::StyleManager::default().is_dark();
        self.css.load_from_string(&format!("{}{}", style::css(cfg, dark), polkit::CSS));
        for t in self.taskbars.borrow().iter() {
            t.apply_config(cfg);
        }
        for w in self.wallpapers.borrow().iter() {
            w.apply_config(cfg);
        }
        self.slideshow.sync(self, cfg);
        self.refresh_indicators();
    }

    fn handle_event(self: &Rc<Self>, event: Event) {
        match event {
            Event::Windows { windows } => {
                *self.windows.borrow_mut() = windows;
                self.refresh_tasks();
            }
            Event::ToggleStartMenu => {
                if let Some(q) = self.quick.borrow().as_ref() {
                    q.hide();
                }
                if let Some(s) = self.start.borrow().as_ref() {
                    s.toggle(None);
                }
            }
            Event::Outputs { .. } | Event::ConfigReloaded => {
                if let Some(u) = self.updates.borrow().as_ref() {
                    u.refresh();
                }
                if let Some(q) = self.quick.borrow().as_ref() {
                    q.refresh();
                }
            }
            _ => {}
        }
    }

    pub fn close_flyouts(&self) {
        if let Some(s) = self.start.borrow().as_ref() {
            s.hide();
        }
        if let Some(q) = self.quick.borrow().as_ref() {
            q.hide();
        }
        if let Some(n) = self.notify.borrow().as_ref() {
            n.hide();
        }
        if let Some(n) = self.net.borrow().as_ref() {
            n.hide();
        }
        if let Some(a) = self.audio.borrow().as_ref() {
            a.hide();
        }
    }
}

/// Ferme un panneau (menu, volume, réseau…) quand il perd le focus, sauf si le pointeur
/// est dessus : un clic à l'intérieur ne doit jamais le fermer, même si le compositeur
/// donne brièvement le clavier à une fenêtre située derrière.
pub fn close_on_focus_loss(window: &gtk::Window, hide: impl Fn() + 'static) {
    let hide = Rc::new(hide);
    let inside = Rc::new(std::cell::Cell::new(false));
    // Focus perdu pendant que le pointeur était dessus (ancien compositeur qui donnait le
    // clavier à la fenêtre derrière) : on fermera quand le pointeur sortira du panneau.
    let lost_inside = Rc::new(std::cell::Cell::new(false));
    let motion = gtk::EventControllerMotion::new();
    {
        let inside = inside.clone();
        motion.connect_enter(move |_, _, _| inside.set(true));
    }
    {
        let (inside, lost_inside, hide, win) = (inside.clone(), lost_inside.clone(), hide.clone(), window.clone());
        motion.connect_leave(move |_| {
            inside.set(false);
            if lost_inside.take() && !win.is_active() && win.is_visible() {
                hide();
            }
        });
    }
    window.add_controller(motion);
    window.connect_is_active_notify(move |w| {
        if w.is_active() {
            lost_inside.set(false);
        } else if w.is_visible() {
            if inside.get() {
                lost_inside.set(true);
            } else {
                hide();
            }
        }
    });
}

/// Le shell devient-il le serveur de notifications / de zone de notification du bus de
/// session ? Pas dans l'aperçu imbriqué (ce bus est celui de la session hôte, GNOME…),
/// sauf avec un bus privé de test (`NOOS_PRIVATE_BUS=1`).
pub fn own_session_names() -> bool {
    std::env::var_os("NOOS_NESTED").is_none() || std::env::var_os("NOOS_PRIVATE_BUS").is_some()
}

/// Connexion permanente au compositeur (thread dédié), reconnexion automatique.
fn spawn_ipc(sender: async_channel::Sender<Event>) {
    std::thread::spawn(move || {
        loop {
            match IpcClient::connect() {
                Ok(mut client) => {
                    if client.send(&Request::Subscribe).is_ok() {
                        while let Ok(Some(event)) = client.recv() {
                            if sender.send_blocking(event).is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id("org.noos.Shell")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(|app| {
        // Le thème libAdapta « NOOS » doit exister avant que les applis ne démarrent.
        let mut cfg = NoosConfig::load();
        // Fonds déjà téléchargés par une version précédente → ~/Images/Wallpapers.
        if let Some(moved) = noos_core::paths::migrate_old_wallpapers(&cfg.appearance.wallpaper) {
            cfg.appearance.wallpaper = moved;
            let _ = cfg.save();
        }
        if let Err(e) = noos_theme::install_theme(&cfg.appearance) {
            eprintln!("noos-shell : installation du thème : {e:#}");
        }

        let theming = Theming::init();
        // Clic sur un curseur (volume…) : il saute directement à la position cliquée, puis suit
        // la souris tant que le bouton reste enfoncé.
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_primary_button_warps_slider(true);
        }
        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 10,
        );

        let shell = Rc::new(Shell {
            app: app.clone(),
            theming: theming.clone(),
            windows: RefCell::new(Vec::new()),
            taskbars: RefCell::new(Vec::new()),
            wallpapers: RefCell::new(Vec::new()),
            start: RefCell::new(None),
            quick: RefCell::new(None),
            notify: RefCell::new(None),
            polkit: RefCell::new(None),
            tray: RefCell::new(None),
            net: RefCell::new(None),
            audio: RefCell::new(None),
            drives: RefCell::new(None),
            updates: RefCell::new(None),
            status: RefCell::new(StatusIcons::default()),
            slideshow: wallpaper::Slideshow::default(),
            css,
        });
        *shell.start.borrow_mut() = Some(startmenu::StartMenu::new(&shell));
        *shell.quick.borrow_mut() = Some(quick::QuickSettings::new(&shell));
        *shell.notify.borrow_mut() = Some(notify::Notifications::new(&shell));
        *shell.polkit.borrow_mut() = Some(polkit::PolkitAgent::new(&shell));
        *shell.tray.borrow_mut() = Some(tray::Tray::new(&shell));
        *shell.net.borrow_mut() = Some(net::NetFlyout::new(&shell));
        *shell.audio.borrow_mut() = Some(audio::AudioFlyout::new(&shell));
        *shell.drives.borrow_mut() = Some(drives::Drives::new(&shell));
        *shell.updates.borrow_mut() = Some(updates::UpdateIcon::new(&shell));
        {
            // Icônes réseau / volume : toutes les 5 s (et juste après une action).
            shell.refresh_status_soon();
            let shell = shell.clone();
            glib::timeout_add_local(net::POLL, move || {
                shell.refresh_status_soon();
                glib::ControlFlow::Continue
            });
        }
        shell.rebuild_outputs();
        shell.apply_style(&cfg);

        {
            let shell = shell.clone();
            theming.connect_changed(move |cfg| shell.apply_style(cfg));
        }
        {
            let shell = shell.clone();
            adap::StyleManager::default().connect_dark_notify(move |_| shell.apply_style(&shell.config()));
        }
        {
            // Écran branché / débranché.
            let shell = shell.clone();
            gdk::Display::default().unwrap().monitors().connect_items_changed(move |_, _, _, _| {
                let shell = shell.clone();
                glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    shell.rebuild_outputs();
                    shell.apply_style(&shell.config());
                });
            });
        }

        let (tx, rx) = async_channel::unbounded();
        spawn_ipc(tx);
        let shell_ev = shell.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                shell_ev.handle_event(event);
            }
        });

        // Fond d'écran Bing du jour (si activé) : au démarrage puis toutes les 30 minutes.
        {
            let shell = shell.clone();
            let check = move || {
                let cfg = shell.config();
                if !cfg.appearance.bing_daily {
                    return;
                }
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(noos_core::bing::today().map(|(p, _)| p.to_string_lossy().to_string()));
                });
                let shell = shell.clone();
                glib::spawn_future_local(async move {
                    if let Ok(Ok(path)) = rx.recv().await {
                        if shell.config().appearance.wallpaper != path {
                            shell.update_config(|c| c.appearance.wallpaper = path);
                        }
                    }
                });
            };
            check();
            glib::timeout_add_seconds_local(30 * 60, move || {
                check();
                glib::ControlFlow::Continue
            });
        }

        // Test : ouvrir un panneau au démarrage (NOOS_SHELL_OPEN=audio|net|quick|notifications).
        if let Ok(which) = std::env::var("NOOS_SHELL_OPEN") {
            let shell = shell.clone();
            glib::timeout_add_local_once(Duration::from_millis(1500), move || match which.as_str() {
                "audio" => shell.audio.borrow().as_ref().map(|a| a.toggle()).unwrap_or(()),
                "net" => shell.net.borrow().as_ref().map(|n| n.toggle()).unwrap_or(()),
                "quick" => shell.quick.borrow().as_ref().map(|q| q.toggle()).unwrap_or(()),
                "notifications" => shell.notify.borrow().as_ref().map(|n| n.toggle()).unwrap_or(()),
                _ => {}
            });
        }

        // Paramètres préchargé en arrière-plan (ouverture quasi instantanée ensuite),
        // une fois le bureau affiché pour ne pas ralentir l'ouverture de session.
        if std::env::var_os("NOOS_NESTED").is_none() {
            glib::timeout_add_local_once(Duration::from_secs(5), || apps::spawn("noos-settings --preload"));
        }

        // Garde l'application en vie même sans fenêtre « normale ».
        std::mem::forget(app.hold());
    });

    app.run_with_args::<&str>(&[])
}

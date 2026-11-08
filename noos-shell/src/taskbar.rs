//! Barre des tâches façon Windows 11 : icônes centrées (bouton Démarrer, recherche, applis
//! épinglées et ouvertes), zone de notification et horloge à droite.

use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

use gtk::{gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::{
    NoosConfig,
    config::TaskbarAlignment,
    ipc::{Request, WindowInfo},
    paths,
};

use crate::{Shell, apps};

pub struct Taskbar {
    pub window: gtk::Window,
    shell: Weak<Shell>,
    /// Écran de cette barre (nom du connecteur).
    output: String,
    root: gtk::CenterBox,
    /// Zones de la barre (gauche, centre = `main`, droite = `end`) et éléments encadrés.
    left: gtk::Box,
    end: gtk::Box,
    frames: Vec<(String, gtk::Overlay, gtk::Box)>,
    /// Barre fine supplémentaire (sinon : le dock).
    top: bool,
    desk_box: gtk::Box,
    status_group: gtk::Box,
    tray_box2: gtk::Box,
    search_label: gtk::Label,
    clock_date: gtk::Label,
    vitals: Rc<noos_widget_vitals::Vitals>,
    main: gtk::Box,
    tasks: gtk::Box,
    /// Un bouton de souris est enfoncé sur la barre : la reconstruction des icônes attend
    /// le relâchement (sinon le bouton cliqué est détruit et le clic est perdu).
    pressed: std::rc::Rc<Cell<bool>>,
    pending: std::rc::Rc<std::cell::RefCell<Option<(Vec<WindowInfo>, NoosConfig)>>>,
    /// Un menu contextuel d'icône est ouvert : la reconstruction attend sa fermeture (sinon
    /// le bouton, et donc son menu, est détruit, par exemple quand l'ouverture du menu retire
    /// le focus à la fenêtre active et que la liste des fenêtres change).
    menu_open: std::rc::Rc<Cell<bool>>,
    /// Dernier contenu affiché (évite de tout reconstruire si rien n'a changé).
    shown: std::cell::RefCell<String>,
    logo: gtk::Image,
    height: Cell<i32>,
    /// Icônes des applis actives, directement dans la barre (option « étaler »).
    tray_inline: gtk::Box,
    /// Bouton « ^ » et son menu (icônes regroupées, comme Windows 11).
    tray_overflow: gtk::MenuButton,
    tray_grid: gtk::FlowBox,
    bell_icon: gtk::Image,
    bell_badge: gtk::Label,
    net_btn: gtk::Button,
    net_icon: gtk::Image,
    /// VPN actif : bouclier dans la zone d'état, menu pour changer de profil.
    vpn_btn: gtk::MenuButton,
    vpn_list: gtk::Box,
    vpn_shown: std::cell::RefCell<Vec<crate::net::Vpn>>,
    vol_btn: gtk::Button,
    vol_icon: gtk::Image,
}

/// Épaisseur d'une barre (dock ou barre fine).
pub fn bar_height(cfg: &NoosConfig, top: bool) -> i32 {
    if top { cfg.topbar.height.clamp(24, 48) as i32 } else { cfg.taskbar.height.clamp(28, 96) as i32 }
}

/// Bord d'une barre : le dock selon la configuration, la barre fine au bord opposé (en haut
/// si le dock est sur un côté).
pub fn bar_edge(cfg: &NoosConfig, top: bool) -> &'static str {
    let dock = match cfg.taskbar.position.as_str() {
        "top" => "top",
        "left" => "left",
        "right" => "right",
        _ => "bottom",
    };
    if !top {
        dock
    } else if dock == "top" {
        "bottom"
    } else {
        "top"
    }
}

/// Taille des icônes selon la hauteur de la barre (24 px pour 48 px, comme Windows 11).
fn icon_size(height: i32) -> i32 {
    (height / 2).clamp(16, 40)
}

/// Un bouton de la barre : appli épinglée et/ou fenêtres ouvertes.
struct Item {
    app_id: String,
    pinned: bool,
    windows: Vec<WindowInfo>,
}

impl Taskbar {
    /// `top` : barre fine supplémentaire (bord opposé au dock).
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor, top: bool) -> Self {
        let cfg = shell.config();
        let bar_h = bar_height(&cfg, top);
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-taskbar"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(bar_h);

        let root = gtk::CenterBox::new();
        root.add_css_class("noos-taskbar");
        if top {
            root.add_css_class("noos-topbar");
        }

        // ── Bouton Démarrer ──
        let start_btn = gtk::Button::new();
        start_btn.set_tooltip_text(Some("Démarrer"));
        let logo = gtk::Image::from_file(paths::data_dir().join("noos-start.svg"));
        // Logo un peu plus grand que les icônes d'applis (repère du bouton Démarrer).
        logo.set_pixel_size(icon_size(bar_h) + 6);
        start_btn.set_child(Some(&logo));
        {
            let shell = shell.clone();
            start_btn.connect_clicked(move |_| {
                if let Some(q) = shell.quick.borrow().as_ref() {
                    q.hide();
                }
                if let Some(s) = shell.start.borrow().as_ref() {
                    s.toggle(None);
                }
            });
        }

        // ── Recherche ──
        let search = gtk::Button::new();
        let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        search_box.append(&gtk::Image::from_icon_name("system-search-symbolic"));
        let search_label = gtk::Label::new(Some("Rechercher"));
        search_label.add_css_class("dim-label");
        search_box.append(&search_label);
        search_box.add_css_class("search-pill");
        search.set_child(Some(&search_box));
        {
            let shell = shell.clone();
            search.connect_clicked(move |_| {
                if let Some(s) = shell.start.borrow().as_ref() {
                    s.toggle(Some(""));
                }
            });
        }

        let tasks = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let main = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        main.set_valign(gtk::Align::Center);
        // ── Vue des tâches (bureaux virtuels) : clic = vue, molette = bureau voisin ──
        let tv_btn = gtk::Button::new();
        let tv_icon = gtk::Image::from_icon_name("window-restore-symbolic");
        tv_icon.set_pixel_size(icon_size(bar_h));
        tv_btn.set_child(Some(&tv_icon));
        tv_btn.add_css_class("task");
        tv_btn.add_css_class("taskview-button");
        tv_btn.set_tooltip_text(Some("Vue des tâches et bureaux (Super+Tab)\nMolette : bureau suivant / précédent"));
        tv_btn.set_visible(cfg.desktops.taskbar_button);
        {
            let shell = shell.clone();
            tv_btn.connect_clicked(move |_| {
                if let Some(tv) = shell.taskview.borrow().as_ref() {
                    tv.toggle();
                }
            });
        }
        {
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
            scroll.connect_scroll(|_, _, dy| {
                let delta = if dy > 0.0 { 1 } else { -1 };
                apps::request(Request::SwitchDesktopRelative { delta });
                glib::Propagation::Stop
            });
            tv_btn.add_controller(scroll);
        }


        // ── Zone de droite ──
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        end.set_valign(gtk::Align::Fill);

        // ── Réseau, volume, réglages rapides : trois boutons distincts ──
        let hide_others = {
            let shell = shell.clone();
            move |keep: &str| {
                if let Some(s) = shell.start.borrow().as_ref() {
                    s.hide();
                }
                if keep != "quick" {
                    if let Some(q) = shell.quick.borrow().as_ref() {
                        q.hide();
                    }
                }
                if keep != "net" {
                    if let Some(n) = shell.net.borrow().as_ref() {
                        n.hide();
                    }
                }
                if keep != "audio" {
                    if let Some(a) = shell.audio.borrow().as_ref() {
                        a.hide();
                    }
                }
                if let Some(n) = shell.notify.borrow().as_ref() {
                    n.hide();
                }
            }
        };
        let status_button = |icon: &str, tip: &str| {
            let b = gtk::Button::new();
            b.set_valign(gtk::Align::Center);
            b.add_css_class("status-icon");
            b.set_tooltip_text(Some(tip));
            let img = gtk::Image::from_icon_name(icon);
            b.set_child(Some(&img));
            (b, img)
        };
        let (net_btn, net_icon) = status_button("network-wired-symbolic", "Réseau");
        let (vol_btn, vol_icon) = status_button("audio-volume-high-symbolic", "Volume");
        let (quick_btn, _) = status_button("emblem-system-symbolic", "Réglages rapides");
        {
            let (shell, hide) = (shell.clone(), hide_others.clone());
            net_btn.connect_clicked(move |_| {
                hide("net");
                if let Some(n) = shell.net.borrow().as_ref() {
                    n.toggle();
                }
            });
        }
        {
            let (shell, hide) = (shell.clone(), hide_others.clone());
            vol_btn.connect_clicked(move |_| {
                hide("audio");
                if let Some(a) = shell.audio.borrow().as_ref() {
                    a.toggle();
                }
            });
        }
        {
            // Molette sur le haut-parleur : ±5 %, comme Windows.
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            let shell = shell.clone();
            scroll.connect_scroll(move |_, _, dy| {
                crate::audio::nudge_default(if dy < 0.0 { 0.05 } else { -0.05 });
                shell.refresh_status_soon();
                glib::Propagation::Stop
            });
            vol_btn.add_controller(scroll);
        }
        {
            // Clic maintenu + glisser sur le haut-parleur : vers le haut ou la droite = plus
            // fort. Un simple clic (sans glisser) ouvre toujours le panneau Volume.
            let drag = gtk::GestureDrag::new();
            drag.set_propagation_phase(gtk::PropagationPhase::Capture);
            let start = std::rc::Rc::new(Cell::new(None::<f64>));
            let last_sent = std::rc::Rc::new(Cell::new(std::time::Instant::now()));
            {
                let start = start.clone();
                drag.connect_drag_begin(move |_, _, _| start.set(None));
            }
            {
                let (start, last_sent, icon) = (start.clone(), last_sent.clone(), vol_icon.clone());
                let btn = vol_btn.clone();
                drag.connect_drag_update(move |g, dx, dy| {
                    let delta = dx - dy;
                    if start.get().is_none() {
                        if delta.abs() < 6.0 && dy.abs() < 6.0 {
                            return;
                        }
                        // Le geste devient un réglage : le clic du bouton est annulé.
                        g.set_state(gtk::EventSequenceState::Claimed);
                        start.set(Some(crate::audio::default_volume().map(|(v, _)| v).unwrap_or(0.5)));
                    }
                    let v = (start.get().unwrap_or(0.5) + delta / 200.0).clamp(0.0, 1.0);
                    icon.set_icon_name(Some(crate::audio::volume_icon(v, false)));
                    btn.set_tooltip_text(Some(&format!("Volume : {} %", (v * 100.0).round())));
                    if last_sent.get().elapsed().as_millis() >= 40 {
                        last_sent.set(std::time::Instant::now());
                        crate::audio::set_default_volume(v);
                    }
                });
            }
            {
                let (start, shell) = (start.clone(), shell.clone());
                drag.connect_drag_end(move |_, dx, dy| {
                    if let Some(s) = start.take() {
                        crate::audio::set_default_volume((s + (dx - dy) / 200.0).clamp(0.0, 1.0));
                        shell.refresh_status_soon();
                    }
                });
            }
            vol_btn.add_controller(drag);
        }
        {
            let (shell, hide) = (shell.clone(), hide_others.clone());
            quick_btn.connect_clicked(move |_| {
                hide("quick");
                if let Some(q) = shell.quick.borrow().as_ref() {
                    q.toggle();
                }
            });
        }
        let vpn_btn = gtk::MenuButton::new();
        vpn_btn.set_valign(gtk::Align::Center);
        vpn_btn.set_icon_name("network-vpn-symbolic");
        vpn_btn.set_always_show_arrow(false);
        vpn_btn.set_direction(gtk::ArrowType::Up);
        vpn_btn.add_css_class("status-icon");
        vpn_btn.add_css_class("vpn-active");
        vpn_btn.set_visible(false);
        let vpn_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        vpn_list.add_css_class("vpn-menu");
        let vpn_pop = gtk::Popover::new();
        vpn_pop.set_has_arrow(false);
        vpn_pop.set_child(Some(&vpn_list));
        vpn_btn.set_popover(Some(&vpn_pop));
        noos_theme::fit_popover(&vpn_pop);
        let status_group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_group.add_css_class("status-group");
        status_group.set_valign(gtk::Align::Center);
        if let Some(u) = shell.updates.borrow().as_ref() {
            status_group.append(&u.button());
        }
        if let Some(d) = shell.drives.borrow().as_ref() {
            status_group.append(&d.button());
        }
        status_group.append(&vpn_btn);
        status_group.append(&net_btn);
        status_group.append(&vol_btn);
        status_group.append(&quick_btn);

        // ── Icônes des applications actives ──
        let tray_inline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tray_inline.set_valign(gtk::Align::Center);
        let tray_overflow = gtk::MenuButton::new();
        tray_overflow.set_valign(gtk::Align::Center);
        tray_overflow.set_icon_name("pan-up-symbolic");
        tray_overflow.set_tooltip_text(Some("Afficher les icônes cachées"));
        tray_overflow.set_direction(gtk::ArrowType::Up);
        tray_overflow.set_always_show_arrow(false);
        tray_overflow.add_css_class("tray-overflow");
        let tray_grid = gtk::FlowBox::new();
        tray_grid.set_selection_mode(gtk::SelectionMode::None);
        tray_grid.set_max_children_per_line(5);
        tray_grid.set_min_children_per_line(1);
        tray_grid.set_homogeneous(true);
        tray_grid.add_css_class("tray-grid");
        let tray_pop = gtk::Popover::new();
        tray_pop.set_has_arrow(false);
        tray_pop.add_css_class("tray-popover");
        tray_pop.set_child(Some(&tray_grid));
        tray_overflow.set_popover(Some(&tray_pop));
        tray_overflow.set_visible(false);

        // ── Horloge et cloche : ouvrent le centre de notifications + calendrier ──
        let clock_btn = gtk::Button::new();
        clock_btn.set_valign(gtk::Align::Center);
        let clock_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let time = gtk::Label::new(None);
        time.add_css_class("clock-time");
        time.set_xalign(1.0);
        let date = gtk::Label::new(None);
        date.add_css_class("clock-date");
        date.set_xalign(1.0);
        clock_box.append(&time);
        clock_box.append(&date);
        clock_btn.set_child(Some(&clock_box));
        let bell_btn = gtk::Button::new();
        bell_btn.set_valign(gtk::Align::Center);
        bell_btn.set_tooltip_text(Some("Notifications"));
        bell_btn.add_css_class("bell");
        let bell_overlay = gtk::Overlay::new();
        let bell_icon = gtk::Image::from_icon_name("preferences-system-notifications-symbolic");
        bell_overlay.set_child(Some(&bell_icon));
        let bell_badge = gtk::Label::new(None);
        bell_badge.add_css_class("bell-badge");
        bell_badge.set_halign(gtk::Align::End);
        bell_badge.set_valign(gtk::Align::Start);
        bell_badge.set_visible(false);
        bell_overlay.add_overlay(&bell_badge);
        bell_btn.set_child(Some(&bell_overlay));
        for b in [&clock_btn, &bell_btn] {
            let shell = shell.clone();
            b.connect_clicked(move |_| {
                if let Some(s) = shell.start.borrow().as_ref() {
                    s.hide();
                }
                if let Some(q) = shell.quick.borrow().as_ref() {
                    q.hide();
                }
                if let Some(n) = shell.notify.borrow().as_ref() {
                    n.toggle();
                }
            });
        }
        let update_clock = {
            let (time, date) = (time.clone(), date.clone());
            move || {
                if let Ok(now) = glib::DateTime::now_local() {
                    time.set_text(&now.format("%H:%M").map(|s| s.to_string()).unwrap_or_default());
                    date.set_text(&now.format("%d/%m/%Y").map(|s| s.to_string()).unwrap_or_default());
                    clock_box_tooltip(&time, &now);
                }
            }
        };
        update_clock();
        glib::timeout_add_seconds_local(1, move || {
            update_clock();
            glib::ControlFlow::Continue
        });

        // Bande « Afficher le bureau » (tout à droite, comme Windows).
        let show_desktop = gtk::Button::new();
        show_desktop.add_css_class("show-desktop");
        show_desktop.set_tooltip_text(Some("Afficher le bureau"));
        show_desktop.connect_clicked(|_| apps::request(Request::ShowDesktop));

        let tray_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tray_box.set_valign(gtk::Align::Center);
        tray_box.append(&tray_overflow);
        tray_box.append(&tray_inline);
        root.set_end_widget(Some(&end));
        // ── Widget Ressources (coin gauche) ──
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        left.set_valign(gtk::Align::Center);
        left.set_margin_start(6);
        let vitals = noos_widget_vitals::Vitals::new();
        // Bureaux virtuels (barre fine par défaut) : une pastille par bureau.
        let desk_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        desk_box.add_css_class("bar-desktops");
        desk_box.set_valign(gtk::Align::Center);
        {
            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
            scroll.connect_scroll(|_, _, dy| {
                apps::request(Request::SwitchDesktopRelative { delta: if dy > 0.0 { 1 } else { -1 } });
                glib::Propagation::Stop
            });
            desk_box.add_controller(scroll);
        }
        // Éléments de la barre, rangés dans les zones selon la configuration (mode édition).
        let frames: Vec<(String, gtk::Overlay, gtk::Box)> = [
            ("vitals", vitals.widget().clone().upcast::<gtk::Widget>()),
            ("start", start_btn.clone().upcast()),
            ("search", search.clone().upcast()),
            ("taskview", tv_btn.clone().upcast()),
            ("tasks", tasks.clone().upcast()),
            ("tray", tray_box.clone().upcast()),
            ("status", status_group.clone().upcast()),
            ("clock", clock_btn.clone().upcast()),
            ("notifications", bell_btn.clone().upcast()),
            ("showdesktop", show_desktop.clone().upcast()),
            ("desktops", desk_box.clone().upcast()),
        ]
        .into_iter()
        .map(|(id, w)| {
            let (o, shield) = crate::editmode::frame(id, &w, Rc::downgrade(shell));
            o.set_widget_name(&format!("bar-{id}"));
            (id.to_string(), o, shield)
        })
        .collect();
        for (k, z) in [&left, &main, &end].into_iter().enumerate() {
            crate::editmode::zone_drop(k + if top { 3 } else { 0 }, z, Rc::downgrade(shell));
        }
        {
            // Clic droit dans une zone vide de la barre : « Modifier la barre des tâches ».
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let pop = gtk::Popover::new();
            pop.set_has_arrow(false);
            pop.set_child(Some(&menu));
            pop.set_parent(&root);
            for (label, icon, act) in [("Modifier la barre des tâches", "document-edit-symbolic", 0), ("Paramètres de la barre des tâches", "emblem-system-symbolic", 1)] {
                let b = gtk::Button::new();
                b.add_css_class("flat");
                let c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                c.append(&gtk::Image::from_icon_name(icon));
                c.append(&gtk::Label::new(Some(label)));
                b.set_child(Some(&c));
                let (pop, sh) = (pop.clone(), Rc::downgrade(shell));
                b.connect_clicked(move |_| {
                    pop.popdown();
                    if act == 1 {
                        apps::spawn("noos-settings taskbar");
                    } else if let Some(s) = sh.upgrade() {
                        if let Some(e) = s.editmode.borrow().as_ref() {
                            e.set_active(true);
                        }
                    }
                });
                menu.append(&b);
            }
            let click = gtk::GestureClick::new();
            click.set_button(3);
            let root2 = root.clone();
            click.connect_pressed(move |g, _, x, y| {
                // Seulement dans le vide (pas sur une icône, qui a son propre menu).
                let empty = root2.pick(x, y, gtk::PickFlags::DEFAULT).is_none_or(|w| w.is::<gtk::CenterBox>() || w.has_css_class("edit-zone") || w.is::<gtk::Box>() && w.first_child().is_none());
                if empty {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                    pop.popup();
                }
            });
            root.add_controller(click);
        }

        window.set_child(Some(&root));
        let bar = Self {
            shell: Rc::downgrade(shell),
            window,
            root,
            left,
            vitals,
            end,
            frames,
            top,
            desk_box,
            status_group: status_group.clone(),
            tray_box2: tray_box.clone(),
            search_label: search_label.clone(),
            clock_date: date.clone(),
            main,
            tasks,
            output: monitor.connector().map(|c| c.to_string()).unwrap_or_default(),
            pressed: std::rc::Rc::new(Cell::new(false)),
            pending: std::rc::Rc::new(std::cell::RefCell::new(None)),
            menu_open: std::rc::Rc::new(Cell::new(false)),
            shown: std::cell::RefCell::new(String::new()),
            logo,
            height: Cell::new(bar_h),
            tray_inline,
            tray_overflow,
            tray_grid,
            bell_icon,
            bell_badge,
            net_btn,
            net_icon,
            vpn_btn,
            vpn_list,
            vpn_shown: std::cell::RefCell::new(Vec::new()),
            vol_btn,
            vol_icon,
        };
        {
            let click = gtk::GestureClick::new();
            click.set_button(0);
            click.set_propagation_phase(gtk::PropagationPhase::Capture);
            let pressed = bar.pressed.clone();
            click.connect_pressed(move |_, _, _, _| pressed.set(true));
            let (pressed, pending, shell) = (bar.pressed.clone(), bar.pending.clone(), Rc::downgrade(shell));
            let done = move || {
                pressed.set(false);
                if pending.borrow_mut().take().is_some() {
                    // Le clic a été traité : on applique la dernière liste de fenêtres reçue.
                    let shell = shell.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(shell) = shell.upgrade() {
                            shell.refresh_tasks();
                        }
                    });
                }
            };
            let done = std::rc::Rc::new(done);
            {
                let done = done.clone();
                click.connect_released(move |_, _, _, _| done());
            }
            click.connect_cancel(move |_, _| done());
            bar.window.add_controller(click);
        }
        bar.apply_config(&cfg);
        bar.window.present();
        bar
    }

    pub fn apply_config(&self, cfg: &NoosConfig) {
        let h = bar_height(cfg, self.top);
        let changed = self.height.replace(h) != h;
        let edge = bar_edge(cfg, self.top);
        let vertical = matches!(edge, "left" | "right");
        // La surface layer-shell prend la taille par défaut de la fenêtre : il faut la changer
        // explicitement, sinon elle garde sa taille de création.
        if vertical {
            self.window.set_default_size(h, -1);
            self.window.set_size_request(h, -1);
        } else {
            self.window.set_default_size(-1, h);
            self.window.set_size_request(-1, h);
        }
        self.window.set_exclusive_zone(h);
        for (e, name) in [(Edge::Top, "top"), (Edge::Bottom, "bottom"), (Edge::Left, "left"), (Edge::Right, "right")] {
            // Le bord de la barre, et les deux bords perpendiculaires (pleine largeur / hauteur).
            let perpendicular = if vertical { matches!(name, "top" | "bottom") } else { matches!(name, "left" | "right") };
            self.window.set_anchor(e, name == edge || perpendicular);
        }
        self.logo.set_pixel_size(icon_size(h) + 6);
        self.orient(vertical, edge);
        self.window.queue_resize();
        let _ = changed;
        self.layout(cfg);
    }

    /// Barre horizontale ou verticale (dock sur un côté).
    fn orient(&self, vertical: bool, edge: &str) {
        let o = if vertical { gtk::Orientation::Vertical } else { gtk::Orientation::Horizontal };
        self.root.set_orientation(o);
        for b in [&self.left, &self.main, &self.end, &self.tasks, &self.status_group, &self.tray_box2, &self.tray_inline, &self.desk_box] {
            b.set_orientation(o);
        }
        self.search_label.set_visible(!vertical);
        self.clock_date.set_visible(!vertical);
        self.vitals.set_vertical(vertical);
        if vertical {
            self.root.add_css_class("vertical");
        } else {
            self.root.remove_css_class("vertical");
        }
        let dir = match edge {
            "top" => gtk::ArrowType::Down,
            "left" => gtk::ArrowType::Right,
            "right" => gtk::ArrowType::Left,
            _ => gtk::ArrowType::Up,
        };
        self.vpn_btn.set_direction(dir);
        self.tray_overflow.set_direction(dir);
    }

    /// Pastilles des bureaux virtuels (élément « desktops »).
    pub fn update_desktops(&self, names: &[String], current: usize) {
        while let Some(c) = self.desk_box.first_child() {
            self.desk_box.remove(&c);
        }
        let vertical = self.desk_box.orientation() == gtk::Orientation::Vertical;
        for (i, n) in names.iter().enumerate() {
            let b = gtk::Button::with_label(&if vertical { (i + 1).to_string() } else { n.clone() });
            b.add_css_class("desk-pill");
            if i == current {
                b.add_css_class("current");
            }
            b.set_tooltip_text(Some(&format!("{n} (Super+Ctrl+← / →)")));
            b.connect_clicked(move |_| apps::request(Request::SwitchDesktop { index: i }));
            self.desk_box.append(&b);
        }
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("desk-pill");
        add.add_css_class("add");
        add.set_tooltip_text(Some("Nouveau bureau (Super+Ctrl+D)"));
        add.connect_clicked(|_| apps::request(Request::AddDesktop { name: None, switch: true }));
        self.desk_box.append(&add);
    }

    /// Range les éléments dans les zones (configuration du mode édition).
    pub fn layout(&self, cfg: &NoosConfig) {
        for (_, o, _) in &self.frames {
            if let Some(p) = o.parent().and_downcast::<gtk::Box>() {
                p.remove(o);
            }
        }
        let z = cfg.bar_zones();
        let k = if self.top { 3 } else { 0 };
        let (mut l, mut c, r) = (z[k].clone(), z[k + 1].clone(), z[k + 2].clone());
        // Alignement à gauche (dock horizontal) : les éléments du centre suivent ceux de gauche.
        if !self.top && cfg.taskbar.alignment == TaskbarAlignment::Left && !cfg.taskbar.vertical() {
            l.append(&mut c);
        }
        // Éléments masqués par leurs réglages (Paramètres › Barre des tâches).
        let off = |id: &str| match id {
            "search" => !cfg.taskbar.show_search,
            "vitals" => !cfg.taskbar.show_vitals,
            "taskview" => !cfg.desktops.taskbar_button,
            _ => false,
        };
        for (id, o, _) in &self.frames {
            o.set_visible(!off(id));
        }
        for (zone, ids) in [(&self.left, l), (&self.main, c), (&self.end, r)] {
            for id in ids {
                if let Some((_, o, _)) = self.frames.iter().find(|(i, _, _)| *i == id) {
                    zone.append(o);
                }
            }
        }
        self.root.set_start_widget(Some(&self.left));
        self.root.set_center_widget(Some(&self.main));
        self.root.set_end_widget(Some(&self.end));
    }

    /// Mode édition : boucliers des éléments et zones visibles.
    pub fn set_editing(&self, on: bool) {
        for (_, _, shield) in &self.frames {
            shield.set_visible(on);
        }
        if on {
            self.root.add_css_class("editing");
        } else {
            self.root.remove_css_class("editing");
        }
    }

    pub fn update(&self, windows: &[WindowInfo], cfg: &NoosConfig) {
        if self.pressed.get() || self.menu_open.get() {
            *self.pending.borrow_mut() = Some((windows.to_vec(), cfg.clone()));
            return;
        }
        // Bureau actif seulement (sauf réglage), et écran de cette barre si demandé.
        let (current, primary) = self
            .shell
            .upgrade()
            .map(|s| (s.desktops.borrow().1, s.primary_output()))
            .unwrap_or((0, None));
        let is_primary = primary.as_ref().is_none_or(|p| *p == self.output);
        let windows: Vec<WindowInfo> = windows
            .iter()
            .filter(|w| cfg.desktops.taskbar_all || w.sticky || w.desktop == current)
            .filter(|w| !cfg.taskbar.windows_on_own_output || w.output == self.output || (w.output.is_empty() && is_primary))
            .cloned()
            .collect();
        let windows = &windows[..];
        let key = format!(
            "{:?}|{:?}|{}|{current}",
            cfg.taskbar.pinned,
            windows.iter().map(|w| (w.id, &w.app_id, &w.title, w.focused, w.minimized, w.desktop, w.sticky)).collect::<Vec<_>>(),
            self.height.get()
        );
        if *self.shown.borrow() == key {
            return;
        }
        *self.shown.borrow_mut() = key;
        while let Some(child) = self.tasks.first_child() {
            self.tasks.remove(&child);
        }

        let mut items: Vec<Item> = cfg
            .taskbar
            .pinned
            .iter()
            .filter(|id| apps::exists(id))
            .map(|id| Item { app_id: id.clone(), pinned: true, windows: vec![] })
            .collect();
        for w in windows {
            if let Some(item) = items.iter_mut().find(|i| apps::matches(&i.app_id, &w.app_id)) {
                item.windows.push(w.clone());
            } else if let Some(item) = items.iter_mut().find(|i| !i.pinned && i.app_id == w.app_id) {
                item.windows.push(w.clone());
            } else {
                items.push(Item { app_id: w.app_id.clone(), pinned: false, windows: vec![w.clone()] });
            }
        }

        let size = icon_size(self.height.get());
        for item in items {
            let (pinned, app_id) = (item.pinned, item.app_id.clone());
            let hold = Hold { menu_open: self.menu_open.clone(), pending: self.pending.clone() };
            let button = task_button(item, size, &self.shell, hold);
            if pinned {
                // Glisser-déposer pour changer l'ordre des épinglées.
                crate::reorder::attach(&button, &app_id, &self.tasks, &self.shell);
            }
            self.tasks.append(&button);
        }
    }
}

impl Taskbar {
    /// Icônes réseau et volume (état lu périodiquement par le shell).
    pub fn update_status(&self, st: &crate::StatusIcons) {
        self.net_icon.set_icon_name(Some(&st.net_icon));
        self.net_btn.set_tooltip_text(Some(&st.net_tooltip));
        self.vol_icon.set_icon_name(Some(&st.vol_icon));
        self.vol_btn.set_tooltip_text(Some(&st.vol_tooltip));
        self.update_vpn(&st.vpns);
    }

    /// Bouclier VPN : visible quand un profil est actif ; son menu liste les profils.
    fn update_vpn(&self, vpns: &[crate::net::Vpn]) {
        let active = vpns.iter().find(|v| v.active);
        self.vpn_btn.set_visible(active.is_some());
        if let Some(a) = active {
            self.vpn_btn.set_tooltip_text(Some(&format!("VPN : {}", a.name)));
        }
        // Menu ouvert : on ne le reconstruit pas sous le pointeur.
        if *self.vpn_shown.borrow() == vpns || self.vpn_btn.popover().is_some_and(|p| p.is_visible()) {
            return;
        }
        *self.vpn_shown.borrow_mut() = vpns.to_vec();
        while let Some(c) = self.vpn_list.first_child() {
            self.vpn_list.remove(&c);
        }
        let title = gtk::Label::new(Some("Profil VPN"));
        title.add_css_class("vpn-menu-title");
        title.set_xalign(0.0);
        self.vpn_list.append(&title);
        let pop = self.vpn_btn.popover();
        let act = {
            let (shell, pop) = (self.shell.clone(), pop.clone());
            move |uuid: Option<String>| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                let (tx, rx) = async_channel::bounded(1);
                std::thread::spawn(move || {
                    let _ = tx.send_blocking(crate::net::switch_vpn(uuid.as_deref()));
                });
                let shell = shell.clone();
                glib::spawn_future_local(async move {
                    if let Ok(Err(e)) = rx.recv().await {
                        eprintln!("noos-shell : VPN : {e}");
                    }
                    if let Some(s) = shell.upgrade() {
                        s.refresh_status_soon();
                    }
                });
            }
        };
        let item = |icon: &str, text: &str| {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("vpn-item");
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            h.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(text));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            h.append(&l);
            b.set_child(Some(&h));
            b
        };
        for v in vpns {
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("vpn-item");
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = gtk::Image::from_icon_name(if v.active { "object-select-symbolic" } else { "network-vpn-symbolic" });
            if v.active {
                icon.add_css_class("vpn-check");
            }
            h.append(&icon);
            let l = gtk::Label::new(Some(&v.name));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            h.append(&l);
            b.set_child(Some(&h));
            b.set_tooltip_text(Some(if v.active { "Profil actif" } else { "Basculer sur ce profil" }));
            let (act, uuid, active) = (act.clone(), v.uuid.clone(), v.active);
            b.connect_clicked(move |_| {
                if !active {
                    act(Some(uuid.clone()));
                }
            });
            self.vpn_list.append(&b);
        }
        self.vpn_list.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let off = item("network-vpn-disconnected-symbolic", "Se déconnecter du VPN");
        {
            let act = act.clone();
            off.connect_clicked(move |_| act(None));
        }
        self.vpn_list.append(&off);
        let settings = item("emblem-system-symbolic", "Paramètres VPN…");
        {
            let pop = pop.clone();
            settings.connect_clicked(move |_| {
                if let Some(p) = &pop {
                    p.popdown();
                }
                let _ = std::process::Command::new("noos-settings").arg("vpn").spawn();
            });
        }
        self.vpn_list.append(&settings);
        if std::env::var_os("NOOS_VPN_DEMO").is_some() {
            let b = self.vpn_btn.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || b.popup());
        }
    }

    /// Zone de notification et cloche.
    pub fn update_indicators(&self, tray: Option<&Rc<crate::tray::Tray>>, cfg: &NoosConfig, notifs: (usize, bool)) {
        // FlowBox : remove_all() (un unparent() direct corrompt sa liste interne d'enfants).
        self.tray_grid.remove_all();
        while let Some(c) = self.tray_inline.first_child() {
            self.tray_inline.remove(&c);
        }
        let items = tray.map(|t| t.items()).unwrap_or_default();
        let size = (icon_size(self.height.get()) - 6).clamp(16, 24);
        if let Some(tray) = tray {
            let pop = self.tray_overflow.popover();
            let close: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(p) = pop.as_ref() {
                    p.popdown();
                }
            });
            for item in &items {
                let b = tray.button(item, size, close.clone());
                if cfg.taskbar.tray_expanded {
                    self.tray_inline.append(&b);
                } else {
                    self.tray_grid.insert(&b, -1);
                }
            }
        }
        self.tray_overflow.set_visible(!cfg.taskbar.tray_expanded && !items.is_empty());
        self.tray_inline.set_visible(cfg.taskbar.tray_expanded && !items.is_empty());

        let (count, dnd) = notifs;
        self.bell_icon.set_icon_name(Some(if dnd {
            "notifications-disabled-symbolic"
        } else {
            "preferences-system-notifications-symbolic"
        }));
        self.bell_badge.set_visible(count > 0 && !dnd);
        self.bell_badge.set_text(&if count > 9 { "9+".to_string() } else { count.to_string() });
    }
}

fn clock_box_tooltip(label: &gtk::Label, now: &glib::DateTime) {
    if let Ok(full) = now.format("%A %e %B %Y") {
        if let Some(parent) = label.parent().and_then(|p| p.parent()) {
            parent.set_tooltip_text(Some(&full));
        }
    }
}

/// De quoi suspendre la reconstruction de la barre pendant qu'un menu d'icône est ouvert.
struct Hold {
    menu_open: std::rc::Rc<Cell<bool>>,
    pending: std::rc::Rc<std::cell::RefCell<Option<(Vec<WindowInfo>, NoosConfig)>>>,
}

fn task_button(item: Item, icon_px: i32, shell: &Weak<Shell>, hold: Hold) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("task");
    let focused = item.windows.iter().any(|w| w.focused && !w.minimized);
    if focused {
        button.add_css_class("focused");
    }

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vbox.set_valign(gtk::Align::Center);
    let icon = gtk::Image::from_gicon(&apps::icon(&item.app_id));
    icon.set_pixel_size(icon_px);
    vbox.append(&icon);
    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indicator.add_css_class("indicator");
    indicator.set_halign(gtk::Align::Center);
    if item.windows.is_empty() {
        indicator.add_css_class("hidden");
    } else if focused {
        indicator.add_css_class("focused");
    }
    vbox.append(&indicator);
    button.set_child(Some(&vbox));

    let tooltip = match item.windows.as_slice() {
        [] => apps::display_name(&item.app_id),
        [w] if !w.title.is_empty() => w.title.clone(),
        ws => format!("{} ({} fenêtres)", apps::display_name(&item.app_id), ws.len()),
    };
    button.set_tooltip_text(Some(&tooltip));

    // Clic gauche : lancer, activer, ou réduire la fenêtre active.
    {
        let app_id = item.app_id.clone();
        let windows = item.windows.clone();
        button.connect_clicked(move |_| match windows.as_slice() {
            [] => apps::launch(&app_id),
            ws => {
                // Plusieurs fenêtres : on passe à la suivante après celle qui a le focus.
                let idx = ws.iter().position(|w| w.focused).map(|i| (i + 1) % ws.len()).unwrap_or(0);
                let target = if ws.len() > 1 && ws.iter().any(|w| w.focused) { &ws[idx] } else { &ws[0] };
                apps::request(Request::Activate { id: target.id });
            }
        });
    }

    // Clic droit : menu contextuel.
    let popover = gtk::Popover::new();
    popover.set_parent(&button);
    popover.set_has_arrow(false);
    {
        let (popover, menu_open) = (popover.clone(), hold.menu_open.clone());
        button.connect_destroy(move |_| {
            // Bouton détruit menu ouvert : ne jamais laisser la barre figée.
            menu_open.set(false);
            popover.unparent();
        });
    }
    noos_theme::fit_popover(&popover);
    {
        // Menu fermé : la barre peut de nouveau être reconstruite (liste de fenêtres reçue
        // entre-temps).
        let (menu_open, pending, shell) = (hold.menu_open.clone(), hold.pending.clone(), shell.clone());
        popover.connect_closed(move |_| {
            menu_open.set(false);
            if pending.borrow_mut().take().is_some() {
                let shell = shell.clone();
                glib::idle_add_local_once(move || {
                    if let Some(shell) = shell.upgrade() {
                        shell.refresh_tasks();
                    }
                });
            }
        });
    }
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let entry = |label: &str, icon: &str| {
        let b = gtk::Button::new();
        let c = adap::ButtonContent::builder().label(label).icon_name(icon).build();
        c.set_halign(gtk::Align::Start);
        b.set_child(Some(&c));
        b.add_css_class("flat");
        b
    };
    let open = entry(&apps::display_name(&item.app_id), "list-add-symbolic");
    {
        let app_id = item.app_id.clone();
        let popover = popover.clone();
        open.connect_clicked(move |_| {
            popover.popdown();
            apps::launch(&app_id);
        });
    }
    menu.append(&open);

    // Épingler / désépingler (comme Windows 11). Il faut un fichier .desktop pour épingler :
    // c'est lui qui sert à relancer l'appli.
    let desktop_id = if item.pinned {
        Some(item.app_id.clone())
    } else {
        apps::find(&item.app_id).and_then(|a| a.id()).map(|id| id.trim_end_matches(".desktop").to_string())
    };
    if let Some(desktop_id) = desktop_id {
        let pinned = item.pinned;
        let label = if pinned { "Désépingler de la barre des tâches" } else { "Épingler à la barre des tâches" };
        let pin = entry(label, "view-pin-symbolic");
        let (shell, popover) = (shell.clone(), popover.clone());
        pin.connect_clicked(move |_| {
            popover.popdown();
            let Some(shell) = shell.upgrade() else { return };
            let id = desktop_id.clone();
            // Après la fermeture du menu : la barre va être reconstruite.
            glib::idle_add_local_once(move || {
                shell.update_config(|c| {
                    if pinned {
                        c.taskbar.pinned.retain(|p| *p != id);
                    } else if !c.taskbar.pinned.contains(&id) {
                        c.taskbar.pinned.push(id);
                    }
                });
                shell.refresh_tasks();
            });
        });
        menu.append(&pin);
    }

    if !item.windows.is_empty() {
        let ids: Vec<u64> = item.windows.iter().map(|w| w.id).collect();
        let (desktops, outputs) = shell.upgrade().map(|s| (s.desktops.borrow().clone(), s.outputs.borrow().clone())).unwrap_or_default();
        let sticky = item.windows.iter().all(|w| w.sticky);
        let win_desktop = item.windows[0].desktop;
        if desktops.0.len() > 1 && !sticky {
            for (i, name) in desktops.0.iter().enumerate() {
                if i == win_desktop {
                    continue;
                }
                let b = entry(&format!("Déplacer vers « {name} »"), "go-next-symbolic");
                let (ids, popover) = (ids.clone(), popover.clone());
                b.connect_clicked(move |_| {
                    popover.popdown();
                    for id in &ids {
                        apps::request(Request::MoveWindowToDesktop { id: *id, index: i, follow: false });
                    }
                });
                menu.append(&b);
            }
        }
        {
            let b = entry(if sticky { "Afficher seulement sur ce bureau" } else { "Afficher sur tous les bureaux" }, "view-pin-symbolic");
            let (ids, popover) = (ids.clone(), popover.clone());
            b.connect_clicked(move |_| {
                popover.popdown();
                for id in &ids {
                    apps::request(Request::SetSticky { id: *id, sticky: !sticky });
                }
            });
            menu.append(&b);
        }
        if outputs.len() > 1 {
            let here = item.windows[0].output.clone();
            for (k, o) in outputs.iter().enumerate() {
                if o.name == here {
                    continue;
                }
                let b = entry(&format!("Déplacer vers l'écran {} ({})", k + 1, o.model), "video-display-symbolic");
                let (ids, popover, name) = (ids.clone(), popover.clone(), o.name.clone());
                b.connect_clicked(move |_| {
                    popover.popdown();
                    for id in &ids {
                        apps::request(Request::MoveWindowToOutput { id: *id, output: name.clone() });
                    }
                });
                menu.append(&b);
            }
        }
        let label = if item.windows.len() > 1 { "Fermer toutes les fenêtres" } else { "Fermer la fenêtre" };
        let close = entry(label, "window-close-symbolic");
        let ids: Vec<u64> = item.windows.iter().map(|w| w.id).collect();
        let popover2 = popover.clone();
        close.connect_clicked(move |_| {
            popover2.popdown();
            for id in &ids {
                apps::request(Request::Close { id: *id });
            }
        });
        menu.append(&close);
    }
    popover.set_child(Some(&menu));
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    {
        let popover = popover.clone();
        let menu_open = hold.menu_open.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            menu_open.set(true);
            popover.popup();
        });
    }
    button.add_controller(gesture);
    // Clic du milieu : nouvelle instance.
    let middle = gtk::GestureClick::new();
    middle.set_button(2);
    {
        let app_id = item.app_id.clone();
        middle.connect_pressed(move |_, _, _, _| apps::launch(&app_id));
    }
    button.add_controller(middle);
    button
}

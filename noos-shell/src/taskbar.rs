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
    root: gtk::CenterBox,
    main: gtk::Box,
    search: gtk::Button,
    tasks: gtk::Box,
    /// Un bouton de souris est enfoncé sur la barre : la reconstruction des icônes attend
    /// le relâchement (sinon le bouton cliqué est détruit et le clic est perdu).
    pressed: std::rc::Rc<Cell<bool>>,
    pending: std::rc::Rc<std::cell::RefCell<Option<(Vec<WindowInfo>, NoosConfig)>>>,
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
    vol_btn: gtk::Button,
    vol_icon: gtk::Image,
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
    pub fn new(shell: &Rc<Shell>, monitor: &gdk::Monitor) -> Self {
        let cfg = shell.config();
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-taskbar"));
        window.set_layer(Layer::Top);
        window.set_monitor(Some(monitor));
        window.set_keyboard_mode(KeyboardMode::None);
        for edge in [Edge::Left, Edge::Right, Edge::Bottom] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(cfg.taskbar.height as i32);
        window.set_default_size(-1, cfg.taskbar.height as i32);

        let root = gtk::CenterBox::new();
        root.add_css_class("noos-taskbar");

        // ── Bouton Démarrer ──
        let start_btn = gtk::Button::new();
        start_btn.set_tooltip_text(Some("Démarrer"));
        let logo = gtk::Image::from_file(paths::data_dir().join("noos-start.svg"));
        logo.set_pixel_size(icon_size(cfg.taskbar.height as i32) + 2);
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
        main.append(&start_btn);
        main.append(&search);
        main.append(&tasks);

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
        let status_group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_group.add_css_class("status-group");
        status_group.set_valign(gtk::Align::Center);
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

        end.append(&tray_overflow);
        end.append(&tray_inline);
        end.append(&status_group);
        end.append(&clock_btn);
        end.append(&bell_btn);
        end.append(&show_desktop);
        root.set_end_widget(Some(&end));

        window.set_child(Some(&root));
        let bar = Self {
            shell: Rc::downgrade(shell),
            window,
            root,
            main,
            search,
            tasks,
            pressed: std::rc::Rc::new(Cell::new(false)),
            pending: std::rc::Rc::new(std::cell::RefCell::new(None)),
            shown: std::cell::RefCell::new(String::new()),
            logo,
            height: Cell::new(cfg.taskbar.height as i32),
            tray_inline,
            tray_overflow,
            tray_grid,
            bell_icon,
            bell_badge,
            net_btn,
            net_icon,
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
        let h = cfg.taskbar.height.clamp(28, 96) as i32;
        let changed = self.height.replace(h) != h;
        // La surface layer-shell prend la taille par défaut de la fenêtre : il faut la changer
        // explicitement, sinon elle garde sa hauteur de création.
        self.window.set_default_size(-1, h);
        self.window.set_size_request(-1, h);
        self.window.set_exclusive_zone(h);
        self.logo.set_pixel_size(icon_size(h) + 2);
        if changed {
            self.window.queue_resize();
        }
        self.search.set_visible(cfg.taskbar.show_search);
        self.root.set_start_widget(gtk::Widget::NONE);
        self.root.set_center_widget(gtk::Widget::NONE);
        match cfg.taskbar.alignment {
            TaskbarAlignment::Center => self.root.set_center_widget(Some(&self.main)),
            TaskbarAlignment::Left => self.root.set_start_widget(Some(&self.main)),
        }
    }

    pub fn update(&self, windows: &[WindowInfo], cfg: &NoosConfig) {
        if self.pressed.get() {
            *self.pending.borrow_mut() = Some((windows.to_vec(), cfg.clone()));
            return;
        }
        let key = format!(
            "{:?}|{:?}|{}",
            cfg.taskbar.pinned,
            windows.iter().map(|w| (w.id, &w.app_id, &w.title, w.focused, w.minimized)).collect::<Vec<_>>(),
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
            self.tasks.append(&task_button(item, size, &self.shell));
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

fn task_button(item: Item, icon_px: i32, shell: &Weak<Shell>) -> gtk::Button {
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
        let popover = popover.clone();
        button.connect_destroy(move |_| popover.unparent());
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
        gesture.connect_pressed(move |_, _, _, _| popover.popup());
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

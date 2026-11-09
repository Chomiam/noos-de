//! Menu Démarrer façon Windows 11 : recherche, grille d'applis épinglées,
//! liste de toutes les applications, utilisateur et alimentation en bas.

use std::{cell::Cell, rc::Rc, time::Instant};

use adap::prelude::*;
use gtk::{gio, glib};
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use noos_core::ipc::Request;

use crate::{Shell, apps};

pub struct StartMenu {
    window: gtk::Window,
    search: gtk::SearchEntry,
    stack: gtk::Stack,
    pinned: gtk::FlowBox,
    all: gtk::ListBox,
    results: gtk::ListBox,
    hidden_at: Cell<Option<Instant>>,
}

fn tile_widget(name: &str, icon: &gio::Icon, tooltip: Option<&str>) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("app-tile");
    let v = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let image = gtk::Image::from_gicon(icon);
    image.set_pixel_size(32);
    v.append(&image);
    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(11);
    label.set_justify(gtk::Justification::Center);
    v.append(&label);
    b.set_child(Some(&v));
    b.set_tooltip_text(tooltip.or(Some(name)));
    b
}

fn app_tile(app: &gio::AppInfo) -> gtk::Button {
    let icon = app.icon().unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast());
    tile_widget(&app.display_name(), &icon, app.description().as_deref())
}

fn app_row(app: &gio::AppInfo) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    h.set_margin_top(6);
    h.set_margin_bottom(6);
    h.set_margin_start(8);
    let icon = match app.icon() {
        Some(i) => gtk::Image::from_gicon(&i),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    icon.set_pixel_size(24);
    h.append(&icon);
    let label = gtk::Label::new(Some(&app.display_name()));
    label.set_xalign(0.0);
    h.append(&label);
    row.set_child(Some(&h));
    unsafe { row.set_data("noos-app", app.clone()) };
    row
}

fn row_app(row: &gtk::ListBoxRow) -> Option<gio::AppInfo> {
    unsafe { row.data::<gio::AppInfo>("noos-app").map(|p| p.as_ref().clone()) }
}

impl StartMenu {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let cfg = shell.config();
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-startmenu"));
        window.set_layer(Layer::Top);
        crate::editmode::anchor_to_bar(&window, &shell.config());
        shell.flyouts.borrow_mut().push(window.downgrade());
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_default_size(640, 680);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.add_css_class("noos-flyout");
        root.set_size_request(640, 680);

        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher des applications"));
        root.append(&search);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_vexpand(true);

        // ── Accueil : épinglées ──
        let home = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let title = gtk::Label::new(Some("Épinglé"));
        title.add_css_class("section-title");
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let all_btn = gtk::Button::new();
        all_btn.set_child(Some(
            &adap::ButtonContent::builder().label("Toutes les applications").icon_name("go-next-symbolic").build(),
        ));
        all_btn.add_css_class("pill");
        header.append(&all_btn);
        home.append(&header);

        let pinned = gtk::FlowBox::new();
        pinned.set_max_children_per_line(6);
        pinned.set_min_children_per_line(6);
        pinned.set_selection_mode(gtk::SelectionMode::None);
        pinned.set_homogeneous(true);
        pinned.set_row_spacing(4);
        pinned.set_column_spacing(4);
        pinned.set_valign(gtk::Align::Start);
        let pinned_scroll = gtk::ScrolledWindow::builder()
            .child(&pinned)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        home.append(&pinned_scroll);
        stack.add_named(&home, Some("home"));

        // ── Toutes les applications ──
        let all_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let back = gtk::Button::new();
        back.set_child(Some(&adap::ButtonContent::builder().label("Retour").icon_name("go-previous-symbolic").build()));
        back.set_halign(gtk::Align::End);
        back.add_css_class("pill");
        all_page.append(&back);
        let all = gtk::ListBox::new();
        all.set_selection_mode(gtk::SelectionMode::None);
        all.set_activate_on_single_click(true);
        all_page.append(
            &gtk::ScrolledWindow::builder().child(&all).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build(),
        );
        stack.add_named(&all_page, Some("all"));

        // ── Résultats de recherche ──
        let results = gtk::ListBox::new();
        results.set_selection_mode(gtk::SelectionMode::Single);
        results.set_activate_on_single_click(true);
        let results_scroll =
            gtk::ScrolledWindow::builder().child(&results).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        stack.add_named(&results_scroll, Some("search"));
        root.append(&stack);

        // ── Pied : utilisateur + alimentation ──
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("footer");
        // NOOS_DEMO_NAME : nom affiché à la place du vôtre (captures d'écran publiques).
        let user_name = std::env::var("NOOS_DEMO_NAME").unwrap_or_else(|_| glib::real_name().to_string_lossy().to_string());
        let user_name = if user_name.is_empty() || user_name == "Unknown" {
            glib::user_name().to_string_lossy().to_string()
        } else {
            user_name
        };
        let user_btn = gtk::Button::new();
        let user_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        user_box.append(&adap::Avatar::new(32, Some(&user_name), true));
        user_box.append(&gtk::Label::new(Some(&user_name)));
        user_btn.set_child(Some(&user_box));
        user_btn.set_tooltip_text(Some("Comptes"));
        user_btn.connect_clicked(|_| apps::launch("org.noos.Settings"));
        footer.append(&user_btn);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);

        let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings_btn.set_tooltip_text(Some("Paramètres"));
        settings_btn.connect_clicked(|_| apps::launch("org.noos.Settings"));
        footer.append(&settings_btn);

        // Alimentation : une rangée qui se déplie dans le menu lui-même (un popover, surface
        // séparée, faisait perdre le focus au menu Démarrer qui se refermait aussitôt).
        let power = gtk::ToggleButton::new();
        power.set_icon_name("system-shutdown-symbolic");
        power.set_tooltip_text(Some("Alimentation"));
        footer.append(&power);
        let power_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        power_row.add_css_class("power-row");
        power_row.set_homogeneous(true);
        // Verrouiller et Veille passent par le compositeur (verrouillage avant la mise en veille).
        for (label, icon, cmd) in [
            ("Verrouiller", "system-lock-screen-symbolic", Some("@lock")),
            ("Veille", "weather-clear-night-symbolic", Some("@suspend")),
            ("Se déconnecter", "system-log-out-symbolic", None),
            ("Redémarrer", "system-reboot-symbolic", Some("systemctl reboot")),
            ("Arrêter", "system-shutdown-symbolic", Some("systemctl poweroff")),
        ] {
            let b = gtk::Button::new();
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            v.append(&gtk::Image::from_icon_name(icon));
            let l = gtk::Label::new(Some(label));
            l.add_css_class("caption");
            v.append(&l);
            b.set_child(Some(&v));
            b.add_css_class("flat");
            b.add_css_class("power-action");
            if cmd == Some("systemctl poweroff") {
                b.add_css_class("power-off");
            }
            let power = power.clone();
            b.connect_clicked(move |_| {
                power.set_active(false);
                match cmd {
                    Some("@lock") => apps::request(Request::Lock),
                    Some("@suspend") => apps::request(Request::Suspend),
                    Some(c) => apps::spawn(c),
                    None => apps::request(Request::Quit),
                }
            });
            power_row.append(&b);
        }
        let power_revealer = gtk::Revealer::new();
        power_revealer.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        power_revealer.set_child(Some(&power_row));
        {
            let r = power_revealer.clone();
            power.connect_toggled(move |b| r.set_reveal_child(b.is_active()));
        }
        root.append(&power_revealer);
        root.append(&footer);
        {
            // Menu refermé : la rangée d'alimentation aussi.
            let power = power.clone();
            window.connect_hide(move |_| power.set_active(false));
        }

        window.set_child(Some(&root));

        let menu = Rc::new(Self {
            window,
            search,
            stack,
            pinned,
            all,
            results,
            hidden_at: Cell::new(None),
        });
        menu.fill(&cfg.taskbar.start_pinned);

        // ── Comportement ──
        {
            let m = menu.clone();
            all_btn.connect_clicked(move |_| m.stack.set_visible_child_name("all"));
        }
        {
            let m = menu.clone();
            back.connect_clicked(move |_| m.stack.set_visible_child_name("home"));
        }
        {
            let m = menu.clone();
            menu.search.connect_search_changed(move |entry| m.on_search(&entry.text()));
        }
        {
            let m = menu.clone();
            menu.search.connect_activate(move |_| {
                if let Some(app) = m.results.row_at_index(0).and_then(|r| row_app(&r)) {
                    m.launch(&app);
                }
            });
        }
        for list in [&menu.all, &menu.results] {
            let m = menu.clone();
            list.connect_row_activated(move |_, row| {
                if let Some(app) = row_app(row) {
                    m.launch(&app);
                }
            });
        }
        {
            let m = menu.clone();
            menu.search.connect_stop_search(move |_| m.hide());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let m = menu.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    m.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            menu.window.add_controller(keys);
        }
        {
            let m = menu.clone();
            crate::close_on_focus_loss(&menu.window, move || m.hide());
        }
        {
            let m = menu.clone();
            shell.theming.connect_changed(move |cfg| m.fill(&cfg.taskbar.start_pinned));
        }
        menu
    }

    fn fill(self: &Rc<Self>, start_pinned: &[String]) {
        while let Some(c) = self.pinned.first_child() {
            self.pinned.remove(&c);
        }
        let mut shown = std::collections::HashSet::new();
        for id in start_pinned {
            if let Some(app) = apps::find(id) {
                let app: gio::AppInfo = app.upcast();
                let key = app.id().map(|s| s.to_string()).unwrap_or_default();
                if !shown.insert(key) {
                    continue;
                }
                let tile = app_tile(&app);
                let m = self.clone();
                tile.connect_clicked(move |_| m.launch(&app));
                self.pinned.insert(&tile, -1);
            } else if let Some((name, icon)) = apps::builtin(id) {
                let tile = tile_widget(name, &gio::ThemedIcon::new(icon).upcast(), None);
                let (m, id) = (self.clone(), id.clone());
                tile.connect_clicked(move |_| {
                    apps::launch(&id);
                    m.hide();
                });
                self.pinned.insert(&tile, -1);
            }
        }
        self.all.remove_all();
        for app in apps::all() {
            self.all.append(&app_row(&app));
        }
    }

    fn on_search(&self, text: &str) {
        if text.trim().is_empty() {
            self.stack.set_visible_child_name("home");
            return;
        }
        self.results.remove_all();
        for app in apps::search(text).into_iter().take(30) {
            self.results.append(&app_row(&app));
        }
        if let Some(first) = self.results.row_at_index(0) {
            self.results.select_row(Some(&first));
        }
        self.stack.set_visible_child_name("search");
    }

    fn launch(&self, app: &gio::AppInfo) {
        apps::launch_info(app);
        self.hide();
    }

    pub fn toggle(&self, search: Option<&str>) {
        // Le clic sur le bouton Démarrer a déjà fermé le menu (perte de focus) : ne pas le rouvrir.
        if let Some(t) = self.hidden_at.get() {
            if t.elapsed().as_millis() < 250 {
                return;
            }
        }
        if self.window.is_visible() {
            self.hide();
        } else {
            self.show(search);
        }
    }

    pub fn show(&self, search: Option<&str>) {
        self.search.set_text(search.unwrap_or(""));
        self.stack.set_visible_child_name("home");
        self.window.present();
        self.search.grab_focus();
    }

    pub fn hide(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
            self.hidden_at.set(Some(Instant::now()));
        }
    }
}

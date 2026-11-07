//! noos-installer : installateur de NOOS (Fedora 45, installation réseau), plein écran.
//!
//! Lancé par l'ISO live via `noos-comp --drm --kiosk noos-installer` (root). Étapes :
//! bienvenue (langue, réseau) → fuseau horaire (carte) → clavier → compte → disque (Btrfs
//! @ / @home, swap) → récapitulatif → installation (journal, erreurs) → redémarrage.

mod account;
mod disk;
mod hardware;
mod install;
mod keyboard;
mod model;
mod tzmap;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

use account::Strength;
use model::Choices;

const APP_ID: &str = "org.noos.Installer";

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        adap::StyleManager::default().set_color_scheme(adap::ColorScheme::ForceDark);
    });
    app.connect_activate(|app| {
        let ui = Installer::new(app);
        ui.window.present();
        // Les rappels de l'interface ne gardent que des références faibles : l'installateur
        // vit aussi longtemps que l'application.
        app.connect_shutdown(move |_| {
            let _ = &ui;
        });
    });
    app.run_with_args::<&str>(&[])
}

/// Une page de l'assistant.
struct Page {
    title: &'static str,
    subtitle: &'static str,
    icon: &'static str,
    widget: gtk::Widget,
    /// Peut-on passer à la suite ? (avec un message sinon)
    valid: Rc<dyn Fn() -> Result<(), String>>,
    /// Appelée à l'affichage de la page.
    enter: Rc<dyn Fn()>,
}

struct Installer {
    app: adap::Application,
    window: adap::ApplicationWindow,
    choices: Rc<RefCell<Choices>>,
    pages: RefCell<Vec<Page>>,
    current: Cell<usize>,
    stack: gtk::Stack,
    steps: gtk::ListBox,
    title: gtk::Label,
    subtitle: gtk::Label,
    back: gtk::Button,
    next: gtk::Button,
    hint: gtk::Label,
    installing: Cell<bool>,
}

impl Installer {
    fn new(app: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder().application(app).title("Installer NOOS").default_width(1280).default_height(800).build();
        window.add_css_class("noos-installer");
        if std::env::var_os("NOOS_INSTALLER_WINDOWED").is_none() {
            window.fullscreen();
        }
        let ui = Rc::new(Self {
            app: app.clone(),
            window,
            choices: Rc::new(RefCell::new(Choices::default())),
            pages: RefCell::new(Vec::new()),
            current: Cell::new(0),
            stack: gtk::Stack::new(),
            steps: gtk::ListBox::new(),
            title: gtk::Label::new(None),
            subtitle: gtk::Label::new(None),
            back: gtk::Button::with_label("Retour"),
            next: gtk::Button::with_label("Suivant"),
            hint: gtk::Label::new(None),
            installing: Cell::new(false),
        });
        ui.build();
        ui
    }

    fn build(self: &Rc<Self>) {
        let pages = vec![
            self.welcome_page(),
            self.timezone_page(),
            self.keyboard_page(),
            self.account_page(),
            self.disk_page(),
            self.summary_page(),
        ];

        // Barre latérale : logo + étapes.
        let side = gtk::Box::new(gtk::Orientation::Vertical, 18);
        side.add_css_class("installer-side");
        side.set_width_request(280);
        side.set_hexpand(false);
        let logo = logo_picture(200);
        logo.set_margin_top(28);
        side.append(&logo);
        let label = gtk::Label::new(Some("Installation"));
        label.add_css_class("installer-side-title");
        side.append(&label);
        self.steps.add_css_class("installer-steps");
        self.steps.set_selection_mode(gtk::SelectionMode::None);
        for (i, p) in pages.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("installer-step");
            let num = gtk::Label::new(Some(&(i + 1).to_string()));
            num.add_css_class("installer-step-num");
            row.append(&num);
            let icon = gtk::Image::from_icon_name(p.icon);
            row.append(&icon);
            let l = gtk::Label::new(Some(p.title));
            l.set_xalign(0.0);
            row.append(&l);
            self.steps.append(&row);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("installer-step");
        let num = gtk::Label::new(Some(&(pages.len() + 1).to_string()));
        num.add_css_class("installer-step-num");
        row.append(&num);
        row.append(&gtk::Image::from_icon_name("system-software-install-symbolic"));
        row.append(&gtk::Label::new(Some("Installation")));
        self.steps.append(&row);
        side.append(&self.steps);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        side.append(&spacer);
        let fedora = gtk::Label::new(Some("Basé sur Fedora 45 · installation réseau"));
        fedora.add_css_class("dim-label");
        fedora.add_css_class("caption");
        fedora.set_margin_bottom(18);
        side.append(&fedora);

        // Contenu.
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.set_hexpand(true);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 4);
        head.add_css_class("installer-head");
        self.title.add_css_class("installer-title");
        self.title.set_xalign(0.0);
        self.subtitle.add_css_class("dim-label");
        self.subtitle.set_xalign(0.0);
        self.subtitle.set_wrap(true);
        head.append(&self.title);
        head.append(&self.subtitle);
        main.append(&head);
        self.stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        self.stack.set_vexpand(true);
        for (i, p) in pages.iter().enumerate() {
            let clamp = adap::Clamp::new();
            clamp.set_maximum_size(980);
            clamp.set_child(Some(&p.widget));
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
            scroll.set_child(Some(&clamp));
            self.stack.add_named(&scroll, Some(&i.to_string()));
        }
        main.append(&self.stack);
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bottom.add_css_class("installer-bottom");
        self.hint.add_css_class("installer-hint");
        self.hint.set_hexpand(true);
        self.hint.set_xalign(0.0);
        self.hint.set_wrap(true);
        bottom.append(&self.hint);
        self.back.add_css_class("pill");
        self.next.add_css_class("pill");
        self.next.add_css_class("suggested-action");
        bottom.append(&self.back);
        bottom.append(&self.next);
        main.append(&bottom);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&side);
        root.append(&main);
        self.window.set_content(Some(&root));
        *self.pages.borrow_mut() = pages;

        let weak = Rc::downgrade(self);
        self.next.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.go_next();
            }
        });
        let weak = Rc::downgrade(self);
        self.back.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                let i = ui.current.get();
                if i > 0 {
                    ui.show(i - 1);
                }
            }
        });
        // Revalidation régulière (saisie, réseau, disques).
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            ui.refresh_nav();
            glib::ControlFlow::Continue
        });
        // NOOS_INSTALLER_PAGE=n : ouvre directement la page n (tests, captures).
        let page = std::env::var("NOOS_INSTALLER_PAGE").unwrap_or_default();
        let dry = std::env::var("NOOS_INSTALL_DRYRUN").is_ok_and(|v| v == "1");
        if page == "install" && dry {
            // Simulation de bout en bout avec des choix d'exemple.
            let mut c = self.choices.borrow_mut();
            c.full_name = "Axel Valens".into();
            c.username = "axel".into();
            c.hostname = "axel-pc".into();
            c.password = "Exemple-de-mot-de-passe-1".into();
            c.disk = disk::disks().ok().and_then(|d| d.into_iter().next());
            drop(c);
            self.show(0);
            self.start_install();
            return;
        }
        let start = page.parse().unwrap_or(0usize);
        self.show(start.min(self.pages.borrow().len() - 1));
        // NOOS_INSTALLER_SCROLL=bottom : page affichée défilée jusqu'en bas (captures).
        if std::env::var("NOOS_INSTALLER_SCROLL").is_ok_and(|v| v == "bottom") {
            if let Some(sc) = self.stack.visible_child().and_downcast::<gtk::ScrolledWindow>() {
                glib::timeout_add_local_once(std::time::Duration::from_millis(800), move || {
                    let adj = sc.vadjustment();
                    adj.set_value(adj.upper());
                });
            }
        }
    }

    fn show(self: &Rc<Self>, i: usize) {
        self.current.set(i);
        self.stack.set_visible_child_name(&i.to_string());
        let pages = self.pages.borrow();
        if let Some(p) = pages.get(i) {
            self.title.set_label(p.title);
            self.subtitle.set_label(p.subtitle);
            (p.enter)();
        }
        for (n, row) in (0..).map_while(|n| self.steps.row_at_index(n).map(|r| (n, r))) {
            row.remove_css_class("current");
            row.remove_css_class("done");
            match (n as usize).cmp(&i) {
                std::cmp::Ordering::Less => row.add_css_class("done"),
                std::cmp::Ordering::Equal => row.add_css_class("current"),
                _ => {}
            }
        }
        self.back.set_visible(i > 0);
        self.next.set_label(if i + 1 == pages.len() { "Installer NOOS" } else { "Suivant" });
        drop(pages);
        self.refresh_nav();
    }

    fn refresh_nav(&self) {
        if self.installing.get() {
            return;
        }
        let pages = self.pages.borrow();
        let Some(p) = pages.get(self.current.get()) else { return };
        match (p.valid)() {
            Ok(()) => {
                self.next.set_sensitive(true);
                self.hint.set_label("");
            }
            Err(msg) => {
                self.next.set_sensitive(false);
                self.hint.set_label(&msg);
            }
        }
    }

    fn go_next(self: &Rc<Self>) {
        let i = self.current.get();
        let n = self.pages.borrow().len();
        if (self.pages.borrow()[i].valid)().is_err() {
            return;
        }
        if i + 1 < n {
            self.show(i + 1);
            return;
        }
        // Dernière page : confirmation avant d'effacer le disque.
        let c = self.choices.borrow().clone();
        let Some(d) = c.disk.clone() else { return };
        let dialog = adap::AlertDialog::new(
            Some("Effacer le disque et installer NOOS ?"),
            Some(&format!(
                "Toutes les données de « {} » ({}) seront définitivement effacées. Cette opération est irréversible.",
                d.title(),
                d.path
            )),
        );
        dialog.add_responses(&[("cancel", "Annuler"), ("install", "Effacer et installer")]);
        dialog.set_response_appearance("install", adap::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, r| {
            if r == "install" {
                if let Some(ui) = weak.upgrade() {
                    ui.start_install();
                }
            }
        });
        dialog.present(Some(&self.window));
    }

    // ───────────────────────── 1. Bienvenue ─────────────────────────

    fn welcome_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        hero.add_css_class("installer-card");
        let t = gtk::Box::new(gtk::Orientation::Vertical, 8);
        t.set_valign(gtk::Align::Center);
        let h = gtk::Label::new(Some("Bienvenue dans NOOS"));
        h.add_css_class("title-1");
        h.set_xalign(0.0);
        t.append(&h);
        let s = gtk::Label::new(Some(
            "Un bureau moderne, rapide et pensé pour le jeu, construit sur Fedora. Quelques questions, puis NOOS s'installe depuis Internet avec les paquets les plus récents.",
        ));
        s.set_wrap(true);
        s.set_xalign(0.0);
        s.add_css_class("dim-label");
        t.append(&s);
        hero.append(&t);
        v.append(&hero);

        let g = adap::PreferencesGroup::new();
        g.set_title("Langue du système");
        let langs: Vec<&str> = model::LOCALES.iter().map(|l| l.1).collect();
        let row = adap::ComboRow::new();
        row.set_title("Langue");
        row.set_model(Some(&gtk::StringList::new(&langs)));
        let c = self.choices.clone();
        row.connect_selected_notify(move |r| {
            if let Some((loc, _, kb, tz)) = model::LOCALES.get(r.selected() as usize) {
                let mut ch = c.borrow_mut();
                ch.locale = loc.to_string();
                // Disposition et fuseau proposés (modifiables aux étapes suivantes).
                let (layout, variant) = kb.split_once(':').unwrap_or((kb, ""));
                ch.kb_layout = layout.into();
                ch.kb_variant = variant.into();
                ch.timezone = tz.to_string();
            }
        });
        g.add(&row);
        v.append(&g);

        // Réseau (obligatoire : installation réseau).
        let g = adap::PreferencesGroup::new();
        g.set_title("Connexion à Internet");
        g.set_description(Some("NOOS télécharge Fedora 45 depuis les miroirs officiels pendant l'installation."));
        let status = adap::ActionRow::new();
        status.set_title("Vérification…");
        let icon = gtk::Image::from_icon_name("network-wired-symbolic");
        status.add_prefix(&icon);
        let wifi = gtk::Button::with_label("Wi-Fi…");
        wifi.set_valign(gtk::Align::Center);
        let retry = gtk::Button::from_icon_name("view-refresh-symbolic");
        retry.set_valign(gtk::Align::Center);
        retry.add_css_class("flat");
        retry.set_tooltip_text(Some("Vérifier à nouveau"));
        status.add_suffix(&wifi);
        status.add_suffix(&retry);
        g.add(&status);
        v.append(&g);

        let online = Rc::new(Cell::new(false));
        let check = {
            let (status, icon, online) = (status.clone(), icon.clone(), online.clone());
            Rc::new(move || {
                let (status, icon, online) = (status.clone(), icon.clone(), online.clone());
                glib::spawn_future_local(async move {
                    let state = gio::spawn_blocking(connectivity).await.unwrap_or(Net::Offline);
                    online.set(state != Net::Offline);
                    let (title, sub, ic) = match state {
                        Net::Wired => ("Connecté (filaire)", "Prêt pour l'installation réseau", "network-wired-symbolic"),
                        Net::Wifi(ref ssid) => ("Connecté (Wi-Fi)", ssid.as_str(), "network-wireless-signal-excellent-symbolic"),
                        Net::Online => ("Connecté", "Prêt pour l'installation réseau", "network-transmit-receive-symbolic"),
                        Net::Offline => ("Aucune connexion", "Branchez un câble réseau ou connectez-vous à un Wi-Fi", "network-offline-symbolic"),
                    };
                    status.set_title(title);
                    status.set_subtitle(sub);
                    icon.set_icon_name(Some(ic));
                });
            })
        };
        check();
        {
            let check = check.clone();
            retry.connect_clicked(move |_| check());
        }
        {
            let (win, check) = (self.window.clone(), check.clone());
            wifi.connect_clicked(move |_| wifi_dialog(&win, check.clone()));
        }
        // Vérification périodique tant que la page est affichée.
        {
            let check = check.clone();
            glib::timeout_add_seconds_local(6, move || {
                check();
                glib::ControlFlow::Continue
            });
        }

        let dry = std::env::var("NOOS_INSTALL_DRYRUN").is_ok_and(|v| v == "1");
        Page {
            title: "Bienvenue",
            subtitle: "Langue et connexion à Internet",
            icon: "emoji-people-symbolic",
            widget: v.upcast(),
            valid: Rc::new(move || {
                if online.get() || dry { Ok(()) } else { Err("Une connexion à Internet est nécessaire (installation réseau).".into()) }
            }),
            enter: Rc::new(|| {}),
        }
    }

    // ───────────────────────── 2. Fuseau horaire ─────────────────────────

    fn timezone_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let cities = Rc::new(tzmap::load_cities());
        let map = tzmap::TzMap::new(cities.clone());
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("installer-map");
        frame.append(&map.widget);
        map.widget.set_content_height(360);
        map.widget.set_vexpand(false);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher une ville (ex. Lyon, Montréal, Bruxelles)"));
        search.set_hexpand(true);
        row.append(&search);
        let info = gtk::Label::new(None);
        info.add_css_class("installer-tz-info");
        row.append(&info);
        v.append(&row);
        v.append(&frame);

        let results = gtk::ListBox::new();
        results.add_css_class("boxed-list");
        let pop = gtk::Popover::new();
        pop.set_autohide(true);
        pop.set_has_arrow(false);
        pop.set_position(gtk::PositionType::Bottom);
        let rs = gtk::ScrolledWindow::new();
        rs.set_max_content_height(320);
        rs.set_propagate_natural_height(true);
        rs.set_min_content_width(420);
        rs.set_child(Some(&results));
        pop.set_child(Some(&rs));
        pop.set_parent(&search);

        let choose = {
            let (c, info, map) = (self.choices.clone(), info.clone(), map.clone());
            Rc::new(move |city: &tzmap::City| {
                c.borrow_mut().timezone = city.tz.clone();
                map.select(city);
                info.set_label(&tz_description(&city.tz));
            })
        };
        {
            let choose = choose.clone();
            map.connect_select(move |city| choose(city));
        }
        let found: Rc<RefCell<Vec<tzmap::City>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let (cities, results, pop, found) = (cities.clone(), results.clone(), pop.clone(), found.clone());
            search.connect_search_changed(move |e| {
                let q = tzmap::normalize(e.text().trim());
                results.remove_all();
                found.borrow_mut().clear();
                if q.len() < 2 {
                    pop.popdown();
                    return;
                }
                let mut hits: Vec<&tzmap::City> = cities
                    .iter()
                    .filter(|c| tzmap::normalize(&c.name).starts_with(&q) || tzmap::normalize(&c.tz).contains(&q))
                    .collect();
                hits.sort_by_key(|c| std::cmp::Reverse(c.population.min(u64::MAX - 1)));
                hits.dedup_by(|a, b| a.name == b.name && a.tz == b.tz);
                for c in hits.into_iter().take(10) {
                    let r = adap::ActionRow::new();
                    r.set_title(&c.label());
                    r.set_subtitle(&c.tz);
                    r.set_activatable(true);
                    results.append(&r);
                    found.borrow_mut().push(c.clone());
                }
                if found.borrow().is_empty() {
                    pop.popdown();
                } else {
                    pop.popup();
                }
            });
        }
        {
            let (found, pop, choose, search) = (found.clone(), pop.clone(), choose.clone(), search.clone());
            results.connect_row_activated(move |_, row| {
                if let Some(c) = found.borrow().get(row.index() as usize) {
                    choose(c);
                    search.set_text("");
                }
                pop.popdown();
            });
        }
        let (c, cities2, choose2) = (self.choices.clone(), cities.clone(), choose.clone());
        Page {
            title: "Fuseau horaire",
            subtitle: "Cliquez sur la carte près de chez vous, ou recherchez votre ville.",
            icon: "preferences-system-time-symbolic",
            widget: v.upcast(),
            valid: Rc::new(|| Ok(())),
            enter: Rc::new(move || {
                let tz = c.borrow().timezone.clone();
                // Ville principale du fuseau (chef-lieu de zone1970.tab).
                if let Some(city) = cities2.iter().find(|x| x.tz == tz && x.population == u64::MAX) {
                    choose2(city);
                }
            }),
        }
    }

    // ───────────────────────── 3. Clavier ─────────────────────────

    fn keyboard_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let layouts = Rc::new(keyboard::layouts());
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher une disposition (français, BÉPO, suisse…)"));
        v.append(&search);
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::Single);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(380);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list));
        v.append(&scroll);
        for l in layouts.iter() {
            let r = adap::ActionRow::new();
            r.set_title(&l.description);
            r.set_subtitle(&l.id());
            list.append(&r);
        }
        let test = gtk::Entry::new();
        test.set_placeholder_text(Some("Testez votre clavier ici : é è à ç ù € @ # …"));
        test.add_css_class("installer-test");
        v.append(&test);

        {
            let ls = layouts.clone();
            let s = search.clone();
            list.set_filter_func(move |row| {
                let q = tzmap::normalize(s.text().trim());
                if q.is_empty() {
                    return true;
                }
                ls.get(row.index() as usize).is_some_and(|l| tzmap::normalize(&l.description).contains(&q) || l.id().contains(&q))
            });
        }
        {
            let list = list.clone();
            search.connect_search_changed(move |_| list.invalidate_filter());
        }
        {
            let (c, ls) = (self.choices.clone(), layouts.clone());
            list.connect_row_selected(move |_, row| {
                let Some(l) = row.and_then(|r| ls.get(r.index() as usize)) else { return };
                {
                    let mut ch = c.borrow_mut();
                    if ch.kb_layout == l.layout && ch.kb_variant == l.variant && !ch.kb_label.is_empty() {
                        ch.kb_label = l.description.clone();
                        return;
                    }
                    ch.kb_layout = l.layout.clone();
                    ch.kb_variant = l.variant.clone();
                    ch.kb_label = l.description.clone();
                }
                let (lay, var) = (l.layout.clone(), l.variant.clone());
                std::thread::spawn(move || keyboard::apply_live(&lay, &var));
            });
        }
        let (c, list2, ls) = (self.choices.clone(), list.clone(), layouts.clone());
        Page {
            title: "Clavier",
            subtitle: "Choisissez la disposition de votre clavier et essayez-la.",
            icon: "input-keyboard-symbolic",
            widget: v.upcast(),
            valid: Rc::new(|| Ok(())),
            enter: Rc::new(move || {
                let (lay, var) = {
                    let ch = c.borrow();
                    (ch.kb_layout.clone(), ch.kb_variant.clone())
                };
                if let Some(i) = ls.iter().position(|l| l.layout == lay && l.variant == var) {
                    if let Some(row) = list2.row_at_index(i as i32) {
                        list2.select_row(Some(&row));
                        let r = row.clone();
                        glib::idle_add_local_once(move || {
                            r.grab_focus();
                        });
                    }
                }
            }),
        }
    }

    // ───────────────────────── 4. Compte ─────────────────────────

    fn account_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let g = adap::PreferencesGroup::new();
        g.set_title("Votre compte");
        g.set_description(Some("Ce compte administre l'ordinateur (groupe wheel)."));
        let full = adap::EntryRow::new();
        full.set_title("Nom complet");
        let user = adap::EntryRow::new();
        user.set_title("Nom d'utilisateur (identifiant)");
        let user_err = gtk::Label::new(None);
        user_err.add_css_class("installer-error");
        user_err.set_xalign(0.0);
        user_err.set_wrap(true);
        let host = adap::EntryRow::new();
        host.set_title("Nom de cet ordinateur");
        g.add(&full);
        g.add(&user);
        g.add(&host);
        v.append(&g);
        v.append(&user_err);

        let g = adap::PreferencesGroup::new();
        g.set_title("Mot de passe");
        let pw = adap::PasswordEntryRow::new();
        pw.set_title("Mot de passe");
        let pw2 = adap::PasswordEntryRow::new();
        pw2.set_title("Confirmer le mot de passe");
        let strong = adap::SwitchRow::new();
        strong.set_title("Exiger un mot de passe fort");
        strong.set_subtitle("12 caractères minimum, majuscules et minuscules, un chiffre et un symbole");
        strong.set_active(true);
        g.add(&pw);
        g.add(&pw2);
        g.add(&strong);
        v.append(&g);

        let meter = gtk::Box::new(gtk::Orientation::Vertical, 6);
        meter.add_css_class("installer-card");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let ml = gtk::Label::new(Some("Robustesse du mot de passe"));
        ml.set_xalign(0.0);
        ml.set_hexpand(true);
        let level_label = gtk::Label::new(None);
        level_label.add_css_class("heading");
        top.append(&ml);
        top.append(&level_label);
        meter.append(&top);
        let bar = gtk::LevelBar::for_interval(0.0, 5.0);
        bar.set_mode(gtk::LevelBarMode::Discrete);
        for name in ["low", "high", "full"] {
            bar.remove_offset_value(Some(name));
        }
        bar.add_css_class("pw-meter");
        meter.append(&bar);
        let reqs = gtk::Label::new(None);
        reqs.add_css_class("caption");
        reqs.add_css_class("dim-label");
        reqs.set_xalign(0.0);
        reqs.set_wrap(true);
        meter.append(&reqs);
        v.append(&meter);

        let user_edited = Rc::new(Cell::new(false));
        let host_edited = Rc::new(Cell::new(false));
        {
            let (user, host, ue, he) = (user.clone(), host.clone(), user_edited.clone(), host_edited.clone());
            full.connect_changed(move |e| {
                if !ue.get() {
                    let s = account::suggest_username(&e.text());
                    user.set_text(&s);
                    ue.set(false);
                    if !he.get() {
                        host.set_text(&if s.is_empty() { String::new() } else { format!("{s}-pc") });
                        he.set(false);
                    }
                }
            });
        }
        // Édition manuelle (focus dans le champ) : plus de suggestion automatique.
        for (row, flag) in [(user.clone(), user_edited.clone()), (host.clone(), host_edited.clone())] {
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, _, _, _| {
                flag.set(true);
                glib::Propagation::Proceed
            });
            row.add_controller(keys);
        }

        let update = {
            let (c, full, user, host, pw, pw2, strong, bar, level_label, reqs, user_err) = (
                self.choices.clone(),
                full.clone(),
                user.clone(),
                host.clone(),
                pw.clone(),
                pw2.clone(),
                strong.clone(),
                bar.clone(),
                level_label.clone(),
                reqs.clone(),
                user_err.clone(),
            );
            Rc::new(move || {
                let password = pw.text().to_string();
                let name = user.text().to_string();
                let context: Vec<String> = full.text().split_whitespace().map(String::from).chain([name.clone()]).collect();
                let ctx: Vec<&str> = context.iter().map(String::as_str).collect();
                let s = account::strength(&password, &ctx);
                bar.set_value(s.level());
                for css in ["pw-very-weak", "pw-weak", "pw-fair", "pw-strong", "pw-very-strong"] {
                    bar.remove_css_class(css);
                    level_label.remove_css_class(css);
                }
                bar.add_css_class(s.css());
                level_label.add_css_class(s.css());
                level_label.set_label(s.label());
                let missing = account::strong_requirements(&password);
                reqs.set_label(&if password.is_empty() {
                    "Évitez votre nom, votre date de naissance et les mots courants.".to_string()
                } else if strong.is_active() && !missing.is_empty() {
                    format!("Il manque : {}.", missing.join(", "))
                } else if s < Strength::Fair {
                    "Ajoutez des mots, des chiffres ou des symboles pour le renforcer.".into()
                } else {
                    "Bon mot de passe.".into()
                });
                match account::username_error(&name) {
                    Some(e) if !name.is_empty() => {
                        user_err.set_label(&e);
                        user.add_css_class("error");
                    }
                    _ => {
                        user_err.set_label("");
                        user.remove_css_class("error");
                    }
                }
                if pw2.text().is_empty() || pw2.text() == pw.text() {
                    pw2.remove_css_class("error");
                } else {
                    pw2.add_css_class("error");
                }
                let mut ch = c.borrow_mut();
                ch.full_name = full.text().trim().to_string();
                ch.username = name;
                ch.hostname = host.text().trim().to_string();
                ch.password = password;
                ch.strong_password = strong.is_active();
            })
        };
        for e in [&full, &user, &host] {
            let u = update.clone();
            e.connect_changed(move |_| u());
        }
        for e in [&pw, &pw2] {
            let u = update.clone();
            e.connect_changed(move |_| u());
        }
        {
            let u = update.clone();
            strong.connect_active_notify(move |_| u());
        }
        update();

        let (c, pw2b) = (self.choices.clone(), pw2.clone());
        let full2 = full.clone();
        Page {
            title: "Compte",
            subtitle: "Créez le compte que vous utiliserez au quotidien.",
            icon: "system-users-symbolic",
            widget: v.upcast(),
            valid: Rc::new(move || {
                let ch = c.borrow();
                if ch.full_name.is_empty() {
                    return Err("Saisissez votre nom.".into());
                }
                if let Some(e) = account::username_error(&ch.username) {
                    return Err(e);
                }
                if let Some(e) = account::hostname_error(&ch.hostname) {
                    return Err(e);
                }
                if ch.password.is_empty() {
                    return Err("Choisissez un mot de passe.".into());
                }
                if ch.password != pw2b.text().as_str() {
                    return Err("Les deux mots de passe ne correspondent pas.".into());
                }
                if ch.full_name.contains(':') {
                    return Err("Le nom complet ne peut pas contenir « : ».".into());
                }
                if ch.strong_password {
                    let missing = account::strong_requirements(&ch.password);
                    if !missing.is_empty() {
                        return Err(format!("Mot de passe trop faible : il manque {}.", missing.join(", ")));
                    }
                    if account::strength(&ch.password, &[ch.username.as_str()]) < Strength::Strong {
                        return Err("Mot de passe trop prévisible : évitez les mots et suites courants.".into());
                    }
                }
                Ok(())
            }),
            enter: Rc::new(move || {
                let f = full2.clone();
                glib::idle_add_local_once(move || {
                    f.grab_focus();
                });
            }),
        }
    }

    // ───────────────────────── 5. Disque ─────────────────────────

    fn disk_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let g = adap::PreferencesGroup::new();
        g.set_title("Disque d'installation");
        g.set_description(Some("Le disque choisi sera entièrement effacé."));
        let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh.add_css_class("flat");
        refresh.set_tooltip_text(Some("Actualiser la liste des disques"));
        g.set_header_suffix(Some(&refresh));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        g.add(&list);
        v.append(&g);

        let current_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        current_box.add_css_class("installer-card");
        v.append(&current_box);

        let g2 = adap::PreferencesGroup::new();
        g2.set_title("Disposition");
        let fs = adap::ActionRow::new();
        fs.set_title("Btrfs avec sous-volumes séparés");
        fs.set_subtitle("@ monté sur /, @home sur /home (instantanés possibles), compression zstd");
        fs.add_prefix(&gtk::Image::from_icon_name("drive-multidisk-symbolic"));
        g2.add(&fs);
        let swap = adap::SwitchRow::new();
        swap.set_title("Partition d'échange (swap)");
        swap.set_subtitle("Mémoire de secours et mise en veille prolongée (hibernation)");
        swap.set_active(self.choices.borrow().swap);
        g2.add(&swap);
        let size = adap::SpinRow::with_range(1.0, 64.0, 1.0);
        size.set_title("Taille du swap (Gio)");
        size.set_subtitle(&format!("Mémoire de ce PC : {}", disk::human(disk::ram_bytes())));
        size.set_value(self.choices.borrow().swap_gib as f64);
        swap.bind_property("active", &size, "sensitive").sync_create().build();
        g2.add(&size);
        v.append(&g2);

        let after_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        after_box.add_css_class("installer-card");
        v.append(&after_box);

        let disks: Rc<RefCell<Vec<disk::Disk>>> = Rc::new(RefCell::new(Vec::new()));
        let redraw = {
            let (c, current_box, after_box) = (self.choices.clone(), current_box.clone(), after_box.clone());
            Rc::new(move || {
                for b in [&current_box, &after_box] {
                    while let Some(ch) = b.first_child() {
                        b.remove(&ch);
                    }
                }
                let ch = c.borrow();
                let Some(d) = ch.disk.as_ref() else {
                    current_box.set_visible(false);
                    after_box.set_visible(false);
                    return;
                };
                current_box.set_visible(true);
                after_box.set_visible(true);
                let t = gtk::Label::new(Some(&format!("Actuellement sur {}", d.path)));
                t.add_css_class("heading");
                t.set_xalign(0.0);
                current_box.append(&t);
                current_box.append(&disk::bar(&disk::current_segments(d)));
                let plan = disk::plan(d.size, disk::uefi(), ch.swap.then_some(ch.swap_gib));
                let t = gtk::Label::new(Some(&format!("Après l'installation ({})", if disk::uefi() { "UEFI" } else { "BIOS" })));
                t.add_css_class("heading");
                t.set_xalign(0.0);
                after_box.append(&t);
                after_box.append(&disk::bar(&disk::plan_segments(&plan)));
            })
        };
        let fill = {
            let (c, list, disks, redraw) = (self.choices.clone(), list.clone(), disks.clone(), redraw.clone());
            Rc::new(move || {
                list.remove_all();
                let found = disk::disks().unwrap_or_default();
                if found.is_empty() {
                    let r = adap::ActionRow::new();
                    r.set_title("Aucun disque disponible");
                    r.set_subtitle("Branchez un disque d'au moins 20 Go puis actualisez.");
                    list.append(&r);
                }
                let mut group: Option<gtk::CheckButton> = None;
                let mut preselect: Option<gtk::CheckButton> = None;
                let selected = c.borrow().disk.as_ref().map(|d| d.path.clone()).or_else(|| std::env::var("NOOS_INSTALLER_DISK").ok());
                for (i, d) in found.iter().enumerate() {
                    let r = adap::ActionRow::new();
                    r.set_title(&d.title());
                    let parts = if d.parts.is_empty() { "vide".to_string() } else { format!("{} partition(s)", d.parts.len()) };
                    r.set_subtitle(&format!("{} · {} · {}", d.kind(), d.path, parts));
                    r.add_prefix(&gtk::Image::from_icon_name(d.icon()));
                    let check = gtk::CheckButton::new();
                    if let Some(g) = &group {
                        check.set_group(Some(g));
                    } else {
                        group = Some(check.clone());
                    }
                    r.add_suffix(&check);
                    r.set_activatable_widget(Some(&check));
                    if d.size < disk::MIN_DISK {
                        r.set_sensitive(false);
                        r.set_subtitle(&format!("Trop petit (20 Go minimum) · {}", d.path));
                    }
                    let (c2, disks2, redraw2) = (c.clone(), disks.clone(), redraw.clone());
                    check.connect_toggled(move |b| {
                        if b.is_active() {
                            c2.borrow_mut().disk = disks2.borrow().get(i).cloned();
                            redraw2();
                        }
                    });
                    list.append(&r);
                    if selected.as_deref() == Some(&d.path) && d.size >= disk::MIN_DISK {
                        preselect = Some(check.clone());
                    }
                }
                // Disque choisi qui a disparu : on l'oublie.
                if let Some(sel) = &selected {
                    if !found.iter().any(|d| &d.path == sel) {
                        c.borrow_mut().disk = None;
                    }
                }
                *disks.borrow_mut() = found;
                // Coché après avoir rempli la liste : le choix est enregistré par le rappel.
                if let Some(chk) = preselect {
                    chk.set_active(true);
                }
                redraw();
            })
        };
        {
            let fill = fill.clone();
            refresh.connect_clicked(move |_| fill());
        }
        {
            let (c, redraw) = (self.choices.clone(), redraw.clone());
            swap.connect_active_notify(move |s| {
                c.borrow_mut().swap = s.is_active();
                redraw();
            });
        }
        {
            let (c, redraw) = (self.choices.clone(), redraw.clone());
            size.connect_value_notify(move |s| {
                c.borrow_mut().swap_gib = s.value() as u32;
                redraw();
            });
        }
        let c = self.choices.clone();
        Page {
            title: "Disque",
            subtitle: "Choisissez où installer NOOS. Btrfs, sous-volumes @ et @home, swap en option.",
            icon: "drive-harddisk-symbolic",
            widget: v.upcast(),
            valid: Rc::new(move || {
                let ch = c.borrow();
                let Some(d) = &ch.disk else { return Err("Choisissez un disque.".into()) };
                let plan = disk::plan(d.size, disk::uefi(), ch.swap.then_some(ch.swap_gib));
                let root = plan.iter().find(|p| p.role == disk::Role::Root).map(|p| p.size).unwrap_or(0);
                if root < 15 * disk::GIB {
                    return Err("Espace insuffisant pour le système : réduisez le swap.".into());
                }
                Ok(())
            }),
            enter: Rc::new(move || fill()),
        }
    }

    // ───────────────────────── 6. Récapitulatif ─────────────────────────

    fn summary_page(self: &Rc<Self>) -> Page {
        let v = page_box();
        let g = adap::PreferencesGroup::new();
        g.set_title("Récapitulatif");
        let rows: Vec<(adap::ActionRow, &str)> = [
            ("Langue", "preferences-desktop-locale-symbolic"),
            ("Fuseau horaire", "preferences-system-time-symbolic"),
            ("Clavier", "input-keyboard-symbolic"),
            ("Compte", "system-users-symbolic"),
            ("Ordinateur", "computer-symbolic"),
            ("Disque", "drive-harddisk-symbolic"),
            ("Partitions", "drive-multidisk-symbolic"),
            ("Démarrage", "system-reboot-symbolic"),
        ]
        .into_iter()
        .map(|(t, icon)| {
            let r = adap::ActionRow::new();
            r.set_title(t);
            r.add_prefix(&gtk::Image::from_icon_name(icon));
            g.add(&r);
            (r, t)
        })
        .collect();
        v.append(&g);
        let g2 = adap::PreferencesGroup::new();
        g2.set_title("Logiciels");
        let fusion = adap::SwitchRow::new();
        fusion.set_title("Pilotes et codecs propriétaires (RPM Fusion)");
        fusion.set_subtitle("Pilote NVIDIA, codecs H.264/H.265, AAC, MP3, DVD, décodage vidéo matériel, Steam…");
        fusion.set_active(true);
        g2.add(&fusion);
        let base = adap::ActionRow::new();
        base.set_title("Fedora 45 (installation réseau) + bureau NOOS");
        base.set_subtitle("Plymouth NOOS, écran de connexion noos-login, GRUB");
        g2.add(&base);
        v.append(&g2);
        // Matériel détecté : ce qui sera installé pour lui (mis à jour avec RPM Fusion).
        let hw = Rc::new(hardware::detect());
        let g3 = adap::PreferencesGroup::new();
        g3.set_title("Matériel détecté");
        let hw_list = gtk::ListBox::new();
        hw_list.add_css_class("boxed-list");
        hw_list.set_selection_mode(gtk::SelectionMode::None);
        g3.add(&hw_list);
        v.append(&g3);
        let fill_hw = {
            let (hw, hw_list) = (hw.clone(), hw_list.clone());
            move |rpmfusion: bool| {
                while let Some(c) = hw_list.first_child() {
                    hw_list.remove(&c);
                }
                for line in hardware::plan(&hw, rpmfusion).summary {
                    let (title, sub) = line.split_once(" : ").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((line.clone(), String::new()));
                    let r = adap::ActionRow::new();
                    r.set_use_markup(false);
                    r.set_title(&title);
                    r.set_subtitle(&sub);
                    let icon = if title.starts_with("Codecs") {
                        "audio-x-generic-symbolic"
                    } else if title.starts_with("Machine virtuelle") {
                        "computer-symbolic"
                    } else {
                        "video-display-symbolic"
                    };
                    r.add_prefix(&gtk::Image::from_icon_name(icon));
                    hw_list.append(&r);
                }
            }
        };
        fill_hw(true);
        let fill_hw = Rc::new(fill_hw);
        let warn = gtk::Label::new(None);
        warn.add_css_class("installer-warning");
        warn.set_wrap(true);
        warn.set_xalign(0.0);
        v.append(&warn);
        {
            let c = self.choices.clone();
            let fill_hw = fill_hw.clone();
            fusion.connect_active_notify(move |s| {
                c.borrow_mut().rpmfusion = s.is_active();
                fill_hw(s.is_active());
            });
        }
        let c = self.choices.clone();
        Page {
            title: "Récapitulatif",
            subtitle: "Vérifiez vos choix avant de lancer l'installation.",
            icon: "object-select-symbolic",
            widget: v.upcast(),
            valid: Rc::new(|| Ok(())),
            enter: Rc::new(move || {
                let ch = c.borrow();
                let lang = model::LOCALES.iter().find(|l| l.0 == ch.locale).map(|l| l.1).unwrap_or(&ch.locale);
                let disk_text = ch.disk.as_ref().map(|d| format!("{} ({})", d.title(), d.path)).unwrap_or_default();
                let plan = ch.disk.as_ref().map(|d| disk::plan(d.size, disk::uefi(), ch.swap.then_some(ch.swap_gib))).unwrap_or_default();
                let parts = plan.iter().map(|p| format!("{} {}", p.role.label(), disk::human(p.size))).collect::<Vec<_>>().join(" · ");
                let values = [
                    lang.to_string(),
                    tz_description(&ch.timezone),
                    format!("{} ({})", ch.kb_label, if ch.kb_variant.is_empty() { ch.kb_layout.clone() } else { format!("{}:{}", ch.kb_layout, ch.kb_variant) }),
                    format!("{} ({}) · administrateur", ch.full_name, ch.username),
                    ch.hostname.clone(),
                    disk_text.clone(),
                    parts,
                    if disk::uefi() { "UEFI · GRUB + shim (Secure Boot)".into() } else { "BIOS · GRUB".into() },
                ];
                for ((row, _), val) in rows.iter().zip(values) {
                    row.set_subtitle(&val);
                }
                warn.set_label(&format!("⚠ Toutes les données de {disk_text} seront effacées."));
            }),
        }
    }

    // ───────────────────────── 7. Installation ─────────────────────────

    fn start_install(self: &Rc<Self>) {
        self.installing.set(true);
        self.back.set_visible(false);
        self.next.set_visible(false);
        self.hint.set_label("Ne pas éteindre l'ordinateur pendant l'installation.");
        let n = self.pages.borrow().len();
        for i in 0..=n {
            if let Some(row) = self.steps.row_at_index(i as i32) {
                row.remove_css_class("current");
                row.add_css_class(if i < n { "done" } else { "current" });
            }
        }
        self.title.set_label("Installation de NOOS");
        self.subtitle.set_label("Téléchargement de Fedora 45 et configuration de votre système. Comptez de 10 à 30 minutes selon votre connexion.");

        let v = page_box();
        let overall = gtk::ProgressBar::new();
        overall.add_css_class("installer-progress");
        let current = gtk::Label::new(Some("Préparation…"));
        current.add_css_class("title-3");
        current.set_xalign(0.0);
        v.append(&current);
        v.append(&overall);
        // Erreur et boutons juste sous la barre : visibles sans faire défiler.
        let err_slot = gtk::Box::new(gtk::Orientation::Vertical, 10);
        v.append(&err_slot);
        let steps = gtk::ListBox::new();
        steps.add_css_class("boxed-list");
        steps.set_selection_mode(gtk::SelectionMode::None);
        let mut icons = Vec::new();
        for s in install::STEPS {
            let r = adap::ActionRow::new();
            r.set_title(s);
            let icon = gtk::Image::from_icon_name("content-loading-symbolic");
            icon.add_css_class("dim-label");
            r.add_prefix(&icon);
            steps.append(&r);
            icons.push(icon);
        }
        v.append(&steps);
        let error = adap::Banner::new("");
        error.set_revealed(false);
        error.add_css_class("installer-error-banner");
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        actions.set_halign(gtk::Align::End);
        let save = gtk::Button::with_label("Enregistrer le journal…");
        save.add_css_class("pill");
        let retry = gtk::Button::with_label("Réessayer");
        retry.add_css_class("pill");
        retry.add_css_class("suggested-action");
        let reboot = gtk::Button::with_label("Redémarrer maintenant");
        reboot.add_css_class("pill");
        reboot.add_css_class("suggested-action");
        for b in [&save, &retry, &reboot] {
            b.set_visible(false);
            actions.append(b);
        }
        let expander = gtk::Expander::new(Some("Détails de l'installation (journal)"));
        let buffer = gtk::TextBuffer::new(None);
        let view = gtk::TextView::with_buffer(&buffer);
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.add_css_class("installer-log");
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(260);
        scroll.set_child(Some(&view));
        expander.set_child(Some(&scroll));
        let err_label = gtk::Label::new(None);
        err_label.add_css_class("installer-error");
        err_label.set_wrap(true);
        err_label.set_xalign(0.0);
        err_label.set_selectable(true);
        err_label.set_visible(false);
        err_slot.append(&err_label);
        err_slot.append(&actions);
        v.append(&expander);

        let clamp = adap::Clamp::new();
        clamp.set_maximum_size(980);
        clamp.set_child(Some(&v));
        let sc = gtk::ScrolledWindow::new();
        sc.set_child(Some(&clamp));
        let name = format!("install-{}", glib::monotonic_time());
        self.stack.add_named(&sc, Some(&name));
        self.stack.set_visible_child_name(&name);

        {
            let win = self.window.clone();
            save.connect_clicked(move |_| {
                let dialog = gtk::FileDialog::builder().title("Enregistrer le journal").initial_name("noos-install.log").build();
                dialog.save(Some(&win), gio::Cancellable::NONE, move |res| {
                    if let Ok(Some(path)) = res.map(|f| f.path()) {
                        let _ = std::fs::copy(install::log_path(), path);
                    }
                });
            });
        }
        reboot.connect_clicked(|_| {
            let _ = std::process::Command::new("systemctl").arg("reboot").spawn();
        });
        {
            let weak = Rc::downgrade(self);
            retry.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.start_install();
                }
            });
        }

        let rx = install::start(self.choices.borrow().clone());
        let total = install::STEPS.len() as f64;
        let step_now = Rc::new(Cell::new(0usize));
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut lines = 0usize;
            while let Ok(ev) = rx.recv().await {
                match ev {
                    install::Event::Step(i) => {
                        for (n, icon) in icons.iter().enumerate() {
                            icon.remove_css_class("dim-label");
                            icon.remove_css_class("accent");
                            icon.set_icon_name(Some(match n.cmp(&i) {
                                std::cmp::Ordering::Less => "object-select-symbolic",
                                std::cmp::Ordering::Equal => "media-playback-start-symbolic",
                                _ => "content-loading-symbolic",
                            }));
                            if n == i {
                                icon.add_css_class("accent");
                            } else if n > i {
                                icon.add_css_class("dim-label");
                            }
                        }
                        step_now.set(i);
                        current.set_label(&format!("{}…", install::STEPS[i]));
                        overall.set_fraction(i as f64 / total);
                    }
                    install::Event::Progress(p) => overall.set_fraction((step_now.get() as f64 + p) / total),
                    install::Event::Line(l) => {
                        let mut end = buffer.end_iter();
                        buffer.insert(&mut end, &l);
                        buffer.insert(&mut end, "\n");
                        lines += 1;
                        // Le journal reste léger : on garde les 4000 dernières lignes à l'écran.
                        if lines > 4000 {
                            let mut s = buffer.start_iter();
                            let mut e = buffer.iter_at_line(500).unwrap_or(buffer.start_iter());
                            buffer.delete(&mut s, &mut e);
                            lines -= 500;
                        }
                        let mark = buffer.create_mark(None, &buffer.end_iter(), false);
                        view.scroll_mark_onscreen(&mark);
                        buffer.delete_mark(&mark);
                    }
                    install::Event::Done => {
                        for icon in &icons {
                            icon.remove_css_class("dim-label");
                            icon.remove_css_class("accent");
                            icon.set_icon_name(Some("object-select-symbolic"));
                        }
                        overall.set_fraction(1.0);
                        current.set_label("NOOS est installé !");
                        reboot.set_visible(true);
                        save.set_visible(true);
                        if let Some(ui) = weak.upgrade() {
                            ui.hint.set_label("Retirez le support d'installation puis redémarrez.");
                        }
                        return;
                    }
                    install::Event::Failed { step, error } => {
                        if let Some(icon) = icons.get(step) {
                            icon.set_icon_name(Some("dialog-error-symbolic"));
                            icon.add_css_class("error");
                        }
                        current.set_label(&format!("Échec : {}", install::STEPS[step]));
                        err_label.set_label(&explain(&error));
                        err_label.set_visible(true);
                        expander.set_expanded(true);
                        save.set_visible(true);
                        retry.set_visible(true);
                        if let Some(ui) = weak.upgrade() {
                            ui.hint.set_label("Le disque n'est peut-être plus utilisable en l'état : réessayez, ou enregistrez le journal pour obtenir de l'aide.");
                        }
                        return;
                    }
                }
            }
        });
        let _ = &self.app;
    }
}

// ───────────────────────────── Réseau ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Net {
    Wired,
    Wifi(String),
    Online,
    Offline,
}

fn connectivity() -> Net {
    let out = std::process::Command::new("nmcli").args(["-t", "-f", "TYPE,STATE,CONNECTION", "device"]).output();
    if let Ok(o) = out {
        let text = String::from_utf8_lossy(&o.stdout);
        for l in text.lines() {
            let f: Vec<&str> = l.splitn(3, ':').collect();
            if f.len() == 3 && f[1] == "connected" {
                match f[0] {
                    "ethernet" => return Net::Wired,
                    "wifi" => return Net::Wifi(f[2].to_string()),
                    _ => {}
                }
            }
        }
    }
    // Sans NetworkManager (aperçu) : test direct.
    let ok = std::process::Command::new("curl")
        .args(["-sSfI", "--max-time", "6", "-o", "/dev/null", "https://mirrors.fedoraproject.org/"])
        .status()
        .is_ok_and(|s| s.success());
    if ok { Net::Online } else { Net::Offline }
}

fn wifi_dialog(win: &adap::ApplicationWindow, done: Rc<dyn Fn()>) {
    let dialog = adap::Dialog::new();
    dialog.set_title("Réseaux Wi-Fi");
    dialog.set_content_width(480);
    dialog.set_content_height(520);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    let v = gtk::Box::new(gtk::Orientation::Vertical, 12);
    v.set_margin_top(12);
    v.set_margin_bottom(12);
    v.set_margin_start(12);
    v.set_margin_end(12);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    let spinner = gtk::Spinner::new();
    spinner.start();
    v.append(&spinner);
    let sc = gtk::ScrolledWindow::new();
    sc.set_vexpand(true);
    sc.set_child(Some(&list));
    v.append(&sc);
    let pw = adap::PasswordEntryRow::new();
    pw.set_title("Mot de passe du réseau");
    let pwg = gtk::ListBox::new();
    pwg.add_css_class("boxed-list");
    pwg.append(&pw);
    v.append(&pwg);
    let status = gtk::Label::new(None);
    status.set_wrap(true);
    v.append(&status);
    let connect = gtk::Button::with_label("Se connecter");
    connect.add_css_class("suggested-action");
    connect.add_css_class("pill");
    v.append(&connect);
    tv.set_content(Some(&v));
    dialog.set_child(Some(&tv));

    let ssids: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let (list, spinner, ssids) = (list.clone(), spinner.clone(), ssids.clone());
        glib::spawn_future_local(async move {
            let out = gio::spawn_blocking(|| {
                std::process::Command::new("nmcli")
                    .args(["-t", "-f", "SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan", "yes"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            spinner.stop();
            spinner.set_visible(false);
            let mut seen = Vec::new();
            for l in out.lines() {
                let f: Vec<&str> = l.rsplitn(3, ':').collect();
                if f.len() < 3 {
                    continue;
                }
                let (sec, signal, ssid) = (f[0], f[1], f[2].replace("\\:", ":"));
                if ssid.is_empty() || seen.contains(&ssid) {
                    continue;
                }
                seen.push(ssid.clone());
                let r = adap::ActionRow::new();
                r.set_title(&ssid);
                r.set_subtitle(&format!("Signal {signal} %{}", if sec.is_empty() || sec == "--" { " · ouvert" } else { " · sécurisé" }));
                r.set_activatable(true);
                list.append(&r);
            }
            if seen.is_empty() {
                let r = adap::ActionRow::new();
                r.set_title("Aucun réseau Wi-Fi trouvé");
                list.append(&r);
            }
            *ssids.borrow_mut() = seen;
        });
    }
    {
        let (list2, ssids, pw, status, dialog2) = (list.clone(), ssids.clone(), pw.clone(), status.clone(), dialog.clone());
        connect.connect_clicked(move |b| {
            let Some(row) = list2.selected_row() else {
                status.set_label("Choisissez un réseau dans la liste.");
                return;
            };
            let Some(ssid) = ssids.borrow().get(row.index() as usize).cloned() else { return };
            let pass = pw.text().to_string();
            b.set_sensitive(false);
            status.set_label(&format!("Connexion à « {ssid} »…"));
            let (b, status, done, dialog) = (b.clone(), status.clone(), done.clone(), dialog2.clone());
            glib::spawn_future_local(async move {
                let res = gio::spawn_blocking(move || {
                    let mut cmd = std::process::Command::new("nmcli");
                    cmd.args(["device", "wifi", "connect", &ssid]);
                    if !pass.is_empty() {
                        cmd.args(["password", &pass]);
                    }
                    cmd.output().map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).to_string()))
                })
                .await;
                b.set_sensitive(true);
                match res {
                    Ok(Ok((true, _))) => {
                        done();
                        dialog.close();
                    }
                    Ok(Ok((false, err))) => status.set_label(&format!("Échec : {}", err.trim())),
                    _ => status.set_label("nmcli introuvable."),
                }
            });
        });
    }
    dialog.present(Some(win));
}

// ───────────────────────────── Utilitaires ─────────────────────────────

fn page_box() -> gtk::Box {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 18);
    v.set_margin_top(12);
    v.set_margin_bottom(24);
    v.set_margin_start(36);
    v.set_margin_end(36);
    v
}

fn logo_picture(width: i32) -> gtk::Picture {
    let dirs = [
        std::env::var("NOOS_DATA_DIR").unwrap_or_default(),
        "/opt/noos/share/noos".into(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../data").into(),
    ];
    let path = dirs.iter().map(|d| std::path::Path::new(d).join("noos-logo.svg")).find(|p| p.is_file());
    // Texture à la bonne taille : la taille naturelle du SVG (480 px) élargirait la colonne.
    let texture = path
        .and_then(|p| gtk::gdk_pixbuf::Pixbuf::from_file_at_scale(p, width, -1, true).ok())
        .map(|pb| gdk::Texture::for_pixbuf(&pb));
    let pic = gtk::Picture::new();
    if let Some(t) = texture {
        pic.set_paintable(Some(&t));
    }
    pic.set_size_request(width, width / 3);
    pic.set_halign(gtk::Align::Center);
    pic.set_can_shrink(true);
    pic.set_content_fit(gtk::ContentFit::Contain);
    pic.add_css_class("installer-logo");
    pic
}

/// « Europe/Paris » → « Europe/Paris · UTC+01:00 · 14:03 ».
fn tz_description(tz: &str) -> String {
    match glib::DateTime::now(&glib::TimeZone::new(Some(tz))).ok() {
        Some(now) => {
            let off = now.utc_offset().0 / 1_000_000;
            let sign = if off < 0 { '-' } else { '+' };
            let off = off.abs();
            format!("{tz} · UTC{sign}{:02}:{:02} · {:02}:{:02}", off / 3600, off % 3600 / 60, now.hour(), now.minute())
        }
        None => tz.to_string(),
    }
}

/// Message d'erreur compréhensible à partir de la sortie des commandes.
fn explain(error: &str) -> String {
    let lower = error.to_lowercase();
    let hint = if lower.contains("internet") || lower.contains("could not resolve") || lower.contains("curl") {
        "Vérifiez la connexion à Internet puis réessayez."
    } else if lower.contains("no space left") {
        "Le disque est plein : choisissez un disque plus grand ou réduisez le swap."
    } else if lower.contains("device or resource busy") || lower.contains("target is busy") {
        "Le disque est utilisé (partition montée ?). Redémarrez le support d'installation puis réessayez."
    } else if lower.contains("dnf") || lower.contains("mirror") || lower.contains("curl error") {
        "Le téléchargement des paquets a échoué (miroir indisponible ?). Réessayez dans quelques instants."
    } else if lower.contains("efibootmgr") {
        "Impossible d'ajouter l'entrée de démarrage UEFI : vérifiez que le PC a démarré en mode UEFI."
    } else {
        "Consultez le journal ci-dessous pour le détail."
    };
    let first = error.lines().next().unwrap_or(error);
    format!("{first}\n\n{hint}")
}

const CSS: &str = r#"
window.noos-installer { background-color: #181825; }
.installer-side { background-color: #11111b; padding: 0 18px; }
.installer-side-title { font-weight: 700; font-size: 13px; letter-spacing: 2px; color: alpha(white, 0.55); margin-top: 6px; }
list.installer-steps { background: transparent; }
list.installer-steps > row { border-radius: 10px; padding: 10px 12px; margin: 2px 0; color: alpha(white, 0.55); background: transparent; }
list.installer-steps > row.current { background-color: alpha(@accent_bg_color, 0.22); color: white; font-weight: 700; }
list.installer-steps > row.done { color: alpha(white, 0.85); }
.installer-step-num { min-width: 24px; min-height: 24px; border-radius: 12px; background-color: alpha(white, 0.08); font-size: 12px; }
row.current .installer-step-num { background-color: @accent_bg_color; color: @accent_fg_color; }
row.done .installer-step-num { background-color: alpha(@accent_bg_color, 0.45); }
.installer-head { padding: 30px 36px 8px 36px; }
.installer-title { font-size: 30px; font-weight: 800; }
.installer-bottom { padding: 14px 36px 22px 36px; border-top: 1px solid alpha(white, 0.06); }
.installer-bottom button { min-width: 130px; min-height: 40px; }
.installer-hint { color: #f9e2af; }
.installer-card { background-color: alpha(white, 0.04); border-radius: 14px; padding: 18px; border: 1px solid alpha(white, 0.06); }
.installer-map { background-color: #1e2030; border-radius: 16px; padding: 8px; border: 1px solid alpha(white, 0.06); }
.installer-tz-info { font-weight: 700; color: #89b4fa; }
.installer-test { min-height: 42px; font-size: 15px; }
.installer-error { color: #f38ba8; }
.installer-warning { color: #f9e2af; font-weight: 700; }
.installer-log { font-size: 12px; padding: 10px; background-color: #11111b; }
progressbar.installer-progress trough, progressbar.installer-progress progress { min-height: 10px; border-radius: 5px; }
levelbar.pw-meter block { min-height: 8px; border-radius: 4px; }
levelbar.pw-meter block.empty { background-color: alpha(white, 0.08); }
levelbar.pw-very-weak block.filled { background-color: #f38ba8; }
levelbar.pw-weak block.filled { background-color: #fab387; }
levelbar.pw-fair block.filled { background-color: #f9e2af; }
levelbar.pw-strong block.filled { background-color: #a6e3a1; }
levelbar.pw-very-strong block.filled { background-color: #40c057; }
label.pw-very-weak { color: #f38ba8; }
label.pw-weak { color: #fab387; }
label.pw-fair { color: #f9e2af; }
label.pw-strong { color: #a6e3a1; }
label.pw-very-strong { color: #40c057; }
"#;

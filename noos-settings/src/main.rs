//! noos-settings — Paramètres de NOOS, organisés comme ceux de Windows 11 :
//! barre latérale (compte, recherche, catégories) et pages à fil d'Ariane avec cartes.
//!
//! Chaque modification est écrite immédiatement dans `~/.config/noos/noos.toml`, le thème
//! libAdapta « NOOS » est régénéré si besoin et le compositeur est prévenu par l'IPC.

mod apps;
mod login;
mod other;
mod personalization;
mod security;
mod system;
mod ui;

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::NoosConfig;
use ui::App;

type Builder = fn(&Rc<App>) -> adap::NavigationPage;

/// Catégories de la barre latérale : (titre, icône, classe de couleur, page).
const CATEGORIES: &[(&str, &str, &str, Builder)] = &[
    ("Système", "computer-symbolic", "nav-blue", system::root),
    ("Bluetooth et appareils", "bluetooth-symbolic", "nav-blue", other::devices),
    ("Personnalisation", "applications-graphics-symbolic", "nav-orange", personalization::root),
    ("Applications", "view-app-grid-symbolic", "nav-purple", apps::root),
    ("Jeux", "input-gaming-symbolic", "nav-green", other::gaming),
    ("Confidentialité et sécurité", "security-high-symbolic", "nav-grey", security::root),
    ("Mises à jour", "software-update-available-symbolic", "nav-teal", other::updates),
];

/// Index de recherche : (libellé, mots-clés, catégorie, sous-page éventuelle).
const SEARCH: &[(&str, &str, usize, Option<Builder>)] = &[
    ("Écran", "écran moniteur display", 0, Some(system::display)),
    ("Notifications", "notifications ne pas déranger alertes bannières", 0, Some(system::notifications)),
    ("Résolution d'écran", "résolution définition taille pixels", 0, Some(system::display)),
    ("Fréquence d'actualisation", "hz fréquence rafraîchissement 144 165", 0, Some(system::display)),
    ("Échelle", "échelle zoom dpi hidpi taille texte", 0, Some(system::display)),
    ("Orientation de l'écran", "orientation rotation portrait paysage", 0, Some(system::display)),
    ("HDR", "hdr luminosité", 0, Some(system::display)),
    ("Alimentation", "alimentation énergie batterie veille extinction", 0, Some(system::power)),
    ("Mode d'alimentation", "performances économie énergie profil", 0, Some(system::power)),
    ("Veille et écran", "veille extinction écran minutes inactivité", 0, Some(system::power)),
    ("Informations système", "à propos processeur mémoire ram version noyau nom du pc", 0, Some(system::about)),
    ("Renommer ce PC", "nom hostname renommer", 0, None),
    ("Souris", "souris pointeur vitesse défilement", 1, None),
    ("Clavier", "clavier disposition azerty qwerty", 1, None),
    ("Arrière-plan", "fond d'écran arrière-plan image wallpaper", 2, Some(personalization::background)),
    ("Couleurs", "couleur accent sombre clair transparence mode", 2, Some(personalization::colors)),
    ("Thèmes", "thème palette coins arrondis css boutons", 2, Some(personalization::themes)),
    ("Barre des tâches", "barre des tâches dock hauteur alignement épingler", 2, Some(personalization::taskbar)),
    ("Fenêtres", "fenêtres ancrage ancrer tiling snap bords coins disposition animation aperçu", 2, Some(personalization::windows)),
    ("Écran de connexion", "écran de connexion login greeter verrouillage session fond", 2, Some(login::login_screen)),
    ("Polices", "police font texte", 2, Some(personalization::fonts)),
    ("Applications par défaut", "applications par défaut ouvrir avec associations type de fichier navigateur", 3, Some(apps::default_apps)),
    ("VRR / FreeSync", "vrr freesync gsync variable jeux", 4, None),
    ("Mode Jeu", "mode jeu gaming", 4, None),
    ("Pare-feu", "pare-feu firewall ports sécurité ufw firewalld", 5, Some(security::firewall_page)),
    ("Ouvrir un port", "port ouvrir pare-feu serveur", 5, Some(security::firewall_page)),
    ("Mises à jour", "mise à jour updates logiciels", 6, None),
];

/// Pages ouvrables directement : `noos-settings firewall`, `noos-settings display`…
const PAGES: &[(&str, usize, Option<Builder>)] = &[
    ("system", 0, None),
    ("display", 0, Some(system::display)),
    ("power", 0, Some(system::power)),
    ("notifications", 0, Some(system::notifications)),
    ("about", 0, Some(system::about)),
    ("devices", 1, None),
    ("personalization", 2, None),
    ("background", 2, Some(personalization::background)),
    ("colors", 2, Some(personalization::colors)),
    ("themes", 2, Some(personalization::themes)),
    ("taskbar", 2, Some(personalization::taskbar)),
    ("windows", 2, Some(personalization::windows)),
    ("login", 2, Some(login::login_screen)),
    ("fonts", 2, Some(personalization::fonts)),
    ("apps", 3, None),
    ("default-apps", 3, Some(apps::default_apps)),
    ("gaming", 4, None),
    ("privacy", 5, None),
    ("firewall", 5, Some(security::firewall_page)),
    ("updates", 6, None),
];

thread_local! {
    static START_PAGE: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_PAGE: RefCell<Option<Box<dyn Fn(&str)>>> = const { RefCell::new(None) };
}

fn nav_row(title: &str, icon: &str, color: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let i = gtk::Image::from_icon_name(icon);
    i.add_css_class(color);
    h.append(&i);
    let l = gtk::Label::new(Some(title));
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    h.append(&l);
    row.set_child(Some(&h));
    row
}

fn user_card() -> gtk::Box {
    let real = glib::real_name().to_string_lossy().to_string();
    let name = if real.is_empty() || real == "Unknown" { glib::user_name().to_string_lossy().to_string() } else { real };
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    h.add_css_class("win11-user");
    h.append(&adap::Avatar::new(64, Some(&name), true));
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_valign(gtk::Align::Center);
    let n = gtk::Label::new(Some(&name));
    n.add_css_class("name");
    n.set_xalign(0.0);
    v.append(&n);
    let a = gtk::Label::new(Some("Compte local"));
    a.add_css_class("dim-label");
    a.add_css_class("caption");
    a.set_xalign(0.0);
    v.append(&a);
    h.append(&v);
    h
}

/// Mesure du démarrage : `NOOS_TRACE_STARTUP=1 noos-settings`.
static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
pub fn trace(step: &str) {
    if std::env::var_os("NOOS_TRACE_STARTUP").is_some() {
        eprintln!("[{:>6.0} ms] {step}", T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0);
    }
}

fn build(gapp: &adap::Application) {
    trace("build : début");
    let window = adap::ApplicationWindow::builder()
        .application(gapp)
        .title("Paramètres")
        .default_width(1180)
        .default_height(780)
        .build();
    window.add_css_class("win11");

    let toasts = adap::ToastOverlay::new();
    let nav = adap::NavigationView::new();
    let app = Rc::new(App { window: window.clone(), config: RefCell::new(NoosConfig::load()), toasts: toasts.clone(), nav: nav.clone() });

    // ── Barre latérale ──
    let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
    side.append(&user_card());
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher un paramètre"));
    search.add_css_class("win11-search");
    side.append(&search);
    let side_stack = gtk::Stack::new();
    side_stack.set_vexpand(true);
    let list = gtk::ListBox::new();
    list.add_css_class("win11-sidebar");
    list.add_css_class("navigation-sidebar");
    for (title, icon, color, _) in CATEGORIES {
        list.append(&nav_row(title, icon, color));
    }
    side_stack.add_named(&gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).build(), Some("nav"));
    let results = gtk::ListBox::new();
    results.add_css_class("win11-sidebar");
    results.add_css_class("navigation-sidebar");
    side_stack.add_named(&gtk::ScrolledWindow::builder().child(&results).hscrollbar_policy(gtk::PolicyType::Never).build(), Some("search"));
    side.append(&side_stack);

    let side_view = adap::ToolbarView::new();
    let side_header = adap::HeaderBar::new();
    side_header.set_title_widget(Some(&adap::WindowTitle::new("Paramètres", "")));
    side_view.add_top_bar(&side_header);
    side_view.set_content(Some(&side));

    toasts.set_child(Some(&nav));
    let split = adap::NavigationSplitView::new();
    split.set_sidebar(Some(&adap::NavigationPage::new(&side_view, "Paramètres")));
    split.set_content(Some(&adap::NavigationPage::new(&toasts, "Contenu")));
    split.set_min_sidebar_width(280.0);
    split.set_max_sidebar_width(340.0);
    window.set_content(Some(&split));

    {
        let (app, split) = (app.clone(), split.clone());
        list.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                let page = (CATEGORIES[row.index() as usize].3)(&app);
                app.nav.replace(&[page]);
                split.set_show_content(true);
            }
        });
    }

    // ── Recherche de paramètres ──
    let matches: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(vec![]));
    {
        let (results, side_stack, matches) = (results.clone(), side_stack.clone(), matches.clone());
        search.connect_search_changed(move |e| {
            let q = e.text().to_lowercase();
            if q.trim().is_empty() {
                side_stack.set_visible_child_name("nav");
                return;
            }
            results.remove_all();
            matches.borrow_mut().clear();
            for (i, (label, keys, cat, _)) in SEARCH.iter().enumerate() {
                if label.to_lowercase().contains(q.trim()) || keys.contains(q.trim()) {
                    let (_, icon, color, _) = CATEGORIES[*cat];
                    results.append(&nav_row(label, icon, color));
                    matches.borrow_mut().push(i);
                }
            }
            if matches.borrow().is_empty() {
                let r = gtk::ListBoxRow::new();
                r.set_activatable(false);
                let l = gtk::Label::new(Some("Aucun paramètre trouvé"));
                l.add_css_class("dim-label");
                l.set_margin_top(12);
                r.set_child(Some(&l));
                results.append(&r);
            }
            side_stack.set_visible_child_name("search");
        });
    }
    {
        let (app, list, matches, search2) = (app.clone(), list.clone(), matches.clone(), search.clone());
        results.connect_row_activated(move |_, row| {
            let Some(&i) = matches.borrow().get(row.index() as usize) else { return };
            let (_, _, cat, sub) = SEARCH[i];
            if let Some(r) = list.row_at_index(cat as i32) {
                list.select_row(Some(&r));
            }
            if let Some(sub) = sub {
                let p = sub(&app);
                app.nav.push(&p);
            }
            search2.set_text("");
        });
    }
    {
        let keys = gtk::EventControllerKey::new();
        let search = search.clone();
        keys.connect_key_pressed(move |_, key, _, mods| {
            if mods.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::f {
                search.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
    }

    // Ouverture d'une page par son nom (ligne de commande).
    {
        let (app, list) = (app.clone(), list.clone());
        let open = move |name: &str| {
            let Some(&(_, cat, sub)) = PAGES.iter().find(|p| p.0 == name) else { return };
            if let Some(r) = list.row_at_index(cat as i32) {
                list.select_row(Some(&r));
            }
            if let Some(sub) = sub {
                let p = sub(&app);
                app.nav.push(&p);
            }
        };
        OPEN_PAGE.with(|o| *o.borrow_mut() = Some(Box::new(open)));
    }
    trace("build : barre latérale");
    if let Some(first) = list.row_at_index(0) {
        list.select_row(Some(&first));
    }
    trace("build : première page");
    if let Some(page) = START_PAGE.with(|s| s.borrow_mut().take()) {
        OPEN_PAGE.with(|o| o.borrow().as_ref().map(|f| f(&page)));
    }
    window.present();
    trace("build : present");
}

/// Charge à l'avance ce qui coûte cher à la première ouverture : le thème d'icônes
/// et les icônes de la barre latérale.
fn preload() {
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
    for (_, icon, _, _) in CATEGORIES {
        let _ = theme.lookup_icon(icon, &[], 20, 1, gtk::TextDirection::None, gtk::IconLookupFlags::empty());
    }
    trace("préchargement terminé");
}

fn main() -> glib::ExitCode {
    trace("main");
    let app = adap::Application::builder()
        .application_id("org.noos.Settings")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|app| {
        trace("startup (GTK/libAdapta initialisés)");
        std::mem::forget(noos_theme::Theming::init());
        let css = gtk::CssProvider::new();
        css.load_from_string(ui::CSS);
        gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        trace("startup : thème");
        // Comme sous Windows, Paramètres reste en mémoire pour la session : fermer la
        // fenêtre la détruit, mais la rouvrir évite tout le démarrage de GTK et surtout le
        // chargement du thème d'icônes (≈ 0,75 s avec Papirus).
        std::mem::forget(app.hold());
    });
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
        } else {
            build(app);
        }
    });
    app.connect_command_line(|app, cmd| {
        let page = cmd.arguments().get(1).map(|a| a.to_string_lossy().trim_start_matches("noos-settings:").to_string());
        // Préchargement à l'ouverture de session (lancé par le shell) : pas de fenêtre.
        if page.as_deref() == Some("--preload") {
            preload();
            return glib::ExitCode::SUCCESS;
        }
        if app.active_window().is_some() {
            app.activate();
            if let Some(p) = page {
                OPEN_PAGE.with(|o| o.borrow().as_ref().map(|f| f(&p)));
            }
        } else {
            START_PAGE.with(|s| *s.borrow_mut() = page);
            app.activate();
        }
        glib::ExitCode::SUCCESS
    });
    app.run()
}

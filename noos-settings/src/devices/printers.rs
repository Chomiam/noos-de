//! « Imprimantes et scanners » (façon Windows 11) : liste des imprimantes, fiche d'une
//! imprimante (file d'attente, niveaux d'encre, page de test, par défaut, pause, partage,
//! pages Web), assistant d'ajout (détection réseau et USB, IPP Everywhere en un clic, sinon
//! choix du pilote) et scanners (SANE).

use std::{cell::RefCell, rc::Rc, time::Duration};

use adap::prelude::*;
use gtk::{gio, glib};

use super::cups::{self, Found, Printer, Service};
use crate::ui::{App, background, card, group, page as new_page};

const CRUMBS: &[&str] = &["Bluetooth et appareils"];

/// Ouvre une adresse dans le navigateur.
pub fn open_url(app: &Rc<App>, url: &str) {
    let app2 = app.clone();
    let url2 = url.to_string();
    gtk::UriLauncher::new(url).launch(Some(&app.window), gio::Cancellable::NONE, move |r| {
        if let Err(e) = r {
            app2.error("Ouvrir la page", &format!("{url2}\n\n{e}"));
        }
    });
}

fn icon_box(name: &str, class: &str, size: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class(class);
    b.set_valign(gtk::Align::Center);
    // L'image s'étend pour se centrer : sans ceci, l'extension remonterait jusqu'à la page.
    b.set_vexpand(false);
    let i = gtk::Image::from_icon_name(name);
    i.set_pixel_size(size);
    i.set_vexpand(true);
    b.append(&i);
    b
}

fn external_icon() -> String {
    for i in ["adw-external-link-symbolic", "external-link-symbolic", "window-new-symbolic"] {
        let t = super::themed(i, "");
        if !t.is_empty() {
            return t;
        }
    }
    "go-next-symbolic".into()
}

fn printer_icon(p: &Printer) -> &'static str {
    match p.connection() {
        "Réseau" | "Réseau (partagée)" => "printer-network-symbolic",
        _ => "printer-symbolic",
    }
}

fn badge(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("net-badge");
    l.add_css_class(class);
    l.set_valign(gtk::Align::Center);
    l
}

/// État vide d'un groupe : icône, titre, explication.
fn empty_state(icon: &str, title: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("dev-empty");
    let i = gtk::Image::from_icon_name(icon);
    i.set_pixel_size(40);
    i.add_css_class("dim-label");
    b.append(&i);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("heading");
    b.append(&t);
    let d = gtk::Label::new(Some(text));
    d.add_css_class("dim-label");
    d.set_wrap(true);
    d.set_justify(gtk::Justification::Center);
    b.append(&d);
    b
}

/// Exécute `f` en arrière-plan ; erreur affichée, `ok` appelée en cas de succès.
fn act<T: Send + 'static>(app: &Rc<App>, title: &str, f: impl FnOnce() -> Result<T, String> + Send + 'static, ok: impl FnOnce(T) + 'static) {
    let task = background(f);
    let (app, title) = (app.clone(), title.to_string());
    glib::spawn_future_local(async move {
        match task.await {
            Ok(v) => ok(v),
            Err(e) => app.error(&title, &e),
        }
    });
}

/// Relance `f` toutes les `secs` secondes tant que `page` existe et est affichée.
fn every(page: &adap::NavigationPage, secs: u32, f: impl Fn() + 'static) {
    let weak = page.downgrade();
    glib::timeout_add_seconds_local(secs, move || {
        let Some(p) = weak.upgrade() else { return glib::ControlFlow::Break };
        if p.is_mapped() {
            f();
        }
        glib::ControlFlow::Continue
    });
}

pub fn page(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = new_page(app, CRUMBS, "Imprimantes et scanners");

    // ── Service / installation ──
    let service_hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    service_hero.add_css_class("win11-hero");
    service_hero.append(&icon_box("printer-error-symbolic", "pr-hero-icon", 34));
    let st = gtk::Box::new(gtk::Orientation::Vertical, 4);
    st.set_hexpand(true);
    st.set_valign(gtk::Align::Center);
    let service_title = gtk::Label::new(None);
    service_title.add_css_class("title-3");
    service_title.set_xalign(0.0);
    let service_text = gtk::Label::new(None);
    service_text.add_css_class("dim-label");
    service_text.set_xalign(0.0);
    service_text.set_wrap(true);
    st.append(&service_title);
    st.append(&service_text);
    service_hero.append(&st);
    let service_btn = gtk::Button::new();
    service_btn.add_css_class("pill");
    service_btn.add_css_class("suggested-action");
    service_btn.set_valign(gtk::Align::Center);
    service_hero.append(&service_btn);
    service_hero.set_visible(false);
    content.append(&service_hero);

    // ── Ajouter ──
    let add_group = group("");
    let add_row = card("list-add-symbolic", "Ajouter une imprimante ou un scanner", "Les imprimantes Wi-Fi et USB récentes s'ajoutent en un clic, sans pilote (IPP Everywhere, AirPrint)");
    let add_btn = gtk::Button::with_label("Ajouter un appareil");
    add_btn.add_css_class("suggested-action");
    add_btn.set_valign(gtk::Align::Center);
    add_row.add_suffix(&add_btn);
    add_group.add(&add_row);
    // Compte hors du groupe lpadmin : chaque modification demandera le mot de passe.
    let lpadmin_row = card(
        "dialog-password-symbolic",
        "Administration des imprimantes",
        "Votre compte n'est pas dans le groupe « lpadmin » : chaque modification demandera le mot de passe administrateur.",
    );
    let lpadmin_btn = gtk::Button::with_label("M'ajouter au groupe");
    lpadmin_btn.set_valign(gtk::Align::Center);
    lpadmin_row.add_suffix(&lpadmin_btn);
    lpadmin_row.set_visible(false);
    add_group.add(&lpadmin_row);
    content.append(&add_group);

    // ── Imprimantes ──
    let printers_group = group("Imprimantes");
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    printers_group.add(&list);
    let loading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    loading.set_halign(gtk::Align::Center);
    loading.set_margin_top(12);
    let sp = gtk::Spinner::new();
    sp.start();
    loading.append(&sp);
    loading.append(&gtk::Label::new(Some("Lecture des imprimantes…")));
    printers_group.add(&loading);
    let no_printer = empty_state(
        "printer-symbolic",
        "Aucune imprimante installée",
        "Allumez votre imprimante, branchez-la en USB ou connectez-la au même réseau que ce PC, puis cliquez sur « Ajouter un appareil ».",
    );
    no_printer.set_visible(false);
    printers_group.add(&no_printer);
    content.append(&printers_group);

    // ── Scanners ──
    let scan_group = group("Scanners");
    let refresh_scan = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_scan.add_css_class("flat");
    refresh_scan.set_tooltip_text(Some("Rechercher de nouveau"));
    scan_group.set_header_suffix(Some(&refresh_scan));
    let scan_list = gtk::ListBox::new();
    scan_list.add_css_class("boxed-list");
    scan_list.set_selection_mode(gtk::SelectionMode::None);
    scan_group.add(&scan_list);
    content.append(&scan_group);

    // ── Paramètres ──
    let more = group("Paramètres associés");
    let cups_row = card("applications-internet-symbolic", "Interface de configuration de CUPS", "Réglages avancés du service d'impression, dans le navigateur (http://localhost:631)");
    cups_row.set_activatable(true);
    cups_row.add_suffix(&gtk::Image::from_icon_name(&external_icon()));
    {
        let app = app.clone();
        cups_row.connect_activated(move |_| open_url(&app, cups::CUPS_WEB));
    }
    more.add(&cups_row);
    content.append(&more);

    // ── Chargement ──
    let load_printers: Rc<dyn Fn()> = {
        let (app, list, loading, no_printer, printers_group, add_group, more) = (app.clone(), list.clone(), loading.clone(), no_printer.clone(), printers_group.clone(), add_group.clone(), more.clone());
        let (service_hero, service_title, service_text, service_btn, lpadmin_row, scan_group) =
            (service_hero.clone(), service_title.clone(), service_text.clone(), service_btn.clone(), lpadmin_row.clone(), scan_group.clone());
        let signature: Rc<RefCell<String>> = Rc::default();
        Rc::new(move || {
            let task = background(|| {
                let s = cups::service();
                let printers = if s == Service::Running { cups::printers() } else { vec![] };
                (s, printers, cups::in_lpadmin())
            });
            let (app, list, loading, no_printer, printers_group, add_group, more) = (app.clone(), list.clone(), loading.clone(), no_printer.clone(), printers_group.clone(), add_group.clone(), more.clone());
            let (service_hero, service_title, service_text, service_btn, lpadmin_row, scan_group, signature) =
                (service_hero.clone(), service_title.clone(), service_text.clone(), service_btn.clone(), lpadmin_row.clone(), scan_group.clone(), signature.clone());
            glib::spawn_future_local(async move {
                let (service, printers, lpadmin) = task.await;
                loading.set_visible(false);
                let running = service == Service::Running;
                service_hero.set_visible(!running);
                for w in [printers_group.upcast_ref::<gtk::Widget>(), add_group.upcast_ref(), more.upcast_ref()] {
                    w.set_visible(running);
                }
                scan_group.set_visible(service != Service::NotInstalled);
                lpadmin_row.set_visible(running && !lpadmin);
                match service {
                    Service::NotInstalled => {
                        service_title.set_text("Impression non installée");
                        service_text.set_text("Le service d'impression CUPS, la détection des imprimantes réseau et les pilotes ne sont pas installés sur ce PC.");
                        service_btn.set_label("Installer");
                    }
                    Service::Stopped => {
                        service_title.set_text("Le service d'impression est arrêté");
                        service_text.set_text("CUPS ne répond pas : vos imprimantes sont inutilisables tant qu'il n'est pas démarré.");
                        service_btn.set_label("Démarrer le service");
                    }
                    Service::Running => {}
                }
                // Pas de reconstruction si rien n'a changé (rafraîchissement périodique).
                let sig = format!("{printers:?}");
                if *signature.borrow() == sig {
                    return;
                }
                *signature.borrow_mut() = sig;
                list.remove_all();
                no_printer.set_visible(running && printers.is_empty());
                list.set_visible(!printers.is_empty());
                for pr in printers {
                    list.append(&printer_row(&app, &pr));
                }
            });
        })
    };
    load_printers();
    {
        let load = load_printers.clone();
        every(&p, 4, move || load());
    }

    let load_scanners: Rc<dyn Fn()> = {
        let (app, scan_list, refresh_scan) = (app.clone(), scan_list.clone(), refresh_scan.clone());
        Rc::new(move || {
            scan_list.remove_all();
            let r = card("", "Recherche des scanners…", "Réseau (eSCL, WSD) et USB : quelques secondes");
            let s = gtk::Spinner::new();
            s.start();
            r.add_prefix(&s);
            scan_list.append(&r);
            refresh_scan.set_sensitive(false);
            let task = background(cups::scanners);
            let (app, scan_list, refresh_scan) = (app.clone(), scan_list.clone(), refresh_scan.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                refresh_scan.set_sensitive(true);
                scan_list.remove_all();
                match res {
                    Ok(v) if v.is_empty() => {
                        scan_list.append(&card(
                            "scanner-symbolic",
                            "Aucun scanner détecté",
                            "Les imprimantes multifonctions réseau récentes sont reconnues sans pilote (eSCL / AirScan). Vérifiez que l'appareil est allumé.",
                        ));
                    }
                    Ok(v) => {
                        let scan_app = cups::scan_app();
                        for s in v {
                            let r = card("scanner-symbolic", &s.name, &s.kind);
                            r.set_tooltip_text(Some(&s.device));
                            if let Some(bin) = scan_app {
                                let b = gtk::Button::with_label("Numériser");
                                b.set_valign(gtk::Align::Center);
                                let (app, dev) = (app.clone(), s.device.clone());
                                b.connect_clicked(move |_| {
                                    if cups::demo() {
                                        app.toast("Démo : le logiciel de numérisation s'ouvrirait ici");
                                        return;
                                    }
                                    let mut cmd = std::process::Command::new(bin);
                                    if bin == "simple-scan" {
                                        cmd.arg(&dev);
                                    }
                                    if let Err(e) = cmd.spawn() {
                                        app.error("Numériser", &e.to_string());
                                    }
                                });
                                r.add_suffix(&b);
                            }
                            scan_list.append(&r);
                        }
                        if scan_app.is_none() {
                            scan_list.append(&card(
                                "dialog-information-symbolic",
                                "Aucun logiciel de numérisation",
                                "Installez « Numériseur de documents » (simple-scan) depuis Logiciels pour numériser vos documents.",
                            ));
                        }
                    }
                    Err(e) => scan_list.append(&card("dialog-warning-symbolic", "Scanners indisponibles", &e)),
                }
            });
        })
    };
    load_scanners();
    {
        let load = load_scanners.clone();
        refresh_scan.connect_clicked(move |_| load());
    }

    {
        let (app, load) = (app.clone(), load_printers.clone());
        service_btn.connect_clicked(move |b| {
            let install = b.label().is_some_and(|l| l == "Installer");
            let plan = if install {
                let mut argv = vec!["apt-get", "install", "-y"];
                argv.extend(PRINT_PACKAGES);
                noos_pkg::Plan::new().root(&argv).root(&["systemctl", "enable", "--now", "cups.service", "cups-browsed.service", "avahi-daemon.service"])
            } else {
                noos_pkg::Plan::new().root(&["systemctl", "enable", "--now", "cups.service"])
            };
            b.set_sensitive(false);
            let (b, load) = (b.clone(), load.clone());
            app.run_plan(if install { "Installer l'impression" } else { "Démarrer le service d'impression" }, Ok(plan), move |_| {
                b.set_sensitive(true);
                load();
            });
        });
    }
    {
        let app = app.clone();
        lpadmin_btn.connect_clicked(move |b| {
            let user = glib::user_name().to_string_lossy().to_string();
            let plan = noos_pkg::Plan::new().root_owned(vec!["usermod".into(), "-aG".into(), "lpadmin".into(), user]);
            let b = b.clone();
            b.set_sensitive(false);
            let app2 = app.clone();
            app.run_plan("Ajout au groupe lpadmin", Ok(plan), move |ok| {
                b.set_sensitive(!ok);
                if ok {
                    app2.toast("Ajouté au groupe lpadmin : effectif à la prochaine ouverture de session");
                }
            });
        });
    }
    {
        let (app, load) = (app.clone(), load_printers.clone());
        add_btn.connect_clicked(move |_| add_dialog(&app, load.clone()));
    }

    // Démos.
    match cups::demo_mode().as_str() {
        "add" | "driver" | "fail" | "none" => {
            let (app, load) = (app.clone(), load_printers.clone());
            glib::timeout_add_local_once(Duration::from_millis(500), move || add_dialog(&app, load));
        }
        "printer" => {
            let app = app.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                let p = printer_page(&app, "EPSON_ET-2850");
                app.push(&p);
            });
        }
        _ => {}
    }
    p
}

/// Paquets de l'impression (même liste que debian/control).
const PRINT_PACKAGES: &[&str] = &[
    "cups",
    "cups-browsed",
    "cups-filters",
    "avahi-daemon",
    "avahi-utils",
    "libnss-mdns",
    "ipp-usb",
    "printer-driver-gutenprint",
    "printer-driver-hpcups",
    "printer-driver-brlaser",
    "sane-utils",
    "sane-airscan",
];

fn printer_row(app: &Rc<App>, p: &Printer) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(p.title());
    let mut sub = vec![p.state_text(), p.connection().to_string()];
    if !p.location.is_empty() {
        sub.push(p.location.clone());
    }
    r.set_subtitle(&sub.join(" · "));
    r.set_activatable(true);
    let ib = icon_box(printer_icon(p), "bt-tile-icon", 20);
    r.add_prefix(&ib);
    if p.default {
        r.add_suffix(&badge("Par défaut", "net-vpn"));
    }
    if p.state == cups::State::Stopped {
        r.add_suffix(&badge("En pause", "net-mid"));
    } else if !p.problems().is_empty() {
        r.add_suffix(&badge("Attention", "net-mid"));
    } else if p.jobs > 0 {
        r.add_suffix(&badge(&format!("{} en attente", p.jobs), "net-ok"));
    }
    r.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    let (app, name) = (app.clone(), p.name.clone());
    r.connect_activated(move |_| {
        let pg = printer_page(&app, &name);
        app.push(&pg);
    });
    r
}

/// Couleur CSS d'un consommable (`#RRGGBB` de CUPS, sinon d'après son nom).
fn marker_css(m: &cups::Marker) -> String {
    let c = m.color.split('#').filter(|s| !s.is_empty()).next().unwrap_or("");
    if c.len() == 6 && c.chars().all(|x| x.is_ascii_hexdigit()) {
        return format!("#{c}");
    }
    let n = m.name.to_lowercase();
    if n.contains("cyan") {
        "#00b7eb".into()
    } else if n.contains("magenta") {
        "#e3007f".into()
    } else if n.contains("yellow") || n.contains("jaune") {
        "#ffd400".into()
    } else {
        "#3a3a3a".into()
    }
}

fn ink_class(m: &cups::Marker) -> &'static str {
    match marker_css(m).to_lowercase().as_str() {
        "#00ffff" | "#00b7eb" => "ink-cyan",
        "#ff00ff" | "#e3007f" => "ink-magenta",
        "#ffff00" | "#ffd400" => "ink-yellow",
        "#000000" | "#3a3a3a" => "ink-black",
        _ => "",
    }
}

fn marker_name(m: &cups::Marker) -> String {
    let n = m.name.to_lowercase();
    let color = if n.contains("cyan") {
        "Cyan"
    } else if n.contains("magenta") {
        "Magenta"
    } else if n.contains("yellow") {
        "Jaune"
    } else if n.contains("black") || n.contains("noir") {
        "Noir"
    } else {
        return m.name.clone();
    };
    let what = if m.kind.contains("toner") || n.contains("toner") { "Toner" } else { "Encre" };
    format!("{what} {}", color.to_lowercase())
}

pub fn printer_page(app: &Rc<App>, name: &str) -> adap::NavigationPage {
    let crumbs = [CRUMBS[0], "Imprimantes et scanners"];
    let initial = cups::printers().into_iter().find(|p| p.name == name);
    let title = initial.as_ref().map(|p| p.title().to_string()).unwrap_or_else(|| name.to_string());
    let (pg, content) = new_page(app, &crumbs, &title);
    let Some(pr) = initial else {
        content.append(&empty_state("printer-error-symbolic", "Imprimante introuvable", "Elle a peut-être été supprimée."));
        return pg;
    };
    let name = name.to_string();

    // ── Bandeau ──
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    hero.add_css_class("win11-hero");
    hero.append(&icon_box(printer_icon(&pr), "pr-hero-icon", 34));
    let tb = gtk::Box::new(gtk::Orientation::Vertical, 6);
    tb.set_hexpand(true);
    tb.set_valign(gtk::Align::Center);
    let tl = gtk::Label::new(Some(pr.title()));
    tl.add_css_class("title-2");
    tl.set_xalign(0.0);
    tl.set_wrap(true);
    tb.append(&tl);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let dot = gtk::Label::new(Some("●"));
    let state = gtk::Label::new(None);
    state.add_css_class("dim-label");
    line.append(&dot);
    line.append(&state);
    let default_badge = badge("Par défaut", "net-vpn");
    line.append(&default_badge);
    tb.append(&line);
    hero.append(&tb);
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 8);
    actions.set_valign(gtk::Align::Center);
    let test = gtk::Button::with_label("Imprimer une page de test");
    test.add_css_class("pill");
    let set_default = gtk::Button::with_label("Définir par défaut");
    set_default.add_css_class("pill");
    actions.append(&test);
    actions.append(&set_default);
    hero.append(&actions);
    content.append(&hero);

    let problems = gtk::ListBox::new();
    problems.add_css_class("boxed-list");
    problems.set_selection_mode(gtk::SelectionMode::None);
    content.append(&problems);

    // ── File d'attente ──
    let queue = group("File d'attente");
    let cancel_all = gtk::Button::with_label("Tout annuler");
    cancel_all.add_css_class("flat");
    queue.set_header_suffix(Some(&cancel_all));
    let jobs_list = gtk::ListBox::new();
    jobs_list.add_css_class("boxed-list");
    jobs_list.set_selection_mode(gtk::SelectionMode::None);
    queue.add(&jobs_list);
    content.append(&queue);

    // ── Encre ──
    let ink = group("Niveaux d'encre et de toner");
    ink.set_description(Some("Indiqués par l'imprimante, à titre indicatif"));
    let ink_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    ink_box.add_css_class("pr-ink");
    ink.add(&ink_box);
    content.append(&ink);

    // ── Réglages ──
    let settings = group("Paramètres");
    let pause = adap::SwitchRow::new();
    pause.set_title("Suspendre l'impression");
    pause.set_subtitle("Les documents restent dans la file d'attente jusqu'à la reprise");
    let pi = gtk::Image::from_icon_name("media-playback-pause-symbolic");
    pi.add_css_class("win11-row-icon");
    pause.add_prefix(&pi);
    settings.add(&pause);
    let share = adap::SwitchRow::new();
    share.set_title("Partager sur le réseau local");
    share.set_subtitle("Les autres ordinateurs du réseau pourront imprimer sur cette imprimante");
    let si = gtk::Image::from_icon_name("emblem-shared-symbolic");
    si.add_css_class("win11-row-icon");
    share.add_prefix(&si);
    settings.add(&share);
    content.append(&settings);

    // ── Informations ──
    let info = group("Informations");
    let info_row = |icon: &str, title: &str, value: &str| {
        let r = card(icon, title, value);
        r.set_subtitle_selectable(true);
        r
    };
    info.add(&info_row("printer-symbolic", "Modèle et pilote", if pr.model.is_empty() { "Inconnu" } else { &pr.model }));
    info.add(&info_row("network-wired-symbolic", "Connexion", &format!("{} — {}", pr.connection(), cups::percent_decode(&pr.uri))));
    if !pr.location.is_empty() {
        info.add(&info_row("mark-location-symbolic", "Emplacement", &pr.location));
    }
    info.add(&info_row("tag-symbolic", "Nom de la file CUPS", &pr.name));
    content.append(&info);

    // ── Configuration ──
    let conf = group("Configuration avancée");
    let web = card("applications-internet-symbolic", "Page Web de l'imprimante", "Recherche de son adresse…");
    web.set_activatable(false);
    web.add_suffix(&gtk::Image::from_icon_name(&external_icon()));
    conf.add(&web);
    let cups_row = card("preferences-system-symbolic", "Ouvrir dans l'interface de CUPS", "Options du pilote, bacs, recto verso, travaux terminés");
    cups_row.set_activatable(true);
    cups_row.add_suffix(&gtk::Image::from_icon_name(&external_icon()));
    {
        let (app, url) = (app.clone(), format!("{}printers/{}", cups::CUPS_WEB, name));
        cups_row.connect_activated(move |_| open_url(&app, &url));
    }
    conf.add(&cups_row);
    content.append(&conf);
    {
        let uri = pr.uri.clone();
        let task = background(move || if cups::demo() && uri.starts_with("dnssd") { Some("http://EPSON1A2B3C.local/".to_string()) } else { cups::web_page(&uri) });
        let (app, web) = (app.clone(), web.clone());
        glib::spawn_future_local(async move {
            match task.await {
                Some(url) => {
                    web.set_subtitle(&format!("Réglages, consommables et réseau de l'imprimante ({url})"));
                    web.set_activatable(true);
                    web.connect_activated(move |_| open_url(&app, &url));
                }
                None => {
                    web.set_subtitle("Indisponible : cette imprimante n'a pas de serveur Web connu (USB, ou adresse introuvable)");
                    web.set_sensitive(false);
                }
            }
        });
    }

    let danger = group("");
    let remove_row = card("user-trash-symbolic", "Supprimer l'imprimante", "Retire l'imprimante et ses documents en attente de ce PC");
    let remove = gtk::Button::with_label("Supprimer");
    remove.add_css_class("destructive-action");
    remove.set_valign(gtk::Align::Center);
    remove_row.add_suffix(&remove);
    danger.add(&remove_row);
    content.append(&danger);

    // ── Mise à jour ──
    let syncing = Rc::new(std::cell::Cell::new(false));
    let render: Rc<dyn Fn(Printer, Vec<cups::Job>)> = {
        let (app, name) = (app.clone(), name.clone());
        let (dot, state, default_badge, set_default, problems, jobs_list, cancel_all, ink, ink_box, pause, share, syncing) =
            (dot.clone(), state.clone(), default_badge.clone(), set_default.clone(), problems.clone(), jobs_list.clone(), cancel_all.clone(), ink.clone(), ink_box.clone(), pause.clone(), share.clone(), syncing.clone());
        let last: Rc<RefCell<String>> = Rc::default();
        Rc::new(move |p: Printer, jobs: Vec<cups::Job>| {
            let sig = format!("{p:?}{jobs:?}");
            if *last.borrow() == sig {
                return;
            }
            *last.borrow_mut() = sig;
            syncing.set(true);
            state.set_text(&p.state_text());
            dot.set_css_classes(&[match (&p.state, p.problems().is_empty()) {
                (cups::State::Stopped, _) => "pr-dot-paused",
                (_, false) => "pr-dot-warn",
                (cups::State::Printing, _) => "pr-dot-busy",
                _ => "pr-dot-ok",
            }]);
            default_badge.set_visible(p.default);
            set_default.set_visible(!p.default);
            pause.set_active(p.state == cups::State::Stopped);
            share.set_active(p.shared);
            // Problèmes.
            problems.remove_all();
            let probs = p.problems();
            problems.set_visible(!probs.is_empty());
            for pb in probs {
                let r = card("dialog-warning-symbolic", &pb, "");
                r.add_css_class("pr-problem");
                problems.append(&r);
            }
            // File d'attente.
            jobs_list.remove_all();
            cancel_all.set_visible(jobs.len() > 1);
            if jobs.is_empty() {
                let r = card("emblem-ok-symbolic", "Aucun document en attente", "Les documents envoyés à l'imprimante apparaîtront ici");
                jobs_list.append(&r);
            }
            for j in jobs {
                let sub = format!("{} · {} · {}", if j.active { "Impression en cours" } else { "En attente" }, crate::network::nm::human_bytes(j.size), j.owner);
                let r = card(if j.active { "document-print-symbolic" } else { "x-office-document-symbolic" }, &j.title, &sub);
                if j.active {
                    let s = gtk::Spinner::new();
                    s.start();
                    r.add_suffix(&s);
                }
                let b = gtk::Button::from_icon_name("window-close-symbolic");
                b.add_css_class("flat");
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some("Annuler l'impression"));
                let (app, name, id, title) = (app.clone(), name.clone(), j.id.clone(), j.title.clone());
                b.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let (n, i) = (name.clone(), id.clone());
                    let app2 = app.clone();
                    let t = title.clone();
                    act(&app, "Annuler l'impression", move || cups::cancel_job(&n, &i), move |_| app2.toast(&format!("« {t} » annulé")));
                });
                r.add_suffix(&b);
                jobs_list.append(&r);
            }
            // Encre.
            while let Some(c) = ink_box.first_child() {
                ink_box.remove(&c);
            }
            ink.set_visible(!p.markers.is_empty());
            for m in &p.markers {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.add_css_class("pr-ink-row");
                let sw = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                sw.add_css_class("pr-ink-swatch");
                sw.set_valign(gtk::Align::Center);
                let css = gtk::CssProvider::new();
                css.load_from_string(&format!("box {{ background-color: {}; }}", marker_css(m)));
                #[allow(deprecated)]
                sw.style_context().add_provider(&css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
                row.append(&sw);
                let n = gtk::Label::new(Some(&marker_name(m)));
                n.set_xalign(0.0);
                n.set_width_chars(16);
                row.append(&n);
                let bar = gtk::LevelBar::for_interval(0.0, 100.0);
                bar.set_hexpand(true);
                bar.set_valign(gtk::Align::Center);
                bar.set_value(m.level.max(0) as f64);
                bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
                bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
                bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
                if (0..=15).contains(&m.level) {
                    bar.add_css_class("pr-low");
                } else {
                    bar.add_css_class(ink_class(m));
                }
                row.append(&bar);
                let pct = gtk::Label::new(Some(&if m.level < 0 { "?".to_string() } else { format!("{} %", m.level) }));
                pct.add_css_class("numeric");
                pct.set_width_chars(5);
                pct.set_xalign(1.0);
                if (0..=15).contains(&m.level) {
                    pct.add_css_class("pr-low-text");
                }
                row.append(&pct);
                ink_box.append(&row);
            }
            syncing.set(false);
        })
    };
    render(pr.clone(), cups::jobs(&name));
    let reload: Rc<dyn Fn()> = {
        let (render, name) = (render.clone(), name.clone());
        Rc::new(move || {
            let n = name.clone();
            let task = background(move || (cups::printers().into_iter().find(|p| p.name == n), cups::jobs(&n)));
            let render = render.clone();
            glib::spawn_future_local(async move {
                if let (Some(p), jobs) = task.await {
                    render(p, jobs);
                }
            });
        })
    };
    {
        let reload = reload.clone();
        every(&pg, 3, move || reload());
    }

    // ── Actions ──
    {
        let (app, name, reload) = (app.clone(), name.clone(), reload.clone());
        test.connect_clicked(move |b| {
            b.set_sensitive(false);
            let n = name.clone();
            let (app2, b2, reload) = (app.clone(), b.clone(), reload.clone());
            let task = background(move || cups::test_page(&n));
            glib::spawn_future_local(async move {
                b2.set_sensitive(true);
                match task.await {
                    Ok(id) => {
                        app2.toast(&format!("Page de test envoyée{}", if id.is_empty() { String::new() } else { format!(" (travail n° {id})") }));
                        reload();
                    }
                    Err(e) => app2.error("Imprimer une page de test", &e),
                }
            });
        });
    }
    {
        let (app, name, reload) = (app.clone(), name.clone(), reload.clone());
        set_default.connect_clicked(move |_| {
            let n = name.clone();
            let (app2, reload) = (app.clone(), reload.clone());
            act(&app, "Définir par défaut", move || cups::set_default(&n), move |_| {
                app2.toast("Imprimante par défaut modifiée");
                reload();
            });
        });
    }
    {
        let (app, name, reload) = (app.clone(), name.clone(), reload.clone());
        cancel_all.connect_clicked(move |_| {
            let (app2, n, reload) = (app.clone(), name.clone(), reload.clone());
            app.confirm("Annuler tous les documents ?", "Tous les documents de la file d'attente de cette imprimante seront annulés.", "Tout annuler", true, move || {
                let n = n.clone();
                let (app3, reload) = (app2.clone(), reload.clone());
                act(&app2, "Annuler les impressions", move || cups::cancel_all(&n), move |_| {
                    app3.toast("File d'attente vidée");
                    reload();
                });
            });
        });
    }
    {
        let (app, name, reload, syncing) = (app.clone(), name.clone(), reload.clone(), syncing.clone());
        pause.connect_active_notify(move |r| {
            if syncing.get() {
                return;
            }
            let (n, on, reload) = (name.clone(), r.is_active(), reload.clone());
            act(&app, if on { "Suspendre l'impression" } else { "Reprendre l'impression" }, move || cups::set_paused(&n, on), move |_| reload());
        });
    }
    {
        let (app, name, reload, syncing) = (app.clone(), name.clone(), reload.clone(), syncing.clone());
        share.connect_active_notify(move |r| {
            if syncing.get() {
                return;
            }
            let (n, on, reload) = (name.clone(), r.is_active(), reload.clone());
            act(&app, "Partage de l'imprimante", move || cups::set_shared(&n, on), move |_| reload());
        });
    }
    {
        let (app, name, title) = (app.clone(), name.clone(), pr.title().to_string());
        remove.connect_clicked(move |_| {
            let (app2, n) = (app.clone(), name.clone());
            app.confirm(
                &format!("Supprimer « {title} » ?"),
                "L'imprimante et ses documents en attente seront supprimés de ce PC. Vous pourrez l'ajouter de nouveau plus tard.",
                "Supprimer",
                true,
                move || {
                    let n = n.clone();
                    let app3 = app2.clone();
                    act(&app2, "Supprimer l'imprimante", move || cups::remove(&n), move |_| {
                        app3.toast("Imprimante supprimée");
                        app3.nav.pop();
                    });
                },
            );
        });
    }
    pg
}

// ───────────────────────────── Assistant d'ajout ─────────────────────────────

fn centered() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 14);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_margin_start(32);
    b.set_margin_end(32);
    b.set_margin_top(20);
    b.set_margin_bottom(20);
    b
}

fn big(name: &str, class: &str) -> gtk::Box {
    let b = icon_box(name, "bt-big-icon", 44);
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.set_halign(gtk::Align::Center);
    b
}

fn text(t: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(t));
    l.set_wrap(true);
    l.set_justify(gtk::Justification::Center);
    l.set_max_width_chars(50);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

/// Adresse saisie à la main (IP, nom d'hôte ou URI complète) → appareil à ajouter.
pub fn manual_found(input: &str) -> Option<Found> {
    let t = input.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return None;
    }
    let uri = if t.contains("://") { t.to_string() } else { format!("ipp://{t}/ipp/print") };
    let host = uri.split("://").nth(1)?.split(['/', ':']).next()?.to_string();
    let mut f = Found {
        uri,
        class: "network".into(),
        model: String::new(),
        info: format!("Imprimante {host}"),
        location: String::new(),
        device_id: String::new(),
        driverless: false,
    };
    f.driverless = f.uri.starts_with("ipp") || cups::looks_driverless(&f);
    Some(f)
}

fn add_dialog(app: &Rc<App>, on_added: Rc<dyn Fn()>) {
    let dialog = adap::Dialog::new();
    dialog.set_title("Ajouter une imprimante");
    dialog.set_content_width(600);
    dialog.set_content_height(640);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    // Fond opaque : avec la transparence (Mica), la page apparaissait à travers l'assistant.
    tv.add_css_class("dev-dialog");
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    tv.set_content(Some(&stack));
    dialog.set_child(Some(&tv));

    // ── Recherche ──
    let search = gtk::Box::new(gtk::Orientation::Vertical, 14);
    search.set_margin_start(24);
    search.set_margin_end(24);
    search.set_margin_top(4);
    search.set_margin_bottom(18);
    let head = gtk::Label::new(Some("Ajouter une imprimante ou un scanner"));
    head.add_css_class("title-2");
    head.set_xalign(0.0);
    search.append(&head);
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let spinner = gtk::Spinner::new();
    state.append(&spinner);
    let state_label = gtk::Label::new(None);
    state_label.set_xalign(0.0);
    state_label.set_hexpand(true);
    state_label.set_wrap(true);
    state.append(&state_label);
    let again = gtk::Button::with_label("Rechercher de nouveau");
    again.add_css_class("flat");
    state.append(&again);
    search.append(&state);
    let found_list = gtk::ListBox::new();
    found_list.add_css_class("boxed-list");
    found_list.set_selection_mode(gtk::SelectionMode::None);
    found_list.set_valign(gtk::Align::Start);
    let nothing = empty_state(
        "printer-symbolic",
        "Aucune imprimante détectée",
        "Vérifiez qu'elle est allumée et connectée au même réseau Wi-Fi que ce PC (ou branchée en USB), puis relancez la recherche. Vous pouvez aussi saisir son adresse ci-dessous.",
    );
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.append(&found_list);
    body.append(&nothing);
    // Ajout manuel par adresse.
    let manual = group("L'imprimante n'est pas dans la liste ?");
    let addr = adap::EntryRow::new();
    addr.set_title("Adresse IP, nom d'hôte ou URI (ex. 192.168.1.42)");
    addr.set_show_apply_button(true);
    manual.add(&addr);
    body.append(&manual);
    let scroll = gtk::ScrolledWindow::builder().child(&body).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    search.append(&scroll);
    stack.add_named(&search, Some("search"));

    // ── Pilote ──
    let drv = gtk::Box::new(gtk::Orientation::Vertical, 12);
    drv.set_margin_start(24);
    drv.set_margin_end(24);
    drv.set_margin_top(4);
    drv.set_margin_bottom(18);
    let drv_title = gtk::Label::new(None);
    drv_title.add_css_class("title-3");
    drv_title.set_xalign(0.0);
    drv_title.set_wrap(true);
    drv.append(&drv_title);
    let drv_hint = gtk::Label::new(Some("Cette imprimante a besoin d'un pilote. Le pilote recommandé est présélectionné ; recherchez par marque ou modèle pour en choisir un autre."));
    drv_hint.set_wrap(true);
    drv_hint.set_xalign(0.0);
    drv_hint.add_css_class("dim-label");
    drv.append(&drv_hint);
    let drv_search = gtk::SearchEntry::new();
    drv_search.set_placeholder_text(Some("Rechercher un pilote (ex. Brother DCP, HP LaserJet, Generic PCL)"));
    drv.append(&drv_search);
    let drv_list = gtk::ListBox::new();
    drv_list.add_css_class("boxed-list");
    drv_list.set_selection_mode(gtk::SelectionMode::None);
    drv_list.set_valign(gtk::Align::Start);
    let drv_scroll = gtk::ScrolledWindow::builder().child(&drv_list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    drv.append(&drv_scroll);
    let drv_more = gtk::Label::new(Some("Pas de pilote pour votre modèle ? Installez « printer-driver-all » depuis Logiciels, ou utilisez un pilote générique (PCL ou PostScript)."));
    drv_more.set_wrap(true);
    drv_more.set_xalign(0.0);
    drv_more.add_css_class("dim-label");
    drv_more.add_css_class("caption");
    drv.append(&drv_more);
    let drv_actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    drv_actions.set_halign(gtk::Align::End);
    let drv_back = gtk::Button::with_label("Retour");
    drv_back.add_css_class("pill");
    let drv_ok = gtk::Button::with_label("Ajouter l'imprimante");
    drv_ok.add_css_class("pill");
    drv_ok.add_css_class("suggested-action");
    drv_ok.set_sensitive(false);
    drv_actions.append(&drv_back);
    drv_actions.append(&drv_ok);
    drv.append(&drv_actions);
    stack.add_named(&drv, Some("driver"));

    // ── Ajout en cours ──
    let adding = centered();
    adding.append(&big("printer-symbolic", "pulse"));
    let adding_label = text("", "title-3");
    adding.append(&adding_label);
    let asp = gtk::Spinner::new();
    asp.start();
    asp.set_size_request(24, 24);
    adding.append(&asp);
    adding.append(&text("Configuration de la file d'impression…", "dim-label"));
    stack.add_named(&adding, Some("adding"));

    // ── Terminé ──
    let done = centered();
    done.append(&big("object-select-symbolic", "success"));
    done.append(&text("Imprimante ajoutée !", "title-2"));
    let done_label = text("", "dim-label");
    done.append(&done_label);
    let make_default = gtk::CheckButton::with_label("Définir comme imprimante par défaut");
    make_default.set_halign(gtk::Align::Center);
    done.append(&make_default);
    let db = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    db.set_halign(gtk::Align::Center);
    let done_test = gtk::Button::with_label("Imprimer une page de test");
    done_test.add_css_class("pill");
    let done_ok = gtk::Button::with_label("Terminé");
    done_ok.add_css_class("pill");
    done_ok.add_css_class("suggested-action");
    db.append(&done_test);
    db.append(&done_ok);
    done.append(&db);
    stack.add_named(&done, Some("done"));

    // ── Échec ──
    let fail = centered();
    fail.append(&big("dialog-warning-symbolic", "warning"));
    fail.append(&text("Impossible d'ajouter l'imprimante", "title-2"));
    let fail_label = text("", "dim-label");
    fail.append(&fail_label);
    let fb = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    fb.set_halign(gtk::Align::Center);
    let fail_back = gtk::Button::with_label("Retour à la liste");
    fail_back.add_css_class("pill");
    let fail_driver = gtk::Button::with_label("Choisir un pilote");
    fail_driver.add_css_class("pill");
    fail_driver.add_css_class("suggested-action");
    fb.append(&fail_back);
    fb.append(&fail_driver);
    fail.append(&fb);
    stack.add_named(&fail, Some("fail"));

    // ── Logique ──
    let current: Rc<RefCell<Option<Found>>> = Rc::default();
    let added_name: Rc<RefCell<String>> = Rc::default();
    let chosen: Rc<RefCell<Option<String>>> = Rc::default();
    let all_drivers: Rc<RefCell<(Vec<cups::Driver>, Vec<cups::Driver>)>> = Rc::default();

    // Ajoute `f` (pilote None = sans pilote).
    let do_add: Rc<dyn Fn(Found, Option<String>)> = {
        let (app, stack, adding_label, done_label, fail_label, fail_driver, added_name, on_added, current) =
            (app.clone(), stack.clone(), adding_label.clone(), done_label.clone(), fail_label.clone(), fail_driver.clone(), added_name.clone(), on_added.clone(), current.clone());
        Rc::new(move |f: Found, driver: Option<String>| {
            // Mémorisé pour « Choisir un pilote » après un échec sans pilote.
            *current.borrow_mut() = Some(f.clone());
            adding_label.set_text(&format!("Ajout de « {} »", f.title()));
            stack.set_visible_child_name("adding");
            let (stack, done_label, fail_label, fail_driver, added_name, on_added, app) =
                (stack.clone(), done_label.clone(), fail_label.clone(), fail_driver.clone(), added_name.clone(), on_added.clone(), app.clone());
            let f2 = f.clone();
            let driverless = driver.is_none();
            let task = background(move || {
                let taken: Vec<String> = cups::printers().into_iter().map(|p| p.name).collect();
                let name = cups::queue_name(&f2.title(), &taken);
                cups::add(&name, &f2, driver.as_deref()).map(|_| name)
            });
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(name) => {
                        *added_name.borrow_mut() = name;
                        done_label.set_text(&format!("« {} » est prête à imprimer{}.", f.title(), if driverless { " (sans pilote, IPP Everywhere)" } else { "" }));
                        stack.set_visible_child_name("done");
                        app.toast(&format!("« {} » ajoutée", f.title()));
                        on_added();
                    }
                    Err(e) => {
                        fail_label.set_text(&e);
                        fail_driver.set_visible(driverless);
                        stack.set_visible_child_name("fail");
                    }
                }
            });
        })
    };

    let fill_drivers: Rc<dyn Fn()> = {
        let (drv_list, drv_search, all_drivers, chosen, drv_ok) = (drv_list.clone(), drv_search.clone(), all_drivers.clone(), chosen.clone(), drv_ok.clone());
        Rc::new(move || {
            drv_list.remove_all();
            let q = drv_search.text().to_lowercase();
            let (matching, all) = &*all_drivers.borrow();
            let list: Vec<&cups::Driver> = if q.trim().is_empty() {
                if matching.is_empty() { all.iter().take(60).collect() } else { matching.iter().collect() }
            } else {
                let words: Vec<&str> = q.split_whitespace().collect();
                all.iter().filter(|d| {
                    let n = d.name.to_lowercase();
                    words.iter().all(|w| n.contains(w))
                }).take(150).collect()
            };
            if list.is_empty() {
                drv_list.append(&card("dialog-information-symbolic", "Aucun pilote ne correspond", "Essayez seulement la marque, ou un pilote générique (« Generic PCL »)."));
            }
            let mut group_btn: Option<gtk::CheckButton> = None;
            for d in list {
                let r = adap::ActionRow::new();
                r.set_use_markup(false);
                let name = d.name.replace(" (recommended)", "");
                r.set_title(&name);
                r.set_subtitle(&d.id);
                let check = gtk::CheckButton::new();
                if let Some(g) = &group_btn {
                    check.set_group(Some(g));
                } else {
                    group_btn = Some(check.clone());
                }
                r.add_prefix(&check);
                r.set_activatable_widget(Some(&check));
                if d.recommended {
                    r.add_suffix(&badge("Recommandé", "net-ok"));
                }
                if chosen.borrow().as_deref() == Some(d.id.as_str()) {
                    check.set_active(true);
                }
                let (chosen, id, drv_ok) = (chosen.clone(), d.id.clone(), drv_ok.clone());
                check.connect_toggled(move |c| {
                    if c.is_active() {
                        *chosen.borrow_mut() = Some(id.clone());
                        drv_ok.set_sensitive(true);
                    }
                });
                drv_list.append(&r);
            }
        })
    };

    let choose_driver: Rc<dyn Fn(Found)> = {
        let (stack, drv_title, drv_list, all_drivers, chosen, drv_ok, fill_drivers, current, drv_search) =
            (stack.clone(), drv_title.clone(), drv_list.clone(), all_drivers.clone(), chosen.clone(), drv_ok.clone(), fill_drivers.clone(), current.clone(), drv_search.clone());
        Rc::new(move |f: Found| {
            drv_title.set_text(&format!("Choisir un pilote pour « {} »", f.title()));
            *current.borrow_mut() = Some(f.clone());
            *chosen.borrow_mut() = None;
            drv_ok.set_sensitive(false);
            drv_search.set_text("");
            drv_list.remove_all();
            let r = card("", "Chargement des pilotes…", "");
            let s = gtk::Spinner::new();
            s.start();
            r.add_prefix(&s);
            drv_list.append(&r);
            stack.set_visible_child_name("driver");
            let task = background(move || cups::drivers(&f));
            let (all_drivers, chosen, drv_ok, fill_drivers) = (all_drivers.clone(), chosen.clone(), drv_ok.clone(), fill_drivers.clone());
            glib::spawn_future_local(async move {
                let d = task.await;
                // Présélection : le premier pilote correspondant (recommandé d'abord).
                if let Some(first) = d.0.iter().find(|d| d.recommended).or(d.0.first()) {
                    *chosen.borrow_mut() = Some(first.id.clone());
                    drv_ok.set_sensitive(true);
                }
                *all_drivers.borrow_mut() = d;
                fill_drivers();
            });
        })
    };

    let scan: Rc<dyn Fn()> = {
        let (found_list, nothing, spinner, state_label, again, do_add, choose_driver) =
            (found_list.clone(), nothing.clone(), spinner.clone(), state_label.clone(), again.clone(), do_add.clone(), choose_driver.clone());
        Rc::new(move || {
            found_list.remove_all();
            found_list.set_visible(false);
            nothing.set_visible(false);
            spinner.start();
            spinner.set_visible(true);
            again.set_visible(false);
            state_label.set_text("Recherche des imprimantes sur le réseau et en USB…");
            let task = background(|| cups::discover(&cups::printers()));
            let (found_list, nothing, spinner, state_label, again, do_add, choose_driver) =
                (found_list.clone(), nothing.clone(), spinner.clone(), state_label.clone(), again.clone(), do_add.clone(), choose_driver.clone());
            glib::spawn_future_local(async move {
                let res = task.await;
                spinner.stop();
                spinner.set_visible(false);
                again.set_visible(true);
                let found = match res {
                    Ok(f) => f,
                    Err(e) => {
                        state_label.set_text(&format!("Recherche impossible : {e}"));
                        nothing.set_visible(true);
                        return;
                    }
                };
                state_label.set_text(&match found.len() {
                    0 => "Recherche terminée".to_string(),
                    1 => "1 imprimante trouvée".to_string(),
                    n => format!("{n} imprimantes trouvées"),
                });
                nothing.set_visible(found.is_empty());
                found_list.set_visible(!found.is_empty());
                for f in found {
                    let r = adap::ActionRow::new();
                    r.set_use_markup(false);
                    r.set_title(&f.title());
                    let mut sub = vec![f.connection().to_string()];
                    if !f.location.is_empty() {
                        sub.push(f.location.clone());
                    }
                    sub.push(if f.driverless { "Sans pilote (IPP Everywhere)".into() } else { "Pilote nécessaire".into() });
                    r.set_subtitle(&sub.join(" · "));
                    r.set_tooltip_text(Some(&cups::percent_decode(&f.uri)));
                    let icon = if f.class == "direct" { "printer-symbolic" } else { "printer-network-symbolic" };
                    r.add_prefix(&icon_box(icon, "bt-tile-icon", 20));
                    if f.driverless {
                        r.add_suffix(&badge("En un clic", "net-ok"));
                    }
                    let b = gtk::Button::with_label(if f.driverless { "Ajouter" } else { "Choisir un pilote…" });
                    b.set_valign(gtk::Align::Center);
                    if f.driverless {
                        b.add_css_class("suggested-action");
                    }
                    let (do_add, choose_driver) = (do_add.clone(), choose_driver.clone());
                    b.connect_clicked(move |_| if f.driverless { do_add(f.clone(), None) } else { choose_driver(f.clone()) });
                    r.add_suffix(&b);
                    found_list.append(&r);
                }
            });
        })
    };

    {
        let scan = scan.clone();
        again.connect_clicked(move |_| scan());
    }
    {
        let (do_add, app) = (do_add.clone(), app.clone());
        addr.connect_apply(move |e| match manual_found(&e.text()) {
            Some(f) => do_add(f, None),
            None => app.toast("Adresse invalide : saisissez une adresse IP, un nom d'hôte ou une URI (ipp://, socket://…)"),
        });
    }
    {
        let fill = fill_drivers.clone();
        drv_search.connect_search_changed(move |_| fill());
    }
    {
        let stack = stack.clone();
        drv_back.connect_clicked(move |_| stack.set_visible_child_name("search"));
    }
    {
        let (current, chosen, do_add) = (current.clone(), chosen.clone(), do_add.clone());
        drv_ok.connect_clicked(move |_| {
            if let (Some(f), Some(d)) = (current.borrow().clone(), chosen.borrow().clone()) {
                do_add(f, Some(d));
            }
        });
    }
    {
        let stack = stack.clone();
        fail_back.connect_clicked(move |_| stack.set_visible_child_name("search"));
    }
    {
        let (current, choose_driver) = (current.clone(), choose_driver.clone());
        fail_driver.connect_clicked(move |_| {
            if let Some(f) = current.borrow().clone() {
                choose_driver(f);
            }
        });
    }
    {
        let (app, added_name) = (app.clone(), added_name.clone());
        done_test.connect_clicked(move |b| {
            let n = added_name.borrow().clone();
            b.set_sensitive(false);
            let app2 = app.clone();
            act(&app, "Imprimer une page de test", move || cups::test_page(&n), move |_| app2.toast("Page de test envoyée"));
        });
    }
    {
        let (app, added_name, make_default, dialog, on_added) = (app.clone(), added_name.clone(), make_default.clone(), dialog.clone(), on_added.clone());
        done_ok.connect_clicked(move |_| {
            if make_default.is_active() {
                let n = added_name.borrow().clone();
                let on_added = on_added.clone();
                act(&app, "Définir par défaut", move || cups::set_default(&n), move |_| on_added());
            }
            dialog.close();
        });
    }

    stack.set_visible_child_name("search");
    dialog.present(Some(&app.window));
    scan();

    // Démos.
    match cups::demo_mode().as_str() {
        "driver" => {
            let choose_driver = choose_driver.clone();
            glib::timeout_add_local_once(Duration::from_millis(3200), move || {
                if let Some(f) = cups::discover(&[]).ok().and_then(|v| v.into_iter().find(|f| f.class == "direct")) {
                    choose_driver(f);
                }
            });
        }
        "fail" => {
            let do_add = do_add.clone();
            glib::timeout_add_local_once(Duration::from_millis(3200), move || {
                if let Some(f) = cups::discover(&[]).ok().and_then(|v| v.into_iter().find(|f| f.driverless)) {
                    do_add(f, None);
                }
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual() {
        let f = manual_found("192.168.1.42").unwrap();
        assert_eq!(f.uri, "ipp://192.168.1.42/ipp/print");
        assert!(f.driverless);
        let f = manual_found("socket://10.0.0.5:9100").unwrap();
        assert!(!f.driverless);
        assert!(manual_found("deux mots").is_none());
    }

    #[test]
    fn markers() {
        let m = cups::Marker { name: "Black Toner".into(), color: "#000000".into(), level: 50, kind: "toner".into() };
        assert_eq!(marker_name(&m), "Toner noir");
        assert_eq!(marker_css(&m), "#000000");
        let m = cups::Marker { name: "Cyan".into(), color: "".into(), level: 50, kind: "ink-tank".into() };
        assert_eq!(marker_name(&m), "Encre cyan");
        assert_eq!(marker_css(&m), "#00b7eb");
    }
}

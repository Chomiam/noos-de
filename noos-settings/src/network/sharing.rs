//! Partage de fichiers sur le réseau local : Samba (SMB), SSH et SFTP, WebDAV.
//!
//! Les réglages système passent par `noos-share-helper` (pkexec, une seule demande de mot de
//! passe pour quelques minutes) ; le WebDAV est un service utilisateur (aucun droit
//! administrateur, sauf pour ouvrir son port dans le pare-feu). `NOOS_SHARE_DEMO=1` : données
//! fictives (captures).

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gio, glib};
use noos_share::{
    client, conf,
    model::{Encrypt, Share, SystemConfig, share_name_for},
    status::{self, LocalAddr, Peer, Service},
    webdav::{Folder, WebDav},
};

use super::{badge, clear, label, spinner};
use crate::ui::{App, background, card, group, page};

type Reload = Rc<dyn Fn()>;

const CRUMBS: &[&str] = &["Réseau et Internet"];

// ───────────────────────────── Briques communes ─────────────────────────────

/// Contenu rechargé en entier après chaque modification (l'état réel fait foi).
fn reloadable<T: Send + 'static>(app: &Rc<App>, holder: &gtk::Box, load: fn() -> T, render: fn(&Rc<App>, &gtk::Box, T, Reload)) -> Reload {
    let me: Rc<std::cell::RefCell<Option<std::rc::Weak<dyn Fn()>>>> = Rc::default();
    let reload: Reload = {
        let (app, weak_holder, me) = (app.clone(), holder.downgrade(), me.clone());
        Rc::new(move || {
            let Some(holder) = weak_holder.upgrade() else { return };
            holder.set_sensitive(false);
            let (app, me) = (app.clone(), me.clone());
            glib::spawn_future_local(async move {
                let data = background(load).await;
                let Some(again) = me.borrow().as_ref().and_then(|w| w.upgrade()) else { return };
                clear(&holder);
                holder.set_sensitive(true);
                render(&app, &holder, data, again);
            });
        })
    };
    *me.borrow_mut() = Some(Rc::downgrade(&reload));
    reload
}

/// Page de partage : chargement, puis rechargement à chaque retour sur la page.
fn sharing_page<T: Send + 'static>(app: &Rc<App>, title: &str, load: fn() -> T, render: fn(&Rc<App>, &gtk::Box, T, Reload)) -> adap::NavigationPage {
    let (p, content) = page(app, CRUMBS, title);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    holder.append(&spinner());
    content.append(&holder);
    let reload = reloadable(app, &holder, load, render);
    reload();
    // La page garde le rechargement en vie (les widgets n'en tiennent qu'une copie).
    let first = std::cell::Cell::new(true);
    p.connect_showing(move |_| {
        if !first.replace(false) {
            reload();
        }
    });
    p
}

/// Opération en arrière-plan, puis rechargement. Les avertissements du script s'affichent.
fn commit(app: &Rc<App>, reload: &Reload, title: &'static str, f: impl FnOnce() -> anyhow::Result<Vec<String>> + Send + 'static) {
    let (app, reload) = (app.clone(), reload.clone());
    glib::spawn_future_local(async move {
        match background(f).await {
            Ok(w) if w.is_empty() => app.toast(&format!("{title} : enregistré")),
            Ok(w) => {
                let d = adap::AlertDialog::new(Some(&format!("{title} : enregistré, avec des réserves")), Some(&w.join("\n\n")));
                d.add_response("ok", "Compris");
                d.present(Some(&app.window));
            }
            Err(e) => app.error(title, &format!("{e:#}")),
        }
        reload();
    });
}

fn hero(icon: &str, on: bool, title: &str, subtitle: &str, switch: &gtk::Switch) -> gtk::Box {
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    h.add_css_class("win11-hero");
    h.add_css_class("net-hero");
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(40);
    img.add_css_class("net-hero-icon");
    img.add_css_class(if on { "net-ok" } else { "net-idle" });
    img.set_valign(gtk::Align::Center);
    h.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    v.append(&label(title, &["title-2"]));
    let s = label(subtitle, &["dim-label"]);
    s.set_wrap(true);
    v.append(&s);
    h.append(&v);
    switch.set_valign(gtk::Align::Center);
    switch.add_css_class("share-main-switch");
    h.append(switch);
    h
}

fn service_badge(s: &Service) -> gtk::Label {
    match (s.installed, s.active, s.failed) {
        (false, _, _) => badge("Non installé", "net-mid"),
        (_, true, _) => badge("Actif", "net-ok"),
        (_, _, true) => badge("Erreur", "net-bad"),
        _ => badge("Arrêté", "net-idle-badge"),
    }
}

/// État du pare-feu NOOS pour les règles portant ces étiquettes.
#[derive(Clone, Default)]
struct Firewall {
    running: bool,
    has_rules: bool,
}

fn firewall(tags: &[&str]) -> Firewall {
    if noos_share::demo() {
        return Firewall { running: true, has_rules: true };
    }
    let running = noos_firewall::proto::status().is_ok_and(|s| s.table_loaded && s.config.enabled);
    let has_rules = noos_firewall::proto::rules().is_ok_and(|r| r.ports.iter().any(|p| p.enabled && tags.contains(&p.note.as_str())));
    Firewall { running, has_rules }
}

/// Ligne « limité au réseau local » : protection effective selon le pare-feu.
fn lan_row(fw: &Firewall, ports: &str, fallback: &str) -> adap::ActionRow {
    let (title, sub, b) = match (fw.running, fw.has_rules) {
        (true, true) => ("Réseau local seulement", format!("Le pare-feu NOOS n'ouvre {ports} qu'aux adresses du réseau local (box, Wi-Fi de la maison). Internet n'y a pas accès."), badge("Protégé", "net-ok")),
        (true, false) => ("Port fermé par le pare-feu", format!("Aucune règle n'ouvre {ports} : les autres appareils ne pourront pas se connecter. Désactivez puis réactivez le partage pour la recréer."), badge("À vérifier", "net-mid")),
        _ => ("Pare-feu NOOS arrêté", format!("Rien ne filtre {ports}. {fallback}"), badge("Non protégé", "net-mid")),
    };
    let r = card("security-high-symbolic", title, &sub);
    r.set_subtitle_lines(4);
    r.add_suffix(&b);
    r
}

fn copy_button(app: &Rc<App>, text: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name("edit-copy-symbolic");
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some("Copier"));
    let (app, text) = (app.clone(), text.to_string());
    b.connect_clicked(move |b| {
        b.clipboard().set_text(&text);
        app.toast("Adresse copiée");
    });
    b
}

/// QR code (à scanner avec un téléphone), dessiné à la main : modules noirs sur fond blanc.
fn qr_area(text: &str, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.add_css_class("share-qr");
    let code = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium).ok();
    area.set_draw_func(move |_, cr, w, h| {
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.paint();
        let Some(code) = &code else { return };
        let n = code.size();
        let total = (n + 4) as f64; // 2 modules de marge de chaque côté
        let scale = (w.min(h) as f64 / total).floor().max(1.0);
        let off_x = ((w as f64 - scale * n as f64) / 2.0).floor();
        let off_y = ((h as f64 - scale * n as f64) / 2.0).floor();
        cr.set_source_rgb(0.0, 0.0, 0.0);
        for y in 0..n {
            for x in 0..n {
                if code.get_module(x, y) {
                    cr.rectangle(off_x + x as f64 * scale, off_y + y as f64 * scale, scale, scale);
                }
            }
        }
        let _ = cr.fill();
    });
    area
}

fn qr_button(text: &str) -> gtk::MenuButton {
    let b = gtk::MenuButton::new();
    b.set_icon_name("view-grid-symbolic");
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some("QR code (téléphone, tablette)"));
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    v.set_margin_top(14);
    v.set_margin_bottom(14);
    v.set_margin_start(14);
    v.set_margin_end(14);
    v.append(&qr_area(text, 220));
    let l = label(text, &["share-address"]);
    l.set_halign(gtk::Align::Center);
    v.append(&l);
    let hint = label("Scannez avec l'appareil photo ou le gestionnaire de fichiers du téléphone.", &["dim-label", "caption"]);
    hint.set_wrap(true);
    hint.set_max_width_chars(30);
    hint.set_justify(gtk::Justification::Center);
    hint.set_halign(gtk::Align::Center);
    v.append(&hint);
    let pop = gtk::Popover::new();
    pop.set_child(Some(&v));
    b.set_popover(Some(&pop));
    // Captures : `NOOS_SHARE_QR=1` ouvre le premier QR code de la page.
    if std::env::var_os("NOOS_SHARE_QR").is_some() && !QR_SHOWN.with(|q| q.replace(true)) {
        let b2 = b.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || b2.popup());
    }
    b
}

thread_local! {
    static QR_SHOWN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Adresse à utiliser depuis un autre appareil : copier, et QR code au besoin.
fn address_row(app: &Rc<App>, icon: &str, title: &str, value: &str, qr: bool) -> adap::ActionRow {
    let r = card(icon, title, value);
    r.add_css_class("share-address-row");
    r.set_subtitle_selectable(true);
    if qr {
        r.add_suffix(&qr_button(value));
    }
    r.add_suffix(&copy_button(app, value));
    r
}

fn ipv4(addrs: &[LocalAddr]) -> Vec<&LocalAddr> {
    addrs.iter().filter(|a| !a.v6).collect()
}

/// Appareils connectés (une ligne par adresse distante).
fn peers_group(peers: &[Peer], what: &str, user_of: impl Fn(&Peer) -> Option<String>) -> adap::PreferencesGroup {
    let g = group("Appareils connectés");
    if peers.is_empty() {
        let r = card("network-workgroup-symbolic", "Aucun appareil connecté", &format!("Les appareils qui utilisent {what} apparaîtront ici."));
        r.add_css_class("share-empty");
        g.add(&r);
        return g;
    }
    for p in peers {
        let title = p.name.clone().unwrap_or_else(|| p.address.clone());
        let mut sub = p.address.clone();
        if let Some(u) = user_of(p).or_else(|| p.user.clone()) {
            sub = format!("{sub} · compte {u}");
        }
        let r = card(if title.to_lowercase().contains("iphone") || title.to_lowercase().contains("android") { "phone-symbolic" } else { "computer-symbolic" }, &title, &sub);
        r.add_suffix(&badge("Connecté", "net-ok"));
        g.add(&r);
    }
    g
}

fn services_group(rows: &[(&str, &str, &Service)]) -> adap::PreferencesGroup {
    let g = group("Services");
    for (title, sub, s) in rows {
        let r = card("system-run-symbolic", title, &format!("{sub} · {}", s.unit));
        r.add_suffix(&service_badge(s));
        g.add(&r);
    }
    g
}

/// Lecture seule / lecture et écriture.
fn access_dropdown(writable: bool) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(&["Lecture seule", "Lecture et écriture"]);
    d.set_selected(u32::from(writable));
    d.set_valign(gtk::Align::Center);
    d.set_tooltip_text(Some("Accès depuis le réseau"));
    d
}

fn remove_button(tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name("user-trash-symbolic");
    b.add_css_class("flat");
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tooltip));
    b
}

fn pick_folder(app: &Rc<App>, f: impl Fn(String) + 'static) {
    let d = gtk::FileDialog::builder().title("Dossier à partager").initial_folder(&gio::File::for_path(glib::home_dir())).build();
    d.select_folder(Some(&app.window), None::<&gio::Cancellable>, move |res| {
        if let Ok(file) = res
            && let Some(p) = file.path()
        {
            f(p.to_string_lossy().to_string());
        }
    });
}

/// Mot de passe saisi deux fois (8 caractères au moins) ; jamais affiché ni enregistré ici.
fn password_dialog(app: &Rc<App>, title: &str, body: &str, on_ok: impl Fn(String) + 'static) {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let p1 = adap::PasswordEntryRow::builder().title("Nouveau mot de passe").build();
    let p2 = adap::PasswordEntryRow::builder().title("Confirmation").build();
    list.append(&p1);
    list.append(&p2);
    let hint = label("8 caractères au moins.", &["dim-label", "caption"]);
    hint.set_margin_top(8);
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bx.append(&list);
    bx.append(&hint);
    let d = adap::AlertDialog::new(Some(title), Some(body));
    d.set_extra_child(Some(&bx));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Enregistrer")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_response_enabled("ok", false);
    let check = {
        let (d, p1, p2, hint) = (d.clone(), p1.clone(), p2.clone(), hint.clone());
        move || {
            let (a, b) = (p1.text(), p2.text());
            let ok = a.chars().count() >= 8 && a == b;
            d.set_response_enabled("ok", ok);
            hint.set_text(if a.chars().count() < 8 {
                "8 caractères au moins."
            } else if a != b {
                "Les deux saisies sont différentes."
            } else {
                "Les deux saisies correspondent."
            });
        }
    };
    let c1 = check.clone();
    p1.connect_changed(move |_| c1());
    p2.connect_changed(move |_| check());
    let p1b = p1.clone();
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            on_ok(p1b.text().to_string());
        }
    });
    d.present(Some(&app.window));
    p1.grab_focus();
}

/// Paquets manquants : carte d'installation (apt-get, pkexec).
fn install_group(app: &Rc<App>, reload: &Reload, what: &str, why: &str, packages: &'static [&'static str]) -> adap::PreferencesGroup {
    let g = group("");
    let r = card("system-software-install-symbolic", &format!("{what} n'est pas installé"), why);
    r.set_subtitle_lines(3);
    let b = gtk::Button::with_label("Installer");
    b.add_css_class("suggested-action");
    b.set_valign(gtk::Align::Center);
    r.add_suffix(&b);
    let (app, reload) = (app.clone(), reload.clone());
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        let mut argv = vec!["env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y"];
        argv.extend_from_slice(packages);
        let reload = reload.clone();
        app.run_plan("Installation", Ok(noos_pkg::Plan::new().root(&argv)), move |_| reload());
    });
    g.add(&r);
    g
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

// ───────────────────────────── Résumé (page Réseau) ─────────────────────────────

#[derive(Clone, Default)]
pub struct Summary {
    pub samba: Option<usize>,
    pub ssh: Option<u16>,
    pub webdav: Option<u16>,
}

/// État des trois services, pour les liens de la page « Réseau et Internet ».
pub fn summary() -> Summary {
    let cfg = SystemConfig::load();
    let dav = WebDav::load();
    Summary {
        samba: (cfg.samba.enabled && status::smbd().active).then(|| cfg.samba.shares.len() + usize::from(cfg.samba.home)),
        ssh: status::sshd().active.then_some(cfg.ssh.port),
        webdav: (dav.enabled && status::webdav().active).then_some(dav.port),
    }
}

pub fn links(app: &Rc<App>, s: &Summary) -> adap::PreferencesGroup {
    let g = group("Partage de fichiers");
    let samba = match s.samba {
        Some(n) => format!("Actif · {n} dossier{} partagé{} avec Windows, Mac et Linux", plural(n), plural(n)),
        None => "Partager des dossiers avec les PC Windows, Mac et Linux du réseau".into(),
    };
    g.add(&crate::ui::link(app, "folder-remote-symbolic", "Partage de fichiers (Samba)", &samba, samba_page));
    let ssh = match s.ssh {
        Some(p) => format!("Actif · port {p}"),
        None => "Terminal à distance et transfert de fichiers chiffré".into(),
    };
    g.add(&crate::ui::link(app, "utilities-terminal-symbolic", "SSH et SFTP", &ssh, ssh_page));
    let dav = match s.webdav {
        Some(p) => format!("Actif · port {p}"),
        None => "Vos fichiers par le Web (HTTPS), depuis un téléphone ou une application".into(),
    };
    g.add(&crate::ui::link(app, "network-server-symbolic", "WebDAV", &dav, webdav_page));
    g
}

// ───────────────────────────── Samba ─────────────────────────────

struct SambaData {
    cfg: SystemConfig,
    smbd: Service,
    wsdd: Service,
    avahi: Service,
    user: String,
    password: bool,
    host: String,
    addrs: Vec<LocalAddr>,
    peers: Vec<Peer>,
    fw: Firewall,
}

fn load_samba() -> SambaData {
    let user = status::current_user();
    let mut peers = status::connections(445);
    for p in &mut peers {
        if p.name.is_none() {
            p.name = status::resolve(&p.address);
        }
    }
    SambaData {
        cfg: SystemConfig::load(),
        smbd: status::smbd(),
        wsdd: status::wsdd(),
        avahi: status::avahi(),
        password: status::samba_password_set(&user),
        user,
        host: status::hostname(),
        addrs: status::local_addresses(),
        peers,
        fw: firewall(&[conf::TAG_SAMBA]),
    }
}

pub fn samba_page(app: &Rc<App>) -> adap::NavigationPage {
    sharing_page(app, "Partage de fichiers", load_samba, render_samba)
}

fn render_samba(app: &Rc<App>, holder: &gtk::Box, d: SambaData, reload: Reload) {
    if !d.smbd.installed {
        holder.append(&install_group(app, &reload, "Samba", "Samba partage vos dossiers avec Windows, macOS, Linux et les téléphones. wsdd2 rend ce PC visible dans « Réseau » sous Windows.", &["samba", "wsdd2", "avahi-daemon"]));
        return;
    }
    let s = d.cfg.samba.clone();
    let on = s.enabled && d.smbd.active;
    let netbios = noos_share::model::netbios_name(&d.host);

    // Enregistre une modification de la configuration (pkexec).
    let save: Rc<dyn Fn(Box<dyn FnOnce(&mut SystemConfig)>)> = {
        let (app, reload, cfg) = (app.clone(), reload.clone(), d.cfg.clone());
        Rc::new(move |f| {
            let mut c = cfg.clone();
            f(&mut c);
            commit(&app, &reload, "Partage de fichiers", move || client::apply(&c));
        })
    };

    let sw = gtk::Switch::new();
    sw.set_active(s.enabled);
    let (title, sub) = match (s.enabled, on) {
        (true, true) => ("Partage activé".to_string(), format!("Ce PC apparaît sous le nom « {netbios} » dans le réseau des autres appareils.")),
        (true, false) => ("Partage en erreur".to_string(), "Le service Samba ne fonctionne pas : voir « Services » plus bas.".to_string()),
        _ => ("Partage désactivé".to_string(), "Activez le partage pour ouvrir vos dossiers depuis un autre PC, un Mac ou un téléphone.".to_string()),
    };
    holder.append(&hero("folder-remote-symbolic", on, &title, &sub, &sw));
    {
        let save = save.clone();
        sw.connect_active_notify(move |w| {
            let v = w.is_active();
            save(Box::new(move |c| c.samba.enabled = v));
        });
    }

    // Pas de mot de passe : personne ne peut se connecter (et c'est voulu).
    if s.enabled && !d.password {
        let g = group("");
        let r = card("dialog-warning-symbolic", "Aucun mot de passe de partage", "Personne ne peut encore se connecter. Choisissez le mot de passe que les autres appareils demanderont (distinct de celui de votre session).");
        r.add_css_class("share-warn");
        r.set_subtitle_lines(3);
        let b = gtk::Button::with_label("Définir…");
        b.add_css_class("suggested-action");
        b.set_valign(gtk::Align::Center);
        r.add_suffix(&b);
        let (app2, reload2) = (app.clone(), reload.clone());
        b.connect_clicked(move |_| ask_samba_password(&app2, &reload2));
        g.add(&r);
        holder.append(&g);
    }

    if s.enabled {
        let g = group("Se connecter depuis un autre appareil");
        g.set_description(Some(&format!("Nom d'utilisateur : {} — mot de passe : celui du partage, pas celui de la session.", d.user)));
        g.add(&address_row(app, "computer-symbolic", "Windows : Explorateur › barre d'adresse", &format!("\\\\{netbios}"), false));
        g.add(&address_row(app, "phone-symbolic", "macOS, Linux, Android, iPhone (app Fichiers)", &format!("smb://{}.local/", d.host), true));
        for a in ipv4(&d.addrs) {
            g.add(&address_row(app, "network-wired-symbolic", &format!("Par adresse IP ({})", a.interface), &format!("smb://{}/", a.ip), true));
        }
        holder.append(&g);
    }

    // ── Dossiers partagés ──
    let g = group("Dossiers partagés");
    g.set_description(Some("Chaque dossier n'est accessible qu'avec un compte de ce PC et son mot de passe de partage. Les droits des fichiers s'appliquent aussi."));
    let add = gtk::Button::with_label("Ajouter un dossier…");
    add.set_valign(gtk::Align::Center);
    g.set_header_suffix(Some(&add));
    {
        let (app2, save, user, shares) = (app.clone(), save.clone(), d.user.clone(), s.shares.clone());
        add.connect_clicked(move |_| {
            let (save, user, shares, app3) = (save.clone(), user.clone(), shares.clone(), app2.clone());
            pick_folder(&app2, move |path| {
                if shares.iter().any(|x| x.path == path) {
                    app3.toast("Ce dossier est déjà partagé");
                    return;
                }
                let mut name = share_name_for(&path);
                let base = name.clone();
                let mut i = 2;
                while shares.iter().any(|x| x.name.eq_ignore_ascii_case(&name)) || name.eq_ignore_ascii_case(&user) || name.eq_ignore_ascii_case("homes") {
                    name = format!("{base} {i}");
                    i += 1;
                }
                let share = Share { name, path, writable: false, owner: user.clone() };
                save(Box::new(move |c| c.samba.shares.push(share)));
            });
        });
    }
    let home = card("user-home-symbolic", "Dossier personnel", &format!("\\\\{netbios}\\{} — réservé à chaque compte, pour son propre dossier", d.user));
    let home_access = access_dropdown(s.home_writable);
    home_access.set_sensitive(s.home);
    let home_sw = gtk::Switch::new();
    home_sw.set_active(s.home);
    home_sw.set_valign(gtk::Align::Center);
    home.add_suffix(&home_access);
    home.add_suffix(&home_sw);
    {
        let save2 = save.clone();
        home_sw.connect_active_notify(move |w| {
            let v = w.is_active();
            save2(Box::new(move |c| c.samba.home = v));
        });
        let save = save.clone();
        home_access.connect_selected_notify(move |dd| {
            let v = dd.selected() == 1;
            save(Box::new(move |c| c.samba.home_writable = v));
        });
    }
    g.add(&home);
    for (i, sh) in s.shares.iter().enumerate() {
        let owner = if sh.owner == d.user { String::new() } else { format!(" · réservé à {}", sh.owner) };
        let r = card("folder-symbolic", &sh.name, &format!("{}{owner}", sh.path));
        let acc = access_dropdown(sh.writable);
        let del = remove_button("Ne plus partager");
        r.add_suffix(&acc);
        r.add_suffix(&del);
        let save2 = save.clone();
        acc.connect_selected_notify(move |dd| {
            let v = dd.selected() == 1;
            save2(Box::new(move |c| {
                if let Some(x) = c.samba.shares.get_mut(i) {
                    x.writable = v;
                }
            }));
        });
        let (save2, app2, name) = (save.clone(), app.clone(), sh.name.clone());
        del.connect_clicked(move |_| {
            let save3 = save2.clone();
            app2.confirm(
                &format!("Ne plus partager « {name} » ?"),
                "Le dossier et ses fichiers restent sur ce PC ; seuls les autres appareils n'y ont plus accès.",
                "Ne plus partager",
                true,
                move || save3(Box::new(move |c| {
                    if i < c.samba.shares.len() {
                        c.samba.shares.remove(i);
                    }
                })),
            );
        });
        g.add(&r);
    }
    if s.shares.is_empty() {
        let r = card("folder-new-symbolic", "Aucun autre dossier partagé", "Ajoutez un dossier (Photos, Jeux, Musique…) pour qu'il apparaisse sous son propre nom, en lecture seule par défaut.");
        r.add_css_class("share-empty");
        r.set_subtitle_lines(3);
        g.add(&r);
    }
    holder.append(&g);

    // ── Identité ──
    let g = group("Identité sur le réseau");
    let host = adap::EntryRow::new();
    host.set_title("Nom de ce PC");
    host.set_text(&d.host);
    host.set_show_apply_button(true);
    {
        let (app2, reload2) = (app.clone(), reload.clone());
        host.connect_apply(move |e| {
            let name = e.text().to_string();
            commit(&app2, &reload2, "Nom de ce PC", move || client::set_hostname(&name));
        });
    }
    g.add(&host);
    let wg = adap::EntryRow::new();
    wg.set_title("Groupe de travail (Windows)");
    wg.set_text(&s.workgroup);
    wg.set_show_apply_button(true);
    {
        let save = save.clone();
        wg.connect_apply(move |e| {
            let v = e.text().to_string();
            save(Box::new(move |c| c.samba.workgroup = v));
        });
    }
    g.add(&wg);
    let disc = crate::ui::switch("", "Visible dans « Réseau » sous Windows", "Annonce WSD (wsdd2). Désactivé, le PC reste joignable par son nom ou son adresse.", s.discoverable);
    {
        let save = save.clone();
        disc.connect_active_notify(move |w| {
            let v = w.is_active();
            save(Box::new(move |c| c.samba.discoverable = v));
        });
    }
    g.add(&disc);
    holder.append(&g);

    // ── Sécurité ──
    let g = group("Sécurité");
    let pw = card(
        "dialog-password-symbolic",
        "Mot de passe de partage",
        &if d.password { format!("Défini pour {} — demandé par les autres appareils", d.user) } else { "Non défini : aucune connexion possible".into() },
    );
    let set = gtk::Button::with_label(if d.password { "Modifier…" } else { "Définir…" });
    set.set_valign(gtk::Align::Center);
    pw.add_suffix(&set);
    {
        let (app2, reload2) = (app.clone(), reload.clone());
        set.connect_clicked(move |_| ask_samba_password(&app2, &reload2));
    }
    if d.password {
        let forget = gtk::Button::with_label("Retirer");
        forget.add_css_class("flat");
        forget.add_css_class("destructive-action");
        forget.set_valign(gtk::Align::Center);
        let (app2, reload2) = (app.clone(), reload.clone());
        forget.connect_clicked(move |_| {
            let (app3, reload3) = (app2.clone(), reload2.clone());
            app2.confirm("Retirer le mot de passe de partage ?", "Plus aucun appareil ne pourra ouvrir vos dossiers partagés tant qu'un nouveau mot de passe n'est pas défini.", "Retirer", true, move || {
                commit(&app3, &reload3, "Mot de passe de partage", client::remove_samba_password)
            });
        });
        pw.add_suffix(&forget);
    }
    g.add(&pw);
    let enc = crate::ui::combo(
        "channel-secure-symbolic",
        "Chiffrement des échanges",
        "SMB 3 chiffre les fichiers sur le réseau ; « toujours » refuse les appareils trop anciens.",
        &["Si possible (recommandé)".into(), "Toujours".into()],
        usize::from(s.encrypt == Encrypt::Required),
    );
    {
        let save = save.clone();
        enc.connect_selected_notify(move |c| {
            let v = if c.selected() == 1 { Encrypt::Required } else { Encrypt::Desired };
            save(Box::new(move |cfg| cfg.samba.encrypt = v));
        });
    }
    g.add(&enc);
    let guest = card("system-users-symbolic", "Accès invité : jamais", "Toute connexion demande un compte de ce PC et son mot de passe de partage. Les anciens protocoles (SMB 1, NetBIOS) sont désactivés.");
    guest.set_subtitle_lines(3);
    guest.add_suffix(&badge("Désactivé", "net-ok"));
    g.add(&guest);
    g.add(&lan_row(&d.fw, "les ports SMB (139, 445)", "Samba refuse malgré tout les adresses hors du réseau local."));
    holder.append(&g);

    if s.enabled {
        holder.append(&peers_group(&d.peers, "vos dossiers partagés", |_| None));
    }
    holder.append(&services_group(&[
        ("Serveur de fichiers", "Samba", &d.smbd),
        ("Annonce aux PC Windows", "WSD", &d.wsdd),
        ("Annonce aux Mac, Linux et téléphones", "mDNS", &d.avahi),
    ]));
}

fn ask_samba_password(app: &Rc<App>, reload: &Reload) {
    let (app2, reload2) = (app.clone(), reload.clone());
    password_dialog(
        app,
        "Mot de passe de partage",
        "Les autres appareils le demanderont avec votre nom d'utilisateur. Choisissez-le différent de celui de votre session : il circule sur le réseau à chaque connexion.",
        move |pw| commit(&app2, &reload2, "Mot de passe de partage", move || client::set_samba_password(&pw)),
    );
}

// ───────────────────────────── SSH et SFTP ─────────────────────────────

struct SshData {
    cfg: SystemConfig,
    sshd: Service,
    avahi: Service,
    user: String,
    host: String,
    addrs: Vec<LocalAddr>,
    peers: Vec<Peer>,
    who: Vec<(String, String)>,
    keys: Vec<status::AuthorizedKey>,
    host_keys: Vec<(String, String)>,
    fw: Firewall,
}

fn load_ssh() -> SshData {
    let cfg = SystemConfig::load();
    let mut peers = status::connections(cfg.ssh.port);
    for p in &mut peers {
        if p.name.is_none() {
            p.name = status::resolve(&p.address);
        }
    }
    SshData {
        sshd: status::sshd(),
        avahi: status::avahi(),
        user: status::current_user(),
        host: status::hostname(),
        addrs: status::local_addresses(),
        peers,
        who: status::ssh_users(),
        keys: status::authorized_keys(),
        host_keys: status::host_fingerprints(),
        fw: firewall(&[conf::TAG_SSH]),
        cfg,
    }
}

pub fn ssh_page(app: &Rc<App>) -> adap::NavigationPage {
    sharing_page(app, "SSH et SFTP", load_ssh, render_ssh)
}

fn render_ssh(app: &Rc<App>, holder: &gtk::Box, d: SshData, reload: Reload) {
    if !d.sshd.installed {
        holder.append(&install_group(app, &reload, "Le serveur SSH", "OpenSSH permet d'ouvrir un terminal sur ce PC depuis un autre appareil, et d'échanger des fichiers de façon chiffrée (SFTP).", &["openssh-server"]));
        return;
    }
    let s = d.cfg.ssh.clone();
    let on = s.enabled && d.sshd.active;
    let save: Rc<dyn Fn(Box<dyn FnOnce(&mut SystemConfig)>)> = {
        let (app, reload, cfg) = (app.clone(), reload.clone(), d.cfg.clone());
        Rc::new(move |f| {
            let mut c = cfg.clone();
            f(&mut c);
            commit(&app, &reload, "SSH et SFTP", move || client::apply(&c));
        })
    };
    let sw = gtk::Switch::new();
    sw.set_active(s.enabled);
    let (title, sub) = match (s.enabled, on) {
        (true, true) => ("Serveur SSH actif", "Terminal à distance (ssh) et transfert de fichiers (SFTP) avec les comptes de ce PC."),
        (true, false) => ("Serveur SSH en erreur", "Le service ne fonctionne pas : voir « Services » plus bas."),
        _ => ("Serveur SSH désactivé", "Activez-le pour ouvrir un terminal ou échanger des fichiers depuis un autre appareil, de façon chiffrée."),
    };
    holder.append(&hero("utilities-terminal-symbolic", on, title, sub, &sw));
    {
        let (save, app2, has_keys) = (save.clone(), app.clone(), !d.keys.is_empty());
        sw.connect_active_notify(move |w| {
            let v = w.is_active();
            if v && !has_keys {
                app2.toast("Conseil : ajoutez une clé, puis désactivez la connexion par mot de passe");
            }
            save(Box::new(move |c| c.ssh.enabled = v));
        });
    }

    let port_suffix = if s.port == 22 { String::new() } else { format!(" -p {}", s.port) };
    let sftp_port = if s.port == 22 { String::new() } else { format!(":{}", s.port) };
    if s.enabled {
        let g = group("Se connecter depuis un autre appareil");
        g.set_description(Some("Windows 10 et 11, macOS et Linux ont la commande ssh. Pour les fichiers : WinSCP, FileZilla, Cyberduck ou le gestionnaire de fichiers (sftp://)."));
        g.add(&address_row(app, "utilities-terminal-symbolic", "Terminal", &format!("ssh {}@{}.local{port_suffix}", d.user, d.host), false));
        g.add(&address_row(app, "folder-remote-symbolic", "Fichiers (SFTP)", &format!("sftp://{}@{}.local{sftp_port}/", d.user, d.host), true));
        for a in ipv4(&d.addrs) {
            g.add(&address_row(app, "network-wired-symbolic", &format!("Par adresse IP ({})", a.interface), &format!("ssh {}@{}{port_suffix}", d.user, a.ip), false));
        }
        holder.append(&g);
    }

    // ── Clés autorisées ──
    let g = group("Clés autorisées");
    g.set_description(Some("Une clé publique (fichier .pub) permet de se connecter à votre compte sans mot de passe, depuis l'appareil qui détient la clé privée correspondante. Fichier : ~/.ssh/authorized_keys"));
    let hb = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let import = gtk::Button::with_label("Importer un .pub…");
    let paste = gtk::Button::with_label("Coller une clé…");
    for b in [&import, &paste] {
        b.set_valign(gtk::Align::Center);
        hb.append(b);
    }
    g.set_header_suffix(Some(&hb));
    {
        let (app2, reload2) = (app.clone(), reload.clone());
        paste.connect_clicked(move |_| paste_key_dialog(&app2, &reload2));
        let (app2, reload2) = (app.clone(), reload.clone());
        import.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Clés publiques SSH (.pub)"));
            filter.add_pattern("*.pub");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let fd = gtk::FileDialog::builder().title("Clé publique à autoriser").filters(&filters).initial_folder(&gio::File::for_path(glib::home_dir().join(".ssh"))).build();
            let (app3, reload3) = (app2.clone(), reload2.clone());
            fd.open(Some(&app2.window), None::<&gio::Cancellable>, move |res| {
                if let Ok(f) = res
                    && let Some(p) = f.path()
                {
                    match std::fs::read_to_string(&p) {
                        Ok(t) => add_key(&app3, &reload3, t.lines().find(|l| !l.trim().is_empty()).unwrap_or("").to_string()),
                        Err(e) => app3.error("Clé SSH", &e.to_string()),
                    }
                }
            });
        });
    }
    if d.keys.is_empty() {
        let r = card("dialog-password-symbolic", "Aucune clé autorisée", "La connexion se fait avec le mot de passe de votre session. Sur l'autre appareil, « ssh-keygen » crée une clé ; importez ici son fichier .pub.");
        r.add_css_class("share-empty");
        r.set_subtitle_lines(3);
        g.add(&r);
    }
    for k in &d.keys {
        let title = if k.comment.is_empty() { "Clé sans nom".to_string() } else { k.comment.clone() };
        let r = card("channel-secure-symbolic", &title, &format!("{} · {}", k.kind, k.fingerprint));
        r.set_subtitle_selectable(true);
        let del = remove_button("Retirer cette clé");
        r.add_suffix(&del);
        let (app2, reload2, line, title) = (app.clone(), reload.clone(), k.line.clone(), title.clone());
        del.connect_clicked(move |_| {
            let (app3, reload3, line) = (app2.clone(), reload2.clone(), line.clone());
            app2.confirm(&format!("Retirer la clé « {title} » ?"), "L'appareil qui la détient ne pourra plus se connecter sans mot de passe.", "Retirer", true, move || {
                let line = line.clone();
                commit(&app3, &reload3, "Clés autorisées", move || remove_key_line(&line).map(|_| vec![]))
            });
        });
        g.add(&r);
    }
    holder.append(&g);

    // ── Réglages ──
    let g = group("Réglages");
    let pwd = crate::ui::switch(
        "dialog-password-symbolic",
        "Connexion par mot de passe",
        "Désactivez-la une fois vos clés ajoutées : seuls les appareils munis d'une clé pourront se connecter. C'est la protection la plus efficace.",
        s.password_auth,
    );
    pwd.set_subtitle_lines(3);
    {
        let (save, app2, no_keys) = (save.clone(), app.clone(), d.keys.is_empty());
        let guard = Rc::new(std::cell::Cell::new(false));
        pwd.connect_active_notify(move |w| {
            if guard.get() {
                return;
            }
            let v = w.is_active();
            if !v && no_keys {
                let (save, w2, guard) = (save.clone(), w.clone(), guard.clone());
                let d = adap::AlertDialog::new(
                    Some("Aucune clé autorisée"),
                    Some("Sans mot de passe ni clé, plus personne (vous compris) ne pourra se connecter en SSH à ce PC. Ajoutez d'abord une clé."),
                );
                d.add_responses(&[("cancel", "Annuler"), ("ok", "Désactiver quand même")]);
                d.set_response_appearance("ok", adap::ResponseAppearance::Destructive);
                d.set_default_response(Some("cancel"));
                d.connect_response(None, move |_, r| {
                    if r == "ok" {
                        save(Box::new(|c| c.ssh.password_auth = false));
                    } else {
                        guard.set(true);
                        w2.set_active(true);
                        guard.set(false);
                    }
                });
                d.present(Some(&app2.window));
                return;
            }
            save(Box::new(move |c| c.ssh.password_auth = v));
        });
    }
    g.add(&pwd);
    let port = crate::ui::spin("network-transmit-receive-symbolic", "Port", "22 par défaut. Un autre port évite le bruit des robots, sans remplacer les clés.", (1.0, 65535.0, 1.0), s.port as f64);
    let apply = gtk::Button::with_label("Appliquer");
    apply.set_valign(gtk::Align::Center);
    apply.set_visible(false);
    port.add_suffix(&apply);
    {
        let (apply2, current) = (apply.clone(), s.port);
        port.connect_value_notify(move |p| apply2.set_visible(p.value() as u16 != current));
        let (save, port2) = (save.clone(), port.clone());
        apply.connect_clicked(move |_| {
            let v = port2.value() as u16;
            save(Box::new(move |c| c.ssh.port = v));
        });
    }
    g.add(&port);
    let root = card("system-lock-screen-symbolic", "Connexion du compte root : interdite", "Les administrateurs se connectent avec leur compte puis utilisent sudo.");
    root.add_suffix(&badge("Protégé", "net-ok"));
    g.add(&root);
    g.add(&lan_row(&d.fw, &format!("le port {}", s.port), "Le serveur SSH répondrait aussi depuis Internet si la box redirigeait ce port."));
    holder.append(&g);

    if !d.host_keys.is_empty() {
        let g = group("Empreintes de ce PC");
        g.set_description(Some("À la première connexion, l'autre appareil affiche une empreinte et demande confirmation : elle doit correspondre à l'une de celles-ci. Sinon, refusez."));
        for (kind, fp) in &d.host_keys {
            let r = address_row(app, "auth-fingerprint-symbolic", kind, fp, false);
            g.add(&r);
        }
        holder.append(&g);
    }

    if s.enabled {
        let who = d.who.clone();
        holder.append(&peers_group(&d.peers, "SSH ou SFTP", move |p| who.iter().find(|(_, h)| *h == p.address || p.name.as_deref() == Some(h.as_str())).map(|(u, _)| u.clone())));
    }
    holder.append(&services_group(&[("Serveur SSH", "OpenSSH", &d.sshd), ("Annonce sur le réseau local", "mDNS", &d.avahi)]));
}

fn paste_key_dialog(app: &Rc<App>, reload: &Reload) {
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::Char);
    view.add_css_class("share-key-input");
    view.set_monospace(true);
    let scroll = gtk::ScrolledWindow::builder().child(&view).min_content_height(110).min_content_width(380).build();
    scroll.add_css_class("card");
    let d = adap::AlertDialog::new(Some("Coller une clé publique"), Some("Contenu du fichier .pub de l'autre appareil (par exemple ~/.ssh/id_ed25519.pub). Jamais la clé privée."));
    d.set_extra_child(Some(&scroll));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Autoriser")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    let (app2, reload2) = (app.clone(), reload.clone());
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            let b = view.buffer();
            add_key(&app2, &reload2, b.text(&b.start_iter(), &b.end_iter(), false).to_string());
        }
    });
    d.present(Some(&app.window));
}

fn add_key(app: &Rc<App>, reload: &Reload, text: String) {
    let line = match status::validate_public_key(&text) {
        Ok(l) => l,
        Err(e) => return app.error("Clé SSH", &format!("{e:#}")),
    };
    commit(app, reload, "Clés autorisées", move || append_key_line(&line).map(|_| vec![]));
}

/// `~/.ssh` en 0700, `authorized_keys` en 0600 (sinon sshd l'ignore).
fn append_key_line(line: &str) -> anyhow::Result<()> {
    if noos_share::demo() {
        return Ok(());
    }
    let path = status::authorized_keys_path();
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    if old.lines().any(|l| l.trim() == line.trim()) {
        anyhow::bail!("cette clé est déjà autorisée");
    }
    let mut new = old;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(line.trim());
    new.push('\n');
    noos_share::webdav::write_private(&path, &new)
}

fn remove_key_line(line: &str) -> anyhow::Result<()> {
    if noos_share::demo() {
        return Ok(());
    }
    let path = status::authorized_keys_path();
    let old = std::fs::read_to_string(&path)?;
    let new: String = old.lines().filter(|l| *l != line).map(|l| format!("{l}\n")).collect();
    noos_share::webdav::write_private(&path, &new)
}

// ───────────────────────────── WebDAV ─────────────────────────────

struct DavData {
    cfg: WebDav,
    svc: Service,
    avahi: Service,
    host: String,
    addrs: Vec<LocalAddr>,
    peers: Vec<Peer>,
    password: bool,
    fingerprint: Option<(String, String)>,
    fw: Firewall,
}

fn load_dav() -> DavData {
    let cfg = WebDav::load();
    let mut peers = if cfg.enabled { status::connections(cfg.port) } else { vec![] };
    for p in &mut peers {
        if p.name.is_none() {
            p.name = status::resolve(&p.address);
        }
    }
    // Étiquette posée par le script privilégié : nom du compte (pas le nom WebDAV, modifiable).
    let tag = conf::tag_webdav(&status::current_user());
    DavData {
        svc: status::webdav(),
        avahi: status::avahi(),
        host: status::hostname(),
        addrs: status::local_addresses(),
        peers,
        password: cfg.password_set(),
        fingerprint: noos_share::webdav::cert_fingerprints(),
        fw: firewall(&[tag.as_str()]),
        cfg,
    }
}

pub fn webdav_page(app: &Rc<App>) -> adap::NavigationPage {
    sharing_page(app, "WebDAV", load_dav, render_dav)
}

fn systemctl_user(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new("systemctl").arg("--user").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("systemctl --user {} : {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// Enregistre la configuration WebDAV et met le service (utilisateur) et le pare-feu à jour.
fn dav_apply(new: WebDav, old: WebDav) -> anyhow::Result<Vec<String>> {
    new.save()?;
    if noos_share::demo() {
        std::thread::sleep(std::time::Duration::from_millis(400));
        return Ok(vec![]);
    }
    let mut warnings = vec![];
    if new.enabled {
        if !new.password_set() {
            anyhow::bail!("définissez d'abord un mot de passe");
        }
        systemctl_user(&["enable", "noos-webdav.service"])?;
        systemctl_user(&["restart", "noos-webdav.service"])?;
        if !old.enabled || old.port != new.port || old.https != new.https {
            warnings.extend(client::webdav_system(Some(new.port), new.https)?);
        }
        // rclone s'arrête aussitôt sur une erreur (port pris, dossier absent…) : on la remonte.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if !status::webdav().active {
            let log = std::process::Command::new("journalctl").args(["--user", "-u", "noos-webdav", "-n", "4", "-o", "cat", "--no-pager"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
            anyhow::bail!("le serveur ne démarre pas :\n{}", log.trim());
        }
    } else {
        let _ = systemctl_user(&["disable", "--now", "noos-webdav.service"]);
        if old.enabled {
            warnings.extend(client::webdav_system(None, false)?);
        }
    }
    Ok(warnings)
}

fn render_dav(app: &Rc<App>, holder: &gtk::Box, d: DavData, reload: Reload) {
    if !d.svc.installed {
        holder.append(&install_group(app, &reload, "rclone", "NOOS utilise « rclone serve webdav » : un serveur léger, lancé avec vos droits, qui ne voit que vos fichiers.", &["rclone"]));
        return;
    }
    let c = d.cfg.clone();
    let on = c.enabled && d.svc.active;
    let save: Rc<dyn Fn(Box<dyn FnOnce(&mut WebDav)>)> = {
        let (app, reload, cfg) = (app.clone(), reload.clone(), d.cfg.clone());
        Rc::new(move |f| {
            let mut n = cfg.clone();
            f(&mut n);
            let old = cfg.clone();
            commit(&app, &reload, "WebDAV", move || dav_apply(n, old));
        })
    };
    let sw = gtk::Switch::new();
    sw.set_active(c.enabled);
    let (title, sub) = match (c.enabled, on) {
        (true, true) => ("Serveur WebDAV actif", format!("Vos dossiers sont accessibles en {} sur le port {}.", if c.https { "HTTPS" } else { "HTTP" }, c.port)),
        (true, false) => ("Serveur WebDAV en erreur", "Le service ne fonctionne pas : voir « Services » plus bas.".to_string()),
        _ => ("Serveur WebDAV désactivé", "Accès à vos fichiers par le Web : applications mobiles, gestionnaires de fichiers, outils de synchronisation.".to_string()),
    };
    holder.append(&hero("network-server-symbolic", on, title, &sub, &sw));
    {
        let (save, app2, reload2, has_pw) = (save.clone(), app.clone(), reload.clone(), d.password);
        let guard = Rc::new(std::cell::Cell::new(false));
        sw.connect_active_notify(move |w| {
            if guard.get() {
                return;
            }
            let v = w.is_active();
            if v && !has_pw {
                // Le serveur ne démarre jamais sans mot de passe : on le demande d'abord.
                guard.set(true);
                w.set_active(false);
                guard.set(false);
                let (save, app3, reload3) = (save.clone(), app2.clone(), reload2.clone());
                password_dialog(&app2, "Mot de passe WebDAV", "Demandé par les autres appareils avec votre nom d'utilisateur. Il est enregistré haché (bcrypt), jamais en clair.", move |pw| {
                    let (save, app4, reload4) = (save.clone(), app3.clone(), reload3.clone());
                    glib::spawn_future_local(async move {
                        match background(move || WebDav::load().set_password(&pw)).await {
                            Ok(()) => save(Box::new(|c| c.enabled = true)),
                            Err(e) => {
                                app4.error("Mot de passe WebDAV", &format!("{e:#}"));
                                reload4();
                            }
                        }
                    });
                });
                return;
            }
            save(Box::new(move |c| c.enabled = v));
        });
    }

    let scheme = if c.https { "davs" } else { "dav" };
    let web = if c.https { "https" } else { "http" };
    if c.enabled {
        let g = group("Se connecter depuis un autre appareil");
        g.set_description(Some(&format!("Nom d'utilisateur : {} — mot de passe : celui du WebDAV.", c.user)));
        g.add(&address_row(app, "folder-remote-symbolic", "Gestionnaire de fichiers (Linux, applications mobiles)", &format!("{scheme}://{}.local:{}/", d.host, c.port), true));
        for a in ipv4(&d.addrs) {
            g.add(&address_row(app, "network-wired-symbolic", &format!("Par adresse IP ({})", a.interface), &format!("{web}://{}:{}/", a.ip, c.port), true));
            if c.https {
                g.add(&address_row(app, "computer-symbolic", "Windows : Explorateur › Ajouter un emplacement réseau", &format!("\\\\{}@SSL@{}\\DavWWWRoot", a.ip, c.port), false));
            }
        }
        holder.append(&g);
    }

    // ── Dossiers ──
    let g = group("Dossiers partagés");
    g.set_description(Some("Chaque dossier apparaît à la racine du serveur. Le serveur fonctionne avec vos droits : il ne voit que ce que vous voyez."));
    let add = gtk::Button::with_label("Ajouter un dossier…");
    add.set_valign(gtk::Align::Center);
    g.set_header_suffix(Some(&add));
    {
        let (app2, save, folders) = (app.clone(), save.clone(), c.folders.clone());
        add.connect_clicked(move |_| {
            let (save, folders, app3) = (save.clone(), folders.clone(), app2.clone());
            pick_folder(&app2, move |path| {
                if folders.iter().any(|f| f.path == path) {
                    app3.toast("Ce dossier est déjà partagé");
                    return;
                }
                let mut name = share_name_for(&path);
                let base = name.clone();
                let mut i = 2;
                while folders.iter().any(|f| f.name.eq_ignore_ascii_case(&name)) {
                    name = format!("{base} {i}");
                    i += 1;
                }
                let f = Folder { name, path, writable: false };
                save(Box::new(move |c| c.folders.push(f)));
            });
        });
    }
    if c.folders.is_empty() {
        let r = card("folder-new-symbolic", "Aucun dossier partagé", "Ajoutez au moins un dossier : sans dossier, le serveur ne démarre pas.");
        r.add_css_class("share-empty");
        g.add(&r);
    }
    let home = glib::home_dir().to_string_lossy().to_string();
    for (i, f) in c.folders.iter().enumerate() {
        let icon = if f.path == home { "user-home-symbolic" } else { "folder-symbolic" };
        let r = card(icon, &f.name, &f.path);
        let acc = access_dropdown(f.writable);
        let del = remove_button("Ne plus partager");
        r.add_suffix(&acc);
        r.add_suffix(&del);
        let save2 = save.clone();
        acc.connect_selected_notify(move |dd| {
            let v = dd.selected() == 1;
            save2(Box::new(move |c| {
                if let Some(x) = c.folders.get_mut(i) {
                    x.writable = v;
                }
            }));
        });
        let save2 = save.clone();
        del.connect_clicked(move |_| {
            save2(Box::new(move |c| {
                if i < c.folders.len() {
                    c.folders.remove(i);
                }
            }))
        });
        g.add(&r);
    }
    holder.append(&g);

    // ── Connexion et sécurité ──
    let g = group("Connexion et sécurité");
    let user = adap::EntryRow::new();
    user.set_title("Nom d'utilisateur (le changer demande un nouveau mot de passe)");
    user.set_text(&c.user);
    user.set_show_apply_button(true);
    {
        let save = save.clone();
        user.connect_apply(move |e| {
            let v = e.text().to_string();
            save(Box::new(move |c| c.user = v));
        });
    }
    g.add(&user);
    let pw = card("dialog-password-symbolic", "Mot de passe", if d.password { "Défini (enregistré haché, jamais en clair)" } else { "Non défini : le serveur ne peut pas démarrer" });
    let set = gtk::Button::with_label(if d.password { "Modifier…" } else { "Définir…" });
    set.set_valign(gtk::Align::Center);
    pw.add_suffix(&set);
    {
        let (app2, save) = (app.clone(), save.clone());
        set.connect_clicked(move |_| {
            let (save, app3) = (save.clone(), app2.clone());
            password_dialog(&app2, "Mot de passe WebDAV", "Demandé par les autres appareils avec le nom d'utilisateur. Il est enregistré haché (bcrypt).", move |pw| {
                let (save, app4) = (save.clone(), app3.clone());
                glib::spawn_future_local(async move {
                    match background(move || WebDav::load().set_password(&pw)).await {
                        // Redémarre le serveur (nouveau mot de passe pris en compte).
                        Ok(()) => save(Box::new(|_| {})),
                        Err(e) => app4.error("Mot de passe WebDAV", &format!("{e:#}")),
                    }
                });
            });
        });
    }
    g.add(&pw);
    let https = crate::ui::switch("channel-secure-symbolic", "HTTPS (chiffré)", "Certificat créé sur ce PC. Les appareils le signalent comme inconnu la première fois : comparez son empreinte, ci-dessous.", c.https);
    https.set_subtitle_lines(3);
    {
        let save = save.clone();
        https.connect_active_notify(move |w| {
            let v = w.is_active();
            save(Box::new(move |c| c.https = v));
        });
    }
    g.add(&https);
    if !c.https {
        let r = card("dialog-warning-symbolic", "Échanges non chiffrés", "Sans HTTPS, le mot de passe et les fichiers circulent en clair : n'importe quel appareil du réseau peut les lire. Windows refuse d'ailleurs ce mode.");
        r.add_css_class("share-warn");
        r.set_subtitle_lines(3);
        g.add(&r);
    } else if let Some((sha256, sha1)) = &d.fingerprint {
        let r = address_row(app, "auth-fingerprint-symbolic", "Empreinte du certificat (SHA-256 : navigateurs, Windows)", sha256, false);
        r.set_subtitle_lines(2);
        g.add(&r);
        g.add(&address_row(app, "auth-fingerprint-symbolic", "Empreinte SHA-1 (Explorateur de NOOS, GNOME)", sha1, false));
    }
    let port = crate::ui::spin("network-transmit-receive-symbolic", "Port", "8443 par défaut (1024 au moins : le serveur n'a pas de droits administrateur).", (1024.0, 65535.0, 1.0), c.port as f64);
    let apply = gtk::Button::with_label("Appliquer");
    apply.set_valign(gtk::Align::Center);
    apply.set_visible(false);
    port.add_suffix(&apply);
    {
        let (apply2, current) = (apply.clone(), c.port);
        port.connect_value_notify(move |p| apply2.set_visible(p.value() as u16 != current));
        let (save, port2) = (save.clone(), port.clone());
        apply.connect_clicked(move |_| {
            let v = port2.value() as u16;
            save(Box::new(move |c| c.port = v));
        });
    }
    g.add(&port);
    g.add(&lan_row(&d.fw, &format!("le port {}", c.port), "Le serveur répondrait à toute adresse qui atteint ce PC."));
    holder.append(&g);

    if c.enabled {
        holder.append(&peers_group(&d.peers, "le WebDAV", |_| None));
    }
    holder.append(&services_group(&[("Serveur WebDAV (votre session)", "rclone", &d.svc), ("Annonce sur le réseau local", "mDNS", &d.avahi)]));
}

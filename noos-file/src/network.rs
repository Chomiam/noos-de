//! Page « Réseau » (comme le Réseau de l'Explorateur de Windows 11) : appareils découverts sur
//! le réseau local (gvfs `network:///` : WSD et partages Windows, mDNS / DNS-SD pour SFTP,
//! WebDAV, Samba des Mac et Linux), partages ouverts, serveurs récents, et « Se connecter à un
//! serveur » (smb://, sftp://, davs://, ftp://, ou `\\PC\partage` façon Windows).
//!
//! Ouvrir un emplacement distant le monte d'abord (gvfs) avec une [`gio::MountOperation`] dont
//! la fenêtre d'identifiants propose de retenir le mot de passe dans le trousseau
//! (`PasswordSave::Permanently` : gvfs l'enregistre via libsecret / gnome-keyring).
//!
//! `NOOS_FILE_NET_DEMO=1` : appareils, partages et serveurs fictifs (captures).

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};

pub const NETWORK_URI: &str = "network:///";

/// Protocoles ouverts par gvfs (montage nécessaire avant de lister).
const REMOTE: &[&str] = &["smb", "sftp", "ssh", "dav", "davs", "ftp", "ftps", "afp", "nfs", "dnssd", "network"];

pub fn is_network(f: &gio::File) -> bool {
    f.uri_scheme().as_deref() == Some("network") && f.parent().is_none()
}

/// Emplacement distant (pas un fichier local, ni l'accueil, ni la corbeille).
pub fn is_remote(f: &gio::File) -> bool {
    f.uri_scheme().is_some_and(|s| REMOTE.contains(&s.as_str()))
}

thread_local! {
    /// Navigation qui suit un montage réussi : pas de nouvelle vérification.
    static MOUNTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Emplacement distant à monter avant de l'afficher.
pub fn needs_mount(f: &gio::File) -> bool {
    !MOUNTED.with(|m| m.get()) && is_remote(f) && !is_network(f) && !demo() && f.find_enclosing_mount(gio::Cancellable::NONE).is_err()
}

fn navigate_mounted(f: gio::File) {
    if let Some(ex) = crate::explorer() {
        MOUNTED.with(|m| m.set(true));
        ex.navigate(f, true);
        MOUNTED.with(|m| m.set(false));
    }
}

fn demo() -> bool {
    std::env::var_os("NOOS_FILE_NET_DEMO").is_some_and(|v| !v.is_empty() && v != "0")
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("home-section");
    l.set_xalign(0.0);
    l
}

fn flow() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_min_children_per_line(1);
    f.set_max_children_per_line(4);
    f.set_column_spacing(10);
    f.set_row_spacing(10);
    f.set_homogeneous(true);
    f
}

/// Libellé du protocole d'un emplacement.
pub fn kind_of(uri: &str) -> &'static str {
    let scheme = uri.split(':').next().unwrap_or("");
    match scheme {
        "smb" => "Partage Windows (SMB)",
        "sftp" | "ssh" => "SFTP (SSH)",
        "dav" => "WebDAV",
        "davs" => "WebDAV sécurisé",
        "ftp" | "ftps" => "FTP",
        "afp" => "Partage Mac (AFP)",
        "nfs" => "NFS",
        "dnssd" if uri.contains("_sftp-ssh") || uri.contains("_ssh") => "SFTP (SSH)",
        "dnssd" if uri.contains("_webdavs") => "WebDAV sécurisé",
        "dnssd" if uri.contains("_webdav") => "WebDAV",
        "dnssd" if uri.contains("_smb") => "Partage (SMB)",
        "dnssd" if uri.contains("_ftp") => "FTP",
        "dnssd" if uri.contains("_afp") => "Partage Mac (AFP)",
        _ => "Emplacement réseau",
    }
}

/// Saisie de l'utilisateur → adresse : `\\PC\partage` (Windows) devient `smb://PC/partage`,
/// une adresse sans protocole est un partage SMB.
pub fn parse_address(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix("\\\\") {
        return Some(format!("smb://{}", rest.replace('\\', "/")));
    }
    if t.contains("://") {
        return Some(t.to_string());
    }
    Some(format!("smb://{}", t.trim_start_matches('/')))
}

/// Titre lisible d'un emplacement distant : « Photos sur salon ».
pub fn remote_label(f: &gio::File) -> String {
    let uri = f.uri().to_string();
    let host = uri.split("://").nth(1).and_then(|r| r.split('/').next()).map(|h| h.rsplit('@').next().unwrap_or(h).to_string()).unwrap_or_default();
    match f.parent() {
        None => host,
        Some(_) => f.basename().map(|b| b.to_string_lossy().to_string()).unwrap_or(host),
    }
}

// ───────────────────────────── Serveurs récents ─────────────────────────────

fn recent_path() -> std::path::PathBuf {
    glib::user_data_dir().join("noos/recent-servers")
}

pub fn recent_servers() -> Vec<String> {
    if demo() {
        return vec!["sftp://alex@nas.local/".into(), "davs://salon.local:8443/".into()];
    }
    std::fs::read_to_string(recent_path()).map(|t| t.lines().map(str::to_string).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

fn remember_server(uri: &str) {
    if demo() {
        return;
    }
    let mut list: Vec<String> = recent_servers().into_iter().filter(|u| u != uri).collect();
    list.insert(0, uri.to_string());
    list.truncate(8);
    let p = recent_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, list.join("\n") + "\n");
}

fn forget_server(uri: &str) {
    let list: Vec<String> = recent_servers().into_iter().filter(|u| u != uri).collect();
    let _ = std::fs::write(recent_path(), list.join("\n") + "\n");
}

// ───────────────────────────── Ouverture et montage ─────────────────────────────

fn window() -> Option<gtk::Window> {
    crate::explorer().map(|e| e.window.clone().upcast())
}

fn toast(text: &str) {
    if let Some(e) = crate::explorer() {
        e.toast(text);
    }
}

/// Ouvre un emplacement distant : résout les annonces DNS-SD, monte au besoin, puis navigue.
pub fn open(file: gio::File) {
    if demo() {
        toast(&format!("Démonstration : ouverture de {}", file.uri()));
        return;
    }
    if file.uri_scheme().as_deref() == Some("dnssd") || (file.uri_scheme().as_deref() == Some("network") && !is_network(&file)) {
        // Entrée découverte : sa cible (`standard::target-uri`), sinon on la monte.
        let f2 = file.clone();
        glib::spawn_future_local(async move {
            let info = f2.query_info_future("standard::target-uri,standard::type", gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await;
            if let Ok(i) = &info
                && let Some(t) = i.attribute_string("standard::target-uri")
                && t != f2.uri()
            {
                open(gio::File::for_uri(&t));
                return;
            }
            let op = mount_operation();
            f2.mount_mountable(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, |res| match res {
                Ok(f) => open(f),
                Err(e) => report(&e),
            });
        });
        return;
    }
    ensure_mounted(file, navigate_mounted);
}

fn report(e: &glib::Error) {
    if e.matches(gio::IOErrorEnum::FailedHandled) || e.matches(gio::IOErrorEnum::Cancelled) {
        return;
    }
    let msg = if e.matches(gio::IOErrorEnum::PermissionDenied) {
        "Accès refusé : vérifiez le nom d'utilisateur et le mot de passe.".to_string()
    } else if e.matches(gio::IOErrorEnum::HostNotFound) || e.matches(gio::IOErrorEnum::HostUnreachable) || e.matches(gio::IOErrorEnum::TimedOut) {
        "Appareil injoignable : vérifiez qu'il est allumé, sur le même réseau, et que son partage est actif.".to_string()
    } else if e.matches(gio::IOErrorEnum::NotSupported) {
        "Protocole non pris en charge (paquet gvfs-backends installé ?).".to_string()
    } else {
        e.message().to_string()
    };
    let d = adap::AlertDialog::new(Some("Connexion impossible"), Some(&msg));
    d.add_response("ok", "Fermer");
    d.present(window().as_ref());
}

/// Monte l'emplacement (si besoin) puis appelle `then` ; retient le serveur dans « Récents ».
pub fn ensure_mounted(file: gio::File, then: impl Fn(gio::File) + 'static) {
    if file.find_enclosing_mount(gio::Cancellable::NONE).is_ok() {
        then(file);
        return;
    }
    let op = mount_operation();
    let f2 = file.clone();
    file.mount_enclosing_volume(gio::MountMountFlags::NONE, Some(&op), gio::Cancellable::NONE, move |res| match res {
        Ok(()) => {
            remember_server(&server_root(&f2));
            then(f2.clone());
        }
        Err(e) if e.matches(gio::IOErrorEnum::AlreadyMounted) => then(f2.clone()),
        Err(e) => report(&e),
    });
}

/// Racine « serveur » d'une adresse (`smb://pc/photos/2024` → `smb://pc/photos/`).
fn server_root(f: &gio::File) -> String {
    let uri = f.uri().to_string();
    let Some((scheme, rest)) = uri.split_once("://") else { return uri };
    let mut parts = rest.splitn(3, '/');
    let host = parts.next().unwrap_or("");
    match (scheme, parts.next()) {
        ("smb", Some(share)) if !share.is_empty() => format!("smb://{host}/{share}/"),
        _ => format!("{scheme}://{host}/"),
    }
}

/// Opération de montage : fenêtre d'identifiants (libAdapta) et questions (empreinte SSH…).
pub fn mount_operation() -> gio::MountOperation {
    let op = gio::MountOperation::new();
    // Le gestionnaire par défaut de GMountOperation répond aussitôt « non traité » : on
    // arrête l'émission pour que seule notre réponse (après la fenêtre) parte vers gvfs.
    op.connect_ask_password(|op, message, default_user, default_domain, flags| {
        op.stop_signal_emission_by_name("ask-password");
        credentials_dialog(op, message, default_user, default_domain, flags);
    });
    // « ask-question » (empreinte SSH inconnue…) : signal à tableau de chaînes, non généré.
    op.connect_local("ask-question", false, |args| {
        let op = args[0].get::<gio::MountOperation>().ok()?;
        op.stop_signal_emission_by_name("ask-question");
        let message = args[1].get::<String>().unwrap_or_default();
        let choices: Vec<String> = args[2].get::<glib::StrV>().map(|v| v.iter().map(|s| s.to_string()).collect()).unwrap_or_default();
        let (title, body) = message.split_once('\n').map(|(a, b)| (a.to_string(), b.trim().to_string())).unwrap_or((message.clone(), String::new()));
        let d = adap::AlertDialog::new(Some(&title), (!body.is_empty()).then_some(body.as_str()));
        for (i, c) in choices.iter().enumerate() {
            d.add_response(&i.to_string(), c);
        }
        d.set_close_response("cancel");
        d.connect_response(None, move |_, r| match r.parse::<i32>() {
            Ok(i) => {
                op.set_choice(i);
                op.reply(gio::MountOperationResult::Handled);
            }
            Err(_) => op.reply(gio::MountOperationResult::Aborted),
        });
        d.present(window().as_ref());
        None
    });
    op
}

/// Identifiants : compte, domaine (Windows), mot de passe, connexion invitée, « retenir ».
fn credentials_dialog(op: &gio::MountOperation, message: &str, default_user: &str, default_domain: &str, flags: gio::AskPasswordFlags) {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let anon = adap::SwitchRow::builder().title("Connexion invitée").subtitle("Sans compte, si l'appareil l'autorise").build();
    let user = adap::EntryRow::builder().title("Nom d'utilisateur").text(if default_user.is_empty() { glib::user_name().to_string_lossy().to_string() } else { default_user.to_string() }).build();
    let domain = adap::EntryRow::builder().title("Domaine ou groupe de travail").text(if default_domain.is_empty() { "WORKGROUP" } else { default_domain }).build();
    let pass = adap::PasswordEntryRow::builder().title("Mot de passe").build();
    let save = adap::SwitchRow::builder().title("Retenir le mot de passe").subtitle("Enregistré, chiffré, dans le trousseau de votre session").build();
    if flags.contains(gio::AskPasswordFlags::ANONYMOUS_SUPPORTED) {
        list.append(&anon);
    }
    if flags.contains(gio::AskPasswordFlags::NEED_USERNAME) {
        list.append(&user);
    }
    if flags.contains(gio::AskPasswordFlags::NEED_DOMAIN) {
        list.append(&domain);
    }
    if flags.contains(gio::AskPasswordFlags::NEED_PASSWORD) {
        list.append(&pass);
    }
    if flags.contains(gio::AskPasswordFlags::SAVING_SUPPORTED) {
        list.append(&save);
    }
    {
        let (user, domain, pass, save) = (user.clone(), domain.clone(), pass.clone(), save.clone());
        anon.connect_active_notify(move |a| {
            for w in [user.upcast_ref::<gtk::Widget>(), domain.upcast_ref(), pass.upcast_ref(), save.upcast_ref()] {
                w.set_sensitive(!a.is_active());
            }
        });
    }
    // Message de gvfs (« Mot de passe requis pour le partage Photos sur salon ») : en titre.
    let first = message.lines().next().unwrap_or(message).to_string();
    let d = adap::AlertDialog::new(Some("Identifiez-vous"), Some(&first));
    d.set_extra_child(Some(&list));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Se connecter")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    let op = op.clone();
    let (anon2, pass2) = (anon.clone(), pass.clone());
    d.connect_response(None, move |_, r| {
        if r != "ok" {
            op.reply(gio::MountOperationResult::Aborted);
            return;
        }
        if anon2.is_active() {
            op.set_anonymous(true);
        } else {
            op.set_anonymous(false);
            op.set_username(Some(user.text().trim()));
            op.set_domain(Some(domain.text().trim()));
            op.set_password(Some(&pass2.text()));
            op.set_password_save(if save.is_active() { gio::PasswordSave::Permanently } else { gio::PasswordSave::ForSession });
        }
        op.reply(gio::MountOperationResult::Handled);
    });
    d.present(window().as_ref());
    pass.grab_focus();
}

/// Démonstration : fenêtre d'identifiants telle qu'un partage SMB la déclenche.
pub fn demo_credentials() {
    let op = gio::MountOperation::new();
    credentials_dialog(
        &op,
        "Mot de passe requis pour le partage « Photos » sur « portable-alex »",
        "axel",
        "WORKGROUP",
        gio::AskPasswordFlags::NEED_USERNAME | gio::AskPasswordFlags::NEED_DOMAIN | gio::AskPasswordFlags::NEED_PASSWORD | gio::AskPasswordFlags::SAVING_SUPPORTED | gio::AskPasswordFlags::ANONYMOUS_SUPPORTED,
    );
}

// ───────────────────────────── Se connecter à un serveur ─────────────────────────────

pub fn connect_dialog(prefill: &str) {
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let entry = adap::EntryRow::builder().title("Adresse du serveur").text(prefill).build();
    entry.set_activates_default(true);
    list.append(&entry);
    bx.append(&list);
    // Exemples : un clic remplit le début de l'adresse.
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    chips.set_homogeneous(true);
    for (label, prefix, tip) in [
        ("Windows", "smb://", "smb://nom-du-pc/partage  ou  \\\\nom-du-pc\\partage"),
        ("SFTP", "sftp://", "sftp://utilisateur@serveur/"),
        ("WebDAV", "davs://", "davs://serveur:8443/"),
        ("FTP", "ftp://", "ftp://serveur/"),
    ] {
        let b = gtk::Button::with_label(label);
        b.add_css_class("pill");
        b.add_css_class("net-chip");
        b.set_tooltip_text(Some(tip));
        let e = entry.clone();
        b.connect_clicked(move |_| {
            let t = e.text().to_string();
            let rest = t.split_once("://").map(|(_, r)| r.to_string()).unwrap_or_else(|| t.trim_start_matches('\\').to_string());
            e.set_text(&format!("{prefix}{rest}"));
            e.grab_focus();
            e.set_position(-1);
        });
        chips.append(&b);
    }
    bx.append(&chips);
    let hint = gtk::Label::new(Some("Exemples : \\\\SALON\\Photos · smb://nas.local/ · sftp://alex@192.168.1.31/"));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_wrap(true);
    hint.set_xalign(0.0);
    bx.append(&hint);
    let recents = recent_servers();
    if !recents.is_empty() {
        let rl = gtk::ListBox::new();
        rl.add_css_class("boxed-list");
        rl.set_selection_mode(gtk::SelectionMode::None);
        for uri in recents.iter().take(4) {
            let r = adap::ActionRow::builder().title(uri.as_str()).subtitle(kind_of(uri)).activatable(true).build();
            r.add_prefix(&gtk::Image::from_icon_name("document-open-recent-symbolic"));
            let (e, u) = (entry.clone(), uri.clone());
            r.connect_activated(move |_| e.set_text(&u));
            rl.append(&r);
        }
        let l = gtk::Label::new(Some("Récents"));
        l.add_css_class("heading");
        l.set_xalign(0.0);
        bx.append(&l);
        bx.append(&rl);
    }
    let d = adap::AlertDialog::new(Some("Se connecter à un serveur"), Some("Partage Windows ou Samba, serveur SFTP, WebDAV ou FTP. Le mot de passe est demandé ensuite, si besoin."));
    d.set_extra_child(Some(&bx));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Se connecter")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_response_enabled("ok", !prefill.trim().is_empty());
    {
        let d2 = d.clone();
        entry.connect_changed(move |e| d2.set_response_enabled("ok", !e.text().trim().is_empty()));
    }
    d.connect_response(None, move |_, r| {
        if r == "ok"
            && let Some(uri) = parse_address(&entry.text())
        {
            let f = gio::File::for_uri(&uri);
            if demo() {
                toast(&format!("Démonstration : connexion à {uri}"));
                return;
            }
            open(f);
        }
    });
    d.present(window().as_ref());
}

// ───────────────────────────── Page « Réseau » ─────────────────────────────

/// Appareil ou service découvert.
#[derive(Clone)]
struct Found {
    name: String,
    icon: gio::Icon,
    file: gio::File,
    kind: String,
}

fn demo_found() -> Vec<Found> {
    let t = |n: &str| -> gio::Icon { gio::ThemedIcon::new(n).upcast() };
    vec![
        Found { name: "PORTABLE-ALEX".into(), icon: t("computer"), file: gio::File::for_uri("smb://portable-alex/"), kind: "Partage Windows (SMB)".into() },
        Found { name: "NAS-MAISON".into(), icon: t("network-server"), file: gio::File::for_uri("smb://nas-maison/"), kind: "Partage Windows (SMB)".into() },
        Found { name: "iMac de Sam".into(), icon: t("computer-apple"), file: gio::File::for_uri("smb://imac-de-sam.local/"), kind: "Partage (SMB)".into() },
        Found { name: "salon".into(), icon: t("network-server"), file: gio::File::for_uri("sftp://salon.local/"), kind: "SFTP (SSH)".into() },
        Found { name: "WebDAV de alex sur salon".into(), icon: t("folder-remote"), file: gio::File::for_uri("davs://salon.local:8443/"), kind: "WebDAV sécurisé".into() },
    ]
}

fn tile(name: &str, kind: &str, icon: &gio::Icon, extra: Option<gtk::Button>, on_click: impl Fn() + 'static) -> gtk::Widget {
    let b = gtk::Button::new();
    b.add_css_class("home-card");
    b.add_css_class("net-card");
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let img = gtk::Image::from_gicon(icon);
    img.set_pixel_size(44);
    h.append(&img);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_valign(gtk::Align::Center);
    v.set_hexpand(true);
    let t = gtk::Label::new(Some(name));
    t.add_css_class("heading");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    v.append(&t);
    let s = gtk::Label::new(Some(kind));
    s.add_css_class("dim-label");
    s.add_css_class("caption");
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    v.append(&s);
    h.append(&v);
    b.set_child(Some(&h));
    b.connect_clicked(move |_| on_click());
    match extra {
        None => b.upcast(),
        Some(x) => {
            // Bouton secondaire (déconnecter, oublier) par-dessus la tuile.
            v.set_margin_end(30);
            let o = gtk::Overlay::new();
            o.set_child(Some(&b));
            x.add_css_class("flat");
            x.add_css_class("circular");
            x.add_css_class("net-card-action");
            x.set_halign(gtk::Align::End);
            x.set_valign(gtk::Align::Center);
            x.set_margin_end(10);
            o.add_overlay(&x);
            o.upcast()
        }
    }
}

/// Entrées de `network:///` (gvfs), lues sans bloquer.
async fn discover() -> Vec<Found> {
    if demo() {
        return demo_found();
    }
    let root = gio::File::for_uri(NETWORK_URI);
    let Ok(en) = root.enumerate_children_future("standard::*", gio::FileQueryInfoFlags::NONE, glib::Priority::DEFAULT).await else { return vec![] };
    let mut out = vec![];
    while let Ok(infos) = en.next_files_future(64, glib::Priority::DEFAULT).await {
        if infos.is_empty() {
            break;
        }
        for i in infos {
            let target = i.attribute_string("standard::target-uri").map(|s| s.to_string());
            let file = match &target {
                Some(t) => gio::File::for_uri(t),
                None => root.child(i.name()),
            };
            let kind = kind_of(target.as_deref().unwrap_or(""));
            out.push(Found { name: i.display_name().to_string(), icon: i.icon().unwrap_or_else(|| gio::ThemedIcon::new("network-server").upcast()), file, kind: kind.into() });
        }
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

/// Partages ouverts (montages gvfs distants).
fn connected() -> Vec<(String, gio::Icon, gio::File, Option<gio::Mount>)> {
    if demo() {
        let t: gio::Icon = gio::ThemedIcon::new("folder-remote").upcast();
        return vec![("Photos sur nas-maison".into(), t, gio::File::for_uri("smb://nas-maison/photos/"), None)];
    }
    gio::VolumeMonitor::get()
        .mounts()
        .into_iter()
        .filter(|m| is_remote(&m.root()))
        .map(|m| (m.name().to_string(), m.icon(), m.root(), Some(m)))
        .collect()
}

/// Construit la page « Réseau » dans `holder` ; la découverte se met à jour toute seule.
pub fn fill(holder: &gtk::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let title = gtk::Label::new(Some("Réseau"));
    title.add_css_class("home-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    top.append(&title);
    let refresh = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh.set_tooltip_text(Some("Actualiser"));
    refresh.set_valign(gtk::Align::Center);
    refresh.add_css_class("flat");
    let connect = gtk::Button::with_label("Se connecter à un serveur…");
    connect.add_css_class("suggested-action");
    connect.add_css_class("pill");
    connect.set_valign(gtk::Align::Center);
    connect.connect_clicked(|_| connect_dialog(""));
    top.append(&refresh);
    top.append(&connect);
    holder.append(&top);

    // Partages ouverts.
    let mounts = connected();
    if !mounts.is_empty() {
        holder.append(&section("Connectés"));
        let fb = flow();
        for (name, icon, root, mount) in mounts {
            let x = gtk::Button::from_icon_name("media-eject-symbolic");
            x.set_tooltip_text(Some("Se déconnecter"));
            let name2 = name.clone();
            x.connect_clicked(move |b| {
                b.set_sensitive(false);
                let Some(m) = &mount else { return };
                let name = name2.clone();
                m.unmount_with_operation(gio::MountUnmountFlags::NONE, Some(&mount_operation()), gio::Cancellable::NONE, move |r| match r {
                    Ok(()) => {
                        toast(&format!("« {name} » déconnecté"));
                        if let Some(ex) = crate::explorer() {
                            ex.navigate(gio::File::for_uri(NETWORK_URI), false);
                        }
                    }
                    Err(e) => toast(&format!("Déconnexion impossible : {}", e.message())),
                });
            });
            let kind = kind_of(&root.uri());
            fb.insert(&tile(&name, kind, &icon, Some(x), move || open(root.clone())), -1);
        }
        holder.append(&fb);
    }

    // Appareils découverts : liste remplie en arrière-plan, actualisée quand gvfs en trouve.
    holder.append(&section("Appareils sur le réseau local"));
    let found_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&found_box);
    let render: Rc<dyn Fn(Vec<Found>)> = {
        let found_box = found_box.clone();
        Rc::new(move |items: Vec<Found>| {
            while let Some(c) = found_box.first_child() {
                found_box.remove(&c);
            }
            if items.is_empty() {
                let st = adap::StatusPage::builder()
                    .icon_name("network-workgroup-symbolic")
                    .title("Aucun appareil trouvé pour l'instant")
                    .description("Les PC Windows, Mac, NAS et serveurs qui partagent des fichiers apparaissent ici en quelques secondes. Sinon, connectez-vous directement par leur adresse.")
                    .build();
                st.add_css_class("compact");
                st.add_css_class("net-empty");
                found_box.append(&st);
                return;
            }
            let fb = flow();
            for f in items {
                let file = f.file.clone();
                fb.insert(&tile(&f.name, &f.kind, &f.icon, None, move || open(file.clone())), -1);
            }
            found_box.append(&fb);
        })
    };
    let searching = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let sp = gtk::Spinner::new();
    sp.start();
    searching.append(&sp);
    let sl = gtk::Label::new(Some("Recherche des appareils…"));
    sl.add_css_class("dim-label");
    searching.append(&sl);
    found_box.append(&searching);
    let run: Rc<dyn Fn()> = {
        let render = render.clone();
        Rc::new(move || {
            let render = render.clone();
            glib::spawn_future_local(async move {
                let items = discover().await;
                render(items);
            });
        })
    };
    run();
    {
        let run = run.clone();
        refresh.connect_clicked(move |_| run());
    }
    if !demo()
        && let Ok(mon) = gio::File::for_uri(NETWORK_URI).monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
    {
        // Plusieurs annonces arrivent d'affilée : une seule relecture.
        let pending = Rc::new(RefCell::new(None::<glib::SourceId>));
        let weak = found_box.downgrade();
        mon.connect_changed(move |_, _, _, _| {
            if weak.upgrade().is_none() {
                return;
            }
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (run, p2) = (run.clone(), pending.clone());
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                p2.borrow_mut().take();
                run();
            }));
        });
        unsafe { found_box.set_data("noos-monitor", mon) };
    }

    // Serveurs récents.
    let recents = recent_servers();
    if !recents.is_empty() {
        holder.append(&section("Serveurs récents"));
        let fb = flow();
        for uri in recents {
            let f = gio::File::for_uri(&uri);
            let icon: gio::Icon = gio::ThemedIcon::new("folder-remote").upcast();
            let x = gtk::Button::from_icon_name("window-close-symbolic");
            x.set_tooltip_text(Some("Retirer de la liste"));
            let u2 = uri.clone();
            x.connect_clicked(move |_| {
                forget_server(&u2);
                if let Some(ex) = crate::explorer() {
                    ex.navigate(gio::File::for_uri(NETWORK_URI), false);
                }
            });
            let label = uri.split("://").nth(1).unwrap_or(&uri).trim_end_matches('/').to_string();
            fb.insert(&tile(&label, kind_of(&uri), &icon, Some(x), move || open(f.clone())), -1);
        }
        holder.append(&fb);
    }

    // Et ce PC ? Lien vers les réglages de partage.
    let share = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    share.add_css_class("net-share-hint");
    let i = gtk::Image::from_icon_name("folder-publicshare-symbolic");
    i.set_pixel_size(24);
    share.append(&i);
    let l = gtk::Label::new(Some("Partager les dossiers de ce PC avec les autres appareils (Samba, SFTP, WebDAV)"));
    l.set_wrap(true);
    l.set_xalign(0.0);
    l.set_hexpand(true);
    share.append(&l);
    let b = gtk::Button::with_label("Paramètres de partage");
    b.set_valign(gtk::Align::Center);
    b.connect_clicked(|_| {
        let _ = gio::AppInfo::create_from_commandline("noos-settings samba", None, gio::AppInfoCreateFlags::NONE).and_then(|a| a.launch(&[], gio::AppLaunchContext::NONE));
    });
    share.append(&b);
    holder.append(&share);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adresses() {
        assert_eq!(parse_address("\\\\SALON\\Photos").as_deref(), Some("smb://SALON/Photos"));
        assert_eq!(parse_address("nas.local/jeux").as_deref(), Some("smb://nas.local/jeux"));
        assert_eq!(parse_address(" sftp://a@b/ ").as_deref(), Some("sftp://a@b/"));
        assert_eq!(parse_address("  "), None);
        assert_eq!(kind_of("dnssd://local/salon._sftp-ssh._tcp"), "SFTP (SSH)");
        assert_eq!(kind_of("davs://x:8443/"), "WebDAV sécurisé");
        assert_eq!(server_root(&gio::File::for_uri("smb://pc/photos/2024/")), "smb://pc/photos/");
        assert_eq!(server_root(&gio::File::for_uri("sftp://a@pc:2222/home/a")), "sftp://a@pc:2222/");
    }
}

//! « Bluetooth et appareils » (façon Windows 11) : Bluetooth (BlueZ) avec tuiles des appareils
//! associés et assistant « Ajouter un appareil », imprimantes et scanners (CUPS, SANE),
//! souris et clavier.
//!
//! Démos : `NOOS_BT_DEMO=1|add|pin|off|none|service` et `NOOS_PRINT_DEMO=1|add|printer|driver`.

pub mod bluez;
mod bt_add;
pub mod cups;
pub mod printers;

use std::{cell::Cell, rc::Rc};

use adap::prelude::*;
use gtk::glib;

use crate::ui::{App, group, link, page};
use bluez::{Availability, Bluetooth, Device};

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Bluetooth et appareils");
    let bt = Bluetooth::get();
    bt_add::install_agent_ui(app);

    // ── Bandeau Bluetooth ──
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    hero.add_css_class("win11-hero");
    let icon_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    icon_box.add_css_class("bt-hero-icon");
    icon_box.set_valign(gtk::Align::Center);
    icon_box.set_vexpand(false);
    let hero_icon = gtk::Image::from_icon_name("bluetooth-active-symbolic");
    hero_icon.set_pixel_size(34);
    hero_icon.set_vexpand(true);
    icon_box.append(&hero_icon);
    hero.append(&icon_box);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 4);
    texts.set_valign(gtk::Align::Center);
    texts.set_hexpand(true);
    let title = gtk::Label::new(Some("Bluetooth"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    let sub = gtk::Label::new(Some("Lecture de l'état du Bluetooth…"));
    sub.add_css_class("dim-label");
    sub.set_xalign(0.0);
    sub.set_wrap(true);
    texts.append(&title);
    texts.append(&sub);
    hero.append(&texts);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    actions.set_valign(gtk::Align::Center);
    let add = gtk::Button::with_label("Ajouter un appareil");
    add.add_css_class("suggested-action");
    add.add_css_class("pill");
    let fix = gtk::Button::new();
    fix.add_css_class("pill");
    fix.set_visible(false);
    let power = gtk::Switch::new();
    power.set_valign(gtk::Align::Center);
    power.set_tooltip_text(Some("Activer ou désactiver le Bluetooth"));
    actions.append(&fix);
    actions.append(&add);
    actions.append(&power);
    hero.append(&actions);
    content.append(&hero);

    // ── Tuiles des appareils associés ──
    let devices_group = group("Vos appareils");
    let tiles = gtk::FlowBox::new();
    tiles.set_selection_mode(gtk::SelectionMode::None);
    tiles.set_homogeneous(true);
    tiles.set_min_children_per_line(1);
    tiles.set_max_children_per_line(3);
    tiles.set_column_spacing(8);
    tiles.set_row_spacing(8);
    devices_group.add(&tiles);
    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
    empty.add_css_class("dev-empty");
    let ei = gtk::Image::from_icon_name("bluetooth-symbolic");
    ei.set_pixel_size(40);
    ei.add_css_class("dim-label");
    empty.append(&ei);
    let el = gtk::Label::new(Some("Aucun appareil associé"));
    el.add_css_class("heading");
    empty.append(&el);
    let ed = gtk::Label::new(Some("Casques, souris, claviers, manettes, téléphones… Cliquez sur « Ajouter un appareil » pour en associer un."));
    ed.add_css_class("dim-label");
    ed.set_wrap(true);
    ed.set_justify(gtk::Justification::Center);
    empty.append(&ed);
    devices_group.add(&empty);
    content.append(&devices_group);

    // ── Réglages ──
    let settings = group("Paramètres Bluetooth");
    let visible = adap::SwitchRow::new();
    visible.set_title("Visible par les autres appareils");
    visible.set_subtitle("Permet à un téléphone ou un autre PC de trouver ce PC (désactivé automatiquement au bout de 3 minutes)");
    let vi = gtk::Image::from_icon_name("view-reveal-symbolic");
    vi.add_css_class("win11-row-icon");
    visible.add_prefix(&vi);
    settings.add(&visible);
    content.append(&settings);

    let others = group("Autres appareils");
    others.add(&link(app, "printer-symbolic", "Imprimantes et scanners", "Ajouter une imprimante, file d'attente, page de test, scanners", printers::page));
    others.add(&link(app, "input-mouse-symbolic", "Souris et pavé tactile", "Vitesse du pointeur, défilement, appui pour cliquer", crate::other::mouse));
    others.add(&link(app, "input-keyboard-symbolic", "Clavier", "Disposition et variante", crate::other::keyboard));
    content.append(&others);

    // ── Mise à jour depuis le modèle ──
    let syncing = Rc::new(Cell::new(false));
    let render = {
        let (app, bt, syncing) = (app.clone(), bt.clone(), syncing.clone());
        let (icon_box, hero_icon, title, sub, add, fix, power) = (icon_box.clone(), hero_icon.clone(), title.clone(), sub.clone(), add.clone(), fix.clone(), power.clone());
        let (devices_group, tiles, empty, settings, visible) = (devices_group.clone(), tiles.clone(), empty.clone(), settings.clone(), visible.clone());
        move || {
            let m = bt.model();
            syncing.set(true);
            let ready = m.availability == Availability::Ready;
            let on = m.adapter.as_ref().is_some_and(|a| a.powered);
            power.set_visible(ready);
            power.set_state(on);
            power.set_active(on);
            add.set_visible(ready && on);
            fix.set_visible(false);
            devices_group.set_visible(ready);
            settings.set_visible(ready && on);
            icon_box.set_css_classes(if ready && on { &["bt-hero-icon"] } else { &["bt-hero-icon", "off"] });
            hero_icon.set_icon_name(Some(if ready && on { "bluetooth-active-symbolic" } else { "bluetooth-disabled-symbolic" }));
            match m.availability {
                Availability::Unknown => {
                    title.set_text("Bluetooth");
                    sub.set_text("Lecture de l'état du Bluetooth…");
                }
                Availability::NoService => {
                    let installed = ["/usr/libexec/bluetooth/bluetoothd", "/usr/sbin/bluetoothd"].iter().any(|p| std::path::Path::new(p).exists());
                    if installed {
                        title.set_text("Le service Bluetooth est arrêté");
                        sub.set_text("Le service bluetoothd ne répond pas. Démarrez-le pour utiliser vos appareils sans fil.");
                        fix.set_label("Démarrer le service");
                    } else {
                        title.set_text("Bluetooth non installé");
                        sub.set_text("La pile Bluetooth (BlueZ) et le module audio de PipeWire ne sont pas installés sur ce PC.");
                        fix.set_label("Installer");
                    }
                    fix.set_visible(true);
                }
                Availability::NoAdapter => {
                    title.set_text("Aucun adaptateur Bluetooth");
                    sub.set_text("Ce PC n'a pas de puce Bluetooth, ou elle est désactivée dans le BIOS/UEFI. Une clé USB Bluetooth suffit pour en ajouter une.");
                }
                Availability::Ready => {
                    let a = m.adapter.clone().unwrap_or_default();
                    title.set_text("Bluetooth");
                    sub.set_text(&if a.blocked() {
                        "Bloqué par le mode Avion ou l'interrupteur sans fil du PC. Activez-le pour le débloquer.".to_string()
                    } else if on {
                        format!("Activé — visible en tant que « {} »", a.name)
                    } else {
                        "Désactivé".to_string()
                    });
                    visible.set_active(a.discoverable);
                }
            }
            // Tuiles.
            while let Some(c) = tiles.first_child() {
                tiles.remove(&c);
            }
            let paired = m.paired();
            empty.set_visible(paired.is_empty());
            tiles.set_visible(!paired.is_empty());
            for d in paired {
                tiles.append(&tile(&app, &bt, &d, on));
            }
            syncing.set(false);
        }
    };
    render();
    {
        let render = render.clone();
        let weak = p.downgrade();
        bt.subscribe(move || {
            if weak.upgrade().is_none() {
                return false;
            }
            render();
            true
        });
    }

    {
        let (app, bt, syncing) = (app.clone(), bt.clone(), syncing.clone());
        power.connect_state_set(move |sw, on| {
            if syncing.get() {
                return glib::Propagation::Proceed;
            }
            sw.set_sensitive(false);
            let (app, bt, sw) = (app.clone(), bt.clone(), sw.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = bt.set_powered(on).await {
                    app.error(if on { "Activer le Bluetooth" } else { "Désactiver le Bluetooth" }, &e);
                    let cur = bt.model().adapter.is_some_and(|a| a.powered);
                    sw.set_state(cur);
                    sw.set_active(cur);
                }
                sw.set_sensitive(true);
            });
            glib::Propagation::Stop
        });
    }
    {
        let (app, bt, syncing) = (app.clone(), bt.clone(), syncing.clone());
        visible.connect_active_notify(move |r| {
            if syncing.get() {
                return;
            }
            let (app, bt, on) = (app.clone(), bt.clone(), r.is_active());
            glib::spawn_future_local(async move {
                if let Err(e) = bt.set_discoverable(on).await {
                    app.toast(&e);
                }
            });
        });
    }
    {
        let app = app.clone();
        add.connect_clicked(move |_| bt_add::open(&app));
    }
    {
        let app = app.clone();
        fix.connect_clicked(move |b| {
            let install = b.label().is_some_and(|l| l == "Installer");
            let plan = if install {
                noos_pkg::Plan::new().root(&["apt-get", "install", "-y", "bluez", "libspa-0.2-bluetooth"]).root(&["systemctl", "enable", "--now", "bluetooth.service"])
            } else {
                noos_pkg::Plan::new().root(&["systemctl", "enable", "--now", "bluetooth.service"])
            };
            b.set_sensitive(false);
            let b = b.clone();
            app.run_plan(if install { "Installer le Bluetooth" } else { "Démarrer le service Bluetooth" }, Ok(plan), move |_| b.set_sensitive(true));
        });
    }
    // Démo : assistant ouvert d'emblée.
    if matches!(bluez::demo_mode().as_str(), "add" | "pair" | "pin") {
        let app = app.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || bt_add::open(&app));
    }
    p
}

/// Icône symbolique existante dans le thème, sinon `fallback`.
pub fn themed(icon: &str, fallback: &'static str) -> String {
    let display = gtk::gdk::Display::default();
    match display {
        Some(d) if gtk::IconTheme::for_display(&d).has_icon(icon) => icon.to_string(),
        _ => fallback.to_string(),
    }
}

fn battery_icon(level: u8) -> String {
    let step = (level as u32 + 5) / 10 * 10;
    themed(&format!("battery-level-{}-symbolic", step.min(100)), "battery-symbolic")
}

/// Tuile d'un appareil associé : icône, nom, état, batterie, menu « … ».
fn tile(app: &Rc<App>, bt: &Rc<Bluetooth>, d: &Device, powered: bool) -> gtk::Box {
    let t = gtk::Box::new(gtk::Orientation::Vertical, 10);
    t.add_css_class("bt-tile");
    if d.connected {
        t.add_css_class("connected");
    }
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let ib = gtk::Box::new(gtk::Orientation::Vertical, 0);
    ib.add_css_class("bt-tile-icon");
    ib.set_vexpand(false);
    let kind = d.kind();
    let i = gtk::Image::from_icon_name(&themed(kind.icon, "bluetooth-symbolic"));
    i.set_pixel_size(22);
    i.set_vexpand(true);
    ib.append(&i);
    top.append(&ib);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    top.append(&spacer);

    // Menu « … ».
    let menu = gtk::MenuButton::new();
    menu.set_icon_name("view-more-symbolic");
    menu.add_css_class("flat");
    menu.set_valign(gtk::Align::Start);
    menu.set_tooltip_text(Some("Plus d'options"));
    let pop = gtk::Popover::new();
    let mb = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let item = |label: &str, destructive: bool| {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        b.set_halign(gtk::Align::Fill);
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        if destructive {
            b.add_css_class("error");
        }
        b
    };
    let toggle = item(if d.connected { "Déconnecter" } else { "Connecter" }, false);
    toggle.set_sensitive(powered);
    let rename = item("Renommer", false);
    let forget = item("Supprimer l'appareil", true);
    mb.append(&toggle);
    mb.append(&rename);
    mb.append(&forget);
    pop.set_child(Some(&mb));
    menu.set_popover(Some(&pop));
    top.append(&menu);
    t.append(&top);

    let name = gtk::Label::new(Some(&d.name));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_tooltip_text(Some(&format!("{} — {} — {}", kind.label, d.status(), d.address)));
    t.append(&name);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    if d.connected {
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("bt-dot");
        status.append(&dot);
    }
    let sl = gtk::Label::new(Some(if d.connected { "Connecté" } else { "Non connecté" }));
    sl.add_css_class("dim-label");
    sl.add_css_class("caption");
    status.append(&sl);
    if let (true, Some(level)) = (d.connected, d.battery) {
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        status.append(&sp);
        let bi = gtk::Image::from_icon_name(&battery_icon(level));
        bi.set_pixel_size(14);
        if level <= 20 {
            bi.add_css_class("bt-battery-low");
        }
        status.append(&bi);
        let bl = gtk::Label::new(Some(&format!("{level} %")));
        bl.add_css_class("caption");
        bl.add_css_class("numeric");
        if level <= 20 {
            bl.add_css_class("bt-battery-low");
        }
        status.append(&bl);
    }
    t.append(&status);

    // Actions du menu.
    let path = d.path.clone();
    {
        let (app, bt, path, pop, on) = (app.clone(), bt.clone(), path.clone(), pop.clone(), !d.connected);
        let name = d.name.clone();
        let t2 = t.clone();
        toggle.connect_clicked(move |_| {
            pop.popdown();
            t2.add_css_class("busy");
            let (app, bt, path, name) = (app.clone(), bt.clone(), path.clone(), name.clone());
            let t2 = t2.clone();
            app.toast(&format!("{} de « {name} »…", if on { "Connexion" } else { "Déconnexion" }));
            glib::spawn_future_local(async move {
                if let Err(e) = bt.connect_device(&path, on).await {
                    app.error(&format!("{} « {name} »", if on { "Connecter" } else { "Déconnecter" }), &e);
                }
                t2.remove_css_class("busy");
            });
        });
    }
    {
        let (app, bt, path, pop, name) = (app.clone(), bt.clone(), path.clone(), pop.clone(), d.name.clone());
        rename.connect_clicked(move |_| {
            pop.popdown();
            let d = adap::AlertDialog::new(Some("Renommer l'appareil"), Some("Ce nom est utilisé seulement sur ce PC."));
            let e = gtk::Entry::new();
            e.set_text(&name);
            e.set_activates_default(true);
            d.set_extra_child(Some(&e));
            d.add_responses(&[("cancel", "Annuler"), ("ok", "Renommer")]);
            d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
            d.set_default_response(Some("ok"));
            let (app2, bt, path) = (app.clone(), bt.clone(), path.clone());
            d.connect_response(None, move |_, r| {
                let n = e.text().trim().to_string();
                if r != "ok" || n.is_empty() {
                    return;
                }
                let (app, bt, path) = (app2.clone(), bt.clone(), path.clone());
                glib::spawn_future_local(async move {
                    if let Err(e) = bt.rename(&path, &n).await {
                        app.error("Renommer l'appareil", &e);
                    }
                });
            });
            d.present(Some(&app.window));
        });
    }
    {
        let (app, bt, pop, name) = (app.clone(), bt.clone(), pop.clone(), d.name.clone());
        forget.connect_clicked(move |_| {
            pop.popdown();
            let (app2, bt, path, name2) = (app.clone(), bt.clone(), path.clone(), name.clone());
            app.confirm(
                &format!("Supprimer « {name} » ?"),
                "L'appareil sera déconnecté et oublié. Pour l'utiliser de nouveau, il faudra le remettre en mode association et l'ajouter.",
                "Supprimer",
                true,
                move || {
                    let (app, bt, path, name) = (app2.clone(), bt.clone(), path.clone(), name2.clone());
                    glib::spawn_future_local(async move {
                        match bt.forget(&path).await {
                            Ok(()) => app.toast(&format!("« {name} » a été supprimé")),
                            Err(e) => app.error("Supprimer l'appareil", &e),
                        }
                    });
                },
            );
        });
    }
    t
}

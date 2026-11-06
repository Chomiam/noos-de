//! Jeux, périphériques (clavier, souris), mises à jour.

use std::rc::Rc;

use adap::prelude::*;
use noos_core::config::VrrPolicy;

use crate::ui::{App, card, combo, group, hero, link, page, spin, switch};

pub fn gaming(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Jeux");
    let cfg = app.cfg();
    let gm = &cfg.gaming;

    let g = group("Affichage");
    let vrr = combo(
        "speedometer-symbolic",
        "Fréquence d'actualisation variable (VRR)",
        "« En plein écran » l'active seulement pour les jeux et vidéos",
        &["Désactivée", "En plein écran", "Toujours"].map(String::from),
        match gm.vrr {
            VrrPolicy::Off => 0,
            VrrPolicy::Fullscreen => 1,
            VrrPolicy::Always => 2,
        },
    );
    {
        let app = app.clone();
        vrr.connect_selected_notify(move |r| {
            let v = match r.selected() {
                0 => VrrPolicy::Off,
                2 => VrrPolicy::Always,
                _ => VrrPolicy::Fullscreen,
            };
            app.update(|c| c.gaming.vrr = v);
        });
    }
    g.add(&vrr);
    for (icon, title, sub, get, set) in [
        (
            "media-playback-start-symbolic",
            "Scanout direct",
            "La fenêtre plein écran va directement à l'écran, sans composition (latence minimale)",
            gm.direct_scanout,
            (|c: &mut noos_core::NoosConfig, v| c.gaming.direct_scanout = v) as fn(&mut noos_core::NoosConfig, bool),
        ),
        (
            "color-select-symbolic",
            "Framebuffer 10 bits",
            "Moins de banding, requis pour le HDR (au prochain démarrage)",
            gm.ten_bit,
            |c, v| c.gaming.ten_bit = v,
        ),
        (
            "weather-clear-symbolic",
            "HDR (expérimental)",
            "Détection des écrans HDR ; la sortie HDR elle-même reste à écrire",
            gm.hdr,
            |c, v| c.gaming.hdr = v,
        ),
    ] {
        let row = switch(icon, title, sub, get);
        let app = app.clone();
        row.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| set(c, v));
        });
        g.add(&row);
    }
    content.append(&g);

    let m = group("Mode Jeu");
    let game = switch(
        "input-gaming-symbolic",
        "Mode Jeu",
        "En plein écran : pas de menu Démarrer par la touche Super, pas de barre ni d'effets par-dessus",
        gm.game_mode,
    );
    {
        let app = app.clone();
        game.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.gaming.game_mode = v);
        });
    }
    m.add(&game);
    m.add(&card("dialog-information-symbolic", "Steam, Proton et jeux X11", "Pris en charge via Xwayland (xwayland-satellite). Les jeux peuvent verrouiller la souris (pointer-constraints)."));
    content.append(&m);
    p
}

pub fn devices(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Bluetooth et appareils");
    let g = group("");
    g.add(&link(app, "input-mouse-symbolic", "Souris et pavé tactile", "Vitesse du pointeur, défilement, appui pour cliquer", mouse));
    g.add(&link(app, "input-keyboard-symbolic", "Clavier", "Disposition et variante", keyboard));
    content.append(&g);
    p
}

fn mouse(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Bluetooth et appareils"], "Souris et pavé tactile");
    let cfg = app.cfg();
    let g = group("");
    g.set_description(Some("Appliqué aux périphériques au prochain branchement ou démarrage de session"));
    let accel = spin("input-mouse-symbolic", "Vitesse du pointeur", "-1 (lent) à 1 (rapide)", (-1.0, 1.0, 0.1), cfg.input.pointer_accel);
    accel.set_digits(1);
    {
        let app = app.clone();
        accel.connect_value_notify(move |r| {
            let v = r.value();
            app.update(|c| c.input.pointer_accel = v);
        });
    }
    g.add(&accel);
    let natural = switch("view-sort-descending-symbolic", "Défilement naturel", "Le contenu suit le mouvement des doigts", cfg.input.natural_scroll);
    {
        let app = app.clone();
        natural.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.input.natural_scroll = v);
        });
    }
    g.add(&natural);
    let tap = switch("input-touchpad-symbolic", "Appuyer pour cliquer", "Pavé tactile", cfg.input.tap_to_click);
    {
        let app = app.clone();
        tap.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.input.tap_to_click = v);
        });
    }
    g.add(&tap);
    content.append(&g);
    p
}

fn keyboard(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Bluetooth et appareils"], "Clavier");
    let cfg = app.cfg();
    let g = group("Disposition");
    g.set_description(Some("Vide = disposition du système (/etc/vconsole.conf). Appliqué au prochain démarrage de session."));
    for (title, value, set) in [
        ("Disposition (fr, us, be…)", cfg.keyboard.layout.clone(), (|c: &mut noos_core::NoosConfig, v: String| c.keyboard.layout = v) as fn(&mut noos_core::NoosConfig, String)),
        ("Variante (oss, azerty, bepo…)", cfg.keyboard.variant.clone(), |c, v| c.keyboard.variant = v),
        ("Options XKB (ex. caps:escape)", cfg.keyboard.options.clone(), |c, v| c.keyboard.options = v),
    ] {
        let row = adap::EntryRow::new();
        row.set_title(title);
        row.set_text(&value);
        row.set_show_apply_button(true);
        let app = app.clone();
        row.connect_apply(move |e| {
            let v = e.text().trim().to_string();
            app.update(|c| set(c, v));
        });
        g.add(&row);
    }
    content.append(&g);
    p
}

pub fn updates(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Mises à jour");
    let img = gtk::Image::from_icon_name("software-update-available-symbolic");
    img.set_pixel_size(72);
    let (h, actions) = hero(&img, "Mises à jour du système et des applications", "Flatpak, paquets de la distribution et image système sont gérés par l'application Logiciels.");
    let open = gtk::Button::with_label("Ouvrir Logiciels");
    open.add_css_class("pill");
    open.add_css_class("suggested-action");
    open.connect_clicked(|_| {
        let _ = std::process::Command::new("noos-software").spawn();
    });
    actions.append(&open);
    content.append(&h);
    p
}

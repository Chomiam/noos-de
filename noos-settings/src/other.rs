//! Jeux, périphériques (clavier, souris), mises à jour.

use std::rc::Rc;

use adap::prelude::*;
use gtk::glib;
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
    let g = group("Démarrage");
    let numlock = switch(
        "input-keyboard-symbolic",
        "Verr Num activé au démarrage",
        "Sur l'écran de connexion et à l'ouverture de session",
        cfg.keyboard.numlock,
    );
    {
        let app = app.clone();
        numlock.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.keyboard.numlock = v);
        });
    }
    g.add(&numlock);
    content.append(&g);
    p
}

pub fn updates(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Mises à jour");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&holder);
    fill_updates(app, &holder);
    p
}

/// Contenu de la page (reconstruit après « Vérifier maintenant »).
fn fill_updates(app: &Rc<App>, content: &gtk::Box) {
    use noos_core::updates as upd;
    while let Some(c) = content.first_child() {
        content.remove(&c);
    }
    let cfg = app.cfg();
    let st = upd::load();
    let img = gtk::Image::from_icon_name(if st.total > 0 { "software-update-urgent-symbolic" } else { "software-update-available-symbolic" });
    img.set_pixel_size(72);
    let (title, sub) = match st.total {
        0 if st.checked_at == 0 => ("Mises à jour du système et des applications".to_string(), "Paquets Fedora et applications Flatpak, gérés par l'application Logiciels.".to_string()),
        0 => ("Votre système est à jour".to_string(), format!("Dernière vérification : {}", upd::ago(st.checked_at))),
        n => (
            if n == 1 { "1 mise à jour disponible".to_string() } else { format!("{n} mises à jour disponibles") },
            format!(
                "{} · vérifié {}",
                st.by_backend.iter().map(|(b, n)| format!("{b} : {n}")).collect::<Vec<_>>().join(", "),
                upd::ago(st.checked_at)
            ),
        ),
    };
    let (h, actions) = hero(&img, &title, &sub);
    let open = gtk::Button::with_label(if st.total > 0 { "Installer dans Logiciels" } else { "Ouvrir Logiciels" });
    open.add_css_class("pill");
    open.add_css_class("suggested-action");
    open.connect_clicked(|_| {
        let _ = std::process::Command::new("noos-software").arg("--updates").spawn();
    });
    let check = gtk::Button::with_label("Vérifier maintenant");
    check.add_css_class("pill");
    {
        let (app, holder) = (app.clone(), content.clone());
        check.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Vérification…");
            let task = crate::ui::background(|| std::process::Command::new("noos-pkg").args(["check", "--force"]).output().map(|o| o.status.success()).unwrap_or(false));
            let (app, b, holder) = (app.clone(), b.clone(), holder.clone());
            glib::spawn_future_local(async move {
                let ok = task.await;
                b.set_sensitive(true);
                b.set_label("Vérifier maintenant");
                let st = noos_core::updates::load();
                app.toast(&if !ok {
                    "Vérification impossible (connexion Internet ?)".to_string()
                } else if st.total == 0 {
                    "Votre système est à jour".to_string()
                } else {
                    format!("{} mise{} à jour disponible{}", st.total, if st.total > 1 { "s" } else { "" }, if st.total > 1 { "s" } else { "" })
                });
                // La page se reconstruit avec le nouvel état.
                fill_updates(&app, &holder);
            });
        });
    }
    actions.append(&open);
    actions.append(&check);
    content.append(&h);

    let g = group("Vérification automatique");
    g.set_description(Some("Un service de votre session interroge les dépôts en arrière-plan, sans mot de passe et à basse priorité. Rien n'est installé sans votre accord."));
    let auto = switch("emblem-synchronizing-symbolic", "Rechercher les mises à jour automatiquement", "Une icône apparaît dans la barre des tâches quand il y en a", cfg.updates.auto_check);
    let labels: Vec<String> = upd::INTERVALS.iter().map(|(_, l)| l.to_string()).collect();
    let freq = combo(
        "preferences-system-time-symbolic",
        "Fréquence",
        "",
        &labels,
        upd::INTERVALS.iter().position(|(h, _)| *h == cfg.updates.interval_hours).unwrap_or(2),
    );
    freq.set_sensitive(cfg.updates.auto_check);
    {
        let (app, freq) = (app.clone(), freq.clone());
        auto.connect_active_notify(move |s| {
            let on = s.is_active();
            freq.set_sensitive(on);
            app.update(|c| c.updates.auto_check = on);
            let h = app.cfg().updates.interval_hours;
            apply_update_timer(on, h);
        });
    }
    {
        let app = app.clone();
        freq.connect_selected_notify(move |r| {
            let h = upd::INTERVALS[r.selected() as usize].0;
            app.update(|c| c.updates.interval_hours = h);
            apply_update_timer(app.cfg().updates.auto_check, h);
        });
    }
    g.add(&auto);
    g.add(&freq);
    let tray = switch("view-refresh-symbolic", "Icône dans la barre des tâches", "Deux flèches et une pastille orange quand des mises à jour sont disponibles", cfg.updates.tray_icon);
    {
        let app = app.clone();
        tray.connect_active_notify(move |s| {
            let v = s.is_active();
            app.update(|c| c.updates.tray_icon = v);
        });
    }
    g.add(&tray);
    content.append(&g);

    if !st.items.is_empty() {
        let list = group("Disponibles");
        for it in st.items.iter().take(12) {
            let r = card(
                if it.backend == "flatpak" { "application-x-addon-symbolic" } else { "package-x-generic-symbolic" },
                &it.name,
                &if it.old.is_empty() || it.old == it.new { it.new.clone() } else { format!("{} → {}", it.old, it.new) },
            );
            list.add(&r);
        }
        if st.items.len() > 12 {
            list.add(&card("view-more-symbolic", &format!("… et {} autres", st.items.len() - 12), "Liste complète dans Logiciels"));
        }
        content.append(&list);
    }
    if !st.errors.is_empty() {
        let e = group("Problèmes lors de la dernière vérification");
        for err in &st.errors {
            e.add(&card("dialog-warning-symbolic", err, ""));
        }
        content.append(&e);
    }
}

/// Fréquence du timer utilisateur `noos-update-check.timer` (surcharge dans
/// ~/.config/systemd/user), puis rechargement ; lancé hors du fil principal.
fn apply_update_timer(on: bool, hours: u32) {
    std::thread::spawn(move || {
        let dir = noos_core::paths::xdg_config_home().join("systemd/user/noos-update-check.timer.d");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("noos-interval.conf"),
            format!("# Paramètres › Mises à jour\n[Timer]\nOnUnitActiveSec=\nOnUnitActiveSec={hours}h\n"),
        );
        let sc = |args: &[&str]| {
            let _ = std::process::Command::new("systemctl").arg("--user").args(args).status();
        };
        sc(&["daemon-reload"]);
        if on {
            sc(&["enable", "noos-update-check.timer"]);
            sc(&["restart", "noos-update-check.timer"]);
        }
    });
}

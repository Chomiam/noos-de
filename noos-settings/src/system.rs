//! Système : écran, alimentation, informations.

use std::{cell::RefCell, process::Command, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_core::{
    config::VrrPolicy,
    ipc::{Event, OutputInfo, Request},
    paths,
};

use crate::ui::{App, background, card, combo, group, hero, ipc, link, page, switch};

fn read(path: &str) -> String {
    std::fs::read_to_string(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn hostname() -> String {
    let h = read("/etc/hostname");
    if h.is_empty() { glib::host_name().to_string() } else { h }
}

fn device_model() -> String {
    let vendor = read("/sys/class/dmi/id/sys_vendor");
    let mut product = read("/sys/class/dmi/id/product_version");
    if product.is_empty() || product.eq_ignore_ascii_case("none") || product.starts_with("To be") {
        product = read("/sys/class/dmi/id/product_name");
    }
    format!("{vendor} {product}").trim().to_string()
}

fn os_name() -> String {
    noos_pkg::SystemInfo::detect().name
}

// ───────────────────────────── Accueil Système ─────────────────────────────

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Système");
    let pc = gtk::Image::from_icon_name("computer");
    pc.set_pixel_size(96);
    let (h, actions) = hero(&pc, &hostname(), &format!("{}\n{}", device_model(), os_name()));
    let rename = gtk::Button::with_label("Renommer");
    rename.add_css_class("pill");
    {
        let app = app.clone();
        rename.connect_clicked(move |_| rename_dialog(&app));
    }
    actions.append(&rename);
    content.append(&h);

    let g = group("");
    g.add(&link(app, "video-display-symbolic", "Écran", "Moniteurs, résolution, échelle, HDR et VRR", display));
    g.add(&link(app, "preferences-system-notifications-symbolic", "Notifications", "Alertes des applications, ne pas déranger", notifications));
    g.add(&link(app, "battery-good-symbolic", "Alimentation", "Mode d'alimentation, extinction de l'écran, veille", power));
    g.add(&link(app, "input-gaming-symbolic", "Jeux", "VRR, scanout direct, mode jeu", crate::other::gaming));
    g.add(&link(app, "help-about-symbolic", "Informations système", "Caractéristiques de l'appareil et versions", about));
    content.append(&g);
    p
}

fn rename_dialog(app: &Rc<App>) {
    let entry = gtk::Entry::new();
    entry.set_text(&hostname());
    entry.set_activates_default(true);
    let d = adap::AlertDialog::new(Some("Renommer votre PC"), Some("Lettres, chiffres et tirets uniquement. Le nouveau nom sera utilisé sur le réseau."));
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Renommer")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let app2 = app.clone();
    d.connect_response(None, move |_, r| {
        let name = entry.text().trim().to_string();
        if r != "ok" || name.is_empty() {
            return;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') || name.len() > 63 {
            app2.toast("Nom invalide");
            return;
        }
        // hostnamectl passe par polkit (systemd-hostnamed) : pas besoin de pkexec.
        let plan = Ok(noos_pkg::Plan::new().user(&["hostnamectl", "hostname", &name]));
        let app3 = app2.clone();
        app2.run_plan("Renommage du PC", plan, move |_| app3.nav.replace(&[root(&app3)]));
    });
    d.present(Some(&app.window));
}

// ───────────────────────────── Écran ─────────────────────────────

const SCALES: &[f64] = &[1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 3.0];
const TRANSFORMS: &[(&str, &str)] = &[("normal", "Paysage"), ("90", "Portrait"), ("180", "Paysage (retourné)"), ("270", "Portrait (retourné)")];

fn hz(mhz: i32) -> String {
    let v = mhz as f64 / 1000.0;
    if (v - v.round()).abs() < 0.05 { format!("{:.0} Hz", v) } else { format!("{v:.2} Hz") }
}

pub fn display(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Système"], "Écran");
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 24);
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner.set_size_request(32, 32);
    holder.append(&spinner);
    content.append(&holder);

    let task = ipc(Request::GetOutputs);
    let app = app.clone();
    glib::spawn_future_local(async move {
        holder.remove(&spinner);
        match task.await {
            Ok(Event::Outputs { outputs }) if !outputs.is_empty() => {
                let selected = Rc::new(RefCell::new(0usize));
                render_display(&app, &holder, Rc::new(outputs), selected);
            }
            _ => holder.append(
                &adap::StatusPage::builder()
                    .icon_name("video-display-symbolic")
                    .title("Compositeur NOOS injoignable")
                    .description("Ouvrez les Paramètres depuis une session NOOS pour régler les écrans.")
                    .build(),
            ),
        }
    });
    p
}

fn render_display(app: &Rc<App>, holder: &gtk::Box, outputs: Rc<Vec<OutputInfo>>, selected: Rc<RefCell<usize>>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let idx = *selected.borrow();
    let o = outputs[idx].clone();

    // ── Disposition des moniteurs (vue d'ensemble cliquable) ──
    let mon_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    mon_box.add_css_class("monitor-box");
    mon_box.set_halign(gtk::Align::Fill);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    inner.set_halign(gtk::Align::Center);
    let max_w = outputs.iter().map(|o| o.width).max().unwrap_or(1).max(1) as f64;
    for (i, out) in outputs.iter().enumerate() {
        let (w, h) = if matches!(out.transform.as_str(), "90" | "270") { (out.height, out.width) } else { (out.width, out.height) };
        let k = 220.0 / max_w.max(w as f64);
        let b = gtk::Button::with_label(&(i + 1).to_string());
        b.add_css_class("monitor");
        if i != idx {
            b.add_css_class("inactive");
        }
        b.set_size_request((w as f64 * k) as i32, (h as f64 * k) as i32);
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(&format!("{} — {} {}", out.name, out.make, out.model)));
        let (app, holder, outputs, selected) = (app.clone(), holder.clone(), outputs.clone(), selected.clone());
        b.connect_clicked(move |_| {
            *selected.borrow_mut() = i;
            render_display(&app, &holder, outputs.clone(), selected.clone());
        });
        inner.append(&b);
    }
    mon_box.append(&inner);
    holder.append(&mon_box);

    let cfg = app.cfg();
    let oc = cfg.output(&o.name).cloned().unwrap_or_default();
    let title = format!("{} — {} {}{}", o.name, o.make, o.model, if o.nested { " (fenêtre de développement)" } else { "" });

    // ── Échelle et disposition ──
    let g = group("Échelle et disposition");
    g.set_description(Some(&title));
    let scale_labels: Vec<String> = SCALES
        .iter()
        .map(|s| format!("{} %{}", (s * 100.0).round(), if *s == 1.0 { " (recommandé)" } else { "" }))
        .collect();
    let cur_scale = SCALES.iter().position(|s| (s - o.scale).abs() < 0.01).unwrap_or(0);
    let scale = combo("zoom-in-symbolic", "Échelle", "Taille du texte, des applications et des autres éléments", &scale_labels, cur_scale);
    {
        let (app, name) = (app.clone(), o.name.clone());
        scale.connect_selected_notify(move |r| {
            let s = SCALES[r.selected() as usize];
            let (app2, name2) = (app.clone(), name.clone());
            let task = ipc(Request::SetScale { output: name.clone(), scale: s });
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(Event::Ok) => app2.update(|c| c.output_mut(&name2).scale = Some(s)),
                    Ok(Event::Error { message }) | Err(message) => app2.error("Échelle", &message),
                    _ => {}
                }
            });
        });
    }
    g.add(&scale);

    // Résolutions uniques, de la plus grande à la plus petite.
    let mut sizes: Vec<(i32, i32, bool)> = Vec::new();
    for m in &o.modes {
        if let Some(e) = sizes.iter_mut().find(|s| s.0 == m.width && s.1 == m.height) {
            e.2 |= m.preferred;
        } else {
            sizes.push((m.width, m.height, m.preferred));
        }
    }
    sizes.sort_by_key(|s| std::cmp::Reverse(s.0 * s.1));
    let size_labels: Vec<String> =
        sizes.iter().map(|(w, h, pref)| format!("{w} × {h}{}", if *pref { " (recommandé)" } else { "" })).collect();
    let cur_size = sizes.iter().position(|s| s.0 == o.width && s.1 == o.height).unwrap_or(0);
    let res = combo(
        "video-display-symbolic",
        "Résolution d'écran",
        if o.nested { "Mode imbriqué : redimensionnez la fenêtre de développement à la souris" } else { "" },
        &size_labels,
        cur_size,
    );
    res.set_sensitive(o.can_set_mode && sizes.len() > 1);
    {
        let (app, holder, outputs, selected, o2) = (app.clone(), holder.clone(), outputs.clone(), selected.clone(), o.clone());
        let sizes = sizes.clone();
        res.connect_selected_notify(move |r| {
            let (w, h, _) = sizes[r.selected() as usize];
            if (w, h) == (o2.width, o2.height) {
                return;
            }
            // Garde la fréquence actuelle si elle existe, sinon la plus haute.
            let refresh = o2
                .modes
                .iter()
                .filter(|m| m.width == w && m.height == h)
                .min_by_key(|m| (m.refresh - o2.refresh).abs())
                .map(|m| m.refresh)
                .unwrap_or(60_000);
            apply_mode(&app, &holder, &outputs, &selected, &o2, (w, h, refresh));
        });
    }
    g.add(&res);

    let orientation_idx = TRANSFORMS.iter().position(|(t, _)| *t == o.transform).unwrap_or(0);
    let orient = combo(
        "object-rotate-right-symbolic",
        "Orientation de l'écran",
        if o.nested { "Indisponible en mode imbriqué" } else { "" },
        &TRANSFORMS.iter().map(|(_, l)| l.to_string()).collect::<Vec<_>>(),
        orientation_idx,
    );
    orient.set_sensitive(!o.nested);
    {
        let (app, holder, outputs, selected, o2) = (app.clone(), holder.clone(), outputs.clone(), selected.clone(), o.clone());
        orient.connect_selected_notify(move |r| {
            let t = TRANSFORMS[r.selected() as usize].0;
            if t == o2.transform {
                return;
            }
            apply_transform(&app, &holder, &outputs, &selected, &o2, t);
        });
    }
    g.add(&orient);
    holder.append(&g);

    // ── Avancé ──
    let adv = group("Paramètres d'affichage avancés");
    let mut rates: Vec<i32> = o.modes.iter().filter(|m| m.width == o.width && m.height == o.height).map(|m| m.refresh).collect();
    rates.sort_by_key(|r| std::cmp::Reverse(*r));
    rates.dedup();
    let rate_labels: Vec<String> = rates.iter().map(|r| hz(*r)).collect();
    let cur_rate = rates.iter().position(|r| (r - o.refresh).abs() < 500).unwrap_or(0);
    let rate = combo("view-refresh-symbolic", "Fréquence d'actualisation", "Plus elle est élevée, plus le mouvement est fluide", &rate_labels, cur_rate);
    rate.set_sensitive(o.can_set_mode && rates.len() > 1);
    {
        let (app, holder, outputs, selected, o2) = (app.clone(), holder.clone(), outputs.clone(), selected.clone(), o.clone());
        rate.connect_selected_notify(move |r| {
            let hz = rates[r.selected() as usize];
            if (hz - o2.refresh).abs() < 500 {
                return;
            }
            apply_mode(&app, &holder, &outputs, &selected, &o2, (o2.width, o2.height, hz));
        });
    }
    adv.add(&rate);

    let vrr_items: Vec<String> =
        ["Réglage global (Jeux)", "Désactivé", "En plein écran (jeux)", "Toujours"].iter().map(|s| s.to_string()).collect();
    let vrr_idx = match oc.vrr {
        None => 0,
        Some(VrrPolicy::Off) => 1,
        Some(VrrPolicy::Fullscreen) => 2,
        Some(VrrPolicy::Always) => 3,
    };
    let vrr = combo(
        "speedometer-symbolic",
        "Fréquence d'actualisation variable",
        if o.vrr_capable {
            if o.vrr_active { "VRR actif en ce moment" } else { "FreeSync / G-Sync compatible" }
        } else {
            "Non pris en charge par cet écran ou ce GPU"
        },
        &vrr_items,
        vrr_idx,
    );
    vrr.set_sensitive(o.vrr_capable);
    {
        let (app, name) = (app.clone(), o.name.clone());
        vrr.connect_selected_notify(move |r| {
            let v = match r.selected() {
                1 => Some(VrrPolicy::Off),
                2 => Some(VrrPolicy::Fullscreen),
                3 => Some(VrrPolicy::Always),
                _ => None,
            };
            app.update(|c| c.output_mut(&name).vrr = v);
        });
    }
    adv.add(&vrr);

    let hdr = switch(
        "weather-clear-symbolic",
        "Utiliser le HDR",
        if o.hdr_capable { "Expérimental : la sortie HDR n'est pas encore implémentée" } else { "Écran ou pilote sans HDR" },
        oc.hdr.unwrap_or(cfg.gaming.hdr),
    );
    hdr.set_sensitive(o.hdr_capable);
    {
        let (app, name) = (app.clone(), o.name.clone());
        hdr.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.output_mut(&name).hdr = Some(v));
        });
    }
    adv.add(&hdr);

    let info = card("dialog-information-symbolic", "Informations sur l'écran", &format!(
        "{} × {} @ {} · format {} · {}",
        o.width,
        o.height,
        hz(o.refresh),
        o.format,
        if o.hdr_capable { "HDR" } else { "SDR" }
    ));
    adv.add(&info);
    holder.append(&adv);
}

/// Applique un mode, puis demande confirmation (rétablissement automatique après 15 s).
fn apply_mode(app: &Rc<App>, holder: &gtk::Box, outputs: &Rc<Vec<OutputInfo>>, selected: &Rc<RefCell<usize>>, o: &OutputInfo, (w, h, r): (i32, i32, i32)) {
    let old = (o.width, o.height, o.refresh);
    let name = o.name.clone();
    let task = ipc(Request::SetMode { output: name.clone(), width: w, height: h, refresh: r });
    let (app, holder, selected) = (app.clone(), holder.clone(), selected.clone());
    let _ = outputs;
    let nested = o.nested;
    glib::spawn_future_local(async move {
        match task.await {
            Ok(Event::Ok) => {
                let keep = {
                    let (app, name) = (app.clone(), name.clone());
                    move || {
                        let spec = format!("{w}x{h}@{}", (r as f64 / 1000.0).round());
                        if !nested {
                            app.update(|c| c.output_mut(&name).mode = Some(spec));
                        }
                    }
                };
                let revert = {
                    let name = name.clone();
                    move || {
                        let _ = noos_core::ipc::IpcClient::fire(&Request::SetMode { output: name.clone(), width: old.0, height: old.1, refresh: old.2 });
                    }
                };
                keep_or_revert(&app, keep, revert, move |app| reload_display(app, &holder, &selected));
            }
            Ok(Event::Error { message }) | Err(message) => {
                app.error("Changement de résolution", &message);
                reload_display(&app, &holder, &selected);
            }
            _ => {}
        }
    });
}

fn apply_transform(app: &Rc<App>, holder: &gtk::Box, outputs: &Rc<Vec<OutputInfo>>, selected: &Rc<RefCell<usize>>, o: &OutputInfo, t: &'static str) {
    let old = o.transform.clone();
    let name = o.name.clone();
    let task = ipc(Request::SetTransform { output: name.clone(), transform: t.into() });
    let (app, holder, selected) = (app.clone(), holder.clone(), selected.clone());
    let _ = outputs;
    glib::spawn_future_local(async move {
        match task.await {
            Ok(Event::Ok) => {
                let keep = {
                    let (app, name) = (app.clone(), name.clone());
                    move || app.update(|c| c.output_mut(&name).transform = Some(t.into()))
                };
                let revert = move || {
                    let _ = noos_core::ipc::IpcClient::fire(&Request::SetTransform { output: name.clone(), transform: old.clone() });
                };
                keep_or_revert(&app, keep, revert, move |app| reload_display(app, &holder, &selected));
            }
            Ok(Event::Error { message }) | Err(message) => app.error("Orientation", &message),
            _ => {}
        }
    });
}

fn reload_display(app: &Rc<App>, holder: &gtk::Box, selected: &Rc<RefCell<usize>>) {
    let task = ipc(Request::GetOutputs);
    let (app, holder, selected) = (app.clone(), holder.clone(), selected.clone());
    glib::spawn_future_local(async move {
        if let Ok(Event::Outputs { outputs }) = task.await {
            if *selected.borrow() >= outputs.len() {
                *selected.borrow_mut() = 0;
            }
            render_display(&app, &holder, Rc::new(outputs), selected);
        }
    });
}

/// Dialogue « Conserver ces paramètres d'affichage ? » avec compte à rebours.
fn keep_or_revert(app: &Rc<App>, keep: impl Fn() + 'static, revert: impl Fn() + Send + Clone + 'static, after: impl Fn(&Rc<App>) + Clone + 'static) {
    let d = adap::AlertDialog::new(Some("Conserver ces paramètres d'affichage ?"), Some("Rétablissement des paramètres précédents dans 15 secondes."));
    d.add_responses(&[("revert", "Rétablir"), ("keep", "Conserver les modifications")]);
    d.set_response_appearance("keep", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("keep"));
    d.set_close_response("revert");
    let remaining = Rc::new(std::cell::Cell::new(15));
    let decided = Rc::new(std::cell::Cell::new(false));
    {
        let (d2, remaining, decided) = (d.clone(), remaining.clone(), decided.clone());
        glib::timeout_add_seconds_local(1, move || {
            if decided.get() {
                return glib::ControlFlow::Break;
            }
            let n = remaining.get() - 1;
            remaining.set(n);
            if n <= 0 {
                d2.close();
                return glib::ControlFlow::Break;
            }
            d2.set_body(&format!("Rétablissement des paramètres précédents dans {n} seconde{}.", if n > 1 { "s" } else { "" }));
            glib::ControlFlow::Continue
        });
    }
    let app2 = app.clone();
    d.connect_response(None, move |_, r| {
        decided.set(true);
        if r == "keep" {
            keep();
            after(&app2);
        } else {
            let revert = revert.clone();
            let task = background(move || revert());
            let app3 = app2.clone();
            let after = after.clone();
            glib::spawn_future_local(async move {
                task.await;
                app3.toast("Paramètres d'affichage rétablis");
                after(&app3);
            });
        }
    });
    d.present(Some(&app.window));
}

// ───────────────────────────── Alimentation ─────────────────────────────

const TIMEOUTS: &[u32] = &[1, 2, 3, 5, 10, 15, 20, 25, 30, 45, 60, 120, 180, 240, 300, 0];

fn timeout_label(m: u32) -> String {
    match m {
        0 => "Jamais".into(),
        1 => "1 minute".into(),
        m if m < 60 => format!("{m} minutes"),
        60 => "1 heure".into(),
        m => format!("{} heures", m / 60),
    }
}

/// Profils d'alimentation via D-Bus (power-profiles-daemon ou tuned-ppd).
struct PowerProfiles {
    dest: &'static str,
    path: &'static str,
}

const PP_IFACES: &[(&str, &str)] = &[
    ("org.freedesktop.UPower.PowerProfiles", "/org/freedesktop/UPower/PowerProfiles"),
    ("net.hadess.PowerProfiles", "/net/hadess/PowerProfiles"),
];

impl PowerProfiles {
    fn detect() -> Option<(Self, String, Vec<String>)> {
        for (dest, path) in PP_IFACES {
            let get = |prop: &str| {
                Command::new("busctl").args(["--system", "get-property", dest, path, dest, prop]).output().ok().filter(|o| o.status.success())
            };
            let Some(active) = get("ActiveProfile") else { continue };
            let active = String::from_utf8_lossy(&active.stdout).trim().trim_start_matches("s ").trim_matches('"').to_string();
            let list = get("Profiles").map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
            let mut profiles: Vec<String> = Vec::new();
            let mut it = list.split('"');
            while let Some(part) = it.next() {
                if part == "Profile" {
                    it.next();
                    if let Some(p) = it.next() {
                        profiles.push(p.to_string());
                    }
                }
            }
            return Some((Self { dest, path }, active, profiles));
        }
        None
    }

    fn set(&self, profile: &str) -> bool {
        Command::new("busctl")
            .args(["--system", "set-property", self.dest, self.path, self.dest, "ActiveProfile", "s", profile])
            .status()
            .is_ok_and(|s| s.success())
    }
}

fn profile_label(p: &str) -> String {
    match p {
        "power-saver" => "Économie d'énergie".into(),
        "balanced" => "Équilibré".into(),
        "performance" => "Performances optimales".into(),
        o => o.into(),
    }
}

fn battery() -> Option<(f64, String, String)> {
    let out = Command::new("upower").args(["-i", "/org/freedesktop/UPower/devices/DisplayDevice"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(&format!("{k}:")).map(|v| v.trim().to_string()));
    if field("power supply").as_deref() != Some("yes") {
        return None;
    }
    let pct = field("percentage")?.trim_end_matches('%').replace(',', ".").parse().ok()?;
    let state = match field("state").as_deref() {
        Some("charging") => "En charge",
        Some("discharging") => "Sur batterie",
        Some("fully-charged") => "Chargée",
        _ => "",
    };
    let time = field("time to empty").or_else(|| field("time to full")).unwrap_or_default();
    Some((pct, state.to_string(), time))
}

pub fn power(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Système"], "Alimentation");
    let cfg = app.cfg();

    let (icon_name, title, sub) = match battery() {
        Some((pct, state, time)) => (
            if pct > 80.0 { "battery-full-symbolic" } else if pct > 40.0 { "battery-good-symbolic" } else { "battery-low-symbolic" },
            format!("{pct:.0} %"),
            format!("{state}{}", if time.is_empty() { String::new() } else { format!(" · {time}") }),
        ),
        None => ("ac-adapter-symbolic", "Branché sur le secteur".to_string(), "Aucune batterie détectée".to_string()),
    };
    let img = gtk::Image::from_icon_name(icon_name);
    img.set_pixel_size(64);
    let (h, _) = hero(&img, &title, &sub);
    content.append(&h);

    let mode = group("Mode d'alimentation");
    let placeholder = card("emblem-synchronizing-symbolic", "Lecture des profils…", "");
    mode.add(&placeholder);
    let task = background(PowerProfiles::detect);
    {
        let (app, mode) = (app.clone(), mode.clone());
        glib::spawn_future_local(async move {
            let result = task.await;
            mode.remove(&placeholder);
            match result {
                Some((pp, active, profiles)) if !profiles.is_empty() => {
                    let labels: Vec<String> = profiles.iter().map(|p| profile_label(p)).collect();
                    let idx = profiles.iter().position(|p| *p == active).unwrap_or(0);
                    let row = combo("power-profile-balanced-symbolic", "Mode d'alimentation", "Optimise l'appareil selon la consommation et les performances", &labels, idx);
                    let pp = Rc::new(pp);
                    row.connect_selected_notify(move |r| {
                        let name = profiles[r.selected() as usize].clone();
                        if !pp.set(&name) {
                            app.toast("Impossible de changer le mode d'alimentation");
                        }
                    });
                    mode.add(&row);
                }
                _ => {
                    mode.add(&card("dialog-warning-symbolic", "Modes d'alimentation indisponibles", "Installez power-profiles-daemon ou tuned-ppd"));
                }
            }
        });
    }
    content.append(&mode);

    let sleep = group("Écran et veille");
    let labels: Vec<String> = TIMEOUTS.iter().map(|m| timeout_label(*m)).collect();
    let pos = |v: u32| TIMEOUTS.iter().position(|m| *m == v).unwrap_or(4);
    let off = combo("video-display-symbolic", "Éteindre mon écran après", "", &labels, pos(cfg.power.screen_off_minutes));
    {
        let app = app.clone();
        off.connect_selected_notify(move |r| {
            let v = TIMEOUTS[r.selected() as usize];
            app.update(|c| c.power.screen_off_minutes = v);
        });
    }
    sleep.add(&off);
    let susp = combo("weather-clear-night-symbolic", "Mettre mon appareil en veille après", "", &labels, pos(cfg.power.suspend_minutes));
    {
        let app = app.clone();
        susp.connect_selected_notify(move |r| {
            let v = TIMEOUTS[r.selected() as usize];
            app.update(|c| c.power.suspend_minutes = v);
        });
    }
    sleep.add(&susp);
    let inhibit = switch(
        "input-gaming-symbolic",
        "Garder l'écran allumé en plein écran",
        "Jeux et vidéos en plein écran empêchent l'extinction (les lecteurs vidéo le demandent aussi d'eux-mêmes)",
        cfg.power.inhibit_fullscreen,
    );
    {
        let app = app.clone();
        inhibit.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.power.inhibit_fullscreen = v);
        });
    }
    sleep.add(&inhibit);
    content.append(&sleep);

    let actions = group("Actions");
    for (icon, title, button, cmd) in [
        ("weather-clear-night-symbolic", "Mettre en veille maintenant", "Mettre en veille", "suspend"),
        ("system-reboot-symbolic", "Redémarrer l'appareil", "Redémarrer", "reboot"),
        ("system-shutdown-symbolic", "Arrêter l'appareil", "Arrêter", "poweroff"),
    ] {
        let r = card(icon, title, "");
        let b = gtk::Button::with_label(button);
        b.set_valign(gtk::Align::Center);
        if cmd != "suspend" {
            b.add_css_class("destructive-action");
        }
        let app2 = app.clone();
        b.connect_clicked(move |_| {
            let go = move || {
                let _ = Command::new("systemctl").arg(cmd).spawn();
            };
            if cmd == "suspend" {
                go();
            } else {
                app2.confirm(&format!("{button} ?"), "Les applications ouvertes seront fermées.", button, true, go);
            }
        });
        r.add_suffix(&b);
        actions.add(&r);
    }
    content.append(&actions);
    p
}

// ───────────────────────────── Informations système ─────────────────────────────

fn cpu_model() -> String {
    read("/proc/cpuinfo").lines().find_map(|l| l.strip_prefix("model name").map(|v| v.trim_start_matches([' ', '\t', ':']).to_string())).unwrap_or_default()
}

fn memory() -> String {
    read("/proc/meminfo")
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse::<f64>().ok())
        .map(|kb| format!("{:.1} Go", kb / 1024.0 / 1024.0))
        .unwrap_or_default()
}

fn gpus() -> String {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/class/drm") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("card") && !n.contains('-') {
                let uevent = read(&format!("/sys/class/drm/{n}/device/uevent"));
                if let Some(d) = uevent.lines().find_map(|l| l.strip_prefix("DRIVER=")) {
                    let vendor = match read(&format!("/sys/class/drm/{n}/device/vendor")).as_str() {
                        "0x8086" => "Intel",
                        "0x1002" => "AMD",
                        "0x10de" => "NVIDIA",
                        _ => "",
                    };
                    v.push(format!("{vendor} ({d})").trim().to_string());
                }
            }
        }
    }
    v.sort();
    v.dedup();
    v.join(", ")
}

pub fn about(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Système"], "Informations système");
    let logo = gtk::Picture::for_filename(paths::data_dir().join("noos-logo.svg"));
    logo.set_can_shrink(true);
    logo.set_content_fit(gtk::ContentFit::Contain);
    logo.set_size_request(240, 80);
    let (h, actions) = hero(&logo, &hostname(), &device_model());
    let copy = gtk::Button::with_label("Copier");
    copy.add_css_class("pill");
    actions.append(&copy);
    content.append(&h);

    let dev = group("Caractéristiques de l'appareil");
    let specs = vec![
        ("computer-symbolic", "Nom de l'appareil", hostname()),
        ("cpu-symbolic", "Processeur", cpu_model()),
        ("media-flash-symbolic", "Mémoire vive installée", memory()),
        ("video-display-symbolic", "Carte graphique", gpus()),
        ("computer-symbolic", "Modèle", device_model()),
    ];
    let mut text = String::new();
    for (icon, title, value) in &specs {
        if value.is_empty() {
            continue;
        }
        let r = card(icon, title, value);
        r.set_subtitle_selectable(true);
        dev.add(&r);
        text.push_str(&format!("{title} : {value}\n"));
    }
    content.append(&dev);

    let sys = group("Caractéristiques du système");
    let kernel = Command::new("uname").arg("-r").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let info = vec![
        ("distributor-logo-symbolic", "Système d'exploitation", os_name()),
        ("applications-system-symbolic", "Noyau Linux", kernel),
        ("preferences-desktop-symbolic", "Environnement", format!("NOOS {}", env!("CARGO_PKG_VERSION"))),
        (
            "applications-graphics-symbolic",
            "Interface",
            format!(
                "GTK {}.{}.{} · libAdapta {}.{}.{}",
                gtk::major_version(),
                gtk::minor_version(),
                gtk::micro_version(),
                adap::major_version(),
                adap::minor_version(),
                adap::micro_version()
            ),
        ),
        ("folder-symbolic", "Configuration", paths::config_file().to_string_lossy().to_string()),
    ];
    for (icon, title, value) in &info {
        let r = card(icon, title, value);
        r.set_subtitle_selectable(true);
        sys.add(&r);
        text.push_str(&format!("{title} : {value}\n"));
    }
    content.append(&sys);
    {
        let app = app.clone();
        copy.connect_clicked(move |_| {
            if let Some(d) = gtk::gdk::Display::default() {
                d.clipboard().set_text(&text);
            }
            app.toast("Caractéristiques copiées");
        });
    }

    let credits = group("");
    let r = card("help-about-symbolic", "À propos de NOOS", "Crédits et licence");
    r.set_activatable(true);
    r.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    {
        let window = app.window.clone();
        r.connect_activated(move |_| {
            let about = adap::AboutDialog::builder()
                .application_name("NOOS")
                .version(env!("CARGO_PKG_VERSION"))
                .developer_name("Axel Valens")
                .comments("Environnement de bureau Wayland en Rust : compositeur Smithay, interface GTK4 + libAdapta.")
                .license_type(gtk::License::Gpl30)
                .build();
            about.add_credit_section(Some("Construit avec"), &["Smithay", "GTK4 / gtk-rs", "libAdapta (Linux Mint)", "gtk4-layer-shell"]);
            about.present(Some(&window));
        });
    }
    credits.add(&r);
    content.append(&credits);
    p
}

pub fn notifications(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Système"], "Notifications");
    let cfg = app.cfg();
    let n = &cfg.notifications;
    let g = group("");
    let dnd = switch(
        "notifications-disabled-symbolic",
        "Ne pas déranger",
        "Les notifications vont directement au centre de notifications, sans bannière (sauf les urgentes)",
        n.do_not_disturb,
    );
    {
        let app = app.clone();
        dnd.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.notifications.do_not_disturb = v);
        });
    }
    g.add(&dnd);
    let sound = switch("audio-volume-high-symbolic", "Son", "Jouer un son à l'arrivée d'une notification", n.sound);
    {
        let app = app.clone();
        sound.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.notifications.sound = v);
        });
    }
    g.add(&sound);
    let secs = [3u32, 5, 6, 10, 15, 30];
    let durations: Vec<String> = secs.iter().map(|s| format!("{s} secondes")).collect();
    let current = secs.iter().position(|s| *s == n.banner_seconds).unwrap_or(2);
    let duration = combo("preferences-system-time-symbolic", "Durée d'affichage des bannières", "Avant qu'elles rejoignent le centre de notifications", &durations, current);
    {
        let app = app.clone();
        duration.connect_selected_notify(move |r| {
            let v = secs[(r.selected() as usize).min(secs.len() - 1)];
            app.update(|c| c.notifications.banner_seconds = v);
        });
    }
    g.add(&duration);
    content.append(&g);

    let test = group("");
    let row = card("preferences-system-notifications-symbolic", "Essayer", "Envoie une notification de test");
    let button = gtk::Button::with_label("Envoyer");
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(|_| {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "Paramètres", "-i", "preferences-system", "Notification de test", "Les notifications NOOS fonctionnent."])
            .spawn();
    });
    row.add_suffix(&button);
    test.add(&row);
    content.append(&test);
    p
}

//! Assistant « Ajouter un appareil » (recherche, association, terminé / échec) et fenêtres de
//! l'agent d'appairage (code à confirmer, code PIN à saisir, code à taper sur un clavier).

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use adap::prelude::*;
use gtk::glib;

use super::{
    bluez::{AgentRequest, Bluetooth, Device},
    themed,
};
use crate::ui::App;

thread_local! {
    /// Fenêtre de l'agent affichée (fermée si BlueZ annule la demande).
    static AGENT_DIALOG: RefCell<Option<adap::AlertDialog>> = const { RefCell::new(None) };
}

fn close_agent_dialog() {
    if let Some(d) = AGENT_DIALOG.with(|a| a.borrow_mut().take()) {
        d.force_close();
    }
}

/// « 482913 » → « 482 913 ».
pub fn format_code(code: &str) -> String {
    let digits: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() == 6 && digits.chars().all(|c| c.is_ascii_digit()) {
        format!("{} {}", &digits[..3], &digits[3..])
    } else {
        digits
    }
}

fn code_label(code: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(&format_code(code)));
    l.add_css_class("bt-code");
    l.set_selectable(true);
    l
}

/// Branche les demandes de l'agent BlueZ sur des fenêtres de Paramètres.
pub fn install_agent_ui(app: &Rc<App>) {
    let window = app.window.clone();
    Bluetooth::get().set_agent_ui(move |req| {
        close_agent_dialog();
        let d = match req {
            AgentRequest::Cancel => return,
            AgentRequest::Confirm { device, passkey, reply } => {
                let d = adap::AlertDialog::new(
                    Some(&format!("Associer « {device} » ?")),
                    Some("Vérifiez que ce code s'affiche aussi sur l'appareil, puis acceptez sur les deux."),
                );
                d.set_extra_child(Some(&code_label(&format!("{passkey:06}"))));
                d.add_responses(&[("no", "Ne correspond pas"), ("ok", "Associer")]);
                d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
                d.set_default_response(Some("ok"));
                d.set_close_response("no");
                d.connect_response(None, move |_, r| if r == "ok" { reply.accept() } else { reply.reject() });
                d
            }
            AgentRequest::PinCode { device, reply } => pin_dialog(
                &format!("Code PIN de « {device} »"),
                "Saisissez le code PIN de l'appareil. Pour la plupart des casques et des enceintes anciens, c'est 0000 ou 1234 (voir sa notice).",
                "0000",
                false,
                move |v| match v {
                    Some(pin) => reply.pin(&pin),
                    None => reply.reject(),
                },
            ),
            AgentRequest::Passkey { device, reply } => pin_dialog(
                &format!("Code de « {device} »"),
                "Saisissez le code à 6 chiffres affiché sur l'appareil.",
                "",
                true,
                move |v| match v.and_then(|s| s.parse::<u32>().ok()) {
                    Some(k) => reply.passkey(k),
                    None => reply.reject(),
                },
            ),
            AgentRequest::Display { device, code, entered } => {
                let d = adap::AlertDialog::new(
                    Some(&format!("Tapez ce code sur « {device} »")),
                    Some("Tapez les chiffres sur le clavier à associer, puis appuyez sur Entrée."),
                );
                let v = gtk::Box::new(gtk::Orientation::Vertical, 8);
                v.append(&code_label(&code));
                if let Some(n) = entered {
                    let p = gtk::Label::new(Some(&format!("{n} chiffre{} tapé{}", if n > 1 { "s" } else { "" }, if n > 1 { "s" } else { "" })));
                    p.add_css_class("dim-label");
                    v.append(&p);
                }
                d.set_extra_child(Some(&v));
                d.add_response("close", "Masquer");
                d
            }
            AgentRequest::Authorize { device, service, reply } => {
                let body = match service {
                    Some(s) => format!("« {device} » demande à utiliser ce PC pour : {s}."),
                    None => format!("« {device} » demande à s'associer à ce PC."),
                };
                let d = adap::AlertDialog::new(Some("Autoriser cet appareil ?"), Some(&body));
                d.add_responses(&[("no", "Refuser"), ("ok", "Autoriser")]);
                d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
                d.set_close_response("no");
                d.connect_response(None, move |_, r| if r == "ok" { reply.accept() } else { reply.reject() });
                d
            }
        };
        {
            let d2 = d.clone();
            d.connect_closed(move |_| {
                AGENT_DIALOG.with(|a| {
                    let mut a = a.borrow_mut();
                    if a.as_ref() == Some(&d2) {
                        *a = None;
                    }
                })
            });
        }
        d.present(Some(&window));
        AGENT_DIALOG.with(|a| *a.borrow_mut() = Some(d));
    });
}

fn pin_dialog(title: &str, body: &str, initial: &str, numeric: bool, done: impl Fn(Option<String>) + 'static) -> adap::AlertDialog {
    let d = adap::AlertDialog::new(Some(title), Some(body));
    let e = gtk::Entry::new();
    e.set_text(initial);
    e.set_activates_default(true);
    e.set_max_length(if numeric { 6 } else { 16 });
    e.add_css_class("bt-pin-entry");
    if numeric {
        e.set_input_purpose(gtk::InputPurpose::Digits);
    }
    d.set_extra_child(Some(&e));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Associer")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    d.set_close_response("cancel");
    {
        let d = d.clone();
        e.connect_changed(move |e| {
            let t = e.text();
            let ok = if numeric { t.len() == 6 && t.chars().all(|c| c.is_ascii_digit()) } else { !t.is_empty() };
            d.set_response_enabled("ok", ok);
        });
    }
    if numeric {
        d.set_response_enabled("ok", false);
    }
    d.connect_response(None, move |_, r| done((r == "ok").then(|| e.text().trim().to_string())));
    d
}

/// Indicateur de signal d'après le RSSI (dBm).
fn signal_icon(rssi: Option<i16>) -> (&'static str, &'static str) {
    match rssi {
        Some(r) if r >= -55 => ("network-cellular-signal-excellent-symbolic", "Signal excellent"),
        Some(r) if r >= -67 => ("network-cellular-signal-good-symbolic", "Bon signal"),
        Some(r) if r >= -80 => ("network-cellular-signal-ok-symbolic", "Signal moyen"),
        Some(_) => ("network-cellular-signal-weak-symbolic", "Signal faible : rapprochez l'appareil"),
        None => ("network-cellular-signal-none-symbolic", "Signal inconnu"),
    }
}

fn centered(spacing: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    b.set_margin_start(32);
    b.set_margin_end(32);
    b.set_margin_top(24);
    b.set_margin_bottom(24);
    b
}

fn big_icon(name: &str, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("bt-big-icon");
    b.set_vexpand(false);
    if !class.is_empty() {
        b.add_css_class(class);
    }
    b.set_halign(gtk::Align::Center);
    let i = gtk::Image::from_icon_name(name);
    i.set_pixel_size(44);
    i.set_vexpand(true);
    b.append(&i);
    b
}

fn label(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_justify(gtk::Justification::Center);
    l.set_max_width_chars(46);
    if !class.is_empty() {
        l.add_css_class(class);
    }
    l
}

/// Ouvre l'assistant : lance la recherche, l'arrête à la fermeture.
pub fn open(app: &Rc<App>) {
    let bt = Bluetooth::get();
    let dialog = adap::Dialog::new();
    dialog.set_title("Ajouter un appareil");
    dialog.set_content_width(560);
    dialog.set_content_height(600);
    let tv = adap::ToolbarView::new();
    let hb = adap::HeaderBar::new();
    tv.add_top_bar(&hb);
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
    let head = gtk::Label::new(Some("Ajouter un appareil Bluetooth"));
    head.add_css_class("title-2");
    head.set_xalign(0.0);
    search.append(&head);
    let tip = gtk::Label::new(Some(
        "Allumez l'appareil et mettez-le en mode association : en général, maintenez son bouton Bluetooth ou d'alimentation jusqu'à ce que son voyant clignote.",
    ));
    tip.set_wrap(true);
    tip.set_xalign(0.0);
    tip.add_css_class("dim-label");
    search.append(&tip);
    let state = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let spinner = gtk::Spinner::new();
    spinner.start();
    state.append(&spinner);
    let state_label = gtk::Label::new(Some("Recherche d'appareils à proximité…"));
    state_label.set_xalign(0.0);
    state_label.set_hexpand(true);
    state.append(&state_label);
    let restart = gtk::Button::with_label("Relancer la recherche");
    restart.add_css_class("flat");
    restart.set_visible(false);
    state.append(&restart);
    search.append(&state);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.set_valign(gtk::Align::Start);
    let scroll = gtk::ScrolledWindow::builder().child(&list).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
    let list_stack = gtk::Stack::new();
    list_stack.set_vexpand(true);
    list_stack.add_named(&scroll, Some("list"));
    let waiting = centered(10);
    waiting.append(&big_icon("bluetooth-active-symbolic", "pulse"));
    waiting.append(&label("Les appareils apparaîtront ici dès qu'ils seront détectés.", "dim-label"));
    list_stack.add_named(&waiting, Some("waiting"));
    let nothing = centered(10);
    nothing.append(&big_icon("bluetooth-symbolic", "off"));
    nothing.append(&label("Aucun appareil trouvé", "title-4"));
    nothing.append(&label(
        "Vérifiez que l'appareil est allumé, chargé, à moins de quelques mètres et en mode association. S'il est déjà connecté à un téléphone, déconnectez-le d'abord.",
        "dim-label",
    ));
    list_stack.add_named(&nothing, Some("nothing"));
    search.append(&list_stack);
    let unnamed = gtk::CheckButton::with_label("Afficher les appareils sans nom");
    unnamed.add_css_class("caption");
    search.append(&unnamed);
    stack.add_named(&search, Some("search"));

    // ── Bluetooth désactivé ──
    let off = centered(12);
    off.append(&big_icon("bluetooth-disabled-symbolic", "off"));
    off.append(&label("Le Bluetooth est désactivé", "title-3"));
    off.append(&label("Activez-le pour rechercher des appareils à proximité.", "dim-label"));
    let enable = gtk::Button::with_label("Activer le Bluetooth");
    enable.add_css_class("suggested-action");
    enable.add_css_class("pill");
    enable.set_halign(gtk::Align::Center);
    off.append(&enable);
    stack.add_named(&off, Some("off"));

    // ── Association en cours ──
    let pairing = centered(14);
    let pairing_icon = gtk::Image::from_icon_name("bluetooth-symbolic");
    pairing_icon.set_pixel_size(44);
    pairing_icon.set_vexpand(true);
    let pi_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pi_box.add_css_class("bt-big-icon");
    pi_box.add_css_class("pulse");
    pi_box.set_vexpand(false);
    pi_box.set_halign(gtk::Align::Center);
    pi_box.append(&pairing_icon);
    pairing.append(&pi_box);
    let pairing_name = label("", "title-2");
    pairing.append(&pairing_name);
    let ps = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    ps.set_halign(gtk::Align::Center);
    let sp2 = gtk::Spinner::new();
    sp2.start();
    ps.append(&sp2);
    ps.append(&label("Association en cours…", ""));
    pairing.append(&ps);
    pairing.append(&label("Si un code s'affiche, vérifiez qu'il est identique sur l'appareil. Acceptez aussi sur l'appareil s'il le demande.", "dim-label"));
    let cancel = gtk::Button::with_label("Annuler");
    cancel.add_css_class("pill");
    cancel.set_halign(gtk::Align::Center);
    pairing.append(&cancel);
    stack.add_named(&pairing, Some("pairing"));

    // ── Terminé ──
    let done = centered(14);
    done.append(&big_icon("object-select-symbolic", "success"));
    done.append(&label("Votre appareil est prêt !", "title-2"));
    let done_name = label("", "dim-label");
    done.append(&done_name);
    let db = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    db.set_halign(gtk::Align::Center);
    let another = gtk::Button::with_label("Ajouter un autre appareil");
    another.add_css_class("pill");
    let finish = gtk::Button::with_label("Terminé");
    finish.add_css_class("pill");
    finish.add_css_class("suggested-action");
    db.append(&another);
    db.append(&finish);
    done.append(&db);
    stack.add_named(&done, Some("done"));

    // ── Échec ──
    let fail = centered(14);
    fail.append(&big_icon("dialog-warning-symbolic", "warning"));
    fail.append(&label("L'association a échoué", "title-2"));
    let fail_msg = label("", "dim-label");
    fail.append(&fail_msg);
    let fb = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    fb.set_halign(gtk::Align::Center);
    let back = gtk::Button::with_label("Retour à la liste");
    back.add_css_class("pill");
    let retry = gtk::Button::with_label("Réessayer");
    retry.add_css_class("pill");
    retry.add_css_class("suggested-action");
    fb.append(&back);
    fb.append(&retry);
    fail.append(&fb);
    stack.add_named(&fail, Some("fail"));

    // ── Logique ──
    let rows: Rc<RefCell<HashMap<String, (gtk::ListBoxRow, adap::ActionRow, gtk::Image)>>> = Rc::default();
    let current: Rc<RefCell<Option<Device>>> = Rc::default();
    let started = Rc::new(Cell::new(glib::monotonic_time()));
    let closed = Rc::new(Cell::new(false));

    let pair: Rc<dyn Fn(Device)> = {
        let (app, bt, stack, current, closed) = (app.clone(), bt.clone(), stack.clone(), current.clone(), closed.clone());
        let (pairing_icon, pairing_name, done_name, fail_msg) = (pairing_icon.clone(), pairing_name.clone(), done_name.clone(), fail_msg.clone());
        Rc::new(move |d: Device| {
            *current.borrow_mut() = Some(d.clone());
            pairing_icon.set_icon_name(Some(&themed(d.kind().icon, "bluetooth-symbolic")));
            pairing_name.set_text(&d.name);
            stack.set_visible_child_name("pairing");
            let (app, bt, stack, closed, done_name, fail_msg) = (app.clone(), bt.clone(), stack.clone(), closed.clone(), done_name.clone(), fail_msg.clone());
            glib::spawn_future_local(async move {
                // La recherche ralentit l'association : arrêtée pendant ce temps.
                let _ = bt.discovery(false).await;
                let r = bt.pair(&d.path).await;
                close_agent_dialog();
                if closed.get() {
                    return;
                }
                match r {
                    Ok(()) => {
                        done_name.set_text(&format!("« {} » est associé{}.", d.name, if bt.model().device(&d.path).is_some_and(|x| x.connected) { " et connecté" } else { "" }));
                        stack.set_visible_child_name("done");
                        app.toast(&format!("« {} » est associé", d.name));
                    }
                    Err(e) => {
                        fail_msg.set_text(&e);
                        stack.set_visible_child_name("fail");
                    }
                }
            });
        })
    };

    let render = {
        let (bt, list, list_stack, rows, unnamed, stack, started) = (bt.clone(), list.clone(), list_stack.clone(), rows.clone(), unnamed.clone(), stack.clone(), started.clone());
        let (state_label, spinner, restart, pair) = (state_label.clone(), spinner.clone(), restart.clone(), pair.clone());
        move || {
            let m = bt.model();
            let Some(a) = m.adapter.clone() else { return };
            let page = stack.visible_child_name().unwrap_or_default();
            if !a.powered {
                if page == "search" {
                    stack.set_visible_child_name("off");
                }
                return;
            }
            if page == "off" {
                stack.set_visible_child_name("search");
            }
            spinner.set_spinning(a.discovering);
            restart.set_visible(!a.discovering);
            state_label.set_text(if a.discovering { "Recherche d'appareils à proximité…" } else { "Recherche terminée" });
            let show_unnamed = unnamed.is_active();
            let nearby: Vec<Device> = m.nearby().into_iter().filter(|d| show_unnamed || d.named).collect();
            let hidden = m.nearby().len() - nearby.len();
            unnamed.set_label(Some(&if hidden > 0 {
                format!("Afficher les appareils sans nom ({hidden})")
            } else {
                "Afficher les appareils sans nom".to_string()
            }));
            let mut map = rows.borrow_mut();
            map.retain(|path, (row, _, _)| {
                let keep = nearby.iter().any(|d| &d.path == path);
                if !keep {
                    list.remove(row);
                }
                keep
            });
            for d in &nearby {
                let kind = d.kind();
                let (sig, tip) = signal_icon(d.rssi);
                if let Some((_, ar, si)) = map.get(&d.path) {
                    ar.set_title(&d.name);
                    si.set_icon_name(Some(sig));
                    si.set_tooltip_text(Some(tip));
                    continue;
                }
                let ar = adap::ActionRow::new();
                ar.set_use_markup(false);
                ar.set_title(&d.name);
                ar.set_subtitle(if d.named { kind.label } else { "Appareil sans nom" });
                ar.set_activatable(true);
                let ib = gtk::Box::new(gtk::Orientation::Vertical, 0);
                ib.add_css_class("bt-tile-icon");
                ib.set_valign(gtk::Align::Center);
                ib.set_vexpand(false);
                let i = gtk::Image::from_icon_name(&themed(kind.icon, "bluetooth-symbolic"));
                i.set_pixel_size(20);
                i.set_vexpand(true);
                ib.append(&i);
                ar.add_prefix(&ib);
                let si = gtk::Image::from_icon_name(sig);
                si.set_tooltip_text(Some(tip));
                si.add_css_class("dim-label");
                ar.add_suffix(&si);
                ar.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                {
                    let (bt, pair, path) = (bt.clone(), pair.clone(), d.path.clone());
                    ar.connect_activated(move |_| {
                        if let Some(d) = bt.model().device(&path).cloned() {
                            pair(d);
                        }
                    });
                }
                list.append(&ar);
                map.insert(d.path.clone(), (ar.clone().upcast(), ar, si));
            }
            let elapsed = (glib::monotonic_time() - started.get()) / 1_000_000;
            list_stack.set_visible_child_name(if !map.is_empty() {
                "list"
            } else if a.discovering && elapsed < 25 {
                "waiting"
            } else {
                "nothing"
            });
        }
    };
    let render: Rc<dyn Fn()> = Rc::new(render);

    let start = {
        let (app, bt, render, started) = (app.clone(), bt.clone(), render.clone(), started.clone());
        Rc::new(move || {
            started.set(glib::monotonic_time());
            let (app, bt, render) = (app.clone(), bt.clone(), render.clone());
            glib::spawn_future_local(async move {
                if let Err(e) = bt.discovery(true).await {
                    app.toast(&format!("Recherche impossible : {e}"));
                }
                render();
            });
        })
    };

    stack.set_visible_child_name(if bt.model().adapter.is_some_and(|a| a.powered) { "search" } else { "off" });
    render();
    start();
    {
        let render = render.clone();
        let weak = dialog.downgrade();
        let closed = closed.clone();
        bt.subscribe(move || {
            if weak.upgrade().is_none() || closed.get() {
                return false;
            }
            render();
            true
        });
    }
    // Rafraîchit l'état vide (« Aucun appareil trouvé » au bout de 25 s) et relance la
    // recherche si BlueZ l'a arrêtée d'elle-même.
    {
        let (render, closed) = (render.clone(), closed.clone());
        glib::timeout_add_seconds_local(5, move || {
            if closed.get() {
                return glib::ControlFlow::Break;
            }
            render();
            glib::ControlFlow::Continue
        });
    }
    {
        let render = render.clone();
        unnamed.connect_toggled(move |_| render());
    }
    {
        let start = start.clone();
        restart.connect_clicked(move |_| start());
    }
    {
        let (app, bt, start) = (app.clone(), bt.clone(), start.clone());
        enable.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (app, bt, start, b) = (app.clone(), bt.clone(), start.clone(), b.clone());
            glib::spawn_future_local(async move {
                match bt.set_powered(true).await {
                    Ok(()) => start(),
                    Err(e) => app.error("Activer le Bluetooth", &e),
                }
                b.set_sensitive(true);
            });
        });
    }
    {
        let (bt, current) = (bt.clone(), current.clone());
        cancel.connect_clicked(move |_| {
            close_agent_dialog();
            if let Some(d) = current.borrow().clone() {
                let bt = bt.clone();
                glib::spawn_future_local(async move { bt.cancel_pairing(&d.path).await });
            }
        });
    }
    {
        let (stack, start) = (stack.clone(), start.clone());
        back.connect_clicked(move |_| {
            stack.set_visible_child_name("search");
            start();
        });
    }
    {
        let (stack, start) = (stack.clone(), start.clone());
        another.connect_clicked(move |_| {
            stack.set_visible_child_name("search");
            start();
        });
    }
    {
        let (current, pair) = (current.clone(), pair.clone());
        retry.connect_clicked(move |_| {
            if let Some(d) = current.borrow().clone() {
                pair(d);
            }
        });
    }
    {
        let dialog = dialog.clone();
        finish.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let (bt, closed) = (bt.clone(), closed.clone());
        dialog.connect_closed(move |_| {
            closed.set(true);
            let bt = bt.clone();
            glib::spawn_future_local(async move {
                let _ = bt.discovery(false).await;
            });
        });
    }
    dialog.present(Some(&app.window));
    // Démo : association du premier appareil trouvé (code à confirmer).
    if super::bluez::demo_mode() == "pair" || super::bluez::demo_mode() == "pin" {
        let (bt, pair) = (bt.clone(), pair.clone());
        let want = if super::bluez::demo_mode() == "pin" { "JBL Flip 6" } else { "iPhone de Camille" };
        glib::timeout_add_local_once(Duration::from_millis(3000), move || {
            if let Some(d) = bt.model().devices.iter().find(|d| d.name == want).cloned() {
                pair(d);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn code() {
        assert_eq!(super::format_code("482913"), "482 913");
        assert_eq!(super::format_code("0000"), "0000");
    }
}

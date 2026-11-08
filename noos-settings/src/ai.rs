//! Paramètres › IA : assistant local (tout reste sur le PC) — intégrations, dictée
//! (touche push-to-talk), modèle de langage selon le matériel, ressources, applications
//! exposant des actions, confidentialité.

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use noos_core::ai::{self, Hardware};

use crate::ui::{App, background, card, combo, group, page, spin, switch};

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "IA");
    let cfg = app.cfg().ai;

    // ── Héros ──
    let hero = gtk::Box::new(gtk::Orientation::Horizontal, 22);
    hero.add_css_class("win11-hero");
    let icon = gtk::Image::from_icon_name("starred-symbolic");
    icon.set_pixel_size(40);
    icon.add_css_class("ai-hero-icon");
    icon.set_valign(gtk::Align::Center);
    hero.append(&icon);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
    v.set_hexpand(true);
    v.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some("Assistant IA local"));
    t.add_css_class("title-2");
    t.set_xalign(0.0);
    v.append(&t);
    let s = gtk::Label::new(Some("Recherche de fichiers, dictée et commandes du bureau. Tout fonctionne sur ce PC : rien n'est envoyé sur Internet."));
    s.add_css_class("dim-label");
    s.set_wrap(true);
    s.set_xalign(0.0);
    v.append(&s);
    let hw_label = gtk::Label::new(Some("Détection du matériel…"));
    hw_label.add_css_class("caption");
    hw_label.set_xalign(0.0);
    hw_label.set_margin_top(4);
    v.append(&hw_label);
    hero.append(&v);
    let master = gtk::Switch::new();
    master.set_active(cfg.enabled);
    master.set_valign(gtk::Align::Center);
    hero.append(&master);
    content.append(&hero);

    // Tout le reste dépend de l'interrupteur général.
    let body = gtk::Box::new(gtk::Orientation::Vertical, 24);
    body.set_sensitive(cfg.enabled);
    content.append(&body);
    {
        let (app, body) = (app.clone(), body.clone());
        master.connect_active_notify(move |s| {
            let on = s.is_active();
            body.set_sensitive(on);
            app.update(|c| c.ai.enabled = on);
        });
    }

    // ── Intégrations ──
    let g = group("Intégrations");
    for (icon, title, sub, get, set) in [
        (
            "system-run-symbolic",
            "Assistant et commandes",
            "« Monte le son à 90 % », « ouvre les paramètres réseau », « mets à jour mon système »",
            cfg.assistant,
            (|c: &mut noos_core::NoosConfig, v| c.ai.assistant = v) as fn(&mut noos_core::NoosConfig, bool),
        ),
        ("system-search-symbolic", "Recherche de fichiers", "Retrouver un document en le décrivant (« la facture EDF de mars »)", cfg.file_search, |c, v| c.ai.file_search = v),
        ("audio-input-microphone-symbolic", "Dictée vocale", "Maintenir la touche de dictée pour écrire avec la voix dans n'importe quel champ", cfg.dictation, |c, v| c.ai.dictation = v),
    ] {
        let r = switch(icon, title, sub, get);
        let app = app.clone();
        r.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| set(c, v));
        });
        g.add(&r);
    }
    body.append(&g);

    // ── Dictée ──
    let g = group("Dictée");
    let key_row = card("input-keyboard-symbolic", "Touche de dictée", "Maintenir pour parler, relâcher pour écrire le texte");
    let key_btn = gtk::Button::with_label(&key_label(&cfg.ptt_key));
    key_btn.add_css_class("ai-key");
    key_btn.set_valign(gtk::Align::Center);
    key_btn.set_tooltip_text(Some("Cliquer puis appuyer sur la nouvelle combinaison"));
    {
        let app = app.clone();
        key_btn.connect_clicked(move |b| capture_key(&app, b));
    }
    key_row.add_suffix(&key_btn);
    g.add(&key_row);
    const LANGS: [(&str, &str); 6] = [("fr", "Français"), ("en", "Anglais"), ("es", "Espagnol"), ("de", "Allemand"), ("it", "Italien"), ("auto", "Détection automatique")];
    let lang = combo(
        "preferences-desktop-locale-symbolic",
        "Langue parlée",
        "",
        &LANGS.map(|(_, n)| n.to_string()),
        LANGS.iter().position(|(c, _)| *c == cfg.dictation_language).unwrap_or(0),
    );
    {
        let app = app.clone();
        lang.connect_selected_notify(move |r| {
            let code = LANGS[r.selected() as usize].0.to_string();
            app.update(|c| c.ai.dictation_language = code);
        });
    }
    g.add(&lang);
    const SPEECH: [(&str, &str); 2] = [("base", "Rapide (140 Mo)"), ("small", "Précis (470 Mo)")];
    let speech = combo(
        "audio-x-generic-symbolic",
        "Reconnaissance vocale",
        "Modèle whisper, sur le processeur",
        &SPEECH.map(|(_, n)| n.to_string()),
        SPEECH.iter().position(|(c, _)| *c == cfg.speech_model).unwrap_or(0),
    );
    {
        let app = app.clone();
        speech.connect_selected_notify(move |r| {
            let id = SPEECH[r.selected() as usize].0.to_string();
            app.update(|c| c.ai.speech_model = id);
        });
    }
    g.add(&speech);
    body.append(&g);

    // ── Modèle de langage (rempli après détection du matériel) ──
    let model_group = group("Modèle de langage");
    model_group.set_description(Some(
        "Les commandes courantes fonctionnent sans modèle. Le modèle comprend les demandes libres et la recherche en langage naturel ; il est exécuté par Ollama, sur la carte graphique (Vulkan) ou le processeur.",
    ));
    body.append(&model_group);

    // ── Ressources ──
    let g = group("Ressources");
    let gpu = switch("video-display-symbolic", "Utiliser la carte graphique", "Vulkan : plus rapide ; désactivé, le processeur fait tout le travail", cfg.use_gpu);
    {
        let app = app.clone();
        gpu.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.ai.use_gpu = v);
        });
    }
    g.add(&gpu);
    let unload = spin("preferences-system-time-symbolic", "Libérer la mémoire après", "Minutes sans requête avant de décharger le modèle", (1.0, 60.0, 1.0), f64::from(cfg.unload_minutes));
    {
        let app = app.clone();
        unload.connect_value_notify(move |r| {
            let v = r.value() as u32;
            app.update(|c| c.ai.unload_minutes = v);
        });
    }
    g.add(&unload);
    let battery = switch("battery-symbolic", "Économie sur batterie", "Sur batterie, seulement les commandes rapides (aucun modèle chargé)", cfg.light_on_battery);
    {
        let app = app.clone();
        battery.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.ai.light_on_battery = v);
        });
    }
    g.add(&battery);
    let confirm = switch("dialog-warning-symbolic", "Confirmer les actions système", "Mise à jour, extinction, suppression : toujours demander avant d'agir", cfg.confirm_system);
    {
        let app = app.clone();
        confirm.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.ai.confirm_system = v);
        });
    }
    g.add(&confirm);
    body.append(&g);

    // ── Applications ──
    let apps_group = group("Applications");
    apps_group.set_description(Some("Ce que l'assistant peut faire dans chaque application. Coupez une application pour qu'il n'y touche pas."));
    body.append(&apps_group);

    // ── Confidentialité ──
    let g = group("Confidentialité");
    g.add(&card("security-high-symbolic", "Tout reste sur ce PC", "Voix, recherches et demandes sont traitées localement ; aucun compte, aucun envoi"));
    let history = switch("document-open-recent-symbolic", "Garder l'historique de l'assistant", "Désactivé : chaque conversation est oubliée à la fermeture du panneau", cfg.keep_history);
    {
        let app = app.clone();
        history.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| c.ai.keep_history = v);
        });
    }
    g.add(&history);
    body.append(&g);

    // Détection du matériel et des applications, en arrière-plan.
    let task = background(|| {
        let hw = if std::env::var_os("NOOS_AI_DEMO").is_some() { demo_hw() } else { ai::detect() };
        let apps = discover_apps();
        (hw, apps)
    });
    let app2 = app.clone();
    glib::spawn_future_local(async move {
        let (hw, apps) = task.await;
        hw_label.set_text(&hw.summary());
        fill_models(&app2, &model_group, &hw);
        fill_apps(&app2, &apps_group, apps);
    });
    p
}

fn demo_hw() -> Hardware {
    Hardware {
        ram_mb: 16000,
        ram_available_mb: 9000,
        cpu_threads: 12,
        gpus: vec![ai::Gpu { name: "AMD Radeon RX 7700 XT".into(), driver: "radv".into(), discrete: true, vram_mb: Some(12288) }],
        vulkan_tools: true,
        ollama: true,
        ollama_vulkan: true,
        whisper: true,
        on_battery: false,
    }
}

/// « Super+H » → « Super + H » (affichage).
fn key_label(k: &str) -> String {
    if k.is_empty() { "Aucune".into() } else { k.replace('+', " + ") }
}

/// Dialogue de capture : la prochaine combinaison appuyée devient la touche de dictée.
fn capture_key(app: &Rc<App>, button: &gtk::Button) {
    let d = adap::AlertDialog::new(Some("Touche de dictée"), Some("Appuyez sur la combinaison voulue (par exemple Super + H, ou une touche seule comme Ctrl droit). Échap pour annuler."));
    let shown = gtk::Label::new(Some("…"));
    shown.add_css_class("title-2");
    d.set_extra_child(Some(&shown));
    d.add_responses(&[("cancel", "Annuler"), ("none", "Aucune touche"), ("ok", "Enregistrer")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_response_enabled("ok", false);
    let chosen = Rc::new(std::cell::RefCell::new(String::new()));
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (shown, chosen, d2) = (shown.clone(), chosen.clone(), d.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            if key == gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            let combo = combo_string(key, mods);
            shown.set_text(&key_label(&combo));
            // Une touche de modification seule (Super, Ctrl…) n'est valable qu'à droite.
            let alone_modifier = matches!(key, gdk::Key::Super_L | gdk::Key::Shift_L | gdk::Key::Shift_R | gdk::Key::Alt_L | gdk::Key::Control_L);
            d2.set_response_enabled("ok", !alone_modifier);
            *chosen.borrow_mut() = combo;
            glib::Propagation::Stop
        });
    }
    d.add_controller(keys);
    let window = app.window.clone();
    let (app, button) = (app.clone(), button.clone());
    d.connect_response(None, move |_, r| {
        let key = match r {
            "ok" => chosen.borrow().clone(),
            "none" => String::new(),
            _ => return,
        };
        button.set_label(&key_label(&key));
        app.update(|c| c.ai.ptt_key = key);
    });
    d.present(Some(&window));
}

fn combo_string(key: gdk::Key, mods: gdk::ModifierType) -> String {
    let mut parts = vec![];
    for (m, name) in [(gdk::ModifierType::SUPER_MASK, "Super"), (gdk::ModifierType::CONTROL_MASK, "Ctrl"), (gdk::ModifierType::ALT_MASK, "Alt"), (gdk::ModifierType::SHIFT_MASK, "Maj")] {
        if mods.contains(m) {
            parts.push(name.to_string());
        }
    }
    let name = match key {
        gdk::Key::Control_R => "Ctrl_droit".to_string(),
        gdk::Key::Alt_R | gdk::Key::ISO_Level3_Shift => "AltGr".to_string(),
        gdk::Key::Super_R => "Super_droit".to_string(),
        gdk::Key::Super_L => return parts.join("+"),
        k => k.to_upper().name().map(|n| n.to_string()).unwrap_or_default(),
    };
    if !parts.contains(&name) {
        parts.push(name);
    }
    parts.join("+")
}

fn fill_models(app: &Rc<App>, g: &adap::PreferencesGroup, hw: &Hardware) {
    let rec = ai::recommended(hw);
    let engine = card(
        if hw.ollama { "emblem-ok-symbolic" } else { "dialog-information-symbolic" },
        "Moteur",
        &match (hw.ollama, hw.ollama_vulkan, hw.best_gpu()) {
            (true, true, Some(gpu)) => format!("Ollama avec Vulkan — {}", gpu.name),
            (true, _, None) => "Ollama sur le processeur (aucune carte graphique Vulkan)".to_string(),
            (true, false, Some(_)) => "Ollama sur le processeur (accélération Vulkan non installée)".to_string(),
            (false, ..) => "Ollama sera installé avec la prochaine mise à jour de NOOS".to_string(),
        },
    );
    g.add(&engine);
    let current = app.cfg().ai.tier;
    let selected = if current.is_empty() { rec.map(|t| t.id).unwrap_or("light").to_string() } else { current };
    let mut first: Option<gtk::CheckButton> = None;
    let mut slots = vec![];
    for t in ai::TIERS {
        let r = adap::ActionRow::new();
        r.set_title(t.label);
        let go = |mb: u64| format!("{:.1}", mb as f64 / 1000.0).replace('.', ",");
        r.set_subtitle(&format!("{} · {} · téléchargement {} Go, mémoire {} Go", t.description, t.model, go(t.download_mb), go(t.memory_mb)));
        let check = gtk::CheckButton::new();
        if let Some(f) = &first {
            check.set_group(Some(f));
        } else {
            first = Some(check.clone());
        }
        check.set_active(selected == t.id);
        r.add_prefix(&check);
        r.set_activatable_widget(Some(&check));
        if rec.is_some_and(|x| x.id == t.id) {
            let b = gtk::Label::new(Some("Conseillé pour ce PC"));
            b.add_css_class("ai-badge");
            b.set_valign(gtk::Align::Center);
            r.add_suffix(&b);
        }
        let too_big = t.memory_mb > hw.ram_mb.saturating_sub(1500) && hw.best_gpu().and_then(|g| g.vram_mb).unwrap_or(0) < t.memory_mb;
        if too_big {
            let w = gtk::Image::from_icon_name("dialog-warning-symbolic");
            w.set_tooltip_text(Some("Trop gros pour la mémoire de ce PC : risque de ralentir tout le système"));
            r.add_suffix(&w);
        }
        // Téléchargement / suppression (rempli par `refresh_models`).
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        slot.set_valign(gtk::Align::Center);
        r.add_suffix(&slot);
        slots.push((t.id, slot));
        let (app, id) = (app.clone(), t.id);
        check.connect_toggled(move |c| {
            if c.is_active() {
                app.update(|cfg| cfg.ai.tier = id.to_string());
            }
        });
        g.add(&r);
    }
    if rec.is_none() {
        g.add(&card("dialog-information-symbolic", "Commandes rapides seulement", "Ce PC a peu de mémoire : l'assistant comprend les commandes courantes, sans modèle de langage"));
    }
    if hw.ollama {
        refresh_models(app, Rc::new(slots));
    }
}

const BUS: &str = "org.noos.Assistant1";

/// Appel asynchrone du démon `noos-ai` (activé à la demande par D-Bus).
fn daemon(method: &str, args: glib::Variant, done: impl FnOnce(serde_json::Value) + 'static) {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return };
    conn.call(
        Some(BUS),
        "/org/noos/Assistant",
        BUS,
        method,
        Some(&args),
        Some(glib::VariantTy::new("(s)").unwrap()),
        gio::DBusCallFlags::NONE,
        60_000,
        gio::Cancellable::NONE,
        move |r| {
            let v = r.ok().and_then(|v| v.get::<(String,)>()).and_then(|(s,)| serde_json::from_str(&s).ok()).unwrap_or(serde_json::Value::Null);
            done(v);
        },
    );
}

type Slots = Rc<Vec<(&'static str, gtk::Box)>>;

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1} Go", bytes as f64 / 1e9).replace('.', ",")
}

/// État des modèles : « Télécharger » ou « Installé · Supprimer » ; suit un téléchargement en cours.
fn refresh_models(app: &Rc<App>, slots: Slots) {
    let app = app.clone();
    daemon("Models", ().to_variant(), move |v| {
        for (id, slot) in slots.iter() {
            clear(slot);
            let Some(t) = v["tiers"].as_array().and_then(|a| a.iter().find(|t| t["id"] == *id)) else { continue };
            if t["installed"].as_bool().unwrap_or(false) {
                let l = gtk::Label::new(Some(&format!("Installé · {}", gb(t["size"].as_u64().unwrap_or(0)))));
                l.add_css_class("dim-label");
                slot.append(&l);
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_tooltip_text(Some("Supprimer ce modèle"));
                let (app2, slots2, id) = (app.clone(), slots.clone(), *id);
                del.connect_clicked(move |_| {
                    let (app3, slots3) = (app2.clone(), slots2.clone());
                    app2.confirm("Supprimer ce modèle ?", "Il faudra le télécharger de nouveau pour l'utiliser.", "Supprimer", true, move || {
                        let (app4, slots4) = (app3.clone(), slots3.clone());
                        daemon("DeleteModel", (id,).to_variant(), move |r| {
                            app4.toast(r["message"].as_str().unwrap_or("Modèle supprimé"));
                            refresh_models(&app4, slots4);
                        });
                    });
                });
                slot.append(&del);
            } else {
                let dl = gtk::Button::with_label("Télécharger");
                dl.add_css_class("pill");
                let (app2, slots2, id) = (app.clone(), slots.clone(), *id);
                dl.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    let (app3, slots3) = (app2.clone(), slots2.clone());
                    daemon("PullModel", (id,).to_variant(), move |r| {
                        if r["ok"].as_bool() != Some(true) {
                            app3.toast(r["message"].as_str().unwrap_or("Téléchargement impossible"));
                        }
                        watch_pull(&app3, slots3);
                    });
                });
                slot.append(&dl);
            }
        }
        watch_pull(&app, slots.clone());
    });
}

/// Barre de progression sur la ligne du modèle en cours de téléchargement.
fn watch_pull(app: &Rc<App>, slots: Slots) {
    let app = app.clone();
    daemon("PullStatus", ().to_variant(), move |v| {
        let model = v["model"].as_str().unwrap_or("");
        let Some((_, slot)) = slots.iter().find(|(id, _)| ai::tier(id).is_some_and(|t| t.model == model)) else { return };
        if v["active"].as_bool() == Some(true) {
            clear(slot);
            let (c, t) = (v["completed"].as_u64().unwrap_or(0), v["total"].as_u64().unwrap_or(0));
            let bar = gtk::ProgressBar::new();
            bar.set_valign(gtk::Align::Center);
            bar.set_size_request(120, -1);
            if t > 0 {
                bar.set_fraction(c as f64 / t as f64);
            } else {
                bar.pulse();
            }
            let l = gtk::Label::new(Some(&if t > 0 { format!("{} / {}", gb(c), gb(t)) } else { "Préparation…".into() }));
            l.add_css_class("dim-label");
            l.add_css_class("caption");
            slot.append(&l);
            slot.append(&bar);
            glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || watch_pull(&app, slots));
        } else if v["done"].as_bool() == Some(true) && slot.first_child().is_some_and(|c| c.is::<gtk::Label>() && slot.last_child().is_some_and(|l| l.is::<gtk::ProgressBar>())) {
            // Fin d'un téléchargement suivi ici : résultat puis état à jour.
            match v["error"].as_str() {
                Some(e) => app.toast(&format!("Échec du téléchargement : {e}")),
                None => app.toast("Modèle téléchargé : l'assistant est prêt."),
            }
            refresh_models(&app, slots);
        }
    });
}

/// Applications publiant des actions (bus de session).
fn discover_apps() -> Vec<noos_actions::AppActions> {
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else { return vec![] };
    noos_actions::discover(&conn).iter().filter_map(|b| noos_actions::list(&conn, b).ok()).collect()
}

fn fill_apps(app: &Rc<App>, g: &adap::PreferencesGroup, apps: Vec<noos_actions::AppActions>) {
    if apps.is_empty() {
        g.add(&card("view-app-grid-symbolic", "Aucune application ouverte n'expose d'actions", "Les applications NOOS les publient quand elles sont lancées"));
        return;
    }
    let disabled = app.cfg().ai.disabled_apps;
    for a in apps {
        let e = adap::ExpanderRow::new();
        e.set_title(&a.name);
        e.set_subtitle(&format!("{} action{}", a.actions.len(), if a.actions.len() > 1 { "s" } else { "" }));
        e.set_show_enable_switch(true);
        e.set_enable_expansion(!disabled.contains(&a.app));
        for act in &a.actions {
            let r = adap::ActionRow::new();
            r.set_title(&act.title);
            r.set_subtitle(&act.description);
            if act.risk == noos_actions::Risk::System {
                let b = gtk::Label::new(Some("Confirmation"));
                b.add_css_class("ai-badge");
                b.set_valign(gtk::Align::Center);
                r.add_suffix(&b);
            }
            e.add_row(&r);
        }
        let (app, id) = (app.clone(), a.app.clone());
        e.connect_enable_expansion_notify(move |e| {
            let on = e.enables_expansion();
            let id = id.clone();
            app.update(move |c| {
                c.ai.disabled_apps.retain(|x| *x != id);
                if !on {
                    c.ai.disabled_apps.push(id);
                }
            });
        });
        g.add(&e);
    }
}

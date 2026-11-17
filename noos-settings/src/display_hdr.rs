//! Paramètres › Système › Écran : HDR10 par écran (interrupteur, luminosité du contenu SDR).
//!
//! L'interrupteur applique le HDR tout de suite (requête IPC `set-hdr`), puis demande de
//! conserver le réglage (retour automatique au bout de 15 s, comme pour la résolution) : un
//! écran mal géré peut rester noir. Le réglage conservé est écrit dans `[[output]] hdr`.

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_core::{
    NoosConfig,
    config::SDR_BRIGHTNESS_RANGE,
    ipc::{Event, OutputInfo, Request},
};

use crate::ui::{App, card, ipc, switch};

/// Texte sous l'interrupteur : état réel, ou pourquoi le HDR est impossible.
pub fn hdr_subtitle(o: &OutputInfo) -> String {
    if !o.hdr_capable {
        return if o.hdr_unavailable.is_empty() {
            "Non pris en charge par cet écran ou ce GPU".into()
        } else {
            o.hdr_unavailable.clone()
        };
    }
    let peak = if o.peak_brightness > 0 { format!(" · crête {} cd/m²", o.peak_brightness) } else { String::new() };
    if o.hdr_active {
        format!("HDR10 actif (PQ, BT.2020, 10 bits){peak}")
    } else {
        format!("Couleurs et contrastes étendus pour les jeux et les vidéos HDR{peak}")
    }
}

/// Ajoute les lignes HDR de l'écran `o` au groupe `adv`. `reload` redessine la page.
pub fn add_rows(app: &Rc<App>, adv: &adap::PreferencesGroup, o: &OutputInfo, cfg: &NoosConfig, reload: Rc<dyn Fn(&Rc<App>)>) {
    let hdr = switch("weather-clear-symbolic", "Utiliser le HDR", &hdr_subtitle(o), o.hdr_active);
    hdr.set_subtitle_lines(3);
    hdr.set_sensitive(o.hdr_capable);
    {
        let (app, name, reload) = (app.clone(), o.name.clone(), reload.clone());
        hdr.connect_active_notify(move |r| {
            let v = r.is_active();
            let task = ipc(Request::SetHdr { output: name.clone(), enabled: v, sdr_brightness: None });
            let (app, name, reload) = (app.clone(), name.clone(), reload.clone());
            glib::spawn_future_local(async move {
                match task.await {
                    Ok(Event::Ok) => {
                        let keep = {
                            let (app, name) = (app.clone(), name.clone());
                            move || app.update(|c| c.output_mut(&name).hdr = Some(v))
                        };
                        let revert = {
                            let name = name.clone();
                            move || {
                                let _ = noos_core::ipc::IpcClient::fire(&Request::SetHdr {
                                    output: name.clone(),
                                    enabled: !v,
                                    sdr_brightness: None,
                                });
                            }
                        };
                        let reload = reload.clone();
                        crate::system::keep_or_revert(&app, keep, revert, move |app| reload(app));
                    }
                    Ok(Event::Error { message }) | Err(message) => {
                        app.error("HDR", &message);
                        reload(&app);
                    }
                    _ => {}
                }
            });
        });
    }
    adv.add(&hdr);

    if !o.hdr_capable {
        return;
    }
    // Luminosité du blanc SDR (bureau, applications) une fois le HDR actif.
    let row = card(
        "display-brightness-symbolic",
        "Luminosité du contenu SDR",
        "Bureau et applications non HDR : à régler pour qu'ils paraissent comme en SDR",
    );
    row.set_subtitle_lines(2);
    let (min, max) = SDR_BRIGHTNESS_RANGE;
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min as f64, max as f64, 10.0);
    scale.set_value(cfg.sdr_brightness_for(&o.name) as f64);
    scale.set_size_request(220, -1);
    scale.set_valign(gtk::Align::Center);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Left);
    scale.set_format_value_func(|_, v| format!("{v:.0} cd/m²"));
    scale.add_mark(noos_core::config::DEFAULT_SDR_BRIGHTNESS as f64, gtk::PositionType::Bottom, None);
    row.set_sensitive(o.hdr_active);
    {
        let (app, name) = (app.clone(), o.name.clone());
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        scale.connect_value_changed(move |s| {
            let nits = s.value().round() as u32;
            // Effet immédiat (simple paramètre du shader), enregistrement un peu plus tard.
            let _task = ipc(Request::SetHdr { output: name.clone(), enabled: true, sdr_brightness: Some(nits) });
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (app, name, pending2) = (app.clone(), name.clone(), pending.clone());
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                pending2.borrow_mut().take();
                app.update(|c| c.output_mut(&name).sdr_brightness = Some(nits));
            });
            *pending.borrow_mut() = Some(id);
        });
    }
    row.add_suffix(&scale);
    adv.add(&row);
}

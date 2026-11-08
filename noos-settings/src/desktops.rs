//! Bureaux virtuels : liste (renommer, réordonner, supprimer, ajouter), comportement (barre
//! des tâches, navigation circulaire, animation, indicateur) et raccourcis clavier. La liste
//! est gérée par le compositeur (requêtes IPC), les options par `[desktops]`.

use std::rc::Rc;

use adap::prelude::*;
use gtk::glib;
use noos_core::ipc::{Event, Request};

use crate::ui::{App, card, group, hero, ipc, page, switch};

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Bureaux virtuels");
    let img = gtk::Image::from_icon_name("window-restore-symbolic");
    img.set_pixel_size(72);
    let (h, actions) = hero(&img, "Bureaux virtuels", "Organisez vos fenêtres sur plusieurs bureaux : travail, jeux, loisirs… et passez de l'un à l'autre d'un raccourci.");
    let tv = gtk::Button::with_label("Vue des tâches");
    tv.add_css_class("pill");
    tv.add_css_class("suggested-action");
    tv.connect_clicked(|_| {
        let _ = ipc(Request::ToggleTaskView);
    });
    actions.append(&tv);
    content.append(&h);

    let list = group("Vos bureaux");
    list.set_description(Some("Le bureau actif est marqué d'une coche. Les fenêtres d'un bureau supprimé rejoignent le bureau voisin."));
    content.append(&list);
    fill_list(app, &list);

    let cfg = app.cfg();
    let g = group("Comportement");
    let opts: [(&str, &str, &str, bool, fn(&mut noos_core::NoosConfig, bool)); 5] = [
        ("view-list-bullet-symbolic", "Barre des tâches : fenêtres de tous les bureaux", "Sinon, seulement celles du bureau actif (comme Windows)", cfg.desktops.taskbar_all, |c, v| c.desktops.taskbar_all = v),
        ("media-playlist-repeat-symbolic", "Navigation circulaire", "Après le dernier bureau, revenir au premier", cfg.desktops.wrap, |c, v| c.desktops.wrap = v),
        ("view-paged-symbolic", "Animation de glissement", "Le nouveau bureau entre par le côté", cfg.desktops.animation, |c, v| c.desktops.animation = v),
        ("dialog-information-symbolic", "Afficher le nom du bureau", "Brièvement, en haut de l'écran, à chaque changement", cfg.desktops.osd, |c, v| c.desktops.osd = v),
        ("window-restore-symbolic", "Bouton « Vue des tâches » dans la barre", "La molette sur ce bouton change de bureau", cfg.desktops.taskbar_button, |c, v| c.desktops.taskbar_button = v),
    ];
    for (icon, title, sub, val, set) in opts {
        let row = switch(icon, title, sub, val);
        let app = app.clone();
        row.connect_active_notify(move |r| {
            let v = r.is_active();
            app.update(|c| set(c, v));
        });
        g.add(&row);
    }
    content.append(&g);

    let keys = group("Raccourcis clavier");
    for (k, what) in [
        ("Super + Tab", "Vue des tâches"),
        ("Super + Ctrl + ← / →", "Bureau précédent / suivant"),
        ("Super + Ctrl + D", "Nouveau bureau"),
        ("Super + Ctrl + F4", "Fermer le bureau actif"),
        ("Super + Ctrl + Maj + ← / →", "Emporter la fenêtre active sur le bureau voisin"),
        ("Super + Maj + ← / →", "Envoyer la fenêtre active sur l'écran voisin"),
    ] {
        let r = card("input-keyboard-symbolic", what, "");
        let l = gtk::Label::new(Some(k));
        l.add_css_class("dim-label");
        l.add_css_class("monospace");
        r.add_suffix(&l);
        keys.add(&r);
    }
    content.append(&keys);
    p
}

/// Liste des bureaux (relue au compositeur après chaque action).
fn fill_list(app: &Rc<App>, list: &adap::PreferencesGroup) {
    let task = ipc(Request::GetDesktops);
    let (app, list) = (app.clone(), list.clone());
    glib::spawn_future_local(async move {
        // Vider le groupe (les lignes sont des enfants de sa liste interne).
        while let Some(r) = list.first_child().and_then(|_| find_rows(&list).into_iter().next()) {
            list.remove(&r);
        }
        let (names, current) = match task.await {
            Ok(Event::Desktops { names, current }) => (names, current),
            _ => {
                list.add(&card("dialog-warning-symbolic", "Compositeur NOOS injoignable", "Ouvrez les Paramètres depuis une session NOOS"));
                return;
            }
        };
        let n = names.len();
        for (i, name) in names.iter().enumerate() {
            let row = adap::EntryRow::new();
            row.set_title(&format!("Bureau {}", i + 1));
            row.set_text(name);
            row.set_show_apply_button(true);
            {
                let (app2, list2) = (app.clone(), list.clone());
                row.connect_apply(move |r| {
                    let _ = ipc(Request::RenameDesktop { index: i, name: r.text().to_string() });
                    refresh(&app2, &list2);
                });
            }
            if i == current {
                let ok = gtk::Image::from_icon_name("object-select-symbolic");
                ok.set_tooltip_text(Some("Bureau actif"));
                row.add_suffix(&ok);
            } else {
                let go = gtk::Button::with_label("Aller");
                go.add_css_class("flat");
                go.set_valign(gtk::Align::Center);
                let (app2, list2) = (app.clone(), list.clone());
                go.connect_clicked(move |_| {
                    let _ = ipc(Request::SwitchDesktop { index: i });
                    refresh(&app2, &list2);
                });
                row.add_suffix(&go);
            }
            let btn = |icon: &str, tip: &str, sensitive: bool, req: Request| {
                let b = gtk::Button::from_icon_name(icon);
                b.add_css_class("flat");
                b.set_valign(gtk::Align::Center);
                b.set_tooltip_text(Some(tip));
                b.set_sensitive(sensitive);
                let (app2, list2) = (app.clone(), list.clone());
                b.connect_clicked(move |_| {
                    let _ = ipc(req.clone());
                    refresh(&app2, &list2);
                });
                b
            };
            row.add_suffix(&btn("go-up-symbolic", "Monter", i > 0, Request::MoveDesktop { from: i, to: i.saturating_sub(1) }));
            row.add_suffix(&btn("go-down-symbolic", "Descendre", i + 1 < n, Request::MoveDesktop { from: i, to: i + 1 }));
            row.add_suffix(&btn("user-trash-symbolic", "Supprimer ce bureau", n > 1, Request::RemoveDesktop { index: i }));
            list.add(&row);
        }
        let add = card("list-add-symbolic", "Ajouter un bureau", "Raccourci : Super + Ctrl + D");
        add.set_activatable(true);
        {
            let (app2, list2) = (app.clone(), list.clone());
            add.connect_activated(move |_| {
                let _ = ipc(Request::AddDesktop { name: None, switch: false });
                refresh(&app2, &list2);
            });
        }
        list.add(&add);
    });
}

/// Lignes ajoutées au groupe (pour le vider).
fn find_rows(list: &adap::PreferencesGroup) -> Vec<gtk::Widget> {
    let mut out = vec![];
    fn walk(w: &gtk::Widget, out: &mut Vec<gtk::Widget>) {
        let mut c = w.first_child();
        while let Some(x) = c {
            if x.is::<adap::PreferencesRow>() {
                out.push(x.clone());
            } else {
                walk(&x, out);
            }
            c = x.next_sibling();
        }
    }
    walk(list.upcast_ref(), &mut out);
    out
}

fn refresh(app: &Rc<App>, list: &adap::PreferencesGroup) {
    let (app, list) = (app.clone(), list.clone());
    glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || fill_list(&app, &list));
}

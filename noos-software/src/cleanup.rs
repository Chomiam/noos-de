//! « Faire de la place » (page Installées) : dépendances inutiles, cache apt, anciens noyaux,
//! runtimes Flatpak inutilisés, données d'applis supprimées. Analyse sans mot de passe
//! (`noos_pkg::cleanup::scan`), nettoyage en une seule opération (une seule élévation).

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_pkg::{
    Plan,
    cleanup::{self, Item, Kind},
};

use crate::{App, ops::background, row};

fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::AptOrphans => "edit-clear-all-symbolic",
        Kind::AptCache => "folder-download-symbolic",
        Kind::OldKernels => "system-run-symbolic",
        Kind::FlatpakUnused => "application-x-addon-symbolic",
        Kind::FlatpakData => "user-trash-symbolic",
    }
}

/// Éléments fictifs (`NOOS_SOFTWARE_CLEANUP=demo`).
fn demo_items() -> Vec<Item> {
    let item = |kind, title: &str, description: &str, size: u64, entries: &[&str], reversible| Item {
        kind,
        title: title.into(),
        description: description.into(),
        size,
        entries: entries.iter().map(|s| s.to_string()).collect(),
        plan: Plan::new().user(&["true"]),
        reversible,
    };
    vec![
        item(Kind::AptOrphans, "Dépendances devenues inutiles", "Paquets installés pour un logiciel supprimé depuis", 184_000_000, &["libgegl-0.4-0t64", "libbabl-0.1-0", "gimp-data", "libmypaint-1.5-1"], false),
        item(Kind::AptCache, "Paquets déjà installés, gardés en cache", "516 fichiers .deb téléchargés (retéléchargés si besoin)", 696_237_032, &[], false),
        item(Kind::OldKernels, "Anciens noyaux Linux", "Le noyau en cours (6.17.9+deb14-amd64) et le plus récent sont gardés", 412_000_000, &["linux-image-6.16.3+deb14-amd64", "linux-headers-6.16.3+deb14-amd64"], false),
        item(Kind::FlatpakUnused, "Runtimes Flatpak inutilisés (système)", "Bibliothèques partagées dont plus aucune application n'a besoin", 1_250_000_000, &["org.gnome.Platform//47", "org.freedesktop.Platform//24.08"], false),
        item(Kind::FlatpakData, "Données d'applications Flatpak supprimées", "Réglages et fichiers internes restés dans ~/.var/app (mis à la corbeille)", 95_000_000, &["com.spotify.Client", "org.telegram.desktop"], true),
    ]
}

impl App {
    /// Ligne « Faire de la place » en tête de la page Installées.
    pub(crate) fn cleanup_group(self: &Rc<Self>) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        let r = row("Faire de la place", "Dépendances inutiles, paquets en cache, anciens noyaux, runtimes Flatpak inutilisés");
        r.add_prefix(&gtk::Image::from_icon_name("drive-harddisk-symbolic"));
        let b = gtk::Button::with_label("Analyser");
        b.set_valign(gtk::Align::Center);
        let app = self.clone();
        b.connect_clicked(move |_| app.show_cleanup(false));
        r.add_suffix(&b);
        r.set_activatable_widget(Some(&b));
        g.add(&r);
        g
    }

    pub(crate) fn show_cleanup(self: &Rc<Self>, demo: bool) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&crate::loading("Mesure de l'espace récupérable…"));
        let scroll = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&scroll));
        tv.add_css_class("noos-sheet");
        let action = gtk::Button::with_label("Nettoyer");
        action.add_css_class("suggested-action");
        action.add_css_class("pill");
        action.set_halign(gtk::Align::Center);
        action.set_margin_top(8);
        action.set_margin_bottom(14);
        action.set_sensitive(false);
        tv.add_bottom_bar(&action);
        let dialog = adap::Dialog::builder().title("Faire de la place").content_width(700).content_height(640).child(&tv).build();
        dialog.present(Some(&self.window));

        let task = background(move || if demo { demo_items() } else { cleanup::scan() });
        let app = self.clone();
        glib::spawn_future_local(async move {
            let items = task.await;
            crate::clear(&content);
            if items.is_empty() {
                content.append(&crate::status_page("emblem-ok-symbolic", "Rien à nettoyer", "Aucun paquet inutile, aucun cache ni runtime à retirer."));
                action.set_visible(false);
                return;
            }
            let total: u64 = items.iter().map(|i| i.size).sum();
            let head = gtk::Label::new(Some(&format!("Jusqu'à {} récupérables", glib::format_size(total))));
            head.add_css_class("title-2");
            head.set_xalign(0.0);
            content.append(&head);
            let intro = gtk::Label::new(Some("Cochez ce qu'il faut retirer. Les logiciels que vous utilisez ne sont pas touchés ; une seule demande de mot de passe pour le tout."));
            intro.set_wrap(true);
            intro.set_xalign(0.0);
            intro.add_css_class("dim-label");
            content.append(&intro);

            let g = adap::PreferencesGroup::new();
            let checks: Rc<RefCell<Vec<(gtk::CheckButton, Item)>>> = Rc::new(RefCell::new(Vec::new()));
            let update = {
                let (checks, action) = (checks.clone(), action.clone());
                move || {
                    let sel: u64 = checks.borrow().iter().filter(|(c, _)| c.is_active()).map(|(_, i)| i.size).sum();
                    let n = checks.borrow().iter().filter(|(c, _)| c.is_active()).count();
                    action.set_sensitive(n > 0);
                    action.set_label(&if n > 0 { format!("Nettoyer ({})", glib::format_size(sel)) } else { "Nettoyer".to_string() });
                }
            };
            let update = Rc::new(update);
            for item in items {
                let check = gtk::CheckButton::new();
                check.set_active(true);
                check.set_valign(gtk::Align::Center);
                {
                    let u = update.clone();
                    check.connect_toggled(move |_| u());
                }
                let subtitle = format!("{} · {}", glib::format_size(item.size), item.description);
                if item.entries.is_empty() {
                    let r = row(&item.title, &subtitle);
                    r.add_prefix(&check);
                    r.add_prefix(&gtk::Image::from_icon_name(icon(item.kind)));
                    r.set_activatable_widget(Some(&check));
                    g.add(&r);
                } else {
                    let x = adap::ExpanderRow::new();
                    x.set_title(&item.title);
                    x.set_subtitle(&subtitle);
                    x.add_prefix(&gtk::Image::from_icon_name(icon(item.kind)));
                    x.add_prefix(&check);
                    for e in item.entries.iter().take(200) {
                        let r = adap::ActionRow::new();
                        r.set_use_markup(false);
                        r.set_title(e);
                        x.add_row(&r);
                    }
                    g.add(&x);
                }
                checks.borrow_mut().push((check, item));
            }
            content.append(&g);
            update();

            let (app2, dlg) = (app.clone(), dialog.clone());
            action.connect_clicked(move |_| {
                let chosen: Vec<Item> = checks.borrow().iter().filter(|(c, _)| c.is_active()).map(|(_, i)| i.clone()).collect();
                let mut plan = Plan::new();
                // Commandes root d'abord (regroupées en un seul pkexec), puis Flatpak et corbeille.
                for i in chosen.iter().filter(|i| matches!(i.kind, Kind::AptOrphans | Kind::OldKernels | Kind::AptCache)) {
                    plan = plan.extend(i.plan.clone());
                }
                for i in chosen.iter().filter(|i| matches!(i.kind, Kind::FlatpakUnused | Kind::FlatpakData)) {
                    plan = plan.extend(i.plan.clone());
                }
                let size: u64 = chosen.iter().map(|i| i.size).sum();
                dlg.close();
                let app3 = app2.clone();
                app2.ops.enqueue(format!("Nettoyage ({})", glib::format_size(size)), 0, move |_| Ok(plan), move |ok| {
                    if ok {
                        app3.toasts.add_toast(adap::Toast::new(&format!("{} libérés", glib::format_size(size))));
                    }
                    app3.load_installed();
                });
            });
        });
    }
}

//! Historique des opérations (bouton de la barre d'en-tête) : journal d'apt, de Flatpak et de
//! Logiciels. « Annuler » retire ce qu'une installation a ajouté ; « Revenir à l'état
//! d'avant » ouvre noos-save sur l'instantané pris juste avant l'opération.

use std::{collections::HashSet, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_pkg::{
    BackendKind,
    history::{self, Operation},
};

use crate::{App, ops::background};

/// Paquets apt et applis Flatpak installés (pour proposer « Annuler »).
fn installed_names() -> HashSet<String> {
    let mut set = HashSet::new();
    let run = |argv: &[&str]| std::process::Command::new(argv[0]).args(&argv[1..]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    for l in run(&["dpkg-query", "-W", "-f=${binary:Package}\t${db:Status-Abbrev}\n"]).lines() {
        if let Some((n, st)) = l.split_once('\t') {
            if st.starts_with("ii") {
                set.insert(n.split(':').next().unwrap_or(n).to_string());
            }
        }
    }
    for l in run(&["flatpak", "list", "--app", "--columns=application"]).lines() {
        set.insert(l.trim().to_string());
    }
    set
}

fn change_line(c: &history::Change, upgrade: bool) -> String {
    match (&c.old, upgrade) {
        (Some(o), true) => format!("{} : {o} → {}", c.name, c.version),
        _ if c.automatic => format!("{} {} (dépendance)", c.name, c.version),
        _ => format!("{} {}", c.name, c.version).trim().to_string(),
    }
}

impl App {
    pub(crate) fn show_history(self: &Rc<Self>) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&crate::loading("Lecture des journaux…"));
        let scroll = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&scroll));
        tv.add_css_class("noos-sheet");
        let dialog = adap::Dialog::builder().title("Historique des opérations").content_width(780).content_height(700).child(&tv).build();
        dialog.present(Some(&self.window));

        let task = background(|| (history::load(300), installed_names()));
        let app = self.clone();
        glib::spawn_future_local(async move {
            let (ops, installed) = task.await;
            crate::clear(&content);
            if ops.is_empty() {
                content.append(&crate::status_page("document-open-recent-symbolic", "Aucune opération", "Les installations, mises à jour et suppressions apparaîtront ici."));
                return;
            }
            let intro = gtk::Label::new(Some(
                "Toutes les installations, mises à jour et suppressions, y compris celles faites dans un terminal. \
                 Un instantané du système est pris avant chaque opération apt : vous pouvez y revenir avec Sauvegarder et Restaurer.",
            ));
            intro.set_wrap(true);
            intro.set_xalign(0.0);
            intro.add_css_class("dim-label");
            content.append(&intro);
            let mut day = String::new();
            let mut group: Option<adap::PreferencesGroup> = None;
            for op in ops {
                let d = op.date.split(' ').next().unwrap_or("").to_string();
                if group.is_none() || d != day {
                    if let Some(g) = group.take() {
                        content.append(&g);
                    }
                    let g = adap::PreferencesGroup::new();
                    g.set_title(&day_title(&d));
                    group = Some(g);
                    day = d;
                }
                group.as_ref().unwrap().add(&app.history_row(&op, &installed, &dialog));
            }
            if let Some(g) = group {
                content.append(&g);
            }
        });
    }

    fn history_row(self: &Rc<Self>, op: &Operation, installed: &HashSet<String>, dialog: &adap::Dialog) -> adap::ExpanderRow {
        let x = adap::ExpanderRow::new();
        x.set_title(&glib::markup_escape_text(&op.summary()));
        let time = op.date.split(' ').nth(1).unwrap_or("");
        let source = match op.backend {
            Some(BackendKind::Flatpak) => "Flatpak",
            _ if op.command.contains("APT::Status-Fd") => "installation de NOOS",
            _ if op.command.contains("--force-confold") => "Logiciels",
            _ if !op.command.is_empty() => "terminal",
            _ => "Logiciels",
        };
        let mut sub = format!("{time} · {source}");
        if !op.requested_by.is_empty() {
            sub.push_str(&format!(" · {}", op.requested_by));
        }
        if let Some(n) = op.snapshot {
            sub.push_str(&format!(" · instantané n° {n}"));
        }
        if !op.success {
            sub.push_str(" · ✗ échec");
        }
        x.set_subtitle(&glib::markup_escape_text(&sub));
        x.add_prefix(&gtk::Image::from_icon_name(if !op.removed.is_empty() && op.installed.is_empty() && op.upgraded.is_empty() {
            "user-trash-symbolic"
        } else if !op.upgraded.is_empty() {
            "software-update-available-symbolic"
        } else {
            "list-add-symbolic"
        }));

        // Actions.
        if let Some(plan) = op.undo_plan(&|n| installed.contains(n)) {
            let b = gtk::Button::with_label("Annuler");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some("Désinstaller ce que cette opération a ajouté"));
            let (app, title, dlg, kind) = (self.clone(), op.summary(), dialog.clone(), op.backend.unwrap_or(BackendKind::Apt));
            b.connect_clicked(move |_| {
                let plan = plan.clone();
                let (app2, dlg2, t2) = (app.clone(), dlg.clone(), title.clone());
                app.confirm(
                    "Annuler cette installation ?",
                    &format!("{title}\n\nCe qui a été installé (et l'est encore) sera désinstallé."),
                    "Désinstaller",
                    true,
                    move || {
                        dlg2.close();
                        let app3 = app2.clone();
                        let idx = app2.backend_index(kind).unwrap_or(0);
                        let plan = plan.clone();
                        app2.ops.enqueue(format!("Annulation — {t2}"), idx, move |_| Ok(plan), move |_| app3.after_change());
                    },
                );
            });
            x.add_suffix(&b);
        }
        if let Some(n) = op.snapshot {
            let b = gtk::Button::from_icon_name("edit-undo-symbolic");
            b.add_css_class("flat");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(&format!("Revenir à l'état d'avant (instantané n° {n}, Sauvegarder et Restaurer)")));
            let toasts = self.toasts.clone();
            b.connect_clicked(move |_| {
                if let Err(e) = std::process::Command::new("noos-save").env("NOOS_SAVE_EXPAND", n.to_string()).spawn() {
                    toasts.add_toast(adap::Toast::new(&format!("Sauvegarder et Restaurer est introuvable : {e}")));
                }
            });
            x.add_suffix(&b);
        }

        for (list, upgrade, label) in [(&op.installed, false, "Installé"), (&op.upgraded, true, "Mis à jour"), (&op.removed, false, "Retiré")] {
            for c in list.iter().take(150) {
                let r = adap::ActionRow::new();
                r.set_use_markup(false);
                r.set_title(&change_line(c, upgrade));
                r.set_subtitle(label);
                x.add_row(&r);
            }
            if list.len() > 150 {
                let r = adap::ActionRow::new();
                r.set_title(&format!("… et {} autres", list.len() - 150));
                x.add_row(&r);
            }
        }
        if !op.command.is_empty() {
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title("Commande");
            r.set_subtitle(&op.command);
            r.set_subtitle_lines(3);
            r.set_subtitle_selectable(true);
            x.add_row(&r);
        }
        x
    }
}

/// « 2026-11-18 » → « Aujourd'hui », « Hier » ou « lundi 16 novembre 2026 ».
fn day_title(d: &str) -> String {
    let p: Vec<i32> = d.split('-').filter_map(|x| x.parse().ok()).collect();
    let Some(dt) = (p.len() == 3).then(|| glib::DateTime::from_local(p[0], p[1], p[2], 12, 0, 0.0).ok()).flatten() else { return d.to_string() };
    let now = glib::DateTime::now_local().ok();
    let ymd = |t: &glib::DateTime| (t.year(), t.day_of_year());
    if let Some(n) = now.as_ref() {
        if ymd(n) == ymd(&dt) {
            return "Aujourd'hui".into();
        }
        if n.add_days(-1).ok().is_some_and(|y| ymd(&y) == ymd(&dt)) {
            return "Hier".into();
        }
    }
    dt.format("%A %e %B %Y").map(|s| s.replace("  ", " ")).unwrap_or_else(|_| d.into()).to_string()
}

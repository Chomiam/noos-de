//! Miroir Debian (onglet Sources) : miroir actuel, tableau des miroirs officiels (drapeau, pays,
//! nom, hébergeur, débit mesuré) avec « Tester » et « Utiliser ». Logique : `noos_pkg::mirrors`.

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::glib;
use noos_pkg::{
    BackendKind,
    mirrors::{self, Mirror, Speed},
};

use crate::{App, ops::background, row};

fn speed_text(s: &Speed) -> String {
    format!("{}/s · {} ms", glib::format_size(s.bytes_per_s as u64), s.latency_ms)
}

impl App {
    /// Groupe « Miroir Debian » de l'onglet Sources.
    pub(crate) fn mirror_group(self: &Rc<Self>) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        g.set_title("Miroir Debian");
        g.set_description(Some("Serveur d'où sont téléchargés les paquets Debian. Un miroir proche et rapide accélère installations et mises à jour."));
        let current = mirrors::current().unwrap_or_else(|| "—".into());
        let host = current.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(&current).to_string();
        let label = if current == mirrors::CDN || host == "deb.debian.org" { format!("{host} — réseau mondial (CDN), choisi automatiquement") } else { host };
        let r = row("Miroir actuel", &label);
        r.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
        let b = gtk::Button::with_label("Choisir le plus rapide…");
        b.set_valign(gtk::Align::Center);
        let app = self.clone();
        b.connect_clicked(move |_| app.show_mirrors());
        r.add_suffix(&b);
        g.add(&r);
        g
    }

    pub(crate) fn show_mirrors(self: &Rc<Self>) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.append(&crate::loading("Liste des miroirs officiels…"));
        let scroll = gtk::ScrolledWindow::builder().child(&content).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let header = adap::HeaderBar::new();
        let test_all = gtk::Button::with_label("Tester les premiers");
        test_all.set_tooltip_text(Some("Mesure le débit du réseau mondial et des miroirs de votre pays, l'un après l'autre"));
        test_all.add_css_class("suggested-action");
        test_all.set_sensitive(false);
        header.pack_start(&test_all);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&scroll));
        tv.add_css_class("noos-sheet");
        let dialog = adap::Dialog::builder().title("Miroirs Debian").content_width(900).content_height(680).child(&tv).build();
        dialog.present(Some(&self.window));

        let task = background(mirrors::load);
        let (app, dlg) = (self.clone(), dialog.clone());
        glib::spawn_future_local(async move {
            let list = task.await;
            crate::clear(&content);
            let intro = gtk::Label::new(Some(&format!(
                "{} miroirs officiels. « Tester » télécharge 1 Mio de l'index des paquets (sans mot de passe) ; « Utiliser » modifie les sources Debian puis met à jour la liste des paquets.",
                list.len()
            )));
            intro.set_wrap(true);
            intro.set_xalign(0.0);
            intro.add_css_class("dim-label");
            content.append(&intro);
            let filter = gtk::SearchEntry::new();
            filter.set_placeholder_text(Some("Filtrer par pays ou par nom"));
            content.append(&filter);

            // Tableau : colonnes alignées par des groupes de tailles.
            let sizes: Vec<gtk::SizeGroup> = (0..4).map(|_| gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal)).collect();
            let table = gtk::ListBox::new();
            table.add_css_class("boxed-list");
            table.set_selection_mode(gtk::SelectionMode::None);
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            head.set_margin_start(12);
            head.set_margin_end(12);
            for (i, t) in ["Pays", "Miroir", "Débit", ""].iter().enumerate() {
                let l = gtk::Label::new(Some(t));
                l.add_css_class("heading");
                l.set_xalign(0.0);
                if i == 1 {
                    l.set_hexpand(true);
                }
                sizes[i].add_widget(&l);
                head.append(&l);
            }
            content.append(&head);
            content.append(&table);

            let testers: Rc<RefCell<Vec<(Mirror, gtk::Label, gtk::Button)>>> = Rc::default();
            let cc = mirrors::session_country();
            for m in &list {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                line.set_margin_top(8);
                line.set_margin_bottom(8);
                line.set_margin_start(12);
                line.set_margin_end(12);
                let country = gtk::Label::new(Some(&format!("{}  {}", mirrors::flag(&m.country_code), m.country)));
                country.set_xalign(0.0);
                country.set_width_chars(22);
                country.set_max_width_chars(22);
                country.set_ellipsize(gtk::pango::EllipsizeMode::End);
                sizes[0].add_widget(&country);
                line.append(&country);
                let names = gtk::Box::new(gtk::Orientation::Vertical, 0);
                names.set_hexpand(true);
                let site = gtk::Label::new(Some(&m.site));
                site.set_xalign(0.0);
                site.set_selectable(true);
                names.append(&site);
                if !m.sponsor.is_empty() {
                    let sp = gtk::Label::new(Some(&m.sponsor));
                    sp.add_css_class("dim-label");
                    sp.add_css_class("caption");
                    sp.set_xalign(0.0);
                    sp.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    names.append(&sp);
                }
                sizes[1].add_widget(&names);
                line.append(&names);
                let result = gtk::Label::new(Some("—"));
                result.set_xalign(0.0);
                result.set_width_chars(18);
                result.add_css_class("dim-label");
                sizes[2].add_widget(&result);
                line.append(&result);
                let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let test = gtk::Button::with_label("Tester");
                test.set_valign(gtk::Align::Center);
                let use_b = gtk::Button::with_label("Utiliser");
                use_b.set_valign(gtk::Align::Center);
                buttons.append(&test);
                buttons.append(&use_b);
                sizes[3].add_widget(&buttons);
                line.append(&buttons);
                {
                    let (m2, res) = (m.clone(), result.clone());
                    test.connect_clicked(move |b| run_test(&m2, &res, b));
                }
                {
                    let (app2, m2, dlg2) = (app.clone(), m.clone(), dlg.clone());
                    use_b.connect_clicked(move |_| {
                        let (app3, url, site) = (app2.clone(), m2.url.clone(), m2.site.clone());
                        let dlg3 = dlg2.clone();
                        app2.confirm(
                            &format!("Utiliser {site} ?"),
                            "Les sources Debian (sid, contrib, non-free…) seront téléchargées depuis ce miroir. Vous pourrez revenir au réseau mondial à tout moment.",
                            "Utiliser",
                            false,
                            move || {
                                dlg3.close();
                                let (app4, url) = (app3.clone(), url.clone());
                                let idx = app3.backend_index(BackendKind::Apt).unwrap_or(0);
                                app3.ops.enqueue(format!("Miroir Debian : {site}"), idx, move |_| mirrors::switch_plan(&url), move |_| app4.load_repos());
                            },
                        );
                    });
                }
                let row_w = gtk::ListBoxRow::new();
                row_w.set_activatable(false);
                row_w.set_child(Some(&line));
                let key = format!("{} {} {}", m.country, m.site, m.country_code).to_lowercase();
                unsafe { row_w.set_data("noos-key", key) };
                table.append(&row_w);
                if m.country_code.is_empty() || m.country_code == cc {
                    testers.borrow_mut().push((m.clone(), result.clone(), test.clone()));
                }
            }
            {
                let t = table.clone();
                filter.connect_search_changed(move |f| {
                    let q = f.text().to_lowercase();
                    let mut c = t.first_child();
                    while let Some(w) = c {
                        c = w.next_sibling();
                        let key: String = unsafe { w.data::<String>("noos-key").map(|k| k.as_ref().clone()).unwrap_or_default() };
                        w.set_visible(q.is_empty() || key.contains(&q));
                    }
                });
            }
            // « Tester les premiers » : le CDN et les miroirs du pays (12 au plus), l'un après l'autre.
            test_all.set_sensitive(true);
            let queue: Vec<(Mirror, gtk::Label, gtk::Button)> = testers.borrow().iter().take(12).cloned().collect();
            test_all.connect_clicked(move |b| {
                b.set_sensitive(false);
                let items = queue.clone();
                let b = b.clone();
                glib::spawn_future_local(async move {
                    let mut best: Option<(f64, gtk::Label)> = None;
                    for (m, res, test) in items {
                        res.set_text("Mesure…");
                        test.set_sensitive(false);
                        let m2 = m.clone();
                        let r = background(move || mirrors::test(&m2)).await;
                        test.set_sensitive(true);
                        match r {
                            Ok(s) => {
                                res.set_text(&speed_text(&s));
                                res.remove_css_class("dim-label");
                                if best.as_ref().is_none_or(|(v, _)| s.bytes_per_s > *v) {
                                    best = Some((s.bytes_per_s, res.clone()));
                                }
                            }
                            Err(_) => res.set_text("Injoignable"),
                        }
                    }
                    if let Some((_, l)) = best {
                        l.set_text(&format!("★ {}", l.text()));
                        l.add_css_class("success");
                    }
                    b.set_sensitive(true);
                });
            });
            if std::env::var_os("NOOS_SOFTWARE_MIRRORS_TEST").is_some() {
                test_all.emit_clicked();
            }
        });
    }
}

fn run_test(m: &Mirror, res: &gtk::Label, b: &gtk::Button) {
    b.set_sensitive(false);
    res.set_text("Mesure…");
    let (m, res, b) = (m.clone(), res.clone(), b.clone());
    glib::spawn_future_local(async move {
        let r = background(move || mirrors::test(&m)).await;
        b.set_sensitive(true);
        match r {
            Ok(s) => {
                res.set_text(&speed_text(&s));
                res.remove_css_class("dim-label");
            }
            Err(_) => res.set_text("Injoignable"),
        }
    });
}

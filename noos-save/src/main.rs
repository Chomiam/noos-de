//! noos-save — « Sauvegarder et Restaurer » de NOOS (GTK4 + libAdapta).
//!
//! Instantanés Btrfs du système (snapper, config « root ») : création manuelle, instantanés
//! automatiques avant / après chaque opération dnf et réguliers, et restauration d'un
//! instantané au prochain redémarrage. Les opérations privilégiées passent par
//! `noos-save-helper` (pkexec) ; voir `snapper.rs`.

mod snapper;

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};
use snapper::{Entry, Kind, Status};

const APP_ID: &str = "org.noos.Save";

/// Exécute `f` dans un fil, résultat récupéré sur la boucle GTK.
fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("fil de travail") }
}

struct App {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    banner: adap::Banner,
    stack: gtk::Stack,
    content: gtk::Box,
    create: gtk::Button,
    busy: gtk::Spinner,
    status: RefCell<Status>,
}

fn row(title: &str, subtitle: &str) -> adap::ActionRow {
    let r = adap::ActionRow::new();
    r.set_use_markup(false);
    r.set_title(title);
    r.set_subtitle(subtitle);
    r
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// « 2026-11-07 06:31:10 » → « 7 nov. 2026 à 06:31 ».
fn nice_date(date: &str) -> String {
    let fmt = || {
        // snapper donne l'heure locale, sans fuseau.
        let dt = glib::DateTime::from_iso8601(&date.replace(' ', "T"), Some(&glib::TimeZone::local())).ok()?;
        dt.format("%-d %b %Y à %H:%M").ok().map(|s| s.to_string())
    };
    fmt().unwrap_or_else(|| date.to_string())
}

/// Titre lisible d'une description dnf (« dnf install -y git » → « dnf : install git »).
fn nice_description(e: &Entry) -> String {
    let d = e.main.description.trim();
    if d.is_empty() {
        return format!("Instantané n° {}", e.main.number);
    }
    if e.main.from_dnf() && e.main.kind != Kind::Single {
        let words: Vec<&str> = d.split_whitespace().filter(|w| !w.starts_with('-')).collect();
        let words: Vec<&str> = match words.first() {
            Some(w) if w.ends_with("dnf") || w.ends_with("dnf5") => words[1..].to_vec(),
            _ => words,
        };
        return format!("dnf : {}", words.join(" "));
    }
    d.to_string()
}

impl App {
    fn new(gapp: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(gapp)
            .title("Sauvegarder et Restaurer")
            .default_width(820)
            .default_height(720)
            .build();
        let header = adap::HeaderBar::new();
        let create = gtk::Button::new();
        create.set_child(Some(&adap::ButtonContent::builder().label("Créer").icon_name("list-add-symbolic").build()));
        create.add_css_class("suggested-action");
        create.set_tooltip_text(Some("Créer un instantané du système maintenant"));
        header.pack_start(&create);
        let busy = gtk::Spinner::new();
        header.pack_end(&busy);

        let banner = adap::Banner::new("");
        banner.set_button_label(Some("Annuler la restauration"));

        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let clamp = adap::Clamp::builder().maximum_size(760).child(&content).build();
        let scroll = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

        let stack = gtk::Stack::new();
        stack.add_named(&scroll, Some("main"));
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&banner);
        page.append(&stack);
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&page));
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&toasts));
        window.set_content(Some(&tv));

        let app = Rc::new(Self {
            window,
            toasts,
            banner,
            stack,
            content,
            create: create.clone(),
            busy,
            status: RefCell::new(Status::default()),
        });
        {
            let app2 = app.clone();
            create.connect_clicked(move |_| app2.ask_create());
        }
        {
            let app2 = app.clone();
            app.banner.connect_button_clicked(move |_| {
                app2.confirm(
                    "Annuler la restauration ?",
                    "Le système redémarrera normalement, dans son état actuel.",
                    "Annuler la restauration",
                    false,
                    {
                        let app3 = app2.clone();
                        move || app3.run(&["cancel"], "Restauration annulée")
                    },
                );
            });
        }
        app
    }

    fn reload(self: &Rc<Self>) {
        self.busy.start();
        let task = background(snapper::read);
        let app = self.clone();
        glib::spawn_future_local(async move {
            let st = task.await;
            app.busy.stop();
            *app.status.borrow_mut() = st;
            app.render();
        });
    }

    /// Lance `noos-save-helper <args>` puis recharge.
    fn run(self: &Rc<Self>, args: &[&str], done: &str) {
        self.busy.start();
        self.window.set_sensitive(false);
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let task = background(move || snapper::helper(&args.iter().map(String::as_str).collect::<Vec<_>>()));
        let (app, done) = (self.clone(), done.to_string());
        glib::spawn_future_local(async move {
            let res = task.await;
            app.window.set_sensitive(true);
            match res {
                Ok(_) => app.toasts.add_toast(adap::Toast::new(&done)),
                Err(e) => {
                    let d = adap::AlertDialog::new(Some("L'opération a échoué"), Some(&format!("{e:#}")));
                    d.add_response("ok", "Fermer");
                    d.present(Some(&app.window));
                }
            }
            app.reload();
        });
    }

    fn confirm(self: &Rc<Self>, title: &str, body: &str, action: &str, destructive: bool, f: impl Fn() + 'static) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance("ok", if destructive { adap::ResponseAppearance::Destructive } else { adap::ResponseAppearance::Suggested });
        d.set_default_response(Some("cancel"));
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f();
            }
        });
        d.present(Some(&self.window));
    }

    fn ask_create(self: &Rc<Self>) {
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Ex. : avant d'installer les pilotes NVIDIA"));
        entry.set_activates_default(true);
        let d = adap::AlertDialog::new(
            Some("Créer un instantané"),
            Some("Une photo du système (hors dossiers personnels) que vous pourrez restaurer plus tard. Les instantanés manuels ne sont jamais supprimés automatiquement."),
        );
        d.set_extra_child(Some(&entry));
        d.add_responses(&[("cancel", "Annuler"), ("create", "Créer")]);
        d.set_response_appearance("create", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("create"));
        let (app, e2) = (self.clone(), entry.clone());
        d.connect_response(None, move |_, r| {
            if r == "create" {
                let text = e2.text().trim().to_string();
                let desc = if text.is_empty() { "Instantané manuel".to_string() } else { text };
                app.run(&["create", &desc], "Instantané créé");
            }
        });
        d.present(Some(&self.window));
        glib::idle_add_local_once(move || {
            entry.grab_focus();
        });
    }

    fn render(self: &Rc<Self>) {
        let st = self.status.borrow().clone();
        clear(&self.content);
        self.create.set_sensitive(st.configured && st.pending.is_none());

        // Restauration en attente.
        match &st.pending {
            Some(p) => {
                let n = p.get("SNAPSHOT").map(String::as_str).unwrap_or("?");
                self.banner.set_title(&format!("L'instantané n° {n} sera restauré au prochain redémarrage"));
                self.banner.set_revealed(true);
            }
            None => self.banner.set_revealed(false),
        }

        if !st.btrfs {
            self.content.append(
                &adap::StatusPage::builder()
                    .icon_name("drive-harddisk-symbolic")
                    .title("Instantanés indisponibles")
                    .description("Le système n'est pas installé sur un système de fichiers Btrfs. L'installateur de NOOS utilise Btrfs : réinstallez pour profiter des instantanés.")
                    .vexpand(true)
                    .build(),
            );
            return;
        }
        if !st.configured {
            let page = adap::StatusPage::builder()
                .icon_name("document-revert-symbolic")
                .title("Protégez votre système")
                .description("Les instantanés gardent une copie du système avant chaque installation ou mise à jour. En cas de problème, vous revenez en arrière en un redémarrage. Vos dossiers personnels ne sont pas concernés.")
                .vexpand(true)
                .build();
            let b = gtk::Button::with_label("Activer les instantanés");
            b.add_css_class("pill");
            b.add_css_class("suggested-action");
            b.set_halign(gtk::Align::Center);
            let app = self.clone();
            b.connect_clicked(move |_| app.run(&["setup"], "Instantanés activés"));
            page.set_child(Some(&b));
            self.content.append(&page);
            return;
        }

        // Ancienne racine conservée après une restauration.
        if let Some(r) = &st.restored {
            let old = r.get("OLD").cloned().unwrap_or_default();
            let g = adap::PreferencesGroup::new();
            g.set_title("Restauration effectuée");
            let r2 = row(
                &format!("Système restauré depuis l'instantané n° {}", r.get("SNAPSHOT").map(String::as_str).unwrap_or("?")),
                &format!("L'ancien état est conservé ({old}) au cas où. Supprimez-le quand tout fonctionne pour libérer de la place."),
            );
            r2.set_subtitle_lines(3);
            r2.add_prefix(&gtk::Image::from_icon_name("emblem-ok-symbolic"));
            if st.pending.is_none() {
                let del = gtk::Button::with_label("Supprimer l'ancien état");
                del.set_valign(gtk::Align::Center);
                del.add_css_class("destructive-action");
                let app = self.clone();
                del.connect_clicked(move |_| {
                    let (app2, old2) = (app.clone(), old.clone());
                    app.confirm(
                        "Supprimer l'ancien état du système ?",
                        "Il ne sera plus possible de revenir à l'état d'avant la restauration.",
                        "Supprimer",
                        true,
                        move || app2.run(&["delete-old", &old2], "Ancien état supprimé"),
                    );
                });
                r2.add_suffix(&del);
            }
            g.add(&r2);
            self.content.append(&g);
        }

        // Liste.
        let g = adap::PreferencesGroup::new();
        g.set_title("Instantanés");
        match &st.entries {
            Err(e) => g.add(&row("Lecture impossible", e)),
            Ok(list) if list.is_empty() => {
                g.set_description(Some("Aucun instantané pour l'instant. Il en sera créé un avant chaque opération dnf, ou cliquez sur « Créer »."))
            }
            Ok(list) => {
                g.set_description(Some(&format!(
                    "{} instantané(s). Restaurer remplace tout le système (pas vos dossiers personnels) au prochain redémarrage.",
                    list.len()
                )));
                for e in list {
                    g.add(&self.entry_row(e, st.pending.is_some()));
                }
            }
        }
        self.content.append(&g);

        self.content.append(&self.settings(&st));
    }

    fn entry_row(self: &Rc<Self>, e: &Entry, pending: bool) -> adap::ActionRow {
        let (icon, kind) = if e.main.manual() {
            ("document-save-symbolic", "Manuel")
        } else if e.main.timeline() {
            ("alarm-symbolic", "Automatique")
        } else if e.post.is_some() || e.main.from_dnf() {
            ("system-software-install-symbolic", "Avant / après dnf")
        } else {
            ("document-revert-symbolic", "Instantané")
        };
        let numbers = e.numbers().iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" / ");
        let r = row(&nice_description(e), &format!("{} · {kind} · n° {numbers}", nice_date(&e.main.date)));
        r.set_title_lines(1);
        r.add_prefix(&gtk::Image::from_icon_name(icon));

        let restore = gtk::Button::with_label("Restaurer…");
        restore.set_valign(gtk::Align::Center);
        restore.set_sensitive(!pending);
        restore.set_tooltip_text(Some(if e.post.is_some() {
            "Revenir à l'état du système juste avant cette opération"
        } else {
            "Revenir à l'état du système à ce moment"
        }));
        {
            let (app, n, title, date) = (self.clone(), e.main.number, nice_description(e), nice_date(&e.main.date));
            let before = e.post.is_some();
            restore.connect_clicked(move |_| {
                let app2 = app.clone();
                app.confirm(
                    &format!("Restaurer « {title} » ?"),
                    &format!(
                        "Au prochain redémarrage, le système reviendra à son état du {date}{}.\n\n\
                         Les logiciels installés et les réglages système depuis seront annulés ; vos dossiers personnels (/home) ne changent pas. \
                         L'état actuel est conservé et vous pourrez annuler tant que vous n'avez pas redémarré.",
                        if before { ", juste avant cette opération" } else { "" }
                    ),
                    "Restaurer au redémarrage",
                    true,
                    move || app2.run(&["rollback", &n.to_string()], "Restauration prévue : redémarrez pour l'appliquer"),
                );
            });
        }
        r.add_suffix(&restore);

        let del = gtk::Button::from_icon_name("user-trash-symbolic");
        del.add_css_class("flat");
        del.set_valign(gtk::Align::Center);
        del.set_tooltip_text(Some("Supprimer cet instantané"));
        del.set_sensitive(!pending);
        {
            let (app, numbers, title) = (self.clone(), e.numbers(), nice_description(e));
            del.connect_clicked(move |_| {
                let (app2, numbers) = (app.clone(), numbers.clone());
                app.confirm(&format!("Supprimer « {title} » ?"), "Cet instantané ne pourra plus être restauré.", "Supprimer", true, move || {
                    let mut args = vec!["delete".to_string()];
                    args.extend(numbers.iter().map(u32::to_string));
                    app2.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "Instantané supprimé");
                });
            });
        }
        r.add_suffix(&del);
        r
    }

    fn settings(self: &Rc<Self>, st: &Status) -> adap::PreferencesGroup {
        let g = adap::PreferencesGroup::new();
        g.set_title("Réglages");

        let dnf = adap::SwitchRow::new();
        dnf.set_title("Avant et après chaque opération dnf");
        dnf.set_subtitle("Installation, suppression ou mise à jour de paquets (Logiciels, terminal…)");
        dnf.set_active(st.dnf_snapshots);
        {
            let app = self.clone();
            dnf.connect_active_notify(move |r| {
                let on = r.is_active();
                app.run(&["dnf", if on { "on" } else { "off" }], if on { "Instantanés dnf activés" } else { "Instantanés dnf désactivés" });
            });
        }
        g.add(&dnf);

        let keep = adap::SpinRow::with_range(2.0, 50.0, 1.0);
        keep.set_title("Instantanés dnf conservés");
        keep.set_subtitle("Les plus anciens sont supprimés automatiquement");
        keep.set_value(f64::from(st.number("NUMBER_LIMIT", 10)));
        self.on_spin(&keep, "NUMBER_LIMIT");
        g.add(&keep);

        let timeline = adap::ExpanderRow::new();
        timeline.set_title("Instantanés réguliers");
        timeline.set_subtitle("Un instantané automatique toutes les heures, avec un nombre limité gardé par période");
        timeline.set_show_enable_switch(true);
        timeline.set_enable_expansion(st.get("TIMELINE_CREATE") == "yes");
        {
            let app = self.clone();
            timeline.connect_enable_expansion_notify(move |r| {
                let on = r.enables_expansion();
                app.run(&["set", "TIMELINE_CREATE", if on { "yes" } else { "no" }], if on { "Instantanés réguliers activés" } else { "Instantanés réguliers désactivés" });
            });
        }
        for (key, title, default) in [
            ("TIMELINE_LIMIT_DAILY", "Quotidiens conservés", 7),
            ("TIMELINE_LIMIT_WEEKLY", "Hebdomadaires conservés", 2),
            ("TIMELINE_LIMIT_MONTHLY", "Mensuels conservés", 1),
        ] {
            let s = adap::SpinRow::with_range(0.0, 30.0, 1.0);
            s.set_title(title);
            s.set_value(f64::from(st.number(key, default)));
            self.on_spin(&s, key);
            timeline.add_row(&s);
        }
        g.add(&timeline);
        g
    }

    /// Enregistre une valeur numérique une seconde après le dernier changement.
    fn on_spin(self: &Rc<Self>, s: &adap::SpinRow, key: &'static str) {
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        let app = self.clone();
        s.connect_value_notify(move |s| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (app2, value, pending2) = (app.clone(), (s.value() as u32).to_string(), pending.clone());
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                pending2.borrow_mut().take();
                app2.run(&["set", key, &value], "Réglage enregistré");
            }));
        });
    }
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::empty()).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
    });
    app.connect_activate(|gapp| {
        if let Some(w) = gapp.active_window() {
            w.present();
            return;
        }
        let a = App::new(gapp);
        a.window.present();
        a.reload();
        let keep = RefCell::new(Some(a.clone()));
        a.window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
        let _ = &a.stack;
    });
    app.run()
}

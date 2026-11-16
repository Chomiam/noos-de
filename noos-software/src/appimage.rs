//! AppImage dans Logiciels : ouverture d'un fichier `.AppImage` (Explorateur, ligne de
//! commande), installation dans `~/Applications` avec lanceur du menu Démarrer, groupe
//! « AppImage » de la page Installées. Logique : `noos_pkg::appimage`.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};
use noos_pkg::appimage::{self, Inspection, Installed};

use crate::{App, ops::background, row};

/// `~/Applications/x` → « ~/Applications/x » (chemins affichés).
fn tilde(p: &std::path::Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

fn app_icon(icon: Option<&std::path::Path>, size: i32) -> gtk::Image {
    let img = match icon {
        Some(p) => gtk::Image::from_file(p),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    img.set_pixel_size(size);
    img
}

fn trash(path: &std::path::Path) -> Result<(), glib::Error> {
    gio::File::for_path(path).trash(None::<&gio::Cancellable>)
}

impl App {
    fn appimage_error(&self, title: &str, err: &str) {
        let d = adap::AlertDialog::new(Some(&format!("Échec : {title}")), Some(err));
        d.add_responses(&[("ok", "Fermer")]);
        d.present(Some(&self.window));
    }

    /// Ouvre une AppImage : déjà installée → lancée ; sinon fenêtre « Installer ».
    pub(crate) fn open_appimage(self: &Rc<Self>, path: PathBuf) {
        let task = background(move || appimage::inspect(&path));
        let app = self.clone();
        glib::spawn_future_local(async move {
            match task.await {
                Ok(insp) if insp.installed.is_some() => {
                    if let Err(e) = appimage::run(&insp.path) {
                        app.toasts.add_toast(adap::Toast::new(&format!("{e:#}")));
                    }
                }
                Ok(insp) => app.appimage_dialog(insp),
                Err(e) => app.toasts.add_toast(adap::Toast::new(&format!("{e:#}"))),
            }
        });
    }

    fn appimage_dialog(self: &Rc<Self>, insp: Inspection) {
        let in_place = insp.path.parent() == Some(appimage::apps_dir().as_path());
        let mut body = String::new();
        if let Some(c) = &insp.comment {
            body.push_str(c);
            body.push_str("\n\n");
        }
        body.push_str(if in_place {
            "Elle est déjà dans ~/Applications : elle sera ajoutée au menu Démarrer."
        } else {
            "Elle sera déplacée dans ~/Applications et ajoutée au menu Démarrer."
        });
        body.push_str("\nLes AppImage ne sont pas isolées du système : n'installez que celles d'une source de confiance.");
        let heading = match &insp.version {
            Some(v) => format!("Installer {} {v} ?", insp.name),
            None => format!("Installer {} ?", insp.name),
        };
        let d = adap::AlertDialog::new(Some(&heading), Some(&body));
        let extra = gtk::Box::new(gtk::Orientation::Vertical, 6);
        extra.append(&app_icon(insp.icon.as_deref(), 96));
        let file = gtk::Label::new(Some(&tilde(&insp.path)));
        file.add_css_class("dim-label");
        file.add_css_class("caption");
        file.set_wrap(true);
        file.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        extra.append(&file);
        d.set_extra_child(Some(&extra));
        d.add_responses(&[("cancel", "Annuler"), ("run", "Lancer sans installer"), ("install", "Installer")]);
        d.set_response_appearance("install", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("install"));
        d.set_close_response("cancel");
        let insp = RefCell::new(Some(insp));
        let app = self.clone();
        d.connect_response(None, move |_, r| {
            let Some(insp) = insp.borrow_mut().take() else { return };
            match r {
                "install" => app.install_appimage(insp),
                "run" => {
                    if let Err(e) = appimage::run(&insp.path) {
                        app.toasts.add_toast(adap::Toast::new(&format!("{e:#}")));
                    }
                }
                _ => {}
            }
        });
        d.present(Some(&self.window));
    }

    fn install_appimage(self: &Rc<Self>, insp: Inspection) {
        let name = insp.name.clone();
        let task = background(move || appimage::install(&insp).map_err(|e| format!("{e:#}")));
        let app = self.clone();
        glib::spawn_future_local(async move {
            match task.await {
                Ok(out) => {
                    let toast = adap::Toast::new(&format!("{name} a été ajouté au menu Démarrer"));
                    toast.set_button_label(Some("Ouvrir"));
                    let path = out.app.path.clone();
                    let a = app.clone();
                    toast.connect_button_clicked(move |_| {
                        if let Err(e) = appimage::run(&path) {
                            a.toasts.add_toast(adap::Toast::new(&format!("{e:#}")));
                        }
                    });
                    app.toasts.add_toast(toast);
                    app.render_installed();
                    if let Some(old) = out.previous {
                        let a = app.clone();
                        app.confirm(
                            "Mettre l'ancienne version à la corbeille ?",
                            &format!("{} n'est plus dans le menu Démarrer : la nouvelle version la remplace.", tilde(&old)),
                            "Mettre à la corbeille",
                            true,
                            move || {
                                if let Err(e) = trash(&old) {
                                    a.toasts.add_toast(adap::Toast::new(&format!("Corbeille : {e}")));
                                }
                                a.render_installed();
                            },
                        );
                    }
                }
                Err(e) => app.appimage_error(&format!("Installation de {name}"), &e),
            }
        });
    }

    fn remove_appimage(self: &Rc<Self>, inst: Installed) {
        let app = self.clone();
        self.confirm(
            &format!("Supprimer {} ?", inst.name),
            &format!("{} sera mise à la corbeille et retirée du menu Démarrer. Ses réglages (dans ~/.config) sont conservés.", tilde(&inst.path)),
            "Supprimer",
            true,
            move || {
                let result = appimage::unintegrate(&inst).map_err(|e| format!("{e:#}")).and_then(|_| {
                    if inst.path.exists() { trash(&inst.path).map_err(|e| e.to_string()) } else { Ok(()) }
                });
                match result {
                    Ok(()) => app.toasts.add_toast(adap::Toast::new(&format!("{} a été supprimé", inst.name))),
                    Err(e) => app.appimage_error(&format!("Suppression de {}", inst.name), &e),
                }
                app.render_installed();
            },
        );
    }

    /// Fenêtre de choix d'un fichier `.AppImage`.
    fn pick_appimage(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("AppImage"));
        for mime in appimage::MIME_TYPES {
            filter.add_mime_type(mime);
        }
        filter.add_suffix("AppImage");
        filter.add_suffix("appimage");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder().title("Ajouter une AppImage").filters(&filters).default_filter(&filter).build();
        if let Some(dl) = glib::user_special_dir(glib::UserDirectory::Downloads) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dl)));
        }
        let app = self.clone();
        dialog.open(Some(&self.window), None::<&gio::Cancellable>, move |res| {
            if let Some(path) = res.ok().and_then(|f| f.path()) {
                app.open_appimage(path);
            }
        });
    }

    /// Groupe « AppImage » de la page Installées (filtré comme les autres groupes).
    pub(crate) fn appimage_group(self: &Rc<Self>, filter: &str) -> adap::PreferencesGroup {
        let list = appimage::installed();
        let loose = appimage::unintegrated();
        let g = adap::PreferencesGroup::new();
        g.set_title(&format!("AppImage — {}", list.len()));
        g.set_description(Some("Applications portables de ~/Applications, ajoutées au menu Démarrer"));
        let add = gtk::Button::with_label("Ajouter…");
        add.add_css_class("flat");
        add.set_valign(gtk::Align::Center);
        add.set_tooltip_text(Some("Installer un fichier .AppImage"));
        let app = self.clone();
        add.connect_clicked(move |_| app.pick_appimage());
        g.set_header_suffix(Some(&add));

        let matches = |name: &str, path: &std::path::Path| {
            filter.is_empty() || name.to_lowercase().contains(filter) || path.to_string_lossy().to_lowercase().contains(filter)
        };
        let shown: Vec<&Installed> = list.iter().filter(|i| matches(&i.name, &i.path)).collect();
        let loose: Vec<&PathBuf> = loose.iter().filter(|p| matches("", p)).collect();
        if shown.is_empty() && loose.is_empty() {
            g.add(&row(
                if list.is_empty() { "Aucune" } else { "Aucun résultat pour ce filtre" },
                if list.is_empty() { "Ouvrez un fichier .AppImage (Explorateur) ou utilisez « Ajouter… »." } else { "" },
            ));
        }
        for inst in shown {
            let mut sub = Vec::new();
            if let Some(v) = &inst.version {
                sub.push(v.clone());
            }
            sub.push(glib::format_size(inst.size()).to_string());
            sub.push(tilde(&inst.path));
            let r = row(&inst.name, &sub.join(" · "));
            r.add_prefix(&app_icon(inst.icon.as_deref(), 32));
            if !inst.path.exists() {
                r.set_subtitle(&format!("Fichier introuvable : {}", tilde(&inst.path)));
            }
            let open = gtk::Button::with_label("Ouvrir");
            open.set_valign(gtk::Align::Center);
            open.set_sensitive(inst.path.exists());
            let (app, path) = (self.clone(), inst.path.clone());
            open.connect_clicked(move |_| {
                if let Err(e) = appimage::run(&path) {
                    app.toasts.add_toast(adap::Toast::new(&format!("{e:#}")));
                }
            });
            let remove = gtk::Button::with_label("Supprimer");
            remove.add_css_class("destructive-action");
            remove.set_valign(gtk::Align::Center);
            let (app, i) = (self.clone(), inst.clone());
            remove.connect_clicked(move |_| app.remove_appimage(i.clone()));
            r.add_suffix(&open);
            r.add_suffix(&remove);
            g.add(&r);
        }
        for path in loose {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let r = row(&name, "Pas dans le menu Démarrer");
            r.add_prefix(&app_icon(None, 32));
            let integrate = gtk::Button::with_label("Intégrer");
            integrate.add_css_class("suggested-action");
            integrate.set_valign(gtk::Align::Center);
            let (app, p) = (self.clone(), path.clone());
            integrate.connect_clicked(move |_| app.open_appimage(p.clone()));
            r.add_suffix(&integrate);
            g.add(&r);
        }
        g
    }
}

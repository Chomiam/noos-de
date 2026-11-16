//! Paquets `.deb` ouverts depuis l'Explorateur : fiche du paquet, simulation des
//! dépendances (sans mot de passe), puis installation par la file d'opérations (pkexec,
//! `apt-get install <chemin>`). Logique : `noos_pkg::deb`.

use std::{path::PathBuf, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};
use noos_pkg::{
    BackendKind,
    deb::{self, DebInfo, Simulation},
};

use crate::{App, ops::background};

/// Liste courte : « a, b, c et 4 autres ».
fn short_list(names: &[String]) -> String {
    match names.len() {
        0 => String::new(),
        1..=4 => names.join(", "),
        n => format!("{} et {} autres", names[..3].join(", "), n - 3),
    }
}

/// Application du paquet installé (lanceur dans /usr/share/applications), pour « Ouvrir ».
fn installed_app(package: &str) -> Option<gio::AppInfo> {
    let out = std::process::Command::new("dpkg-query").args(["-L", package]).output().ok()?;
    let ids: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("/usr/share/applications/") && l.ends_with(".desktop"))
        .filter_map(|l| l.rsplit('/').next().map(str::to_string))
        .collect();
    gio::AppInfo::all().into_iter().find(|a| a.should_show() && a.id().is_some_and(|id| ids.iter().any(|i| *i == id.as_str())))
}

impl App {
    /// Ouvre un fichier .deb : lecture + simulation en arrière-plan, puis fenêtre.
    pub(crate) fn open_deb(self: &Rc<Self>, path: PathBuf) {
        if self.backend_index(BackendKind::Apt).is_none() {
            self.toasts.add_toast(adap::Toast::new("apt est indisponible sur ce système"));
            return;
        }
        let task = background(move || {
            deb::inspect(&path).map(|info| {
                let sim = deb::simulate(&info).map_err(|e| format!("{e:#}"));
                (info, sim)
            })
        });
        let app = self.clone();
        glib::spawn_future_local(async move {
            match task.await {
                Ok((info, sim)) => app.deb_dialog(info, sim),
                Err(e) => app.toasts.add_toast(adap::Toast::new(&format!("{e:#}"))),
            }
        });
    }

    fn deb_dialog(self: &Rc<Self>, info: DebInfo, sim: Result<Simulation, String>) {
        let same = info.installed_version.as_deref() == Some(info.version.as_str());
        let (heading, action) = match &info.installed_version {
            _ if sim.is_err() => (format!("Impossible d'installer {} {}", info.package, info.version), ""),
            Some(_) if same =>(format!("Réinstaller {} {} ?", info.package, info.version), "Réinstaller"),
            Some(v) => (format!("Mettre à jour {} ({v} → {}) ?", info.package, info.version), "Mettre à jour"),
            None => (format!("Installer {} {} ?", info.package, info.version), "Installer"),
        };
        let mut body = info.summary.clone();
        let mut ok = true;
        match &sim {
            Ok(s) => {
                if !s.extra.is_empty() {
                    body.push_str(&format!("\n\nDépendances installées depuis les dépôts : {}.", short_list(&s.extra)));
                }
                if !s.upgraded.is_empty() {
                    body.push_str(&format!("\nMis à jour au passage : {}.", short_list(&s.upgraded)));
                }
                if !s.removed.is_empty() {
                    body.push_str(&format!("\n⚠ Paquets retirés (conflit) : {}.", short_list(&s.removed)));
                }
            }
            Err(e) => {
                ok = false;
                body.push_str(&format!("\n\nCe paquet ne peut pas être installé :\n{e}"));
            }
        }
        if ok {
            body.push_str("\n\nUn paquet .deb s'installe pour tout le système, avec les droits administrateur : n'installez que ceux d'une source de confiance.");
        }
        let d = adap::AlertDialog::new(Some(&heading), Some(&body));

        // Détails : architecture, taille, mainteneur, site, fichier.
        let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let mut lines = vec![format!("Architecture : {}", info.architecture)];
        if info.installed_size > 0 {
            lines.push(format!("Taille une fois installé : {}", glib::format_size(info.installed_size)));
        }
        if !info.maintainer.is_empty() {
            lines.push(format!("Mainteneur : {}", info.maintainer));
        }
        if let Some(h) = &info.homepage {
            lines.push(format!("Site : {h}"));
        }
        let home = glib::home_dir();
        lines.push(match info.path.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => info.path.display().to_string(),
        });
        for l in lines {
            let label = gtk::Label::new(Some(&l));
            label.add_css_class("dim-label");
            label.add_css_class("caption");
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            details.append(&label);
        }
        let theme = gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap());
        let icon = gtk::Image::from_icon_name(if theme.has_icon("application-x-deb") { "application-x-deb" } else { "package-x-generic" });
        icon.set_pixel_size(64);
        let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
        extra.append(&icon);
        extra.append(&details);
        d.set_extra_child(Some(&extra));

        if ok {
            d.add_responses(&[("cancel", "Annuler"), ("install", action)]);
            d.set_response_appearance("install", adap::ResponseAppearance::Suggested);
            d.set_default_response(Some("install"));
        } else {
            d.add_responses(&[("cancel", "Fermer")]);
        }
        d.set_close_response("cancel");
        let app = self.clone();
        d.connect_response(None, move |_, r| {
            if r == "install" {
                app.install_deb(info.clone());
            }
        });
        d.present(Some(&self.window));
    }

    fn install_deb(self: &Rc<Self>, info: DebInfo) {
        let Some(idx) = self.backend_index(BackendKind::Apt) else { return };
        let plan = deb::install_plan(&info);
        let app = self.clone();
        let name = info.package.clone();
        self.ops.enqueue(
            format!("Installation de {} {} (fichier .deb)", info.package, info.version),
            idx,
            move |_| Ok(plan),
            move |ok| {
                app.after_change();
                if !ok {
                    return;
                }
                let toast = adap::Toast::new(&format!("{name} est installé"));
                if let Some(appinfo) = installed_app(&name) {
                    toast.set_button_label(Some("Ouvrir"));
                    let a = app.clone();
                    toast.connect_button_clicked(move |_| {
                        if let Err(e) = appinfo.launch(&[], None::<&gio::AppLaunchContext>) {
                            a.toasts.add_toast(adap::Toast::new(&format!("{e}")));
                        }
                    });
                }
                app.toasts.add_toast(toast);
            },
        );
    }
}

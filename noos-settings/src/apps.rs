//! Applications : applications installées (Logiciels) et applications par défaut
//! (`default_apps.rs`).

use std::rc::Rc;

use crate::ui::{App, card, group, link, page};
use adap::prelude::*;

pub fn root(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &[], "Applications");
    let g = group("");
    g.add(&link(
        app,
        "preferences-system-symbolic",
        "Applications par défaut",
        "Applications qui ouvrent vos fichiers et vos liens",
        default_apps,
    ));
    let installed = card("view-app-grid-symbolic", "Applications installées", "Installer, mettre à jour ou supprimer des applications");
    installed.set_activatable(true);
    installed.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    installed.connect_activated(|_| {
        let _ = std::process::Command::new("noos-software").env("NOOS_SOFTWARE_PAGE", "installed").spawn();
    });
    g.add(&installed);
    content.append(&g);
    p
}

pub fn default_apps(app: &Rc<App>) -> adap::NavigationPage {
    crate::default_apps::page(app)
}

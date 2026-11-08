//! Icône « Mises à jour disponibles » de la barre des tâches (deux flèches en cercle et
//! pastille orange, comme Windows) : visible quand le service `noos-update-check` a trouvé des
//! mises à jour ; un clic ouvre la page Mises à jour de Logiciels.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{Shell, apps};

const CSS: &str = r#"
.update-dot { min-width: 8px; min-height: 8px; border-radius: 99px; background-color: #ff8c1a;
  box-shadow: 0 0 0 2px alpha(black, 0.35); margin: 5px 3px 0 0; }
"#;

pub struct UpdateIcon {
    buttons: std::cell::RefCell<Vec<gtk::Button>>,
    shell: std::rc::Weak<Shell>,
    _monitor: Option<gio::FileMonitor>,
}

fn summary(st: &noos_core::updates::State) -> String {
    let mut s = match st.total {
        1 => "1 mise à jour disponible".to_string(),
        n => format!("{n} mises à jour disponibles"),
    };
    for (name, n) in &st.by_backend {
        s.push_str(&format!("\n{name} : {n}"));
    }
    s.push_str(&format!("\nVérifié {}", noos_core::updates::ago(st.checked_at)));
    s
}

impl UpdateIcon {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        if let Some(d) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }
        let path = noos_core::updates::path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Le fichier d'état est remplacé (écriture atomique) : on surveille son dossier.
        let monitor = gio::File::for_path(path.parent().unwrap_or(&path)).monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE).ok();
        let u = Rc::new(Self { buttons: Default::default(), shell: Rc::downgrade(shell), _monitor: monitor.clone() });
        if let Some(m) = monitor {
            let w = Rc::downgrade(&u);
            m.connect_changed(move |_, f, _, _| {
                if f.basename().is_some_and(|b| b.as_os_str() == "updates.json") {
                    if let Some(u) = w.upgrade() {
                        u.refresh();
                    }
                }
            });
        }
        // Le texte « vérifié il y a… » vieillit : relecture toutes les 10 minutes.
        {
            let w = Rc::downgrade(&u);
            glib::timeout_add_seconds_local(600, move || {
                let Some(u) = w.upgrade() else { return glib::ControlFlow::Break };
                u.refresh();
                glib::ControlFlow::Continue
            });
        }
        u
    }

    /// Bouton pour une barre des tâches.
    pub fn button(self: &Rc<Self>) -> gtk::Button {
        let b = gtk::Button::new();
        b.add_css_class("status-icon");
        b.set_valign(gtk::Align::Center);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&gtk::Image::from_icon_name("view-refresh-symbolic")));
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("update-dot");
        dot.set_halign(gtk::Align::End);
        dot.set_valign(gtk::Align::Start);
        dot.set_can_target(false);
        overlay.add_overlay(&dot);
        b.set_child(Some(&overlay));
        b.connect_clicked(|_| apps::spawn("noos-software --updates"));
        self.buttons.borrow_mut().push(b.clone());
        self.refresh();
        b
    }

    pub fn refresh(&self) {
        let st = noos_core::updates::load();
        let show = st.total > 0 && self.shell.upgrade().is_none_or(|s| s.config().updates.tray_icon) || std::env::var_os("NOOS_UPDATES_DEMO").is_some();
        let tip = if std::env::var_os("NOOS_UPDATES_DEMO").is_some() && st.total == 0 { "12 mises à jour disponibles\nFedora : 9\nApplications Flatpak : 3".to_string() } else { summary(&st) };
        for b in self.buttons.borrow().iter() {
            b.set_visible(show);
            b.set_tooltip_text(Some(&tip));
        }
    }
}

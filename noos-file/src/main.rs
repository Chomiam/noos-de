//! noos-file — explorateur de fichiers de NOOS, dans l'esprit de l'Explorateur de Windows 11.
//!
//! - barre latérale : Accueil, dossiers personnels, disques et volumes montés, corbeille ;
//! - précédent / suivant / dossier parent, fil d'Ariane (Ctrl+L pour saisir un chemin) ;
//! - vues Grille et Détails, recherche dans le dossier, fichiers cachés (Ctrl+H) ;
//! - couper / copier / coller, renommer, nouveau dossier, mise à la corbeille (Suppr).

use std::{cell::RefCell, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "org.noos.Files";
const ATTRS: &str = "standard::*,time::modified,thumbnail::path,access::can-rename";

#[derive(Clone, Copy, PartialEq)]
enum ClipOp {
    Copy,
    Cut,
}

struct Explorer {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    current: RefCell<gio::File>,
    back: RefCell<Vec<gio::File>>,
    forward: RefCell<Vec<gio::File>>,
    dir_list: gtk::DirectoryList,
    filter: gtk::CustomFilter,
    selection: gtk::MultiSelection,
    show_hidden: RefCell<bool>,
    query: RefCell<String>,
    crumbs: gtk::Box,
    folder_title: adap::WindowTitle,
    location: gtk::Entry,
    location_stack: gtk::Stack,
    view_stack: gtk::Stack,
    status: gtk::Label,
    back_btn: gtk::Button,
    forward_btn: gtk::Button,
    up_btn: gtk::Button,
    sidebar: gtk::ListBox,
    clipboard: RefCell<Option<(ClipOp, Vec<gio::File>)>>,
}

fn file_of(obj: &glib::Object) -> Option<gio::File> {
    obj.downcast_ref::<gio::FileInfo>()?.attribute_object("standard::file")?.downcast().ok()
}

fn is_dir(info: &gio::FileInfo) -> bool {
    info.file_type() == gio::FileType::Directory
}

fn human_size(bytes: i64) -> String {
    glib::format_size(bytes.max(0) as u64).to_string()
}

fn special_dir(dir: glib::UserDirectory) -> Option<std::path::PathBuf> {
    glib::user_special_dir(dir)
}

impl Explorer {
    fn new(app: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Explorateur de fichiers")
            .default_width(1150)
            .default_height(720)
            .build();

        // ── Barre latérale ──
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let sidebar_page = adap::ToolbarView::new();
        let sb_header = adap::HeaderBar::new();
        sb_header.set_title_widget(Some(&adap::WindowTitle::new("Explorateur", "")));
        sidebar_page.add_top_bar(&sb_header);
        sidebar_page.set_content(Some(
            &gtk::ScrolledWindow::builder().child(&sidebar).hscrollbar_policy(gtk::PolicyType::Never).build(),
        ));

        // ── Contenu ──
        let header = adap::HeaderBar::new();
        let back_btn = gtk::Button::from_icon_name("go-previous-symbolic");
        back_btn.set_tooltip_text(Some("Précédent (Alt+←)"));
        let forward_btn = gtk::Button::from_icon_name("go-next-symbolic");
        forward_btn.set_tooltip_text(Some("Suivant (Alt+→)"));
        let up_btn = gtk::Button::from_icon_name("go-up-symbolic");
        up_btn.set_tooltip_text(Some("Dossier parent (Alt+↑)"));
        // Rangée de navigation sous la barre de titre (Précédent, Suivant, Parent, adresse sur
        // toute la largeur, recherche), comme l'Explorateur de Windows 11.
        let nav_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        nav_row.add_css_class("nav-row");
        nav_row.append(&back_btn);
        nav_row.append(&forward_btn);
        nav_row.append(&up_btn);
        for b in [&back_btn, &forward_btn, &up_btn] {
            b.add_css_class("flat");
        }
        let folder_title = adap::WindowTitle::new("Explorateur de fichiers", "");
        header.set_title_widget(Some(&folder_title));

        // ── Barre d'adresse (comme l'Explorateur de Windows) : fil d'Ariane cliquable ; un
        // clic dans la zone vide affiche le chemin complet en texte, sélectionné, à copier
        // ou à modifier (Échap ou clic ailleurs : retour au fil d'Ariane). ──
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        crumbs.set_valign(gtk::Align::Center);
        let crumbs_scroll = gtk::ScrolledWindow::builder()
            .child(&crumbs)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .min_content_width(200)
            .hexpand(true)
            .build();
        let location = gtk::Entry::new();
        location.set_hexpand(true);
        location.add_css_class("address-entry");
        location.set_placeholder_text(Some("Chemin, par ex. ~/Documents ou /etc"));
        let location_stack = gtk::Stack::new();
        location_stack.add_named(&crumbs_scroll, Some("crumbs"));
        location_stack.add_named(&location, Some("entry"));
        location_stack.set_hexpand(true);
        location_stack.set_hhomogeneous(false);
        location_stack.set_interpolate_size(true);
        let address = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        address.add_css_class("address-bar");
        address.set_hexpand(true);
        address.set_size_request(320, -1);
        address.append(&location_stack);
        nav_row.append(&address);
        {
            // Clic dans la zone vide (les boutons du fil d'Ariane gardent leurs propres clics).
            let click = gtk::GestureClick::new();
            let (stack, entry) = (location_stack.clone(), location.clone());
            click.connect_released(move |_, _, _, _| {
                if stack.visible_child_name().as_deref() == Some("crumbs") {
                    stack.set_visible_child_name("entry");
                    entry.grab_focus();
                    entry.select_region(0, -1);
                }
            });
            crumbs_scroll.add_controller(click);
        }
        {
            // Échap ou perte du focus : retour au fil d'Ariane.
            let keys = gtk::EventControllerKey::new();
            let stack = location_stack.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    stack.set_visible_child_name("crumbs");
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            location.add_controller(keys);
            let focus = gtk::EventControllerFocus::new();
            let stack = location_stack.clone();
            focus.connect_leave(move |_| stack.set_visible_child_name("crumbs"));
            location.add_controller(focus);
        }
        {
            // Le dossier courant (fin du fil d'Ariane) reste visible même si le chemin est long :
            // à chaque changement de taille du contenu ou de la zone visible, on va au bout.
            let adj = crumbs_scroll.hadjustment();
            let to_end = |adj: &gtk::Adjustment| adj.set_value((adj.upper() - adj.page_size()).max(0.0));
            adj.connect_upper_notify(move |a| to_end(a));
            adj.connect_page_size_notify(move |a| to_end(a));
        }

        let view_toggle = gtk::ToggleButton::new();
        view_toggle.set_icon_name("view-list-symbolic");
        view_toggle.set_tooltip_text(Some("Vue Détails"));
        let new_folder = gtk::Button::from_icon_name("folder-new-symbolic");
        new_folder.set_tooltip_text(Some("Nouveau dossier (Ctrl+Maj+N)"));
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher"));
        search.set_width_chars(18);
        header.pack_end(&view_toggle);
        header.pack_end(&new_folder);
        nav_row.append(&search);

        // ── Modèle de liste ──
        let home = gio::File::for_path(glib::home_dir());
        let dir_list = gtk::DirectoryList::new(Some(ATTRS), Some(&home));
        dir_list.set_monitored(true);
        let filter = gtk::CustomFilter::new(|_| true);
        let filtered = gtk::FilterListModel::new(Some(dir_list.clone()), Some(filter.clone()));
        let sorter = gtk::CustomSorter::new(|a, b| {
            let (a, b) = (a.downcast_ref::<gio::FileInfo>().unwrap(), b.downcast_ref::<gio::FileInfo>().unwrap());
            match (is_dir(a), is_dir(b)) {
                (true, false) => gtk::Ordering::Smaller,
                (false, true) => gtk::Ordering::Larger,
                _ => glib::FilenameCollationKey::from(a.display_name().as_str())
                    .cmp(&glib::FilenameCollationKey::from(b.display_name().as_str()))
                    .into(),
            }
        });
        let sorted = gtk::SortListModel::new(Some(filtered), Some(sorter));
        let selection = gtk::MultiSelection::new(Some(sorted));

        let grid = build_grid(&selection);
        let details = build_details(&selection);
        let view_stack = gtk::Stack::new();
        view_stack.add_named(&gtk::ScrolledWindow::builder().child(&grid).vexpand(true).build(), Some("grid"));
        view_stack.add_named(&gtk::ScrolledWindow::builder().child(&details).vexpand(true).build(), Some("details"));

        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.add_css_class("dim-label");
        status.set_margin_start(12);
        status.set_margin_top(4);
        status.set_margin_bottom(4);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_box.append(&view_stack);
        content_box.append(&status);

        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&content_box));
        let content_page = adap::ToolbarView::new();
        content_page.add_top_bar(&header);
        content_page.add_top_bar(&nav_row);
        content_page.set_content(Some(&toasts));

        let split = adap::NavigationSplitView::new();
        split.set_sidebar(Some(&adap::NavigationPage::new(&sidebar_page, "Emplacements")));
        split.set_content(Some(&adap::NavigationPage::new(&content_page, "Fichiers")));
        split.set_min_sidebar_width(200.0);
        window.set_content(Some(&split));

        let ex = Rc::new(Self {
            window,
            toasts,
            current: RefCell::new(home.clone()),
            back: RefCell::new(vec![]),
            forward: RefCell::new(vec![]),
            dir_list,
            filter,
            selection,
            show_hidden: RefCell::new(false),
            query: RefCell::new(String::new()),
            crumbs,
            folder_title,
            location,
            location_stack,
            view_stack,
            status,
            back_btn,
            forward_btn,
            up_btn,
            sidebar,
            clipboard: RefCell::new(None),
        });

        ex.setup_filter();
        ex.fill_sidebar();
        ex.connect_signals(&grid, &details, &view_toggle, &new_folder, &search);
        ex.navigate(home, true);
        ex
    }

    fn setup_filter(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.filter.set_filter_func(move |obj| {
            let Some(ex) = weak.upgrade() else { return true };
            let info = obj.downcast_ref::<gio::FileInfo>().unwrap();
            if !*ex.show_hidden.borrow() && (info.is_hidden() || info.is_backup()) {
                return false;
            }
            let q = ex.query.borrow();
            q.is_empty() || info.display_name().to_lowercase().contains(q.as_str())
        });
        let weak = Rc::downgrade(self);
        self.selection.connect_items_changed(move |_, _, _, _| {
            if let Some(ex) = weak.upgrade() {
                ex.update_status();
            }
        });
        let weak = Rc::downgrade(self);
        self.selection.connect_selection_changed(move |_, _, _| {
            if let Some(ex) = weak.upgrade() {
                ex.update_status();
            }
        });
    }

    fn fill_sidebar(self: &Rc<Self>) {
        while let Some(row) = self.sidebar.row_at_index(0) {
            self.sidebar.remove(&row);
        }
        let mut places: Vec<(String, String, gio::File)> = vec![(
            "Accueil".into(),
            "user-home-symbolic".into(),
            gio::File::for_path(glib::home_dir()),
        )];
        for (dir, label, icon) in [
            (glib::UserDirectory::Desktop, "Bureau", "user-desktop-symbolic"),
            (glib::UserDirectory::Documents, "Documents", "folder-documents-symbolic"),
            (glib::UserDirectory::Downloads, "Téléchargements", "folder-download-symbolic"),
            (glib::UserDirectory::Pictures, "Images", "folder-pictures-symbolic"),
            (glib::UserDirectory::Music, "Musique", "folder-music-symbolic"),
            (glib::UserDirectory::Videos, "Vidéos", "folder-videos-symbolic"),
        ] {
            if let Some(p) = special_dir(dir).filter(|p| p.exists() && p != &glib::home_dir()) {
                places.push((label.into(), icon.into(), gio::File::for_path(p)));
            }
        }
        places.push(("Ce PC".into(), "drive-harddisk-symbolic".into(), gio::File::for_path("/")));
        for mount in gio::VolumeMonitor::get().mounts() {
            let icon = gio::prelude::IconExt::to_string(&mount.symbolic_icon()).map(|s| s.to_string());
            places.push((
                mount.name().to_string(),
                icon.unwrap_or_else(|| "drive-removable-media-symbolic".into()),
                mount.root(),
            ));
        }
        places.push(("Corbeille".into(), "user-trash-symbolic".into(), gio::File::for_uri("trash:///")));

        for (label, icon, file) in places {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            h.append(&gtk::Image::from_icon_name(&icon));
            let l = gtk::Label::new(Some(&label));
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&l);
            row.set_child(Some(&h));
            unsafe { row.set_data("noos-file", file) };
            self.sidebar.append(&row);
        }
    }

    fn connect_signals(
        self: &Rc<Self>,
        grid: &gtk::GridView,
        details: &gtk::ColumnView,
        view_toggle: &gtk::ToggleButton,
        new_folder: &gtk::Button,
        search: &gtk::SearchEntry,
    ) {
        let ex = self.clone();
        self.sidebar.connect_row_activated(move |_, row| {
            if let Some(f) = unsafe { row.data::<gio::File>("noos-file") } {
                let f = unsafe { f.as_ref() }.clone();
                ex.navigate(f, true);
            }
        });
        let ex = self.clone();
        self.back_btn.connect_clicked(move |_| ex.go_back());
        let ex = self.clone();
        self.forward_btn.connect_clicked(move |_| ex.go_forward());
        let ex = self.clone();
        self.up_btn.connect_clicked(move |_| ex.go_up());

        let ex = self.clone();
        grid.connect_activate(move |_, pos| ex.open_position(pos));
        let ex = self.clone();
        details.connect_activate(move |_, pos| ex.open_position(pos));

        let ex = self.clone();
        view_toggle.connect_toggled(move |b| {
            ex.view_stack.set_visible_child_name(if b.is_active() { "details" } else { "grid" });
            b.set_icon_name(if b.is_active() { "view-grid-symbolic" } else { "view-list-symbolic" });
        });
        let ex = self.clone();
        new_folder.connect_clicked(move |_| ex.new_folder());
        let ex = self.clone();
        search.connect_search_changed(move |s| {
            *ex.query.borrow_mut() = s.text().to_lowercase();
            ex.filter.changed(gtk::FilterChange::Different);
        });

        let ex = self.clone();
        self.location.connect_activate(move |e| {
            let text = e.text().to_string();
            let file = if text.contains("://") {
                gio::File::for_uri(&text)
            } else {
                gio::File::for_path(shellexpand_home(&text))
            };
            ex.location_stack.set_visible_child_name("crumbs");
            ex.navigate(file, true);
        });

        // Clic dans le vide (hors de tout élément, sans glisser) : désélectionner tout.
        for view in [grid.clone().upcast::<gtk::Widget>(), details.clone().upcast::<gtk::Widget>()] {
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let pressed_at = Rc::new(std::cell::Cell::new((0.0, 0.0)));
            {
                let pressed_at = pressed_at.clone();
                click.connect_pressed(move |_, _, x, y| pressed_at.set((x, y)));
            }
            let (ex, v) = (self.clone(), view.clone());
            click.connect_released(move |g, n, x, y| {
                let (px, py) = pressed_at.get();
                let moved = (x - px).abs() > 4.0 || (y - py).abs() > 4.0;
                let mods = g.current_event_state();
                if n != 1 || moved || mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK) {
                    return;
                }
                if !on_item(&v, x, y) {
                    ex.selection.unselect_all();
                }
            });
            view.add_controller(click);
        }

        // Menu contextuel (clic droit) sur les vues.
        for view in [grid.clone().upcast::<gtk::Widget>(), details.clone().upcast::<gtk::Widget>()] {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let ex = self.clone();
            let v = view.clone();
            gesture.connect_pressed(move |g, _, x, y| {
                g.set_state(gtk::EventSequenceState::Claimed);
                ex.context_menu(&v, x, y);
            });
            view.add_controller(gesture);
        }

        self.install_actions();

        // Test visuel sans souris : NOOS_FILE_SELECT_DEMO=0,1,2 sélectionne ces éléments, puis
        // (4 s plus tard) rejoue un clic dans le coin inférieur droit de la vue en icônes.
        if let Ok(list) = std::env::var("NOOS_FILE_SELECT_DEMO") {
            let (ex, g) = (self.clone(), grid.clone().upcast::<gtk::Widget>());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                for i in list.split(',').filter_map(|v| v.trim().parse::<u32>().ok()) {
                    ex.selection.select_item(i, false);
                }
                glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
                    let (x, y) = (g.width() as f64 - 20.0, g.height() as f64 - 20.0);
                    let hit = on_item(&g, x, y);
                    eprintln!("noos-file : démo clic vide ({x}, {y}) sur un élément : {hit}");
                    if !hit {
                        ex.selection.unselect_all();
                    }
                });
            });
        }

        // Test visuel sans souris : NOOS_FILE_MENU_DEMO=x,y ouvre le menu contextuel de la vue
        // en icônes à cette position (y négatif : depuis le bas de la vue).
        if let Ok(pos) = std::env::var("NOOS_FILE_MENU_DEMO") {
            let (ex, g) = (self.clone(), grid.clone().upcast::<gtk::Widget>());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                let mut it = pos.split(',').filter_map(|v| v.trim().parse::<f64>().ok());
                let (x, y) = (it.next().unwrap_or(100.0), it.next().unwrap_or(100.0));
                let y = if y < 0.0 { g.height() as f64 + y } else { y };
                ex.context_menu(&g, x, y);
            });
        }
    }

    fn install_actions(self: &Rc<Self>) {
        let app = self.window.application().unwrap();
        let add = |name: &str, accels: &[&str], f: Box<dyn Fn(&Rc<Explorer>)>| {
            let action = gio::SimpleAction::new(name, None);
            let ex = self.clone();
            action.connect_activate(move |_, _| f(&ex));
            self.window.add_action(&action);
            app.set_accels_for_action(&format!("win.{name}"), accels);
        };
        add("back", &["<Alt>Left", "BackSpace"], Box::new(|ex| ex.go_back()));
        add("forward", &["<Alt>Right"], Box::new(|ex| ex.go_forward()));
        add("up", &["<Alt>Up"], Box::new(|ex| ex.go_up()));
        add("location", &["<Ctrl>l"], Box::new(|ex| ex.edit_location()));
        add("hidden", &["<Ctrl>h"], Box::new(|ex| ex.toggle_hidden()));
        add("new-folder", &["<Ctrl><Shift>n"], Box::new(|ex| ex.new_folder()));
        add("trash", &["Delete"], Box::new(|ex| ex.trash_selected()));
        add("rename", &["F2"], Box::new(|ex| ex.rename_selected()));
        add("copy", &["<Ctrl>c"], Box::new(|ex| ex.clip(ClipOp::Copy)));
        add("cut", &["<Ctrl>x"], Box::new(|ex| ex.clip(ClipOp::Cut)));
        add("paste", &["<Ctrl>v"], Box::new(|ex| ex.paste()));
        add("open", &[], Box::new(|ex| ex.open_selected()));
        add("copy-path", &["<Ctrl><Shift>c"], Box::new(|ex| ex.copy_path()));
        add("terminal", &[], Box::new(|ex| ex.open_terminal()));
        add("refresh", &["F5"], Box::new(|ex| ex.navigate(ex.current.borrow().clone(), false)));
    }

    // ───────────────────────── Navigation ─────────────────────────

    fn navigate(&self, file: gio::File, push_history: bool) {
        let previous = self.current.replace(file.clone());
        if push_history && !previous.equal(&file) {
            self.back.borrow_mut().push(previous);
            self.forward.borrow_mut().clear();
        }
        self.dir_list.set_file(Some(&file));
        self.update_crumbs(&file);
        self.update_nav_buttons();
        let title = file.basename().map(|b| b.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
        let title = if file.uri() == "trash:///" { "Corbeille".to_string() } else { title };
        self.window.set_title(Some(&format!("{title} — Explorateur de fichiers")));
    }

    fn go_back(&self) {
        let Some(prev) = self.back.borrow_mut().pop() else { return };
        self.forward.borrow_mut().push(self.current.borrow().clone());
        self.navigate(prev, false);
    }

    fn go_forward(&self) {
        let Some(next) = self.forward.borrow_mut().pop() else { return };
        self.back.borrow_mut().push(self.current.borrow().clone());
        self.navigate(next, false);
    }

    fn go_up(&self) {
        let parent = self.current.borrow().parent();
        if let Some(parent) = parent {
            self.navigate(parent, true);
        }
    }

    fn update_nav_buttons(&self) {
        self.back_btn.set_sensitive(!self.back.borrow().is_empty());
        self.forward_btn.set_sensitive(!self.forward.borrow().is_empty());
        self.up_btn.set_sensitive(self.current.borrow().parent().is_some());
    }

    fn update_crumbs(&self, file: &gio::File) {
        while let Some(c) = self.crumbs.first_child() {
            self.crumbs.remove(&c);
        }
        let mut chain = vec![file.clone()];
        while let Some(p) = chain.last().unwrap().parent() {
            chain.push(p);
        }
        chain.reverse();
        // Raccourcit le chemin sous le dossier personnel : « Accueil › Documents ».
        let home = gio::File::for_path(glib::home_dir());
        if let Some(pos) = chain.iter().position(|f| f.equal(&home)) {
            chain.drain(..pos);
        }
        // Chemin long : les premiers dossiers passent derrière un bouton « … » (menu),
        // les 4 derniers restent affichés (comme l'Explorateur de Windows).
        const VISIBLE: usize = 4;
        let label_of = |f: &gio::File| {
            if f.equal(&home) {
                "Accueil".to_string()
            } else if f.uri() == "trash:///" {
                "Corbeille".to_string()
            } else {
                f.basename().map(|b| b.to_string_lossy().to_string()).filter(|s| s != "/").unwrap_or_else(|| "Ce PC".into())
            }
        };
        if chain.len() > VISIBLE {
            let hidden: Vec<gio::File> = chain.drain(..chain.len() - VISIBLE).collect();
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-horizontal-symbolic");
            more.add_css_class("flat");
            more.set_tooltip_text(Some("Dossiers parents"));
            let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let pop = gtk::Popover::new();
            for f in hidden.iter().rev() {
                let b = gtk::Button::with_label(&label_of(f));
                b.add_css_class("flat");
                b.set_halign(gtk::Align::Fill);
                if let Some(child) = b.child().and_downcast::<gtk::Label>() {
                    child.set_xalign(0.0);
                }
                let (target, ex_weak, pop) = (f.clone(), self.weak_self(), pop.clone());
                b.connect_clicked(move |_| {
                    pop.popdown();
                    if let Some(ex) = ex_weak.upgrade() {
                        ex.navigate(target.clone(), true);
                    }
                });
                list.append(&b);
            }
            pop.set_child(Some(&list));
            more.set_popover(Some(&pop));
            self.crumbs.append(&more);
            let sep = gtk::Image::from_icon_name("go-next-symbolic");
            sep.add_css_class("dim-label");
            self.crumbs.append(&sep);
        }
        for (i, f) in chain.iter().enumerate() {
            let label = if f.equal(&home) {
                "Accueil".to_string()
            } else if f.uri() == "trash:///" {
                "Corbeille".to_string()
            } else {
                f.basename().map(|b| b.to_string_lossy().to_string()).filter(|s| s != "/").unwrap_or_else(|| "Ce PC".into())
            };
            let b = gtk::Button::with_label(&label);
            b.add_css_class("flat");
            if i == chain.len() - 1 {
                b.add_css_class("heading");
            }
            let target = f.clone();
            let ex_weak = self.weak_self();
            b.connect_clicked(move |_| {
                if let Some(ex) = ex_weak.upgrade() {
                    ex.navigate(target.clone(), true);
                }
            });
            self.crumbs.append(&b);
            if i + 1 < chain.len() {
                let sep = gtk::Image::from_icon_name("go-next-symbolic");
                sep.add_css_class("dim-label");
                self.crumbs.append(&sep);
            }
        }
        let full = file.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| file.uri().to_string());
        self.location.set_text(&full);
        let name = chain.last().map(|f| {
            if f.equal(&home) { "Accueil".to_string() } else { f.basename().map(|b| b.to_string_lossy().to_string()).filter(|s| s != "/").unwrap_or_else(|| "Ce PC".into()) }
        });
        self.folder_title.set_title(&name.unwrap_or_default());
        self.crumbs.set_tooltip_text(Some(&full));
        // Après la mise en page : aller au bout du fil d'Ariane (dossier courant visible).
        if let Some(adj) = self.crumbs.parent().and_downcast::<gtk::Viewport>().and_then(|v| v.hadjustment()) {
            for delay in [30u64, 150] {
                let adj = adj.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
                    adj.set_value((adj.upper() - adj.page_size()).max(0.0));
                });
            }
        }
    }

    fn weak_self(&self) -> std::rc::Weak<Explorer> {
        EXPLORER.with(|e| e.borrow().as_ref().map(Rc::downgrade).unwrap_or_default())
    }

    fn edit_location(&self) {
        self.location_stack.set_visible_child_name("entry");
        self.location.grab_focus();
        self.location.select_region(0, -1);
    }

    fn toggle_hidden(&self) {
        let v = !*self.show_hidden.borrow();
        *self.show_hidden.borrow_mut() = v;
        self.filter.changed(gtk::FilterChange::Different);
        self.toast(if v { "Fichiers cachés affichés" } else { "Fichiers cachés masqués" });
    }

    fn update_status(&self) {
        let n = self.selection.n_items();
        let selected = self.selection.selection().size();
        let text = match (n, selected) {
            (0, _) => "Dossier vide".to_string(),
            (n, 0) => format!("{n} élément{}", if n > 1 { "s" } else { "" }),
            (n, s) => {
                let size: i64 = self.selected_infos().iter().filter(|i| !is_dir(i)).map(|i| i.size()).sum();
                format!("{n} éléments — {s} sélectionné{} ({})", if s > 1 { "s" } else { "" }, human_size(size))
            }
        };
        self.status.set_text(&text);
    }

    // ───────────────────────── Actions sur les fichiers ─────────────────────────

    fn selected_infos(&self) -> Vec<gio::FileInfo> {
        let bitset = self.selection.selection();
        (0..bitset.size())
            .filter_map(|i| self.selection.item(bitset.nth(i as u32)))
            .filter_map(|o| o.downcast::<gio::FileInfo>().ok())
            .collect()
    }

    fn selected_files(&self) -> Vec<gio::File> {
        self.selected_infos().iter().filter_map(|i| file_of(i.upcast_ref())).collect()
    }

    fn open_position(&self, pos: u32) {
        let Some(obj) = self.selection.item(pos) else { return };
        let info = obj.downcast_ref::<gio::FileInfo>().unwrap();
        let Some(file) = file_of(&obj) else { return };
        if is_dir(info) {
            self.navigate(file, true);
        } else {
            self.open_file(&file);
        }
    }

    fn open_file(&self, file: &gio::File) {
        let ctx = gdk::Display::default().map(|d| d.app_launch_context());
        if let Err(e) = gio::AppInfo::launch_default_for_uri(&file.uri(), ctx.as_ref()) {
            self.toast(&format!("Impossible d'ouvrir : {e}"));
        }
    }

    fn open_selected(&self) {
        let infos = self.selected_infos();
        if let [info] = infos.as_slice() {
            if let Some(f) = file_of(info.upcast_ref()) {
                if is_dir(info) {
                    self.navigate(f, true);
                    return;
                }
            }
        }
        for f in self.selected_files() {
            self.open_file(&f);
        }
    }

    fn new_folder(&self) {
        let dir = self.current.borrow().clone();
        let mut name = "Nouveau dossier".to_string();
        let mut i = 2;
        while dir.child(&name).query_exists(gio::Cancellable::NONE) {
            name = format!("Nouveau dossier ({i})");
            i += 1;
        }
        if let Err(e) = dir.child(&name).make_directory(gio::Cancellable::NONE) {
            self.toast(&format!("Création impossible : {e}"));
        }
    }

    fn trash_selected(&self) {
        let files = self.selected_files();
        if files.is_empty() {
            return;
        }
        let mut ok = 0;
        for f in &files {
            match f.trash(gio::Cancellable::NONE) {
                Ok(()) => ok += 1,
                Err(e) => self.toast(&format!("Mise à la corbeille impossible : {e}")),
            }
        }
        if ok > 0 {
            self.toast(&format!("{ok} élément{} mis à la corbeille", if ok > 1 { "s" } else { "" }));
        }
    }

    fn rename_selected(&self) {
        let files = self.selected_files();
        let [file] = files.as_slice() else { return };
        let old = file.basename().map(|b| b.to_string_lossy().to_string()).unwrap_or_default();
        let entry = gtk::Entry::new();
        entry.set_text(&old);
        entry.set_activates_default(true);
        let dialog = adap::AlertDialog::new(Some("Renommer"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Annuler"), ("rename", "Renommer")]);
        dialog.set_response_appearance("rename", adap::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        let file = file.clone();
        let toasts = self.toasts.clone();
        let old_name = old.clone();
        dialog.connect_response(None, move |_, resp| {
            if resp != "rename" {
                return;
            }
            let new = entry.text().to_string();
            if new.is_empty() || new == old_name {
                return;
            }
            if let Err(e) = file.set_display_name(&new, gio::Cancellable::NONE) {
                toasts.add_toast(adap::Toast::new(&format!("Renommage impossible : {e}")));
            }
        });
        dialog.present(Some(&self.window));
        // Sélectionne le nom sans l'extension, comme Windows.
        let stem = old.rfind('.').filter(|&i| i > 0).unwrap_or(old.len()) as i32;
        let e2 = dialog.extra_child().and_downcast::<gtk::Entry>().unwrap();
        glib::idle_add_local_once(move || {
            e2.grab_focus();
            e2.select_region(0, stem);
        });
    }

    fn clip(&self, op: ClipOp) {
        let files = self.selected_files();
        if files.is_empty() {
            return;
        }
        // Presse-papiers système (liste d'URI) + mémoire interne pour l'opération.
        if let Some(display) = gdk::Display::default() {
            let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
            display.clipboard().set_text(&uris.join("\n"));
        }
        let n = files.len();
        *self.clipboard.borrow_mut() = Some((op, files));
        self.toast(&format!(
            "{n} élément{} {}",
            if n > 1 { "s" } else { "" },
            if op == ClipOp::Copy { "copié(s)" } else { "coupé(s)" }
        ));
    }

    fn paste(&self) {
        let Some((op, files)) = self.clipboard.borrow().clone() else { return };
        let dest_dir = self.current.borrow().clone();
        let toasts = self.toasts.clone();
        if op == ClipOp::Cut {
            *self.clipboard.borrow_mut() = None;
        }
        glib::spawn_future_local(async move {
            for src in files {
                let Some(name) = src.basename() else { continue };
                let mut dest = dest_dir.child(&name);
                if dest.query_exists(gio::Cancellable::NONE) {
                    let stem = name.to_string_lossy().to_string();
                    let mut i = 2;
                    while dest.query_exists(gio::Cancellable::NONE) {
                        dest = dest_dir.child(format!("{stem} ({i})"));
                        i += 1;
                    }
                }
                let result = if op == ClipOp::Cut {
                    src.move_(&dest, gio::FileCopyFlags::NONE, gio::Cancellable::NONE, None)
                } else if src.query_file_type(gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                    == gio::FileType::Directory
                {
                    copy_dir_recursive(&src, &dest).await
                } else {
                    src.copy_future(&dest, gio::FileCopyFlags::NONE, glib::Priority::DEFAULT).0.await
                };
                if let Err(e) = result {
                    toasts.add_toast(adap::Toast::new(&format!("Échec pour {} : {e}", name.to_string_lossy())));
                }
            }
        });
    }

    fn copy_path(&self) {
        let paths: Vec<String> = self
            .selected_files()
            .iter()
            .map(|f| f.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| f.uri().to_string()))
            .collect();
        let text = if paths.is_empty() {
            self.current.borrow().path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
        } else {
            paths.join("\n")
        };
        if let Some(d) = gdk::Display::default() {
            d.clipboard().set_text(&text);
        }
        self.toast("Chemin copié");
    }

    fn open_terminal(&self) {
        let Some(dir) = self.current.borrow().path() else { return };
        for term in ["ptyxis", "kgx", "gnome-terminal", "foot", "konsole", "xterm"] {
            if std::process::Command::new(term).current_dir(&dir).spawn().is_ok() {
                return;
            }
        }
        self.toast("Aucun terminal trouvé");
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    fn context_menu(&self, parent: &gtk::Widget, x: f64, y: f64) {
        let menu = gio::Menu::new();
        let has_sel = !self.selection.selection().is_empty();
        if has_sel {
            let s1 = gio::Menu::new();
            s1.append(Some("Ouvrir"), Some("win.open"));
            menu.append_section(None, &s1);
            let s2 = gio::Menu::new();
            s2.append(Some("Couper"), Some("win.cut"));
            s2.append(Some("Copier"), Some("win.copy"));
            s2.append(Some("Renommer"), Some("win.rename"));
            s2.append(Some("Copier le chemin"), Some("win.copy-path"));
            menu.append_section(None, &s2);
            let s3 = gio::Menu::new();
            s3.append(Some("Mettre à la corbeille"), Some("win.trash"));
            menu.append_section(None, &s3);
        } else {
            let s1 = gio::Menu::new();
            s1.append(Some("Nouveau dossier"), Some("win.new-folder"));
            s1.append(Some("Coller"), Some("win.paste"));
            menu.append_section(None, &s1);
            let s2 = gio::Menu::new();
            s2.append(Some("Ouvrir dans le terminal"), Some("win.terminal"));
            s2.append(Some("Afficher / masquer les fichiers cachés"), Some("win.hidden"));
            s2.append(Some("Actualiser"), Some("win.refresh"));
            menu.append_section(None, &s2);
        }
        // Le menu est rattaché au contenu de la fenêtre, pas à la vue : en CSS, il serait sinon
        // un descendant de la grille (gridview, .view) et hériterait de règles qui faussent la
        // mesure de sa hauteur (dernière entrée coupée, menu à faire défiler).
        let host = self.window.content().unwrap_or_else(|| parent.clone());
        let (hx, hy) = parent
            .compute_point(&host, &gtk::graphene::Point::new(x as f32, y as f32))
            .map(|p| (p.x() as i32, p.y() as i32))
            .unwrap_or((x as i32, y as i32));
        let pop = gtk::PopoverMenu::from_model(Some(&menu));
        pop.set_parent(&host);
        pop.set_has_arrow(false);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(hx, hy, 1, 1)));
        pop.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        noos_theme::fit_popover(&pop);
        pop.popup();
    }
}

async fn copy_dir_recursive(src: &gio::File, dest: &gio::File) -> Result<(), glib::Error> {
    dest.make_directory(gio::Cancellable::NONE)?;
    let children = src.enumerate_children("standard::name,standard::type", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)?;
    for info in children {
        let info = info?;
        let (s, d) = (src.child(info.name()), dest.child(info.name()));
        if info.file_type() == gio::FileType::Directory {
            Box::pin(copy_dir_recursive(&s, &d)).await?;
        } else {
            s.copy_future(&d, gio::FileCopyFlags::NOFOLLOW_SYMLINKS, glib::Priority::DEFAULT).0.await?;
        }
    }
    Ok(())
}

fn shellexpand_home(text: &str) -> std::path::PathBuf {
    match text.strip_prefix('~') {
        Some(rest) => glib::home_dir().join(rest.trim_start_matches('/')),
        None => std::path::PathBuf::from(text),
    }
}

fn file_icon(info: &gio::FileInfo, image: &gtk::Image) {
    if let Some(thumb) = info.attribute_byte_string("thumbnail::path") {
        image.set_from_file(Some(thumb.as_str()));
    } else if let Some(icon) = info.icon() {
        image.set_from_gicon(&icon);
    } else {
        image.set_icon_name(Some("text-x-generic"));
    }
}

/// Le point (x, y) de la vue est-il sur un élément (case de grille ou ligne de liste) ?
fn on_item(view: &gtk::Widget, x: f64, y: f64) -> bool {
    let mut w = view.pick(x, y, gtk::PickFlags::DEFAULT);
    while let Some(cur) = w {
        if &cur == view {
            return false;
        }
        if matches!(cur.css_name().as_str(), "child" | "row" | "cell") {
            return true;
        }
        w = cur.parent();
    }
    false
}

fn build_grid(selection: &gtk::MultiSelection) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        // Tuile de taille fixe, centrée dans la case : c'est elle qui porte le surlignage de
        // sélection (la case, elle, s'étire sur toute la largeur de la colonne).
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        v.add_css_class("tile");
        v.set_halign(gtk::Align::Center);
        v.set_valign(gtk::Align::Start);
        v.set_size_request(92, -1);
        let image = gtk::Image::new();
        image.set_pixel_size(64);
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(14);
        label.set_justify(gtk::Justification::Center);
        v.append(&image);
        v.append(&label);
        item.set_child(Some(&v));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let info = item.item().and_downcast::<gio::FileInfo>().unwrap();
        let v = item.child().and_downcast::<gtk::Box>().unwrap();
        let image = v.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = v.last_child().and_downcast::<gtk::Label>().unwrap();
        file_icon(&info, &image);
        label.set_text(&info.display_name());
        v.set_tooltip_text(Some(&info.display_name()));
    });
    let grid = gtk::GridView::new(Some(selection.clone()), Some(factory));
    grid.set_max_columns(14);
    grid.set_enable_rubberband(true);
    grid
}

fn build_details(selection: &gtk::MultiSelection) -> gtk::ColumnView {
    let view = gtk::ColumnView::new(Some(selection.clone()));
    view.set_show_column_separators(false);
    view.set_enable_rubberband(true);
    view.add_css_class("data-table");

    let column = |title: &str, expand: bool, bind: fn(&gio::FileInfo) -> String, with_icon: bool| {
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if with_icon {
                let img = gtk::Image::new();
                img.set_pixel_size(16);
                h.append(&img);
            }
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&l);
            item.set_child(Some(&h));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let info = item.item().and_downcast::<gio::FileInfo>().unwrap();
            let h = item.child().and_downcast::<gtk::Box>().unwrap();
            if with_icon {
                let img = h.first_child().and_downcast::<gtk::Image>().unwrap();
                file_icon(&info, &img);
            }
            let l = h.last_child().and_downcast::<gtk::Label>().unwrap();
            l.set_text(&bind(&info));
        });
        let col = gtk::ColumnViewColumn::new(Some(title), Some(factory));
        col.set_expand(expand);
        col.set_resizable(true);
        col
    };

    view.append_column(&column("Nom", true, |i| i.display_name().to_string(), true));
    view.append_column(&column(
        "Modifié le",
        false,
        |i| {
            i.modification_date_time()
                .and_then(|d| d.to_local().ok())
                .and_then(|d| d.format("%d/%m/%Y %H:%M").ok())
                .map(|s| s.to_string())
                .unwrap_or_default()
        },
        false,
    ));
    view.append_column(&column(
        "Type",
        false,
        |i| {
            if is_dir(i) {
                "Dossier de fichiers".into()
            } else {
                i.content_type().map(|t| gio::content_type_get_description(&t).to_string()).unwrap_or_default()
            }
        },
        false,
    ));
    view.append_column(&column("Taille", false, |i| if is_dir(i) { String::new() } else { human_size(i.size()) }, false));
    view
}

thread_local! {
    static EXPLORER: RefCell<Option<Rc<Explorer>>> = const { RefCell::new(None) };
}

/// Barre d'adresse : un champ arrondi qui occupe toute la largeur disponible.
const CSS: &str = r#"
.nav-row { padding: 4px 10px 8px 10px; }
.address-bar {
  background-color: alpha(@window_fg_color, 0.06);
  border: 1px solid alpha(@window_fg_color, 0.10);
  border-radius: 8px;
  min-height: 32px;
  padding: 0 4px;
}
.address-bar:hover { background-color: alpha(@window_fg_color, 0.09); }
.address-bar button { min-height: 26px; padding: 0 8px; border-radius: 6px; }
.address-bar entry.address-entry { background: none; border: none; box-shadow: none; outline: none; min-height: 30px; }
/* Vue en icônes : surlignage ajusté à l'icône et au nom, pas à toute la case. */
gridview > child { padding: 2px; margin: 0; background: none; box-shadow: none; outline: none; }
gridview > child:hover, gridview > child:selected, gridview > child:selected:hover { background: none; }
gridview .tile { padding: 6px 4px; border-radius: 8px; }
gridview > child:hover .tile { background-color: alpha(@window_fg_color, 0.06); }
gridview > child:selected .tile { background-color: alpha(@accent_bg_color, 0.22); box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.45); }
gridview > child:selected:hover .tile { background-color: alpha(@accent_bg_color, 0.30); }
gridview > child:focus-visible .tile { outline: 2px solid alpha(@accent_color, 0.6); outline-offset: -2px; }
"#;

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(|_| {
        let theming = noos_theme::Theming::init();
        std::mem::forget(theming);
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        let ex = EXPLORER.with(|e| e.borrow().clone());
        match ex {
            Some(ex) => ex.window.present(),
            None => {
                let ex = Explorer::new(app);
                EXPLORER.with(|e| *e.borrow_mut() = Some(ex.clone()));
                ex.window.present();
            }
        }
    });
    app.connect_open(|app, files, _| {
        app.activate();
        if let (Some(ex), Some(f)) = (EXPLORER.with(|e| e.borrow().clone()), files.first()) {
            ex.navigate(f.clone(), true);
        }
    });
    app.run()
}

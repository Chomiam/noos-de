//! noos-archive — « Archives » de NOOS (GTK4 + libAdapta).
//!
//! Parcourir, extraire, créer, tester, modifier et convertir des archives de plus de 60
//! formats (moteurs : 7-Zip, libarchive, unar ; images ISO : xorriso). Voir `engine.rs`.
//!
//! Ligne de commande (menu contextuel de l'Explorateur) :
//!   noos-archive [ARCHIVE]                  ouvrir
//!   noos-archive --extract-here FICHIERS…   extraire à côté (sous-dossier si besoin)
//!   noos-archive --extract-to FICHIERS…     extraire en choisissant la destination
//!   noos-archive --compress FICHIERS…       page de création préremplie
//!   noos-archive --quick zip|7z FICHIERS…   compresser aussitôt, à côté des fichiers
//!
//! Tests : `NOOS_ARCHIVE_DEMO=open|create` (archive / fichiers d'exemple).

mod create;
mod engine;
mod jobs;

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

use engine::{Archive, Entry, OpenError, OutFormat};
use jobs::human;

const APP_ID: &str = "org.noos.Archive";

const CSS: &str = "
.drop-zone { border: 2px dashed alpha(currentColor, 0.25); border-radius: 18px; padding: 42px 30px; }
.drop-zone.hover { border-color: @accent_bg_color; background: alpha(@accent_bg_color, 0.08); }
.fmt-chip { font-family: monospace; font-size: 0.82em; padding: 2px 8px; border-radius: 6px; background: alpha(currentColor, 0.07); }
togglebutton.fmt-tile { padding: 10px 12px; border-radius: 10px; min-height: 62px; }
togglebutton.fmt-tile:checked { background: alpha(@accent_bg_color, 0.22); box-shadow: inset 0 0 0 2px @accent_bg_color; color: inherit; }
togglebutton.fmt-tile .fmt-ext { font-family: monospace; font-weight: 800; font-size: 1.1em; }
.arc-status { padding: 6px 14px; border-top: 1px solid alpha(currentColor, 0.08); font-size: 0.9em; }
.arc-path { font-weight: 700; }
.ratio { min-width: 46px; }
.ratio trough, .ratio progress { min-height: 5px; border-radius: 3px; }
columnview.arc-list row cell { padding-top: 5px; padding-bottom: 5px; }
.lock-badge { color: @warning_color; }
";

struct Recent;
impl Recent {
    fn path() -> PathBuf {
        noos_core::paths::config_dir().join("archives-recent.txt")
    }
    fn list() -> Vec<PathBuf> {
        std::fs::read_to_string(Self::path()).unwrap_or_default().lines().map(PathBuf::from).filter(|p| p.exists()).take(8).collect()
    }
    fn add(p: &Path) {
        let mut v = Self::list();
        v.retain(|x| x != p);
        v.insert(0, p.to_path_buf());
        let _ = std::fs::create_dir_all(noos_core::paths::config_dir());
        let _ = std::fs::write(Self::path(), v.iter().take(8).map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join("\n"));
    }
}

struct App {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    stack: gtk::Stack,
    title: adap::WindowTitle,
    back: gtk::Button,
    extract_btn: gtk::Button,
    add_btn: gtk::Button,
    menu_btn: gtk::MenuButton,
    search: gtk::SearchEntry,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    status: gtk::Label,
    recent: gtk::ListBox,
    create: RefCell<Option<Rc<create::CreatePage>>>,
    archive: RefCell<Option<Archive>>,
    dir: RefCell<String>,
}

fn entry_of(o: &glib::Object) -> Entry {
    o.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<Entry>().clone()).unwrap_or_default()
}

fn icon_for(e: &Entry) -> gio::Icon {
    if e.is_dir {
        return gio::ThemedIcon::new("folder").upcast();
    }
    let (ct, _) = gio::content_type_guess(Some(e.name()), None);
    gio::content_type_get_icon(&ct)
}

impl App {
    fn new(gapp: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder().application(gapp).title("Archives").default_width(1000).default_height(700).build();
        let header = adap::HeaderBar::new();
        let title = adap::WindowTitle::new("Archives", "");
        header.set_title_widget(Some(&title));
        let back = gtk::Button::from_icon_name("go-up-symbolic");
        back.set_tooltip_text(Some("Dossier parent (Retour arrière)"));
        header.pack_start(&back);
        let open = gtk::Button::from_icon_name("document-open-symbolic");
        open.set_tooltip_text(Some("Ouvrir une archive (Ctrl+O)"));
        open.set_action_name(Some("win.open"));
        header.pack_start(&open);
        let new = gtk::Button::from_icon_name("list-add-symbolic");
        new.set_tooltip_text(Some("Créer une archive (Ctrl+N)"));
        new.set_action_name(Some("win.new"));
        header.pack_start(&new);
        let menu = gio::Menu::new();
        menu.append(Some("Tester l'intégrité"), Some("win.test"));
        menu.append(Some("Convertir vers un autre format…"), Some("win.convert"));
        menu.append(Some("Monter l'image disque"), Some("win.mount"));
        menu.append(Some("Ouvrir le dossier contenant"), Some("win.reveal"));
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&menu));
        header.pack_end(&menu_btn);
        let extract_btn = gtk::Button::new();
        extract_btn.set_child(Some(&adap::ButtonContent::builder().icon_name("folder-download-symbolic").label("Extraire").build()));
        extract_btn.add_css_class("suggested-action");
        extract_btn.set_action_name(Some("win.extract"));
        header.pack_end(&extract_btn);
        let add_btn = gtk::Button::from_icon_name("list-add-symbolic");
        add_btn.set_tooltip_text(Some("Ajouter des fichiers à l'archive"));
        add_btn.set_action_name(Some("win.add"));
        header.pack_end(&add_btn);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher dans l'archive"));
        search.set_width_chars(24);
        header.pack_end(&search);

        // ── Accueil ──
        let welcome = gtk::Box::new(gtk::Orientation::Vertical, 20);
        welcome.set_valign(gtk::Align::Center);
        welcome.set_halign(gtk::Align::Center);
        welcome.set_margin_top(24);
        welcome.set_margin_bottom(24);
        let zone = gtk::Box::new(gtk::Orientation::Vertical, 12);
        zone.add_css_class("drop-zone");
        let icon = gtk::Image::from_icon_name("package-x-generic");
        icon.set_pixel_size(88);
        zone.append(&icon);
        let t = gtk::Label::new(Some("Archives"));
        t.add_css_class("title-1");
        zone.append(&t);
        let s = gtk::Label::new(Some("Déposez une archive pour l'ouvrir, ou des fichiers pour en créer une"));
        s.add_css_class("dim-label");
        zone.append(&s);
        let btns = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        btns.set_halign(gtk::Align::Center);
        btns.set_margin_top(8);
        for (label, action, cls) in [("Ouvrir une archive", "win.open", "suggested-action"), ("Créer une archive", "win.new", "")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("pill");
            if !cls.is_empty() {
                b.add_css_class(cls);
            }
            b.set_action_name(Some(action));
            btns.append(&b);
        }
        zone.append(&btns);
        welcome.append(&zone);
        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_max_children_per_line(10);
        chips.set_min_children_per_line(10);
        chips.set_halign(gtk::Align::Center);
        chips.set_size_request(640, -1);
        for f in ["7z", "zip", "rar", "tar.gz", "tar.xz", "tar.zst", "tar.bz2", "iso", "dmg", "wim", "cab", "msi", "deb", "rpm", "jar", "apk", "epub", "cbz", "cbr", "squashfs", "vhd", "vmdk", "qcow2", "arj", "lzh", "cpio", "sit", "alz", "xar", "ar"] {
            let l = gtk::Label::new(Some(f));
            l.add_css_class("fmt-chip");
            chips.append(&l);
        }
        welcome.append(&chips);
        let more = gtk::Label::new(Some("et une quarantaine d'autres formats en lecture (7-Zip, libarchive, unar)"));
        more.add_css_class("dim-label");
        more.add_css_class("caption");
        welcome.append(&more);
        let recent = gtk::ListBox::new();
        recent.add_css_class("boxed-list");
        recent.set_selection_mode(gtk::SelectionMode::None);
        recent.set_size_request(520, -1);
        recent.set_halign(gtk::Align::Center);
        welcome.append(&recent);
        let welcome_scroll = gtk::ScrolledWindow::builder().child(&welcome).hscrollbar_policy(gtk::PolicyType::Never).build();

        // ── Navigateur ──
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let filter = gtk::CustomFilter::new(|_| true);
        let filtered = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let sorted = gtk::SortListModel::new(Some(filtered), None::<gtk::Sorter>);
        let selection = gtk::MultiSelection::new(Some(sorted.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.add_css_class("arc-list");
        view.set_show_row_separators(false);
        sorted.set_sorter(view.sorter().as_ref());
        let col = |title: &str, expand: bool, bind: fn(&gtk::Box, &Entry), cmp: Option<fn(&Entry, &Entry) -> std::cmp::Ordering>| {
            let f = gtk::SignalListItemFactory::new();
            f.connect_setup(|_, li| {
                let li = li.downcast_ref::<gtk::ListItem>().unwrap();
                let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                li.set_child(Some(&b));
            });
            f.connect_bind(move |_, li| {
                let li = li.downcast_ref::<gtk::ListItem>().unwrap();
                let b = li.child().and_downcast::<gtk::Box>().unwrap();
                while let Some(c) = b.first_child() {
                    b.remove(&c);
                }
                if let Some(o) = li.item() {
                    bind(&b, &entry_of(&o));
                }
            });
            let c = gtk::ColumnViewColumn::new(Some(title), Some(f));
            c.set_expand(expand);
            c.set_resizable(true);
            if let Some(cmp) = cmp {
                c.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
                    let (a, b) = (entry_of(a), entry_of(b));
                    // Dossiers toujours en premier.
                    b.is_dir.cmp(&a.is_dir).then_with(|| cmp(&a, &b)).into()
                })));
            }
            c
        };
        let name_col = col(
            "Nom",
            true,
            |b, e| {
                let i = gtk::Image::from_gicon(&icon_for(e));
                i.set_pixel_size(22);
                b.append(&i);
                let l = gtk::Label::new(Some(e.name()));
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                b.append(&l);
                if e.encrypted {
                    let lock = gtk::Image::from_icon_name("channel-secure-symbolic");
                    lock.add_css_class("lock-badge");
                    lock.set_tooltip_text(Some("Chiffré"));
                    b.append(&lock);
                }
            },
            Some(|a, b| a.name().to_lowercase().cmp(&b.name().to_lowercase())),
        );
        view.append_column(&name_col);
        view.append_column(&col(
            "Taille",
            false,
            |b, e| {
                let l = gtk::Label::new(Some(&human(e.size)));
                l.add_css_class("numeric");
                b.append(&l);
            },
            Some(|a, b| a.size.cmp(&b.size)),
        ));
        view.append_column(&col(
            "Compression",
            false,
            |b, e| {
                if let Some(p) = e.packed.filter(|_| e.size > 0 && !e.is_dir) {
                    let ratio = (p as f64 / e.size as f64).min(1.0);
                    let bar = gtk::LevelBar::for_interval(0.0, 1.0);
                    bar.set_value(1.0 - ratio);
                    bar.add_css_class("ratio");
                    bar.set_valign(gtk::Align::Center);
                    bar.set_tooltip_text(Some(&format!("{} compressé ({} gagnés)", human(p), human(e.size.saturating_sub(p)))));
                    b.append(&bar);
                    let l = gtk::Label::new(Some(&format!("−{:.0} %", (1.0 - ratio) * 100.0)));
                    l.add_css_class("dim-label");
                    l.add_css_class("caption");
                    b.append(&l);
                }
            },
            Some(|a, b| {
                let r = |e: &Entry| e.packed.map(|p| p as f64 / e.size.max(1) as f64).unwrap_or(1.0);
                r(a).total_cmp(&r(b))
            }),
        ));
        view.append_column(&col(
            "Modifié",
            false,
            |b, e| {
                let l = gtk::Label::new(Some(&e.modified));
                l.add_css_class("dim-label");
                b.append(&l);
            },
            Some(|a, b| a.modified.cmp(&b.modified)),
        ));
        view.sort_by_column(Some(&name_col), gtk::SortType::Ascending);
        let list_scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();
        let status = gtk::Label::new(None);
        status.add_css_class("arc-status");
        status.set_xalign(0.0);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let browser = gtk::Box::new(gtk::Orientation::Vertical, 0);
        browser.append(&list_scroll);
        browser.append(&status);

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&welcome_scroll, Some("welcome"));
        stack.add_named(&browser, Some("browser"));
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&toasts));
        window.set_content(Some(&tv));

        let app = Rc::new(Self {
            window,
            toasts,
            stack,
            title,
            back,
            extract_btn,
            add_btn,
            menu_btn,
            search,
            store,
            selection,
            status,
            recent,
            create: RefCell::new(None),
            archive: RefCell::new(None),
            dir: RefCell::new(String::new()),
        });

        // Recherche : filtre sur le nom (dans tout le dossier courant).
        {
            let app2 = Rc::downgrade(&app);
            let filter2 = filter.clone();
            app.search.connect_search_changed(move |s| {
                let q = s.text().to_lowercase();
                filter2.set_filter_func(move |o| q.is_empty() || entry_of(o).name().to_lowercase().contains(&q));
                if let Some(a) = app2.upgrade() {
                    a.update_status();
                }
            });
        }
        {
            let app2 = Rc::downgrade(&app);
            view.connect_activate(move |_, pos| {
                let Some(a) = app2.upgrade() else { return };
                let Some(o) = a.selection.item(pos) else { return };
                let e = entry_of(&o);
                if e.is_dir {
                    a.enter(&e.path);
                } else {
                    a.open_entry(&e);
                }
            });
        }
        {
            let app2 = Rc::downgrade(&app);
            app.back.connect_clicked(move |_| {
                if let Some(a) = app2.upgrade() {
                    a.go_up();
                }
            });
        }
        // Clic droit : actions sur la sélection.
        {
            let rc = gtk::GestureClick::new();
            rc.set_button(gdk::BUTTON_SECONDARY);
            let app2 = Rc::downgrade(&app);
            let v2 = view.clone();
            rc.connect_pressed(move |_, _, x, y| {
                let Some(a) = app2.upgrade() else { return };
                let m = gio::Menu::new();
                m.append(Some("Ouvrir"), Some("win.open-selected"));
                m.append(Some("Extraire la sélection…"), Some("win.extract-selected"));
                if a.archive.borrow().as_ref().is_some_and(Archive::writable) {
                    m.append(Some("Supprimer de l'archive"), Some("win.delete"));
                }
                let pop = gtk::PopoverMenu::from_model(Some(&m));
                pop.set_parent(&v2);
                pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.set_has_arrow(false);
                pop.connect_closed(|p| {
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.unparent());
                });
                noos_theme::fit_popover(&pop);
                pop.popup();
            });
            view.add_controller(rc);
        }
        // Glisser-déposer : une archive l'ouvre, des fichiers créent une archive (ou s'ajoutent
        // à l'archive ouverte si elle est modifiable).
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        {
            let z = zone.clone();
            drop.connect_enter(move |_, _, _| {
                z.add_css_class("hover");
                gdk::DragAction::COPY
            });
            let z = zone.clone();
            drop.connect_leave(move |_| z.remove_css_class("hover"));
            let app2 = Rc::downgrade(&app);
            drop.connect_drop(move |_, v, _, _| {
                zone.remove_css_class("hover");
                let (Some(a), Ok(list)) = (app2.upgrade(), v.get::<gdk::FileList>()) else { return false };
                let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
                a.dropped(paths);
                true
            });
        }
        app.window.add_controller(drop);
        app.setup_actions();
        app.refresh_recent();
        app.update_chrome();
        app
    }

    fn toast(&self, t: &str) {
        self.toasts.add_toast(adap::Toast::new(t));
    }

    fn error(&self, title: &str, body: &str) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_response("ok", "Fermer");
        d.present(Some(&self.window));
    }

    fn update_chrome(&self) {
        let page = self.stack.visible_child_name().unwrap_or_default();
        let browsing = page == "browser";
        let a = self.archive.borrow();
        self.back.set_visible(browsing);
        self.back.set_sensitive(!self.dir.borrow().is_empty());
        self.extract_btn.set_visible(browsing);
        self.search.set_visible(browsing);
        self.menu_btn.set_visible(browsing);
        self.add_btn.set_visible(browsing && a.as_ref().is_some_and(Archive::writable));
        match (&*a, page.as_str()) {
            (Some(a), "browser") => {
                self.title.set_title(&a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                let d = self.dir.borrow();
                self.title.set_subtitle(&if d.is_empty() { format!("{} · racine", a.format) } else { format!("{} · /{}", a.format, d) });
            }
            (_, "create") => {
                self.title.set_title("Créer une archive");
                self.title.set_subtitle("");
            }
            _ => {
                self.title.set_title("Archives");
                self.title.set_subtitle("");
            }
        }
        if let Some(w) = self.window.lookup_action("mount").and_downcast::<gio::SimpleAction>() {
            w.set_enabled(a.as_ref().is_some_and(|a| noos_core::disk_image::is_disk_image(&a.path)));
        }
    }

    fn update_status(&self) {
        let a = self.archive.borrow();
        let Some(a) = a.as_ref() else { return };
        let total = a.total_size();
        let packed = a.physical_size;
        let ratio = if total > 0 { 100.0 - packed as f64 * 100.0 / total as f64 } else { 0.0 };
        let mut parts = vec![
            format!("{} fichier{}", a.files(), if a.files() > 1 { "s" } else { "" }),
            format!("{} dossier{}", a.dirs(), if a.dirs() > 1 { "s" } else { "" }),
            format!("{} → {}{}", human(total), human(packed), if ratio > 0.5 { format!(" (−{ratio:.0} %)") } else { String::new() }),
            a.format.clone(),
        ];
        if !a.method.is_empty() {
            parts.push(a.method.clone());
        }
        if a.solid == Some(true) {
            parts.push("solide".into());
        }
        if a.encrypted() {
            parts.push("🔒 chiffrée".into());
        }
        if !a.writable() {
            parts.push("lecture seule".into());
        }
        self.status.set_text(&parts.join("  ·  "));
    }

    fn refresh_recent(self: &Rc<Self>) {
        while let Some(c) = self.recent.first_child() {
            self.recent.remove(&c);
        }
        let list = Recent::list();
        self.recent.set_visible(!list.is_empty());
        for p in list {
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            r.set_subtitle(&p.parent().map(|d| d.display().to_string()).unwrap_or_default());
            r.add_prefix(&gtk::Image::from_icon_name("package-x-generic-symbolic"));
            r.set_activatable(true);
            let a = Rc::downgrade(self);
            r.connect_activated(move |_| {
                if let Some(a) = a.upgrade() {
                    a.open_archive(&p, None);
                }
            });
            self.recent.append(&r);
        }
    }

    // ───────────────────────── Navigation ─────────────────────────

    fn show_dir(&self) {
        let children = match self.archive.borrow().as_ref() {
            Some(a) => a.children(&self.dir.borrow()),
            None => return,
        };
        self.store.remove_all();
        for e in children {
            self.store.append(&glib::BoxedAnyObject::new(e));
        }
        self.update_chrome();
        self.update_status();
    }

    fn enter(&self, dir: &str) {
        *self.dir.borrow_mut() = dir.to_string();
        self.search.set_text("");
        self.show_dir();
    }

    fn go_up(&self) {
        let parent = self.dir.borrow().rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        self.enter(&parent);
    }

    fn selected_paths(&self) -> Vec<String> {
        let sel = self.selection.selection();
        (0..sel.size()).filter_map(|i| self.selection.item(sel.nth(i as u32))).map(|o| entry_of(&o).path).collect()
    }

    /// Ouvre une archive (mot de passe demandé si nécessaire).
    fn open_archive(self: &Rc<Self>, path: &Path, password: Option<String>) {
        let p = path.to_path_buf();
        let pw = password.clone();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(engine::open(&p, pw.as_deref()));
        });
        let (a, path) = (self.clone(), path.to_path_buf());
        glib::spawn_future_local(async move {
            match rx.recv().await.unwrap_or(Err(OpenError::Other("interrompu".into()))) {
                Ok(arc) => {
                    Recent::add(&path);
                    *a.archive.borrow_mut() = Some(arc);
                    *a.dir.borrow_mut() = String::new();
                    a.stack.set_visible_child_name("browser");
                    a.show_dir();
                }
                Err(OpenError::Password) => {
                    let (a2, path2) = (a.clone(), path.clone());
                    jobs::ask_password(&a.window, &path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), password.is_some(), move |pw| {
                        a2.open_archive(&path2, Some(pw));
                    });
                }
                Err(OpenError::Other(e)) => a.error("Impossible d'ouvrir l'archive", &format!("{}\n\n{e}", path.display())),
            }
        });
    }

    /// Double-clic sur un fichier : extrait dans le cache et l'ouvre avec son application.
    fn open_entry(self: &Rc<Self>, e: &Entry) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let dest = glib::user_cache_dir().join("noos-archive").join(format!("{}", std::process::id()));
        let o = engine::ExtractOptions { dest: dest.clone(), only: vec![e.path.clone()], keep_paths: true, overwrite: engine::Overwrite::Replace, password: a.password.clone() };
        let file = dest.join(&e.path);
        let me = self.clone();
        jobs::run(&self.window, &format!("Ouverture de « {} »", e.name()), e.size, move |job, send| engine::extract(job, &a, &o, send), Box::new(move |r| match r {
            Ok(_) => {
                if let Err(err) = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(&file).uri(), None::<&gio::AppLaunchContext>) {
                    me.toast(&format!("Aucune application pour ce fichier : {err}"));
                }
            }
            Err(err) => me.error("Ouverture impossible", &err),
        }));
    }

    fn extract(self: &Rc<Self>, only: Vec<String>) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let me = self.clone();
        let a2 = a.clone();
        jobs::extract_dialog(&self.window, &a, only, move |o, open| me.run_extract(&a2, o, open));
    }

    fn run_extract(self: &Rc<Self>, a: &Archive, o: engine::ExtractOptions, open_after: bool) {
        let a2 = a.clone();
        let me = self.clone();
        let total = a.total_size();
        jobs::run(&self.window, &format!("Extraction de « {} »", a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()), total, move |job, send| engine::extract(job, &a2, &o, send), Box::new(move |r| match r {
            Ok(dest) => {
                let t = adap::Toast::new("Extraction terminée");
                t.set_button_label(Some("Ouvrir le dossier"));
                let d = dest.clone();
                t.connect_button_clicked(move |_| jobs::show_folder(&d));
                me.toasts.add_toast(t);
                if open_after {
                    jobs::show_folder(&dest);
                }
            }
            Err(e) if e.contains("Mot de passe") => me.error("Mot de passe incorrect", "L'archive n'a pas pu être déchiffrée."),
            Err(e) => me.error("Échec de l'extraction", &e),
        }));
    }

    fn show_create(self: &Rc<Self>, files: &[PathBuf]) {
        if self.create.borrow().is_none() {
            let me = Rc::downgrade(self);
            let page = create::CreatePage::new(move |o| {
                if let Some(a) = me.upgrade() {
                    a.run_create(o);
                }
            });
            self.stack.add_named(&page.root, Some("create"));
            *self.create.borrow_mut() = Some(page);
        }
        if let Some(p) = self.create.borrow().as_ref() {
            p.add(files);
        }
        self.stack.set_visible_child_name("create");
        self.update_chrome();
    }

    fn run_create(self: &Rc<Self>, o: engine::CreateOptions) {
        let total: u64 = 0;
        let me = self.clone();
        let name = o.output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if o.output.exists() {
            let d = adap::AlertDialog::new(Some("Remplacer l'archive existante ?"), Some(&format!("« {name} » existe déjà dans ce dossier.")));
            d.add_responses(&[("cancel", "Annuler"), ("ok", "Remplacer")]);
            d.set_response_appearance("ok", adap::ResponseAppearance::Destructive);
            let (me2, o2) = (self.clone(), o.clone());
            d.connect_response(None, move |_, r| {
                if r == "ok" {
                    me2.start_create(o2.clone(), 0);
                }
            });
            d.present(Some(&self.window));
            return;
        }
        me.start_create(o, total);
    }

    fn start_create(self: &Rc<Self>, o: engine::CreateOptions, total: u64) {
        let me = self.clone();
        let name = o.output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        jobs::run(&self.window, &format!("Création de « {name} »"), total, move |job, send| engine::create(job, &o, send), Box::new(move |r| match r {
            Ok(path) => {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let t = adap::Toast::new(&format!("Archive créée ({})", human(size)));
                t.set_button_label(Some("Afficher"));
                let p = path.clone();
                t.connect_button_clicked(move |_| jobs::show_folder(p.parent().unwrap_or(Path::new("/"))));
                me.toasts.add_toast(t);
                me.open_archive(&path, None);
            }
            Err(e) => me.error("Échec de la création", &e),
        }));
    }

    fn dropped(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if let [one] = paths.as_slice() {
            if one.is_file() && engine::is_archive(one) && self.stack.visible_child_name().as_deref() != Some("create") {
                self.open_archive(one, None);
                return;
            }
        }
        let writable_open = self.stack.visible_child_name().as_deref() == Some("browser") && self.archive.borrow().as_ref().is_some_and(Archive::writable);
        if writable_open {
            self.add_files(paths);
        } else {
            self.show_create(&paths);
        }
    }

    fn add_files(self: &Rc<Self>, files: Vec<PathBuf>) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let into = self.dir.borrow().clone();
        let me = self.clone();
        let path = a.path.clone();
        let pw = a.password.clone();
        let dir = into.clone();
        jobs::run(&self.window, "Ajout à l'archive", 0, move |job, send| engine::add(job, &a, &files, &into, send).map(|_| a.path.clone()), Box::new(move |r| match r {
            Ok(_) => {
                me.toast("Fichiers ajoutés");
                me.reload(&path, pw, &dir);
            }
            Err(e) => me.error("Ajout impossible", &e),
        }));
    }

    fn reload(self: &Rc<Self>, path: &Path, pw: Option<String>, dir: &str) {
        if let Ok(arc) = engine::open(path, pw.as_deref()) {
            *self.archive.borrow_mut() = Some(arc);
            *self.dir.borrow_mut() = dir.to_string();
            self.show_dir();
        }
    }

    fn delete_selected(self: &Rc<Self>) {
        let paths = self.selected_paths();
        let Some(a) = self.archive.borrow().clone() else { return };
        if paths.is_empty() {
            return;
        }
        let d = adap::AlertDialog::new(
            Some(&format!("Supprimer {} élément{} de l'archive ?", paths.len(), if paths.len() > 1 { "s" } else { "" })),
            Some("Ils seront retirés de l'archive (les fichiers sur le disque ne changent pas)."),
        );
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Supprimer")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Destructive);
        let me = self.clone();
        d.connect_response(None, move |_, r| {
            if r != "ok" {
                return;
            }
            let (a2, paths2, me2) = (a.clone(), paths.clone(), me.clone());
            let (path, pw, dir) = (a.path.clone(), a.password.clone(), me.dir.borrow().clone());
            jobs::run(&me.window, "Suppression dans l'archive", 0, move |job, send| engine::delete(job, &a2, &paths2, send).map(|_| a2.path.clone()), Box::new(move |r| match r {
                Ok(_) => {
                    me2.toast("Éléments supprimés de l'archive");
                    me2.reload(&path, pw, &dir);
                }
                Err(e) => me2.error("Suppression impossible", &e),
            }));
        });
        d.present(Some(&self.window));
    }

    fn test(self: &Rc<Self>) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let me = self.clone();
        let total = a.total_size();
        jobs::run(&self.window, "Vérification de l'archive", total, move |job, send| engine::test(job, &a, send).map(|_| a.path.clone()), Box::new(move |r| {
            let (title, body) = match r {
                Ok(_) => ("Archive intacte", "Tous les fichiers ont été lus et leurs sommes de contrôle sont correctes.".to_string()),
                Err(e) => ("Archive endommagée", e),
            };
            me.error(title, &body);
        }));
    }

    fn convert(self: &Rc<Self>) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let formats: Vec<OutFormat> = OutFormat::ALL.into_iter().filter(|f| *f != OutFormat::Iso).collect();
        let names: Vec<String> = formats.iter().map(|f| format!(".{} — {}", f.ext(), f.description())).collect();
        let combo = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let d = adap::AlertDialog::new(Some("Convertir l'archive"), Some(&format!("« {} » sera recréée dans le format choisi, à côté de l'original.", a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())));
        d.set_extra_child(Some(&combo));
        d.add_responses(&[("cancel", "Annuler"), ("ok", "Convertir")]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
        let me = self.clone();
        d.connect_response(None, move |_, r| {
            if r != "ok" {
                return;
            }
            let f = formats[combo.selected() as usize];
            let out = a.path.parent().unwrap_or(Path::new("/")).join(format!("{}.{}", engine::stem(&a.path), f.ext()));
            let o = engine::CreateOptions { sources: vec![], output: out, format: f, level: 7, password: None, encrypt_names: false, split_mb: 0, solid: true, excludes: vec![], label: String::new() };
            let (a2, me2) = (a.clone(), me.clone());
            jobs::run(&me.window, &format!("Conversion en .{}", f.ext()), a.total_size(), move |job, send| engine::convert(job, &a2, &o, send), Box::new(move |r| match r {
                Ok(p) => {
                    let before = std::fs::metadata(&me2.archive.borrow().as_ref().map(|x| x.path.clone()).unwrap_or_default()).map(|m| m.len()).unwrap_or(0);
                    let after = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    me2.toast(&format!("Converti : {} → {}", human(before), human(after)));
                    me2.open_archive(&p, None);
                }
                Err(e) => me2.error("Conversion impossible", &e),
            }));
        });
        d.present(Some(&self.window));
    }

    fn mount(self: &Rc<Self>) {
        let Some(a) = self.archive.borrow().clone() else { return };
        let me = self.clone();
        let (tx, rx) = async_channel::bounded(1);
        let path = a.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send_blocking(noos_core::disk_image::mount(&path));
        });
        glib::spawn_future_local(async move {
            match rx.recv().await {
                Ok(Ok(m)) => {
                    if let Some(mp) = m.mountpoints.first() {
                        jobs::show_folder(mp);
                    }
                    me.toast("Image montée (lecture seule) : démontez-la depuis l'Explorateur");
                }
                Ok(Err(e)) => me.error("Montage impossible", &e),
                Err(_) => {}
            }
        });
    }

    fn choose_archive(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Archives"));
        for e in engine::EXTENSIONS {
            filter.add_suffix(e);
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("Tous les fichiers"));
        all.add_pattern("*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        filters.append(&all);
        let fd = gtk::FileDialog::builder().title("Ouvrir une archive").filters(&filters).build();
        let me = self.clone();
        fd.open(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                me.open_archive(&p, None);
            }
        });
    }

    fn setup_actions(self: &Rc<Self>) {
        let add = |name: &str, accels: &[&str], f: fn(&Rc<App>)| {
            let a = gio::SimpleAction::new(name, None);
            let me = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(me) = me.upgrade() {
                    f(&me);
                }
            });
            self.window.add_action(&a);
            if let Some(app) = self.window.application() {
                app.set_accels_for_action(&format!("win.{name}"), accels);
            }
        };
        add("open", &["<Control>o"], |a| a.choose_archive());
        add("new", &["<Control>n"], |a| a.show_create(&[]));
        add("extract", &["<Control>e"], |a| a.extract(vec![]));
        add("extract-selected", &[], |a| {
            let s = a.selected_paths();
            a.extract(s);
        });
        add("open-selected", &[], |a| {
            if let Some(e) = a.selected_paths().first().and_then(|p| a.archive.borrow().as_ref()?.entries.iter().find(|e| &e.path == p).cloned()) {
                if e.is_dir {
                    a.enter(&e.path);
                } else {
                    a.open_entry(&e);
                }
            }
        });
        add("delete", &["Delete"], |a| a.delete_selected());
        add("test", &[], |a| a.test());
        add("convert", &[], |a| a.convert());
        add("mount", &[], |a| a.mount());
        add("reveal", &[], |a| {
            if let Some(p) = a.archive.borrow().as_ref().and_then(|x| x.path.parent().map(Path::to_path_buf)) {
                jobs::show_folder(&p);
            }
        });
        add("add", &[], |a| {
            let fd = gtk::FileDialog::builder().title("Ajouter à l'archive").build();
            let me = a.clone();
            fd.open_multiple(Some(&a.window), gio::Cancellable::NONE, move |r| {
                if let Ok(files) = r {
                    let v: Vec<PathBuf> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path()).collect();
                    me.add_files(v);
                }
            });
        });
        add("up", &["<Alt>Up", "BackSpace"], |a| a.go_up());
    }
}

/// Mode « Extraire ici » : sans fenêtre principale, une progression par archive.
fn extract_here(app: &adap::Application, files: Vec<PathBuf>, ask_dest: bool) {
    let hold = app.hold();
    let win = adap::ApplicationWindow::builder().application(app).title("Archives").default_width(480).default_height(160).build();
    win.set_content(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    win.present();
    let queue = Rc::new(RefCell::new(files));
    fn next(win: adap::ApplicationWindow, queue: Rc<RefCell<Vec<PathBuf>>>, ask: bool, hold: Rc<RefCell<Option<gio::ApplicationHoldGuard>>>) {
        let Some(path) = (if queue.borrow().is_empty() { None } else { Some(queue.borrow_mut().remove(0)) }) else {
            hold.borrow_mut().take();
            win.close();
            return;
        };
        let cont = {
            let (win, queue, hold) = (win.clone(), queue.clone(), hold.clone());
            Rc::new(move || next(win.clone(), queue.clone(), ask, hold.clone()))
        };
        open_then(&win, &path, None, cont.clone(), Rc::new(move |win: &adap::ApplicationWindow, a: Archive| {
            let total = a.total_size();
            let run = {
                let (win, cont) = (win.clone(), cont.clone());
                move |a: Archive, o: engine::ExtractOptions| {
                    let name = a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let cont = cont.clone();
                    let win2 = win.clone();
                    jobs::run(&win, &format!("Extraction de « {name} »"), total, move |job, send| engine::extract(job, &a, &o, send), Box::new(move |r| {
                        match r {
                            Ok(d) => {
                                if let Some(app) = gio::Application::default() {
                                    let n = gio::Notification::new("Extraction terminée");
                                    n.set_body(Some(&format!("{name} → {}", d.display())));
                                    app.send_notification(None, &n);
                                }
                            }
                            Err(e) => {
                                let d = adap::AlertDialog::new(Some("Échec de l'extraction"), Some(&format!("{name}\n\n{e}")));
                                d.add_response("ok", "Fermer");
                                d.present(Some(&win2));
                            }
                        }
                        cont();
                    }));
                }
            };
            if ask {
                let a2 = a.clone();
                jobs::extract_dialog(win, &a, vec![], move |o, _| run(a2.clone(), o));
            } else {
                let base = a.path.parent().map(Path::to_path_buf).unwrap_or_else(glib::home_dir);
                let o = engine::ExtractOptions { dest: engine::smart_destination(&a, &base), only: vec![], keep_paths: true, overwrite: engine::Overwrite::Rename, password: a.password.clone() };
                run(a, o);
            }
        }));
    }
    next(win, queue, ask_dest, Rc::new(RefCell::new(Some(hold))));
}

/// Mode « Compresser en ZIP / 7z » : archive créée à côté des fichiers, sans questions.
fn quick_compress(app: &adap::Application, format: OutFormat, files: Vec<PathBuf>) {
    let hold = app.hold();
    let win = adap::ApplicationWindow::builder().application(app).title("Archives").default_width(480).default_height(160).build();
    win.set_content(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    win.present();
    let Some(first) = files.first().cloned() else { return };
    let dir = first.parent().map(Path::to_path_buf).unwrap_or_else(glib::home_dir);
    let base = if files.len() == 1 {
        first.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
    } else {
        dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
    };
    let mut out = dir.join(format!("{base}.{}", format.ext()));
    let mut i = 2;
    while out.exists() {
        out = dir.join(format!("{base} ({i}).{}", format.ext()));
        i += 1;
    }
    let o = engine::CreateOptions {
        sources: files,
        output: out.clone(),
        format,
        level: 5,
        password: None,
        encrypt_names: false,
        split_mb: 0,
        solid: true,
        excludes: vec![],
        label: String::new(),
    };
    let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let w2 = win.clone();
    let hold = RefCell::new(Some(hold));
    jobs::run(&win, &format!("Création de « {name} »"), 0, move |job, send| engine::create(job, &o, send), Box::new(move |r| {
        if let Err(e) = r {
            let d = adap::AlertDialog::new(Some("Échec de la compression"), Some(&e));
            d.add_response("ok", "Fermer");
            let w3 = w2.clone();
            d.connect_response(None, move |_, _| w3.close());
            d.present(Some(&w2));
        } else {
            w2.close();
        }
        hold.borrow_mut().take();
    }));
}

/// Ouvre une archive (mot de passe demandé si besoin) puis appelle `then`.
type Then = Rc<dyn Fn(&adap::ApplicationWindow, Archive)>;

fn open_then(win: &adap::ApplicationWindow, path: &Path, pw: Option<String>, skip: Rc<dyn Fn()>, then: Then) {
    let p = path.to_path_buf();
    let pw2 = pw.clone();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(engine::open(&p, pw2.as_deref()));
    });
    let (win, path) = (win.clone(), path.to_path_buf());
    glib::spawn_future_local(async move {
        match rx.recv().await {
            Ok(Ok(a)) => then(&win, a),
            Ok(Err(OpenError::Password)) => {
                let (win2, path2, skip2, then2) = (win.clone(), path.clone(), skip.clone(), then.clone());
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                jobs::ask_password(&win, &name, pw.is_some(), move |pw| {
                    open_then(&win2, &path2, Some(pw), skip2.clone(), then2.clone());
                });
            }
            Ok(Err(OpenError::Other(e))) => {
                let d = adap::AlertDialog::new(Some("Archive illisible"), Some(&format!("{}\n\n{e}", path.display())));
                d.add_response("ok", "Fermer");
                let skip = skip.clone();
                d.connect_response(None, move |_, _| skip());
                d.present(Some(&win));
            }
            Err(_) => skip(),
        }
    });
}

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().cloned().unwrap_or_default();
    let files: Vec<PathBuf> = args.iter().filter(|a| !a.starts_with("--")).map(PathBuf::from).map(|p| p.canonicalize().unwrap_or(p)).collect();
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::NON_UNIQUE).build();
    app.connect_startup(|_| {
        std::mem::forget(noos_theme::Theming::init());
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        if let Some(d) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
    });
    app.connect_activate(move |gapp| {
        match mode.as_str() {
            "--extract-here" => return extract_here(gapp, files.clone(), false),
            "--extract-to" => return extract_here(gapp, files.clone(), true),
            "--quick" => {
                let f = match args.get(1).map(String::as_str) {
                    Some("7z") => OutFormat::SevenZ,
                    Some("tar.zst") => OutFormat::TarZst,
                    _ => OutFormat::Zip,
                };
                let files: Vec<PathBuf> = files.iter().filter(|p| p.exists()).cloned().collect();
                return quick_compress(gapp, f, files);
            }
            _ => {}
        }
        let a = App::new(gapp);
        a.window.present();
        match (mode.as_str(), std::env::var("NOOS_ARCHIVE_DEMO").ok().as_deref()) {
            ("--compress", _) => a.show_create(&files),
            (_, Some("create")) => {
                let d = glib::home_dir();
                a.show_create(&[d.join("Documents"), d.join("Images")].into_iter().filter(|p| p.exists()).collect::<Vec<_>>());
            }
            (_, Some(demo)) if demo.starts_with('/') => a.open_archive(Path::new(demo), None),
            _ => {
                if let Some(f) = files.first() {
                    if f.is_file() {
                        a.open_archive(f, None);
                    } else {
                        a.show_create(&files);
                    }
                }
            }
        }
        let keep = RefCell::new(Some(a.clone()));
        a.window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
    });
    app.run_with_args::<&str>(&[])
}

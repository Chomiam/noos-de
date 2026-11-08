//! noos-file — explorateur de fichiers de NOOS, dans l'esprit de l'Explorateur de Windows 11.
//!
//! - Accueil : accès rapide, disques (espace libre), fichiers récents (`home.rs`) ;
//! - barre latérale, précédent / suivant / parent, fil d'Ariane (Ctrl+L : saisir un chemin) ;
//! - barre de commandes : Nouveau, couper / copier / coller, renommer, supprimer, trier,
//!   affichage ; vues Icônes (zoom Ctrl+molette, miniatures) et Détails ;
//! - volet de détails (Alt+P) et aperçu rapide (Espace) (`preview.rs`) ;
//! - glisser-déposer (déplacer, Ctrl : copier), « Ouvrir avec » ;
//! - copies et déplacements suivis (progression, vitesse, temps restant, annulation) (`transfers.rs`) ;
//! - corbeille : restaurer, supprimer définitivement, vider.

mod home;
mod preview;
mod thumbs;
mod transfers;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

const APP_ID: &str = "org.noos.Files";
const ATTRS: &str = "standard::*,time::modified,thumbnail::path,access::can-rename,trash::orig-path,trash::deletion-date";
/// Emplacement virtuel de la page d'accueil.
const HOME_URI: &str = "noos-home:///";
const TRASH_URI: &str = "trash:///";
const ICON_SIZES: [i32; 5] = [48, 64, 96, 128, 160];

#[derive(Clone, Copy, PartialEq)]
enum ClipOp {
    Copy,
    Cut,
}

thread_local! {
    static EXPLORER: RefCell<Option<Rc<Explorer>>> = const { RefCell::new(None) };
    /// Taille des icônes de la vue Icônes (lue par la fabrique de la grille).
    static ICON_SIZE: Cell<i32> = const { Cell::new(64) };
    /// Tri : clé (« name », « date », « size », « type ») et ordre décroissant.
    static SORT: RefCell<(String, bool)> = RefCell::new(("name".into(), false));
}

/// Volume d'un disque (udisks2), pour la barre latérale et l'accueil.
#[derive(Clone, Debug)]
pub struct DriveVolume {
    pub name: String,
    pub icon: String,
    pub block: String,
    pub drive: String,
    pub drive_name: String,
    pub mount: Option<String>,
    pub external: bool,
    pub size: u64,
    pub fs: String,
}

/// Volumes utiles : tous les systèmes de fichiers, sauf ceux du système (/, /boot, /home,
/// déjà couverts par « Ce PC »), l'échange et les volumes cachés.
pub fn drive_volumes() -> Vec<DriveVolume> {
    let Ok(conn) = noos_disk::udisks::system_bus() else { return vec![] };
    let Ok(m) = noos_disk::udisks::load(&conn) else { return vec![] };
    let mut out = vec![];
    for d in &m.drives {
        for v in m.volumes(&d.path) {
            let sys = v.filesystem.iter().flatten().any(|mp| mp == "/" || mp.starts_with("/boot") || mp == "/home" || mp == "/var" || mp == "/sysroot");
            if sys || v.hint_ignore || v.id_type == "swap" || (v.hint_system && !d.is_external() && v.mounted_at().is_none() && v.fstab.is_none()) {
                continue;
            }
            let name = if !v.id_label.is_empty() {
                v.id_label.clone()
            } else if d.is_external() {
                d.name()
            } else {
                format!("Volume {}", noos_disk::human(v.size))
            };
            out.push(DriveVolume {
                name,
                icon: d.icon().to_string(),
                block: v.path.clone(),
                drive: d.path.clone(),
                drive_name: d.name(),
                mount: v.mounted_at().map(String::from),
                external: d.is_external(),
                size: v.size,
                fs: noos_disk::fs_name(&v.id_type),
            });
        }
    }
    out
}

fn explorer() -> Option<Rc<Explorer>> {
    EXPLORER.with(|e| e.borrow().clone())
}

struct Explorer {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    current: RefCell<gio::File>,
    back: RefCell<Vec<gio::File>>,
    forward: RefCell<Vec<gio::File>>,
    dir_list: gtk::DirectoryList,
    filter: gtk::CustomFilter,
    sorter: gtk::CustomSorter,
    selection: gtk::MultiSelection,
    show_hidden: Cell<bool>,
    query: RefCell<String>,
    crumbs: gtk::Box,
    folder_title: adap::WindowTitle,
    location: gtk::Entry,
    location_stack: gtk::Stack,
    view_stack: gtk::Stack,
    /// « grid » ou « details » : vue utilisée hors de l'accueil.
    view_mode: RefCell<String>,
    grid: gtk::GridView,
    empty_page: adap::StatusPage,
    home_box: gtk::Box,
    status: gtk::Label,
    transfers: transfers::Panel,
    zoom: gtk::Scale,
    details_revealer: gtk::Revealer,
    details_holder: gtk::Box,
    trash_bar: gtk::Box,
    back_btn: gtk::Button,
    forward_btn: gtk::Button,
    up_btn: gtk::Button,
    sidebar: gtk::ListBox,
    clipboard: RefCell<Option<(ClipOp, Vec<gio::File>)>>,
    /// Bandeau « image disque montée » (dossier courant dans une ISO montée).
    image_banner: adap::Banner,
    image_shown: RefCell<Option<std::path::PathBuf>>,
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

fn is_home(f: &gio::File) -> bool {
    f.uri() == HOME_URI
}

fn is_trash(f: &gio::File) -> bool {
    f.uri().starts_with("trash:")
}

/// Icône du type de fichier (les miniatures sont chargées à part, voir `thumbs.rs`).
fn set_icon(info: &gio::FileInfo, image: &gtk::Image) {
    image.remove_css_class("thumb");
    if let Some(thumb) = info.attribute_byte_string("thumbnail::path") {
        image.set_from_file(Some(thumb.as_str()));
        image.add_css_class("thumb");
    } else if let Some(icon) = info.icon() {
        image.set_from_gicon(&icon);
    } else {
        image.set_icon_name(Some("text-x-generic"));
    }
}

/// Miniature chargée en arrière-plan ; ignorée si la case a été réutilisée entre-temps.
fn load_thumbnail(info: &gio::FileInfo, image: &gtk::Image) {
    if thumbs::supported(info).is_none() || info.attribute_byte_string("thumbnail::path").is_some() {
        return;
    }
    let Some(file) = file_of(info.upcast_ref()) else { return };
    let uri = file.uri().to_string();
    unsafe { image.set_data("noos-uri", uri.clone()) };
    let (info, image) = (info.clone(), image.clone());
    glib::spawn_future_local(async move {
        let Some(path) = thumbs::thumbnail(&file, &info).await else { return };
        let still = unsafe { image.data::<String>("noos-uri").map(|p| p.as_ref() == &uri) }.unwrap_or(false);
        if still {
            image.set_from_file(Some(&path));
            image.add_css_class("thumb");
        }
    });
}

/// Ordre d'affichage : dossiers d'abord, puis la clé de tri choisie.
fn compare(a: &gio::FileInfo, b: &gio::FileInfo) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (is_dir(a), is_dir(b)) {
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    let name = || {
        glib::FilenameCollationKey::from(a.display_name().as_str()).cmp(&glib::FilenameCollationKey::from(b.display_name().as_str()))
    };
    let (key, desc) = SORT.with(|s| s.borrow().clone());
    let ord = match key.as_str() {
        "date" => {
            let t = |i: &gio::FileInfo| i.modification_date_time().map(|d| d.to_unix()).unwrap_or(0);
            t(a).cmp(&t(b)).then_with(name)
        }
        "size" => a.size().cmp(&b.size()).then_with(name),
        "type" => preview::kind_label(a).cmp(&preview::kind_label(b)).then_with(name),
        _ => name(),
    };
    if desc { ord.reverse() } else { ord }
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

impl Explorer {
    fn new(app: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Explorateur de fichiers")
            .default_width(1240)
            .default_height(780)
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

        // ── Barre de titre et rangée de navigation ──
        let header = adap::HeaderBar::new();
        let back_btn = gtk::Button::from_icon_name("go-previous-symbolic");
        back_btn.set_tooltip_text(Some("Précédent (Alt+←)"));
        let forward_btn = gtk::Button::from_icon_name("go-next-symbolic");
        forward_btn.set_tooltip_text(Some("Suivant (Alt+→)"));
        let up_btn = gtk::Button::from_icon_name("go-up-symbolic");
        up_btn.set_tooltip_text(Some("Dossier parent (Alt+↑)"));
        let nav_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        nav_row.add_css_class("nav-row");
        for b in [&back_btn, &forward_btn, &up_btn] {
            b.add_css_class("flat");
            nav_row.append(b);
        }
        let folder_title = adap::WindowTitle::new("Explorateur de fichiers", "");
        header.set_title_widget(Some(&folder_title));

        // Barre d'adresse : fil d'Ariane ; un clic dans la zone vide affiche le chemin en texte.
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
            let keys = gtk::EventControllerKey::new();
            let stack = location_stack.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
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
            let adj = crumbs_scroll.hadjustment();
            let to_end = |adj: &gtk::Adjustment| adj.set_value((adj.upper() - adj.page_size()).max(0.0));
            adj.connect_upper_notify(move |a| to_end(a));
            adj.connect_page_size_notify(move |a| to_end(a));
        }
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher"));
        search.set_width_chars(18);
        nav_row.append(&search);

        // ── Barre de commandes (comme Windows 11) ──
        let (command_bar, trash_bar) = command_bar();

        // ── Modèle de liste ──
        let home = gio::File::for_path(glib::home_dir());
        let dir_list = gtk::DirectoryList::new(Some(ATTRS), Some(&home));
        dir_list.set_monitored(true);
        let filter = gtk::CustomFilter::new(|_| true);
        let filtered = gtk::FilterListModel::new(Some(dir_list.clone()), Some(filter.clone()));
        let sorter = gtk::CustomSorter::new(|a, b| {
            let (a, b) = (a.downcast_ref::<gio::FileInfo>().unwrap(), b.downcast_ref::<gio::FileInfo>().unwrap());
            compare(a, b).into()
        });
        let sorted = gtk::SortListModel::new(Some(filtered), Some(sorter.clone()));
        let selection = gtk::MultiSelection::new(Some(sorted));

        let grid = build_grid(&selection);
        let details = build_details(&selection);
        let home_box = gtk::Box::new(gtk::Orientation::Vertical, 14);
        home_box.add_css_class("home-page");
        let home_clamp = adap::Clamp::builder().maximum_size(1100).child(&home_box).build();
        let view_stack = gtk::Stack::new();
        view_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        view_stack.set_transition_duration(120);
        view_stack.add_named(&gtk::ScrolledWindow::builder().child(&grid).vexpand(true).build(), Some("grid"));
        view_stack.add_named(&gtk::ScrolledWindow::builder().child(&details).vexpand(true).build(), Some("details"));
        view_stack.add_named(
            &gtk::ScrolledWindow::builder().child(&home_clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build(),
            Some("home"),
        );
        // Dossier vide : page d'état par-dessus la vue (les clics droits passent dessous).
        let empty_page = adap::StatusPage::builder()
            .icon_name("folder-open-symbolic")
            .title("Ce dossier est vide")
            .description("Glissez des fichiers ici, ou faites un clic droit pour créer un dossier.")
            .build();
        empty_page.set_can_target(false);
        empty_page.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&view_stack));
        overlay.add_overlay(&empty_page);
        // Copies et déplacements en cours : cartes en bas à droite, par-dessus la vue.
        let transfers = transfers::Panel::new();
        overlay.add_overlay(&transfers.widget);

        // ── Volet de détails ──
        let details_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details_holder.set_size_request(300, -1);
        details_holder.add_css_class("details-side");
        let details_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .transition_duration(160)
            .child(&details_holder)
            .build();
        let main_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        overlay.set_hexpand(true);
        main_row.append(&overlay);
        main_row.append(&details_revealer);

        // ── Barre d'état : texte et zoom ──
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        status.add_css_class("dim-label");
        let zoom = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, (ICON_SIZES.len() - 1) as f64, 1.0);
        zoom.set_size_request(120, -1);
        zoom.set_draw_value(false);
        zoom.set_round_digits(0);
        zoom.set_value(1.0);
        zoom.set_tooltip_text(Some("Taille des icônes (Ctrl+molette)"));
        let zoom_icon = gtk::Image::from_icon_name("zoom-in-symbolic");
        zoom_icon.add_css_class("dim-label");
        let status_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        status_bar.add_css_class("status-bar");
        status_bar.append(&status);
        status_bar.append(&zoom_icon);
        status_bar.append(&zoom);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_row.set_vexpand(true);
        let image_banner = adap::Banner::new("");
        image_banner.set_button_label(Some("Démonter"));
        content_box.append(&image_banner);
        content_box.append(&main_row);
        content_box.append(&status_bar);

        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&content_box));
        let content_page = adap::ToolbarView::new();
        content_page.add_top_bar(&header);
        content_page.add_top_bar(&nav_row);
        content_page.add_top_bar(&command_bar);
        content_page.set_content(Some(&toasts));

        let split = adap::NavigationSplitView::new();
        split.set_sidebar(Some(&adap::NavigationPage::new(&sidebar_page, "Emplacements")));
        split.set_content(Some(&adap::NavigationPage::new(&content_page, "Fichiers")));
        split.set_min_sidebar_width(210.0);
        window.set_content(Some(&split));

        let start = gio::File::for_uri(HOME_URI);
        let ex = Rc::new(Self {
            window,
            toasts,
            current: RefCell::new(start.clone()),
            back: RefCell::new(vec![]),
            forward: RefCell::new(vec![]),
            dir_list,
            filter,
            sorter,
            selection,
            show_hidden: Cell::new(false),
            query: RefCell::new(String::new()),
            crumbs,
            folder_title,
            location,
            location_stack,
            view_stack,
            view_mode: RefCell::new("grid".into()),
            grid: grid.clone(),
            empty_page,
            home_box,
            status,
            transfers,
            zoom,
            details_revealer,
            details_holder,
            trash_bar,
            back_btn,
            forward_btn,
            up_btn,
            sidebar,
            clipboard: RefCell::new(None),
            image_banner,
            image_shown: RefCell::new(None),
        });
        EXPLORER.with(|e| *e.borrow_mut() = Some(ex.clone()));

        ex.setup_filter();
        ex.fill_sidebar();
        // Disques branchés / retirés / montés : la barre latérale suit (udisks2).
        if let Ok(conn) = noos_disk::udisks::system_bus() {
            let w = Rc::downgrade(&ex);
            let ids = noos_disk::udisks::watch(&conn, move || {
                if let Some(ex) = w.upgrade() {
                    ex.fill_sidebar();
                    let cur = ex.current.borrow().clone();
                    if is_home(&cur) {
                        ex.navigate(cur, false);
                    }
                }
            });
            std::mem::forget(ids);
        }
        ex.connect_signals(&grid, &details, &search);
        ex.navigate(start, true);
        ex
    }

    fn setup_filter(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.filter.set_filter_func(move |obj| {
            let Some(ex) = weak.upgrade() else { return true };
            let info = obj.downcast_ref::<gio::FileInfo>().unwrap();
            if !ex.show_hidden.get() && (info.is_hidden() || info.is_backup()) {
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
                ex.update_actions();
                ex.update_details();
            }
        });
        let weak = Rc::downgrade(self);
        self.dir_list.connect_loading_notify(move |_| {
            if let Some(ex) = weak.upgrade() {
                ex.update_status();
            }
        });
    }

    fn fill_sidebar(self: &Rc<Self>) {
        while let Some(row) = self.sidebar.row_at_index(0) {
            self.sidebar.remove(&row);
        }
        let themed = |name: &str| -> gio::Icon { gio::ThemedIcon::new(name).upcast() };
        let mut places: Vec<(String, gio::Icon, gio::File)> = vec![
            ("Accueil".into(), themed("go-home-symbolic"), gio::File::for_uri(HOME_URI)),
            ("Dossier personnel".into(), themed("user-home-symbolic"), gio::File::for_path(glib::home_dir())),
        ];
        for (dir, label, icon) in [
            (glib::UserDirectory::Desktop, "Bureau", "user-desktop-symbolic"),
            (glib::UserDirectory::Documents, "Documents", "folder-documents-symbolic"),
            (glib::UserDirectory::Downloads, "Téléchargements", "folder-download-symbolic"),
            (glib::UserDirectory::Pictures, "Images", "folder-pictures-symbolic"),
            (glib::UserDirectory::Music, "Musique", "folder-music-symbolic"),
            (glib::UserDirectory::Videos, "Vidéos", "folder-videos-symbolic"),
        ] {
            if let Some(p) = glib::user_special_dir(dir).filter(|p| p.exists() && p != &glib::home_dir()) {
                places.push((label.into(), themed(icon), gio::File::for_path(p)));
            }
        }
        places.push(("Ce PC".into(), themed("drive-harddisk-symbolic"), gio::File::for_path("/")));
        let n_places = places.len();
        places.push(("Corbeille".into(), themed("user-trash-symbolic"), gio::File::for_uri(TRASH_URI)));
        // Disques et clés USB (udisks2) : montés ou non ; éjection des périphériques externes.
        let volumes = drive_volumes();

        let mut places: Vec<(String, gio::Icon, Option<gio::File>, Option<DriveVolume>)> = places.into_iter().map(|(l, i, f)| (l, i, Some(f), None)).collect();
        let trash = places.pop().unwrap();
        for v in volumes {
            places.push((v.name.clone(), themed(&v.icon), v.mount.as_ref().map(gio::File::for_path), Some(v)));
        }
        places.push(trash);
        let _ = n_places;
        for (i, (label, icon, file, vol)) in places.into_iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            h.append(&gtk::Image::from_gicon(&icon));
            let l = gtk::Label::new(Some(&label));
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            l.set_hexpand(true);
            h.append(&l);
            if let Some(v) = &vol {
                if v.mount.is_none() {
                    l.add_css_class("dim-label");
                }
                if v.external {
                    let ej = gtk::Button::from_icon_name("media-eject-symbolic");
                    ej.add_css_class("flat");
                    ej.add_css_class("sidebar-eject");
                    ej.set_tooltip_text(Some("Retirer en toute sécurité"));
                    let (drive, name) = (v.drive.clone(), v.drive_name.clone());
                    let ex = Rc::downgrade(self);
                    ej.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        let Ok(conn) = noos_disk::udisks::system_bus() else { return };
                        let Ok(m) = noos_disk::udisks::load(&conn) else { return };
                        let (name, ex) = (name.clone(), ex.clone());
                        // Hors du volume avant de le démonter.
                        if let Some(e) = ex.upgrade() {
                            e.navigate(gio::File::for_uri(HOME_URI), true);
                        }
                        noos_disk::ops::safely_remove(&conn, &m, &drive, move |r| {
                            let Some(e) = ex.upgrade() else { return };
                            match r {
                                Ok(()) => e.toast(&format!("« {name} » peut être retiré en toute sécurité")),
                                Err(err) => e.toast(&format!("Impossible de retirer « {name} » : {err}")),
                            }
                        });
                    });
                    h.append(&ej);
                }
            }
            row.set_child(Some(&h));
            // Séparateurs : après « Dossier personnel » et avant « Ce PC ».
            if i == 2 || label == "Ce PC" {
                row.add_css_class("section-start");
            }
            if let Some(v) = vol {
                unsafe { row.set_data("noos-volume", v) };
            }
            let Some(file) = file else {
                self.sidebar.append(&row);
                continue;
            };
            // Déposer des fichiers sur un emplacement : les y déplacer (Corbeille : les jeter).
            if !is_home(&file) {
                let target = file.clone();
                add_drop_target(&row, move |files, copy| {
                    if let Some(ex) = explorer() {
                        if is_trash(&target) {
                            ex.trash_files(&files);
                        } else {
                            ex.transfer(files, target.clone(), copy);
                        }
                    }
                });
            }
            unsafe { row.set_data("noos-file", file) };
            self.sidebar.append(&row);
        }
    }

    fn connect_signals(self: &Rc<Self>, grid: &gtk::GridView, details: &gtk::ColumnView, search: &gtk::SearchEntry) {
        let ex = self.clone();
        self.sidebar.connect_row_activated(move |_, row| {
            if let Some(f) = unsafe { row.data::<gio::File>("noos-file") } {
                let f = unsafe { f.as_ref() }.clone();
                ex.navigate(f, true);
            } else if let Some(v) = unsafe { row.data::<DriveVolume>("noos-volume") } {
                // Volume non monté : le monter (avec vos droits) puis l'ouvrir.
                let v = unsafe { v.as_ref() }.clone();
                ex.mount_and_open(&v.block, &v.name);
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
        search.connect_search_changed(move |s| {
            *ex.query.borrow_mut() = s.text().to_lowercase();
            ex.filter.changed(gtk::FilterChange::Different);
        });

        let ex = self.clone();
        self.location.connect_activate(move |e| {
            let text = e.text().to_string();
            let file = if text.contains("://") { gio::File::for_uri(&text) } else { gio::File::for_path(shellexpand_home(&text)) };
            ex.location_stack.set_visible_child_name("crumbs");
            ex.navigate(file, true);
        });

        let ex = self.clone();
        self.zoom.connect_value_changed(move |s| {
            let size = ICON_SIZES[s.value().round().clamp(0.0, (ICON_SIZES.len() - 1) as f64) as usize];
            ex.set_icon_size(size);
        });

        for view in [grid.clone().upcast::<gtk::Widget>(), details.clone().upcast::<gtk::Widget>()] {
            // Clic dans le vide (hors de tout élément, sans glisser) : désélectionner tout.
            let click = gtk::GestureClick::new();
            click.set_button(gdk::BUTTON_PRIMARY);
            let pressed_at = Rc::new(Cell::new((0.0, 0.0)));
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

            // Menu contextuel (clic droit) : sur un élément non sélectionné, il le sélectionne.
            let gesture = gtk::GestureClick::new();
            gesture.set_button(3);
            let (ex, v) = (self.clone(), view.clone());
            gesture.connect_pressed(move |g, _, x, y| {
                g.set_state(gtk::EventSequenceState::Claimed);
                if !on_item(&v, x, y) {
                    ex.selection.unselect_all();
                }
                ex.context_menu(&v, x, y);
            });
            view.add_controller(gesture);

            // Espace : aperçu rapide ; Suppr : corbeille (seulement quand la vue a le focus,
            // pour ne pas voler la frappe dans la recherche ou le renommage).
            let keys = gtk::EventControllerKey::new();
            let ex = self.clone();
            keys.connect_key_pressed(move |_, key, _, mods| {
                if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                    return glib::Propagation::Proceed;
                }
                match key {
                    gdk::Key::space => {
                        ex.quick_look();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Delete if mods.contains(gdk::ModifierType::SHIFT_MASK) => {
                        ex.delete_forever_selected();
                        glib::Propagation::Stop
                    }
                    gdk::Key::Delete => {
                        ex.trash_selected();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            });
            view.add_controller(keys);

            // Déposer des fichiers dans la vue : dans le dossier courant.
            add_drop_target(&view, |files, copy| {
                if let Some(ex) = explorer() {
                    let dest = ex.current.borrow().clone();
                    if !is_home(&dest) && !is_trash(&dest) {
                        ex.transfer(files, dest, copy);
                    }
                }
            });
        }

        // Ctrl+molette : taille des icônes.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let ex = self.clone();
        scroll.connect_scroll(move |c, _, dy| {
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            ex.zoom_by(if dy < 0.0 { 1 } else { -1 });
            glib::Propagation::Stop
        });
        grid.add_controller(scroll);

        self.install_actions();
        self.demo_hooks(grid);
    }

    /// Variables de test (captures sans souris), voir CLAUDE.md.
    fn demo_hooks(self: &Rc<Self>, grid: &gtk::GridView) {
        let after = |ms: u64, f: Box<dyn FnOnce()>| glib::timeout_add_local_once(std::time::Duration::from_millis(ms), f);
        if let Ok(list) = std::env::var("NOOS_FILE_SELECT_DEMO") {
            let (ex, g) = (self.clone(), grid.clone().upcast::<gtk::Widget>());
            after(1500, Box::new(move || {
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
            }));
        }
        if let Ok(pos) = std::env::var("NOOS_FILE_MENU_DEMO") {
            let (ex, g) = (self.clone(), grid.clone().upcast::<gtk::Widget>());
            after(1500, Box::new(move || {
                let mut it = pos.split(',').filter_map(|v| v.trim().parse::<f64>().ok());
                let (x, y) = (it.next().unwrap_or(100.0), it.next().unwrap_or(100.0));
                let y = if y < 0.0 { g.height() as f64 + y } else { y };
                ex.context_menu(&g, x, y);
            }));
        }
        // NOOS_FILE_DEMO=<chemin>[;details][;list][;select=0,2][;look][;zoom=3][;sort=size][;copy=<source>]
        //                [;menu][;mount][;trash]
        if let Ok(spec) = std::env::var("NOOS_FILE_DEMO") {
            let ex = self.clone();
            after(300, Box::new(move || {
                for part in spec.split(';') {
                    match part.split_once('=') {
                        Some(("zoom", v)) => ex.zoom.set_value(v.parse().unwrap_or(1.0)),
                        Some(("copy", v)) => {
                            let (ex, src) = (ex.clone(), gio::File::for_path(v));
                            glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                                let dest = ex.current.borrow().clone();
                                ex.transfer(vec![src], dest, true);
                            });
                        }
                        Some(("sort", v)) => ex.activate("sort", Some(&v.to_variant())),
                        Some(("select", v)) => {
                            let ex = ex.clone();
                            let v = v.to_string();
                            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
                                for i in v.split(',').filter_map(|i| i.parse::<u32>().ok()) {
                                    ex.selection.select_item(i, false);
                                }
                            });
                        }
                        None if part == "details" => ex.activate("details-pane", None),
                        // Tests : menu contextuel de la sélection, monter l'image, corbeille.
                        // (après la sélection, faite à 1,2 s)
                        None if matches!(part, "menu" | "mount" | "trash") => {
                            let (ex, part) = (ex.clone(), part.to_string());
                            glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || match part.as_str() {
                                "menu" => {
                                    let g = ex.grid.clone().upcast::<gtk::Widget>();
                                    ex.context_menu(&g, 220.0, 120.0);
                                }
                                "mount" => ex.mount_selected(),
                                _ => ex.trash_selected(),
                            });
                        }
                        None if part == "list" => ex.activate("view", Some(&"details".to_variant())),
                        None if part == "look" => {
                            let ex = ex.clone();
                            glib::timeout_add_local_once(std::time::Duration::from_millis(2200), move || ex.quick_look());
                        }
                        None if !part.is_empty() => {
                            let f = if part.contains("://") { gio::File::for_uri(part) } else { gio::File::for_path(shellexpand_home(part)) };
                            ex.navigate(f, true);
                        }
                        _ => {}
                    }
                }
            }));
        }
    }

    fn activate(&self, action: &str, param: Option<&glib::Variant>) {
        gtk::prelude::ActionGroupExt::activate_action(&self.window, action, param);
    }

    fn install_actions(self: &Rc<Self>) {
        let app = self.window.application().unwrap();
        self.image_banner.connect_button_clicked(|_| {
            if let Some(ex) = explorer() {
                let img = ex.image_shown.borrow().clone();
                if let Some(img) = img {
                    ex.unmount_image(img);
                }
            }
        });
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
        add("home", &["<Alt>Home"], Box::new(|ex| ex.navigate(gio::File::for_uri(HOME_URI), true)));
        add("location", &["<Ctrl>l"], Box::new(|ex| ex.edit_location()));
        add("new-folder", &["<Ctrl><Shift>n"], Box::new(|ex| ex.new_item(true)));
        add("new-text", &[], Box::new(|ex| ex.new_item(false)));
        add("trash", &[], Box::new(|ex| ex.trash_selected()));
        add("extract-here", &[], Box::new(|ex| ex.archive_tool("--extract-here", false)));
        add("extract-to", &[], Box::new(|ex| ex.archive_tool("--extract-to", false)));
        add("open-archive", &[], Box::new(|ex| ex.archive_tool("", false)));
        add("compress", &[], Box::new(|ex| ex.archive_tool("--compress", true)));
        add("compress-zip", &[], Box::new(|ex| ex.archive_tool("--quick zip", true)));
        add("compress-7z", &[], Box::new(|ex| ex.archive_tool("--quick 7z", true)));
        add("mount-image", &[], Box::new(|ex| ex.mount_selected()));
        add("unmount-image", &[], Box::new(|ex| {
            if let Some(p) = ex.selected_files().first().and_then(|f| f.path()) {
                ex.unmount_image(p);
            }
        }));
        add("rename", &["F2"], Box::new(|ex| ex.rename_selected()));
        add("copy", &["<Ctrl>c"], Box::new(|ex| ex.clip(ClipOp::Copy)));
        add("cut", &["<Ctrl>x"], Box::new(|ex| ex.clip(ClipOp::Cut)));
        add("paste", &["<Ctrl>v"], Box::new(|ex| ex.paste()));
        add("open", &[], Box::new(|ex| ex.open_selected()));
        add("quick-look", &[], Box::new(|ex| ex.quick_look()));
        add("copy-path", &["<Ctrl><Shift>c"], Box::new(|ex| ex.copy_path()));
        add("terminal", &[], Box::new(|ex| ex.open_terminal()));
        add("refresh", &["F5"], Box::new(|ex| ex.navigate(ex.current.borrow().clone(), false)));
        add("select-all", &["<Ctrl>a"], Box::new(|ex| {
            ex.selection.select_all();
        }));
        add("zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"], Box::new(|ex| ex.zoom_by(1)));
        add("zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"], Box::new(|ex| ex.zoom_by(-1)));
        add("restore", &[], Box::new(|ex| ex.restore_selected()));
        add("delete-forever", &[], Box::new(|ex| ex.delete_forever_selected()));
        add("empty-trash", &[], Box::new(|ex| ex.empty_trash()));
        add("search", &["<Ctrl>f"], Box::new(|ex| {
            if let Some(s) = find_search(&ex.window) {
                s.grab_focus();
            }
        }));

        // Actions à état : cases à cocher et choix dans les menus.
        let hidden = gio::SimpleAction::new_stateful("hidden", None, &false.to_variant());
        let ex = self.clone();
        hidden.connect_activate(move |a, _| {
            let v = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&v.to_variant());
            ex.show_hidden.set(v);
            ex.filter.changed(gtk::FilterChange::Different);
        });
        self.window.add_action(&hidden);
        app.set_accels_for_action("win.hidden", &["<Ctrl>h"]);

        let pane = gio::SimpleAction::new_stateful("details-pane", None, &false.to_variant());
        let ex = self.clone();
        pane.connect_activate(move |a, _| {
            let v = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&v.to_variant());
            ex.details_revealer.set_reveal_child(v);
            ex.update_details();
        });
        self.window.add_action(&pane);
        app.set_accels_for_action("win.details-pane", &["<Alt>p"]);

        let view = gio::SimpleAction::new_stateful("view", Some(glib::VariantTy::STRING), &"grid".to_variant());
        let ex = self.clone();
        view.connect_activate(move |a, p| {
            let Some(mode) = p.and_then(|p| p.get::<String>()) else { return };
            a.set_state(&mode.to_variant());
            *ex.view_mode.borrow_mut() = mode.clone();
            if !is_home(&ex.current.borrow()) {
                ex.view_stack.set_visible_child_name(&mode);
            }
            ex.update_status();
        });
        self.window.add_action(&view);
        app.set_accels_for_action("win.view::grid", &["<Ctrl>1"]);
        app.set_accels_for_action("win.view::details", &["<Ctrl>2"]);

        let sort = gio::SimpleAction::new_stateful("sort", Some(glib::VariantTy::STRING), &"name".to_variant());
        let ex = self.clone();
        sort.connect_activate(move |a, p| {
            let Some(key) = p.and_then(|p| p.get::<String>()) else { return };
            a.set_state(&key.to_variant());
            SORT.with(|s| s.borrow_mut().0 = key);
            ex.sorter.changed(gtk::SorterChange::Different);
        });
        self.window.add_action(&sort);
        let desc = gio::SimpleAction::new_stateful("sort-desc", None, &false.to_variant());
        let ex = self.clone();
        desc.connect_activate(move |a, _| {
            let v = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&v.to_variant());
            SORT.with(|s| s.borrow_mut().1 = v);
            ex.sorter.changed(gtk::SorterChange::Inverted);
        });
        self.window.add_action(&desc);

        let open_with = gio::SimpleAction::new("open-with", Some(glib::VariantTy::STRING));
        let ex = self.clone();
        open_with.connect_activate(move |_, p| {
            let Some(id) = p.and_then(|p| p.get::<String>()) else { return };
            let Some(app) = gio::AppInfo::all().into_iter().find(|a| a.id().is_some_and(|i| i == id)) else { return };
            let files = ex.selected_files();
            let ctx = gdk::Display::default().map(|d| d.app_launch_context());
            if let Err(e) = app.launch(&files, ctx.as_ref()) {
                ex.toast(&format!("Impossible d'ouvrir : {e}"));
            }
        });
        self.window.add_action(&open_with);
        self.update_actions();
    }

    /// Actions qui demandent une sélection, ou qui dépendent de l'emplacement (corbeille).
    fn update_actions(&self) {
        let has_sel = !self.selection.selection().is_empty();
        let current = self.current.borrow().clone();
        let (home, trash) = (is_home(&current), is_trash(&current));
        let set = |name: &str, on: bool| {
            if let Some(a) = self.window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
                a.set_enabled(on);
            }
        };
        for name in ["cut", "copy", "trash", "open", "quick-look"] {
            set(name, has_sel && !(trash && name == "cut"));
        }
        set("rename", has_sel && !trash && self.selection.selection().size() == 1);
        set("paste", !home && !trash);
        set("new-folder", !home && !trash);
        set("new-text", !home && !trash);
        set("restore", trash && has_sel);
        set("delete-forever", trash && has_sel);
        set("trash", has_sel && !trash);
        self.trash_bar.set_visible(trash);
    }

    // ───────────────────────── Navigation ─────────────────────────

    fn navigate(&self, file: gio::File, push_history: bool) {
        let previous = self.current.replace(file.clone());
        if push_history && !previous.equal(&file) {
            self.back.borrow_mut().push(previous);
            self.forward.borrow_mut().clear();
        }
        self.selection.unselect_all();
        if is_home(&file) {
            self.view_stack.set_visible_child_name("home");
            let nav: home::Nav = Rc::new(|f| {
                if let Some(ex) = explorer() {
                    ex.navigate(f, true);
                }
            });
            let open: Rc<dyn Fn(gio::File)> = Rc::new(|f| {
                if let Some(ex) = explorer() {
                    ex.open_file(&f);
                }
            });
            home::fill(&self.home_box, &nav, &open);
        } else {
            self.dir_list.set_file(Some(&file));
            self.view_stack.set_visible_child_name(&self.view_mode.borrow());
        }
        self.update_crumbs(&file);
        self.update_nav_buttons();
        self.update_status();
        self.update_actions();
        self.update_details();
        let title = self.folder_title.title();
        self.window.set_title(Some(&format!("{title} — Explorateur de fichiers")));
        self.check_image(&file);
    }

    /// Dossier dans une image disque montée : bandeau avec « Démonter ».
    fn check_image(&self, dir: &gio::File) {
        let Some(path) = dir.path() else {
            self.image_banner.set_revealed(false);
            return;
        };
        glib::spawn_future_local(async move {
            let found = gio::spawn_blocking(move || noos_core::disk_image::image_for_dir(&path)).await.ok().flatten();
            let Some(ex) = explorer() else { return };
            match found {
                Some(m) => {
                    let name = m.image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ex.image_banner.set_title(&format!("Image disque « {name} » montée (lecture seule)"));
                    ex.image_banner.set_revealed(true);
                    *ex.image_shown.borrow_mut() = Some(m.image);
                }
                None => {
                    ex.image_banner.set_revealed(false);
                    ex.image_shown.borrow_mut().take();
                }
            }
        });
    }

    /// Lance l'application Archives sur la sélection (`mode` : options de ligne de commande).
    fn archive_tool(&self, mode: &str, _create: bool) {
        let files: Vec<String> = self.selected_files().iter().filter_map(|f| f.path()).map(|p| p.to_string_lossy().into_owned()).collect();
        if files.is_empty() {
            return;
        }
        let mut cmd = std::process::Command::new("noos-archive");
        cmd.args(mode.split_whitespace()).args(&files);
        if let Err(e) = cmd.spawn() {
            self.toast(&format!("Archives introuvable : {e}"));
        }
    }

    /// Monte un volume (udisks : avec les droits de l'utilisateur) puis l'ouvre.
    pub fn mount_and_open(self: &Rc<Self>, block: &str, name: &str) {
        let Ok(conn) = noos_disk::udisks::system_bus() else { return };
        let (ex, name) = (Rc::downgrade(self), name.to_string());
        self.toast(&format!("Ouverture de « {name} »…"));
        noos_disk::ops::mount(&conn, block, move |r| {
            let Some(ex) = ex.upgrade() else { return };
            match r {
                Ok(mp) => ex.navigate(gio::File::for_path(mp), true),
                Err(e) => ex.toast(&format!("Impossible d'ouvrir « {name} » : {e}")),
            }
        });
    }

    fn mount_selected(&self) {
        let Some(path) = self.selected_files().first().and_then(|f| f.path()) else { return };
        self.toast("Montage de l'image…");
        glib::spawn_future_local(async move {
            let res = gio::spawn_blocking(move || noos_core::disk_image::mount(&path)).await;
            let Some(ex) = explorer() else { return };
            match res {
                Ok(Ok(m)) => {
                    if let Some(mp) = m.mountpoints.first() {
                        ex.navigate(gio::File::for_path(mp), true);
                    }
                }
                Ok(Err(e)) => ex.toast(&format!("Montage impossible : {e}")),
                Err(_) => {}
            }
        });
    }

    /// Démonte l'image ; si on était dedans, revient au dossier de l'image.
    fn unmount_image(&self, image: std::path::PathBuf) {
        let inside = self.image_shown.borrow().as_ref() == Some(&image);
        glib::spawn_future_local(async move {
            let img = image.clone();
            let res = gio::spawn_blocking(move || noos_core::disk_image::unmount(&img)).await;
            let Some(ex) = explorer() else { return };
            match res {
                Ok(Ok(())) => {
                    ex.toast("Image démontée");
                    if inside {
                        if let Some(parent) = image.parent() {
                            ex.navigate(gio::File::for_path(parent), true);
                        }
                    }
                    ex.image_banner.set_revealed(false);
                }
                Ok(Err(e)) => ex.toast(&format!("Démontage impossible : {e}")),
                Err(_) => {}
            }
        });
    }

    /// Images disque montées parmi `files` : alerte avant suppression, puis `then`.
    fn guard_mounted_images(&self, files: Vec<gio::File>, then: impl Fn(Vec<gio::File>) + 'static) {
        let mounted: Vec<std::path::PathBuf> = files
            .iter()
            .filter_map(|f| f.path())
            .filter(|p| noos_core::disk_image::is_disk_image(p) && noos_core::disk_image::mounted(p).is_some())
            .collect();
        if mounted.is_empty() {
            return then(files);
        }
        let names = mounted.iter().map(|p| format!("« {} »", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())).collect::<Vec<_>>().join(", ");
        let d = adap::AlertDialog::new(
            Some(if mounted.len() > 1 { "Images encore montées" } else { "Image encore montée" }),
            Some(&format!("{names} {} encore {}. Démontez avant de supprimer : sinon le système garderait le fichier ouvert et l'espace ne serait pas libéré.", if mounted.len() > 1 { "sont" } else { "est" }, if mounted.len() > 1 { "montées" } else { "montée" })),
        );
        d.add_responses(&[("cancel", "Annuler"), ("unmount", "Démonter et continuer")]);
        d.set_response_appearance("unmount", adap::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        let then = std::rc::Rc::new(then);
        d.connect_response(None, move |_, r| {
            if r != "unmount" {
                return;
            }
            let (files, mounted, then) = (files.clone(), mounted.clone(), then.clone());
            glib::spawn_future_local(async move {
                let res = gio::spawn_blocking(move || mounted.iter().try_for_each(|p| noos_core::disk_image::unmount(p))).await;
                let Some(ex) = explorer() else { return };
                match res {
                    Ok(Ok(())) => {
                        ex.image_banner.set_revealed(false);
                        then(files);
                    }
                    Ok(Err(e)) => ex.toast(&format!("Démontage impossible : {e}")),
                    Err(_) => {}
                }
            });
        });
        d.present(Some(&self.window));
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
        let current = self.current.borrow().clone();
        if is_home(&current) || is_trash(&current) && current.uri() == TRASH_URI {
            return;
        }
        if let Some(parent) = current.parent() {
            self.navigate(parent, true);
        }
    }

    fn update_nav_buttons(&self) {
        let current = self.current.borrow().clone();
        self.back_btn.set_sensitive(!self.back.borrow().is_empty());
        self.forward_btn.set_sensitive(!self.forward.borrow().is_empty());
        self.up_btn.set_sensitive(!is_home(&current) && current.uri() != TRASH_URI && current.parent().is_some());
    }

    fn label_of(f: &gio::File) -> String {
        if is_home(f) {
            "Accueil".into()
        } else if f.equal(&gio::File::for_path(glib::home_dir())) {
            "Dossier personnel".into()
        } else if f.uri() == TRASH_URI {
            "Corbeille".into()
        } else {
            f.basename().map(|b| b.to_string_lossy().to_string()).filter(|s| s != "/").unwrap_or_else(|| "Ce PC".into())
        }
    }

    fn update_crumbs(&self, file: &gio::File) {
        while let Some(c) = self.crumbs.first_child() {
            self.crumbs.remove(&c);
        }
        let mut chain = vec![file.clone()];
        if !is_home(file) && file.uri() != TRASH_URI {
            while let Some(p) = chain.last().unwrap().parent() {
                chain.push(p);
            }
        }
        chain.reverse();
        // Raccourcit le chemin sous le dossier personnel : « Dossier personnel › Documents ».
        let home = gio::File::for_path(glib::home_dir());
        if let Some(pos) = chain.iter().position(|f| f.equal(&home)) {
            chain.drain(..pos);
        }
        // Chemin long : les premiers dossiers passent derrière un bouton « … ».
        const VISIBLE: usize = 4;
        if chain.len() > VISIBLE {
            let hidden: Vec<gio::File> = chain.drain(..chain.len() - VISIBLE).collect();
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-horizontal-symbolic");
            more.add_css_class("flat");
            more.set_tooltip_text(Some("Dossiers parents"));
            let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let pop = gtk::Popover::new();
            for f in hidden.iter().rev() {
                let b = gtk::Button::with_label(&Self::label_of(f));
                b.add_css_class("flat");
                if let Some(child) = b.child().and_downcast::<gtk::Label>() {
                    child.set_xalign(0.0);
                }
                let (target, pop) = (f.clone(), pop.clone());
                b.connect_clicked(move |_| {
                    pop.popdown();
                    if let Some(ex) = explorer() {
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
            let b = gtk::Button::with_label(&Self::label_of(f));
            b.add_css_class("flat");
            if i == chain.len() - 1 {
                b.add_css_class("heading");
            }
            let target = f.clone();
            b.connect_clicked(move |_| {
                if let Some(ex) = explorer() {
                    ex.navigate(target.clone(), true);
                }
            });
            // Déposer des fichiers sur un élément du fil d'Ariane : dans ce dossier.
            if !is_home(f) && !is_trash(f) {
                let dest = f.clone();
                add_drop_target(&b, move |files, copy| {
                    if let Some(ex) = explorer() {
                        ex.transfer(files, dest.clone(), copy);
                    }
                });
            }
            self.crumbs.append(&b);
            if i + 1 < chain.len() {
                let sep = gtk::Image::from_icon_name("go-next-symbolic");
                sep.add_css_class("dim-label");
                self.crumbs.append(&sep);
            }
        }
        let full = if is_home(file) {
            "Accueil".into()
        } else {
            file.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| file.uri().to_string())
        };
        self.location.set_text(if is_home(file) { "" } else { &full });
        self.folder_title.set_title(&chain.last().map(Self::label_of).unwrap_or_default());
        self.crumbs.set_tooltip_text(Some(&full));
        if let Some(adj) = self.crumbs.parent().and_downcast::<gtk::Viewport>().and_then(|v| v.hadjustment()) {
            for delay in [30u64, 150] {
                let adj = adj.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
                    adj.set_value((adj.upper() - adj.page_size()).max(0.0));
                });
            }
        }
    }

    fn edit_location(&self) {
        self.location_stack.set_visible_child_name("entry");
        self.location.grab_focus();
        self.location.select_region(0, -1);
    }

    fn set_icon_size(&self, size: i32) {
        if ICON_SIZE.with(|s| s.replace(size)) == size {
            return;
        }
        self.grid.set_factory(Some(&grid_factory()));
        let idx = ICON_SIZES.iter().position(|&s| s == size).unwrap_or(1) as f64;
        if (self.zoom.value() - idx).abs() > 0.1 {
            self.zoom.set_value(idx);
        }
    }

    fn zoom_by(&self, step: i32) {
        let idx = (self.zoom.value().round() as i32 + step).clamp(0, ICON_SIZES.len() as i32 - 1);
        self.zoom.set_value(idx as f64);
    }

    fn update_status(&self) {
        let current = self.current.borrow().clone();
        self.zoom.parent().inspect(|bar| {
            // Zoom (icône et curseur) : inutile sur l'accueil et en vue Détails.
            let show = !is_home(&current) && *self.view_mode.borrow() == "grid";
            self.zoom.set_visible(show);
            if let Some(icon) = self.zoom.prev_sibling() {
                icon.set_visible(show);
            }
            let _ = bar;
        });
        if is_home(&current) {
            self.status.set_text("Accueil");
            self.empty_page.set_visible(false);
            return;
        }
        let n = self.selection.n_items();
        let selected = self.selection.selection().size();
        let loading = self.dir_list.is_loading();
        self.empty_page.set_visible(n == 0 && !loading);
        if n == 0 && !loading {
            let (title, desc) = if !self.query.borrow().is_empty() {
                ("Aucun résultat", "Aucun élément de ce dossier ne correspond à la recherche.")
            } else if is_trash(&current) {
                ("La corbeille est vide", "Les éléments supprimés apparaissent ici ; vous pouvez les restaurer.")
            } else {
                ("Ce dossier est vide", "Glissez des fichiers ici, ou faites un clic droit pour créer un dossier.")
            };
            self.empty_page.set_title(title);
            self.empty_page.set_description(Some(desc));
        }
        let text = match (n, selected) {
            (0, _) if loading => "Chargement…".to_string(),
            (0, _) => "Aucun élément".to_string(),
            (n, 0) => format!("{n} élément{}", plural(n as usize)),
            (n, s) => {
                let size: i64 = self.selected_infos().iter().filter(|i| !is_dir(i)).map(|i| i.size()).sum();
                let size = if size > 0 { format!(" ({})", human_size(size)) } else { String::new() };
                format!("{n} éléments — {s} sélectionné{}{size}", plural(s as usize))
            }
        };
        self.status.set_text(&text);
    }

    fn update_details(&self) {
        if !self.details_revealer.reveals_child() {
            return;
        }
        while let Some(c) = self.details_holder.first_child() {
            self.details_holder.remove(&c);
        }
        let current = self.current.borrow().clone();
        let selected = if is_home(&current) { vec![] } else { self.selected_infos() };
        self.details_holder.append(&preview::details(&selected, &current, &Self::label_of(&current)));
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
        } else if is_trash(&self.current.borrow()) {
            preview::quick_look(&self.window, info);
        } else {
            self.open_file(&file);
        }
    }

    fn open_file(&self, file: &gio::File) {
        let ctx = gdk::Display::default().map(|d| d.app_launch_context());
        match gio::AppInfo::launch_default_for_uri(&file.uri(), ctx.as_ref()) {
            Ok(()) => {
                // Fichiers récents de l'accueil.
                gtk::RecentManager::default().add_item(&file.uri());
            }
            Err(e) => self.toast(&format!("Impossible d'ouvrir : {e}")),
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

    fn quick_look(&self) {
        if let Some(info) = self.selected_infos().first() {
            preview::quick_look(&self.window, info);
        }
    }

    /// Nouveau dossier ou document texte, puis renommage immédiat (comme Windows).
    fn new_item(&self, folder: bool) {
        let dir = self.current.borrow().clone();
        if is_home(&dir) || is_trash(&dir) {
            return;
        }
        let (base, ext) = if folder { ("Nouveau dossier", "") } else { ("Nouveau document texte", ".txt") };
        let mut name = format!("{base}{ext}");
        let mut i = 2;
        while dir.child(&name).query_exists(gio::Cancellable::NONE) {
            name = format!("{base} ({i}){ext}");
            i += 1;
        }
        let file = dir.child(&name);
        let res = if folder {
            file.make_directory(gio::Cancellable::NONE)
        } else {
            file.create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE).map(|_| ())
        };
        match res {
            Ok(()) => self.rename_file(&file),
            Err(e) => self.toast(&format!("Création impossible : {e}")),
        }
    }

    fn trash_files(&self, files: &[gio::File]) {
        let mut ok = 0;
        for f in files {
            match f.trash(gio::Cancellable::NONE) {
                Ok(()) => ok += 1,
                Err(e) => self.toast(&format!("Mise à la corbeille impossible : {e}")),
            }
        }
        if ok > 0 {
            let toast = adap::Toast::new(&format!("{ok} élément{} mis à la corbeille", plural(ok)));
            toast.set_button_label(Some("Ouvrir la corbeille"));
            toast.connect_button_clicked(|_| {
                if let Some(ex) = explorer() {
                    ex.navigate(gio::File::for_uri(TRASH_URI), true);
                }
            });
            self.toasts.add_toast(toast);
        }
    }

    fn trash_selected(&self) {
        if is_trash(&self.current.borrow()) {
            self.delete_forever_selected();
            return;
        }
        let files = self.selected_files();
        if !files.is_empty() {
            self.guard_mounted_images(files, |files| {
                if let Some(ex) = explorer() {
                    ex.trash_files(&files);
                }
            });
        }
    }

    fn confirm(&self, title: &str, body: &str, action: &str, f: impl Fn() + 'static) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_responses(&[("cancel", "Annuler"), ("ok", action)]);
        d.set_response_appearance("ok", adap::ResponseAppearance::Destructive);
        d.set_default_response(Some("cancel"));
        d.connect_response(None, move |_, r| {
            if r == "ok" {
                f();
            }
        });
        d.present(Some(&self.window));
    }

    fn delete_forever_selected(&self) {
        let files = self.selected_files();
        if files.is_empty() {
            return;
        }
        if files.iter().filter_map(|f| f.path()).any(|p| noos_core::disk_image::is_disk_image(&p) && noos_core::disk_image::mounted(&p).is_some()) {
            self.guard_mounted_images(files, |_| {
                if let Some(ex) = explorer() {
                    ex.delete_forever_selected();
                }
            });
            return;
        }
        let n = files.len();
        self.confirm(
            &format!("Supprimer définitivement {n} élément{} ?", plural(n)),
            "Ils ne pourront pas être récupérés.",
            "Supprimer",
            move || {
                let files = files.clone();
                glib::spawn_future_local(async move {
                    for f in files {
                        let f2 = f.clone();
                        let res = gio::spawn_blocking(move || transfers::delete_recursive(&f2, None).map_err(|e| e.to_string())).await;
                        if let (Some(ex), Ok(Err(e))) = (explorer(), res) {
                            ex.toast(&format!("Suppression impossible : {e}"));
                        }
                    }
                });
            },
        );
    }

    fn restore_selected(&self) {
        let mut ok = 0;
        for info in self.selected_infos() {
            let (Some(file), Some(orig)) = (file_of(info.upcast_ref()), info.attribute_byte_string("trash::orig-path")) else {
                continue;
            };
            let dest = gio::File::for_path(orig.as_str());
            if let Some(parent) = dest.parent() {
                let _ = parent.make_directory_with_parents(gio::Cancellable::NONE);
            }
            match file.move_(&dest, gio::FileCopyFlags::NONE, gio::Cancellable::NONE, None) {
                Ok(()) => ok += 1,
                Err(e) => self.toast(&format!("Restauration impossible : {e}")),
            }
        }
        if ok > 0 {
            self.toast(&format!("{ok} élément{} restauré{}", plural(ok), plural(ok)));
        }
    }

    fn empty_trash(&self) {
        self.confirm(
            "Vider la corbeille ?",
            "Tous les éléments de la corbeille seront supprimés définitivement.",
            "Vider la corbeille",
            || {
                glib::spawn_future_local(async move {
                    let res = gio::spawn_blocking(|| {
                        let trash = gio::File::for_uri(TRASH_URI);
                        let children = trash.enumerate_children("standard::name", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)?;
                        for info in children {
                            trash.child(info?.name()).delete(gio::Cancellable::NONE)?;
                        }
                        Ok::<_, glib::Error>(())
                    })
                    .await;
                    if let Some(ex) = explorer() {
                        match res {
                            Ok(Ok(())) => ex.toast("Corbeille vidée"),
                            Ok(Err(e)) => ex.toast(&format!("Corbeille : {e}")),
                            Err(_) => {}
                        }
                    }
                });
            },
        );
    }

    fn rename_selected(&self) {
        let files = self.selected_files();
        if let [file] = files.as_slice() {
            self.rename_file(file);
        }
    }

    fn rename_file(&self, file: &gio::File) {
        let old = file.basename().map(|b| b.to_string_lossy().to_string()).unwrap_or_default();
        let entry = gtk::Entry::new();
        entry.set_text(&old);
        entry.set_activates_default(true);
        let dialog = adap::AlertDialog::new(Some("Renommer"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Annuler"), ("rename", "Renommer")]);
        dialog.set_response_appearance("rename", adap::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        let (file, toasts, old_name, e) = (file.clone(), self.toasts.clone(), old.clone(), entry.clone());
        dialog.connect_response(None, move |_, resp| {
            let new = e.text().to_string();
            if resp != "rename" || new.is_empty() || new == old_name {
                return;
            }
            if let Err(err) = file.set_display_name(&new, gio::Cancellable::NONE) {
                toasts.add_toast(adap::Toast::new(&format!("Renommage impossible : {err}")));
            }
        });
        dialog.present(Some(&self.window));
        // Sélectionne le nom sans l'extension, comme Windows.
        let stem = old.rfind('.').filter(|&i| i > 0).unwrap_or(old.len()) as i32;
        glib::idle_add_local_once(move || {
            entry.grab_focus();
            entry.select_region(0, stem);
        });
    }

    fn clip(&self, op: ClipOp) {
        let files = self.selected_files();
        if files.is_empty() {
            return;
        }
        if let Some(display) = gdk::Display::default() {
            let uris: Vec<String> = files.iter().map(|f| f.uri().to_string()).collect();
            display.clipboard().set_text(&uris.join("\n"));
        }
        let n = files.len();
        *self.clipboard.borrow_mut() = Some((op, files));
        self.toast(&format!("{n} élément{} {}", plural(n), if op == ClipOp::Copy { "copié(s)" } else { "coupé(s)" }));
    }

    fn paste(&self) {
        let Some((op, files)) = self.clipboard.borrow().clone() else { return };
        if op == ClipOp::Cut {
            *self.clipboard.borrow_mut() = None;
        }
        let dest = self.current.borrow().clone();
        self.transfer(files, dest, op == ClipOp::Copy);
    }

    /// Copie ou déplace des fichiers dans `dest`, avec suivi de la progression (`transfers.rs`).
    fn transfer(&self, files: Vec<gio::File>, dest: gio::File, copy: bool) {
        let files: Vec<gio::File> = files
            .into_iter()
            // Pas de déplacement d'un élément dans son propre dossier ou dans lui-même.
            .filter(|f| !(f.equal(&dest) || dest.has_prefix(f) || (!copy && f.parent().is_some_and(|p| p.equal(&dest)))))
            .collect();
        if files.is_empty() {
            return;
        }
        let kind = if copy { transfers::Kind::Copy } else { transfers::Kind::Move };
        self.transfers.start(files, dest.clone(), kind, &Self::label_of(&dest), move |n, errors| {
            if let Some(ex) = explorer() {
                if errors.is_empty() {
                    ex.toast(&format!("{n} élément{} {}", plural(n), if copy { "copié(s)" } else { "déplacé(s)" }));
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
        for term in ["noos-term", "ptyxis", "kgx", "gnome-terminal", "foot", "konsole", "xterm"] {
            let mut cmd = std::process::Command::new(term);
            if term == "noos-term" {
                cmd.arg("--working-directory").arg(&dir);
            }
            if cmd.current_dir(&dir).spawn().is_ok() {
                return;
            }
        }
        self.toast("Aucun terminal trouvé");
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    fn context_menu(&self, parent: &gtk::Widget, x: f64, y: f64) {
        let current = self.current.borrow().clone();
        if is_home(&current) {
            return;
        }
        let trash = is_trash(&current);
        let menu = gio::Menu::new();
        let infos = self.selected_infos();
        let section = |items: &[(&str, &str)]| {
            let s = gio::Menu::new();
            for (label, action) in items {
                s.append(Some(label), Some(action));
            }
            s
        };
        if trash && !infos.is_empty() {
            menu.append_section(None, &section(&[("Aperçu", "win.quick-look"), ("Restaurer", "win.restore")]));
            menu.append_section(None, &section(&[("Supprimer définitivement", "win.delete-forever")]));
        } else if trash {
            menu.append_section(None, &section(&[("Vider la corbeille", "win.empty-trash"), ("Actualiser", "win.refresh")]));
        } else if !infos.is_empty() {
            let open = section(&[("Ouvrir", "win.open"), ("Aperçu", "win.quick-look")]);
            // « Ouvrir avec » : applications associées au type du fichier.
            if let [info] = infos.as_slice() {
                if let Some(ct) = info.content_type().filter(|_| !is_dir(info)) {
                    let apps = gio::AppInfo::all_for_type(&ct);
                    if !apps.is_empty() {
                        let sub = gio::Menu::new();
                        for app in apps.iter().take(8) {
                            if let Some(id) = app.id() {
                                let item = gio::MenuItem::new(Some(&app.display_name()), None);
                                item.set_action_and_target_value(Some("win.open-with"), Some(&id.to_variant()));
                                sub.append_item(&item);
                            }
                        }
                        open.append_submenu(Some("Ouvrir avec"), &sub);
                    }
                }
            }
            menu.append_section(None, &open);
            // Archives et images disque.
            let paths: Vec<std::path::PathBuf> = self.selected_files().iter().filter_map(|f| f.path()).collect();
            let arch = gio::Menu::new();
            let plain_archive = |p: &std::path::Path| {
                noos_core::archive::is_archive(p)
                    && !matches!(p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub"))
            };
            if !paths.is_empty() && paths.iter().all(|p| p.is_file() && plain_archive(p)) {
                arch.append(Some("Extraire ici"), Some("win.extract-here"));
                arch.append(Some("Extraire vers…"), Some("win.extract-to"));
                if paths.len() == 1 {
                    arch.append(Some("Ouvrir avec Archives"), Some("win.open-archive"));
                }
            }
            if let [one] = paths.as_slice() {
                if noos_core::disk_image::is_disk_image(one) {
                    if noos_core::disk_image::mounted(one).is_some() {
                        arch.append(Some("Démonter l'image"), Some("win.unmount-image"));
                    } else {
                        arch.append(Some("Monter l'image"), Some("win.mount-image"));
                    }
                }
            }
            if !paths.is_empty() {
                let comp = gio::Menu::new();
                comp.append(Some("En fichier ZIP"), Some("win.compress-zip"));
                comp.append(Some("En fichier 7z"), Some("win.compress-7z"));
                comp.append(Some("Plus d'options…"), Some("win.compress"));
                arch.append_submenu(Some("Compresser"), &comp);
            }
            menu.append_section(None, &arch);
            menu.append_section(
                None,
                &section(&[("Couper", "win.cut"), ("Copier", "win.copy"), ("Renommer", "win.rename"), ("Copier le chemin", "win.copy-path")]),
            );
            menu.append_section(None, &section(&[("Mettre à la corbeille", "win.trash"), ("Propriétés", "win.details-pane")]));
        } else {
            let new = gio::Menu::new();
            new.append(Some("Dossier"), Some("win.new-folder"));
            new.append(Some("Document texte"), Some("win.new-text"));
            let s1 = gio::Menu::new();
            s1.append_submenu(Some("Nouveau"), &new);
            s1.append(Some("Coller"), Some("win.paste"));
            menu.append_section(None, &s1);
            let sort = gio::Menu::new();
            for (label, key) in [("Nom", "name"), ("Date de modification", "date"), ("Taille", "size"), ("Type", "type")] {
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(Some("win.sort"), Some(&key.to_variant()));
                sort.append_item(&item);
            }
            sort.append(Some("Ordre décroissant"), Some("win.sort-desc"));
            let s2 = gio::Menu::new();
            s2.append_submenu(Some("Trier par"), &sort);
            s2.append(Some("Ouvrir dans le terminal"), Some("win.terminal"));
            s2.append(Some("Fichiers cachés"), Some("win.hidden"));
            s2.append(Some("Actualiser"), Some("win.refresh"));
            menu.append_section(None, &s2);
        }
        // Rattaché au contenu de la fenêtre (pas à la vue) ; taille corrigée par fit_popover.
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

/// Barre de commandes : (barre, groupe « corbeille » affiché seulement dans la corbeille).
fn command_bar() -> (gtk::Box, gtk::Box) {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    bar.add_css_class("command-bar");

    let new_menu = gio::Menu::new();
    new_menu.append(Some("Dossier"), Some("win.new-folder"));
    new_menu.append(Some("Document texte"), Some("win.new-text"));
    let new_btn = gtk::MenuButton::new();
    new_btn.set_child(Some(&adap::ButtonContent::builder().icon_name("list-add-symbolic").label("Nouveau").build()));
    new_btn.set_menu_model(Some(&new_menu));
    new_btn.add_css_class("flat");
    bar.append(&new_btn);
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    for (icon, action, tip) in [
        ("edit-cut-symbolic", "win.cut", "Couper (Ctrl+X)"),
        ("edit-copy-symbolic", "win.copy", "Copier (Ctrl+C)"),
        ("edit-paste-symbolic", "win.paste", "Coller (Ctrl+V)"),
        ("document-edit-symbolic", "win.rename", "Renommer (F2)"),
        ("user-trash-symbolic", "win.trash", "Supprimer (Suppr)"),
    ] {
        let b = gtk::Button::from_icon_name(icon);
        b.set_action_name(Some(action));
        b.set_tooltip_text(Some(tip));
        b.add_css_class("flat");
        bar.append(&b);
    }
    bar.append(&gtk::Separator::new(gtk::Orientation::Vertical));

    let sort = gio::Menu::new();
    let keys = gio::Menu::new();
    for (label, key) in [("Nom", "name"), ("Date de modification", "date"), ("Taille", "size"), ("Type", "type")] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.sort"), Some(&key.to_variant()));
        keys.append_item(&item);
    }
    sort.append_section(None, &keys);
    let order = gio::Menu::new();
    order.append(Some("Ordre décroissant"), Some("win.sort-desc"));
    sort.append_section(None, &order);
    let sort_btn = gtk::MenuButton::new();
    sort_btn.set_child(Some(&adap::ButtonContent::builder().icon_name("view-sort-ascending-symbolic").label("Trier").build()));
    sort_btn.set_menu_model(Some(&sort));
    sort_btn.add_css_class("flat");
    bar.append(&sort_btn);

    let view = gio::Menu::new();
    let modes = gio::Menu::new();
    for (label, mode) in [("Icônes", "grid"), ("Détails", "details")] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some("win.view"), Some(&mode.to_variant()));
        modes.append_item(&item);
    }
    view.append_section(None, &modes);
    let zoom = gio::Menu::new();
    zoom.append(Some("Agrandir les icônes"), Some("win.zoom-in"));
    zoom.append(Some("Réduire les icônes"), Some("win.zoom-out"));
    view.append_section(None, &zoom);
    let show = gio::Menu::new();
    show.append(Some("Volet de détails"), Some("win.details-pane"));
    show.append(Some("Fichiers cachés"), Some("win.hidden"));
    view.append_section(None, &show);
    let view_btn = gtk::MenuButton::new();
    view_btn.set_child(Some(&adap::ButtonContent::builder().icon_name("view-grid-symbolic").label("Affichage").build()));
    view_btn.set_menu_model(Some(&view));
    view_btn.add_css_class("flat");
    bar.append(&view_btn);

    // Corbeille : actions propres, à droite.
    let trash_bar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    trash_bar.set_hexpand(true);
    trash_bar.set_halign(gtk::Align::End);
    for (label, action) in [("Restaurer", "win.restore"), ("Vider la corbeille", "win.empty-trash")] {
        let b = gtk::Button::with_label(label);
        b.set_action_name(Some(action));
        b.add_css_class("flat");
        trash_bar.append(&b);
    }
    trash_bar.set_visible(false);
    bar.append(&trash_bar);

    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let details = gtk::ToggleButton::new();
    details.set_child(Some(&adap::ButtonContent::builder().icon_name("sidebar-show-right-symbolic").label("Détails").build()));
    details.set_action_name(Some("win.details-pane"));
    details.add_css_class("flat");
    details.set_tooltip_text(Some("Volet de détails (Alt+P)"));
    bar.append(&details);
    (bar, trash_bar)
}

fn find_search(window: &adap::ApplicationWindow) -> Option<gtk::SearchEntry> {
    let mut stack = vec![window.clone().upcast::<gtk::Widget>()];
    while let Some(w) = stack.pop() {
        if let Ok(s) = w.clone().downcast::<gtk::SearchEntry>() {
            return Some(s);
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            c = child.next_sibling();
            stack.push(child);
        }
    }
    None
}

/// Cible de dépôt de fichiers sur `widget` : `on_drop(fichiers, copier)` (Ctrl : copier).
fn add_drop_target(widget: &impl IsA<gtk::Widget>, on_drop: impl Fn(Vec<gio::File>, bool) + 'static) {
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY | gdk::DragAction::MOVE);
    target.connect_drop(move |t, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else { return false };
        let copy = t.current_event_state().contains(gdk::ModifierType::CONTROL_MASK);
        on_drop(list.files(), copy);
        true
    });
    widget.add_controller(target);
}

fn shellexpand_home(text: &str) -> std::path::PathBuf {
    match text.strip_prefix('~') {
        Some(rest) => glib::home_dir().join(rest.trim_start_matches('/')),
        None => std::path::PathBuf::from(text),
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

/// Fichiers glissés depuis une case : la sélection si la case en fait partie, sinon la case.
fn drag_files(item: &gtk::ListItem) -> Vec<gio::File> {
    if item.is_selected() {
        if let Some(ex) = explorer() {
            return ex.selected_files();
        }
    }
    item.item().and_then(|o| file_of(&o)).into_iter().collect()
}

/// Rend une case glissable (fichiers) et, si c'est un dossier, cible de dépôt.
fn attach_dnd(cell: &gtk::Widget, item: &gtk::ListItem, image: &gtk::Image) {
    let src = gtk::DragSource::new();
    src.set_actions(gdk::DragAction::COPY | gdk::DragAction::MOVE);
    let it = item.downgrade();
    src.connect_prepare(move |_, _, _| {
        let item = it.upgrade()?;
        let files = drag_files(&item);
        if files.is_empty() {
            return None;
        }
        Some(gdk::ContentProvider::for_value(&gdk::FileList::from_array(&files).to_value()))
    });
    let img = image.clone();
    src.connect_drag_begin(move |s, _| {
        let paintable = gtk::WidgetPaintable::new(Some(&img));
        s.set_icon(Some(&paintable), 16, 16);
    });
    cell.add_controller(src);

    let it = item.downgrade();
    add_drop_target(cell, move |files, copy| {
        let Some(item) = it.upgrade() else { return };
        let Some(ex) = explorer() else { return };
        let info = item.item().and_downcast::<gio::FileInfo>();
        let dest = match (info.as_ref().filter(|i| is_dir(i)), info.as_ref().and_then(|i| file_of(i.upcast_ref()))) {
            (Some(_), Some(folder)) => folder,
            _ => ex.current.borrow().clone(),
        };
        if !is_trash(&dest) && !is_home(&dest) {
            ex.transfer(files, dest, copy);
        }
    });
}

fn grid_factory() -> gtk::SignalListItemFactory {
    let size = ICON_SIZE.with(Cell::get);
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        // Tuile de taille fixe, centrée dans la case : c'est elle qui porte le surlignage.
        let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
        v.add_css_class("tile");
        v.set_halign(gtk::Align::Center);
        v.set_valign(gtk::Align::Start);
        v.set_size_request(size + 28, -1);
        let image = gtk::Image::new();
        image.set_pixel_size(size);
        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(((size + 28) / 7).max(10));
        label.set_justify(gtk::Justification::Center);
        v.append(&image);
        v.append(&label);
        attach_dnd(v.upcast_ref(), item, &image);
        item.set_child(Some(&v));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let info = item.item().and_downcast::<gio::FileInfo>().unwrap();
        let v = item.child().and_downcast::<gtk::Box>().unwrap();
        let image = v.first_child().and_downcast::<gtk::Image>().unwrap();
        let label = v.last_child().and_downcast::<gtk::Label>().unwrap();
        set_icon(&info, &image);
        load_thumbnail(&info, &image);
        label.set_text(&info.display_name());
        v.set_tooltip_text(Some(&format!("{}\n{}", info.display_name(), preview::kind_label(&info))));
    });
    factory
}

fn build_grid(selection: &gtk::MultiSelection) -> gtk::GridView {
    let grid = gtk::GridView::new(Some(selection.clone()), Some(grid_factory()));
    grid.set_max_columns(20);
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
            let img = gtk::Image::new();
            if with_icon {
                img.set_pixel_size(20);
                h.append(&img);
            }
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&l);
            if with_icon {
                attach_dnd(h.upcast_ref(), item, &img);
            }
            item.set_child(Some(&h));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let info = item.item().and_downcast::<gio::FileInfo>().unwrap();
            let h = item.child().and_downcast::<gtk::Box>().unwrap();
            if with_icon {
                let img = h.first_child().and_downcast::<gtk::Image>().unwrap();
                set_icon(&info, &img);
                load_thumbnail(&info, &img);
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
    view.append_column(&column("Type", false, preview::kind_label, false));
    view.append_column(&column("Taille", false, |i| if is_dir(i) { String::new() } else { human_size(i.size()) }, false));
    view
}

const CSS: &str = r#"
.nav-row { padding: 4px 10px 6px 10px; }
.address-bar {
  background-color: alpha(@window_fg_color, 0.06);
  border: 1px solid alpha(@window_fg_color, 0.10);
  border-radius: 8px;
  min-height: 32px;
  padding: 0 4px;
}
.address-bar:hover { background-color: alpha(@window_fg_color, 0.09); }
.address-bar button { min-height: 26px; padding: 0 8px; border-radius: 6px; }
.address-bar button:drop(active) { background-color: alpha(@accent_bg_color, 0.30); }
.address-bar entry.address-entry { background: none; border: none; box-shadow: none; outline: none; min-height: 30px; }

/* Barre de commandes (Windows 11) */
.command-bar { padding: 2px 10px 6px 10px; border-bottom: 1px solid alpha(@window_fg_color, 0.07); }
.command-bar > separator { margin: 6px 6px; background: alpha(@window_fg_color, 0.12); min-width: 1px; }
.command-bar button, .command-bar menubutton > button { min-height: 30px; padding: 0 10px; border-radius: 6px; }

/* Barre latérale */
.navigation-sidebar row.section-start { margin-top: 10px; }
.navigation-sidebar button.sidebar-eject { min-height: 22px; min-width: 22px; padding: 2px; margin: -4px -6px -4px 0; }
.navigation-sidebar row:drop(active) { background-color: alpha(@accent_bg_color, 0.30); box-shadow: inset 0 0 0 1px @accent_bg_color; }

/* Vue en icônes : surlignage ajusté à l'icône et au nom, pas à toute la case. */
gridview { padding: 8px; }
gridview > child { padding: 2px; margin: 0; background: none; box-shadow: none; outline: none; }
gridview > child:hover, gridview > child:selected, gridview > child:selected:hover { background: none; }
gridview .tile { padding: 8px 6px 6px 6px; border-radius: 10px; transition: background-color 120ms ease; }
gridview > child:hover .tile { background-color: alpha(@window_fg_color, 0.06); }
gridview > child:selected .tile { background-color: alpha(@accent_bg_color, 0.22); box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.45); }
gridview > child:selected:hover .tile { background-color: alpha(@accent_bg_color, 0.30); }
gridview > child:focus-visible .tile { outline: 2px solid alpha(@accent_color, 0.6); outline-offset: -2px; }
gridview .tile:drop(active) { background-color: alpha(@accent_bg_color, 0.35); box-shadow: inset 0 0 0 2px @accent_bg_color; }
image.thumb { border-radius: 6px; }
columnview > listview > row:drop(active) { background-color: alpha(@accent_bg_color, 0.25); }

/* Copies et déplacements en cours */
.transfer-card {
  padding: 12px 14px; border-radius: 12px;
  background-color: @popover_bg_color; color: @popover_fg_color;
  box-shadow: 0 6px 22px rgba(0, 0, 0, 0.35), inset 0 0 0 1px alpha(@window_fg_color, 0.08);
}
.transfer-card progressbar trough { min-height: 6px; border-radius: 3px; }
.transfer-card progressbar progress { min-height: 6px; border-radius: 3px; }
.transfer-card .transfer-icon { color: @accent_color; }
.transfer-card.finished progressbar progress { background-color: @success_bg_color; }
.transfer-card.failed progressbar progress { background-color: @error_bg_color; }
.transfer-card.cancelled progressbar progress { background-color: alpha(@window_fg_color, 0.3); }

/* Barre d'état */
.status-bar { padding: 4px 12px; border-top: 1px solid alpha(@window_fg_color, 0.07); }
.status-bar scale { padding: 0; }

/* Volet de détails */
.details-side { border-left: 1px solid alpha(@window_fg_color, 0.08); background-color: alpha(@window_fg_color, 0.02); }
.details-visual image.thumb { border-radius: 10px; }
.details-pane row.property { min-height: 44px; }
textview.text-preview { border-radius: 8px; background-color: alpha(@window_fg_color, 0.04); font-size: 0.9em; }
picture.preview-image { border-radius: 8px; }

/* Accueil */
.home-page { margin: 22px 24px 28px 24px; }
.home-title { font-size: 1.8em; font-weight: 700; margin-bottom: 4px; }
.home-section { font-weight: 700; font-size: 1.05em; margin-top: 14px; opacity: 0.85; }
.home-card {
  padding: 12px 14px; border-radius: 12px;
  background-color: alpha(@window_fg_color, 0.045);
  box-shadow: inset 0 0 0 1px alpha(@window_fg_color, 0.06);
}
.home-card:hover { background-color: alpha(@accent_bg_color, 0.14); box-shadow: inset 0 0 0 1px alpha(@accent_bg_color, 0.35); }
.home-card levelbar trough { min-height: 6px; border-radius: 3px; }
.home-card levelbar block.filled { background-color: @accent_bg_color; border-radius: 3px; }
.home-card levelbar block.full-low { background-color: @warning_bg_color; }
.home-card levelbar block.full-high { background-color: @error_bg_color; }
"#;

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_startup(|_| {
        let theming = noos_theme::Theming::init();
        std::mem::forget(theming);
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    });
    app.connect_activate(|app| match explorer() {
        Some(ex) => ex.window.present(),
        None => {
            let ex = Explorer::new(app);
            ex.window.present();
        }
    });
    app.connect_open(|app, files, _| {
        app.activate();
        if let (Some(ex), Some(f)) = (explorer(), files.first()) {
            ex.navigate(f.clone(), true);
        }
    });
    app.run()
}

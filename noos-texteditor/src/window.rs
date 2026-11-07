//! Fenêtre de l'éditeur : onglets, panneau de dossier, accueil, recherche / remplacement,
//! barre d'état, préférences, session.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use sourceview5::prelude::*;

use crate::{config::Config, doc::Doc};

pub struct Win {
    pub window: adap::ApplicationWindow,
    tabs: adap::TabView,
    stack: gtk::Stack,
    split: adap::OverlaySplitView,
    title: adap::WindowTitle,
    toasts: adap::ToastOverlay,
    // Panneau de dossier.
    tree_title: gtk::Label,
    tree: gtk::ListView,
    // Accueil.
    recent: gtk::ListBox,
    // Recherche.
    search_rev: gtk::Revealer,
    find: gtk::SearchEntry,
    replace_rev: gtk::Revealer,
    replace: gtk::Entry,
    count: gtk::Label,
    settings: sourceview5::SearchSettings,
    // Barre d'état.
    lang_btn: gtk::MenuButton,
    lang_list: gtk::ListBox,
    pos: gtk::Label,
    indent_btn: gtk::MenuButton,
    enc: gtk::Label,
    eol_btn: gtk::MenuButton,
    // État.
    docs: RefCell<Vec<Rc<Doc>>>,
    pub cfg: RefCell<Config>,
    css: gtk::CssProvider,
    zoom: Cell<f64>,
    untitled: Cell<u32>,
    autosave: RefCell<Option<glib::SourceId>>,
}

fn button(icon: &str, tip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.add_css_class("flat");
    b
}

fn home_relative(p: &Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

impl Win {
    pub fn new(app: &adap::Application) -> Rc<Self> {
        let cfg = Config::load();
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Éditeur de texte")
            .default_width(cfg.width)
            .default_height(cfg.height)
            .build();

        // ── Barre de titre ──
        let header = adap::HeaderBar::new();
        let title = adap::WindowTitle::new("Éditeur de texte", "");
        header.set_title_widget(Some(&title));
        let side_btn = gtk::ToggleButton::new();
        side_btn.set_icon_name("sidebar-show-symbolic");
        side_btn.set_tooltip_text(Some("Panneau du dossier (F9)"));
        side_btn.set_action_name(Some("win.toggle-sidebar"));
        header.pack_start(&side_btn);
        let open = gtk::Button::with_label("Ouvrir");
        open.set_action_name(Some("win.open"));
        open.set_tooltip_text(Some("Ouvrir un fichier (Ctrl+O)"));
        header.pack_start(&open);
        let new_btn = button("tab-new-symbolic", "Nouveau document (Ctrl+N)");
        new_btn.set_action_name(Some("win.new"));
        header.pack_start(&new_btn);
        let menu = gio::Menu::new();
        let file = gio::Menu::new();
        file.append(Some("Enregistrer sous…"), Some("win.save-as"));
        file.append(Some("Tout enregistrer"), Some("win.save-all"));
        file.append(Some("Ouvrir un dossier…"), Some("win.open-folder"));
        menu.append_section(None, &file);
        let edit = gio::Menu::new();
        edit.append(Some("Rechercher et remplacer"), Some("win.replace"));
        edit.append(Some("Aller à la ligne…"), Some("win.goto"));
        edit.append(Some("Commenter / décommenter"), Some("win.comment"));
        let lines = gio::Menu::new();
        lines.append(Some("Dupliquer"), Some("win.duplicate"));
        lines.append(Some("Supprimer"), Some("win.delete-line"));
        lines.append(Some("Trier"), Some("win.sort"));
        lines.append(Some("Supprimer les espaces en fin de ligne"), Some("win.trim"));
        edit.append_submenu(Some("Lignes"), &lines);
        let case = gio::Menu::new();
        case.append(Some("MAJUSCULES"), Some("win.upper"));
        case.append(Some("minuscules"), Some("win.lower"));
        case.append(Some("Inverser la casse"), Some("win.toggle-case"));
        case.append(Some("Première Lettre En Majuscule"), Some("win.title-case"));
        edit.append_submenu(Some("Casse"), &case);
        menu.append_section(None, &edit);
        let view = gio::Menu::new();
        view.append(Some("Retour à la ligne"), Some("win.wrap"));
        view.append(Some("Mini-carte"), Some("win.minimap"));
        view.append(Some("Espaces visibles"), Some("win.whitespace"));
        view.append(Some("Agrandir le texte"), Some("win.zoom-in"));
        view.append(Some("Réduire le texte"), Some("win.zoom-out"));
        menu.append_section(None, &view);
        let misc = gio::Menu::new();
        misc.append(Some("Préférences"), Some("win.prefs"));
        misc.append(Some("Raccourcis clavier"), Some("win.shortcuts"));
        menu.append_section(None, &misc);
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&menu));
        menu_btn.set_tooltip_text(Some("Menu"));
        header.pack_end(&menu_btn);
        let find_btn = button("edit-find-symbolic", "Rechercher (Ctrl+F)");
        find_btn.set_action_name(Some("win.find"));
        header.pack_end(&find_btn);
        let save = gtk::Button::with_label("Enregistrer");
        save.set_action_name(Some("win.save"));
        save.set_tooltip_text(Some("Enregistrer (Ctrl+S)"));
        header.pack_end(&save);

        // ── Onglets ──
        let tabs = adap::TabView::new();
        let tab_bar = adap::TabBar::new();
        tab_bar.set_view(Some(&tabs));
        tab_bar.set_autohide(false);

        // ── Recherche / remplacement ──
        let settings = sourceview5::SearchSettings::new();
        settings.set_wrap_around(true);
        let find = gtk::SearchEntry::new();
        find.set_placeholder_text(Some("Rechercher"));
        find.set_hexpand(true);
        let count = gtk::Label::new(None);
        count.add_css_class("dim-label");
        count.add_css_class("numeric");
        count.set_width_chars(10);
        let prev = button("go-up-symbolic", "Précédent (Maj+Entrée)");
        prev.set_action_name(Some("win.find-prev"));
        let next = button("go-down-symbolic", "Suivant (Entrée)");
        next.set_action_name(Some("win.find-next"));
        let toggle = |label: &str, tip: &str| {
            let t = gtk::ToggleButton::with_label(label);
            t.set_tooltip_text(Some(tip));
            t.add_css_class("flat");
            t.add_css_class("search-opt");
            t
        };
        let case_t = toggle("Aa", "Respecter la casse");
        let word_t = toggle("ab", "Mot entier");
        let regex_t = toggle(".*", "Expression régulière");
        case_t.bind_property("active", &settings, "case-sensitive").bidirectional().build();
        word_t.bind_property("active", &settings, "at-word-boundaries").bidirectional().build();
        regex_t.bind_property("active", &settings, "regex-enabled").bidirectional().build();
        let rep_toggle = gtk::ToggleButton::new();
        rep_toggle.set_icon_name("edit-find-replace-symbolic");
        rep_toggle.set_tooltip_text(Some("Remplacer (Ctrl+H)"));
        rep_toggle.add_css_class("flat");
        let close_s = button("window-close-symbolic", "Fermer (Échap)");
        let row1 = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for w in [find.upcast_ref::<gtk::Widget>(), count.upcast_ref(), prev.upcast_ref(), next.upcast_ref(), case_t.upcast_ref(), word_t.upcast_ref(), regex_t.upcast_ref(), rep_toggle.upcast_ref(), close_s.upcast_ref()] {
            row1.append(w);
        }
        let replace = gtk::Entry::new();
        replace.set_placeholder_text(Some("Remplacer par"));
        replace.set_hexpand(true);
        let rep_one = gtk::Button::with_label("Remplacer");
        rep_one.set_action_name(Some("win.replace-one"));
        let rep_all = gtk::Button::with_label("Tout remplacer");
        rep_all.set_action_name(Some("win.replace-all"));
        let row2 = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        row2.append(&replace);
        row2.append(&rep_one);
        row2.append(&rep_all);
        let replace_rev = gtk::Revealer::new();
        replace_rev.set_child(Some(&row2));
        rep_toggle.bind_property("active", &replace_rev, "reveal-child").bidirectional().build();
        let sbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        sbox.add_css_class("noos-searchbar");
        sbox.append(&row1);
        sbox.append(&replace_rev);
        let search_rev = gtk::Revealer::new();
        search_rev.set_child(Some(&sbox));
        search_rev.set_transition_type(gtk::RevealerTransitionType::SlideDown);

        // ── Accueil ──
        let welcome = gtk::Box::new(gtk::Orientation::Vertical, 18);
        welcome.set_valign(gtk::Align::Center);
        welcome.set_halign(gtk::Align::Center);
        let logo = gtk::Image::from_icon_name("accessories-text-editor");
        logo.set_pixel_size(96);
        welcome.append(&logo);
        let wt = gtk::Label::new(Some("Éditeur de texte"));
        wt.add_css_class("title-1");
        welcome.append(&wt);
        let ws = gtk::Label::new(Some("Plus de 150 langages reconnus automatiquement · glissez des fichiers ici"));
        ws.add_css_class("dim-label");
        welcome.append(&ws);
        let wb = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        wb.set_halign(gtk::Align::Center);
        for (label, action, cls) in [("Nouveau document", "win.new", "suggested-action"), ("Ouvrir un fichier", "win.open", ""), ("Ouvrir un dossier", "win.open-folder", "")] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("pill");
            if !cls.is_empty() {
                b.add_css_class(cls);
            }
            b.set_action_name(Some(action));
            wb.append(&b);
        }
        welcome.append(&wb);
        let recent = gtk::ListBox::new();
        recent.add_css_class("boxed-list");
        recent.set_selection_mode(gtk::SelectionMode::None);
        recent.set_size_request(460, -1);
        welcome.append(&recent);

        // ── Barre d'état ──
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        status.add_css_class("noos-status");
        let pos = gtk::Label::new(Some("Ln 1, Col 1"));
        pos.add_css_class("numeric");
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        let lang_btn = gtk::MenuButton::new();
        lang_btn.set_label("Texte brut");
        lang_btn.add_css_class("flat");
        lang_btn.set_direction(gtk::ArrowType::Up);
        lang_btn.set_tooltip_text(Some("Langage (coloration)"));
        let lang_list = gtk::ListBox::new();
        lang_list.set_selection_mode(gtk::SelectionMode::None);
        let lang_search = gtk::SearchEntry::new();
        lang_search.set_placeholder_text(Some("Rechercher un langage"));
        let lang_scroll = gtk::ScrolledWindow::builder().child(&lang_list).min_content_height(320).min_content_width(260).hscrollbar_policy(gtk::PolicyType::Never).build();
        let lang_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        lang_box.append(&lang_search);
        lang_box.append(&lang_scroll);
        let lang_pop = gtk::Popover::new();
        lang_pop.set_child(Some(&lang_box));
        lang_btn.set_popover(Some(&lang_pop));
        {
            let ls = lang_search.clone();
            lang_pop.connect_map(move |_| {
                ls.set_text("");
                ls.grab_focus();
            });
        }
        let indent_btn = gtk::MenuButton::new();
        indent_btn.add_css_class("flat");
        indent_btn.set_direction(gtk::ArrowType::Up);
        indent_btn.set_tooltip_text(Some("Indentation"));
        let enc = gtk::Label::new(Some("UTF-8"));
        enc.add_css_class("dim-label");
        enc.set_tooltip_text(Some("Encodage du fichier (conservé à l'enregistrement)"));
        let eol_btn = gtk::MenuButton::new();
        eol_btn.add_css_class("flat");
        eol_btn.set_direction(gtk::ArrowType::Up);
        eol_btn.set_tooltip_text(Some("Fins de ligne"));
        status.append(&pos);
        status.append(&spacer);
        status.append(&indent_btn);
        status.append(&enc);
        status.append(&eol_btn);
        status.append(&lang_btn);

        // ── Panneau du dossier ──
        let tree_title = gtk::Label::new(Some("Aucun dossier"));
        tree_title.add_css_class("heading");
        tree_title.set_xalign(0.0);
        tree_title.set_hexpand(true);
        tree_title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let folder_btn = button("folder-open-symbolic", "Ouvrir un dossier");
        folder_btn.set_action_name(Some("win.open-folder"));
        let th = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        th.set_margin_start(12);
        th.set_margin_end(6);
        th.set_margin_top(8);
        th.set_margin_bottom(4);
        th.append(&tree_title);
        th.append(&folder_btn);
        let tree = gtk::ListView::new(None::<gtk::NoSelection>, None::<gtk::ListItemFactory>);
        tree.add_css_class("navigation-sidebar");
        let tree_scroll = gtk::ScrolledWindow::builder().child(&tree).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let side = gtk::Box::new(gtk::Orientation::Vertical, 0);
        side.add_css_class("noos-tree");
        side.append(&th);
        side.append(&tree_scroll);

        // ── Assemblage ──
        let editor = gtk::Box::new(gtk::Orientation::Vertical, 0);
        editor.append(&tab_bar);
        editor.append(&search_rev);
        editor.append(&tabs);
        tabs.set_vexpand(true);
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.add_named(&welcome, Some("welcome"));
        stack.add_named(&editor, Some("editor"));
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.append(&stack);
        stack.set_vexpand(true);
        main.append(&status);
        let split = adap::OverlaySplitView::new();
        split.set_sidebar(Some(&side));
        split.set_content(Some(&main));
        split.set_min_sidebar_width(200.0);
        split.set_max_sidebar_width(320.0);
        split.set_show_sidebar(cfg.sidebar && cfg.folder.is_some());
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&split));
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&header);
        tv.set_content(Some(&toasts));
        window.set_content(Some(&tv));

        let css = gtk::CssProvider::new();
        gtk::style_context_add_provider_for_display(&gtk::prelude::WidgetExt::display(&window), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

        let win = Rc::new(Self {
            window,
            tabs,
            stack,
            split,
            title,
            toasts,
            tree_title,
            tree,
            recent,
            search_rev,
            find,
            replace_rev,
            replace,
            count,
            settings,
            lang_btn,
            lang_list,
            pos,
            indent_btn,
            enc,
            eol_btn,
            docs: RefCell::new(vec![]),
            cfg: RefCell::new(cfg),
            css,
            zoom: Cell::new(1.0),
            untitled: Cell::new(0),
            autosave: RefCell::new(None),
        });
        win.setup_actions();
        win.setup_signals(&lang_search, &close_s);
        win.fill_languages();
        win.apply_font();
        win.refresh_recent();
        win.update_chrome();
        if let Some(f) = win.cfg.borrow().folder.clone() {
            win.set_folder(Path::new(&f));
        }
        win
    }

    // ───────────────────────────── Documents ─────────────────────────────

    pub fn current(&self) -> Option<Rc<Doc>> {
        let page = self.tabs.selected_page()?;
        let child = page.child();
        self.docs.borrow().iter().find(|d| d.root.upcast_ref::<gtk::Widget>() == &child).cloned()
    }

    fn page_of(&self, doc: &Doc) -> Option<adap::TabPage> {
        Some(self.tabs.page(&doc.root))
    }

    pub fn new_doc(self: &Rc<Self>) -> Rc<Doc> {
        self.untitled.set(self.untitled.get() + 1);
        let doc = Doc::new(&self.cfg.borrow(), self.untitled.get(), &self.settings);
        self.apply_scheme_to(&doc);
        let page = self.tabs.append(&doc.root);
        page.set_title(&doc.title());
        page.set_icon(Some(&gio::ThemedIcon::new("text-x-generic-symbolic")));
        self.docs.borrow_mut().push(doc.clone());
        self.wire_doc(&doc);
        self.tabs.set_selected_page(&page);
        self.stack.set_visible_child_name("editor");
        doc.view.grab_focus();
        doc
    }

    fn wire_doc(self: &Rc<Self>, doc: &Rc<Doc>) {
        let weak = Rc::downgrade(self);
        let d = Rc::downgrade(doc);
        doc.buffer.connect_modified_changed(move |_| {
            if let (Some(w), Some(d)) = (weak.upgrade(), d.upgrade()) {
                w.update_tab(&d);
                w.update_chrome();
            }
        });
        let weak = Rc::downgrade(self);
        doc.buffer.connect_cursor_moved(move |_| {
            if let Some(w) = weak.upgrade() {
                w.update_status();
            }
        });
        let weak = Rc::downgrade(self);
        doc.buffer.connect_changed(move |_| {
            if let Some(w) = weak.upgrade() {
                w.schedule_autosave();
            }
        });
        let weak = Rc::downgrade(self);
        doc.buffer.connect_language_notify(move |_| {
            if let Some(w) = weak.upgrade() {
                w.update_status();
            }
        });
        let weak = Rc::downgrade(self);
        doc.search.connect_occurrences_count_notify(move |_| {
            if let Some(w) = weak.upgrade() {
                w.update_count();
            }
        });
        let (weak, d) = (Rc::downgrade(self), Rc::downgrade(doc));
        doc.banner.connect_button_clicked(move |b| {
            b.set_revealed(false);
            if let (Some(w), Some(d)) = (weak.upgrade(), d.upgrade()) {
                if let Some(p) = d.path() {
                    let w2 = w.clone();
                    d.load(&p, move |r| {
                        if let Err(e) = r {
                            w2.toast(&format!("Rechargement impossible : {e}"));
                        }
                    });
                }
            }
        });
    }

    fn update_tab(&self, doc: &Doc) {
        if let Some(page) = self.page_of(doc) {
            page.set_title(&format!("{}{}", if doc.modified() { "● " } else { "" }, doc.title()));
            page.set_tooltip(&doc.path().map(|p| home_relative(&p)).unwrap_or_else(|| "Pas encore enregistré".into()));
            let icon = doc
                .path()
                .map(|p| gio::content_type_get_symbolic_icon(&gio::content_type_guess(Some(&p), None).0))
                .unwrap_or_else(|| gio::ThemedIcon::new("text-x-generic-symbolic").upcast());
            page.set_icon(Some(&icon));
        }
    }

    /// Ouvre un fichier (onglet existant sélectionné ; document vide réutilisé).
    pub fn open_path(self: &Rc<Self>, path: &Path) {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if path.is_dir() {
            self.set_folder(&path);
            return;
        }
        if let Some(d) = self.docs.borrow().iter().find(|d| d.path().as_deref() == Some(path.as_path())).cloned() {
            if let Some(p) = self.page_of(&d) {
                self.tabs.set_selected_page(&p);
            }
            return;
        }
        let doc = match self.current().filter(|d| d.is_empty_untitled()) {
            Some(d) => d,
            None => self.new_doc(),
        };
        let weak = Rc::downgrade(self);
        let (d2, p2) = (doc.clone(), path.clone());
        doc.load(&path, move |res| {
            let Some(w) = weak.upgrade() else { return };
            match res {
                Ok(()) => {
                    w.update_tab(&d2);
                    w.cfg.borrow_mut().add_recent(&p2.to_string_lossy());
                    w.refresh_recent();
                    w.update_chrome();
                }
                Err(e) => {
                    w.toast(&format!("Ouverture de {} impossible : {e}", p2.display()));
                    w.close_doc(&d2);
                }
            }
        });
    }

    fn close_doc(self: &Rc<Self>, doc: &Rc<Doc>) {
        if let Some(p) = self.page_of(doc) {
            self.tabs.close_page(&p);
        }
    }

    pub fn save(self: &Rc<Self>, doc: &Rc<Doc>, then: impl FnOnce(bool) + 'static) {
        if doc.path().is_none() {
            return self.save_as(doc, then);
        }
        let weak = Rc::downgrade(self);
        let d = doc.clone();
        doc.save(None, move |r| {
            let Some(w) = weak.upgrade() else { return };
            match r {
                Ok(()) => {
                    w.update_tab(&d);
                    then(true)
                }
                Err(e) => {
                    w.error("Enregistrement impossible", &e);
                    then(false)
                }
            }
        });
    }

    pub fn save_as(self: &Rc<Self>, doc: &Rc<Doc>, then: impl FnOnce(bool) + 'static) {
        let dialog = gtk::FileDialog::builder().title("Enregistrer sous").modal(true).build();
        dialog.set_initial_name(Some(&doc.title()));
        if let Some(dir) = doc.path().and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.cfg.borrow().folder.clone().map(PathBuf::from)) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let (w, d) = (self.clone(), doc.clone());
        dialog.save(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(f) = res else { return then(false) };
            let Some(path) = f.path() else { return then(false) };
            let (w2, d2, p2) = (w.clone(), d.clone(), path.clone());
            d.save(Some(&path), move |r| match r {
                Ok(()) => {
                    w2.update_tab(&d2);
                    w2.cfg.borrow_mut().add_recent(&p2.to_string_lossy());
                    w2.update_chrome();
                    then(true)
                }
                Err(e) => {
                    w2.error("Enregistrement impossible", &e);
                    then(false)
                }
            });
        });
    }

    fn schedule_autosave(self: &Rc<Self>) {
        if !self.cfg.borrow().autosave {
            return;
        }
        if let Some(id) = self.autosave.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        *self.autosave.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            let Some(w) = weak.upgrade() else { return };
            w.autosave.borrow_mut().take();
            for d in w.docs.borrow().iter().filter(|d| d.modified() && d.path().is_some()) {
                w.save(d, |_| {});
            }
        }));
    }

    // ───────────────────────────── Interface ─────────────────────────────

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    fn error(&self, title: &str, body: &str) {
        let d = adap::AlertDialog::new(Some(title), Some(body));
        d.add_response("ok", "Fermer");
        d.present(Some(&self.window));
    }

    fn update_chrome(&self) {
        match self.current() {
            Some(d) => {
                self.title.set_title(&format!("{}{}", if d.modified() { "● " } else { "" }, d.title()));
                self.title.set_subtitle(&d.path().and_then(|p| p.parent().map(home_relative)).unwrap_or_default());
            }
            None => {
                self.title.set_title("Éditeur de texte");
                self.title.set_subtitle(&self.cfg.borrow().folder.clone().map(|f| home_relative(Path::new(&f))).unwrap_or_default());
            }
        }
        self.update_status();
    }

    fn update_status(&self) {
        let has_doc = self.current().is_some();
        for w in [self.lang_btn.upcast_ref::<gtk::Widget>(), self.indent_btn.upcast_ref(), self.eol_btn.upcast_ref(), self.enc.upcast_ref()] {
            w.set_visible(has_doc);
        }
        let Some(d) = self.current() else {
            self.pos.set_text("");
            return;
        };
        let (l, c, sel) = d.position();
        self.pos.set_text(&if sel > 0 { format!("Ln {l}, Col {c} · {sel} sélectionnés") } else { format!("Ln {l}, Col {c}") });
        self.lang_btn.set_label(&d.buffer.language().map(|l| l.name().to_string()).unwrap_or_else(|| "Texte brut".into()));
        let cfg = self.cfg.borrow();
        let v = &d.view;
        self.indent_btn.set_label(&if v.is_insert_spaces_instead_of_tabs() { format!("Espaces : {}", v.tab_width()) } else { format!("Tabulations : {}", v.tab_width()) });
        let _ = &cfg;
        self.enc.set_text(&d.encoding.borrow().as_ref().map(|e| e.charset().to_string()).unwrap_or_else(|| "UTF-8".into()));
        self.eol_btn.set_label(match d.newline.get() {
            sourceview5::NewlineType::CrLf => "CRLF",
            sourceview5::NewlineType::Cr => "CR",
            _ => "LF",
        });
        self.update_count();
    }

    fn update_count(&self) {
        let Some(d) = self.current() else { return };
        if !self.search_rev.reveals_child() || self.find.text().is_empty() {
            self.count.set_text("");
            return;
        }
        let n = d.search.occurrences_count();
        if n < 0 {
            self.count.set_text("…");
            return;
        }
        let at = d.buffer.selection_bounds().map(|(a, b)| d.search.occurrence_position(&a, &b)).unwrap_or(0);
        self.count.set_text(&match (n, at) {
            (0, _) => "Aucun".to_string(),
            (n, a) if a > 0 => format!("{a} sur {n}"),
            (n, _) => format!("{n} trouvé{}", if n > 1 { "s" } else { "" }),
        });
        if n == 0 {
            self.find.add_css_class("error");
        } else {
            self.find.remove_css_class("error");
        }
    }

    fn refresh_recent(self: &Rc<Self>) {
        while let Some(c) = self.recent.first_child() {
            self.recent.remove(&c);
        }
        let list: Vec<String> = self.cfg.borrow().recent.iter().filter(|p| Path::new(p).exists()).take(6).cloned().collect();
        self.recent.set_visible(!list.is_empty());
        for p in list {
            let path = PathBuf::from(&p);
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            r.set_subtitle(&path.parent().map(home_relative).unwrap_or_default());
            r.add_prefix(&gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(&gio::content_type_guess(Some(&path), None).0)));
            r.set_activatable(true);
            let w = Rc::downgrade(self);
            r.connect_activated(move |_| {
                if let Some(w) = w.upgrade() {
                    w.open_path(&path);
                }
            });
            self.recent.append(&r);
        }
    }

    fn fill_languages(self: &Rc<Self>) {
        let lm = sourceview5::LanguageManager::default();
        let mut langs: Vec<sourceview5::Language> = lm.language_ids().iter().filter_map(|id| lm.language(id)).filter(|l| !l.is_hidden()).collect();
        langs.sort_by_key(|l| l.name().to_lowercase());
        let add = |name: &str, section: &str, lang: Option<sourceview5::Language>| {
            let r = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = gtk::Label::new(Some(name));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            h.append(&l);
            let s = gtk::Label::new(Some(section));
            s.add_css_class("dim-label");
            s.add_css_class("caption");
            h.append(&s);
            h.set_margin_top(6);
            h.set_margin_bottom(6);
            h.set_margin_start(8);
            h.set_margin_end(8);
            r.set_child(Some(&h));
            r.set_widget_name(&name.to_lowercase());
            let w = Rc::downgrade(self);
            let gesture = gtk::GestureClick::new();
            gesture.connect_released(move |_, _, _, _| {
                if let Some(w) = w.upgrade() {
                    if let Some(d) = w.current() {
                        d.buffer.set_language(lang.as_ref());
                    }
                    if let Some(p) = w.lang_btn.popover() {
                        p.popdown();
                    }
                }
            });
            r.add_controller(gesture);
            self.lang_list.append(&r);
        };
        add("Texte brut", "", None);
        for l in &langs {
            add(&l.name(), &l.section().to_string(), Some(l.clone()));
        }
        eprintln!("noos-texteditor : {} langages disponibles", langs.len());
    }

    /// Thème de coloration : « noos » suit le mode clair / sombre de NOOS.
    fn scheme(&self) -> Option<sourceview5::StyleScheme> {
        let sm = sourceview5::StyleSchemeManager::default();
        let id = self.cfg.borrow().scheme.clone();
        let dark = adap::StyleManager::default().is_dark();
        let id = if id == "noos" { if dark { "noos-dark".to_string() } else { "noos-light".to_string() } } else { id };
        sm.scheme(&id).or_else(|| sm.scheme(if dark { "Adwaita-dark" } else { "Adwaita" }))
    }

    fn apply_scheme_to(&self, doc: &Doc) {
        doc.buffer.set_style_scheme(self.scheme().as_ref());
    }

    pub fn apply_all(&self) {
        let cfg = self.cfg.borrow().clone();
        for d in self.docs.borrow().iter() {
            d.apply(&cfg);
            self.apply_scheme_to(d);
        }
        self.apply_font();
        self.update_status();
    }

    fn apply_font(&self) {
        let desc = gtk::pango::FontDescription::from_string(&self.cfg.borrow().font);
        let family = desc.family().map(|f| f.to_string()).unwrap_or_else(|| "Monospace".into());
        let size = if desc.size() > 0 { desc.size() as f64 / gtk::pango::SCALE as f64 } else { 11.0 };
        self.css.load_from_string(&format!(
            "textview.noos-code {{ font-family: \"{family}\", monospace; font-size: {:.1}pt; }}\n",
            size * self.zoom.get()
        ));
    }

    // ───────────────────────────── Dossier ─────────────────────────────

    pub fn set_folder(self: &Rc<Self>, dir: &Path) {
        self.cfg.borrow_mut().folder = Some(dir.to_string_lossy().into_owned());
        self.tree_title.set_text(&dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string()));
        self.tree_title.set_tooltip_text(Some(&home_relative(dir)));
        let root = dir_model(&gio::File::for_path(dir));
        let tree = gtk::TreeListModel::new(root, false, false, |item| {
            let info = item.downcast_ref::<gio::FileInfo>()?;
            if info.file_type() != gio::FileType::Directory {
                return None;
            }
            let file = info.attribute_object("standard::file")?.downcast::<gio::File>().ok()?;
            Some(dir_model(&file).upcast())
        });
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let exp = gtk::TreeExpander::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            h.append(&gtk::Image::new());
            let l = gtk::Label::new(None);
            l.set_xalign(0.0);
            l.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            h.append(&l);
            exp.set_child(Some(&h));
            item.set_child(Some(&exp));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(row) = item.item().and_downcast::<gtk::TreeListRow>() else { return };
            let exp = item.child().and_downcast::<gtk::TreeExpander>().unwrap();
            exp.set_list_row(Some(&row));
            let Some(info) = row.item().and_downcast::<gio::FileInfo>() else { return };
            let h = exp.child().and_downcast::<gtk::Box>().unwrap();
            let img = h.first_child().and_downcast::<gtk::Image>().unwrap();
            if let Some(icon) = info.symbolic_icon() {
                img.set_from_gicon(&icon);
            }
            h.last_child().and_downcast::<gtk::Label>().unwrap().set_text(&info.display_name());
        });
        self.tree.set_factory(Some(&factory));
        self.tree.set_model(Some(&gtk::SingleSelection::new(Some(tree))));
        self.split.set_show_sidebar(true);
        self.cfg.borrow_mut().sidebar = true;
        self.update_chrome();
    }

    // ───────────────────────────── Recherche ─────────────────────────────

    fn show_search(&self, replace: bool) {
        self.search_rev.set_reveal_child(true);
        self.replace_rev.set_reveal_child(replace);
        if let Some(d) = self.current() {
            if let Some((a, b)) = d.buffer.selection_bounds() {
                let t = d.buffer.text(&a, &b, false);
                if !t.contains('\n') && !t.is_empty() {
                    self.find.set_text(&t);
                }
            }
        }
        for d in self.docs.borrow().iter() {
            d.search.set_highlight(true);
        }
        self.find.grab_focus();
        self.find.select_region(0, -1);
        self.update_count();
    }

    fn hide_search(&self) {
        self.search_rev.set_reveal_child(false);
        for d in self.docs.borrow().iter() {
            d.search.set_highlight(false);
        }
        if let Some(d) = self.current() {
            d.view.grab_focus();
        }
    }

    fn find_step(&self, forward: bool) {
        let Some(d) = self.current() else { return };
        let (a, b) = d.buffer.selection_bounds().unwrap_or_else(|| {
            let i = d.buffer.iter_at_mark(&d.buffer.get_insert());
            (i, i)
        });
        let res = if forward { d.search.forward(&b) } else { d.search.backward(&a) };
        if let Some((s, e, _)) = res {
            d.buffer.select_range(&s, &e);
            d.view.scroll_to_iter(&mut s.clone(), 0.2, false, 0.0, 0.0);
        }
        self.update_count();
    }

    fn replace_one(&self) {
        let Some(d) = self.current() else { return };
        if let Some((mut a, mut b)) = d.buffer.selection_bounds() {
            if d.search.occurrence_position(&a, &b) > 0 {
                let _ = d.search.replace(&mut a, &mut b, &self.replace.text());
            }
        }
        self.find_step(true);
    }

    fn replace_all(&self) {
        let Some(d) = self.current() else { return };
        let n = d.search.occurrences_count().max(0);
        match d.search.replace_all(&self.replace.text()) {
            Ok(()) => self.toast(&format!("{n} remplacement{}", if n > 1 { "s" } else { "" })),
            Err(e) => self.toast(&format!("Remplacement impossible : {}", e.message())),
        }
    }

    // ───────────────────────────── Fermeture ─────────────────────────────

    /// Demande quoi faire des documents non enregistrés, puis appelle `then` si on continue.
    fn confirm_unsaved(self: &Rc<Self>, docs: Vec<Rc<Doc>>, then: impl Fn() + 'static) {
        let dirty: Vec<Rc<Doc>> = docs.into_iter().filter(|d| d.modified()).collect();
        if dirty.is_empty() {
            return then();
        }
        let names = dirty.iter().map(|d| format!("• {}", d.title())).collect::<Vec<_>>().join("\n");
        let d = adap::AlertDialog::new(
            Some(if dirty.len() > 1 { "Enregistrer les modifications ?" } else { "Enregistrer les modifications ?" }),
            Some(&format!("{names}\n\nSans enregistrement, les modifications seront perdues.")),
        );
        d.add_responses(&[("cancel", "Annuler"), ("discard", "Ne pas enregistrer"), ("save", "Enregistrer")]);
        d.set_response_appearance("discard", adap::ResponseAppearance::Destructive);
        d.set_response_appearance("save", adap::ResponseAppearance::Suggested);
        d.set_default_response(Some("save"));
        d.set_close_response("cancel");
        let w = self.clone();
        let then = Rc::new(then);
        d.connect_response(None, move |_, r| match r {
            "discard" => {
                for doc in &dirty {
                    doc.buffer.set_modified(false);
                }
                then()
            }
            "save" => {
                let left = Rc::new(Cell::new(dirty.len()));
                let ok = Rc::new(Cell::new(true));
                for doc in &dirty {
                    let (left, ok, then) = (left.clone(), ok.clone(), then.clone());
                    w.save(doc, move |saved| {
                        ok.set(ok.get() && saved);
                        left.set(left.get() - 1);
                        if left.get() == 0 && ok.get() {
                            then();
                        }
                    });
                }
            }
            _ => {}
        });
        d.present(Some(&self.window));
    }

    fn save_session(&self) {
        let mut cfg = self.cfg.borrow_mut();
        cfg.session = self.docs.borrow().iter().filter_map(|d| d.path()).map(|p| p.to_string_lossy().into_owned()).collect();
        cfg.width = self.window.default_width();
        cfg.height = self.window.default_height();
        cfg.sidebar = self.split.shows_sidebar();
        cfg.save();
    }

    pub fn restore_session(self: &Rc<Self>) {
        let (restore, files) = {
            let c = self.cfg.borrow();
            (c.restore_session, c.session.clone())
        };
        if restore {
            for f in files.iter().filter(|f| Path::new(f).is_file()) {
                self.open_path(Path::new(f));
            }
        }
    }

    // ───────────────────────────── Actions ─────────────────────────────

    fn setup_actions(self: &Rc<Self>) {
        let act = |name: &str, f: Box<dyn Fn(&Rc<Self>)>| {
            let a = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            a.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    f(&w);
                }
            });
            self.window.add_action(&a);
        };
        let doc_act = |name: &str, f: fn(&Doc)| {
            act(
                name,
                Box::new(move |w: &Rc<Self>| {
                    if let Some(d) = w.current() {
                        f(&d)
                    }
                }),
            );
        };
        act("new", Box::new(|w| {
            w.new_doc();
        }));
        act("open", Box::new(|w| w.choose_files()));
        act("open-folder", Box::new(|w| w.choose_folder()));
        act("save", Box::new(|w| {
            if let Some(d) = w.current() {
                w.save(&d, |_| {});
            }
        }));
        act("save-as", Box::new(|w| {
            if let Some(d) = w.current() {
                w.save_as(&d, |_| {});
            }
        }));
        act("save-all", Box::new(|w| {
            for d in w.docs.borrow().clone().iter().filter(|d| d.modified()) {
                w.save(d, |_| {});
            }
        }));
        act("close-tab", Box::new(|w| {
            if let Some(p) = w.tabs.selected_page() {
                w.tabs.close_page(&p);
            }
        }));
        act("find", Box::new(|w| w.show_search(false)));
        act("replace", Box::new(|w| w.show_search(true)));
        act("find-next", Box::new(|w| w.find_step(true)));
        act("find-prev", Box::new(|w| w.find_step(false)));
        act("replace-one", Box::new(|w| w.replace_one()));
        act("replace-all", Box::new(|w| w.replace_all()));
        act("goto", Box::new(|w| w.ask_goto()));
        act("toggle-sidebar", Box::new(|w| {
            let show = !w.split.shows_sidebar();
            if show && w.cfg.borrow().folder.is_none() {
                return w.choose_folder();
            }
            w.split.set_show_sidebar(show);
        }));
        act("zoom-in", Box::new(|w| {
            w.zoom.set((w.zoom.get() + 0.1).min(3.0));
            w.apply_font();
        }));
        act("zoom-out", Box::new(|w| {
            w.zoom.set((w.zoom.get() - 0.1).max(0.5));
            w.apply_font();
        }));
        act("zoom-reset", Box::new(|w| {
            w.zoom.set(1.0);
            w.apply_font();
        }));
        act("prefs", Box::new(|w| crate::prefs::show(w)));
        act("shortcuts", Box::new(|w| w.show_shortcuts()));
        for (name, field) in [("wrap", 0), ("minimap", 1), ("whitespace", 2)] {
            act(name, Box::new(move |w| {
                {
                    let mut c = w.cfg.borrow_mut();
                    let v = match field {
                        0 => &mut c.wrap,
                        1 => &mut c.minimap,
                        _ => &mut c.whitespace,
                    };
                    *v = !*v;
                    c.save();
                }
                w.apply_all();
            }));
        }
        doc_act("comment", |d| d.toggle_comment());
        doc_act("duplicate", |d| d.duplicate_lines());
        doc_act("delete-line", |d| d.delete_lines());
        doc_act("sort", |d| d.sort_lines());
        doc_act("trim", |d| d.trim_trailing());
        doc_act("upper", |d| d.change_case(sourceview5::ChangeCaseType::Upper));
        doc_act("lower", |d| d.change_case(sourceview5::ChangeCaseType::Lower));
        doc_act("toggle-case", |d| d.change_case(sourceview5::ChangeCaseType::Toggle));
        doc_act("title-case", |d| d.change_case(sourceview5::ChangeCaseType::Title));
    }

    fn setup_signals(self: &Rc<Self>, lang_search: &gtk::SearchEntry, close_search: &gtk::Button) {
        // Changement d'onglet.
        let w = Rc::downgrade(self);
        self.tabs.connect_selected_page_notify(move |_| {
            if let Some(w) = w.upgrade() {
                w.update_chrome();
            }
        });
        // Fermeture d'un onglet : proposer d'enregistrer (Annuler / Ne pas enregistrer / Enregistrer).
        let w = Rc::downgrade(self);
        self.tabs.connect_close_page(move |tabs, page| {
            let Some(w) = w.upgrade() else { return glib::Propagation::Proceed };
            let child = page.child();
            let doc = w.docs.borrow().iter().find(|d| d.root.upcast_ref::<gtk::Widget>() == &child).cloned();
            let finish = {
                let (tabs, page, w, doc) = (tabs.clone(), page.clone(), w.clone(), doc.clone());
                move |close: bool| {
                    tabs.close_page_finish(&page, close);
                    if !close {
                        return;
                    }
                    if let Some(doc) = &doc {
                        w.docs.borrow_mut().retain(|d| !Rc::ptr_eq(d, doc));
                    }
                    if w.docs.borrow().is_empty() {
                        w.stack.set_visible_child_name("welcome");
                        w.refresh_recent();
                    }
                    w.update_chrome();
                }
            };
            let Some(doc) = doc.filter(|d| d.modified()) else {
                finish(true);
                return glib::Propagation::Stop;
            };
            let d = adap::AlertDialog::new(
                Some(&format!("Enregistrer « {} » ?", doc.title())),
                Some("Sans enregistrement, les modifications seront perdues."),
            );
            d.add_responses(&[("cancel", "Annuler"), ("discard", "Ne pas enregistrer"), ("save", "Enregistrer")]);
            d.set_response_appearance("discard", adap::ResponseAppearance::Destructive);
            d.set_response_appearance("save", adap::ResponseAppearance::Suggested);
            d.set_default_response(Some("save"));
            d.set_close_response("cancel");
            let w2 = w.clone();
            d.connect_response(None, move |_, r| match r {
                "discard" => finish(true),
                "save" => {
                    let finish = finish.clone();
                    w2.save(&doc, move |ok| finish(ok));
                }
                _ => finish(false),
            });
            d.present(Some(&w.window));
            glib::Propagation::Stop
        });
        // Fermeture de la fenêtre.
        let w = Rc::downgrade(self);
        self.window.connect_close_request(move |win| {
            let Some(w) = w.upgrade() else { return glib::Propagation::Proceed };
            if w.docs.borrow().iter().all(|d| !d.modified()) {
                w.save_session();
                return glib::Propagation::Proceed;
            }
            let (w2, win2) = (w.clone(), win.clone());
            w.confirm_unsaved(w.docs.borrow().clone(), move || {
                w2.save_session();
                for d in w2.docs.borrow().iter() {
                    d.buffer.set_modified(false);
                }
                win2.destroy();
            });
            glib::Propagation::Stop
        });
        // Recherche.
        let w = Rc::downgrade(self);
        self.find.connect_search_changed(move |e| {
            if let Some(w) = w.upgrade() {
                w.settings.set_search_text(Some(&e.text()).filter(|t| !t.is_empty()).map(|t| t.as_str()));
                w.update_count();
            }
        });
        let w = Rc::downgrade(self);
        self.find.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.find_step(true);
            }
        });
        let w = Rc::downgrade(self);
        self.find.connect_stop_search(move |_| {
            if let Some(w) = w.upgrade() {
                w.hide_search();
            }
        });
        let w = Rc::downgrade(self);
        close_search.connect_clicked(move |_| {
            if let Some(w) = w.upgrade() {
                w.hide_search();
            }
        });
        let w = Rc::downgrade(self);
        self.replace.connect_activate(move |_| {
            if let Some(w) = w.upgrade() {
                w.replace_one();
            }
        });
        // Filtre des langages.
        let list = self.lang_list.clone();
        lang_search.connect_search_changed(move |s| {
            let q = s.text().to_lowercase();
            let mut row = list.first_child();
            while let Some(r) = row {
                r.set_visible(q.is_empty() || r.widget_name().contains(q.as_str()));
                row = r.next_sibling();
            }
        });
        // Indentation et fins de ligne (menus de la barre d'état).
        self.indent_btn.set_popover(Some(&self.indent_popover()));
        self.eol_btn.set_popover(Some(&self.eol_popover()));
        // Panneau : ouvrir un fichier.
        let w = Rc::downgrade(self);
        self.tree.connect_activate(move |list, pos| {
            let Some(w) = w.upgrade() else { return };
            let Some(row) = list.model().and_then(|m| m.item(pos)).and_downcast::<gtk::TreeListRow>() else { return };
            let Some(info) = row.item().and_downcast::<gio::FileInfo>() else { return };
            if info.file_type() == gio::FileType::Directory {
                row.set_expanded(!row.is_expanded());
                return;
            }
            if let Some(path) = info.attribute_object("standard::file").and_downcast::<gio::File>().and_then(|f| f.path()) {
                w.open_path(&path);
            }
        });
        // Glisser-déposer de fichiers sur la fenêtre.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let w = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(w), Ok(list)) = (w.upgrade(), value.get::<gdk::FileList>()) else { return false };
            for f in list.files() {
                if let Some(p) = f.path() {
                    w.open_path(&p);
                }
            }
            true
        });
        self.window.add_controller(drop);
        // Mode clair / sombre : thème « noos ».
        let w = Rc::downgrade(self);
        adap::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(w) = w.upgrade() {
                w.apply_all();
            }
        });
        // Molette + Ctrl : zoom.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = Rc::downgrade(self);
        scroll.connect_scroll(move |c, _, dy| {
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            if let Some(w) = w.upgrade() {
                w.zoom.set((w.zoom.get() - dy * 0.1).clamp(0.5, 3.0));
                w.apply_font();
            }
            glib::Propagation::Stop
        });
        self.tabs.add_controller(scroll);
    }

    fn indent_popover(self: &Rc<Self>) -> gtk::Popover {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
        b.set_margin_top(6);
        b.set_margin_bottom(6);
        b.set_margin_start(6);
        b.set_margin_end(6);
        let spaces = gtk::CheckButton::with_label("Espaces");
        let tabs = gtk::CheckButton::with_label("Tabulations");
        tabs.set_group(Some(&spaces));
        let cfg = self.cfg.borrow().clone();
        spaces.set_active(cfg.spaces);
        tabs.set_active(!cfg.spaces);
        b.append(&spaces);
        b.append(&tabs);
        let width = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        width.add_css_class("linked");
        let mut first: Option<gtk::ToggleButton> = None;
        for n in [2u32, 4, 8] {
            let t = gtk::ToggleButton::with_label(&n.to_string());
            if let Some(f) = &first {
                t.set_group(Some(f));
            } else {
                first = Some(t.clone());
            }
            t.set_active(cfg.tab_width == n);
            let w = Rc::downgrade(self);
            t.connect_toggled(move |t| {
                if let (true, Some(w)) = (t.is_active(), w.upgrade()) {
                    if let Some(d) = w.current() {
                        d.view.set_tab_width(n);
                    }
                    w.update_status();
                }
            });
            width.append(&t);
        }
        b.append(&gtk::Label::new(Some("Largeur")));
        b.append(&width);
        let w = Rc::downgrade(self);
        spaces.connect_toggled(move |s| {
            if let Some(w) = w.upgrade() {
                if let Some(d) = w.current() {
                    d.view.set_insert_spaces_instead_of_tabs(s.is_active());
                }
                w.update_status();
            }
        });
        let p = gtk::Popover::new();
        p.set_child(Some(&b));
        p
    }

    fn eol_popover(self: &Rc<Self>) -> gtk::Popover {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let p = gtk::Popover::new();
        for (label, nl) in [("LF — Linux, macOS", sourceview5::NewlineType::Lf), ("CRLF — Windows", sourceview5::NewlineType::CrLf), ("CR — ancien Mac", sourceview5::NewlineType::Cr)] {
            let btn = gtk::Button::with_label(label);
            btn.add_css_class("flat");
            let (w, p2) = (Rc::downgrade(self), p.clone());
            btn.connect_clicked(move |_| {
                p2.popdown();
                if let Some(w) = w.upgrade() {
                    if let Some(d) = w.current() {
                        d.newline.set(nl);
                        d.buffer.set_modified(true);
                    }
                    w.update_status();
                }
            });
            b.append(&btn);
        }
        p.set_child(Some(&b));
        p
    }

    fn choose_files(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Ouvrir des fichiers").modal(true).build();
        if let Some(dir) = self.current().and_then(|d| d.path()).and_then(|p| p.parent().map(Path::to_path_buf)).or_else(|| self.cfg.borrow().folder.clone().map(PathBuf::from)) {
            dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
        }
        let w = self.clone();
        dialog.open_multiple(Some(&self.window), gio::Cancellable::NONE, move |res| {
            let Ok(files) = res else { return };
            for i in 0..files.n_items() {
                if let Some(p) = files.item(i).and_downcast::<gio::File>().and_then(|f| f.path()) {
                    w.open_path(&p);
                }
            }
        });
    }

    fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Ouvrir un dossier").modal(true).build();
        let w = self.clone();
        dialog.select_folder(Some(&self.window), gio::Cancellable::NONE, move |res| {
            if let Some(p) = res.ok().and_then(|f| f.path()) {
                w.set_folder(&p);
            }
        });
    }

    fn ask_goto(self: &Rc<Self>) {
        let Some(d) = self.current() else { return };
        let total = d.buffer.line_count();
        let entry = gtk::Entry::new();
        entry.set_input_purpose(gtk::InputPurpose::Digits);
        entry.set_placeholder_text(Some(&format!("1 à {total}")));
        entry.set_activates_default(true);
        let dialog = adap::AlertDialog::new(Some("Aller à la ligne"), None);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Annuler"), ("go", "Aller")]);
        dialog.set_response_appearance("go", adap::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("go"));
        let e2 = entry.clone();
        dialog.connect_response(None, move |_, r| {
            if r == "go" {
                if let Ok(n) = e2.text().trim().parse::<i32>() {
                    d.goto_line(n);
                }
            }
        });
        dialog.present(Some(&self.window));
        glib::idle_add_local_once(move || {
            entry.grab_focus();
        });
    }

    fn show_shortcuts(&self) {
        let rows = [
            ("Ctrl+N / Ctrl+O / Ctrl+S", "Nouveau, ouvrir, enregistrer"),
            ("Ctrl+Maj+S", "Enregistrer sous"),
            ("Ctrl+W", "Fermer l'onglet"),
            ("Ctrl+F / Ctrl+H", "Rechercher / remplacer"),
            ("Entrée / Maj+Entrée, F3", "Occurrence suivante / précédente"),
            ("Ctrl+L", "Aller à la ligne"),
            ("Ctrl+/", "Commenter / décommenter"),
            ("Ctrl+D", "Dupliquer la ligne"),
            ("Ctrl+Maj+K", "Supprimer la ligne"),
            ("Alt+↑ / Alt+↓", "Déplacer la ligne"),
            ("Ctrl+Espace", "Autocomplétion"),
            ("Ctrl++ / Ctrl+- / Ctrl+0", "Zoom"),
            ("F9", "Panneau du dossier"),
        ];
        let g = adap::PreferencesGroup::new();
        for (k, d) in rows {
            let r = adap::ActionRow::new();
            r.set_title(d);
            let l = gtk::Label::new(Some(k));
            l.add_css_class("dim-label");
            l.add_css_class("monospace");
            r.add_suffix(&l);
            g.add(&r);
        }
        let dialog = adap::Dialog::new();
        dialog.set_title("Raccourcis clavier");
        dialog.set_content_width(520);
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        g.set_margin_start(18);
        g.set_margin_end(18);
        g.set_margin_bottom(18);
        tv.set_content(Some(&g));
        dialog.set_child(Some(&tv));
        dialog.present(Some(&self.window));
    }
}

/// Contenu d'un dossier, dossiers d'abord puis par nom, sans les fichiers cachés.
fn dir_model(dir: &gio::File) -> gio::ListModel {
    let list = gtk::DirectoryList::new(Some("standard::name,standard::display-name,standard::symbolic-icon,standard::type,standard::is-hidden"), Some(dir));
    let filter = gtk::CustomFilter::new(|o| {
        let info = o.downcast_ref::<gio::FileInfo>().unwrap();
        !info.is_hidden() && !info.name().to_string_lossy().starts_with('.')
    });
    let filtered = gtk::FilterListModel::new(Some(list), Some(filter));
    let sorter = gtk::CustomSorter::new(|a, b| {
        let (a, b) = (a.downcast_ref::<gio::FileInfo>().unwrap(), b.downcast_ref::<gio::FileInfo>().unwrap());
        let (da, db) = (a.file_type() == gio::FileType::Directory, b.file_type() == gio::FileType::Directory);
        db.cmp(&da).then_with(|| a.display_name().to_lowercase().cmp(&b.display_name().to_lowercase())).into()
    });
    gtk::SortListModel::new(Some(filtered), Some(sorter)).upcast()
}

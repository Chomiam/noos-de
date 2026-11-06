//! noos-keys — Porte-clés de NOOS (GTK 4 + libAdapta).
//!
//! - **Mots de passe** : trousseaux du Secret Service (oo7 sur Fedora, gnome-keyring,
//!   KeePassXC…) : recherche, affichage, copie (effacée du presse-papiers après 30 s),
//!   ajout, modification, suppression, verrouillage / déverrouillage, nouveaux trousseaux.
//! - **Clés SSH** : `~/.ssh`, empreintes, copie de la clé publique, génération.

mod secret;
mod ssh;

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use secret::{Collection, Item, Secrets};

const APP_ID: &str = "org.noos.Keys";
const CLEAR_AFTER: u64 = 30;

struct App {
    window: adap::ApplicationWindow,
    toasts: adap::ToastOverlay,
    secrets: RefCell<Option<Secrets>>,
    collections: RefCell<Vec<Collection>>,
    sidebar: gtk::ListBox,
    split: adap::NavigationSplitView,
    content: gtk::Stack,
    // Page « trousseau »
    kr_title: adap::WindowTitle,
    kr_lock: gtk::Button,
    kr_menu: gtk::MenuButton,
    kr_add: gtk::Button,
    kr_stack: gtk::Stack,
    kr_list: gtk::ListBox,
    search: gtk::SearchEntry,
    current: RefCell<Option<Collection>>,
    items: RefCell<Vec<Item>>,
    // Page « clés SSH »
    ssh_list: gtk::ListBox,
    error_page: adap::StatusPage,
}

fn spawn<F: std::future::Future<Output = ()> + 'static>(f: F) {
    glib::spawn_future_local(f);
}

fn date(ts: u64) -> String {
    if ts == 0 {
        return "—".into();
    }
    glib::DateTime::from_unix_local(ts as i64)
        .ok()
        .and_then(|d| d.format("%d/%m/%Y à %H:%M").ok())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

impl App {
    fn new(app: &adap::Application) -> Rc<Self> {
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Porte-clés")
            .default_width(1000)
            .default_height(660)
            .width_request(360)
            .height_request(420)
            .build();
        let toasts = adap::ToastOverlay::new();

        // ── Barre latérale ──
        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let side_scroll = gtk::ScrolledWindow::new();
        side_scroll.set_vexpand(true);
        side_scroll.set_child(Some(&sidebar));
        let side_header = adap::HeaderBar::new();
        side_header.set_title_widget(Some(&adap::WindowTitle::new("Porte-clés", "")));
        let new_menu = gio::Menu::new();
        new_menu.append(Some("Nouveau mot de passe"), Some("win.new-password"));
        new_menu.append(Some("Nouveau trousseau"), Some("win.new-keyring"));
        new_menu.append(Some("Nouvelle clé SSH"), Some("win.new-ssh"));
        let new_btn = gtk::MenuButton::new();
        new_btn.set_icon_name("list-add-symbolic");
        new_btn.set_tooltip_text(Some("Créer"));
        new_btn.set_menu_model(Some(&new_menu));
        side_header.pack_start(&new_btn);
        let side_view = adap::ToolbarView::new();
        side_view.add_top_bar(&side_header);
        side_view.set_content(Some(&side_scroll));
        let side_page = adap::NavigationPage::new(&side_view, "Porte-clés");

        // ── Page trousseau ──
        let kr_title = adap::WindowTitle::new("Mots de passe", "");
        let kr_header = adap::HeaderBar::new();
        kr_header.set_title_widget(Some(&kr_title));
        let kr_add = gtk::Button::from_icon_name("list-add-symbolic");
        kr_add.set_tooltip_text(Some("Nouveau mot de passe"));
        kr_add.set_action_name(Some("win.new-password"));
        kr_header.pack_start(&kr_add);
        let kr_menu = gtk::MenuButton::new();
        kr_menu.set_icon_name("view-more-symbolic");
        let menu = gio::Menu::new();
        menu.append(Some("Utiliser par défaut"), Some("win.keyring-default"));
        menu.append(Some("Supprimer le trousseau…"), Some("win.keyring-delete"));
        kr_menu.set_menu_model(Some(&menu));
        kr_header.pack_end(&kr_menu);
        let kr_lock = gtk::Button::from_icon_name("changes-prevent-symbolic");
        kr_header.pack_end(&kr_lock);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Rechercher un mot de passe"));
        search.set_hexpand(true);
        let search_clamp = adap::Clamp::new();
        search_clamp.set_maximum_size(720);
        search_clamp.set_child(Some(&search));
        search_clamp.set_margin_top(12);
        search_clamp.set_margin_start(12);
        search_clamp.set_margin_end(12);

        let kr_list = gtk::ListBox::new();
        kr_list.add_css_class("boxed-list");
        kr_list.set_selection_mode(gtk::SelectionMode::None);
        kr_list.set_valign(gtk::Align::Start);
        let list_clamp = adap::Clamp::new();
        list_clamp.set_maximum_size(720);
        list_clamp.set_child(Some(&kr_list));
        list_clamp.set_margin_top(12);
        list_clamp.set_margin_bottom(24);
        list_clamp.set_margin_start(12);
        list_clamp.set_margin_end(12);
        let list_scroll = gtk::ScrolledWindow::new();
        list_scroll.set_vexpand(true);
        list_scroll.set_child(Some(&list_clamp));
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_box.append(&search_clamp);
        list_box.append(&list_scroll);

        let locked_page = adap::StatusPage::builder()
            .icon_name("changes-prevent-symbolic")
            .title("Trousseau verrouillé")
            .description("Déverrouillez-le avec son mot de passe pour voir et gérer ses éléments.")
            .build();
        let unlock_btn = gtk::Button::with_label("Déverrouiller");
        unlock_btn.add_css_class("pill");
        unlock_btn.add_css_class("suggested-action");
        unlock_btn.set_halign(gtk::Align::Center);
        unlock_btn.set_action_name(Some("win.keyring-unlock"));
        locked_page.set_child(Some(&unlock_btn));
        let empty_page = adap::StatusPage::builder()
            .icon_name("dialog-password-symbolic")
            .title("Aucun mot de passe")
            .description("Les mots de passe enregistrés par vos applications apparaîtront ici.")
            .build();
        let kr_stack = gtk::Stack::new();
        kr_stack.add_named(&list_box, Some("list"));
        kr_stack.add_named(&locked_page, Some("locked"));
        kr_stack.add_named(&empty_page, Some("empty"));
        let spinner = gtk::Spinner::new();
        spinner.start();
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        kr_stack.add_named(&spinner, Some("loading"));
        let kr_view = adap::ToolbarView::new();
        kr_view.add_top_bar(&kr_header);
        kr_view.set_content(Some(&kr_stack));

        // ── Page SSH ──
        let ssh_header = adap::HeaderBar::new();
        ssh_header.set_title_widget(Some(&adap::WindowTitle::new("Clés SSH", &ssh::dir().to_string_lossy())));
        let generate = gtk::Button::with_label("Générer");
        generate.add_css_class("suggested-action");
        generate.set_action_name(Some("win.new-ssh"));
        ssh_header.pack_start(&generate);
        let ssh_list = gtk::ListBox::new();
        ssh_list.add_css_class("boxed-list");
        ssh_list.set_selection_mode(gtk::SelectionMode::None);
        ssh_list.set_valign(gtk::Align::Start);
        let ssh_clamp = adap::Clamp::new();
        ssh_clamp.set_maximum_size(720);
        ssh_clamp.set_child(Some(&ssh_list));
        ssh_clamp.set_margin_top(18);
        ssh_clamp.set_margin_start(12);
        ssh_clamp.set_margin_end(12);
        let ssh_scroll = gtk::ScrolledWindow::new();
        ssh_scroll.set_vexpand(true);
        ssh_scroll.set_child(Some(&ssh_clamp));
        let ssh_view = adap::ToolbarView::new();
        ssh_view.add_top_bar(&ssh_header);
        ssh_view.set_content(Some(&ssh_scroll));

        let error_page = adap::StatusPage::builder().icon_name("dialog-warning-symbolic").title("Service de secrets indisponible").build();
        let error_view = adap::ToolbarView::new();
        error_view.add_top_bar(&adap::HeaderBar::new());
        error_view.set_content(Some(&error_page));

        let content = gtk::Stack::new();
        content.set_transition_type(gtk::StackTransitionType::Crossfade);
        content.add_named(&kr_view, Some("keyring"));
        content.add_named(&ssh_view, Some("ssh"));
        content.add_named(&error_view, Some("error"));
        toasts.set_child(Some(&content));
        let content_page = adap::NavigationPage::new(&toasts, "Porte-clés");

        let split = adap::NavigationSplitView::new();
        split.set_sidebar(Some(&side_page));
        split.set_content(Some(&content_page));
        split.set_min_sidebar_width(220.0);
        window.set_content(Some(&split));
        let bp = adap::Breakpoint::new(adap::BreakpointCondition::new_length(adap::BreakpointConditionLengthType::MaxWidth, 640.0, adap::LengthUnit::Sp));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let a = Rc::new(Self {
            window,
            toasts,
            secrets: RefCell::new(None),
            collections: RefCell::new(Vec::new()),
            sidebar,
            split,
            content,
            kr_title,
            kr_lock,
            kr_menu,
            kr_add,
            kr_stack,
            kr_list,
            search,
            current: RefCell::new(None),
            items: RefCell::new(Vec::new()),
            ssh_list,
            error_page,
        });
        a.setup(app);
        a
    }

    fn toast(&self, text: &str) {
        self.toasts.add_toast(adap::Toast::new(text));
    }

    fn secrets(&self) -> Option<Secrets> {
        self.secrets.borrow().clone()
    }

    // ───────────── Chargement ─────────────

    fn start(self: &Rc<Self>) {
        let a = self.clone();
        spawn(async move {
            match Secrets::connect().await {
                Ok(s) => {
                    *a.secrets.borrow_mut() = Some(s);
                    a.reload(None).await;
                }
                Err(e) => {
                    a.error_page.set_description(Some(&format!(
                        "{e:#}\n\nLes mots de passe sont gérés par le service de secrets de la session (oo7 ou gnome-keyring). Les clés SSH restent accessibles."
                    )));
                    a.fill_sidebar();
                    a.content.set_visible_child_name("error");
                }
            }
        });
    }

    /// Recharge la liste des trousseaux, puis réaffiche `select` (ou le trousseau courant).
    async fn reload(self: &Rc<Self>, select: Option<String>) {
        let Some(s) = self.secrets() else { return };
        match s.collections().await {
            Ok(c) => *self.collections.borrow_mut() = c,
            Err(e) => self.toast(&format!("{e:#}")),
        }
        let keep = select.or_else(|| self.current.borrow().as_ref().map(|c| c.path.clone()));
        self.fill_sidebar();
        let idx = keep
            .and_then(|p| self.collections.borrow().iter().position(|c| c.path == p))
            .unwrap_or(0);
        if self.content.visible_child_name().as_deref() == Some("ssh") {
            return;
        }
        if let Some(row) = self.sidebar.row_at_index(idx as i32 + 1) {
            self.sidebar.select_row(Some(&row));
        }
    }

    fn fill_sidebar(self: &Rc<Self>) {
        self.sidebar.remove_all();
        let section = |title: &str| {
            let l = gtk::Label::new(Some(title));
            l.add_css_class("heading");
            l.add_css_class("dim-label");
            l.set_xalign(0.0);
            l.set_margin_top(10);
            l.set_margin_start(6);
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&l));
            row.set_selectable(false);
            row.set_activatable(false);
            row
        };
        self.sidebar.append(&section("Mots de passe"));
        for c in self.collections.borrow().iter() {
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            h.append(&gtk::Image::from_icon_name(if c.path.ends_with("/session") { "document-open-recent-symbolic" } else { "dialog-password-symbolic" }));
            let label = if c.path.ends_with("/session") { "Session (temporaire)".to_string() } else { c.label.clone() };
            let l = gtk::Label::new(Some(&label));
            l.set_xalign(0.0);
            l.set_hexpand(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::End);
            h.append(&l);
            if c.is_default {
                let d = gtk::Image::from_icon_name("starred-symbolic");
                d.set_tooltip_text(Some("Trousseau par défaut"));
                d.add_css_class("dim-label");
                h.append(&d);
            }
            if c.locked {
                let lock = gtk::Image::from_icon_name("changes-prevent-symbolic");
                lock.set_tooltip_text(Some("Verrouillé"));
                h.append(&lock);
            }
            let row = gtk::ListBoxRow::new();
            row.set_child(Some(&h));
            self.sidebar.append(&row);
        }
        self.sidebar.append(&section("Clés"));
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        h.append(&gtk::Image::from_icon_name("network-server-symbolic"));
        h.append(&gtk::Label::new(Some("Clés SSH")));
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&h));
        self.sidebar.append(&row);
    }

    fn on_select(self: &Rc<Self>, index: i32) {
        let n = self.collections.borrow().len() as i32;
        if index >= 1 && index <= n {
            let c = self.collections.borrow()[(index - 1) as usize].clone();
            self.show_collection(c);
        } else if index == n + 2 {
            *self.current.borrow_mut() = None;
            self.content.set_visible_child_name("ssh");
            self.fill_ssh();
        }
        self.split.set_show_content(true);
    }

    fn show_collection(self: &Rc<Self>, c: Collection) {
        self.content.set_visible_child_name("keyring");
        let title = if c.path.ends_with("/session") { "Session (temporaire)".to_string() } else { c.label.clone() };
        self.kr_title.set_title(&title);
        self.kr_title.set_subtitle(&if c.locked { "Verrouillé".to_string() } else { format!("{} élément(s)", c.items.len()) });
        self.kr_lock.set_icon_name(if c.locked { "changes-allow-symbolic" } else { "changes-prevent-symbolic" });
        self.kr_lock.set_tooltip_text(Some(if c.locked { "Déverrouiller" } else { "Verrouiller" }));
        self.kr_add.set_sensitive(!c.locked);
        self.kr_menu.set_sensitive(!c.path.ends_with("/session"));
        *self.current.borrow_mut() = Some(c.clone());
        if c.locked {
            self.kr_stack.set_visible_child_name("locked");
            return;
        }
        self.kr_stack.set_visible_child_name("loading");
        let (a, s) = (self.clone(), self.secrets());
        spawn(async move {
            let Some(s) = s else { return };
            let items = s.items(&c).await;
            if a.current.borrow().as_ref().map(|x| &x.path) != Some(&c.path) {
                return;
            }
            *a.items.borrow_mut() = items;
            a.fill_items();
        });
    }

    fn fill_items(self: &Rc<Self>) {
        self.kr_list.remove_all();
        let q = self.search.text().to_lowercase();
        let items = self.items.borrow().clone();
        let matching: Vec<&Item> = items
            .iter()
            .filter(|i| {
                q.is_empty()
                    || i.label.to_lowercase().contains(&q)
                    || i.attributes.values().any(|v| v.to_lowercase().contains(&q))
            })
            .collect();
        if items.is_empty() {
            self.kr_stack.set_visible_child_name("empty");
            return;
        }
        self.kr_stack.set_visible_child_name("list");
        for item in matching {
            let (icon, sub) = secret::describe(item);
            let row = adap::ActionRow::new();
            row.set_title(&glib::markup_escape_text(if item.label.is_empty() { "(sans nom)" } else { &item.label }));
            row.set_subtitle(&glib::markup_escape_text(&sub));
            row.set_subtitle_lines(1);
            row.add_prefix(&gtk::Image::from_icon_name(icon));
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.add_css_class("flat");
            copy.set_valign(gtk::Align::Center);
            copy.set_tooltip_text(Some("Copier le mot de passe"));
            {
                let (a, path) = (self.clone(), item.path.clone());
                copy.connect_clicked(move |_| {
                    let (a, path) = (a.clone(), path.clone());
                    spawn(async move {
                        let Some(s) = a.secrets() else { return };
                        match s.secret(&path).await {
                            Ok(secret) => a.copy_secret(&secret),
                            Err(e) => a.toast(&format!("{e:#}")),
                        }
                    });
                });
            }
            row.add_suffix(&copy);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.set_activatable(true);
            {
                let (a, item) = (self.clone(), item.clone());
                row.connect_activated(move |_| a.item_dialog(&item));
            }
            self.kr_list.append(&row);
        }
    }

    /// Copie un secret ; il est effacé du presse-papiers 30 s plus tard (s'il y est encore).
    fn copy_secret(&self, secret: &str) {
        let clipboard = WidgetExt::display(&self.window).clipboard();
        clipboard.set_text(secret);
        self.toast(&format!("Copié — effacé du presse-papiers dans {CLEAR_AFTER} s"));
        glib::timeout_add_seconds_local_once(CLEAR_AFTER as u32, move || {
            if clipboard.is_local() {
                clipboard.set_content(None::<&gdk::ContentProvider>).ok();
            }
        });
    }

    // ───────────── Détail d'un élément ─────────────

    fn item_dialog(self: &Rc<Self>, item: &Item) {
        let dialog = adap::Dialog::new();
        dialog.set_title(if item.label.is_empty() { "Mot de passe" } else { &item.label });
        dialog.set_content_width(520);
        let header = adap::HeaderBar::new();
        let save = gtk::Button::with_label("Enregistrer");
        save.add_css_class("suggested-action");
        save.set_sensitive(false);
        header.pack_end(&save);
        let page = adap::PreferencesPage::new();

        let main = adap::PreferencesGroup::new();
        let label = adap::EntryRow::new();
        label.set_title("Nom");
        label.set_text(&item.label);
        main.add(&label);
        let password = adap::PasswordEntryRow::new();
        password.set_title("Mot de passe");
        password.set_sensitive(false);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.add_css_class("flat");
        copy.set_valign(gtk::Align::Center);
        copy.set_tooltip_text(Some("Copier"));
        password.add_suffix(&copy);
        main.add(&password);
        page.add(&main);

        if !item.attributes.is_empty() {
            let details = adap::PreferencesGroup::new();
            details.set_title("Détails");
            let mut attrs: Vec<(&String, &String)> = item.attributes.iter().collect();
            attrs.sort();
            for (k, v) in attrs {
                let row = adap::ActionRow::new();
                row.set_title(&glib::markup_escape_text(k));
                row.set_subtitle(&glib::markup_escape_text(v));
                row.set_subtitle_selectable(true);
                row.add_css_class("property");
                details.add(&row);
            }
            page.add(&details);
        }
        let dates = adap::PreferencesGroup::new();
        for (t, v) in [("Créé le", item.created), ("Modifié le", item.modified)] {
            let row = adap::ActionRow::new();
            row.set_title(t);
            row.set_subtitle(&date(v));
            row.add_css_class("property");
            dates.add(&row);
        }
        page.add(&dates);

        let danger = adap::PreferencesGroup::new();
        let delete = gtk::Button::with_label("Supprimer ce mot de passe");
        delete.add_css_class("destructive-action");
        delete.add_css_class("pill");
        delete.set_halign(gtk::Align::Center);
        danger.add(&delete);
        page.add(&danger);

        let view = adap::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        dialog.set_child(Some(&view));

        // Chargement du secret.
        let original = Rc::new(RefCell::new(String::new()));
        {
            let (a, path, password, original) = (self.clone(), item.path.clone(), password.clone(), original.clone());
            spawn(async move {
                let Some(s) = a.secrets() else { return };
                match s.secret(&path).await {
                    Ok(v) => {
                        password.set_text(&v);
                        *original.borrow_mut() = v;
                        password.set_sensitive(true);
                    }
                    Err(e) => a.toast(&format!("{e:#}")),
                }
            });
        }
        let changed = {
            let (label, password, original, save, orig_label) = (label.clone(), password.clone(), original.clone(), save.clone(), item.label.clone());
            move || save.set_sensitive(label.text() != orig_label || password.text() != *original.borrow())
        };
        let changed = Rc::new(changed);
        {
            let c = changed.clone();
            label.connect_changed(move |_| c());
        }
        {
            let c = changed.clone();
            password.connect_changed(move |_| c());
        }
        {
            let (a, password) = (self.clone(), password.clone());
            copy.connect_clicked(move |_| a.copy_secret(&password.text()));
        }
        {
            let (a, item, dialog2) = (self.clone(), item.clone(), dialog.clone());
            let (label, password, original) = (label.clone(), password.clone(), original.clone());
            save.connect_clicked(move |_| {
                let (a, item, dialog2) = (a.clone(), item.clone(), dialog2.clone());
                let (new_label, new_pw, old_pw) = (label.text().to_string(), password.text().to_string(), original.borrow().clone());
                spawn(async move {
                    let Some(s) = a.secrets() else { return };
                    let mut res = Ok(());
                    if new_label != item.label {
                        res = s.set_label(&item.path, &new_label).await;
                    }
                    if res.is_ok() && new_pw != old_pw {
                        res = s.set_secret(&item.path, &new_pw).await;
                    }
                    match res {
                        Ok(()) => {
                            a.toast("Modifications enregistrées");
                            dialog2.close();
                            a.reload(None).await;
                        }
                        Err(e) => a.toast(&format!("{e:#}")),
                    }
                });
            });
        }
        {
            let (a, item, dialog2) = (self.clone(), item.clone(), dialog.clone());
            delete.connect_clicked(move |_| {
                let confirm = adap::AlertDialog::new(
                    Some("Supprimer ce mot de passe ?"),
                    Some(&format!("« {} » sera définitivement supprimé du trousseau. Les applications qui l'utilisent vous le redemanderont.", item.label)),
                );
                confirm.add_response("cancel", "Annuler");
                confirm.add_response("delete", "Supprimer");
                confirm.set_response_appearance("delete", adap::ResponseAppearance::Destructive);
                confirm.set_close_response("cancel");
                let (a, item, dialog2) = (a.clone(), item.clone(), dialog2.clone());
                confirm.choose(Some(&a.window.clone()), gio::Cancellable::NONE, move |r| {
                    if r != "delete" {
                        return;
                    }
                    spawn(async move {
                        let Some(s) = a.secrets() else { return };
                        match s.delete_item(&item.path).await {
                            Ok(()) => {
                                dialog2.close();
                                a.toast("Mot de passe supprimé");
                                a.reload(None).await;
                            }
                            Err(e) => a.toast(&format!("{e:#}")),
                        }
                    });
                });
            });
        }
        dialog.present(Some(&self.window));
    }

    // ───────────── Création ─────────────

    fn new_password(self: &Rc<Self>) {
        let Some(c) = self.current.borrow().clone().or_else(|| self.collections.borrow().iter().find(|c| c.is_default).cloned()) else {
            self.toast("Aucun trousseau disponible");
            return;
        };
        let dialog = adap::AlertDialog::new(Some("Nouveau mot de passe"), Some(&format!("Dans le trousseau « {} »", c.label)));
        let group = adap::PreferencesGroup::new();
        let name = adap::EntryRow::new();
        name.set_title("Nom (ex. Banque, NAS…)");
        let user = adap::EntryRow::new();
        user.set_title("Identifiant (facultatif)");
        let site = adap::EntryRow::new();
        site.set_title("Site ou serveur (facultatif)");
        let pw = adap::PasswordEntryRow::new();
        pw.set_title("Mot de passe");
        for r in [name.upcast_ref::<gtk::Widget>(), user.upcast_ref(), site.upcast_ref(), pw.upcast_ref()] {
            group.add(r);
        }
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Annuler");
        dialog.add_response("save", "Enregistrer");
        dialog.set_response_appearance("save", adap::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let a = self.clone();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if r != "save" || pw.text().is_empty() {
                return;
            }
            let label = if name.text().is_empty() { site.text().to_string() } else { name.text().to_string() };
            let mut attrs: HashMap<String, String> = HashMap::new();
            attrs.insert("xdg:schema".into(), "org.noos.Keys.Password".into());
            if !user.text().is_empty() {
                attrs.insert("username".into(), user.text().to_string());
            }
            if !site.text().is_empty() {
                attrs.insert("server".into(), site.text().to_string());
            }
            let (a, value, c) = (a.clone(), pw.text().to_string(), c.clone());
            spawn(async move {
                let Some(s) = a.secrets() else { return };
                if c.locked {
                    let _ = s.unlock(&[c.path.clone()]).await;
                }
                match s.create_item(&c.path, &label, attrs, &value).await {
                    Ok(()) => {
                        a.toast("Mot de passe enregistré");
                        a.reload(Some(c.path.clone())).await;
                    }
                    Err(e) => a.toast(&format!("{e:#}")),
                }
            });
        });
    }

    fn new_keyring(self: &Rc<Self>) {
        let dialog = adap::AlertDialog::new(
            Some("Nouveau trousseau"),
            Some("Le service de secrets vous demandera ensuite le mot de passe qui le protégera."),
        );
        let group = adap::PreferencesGroup::new();
        let name = adap::EntryRow::new();
        name.set_title("Nom du trousseau");
        group.add(&name);
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Annuler");
        dialog.add_response("create", "Créer");
        dialog.set_response_appearance("create", adap::ResponseAppearance::Suggested);
        dialog.set_close_response("cancel");
        let a = self.clone();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if r != "create" || name.text().trim().is_empty() {
                return;
            }
            let (a, label) = (a.clone(), name.text().trim().to_string());
            spawn(async move {
                let Some(s) = a.secrets() else { return };
                match s.create_collection(&label).await {
                    Ok(()) => {
                        a.toast(&format!("Trousseau « {label} » créé"));
                        a.reload(None).await;
                    }
                    Err(e) => a.toast(&format!("{e:#}")),
                }
            });
        });
    }

    // ───────────── Clés SSH ─────────────

    fn fill_ssh(self: &Rc<Self>) {
        self.ssh_list.remove_all();
        let keys = ssh::list();
        if keys.is_empty() {
            let row = adap::ActionRow::new();
            row.set_title("Aucune clé SSH");
            row.set_subtitle("Générez-en une pour vous connecter à des serveurs ou à GitHub/GitLab sans mot de passe");
            self.ssh_list.append(&row);
        }
        for key in keys {
            let row = adap::ActionRow::new();
            let name = key.public.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            row.set_title(&glib::markup_escape_text(if key.comment.is_empty() { &name } else { &key.comment }));
            row.set_subtitle(&glib::markup_escape_text(&format!(
                "{} · {} · {}{}",
                key.kind,
                name,
                key.fingerprint,
                if key.has_private { "" } else { " · clé publique seule" }
            )));
            row.set_subtitle_lines(2);
            row.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
            let copy = gtk::Button::with_label("Copier la clé publique");
            copy.add_css_class("flat");
            copy.set_valign(gtk::Align::Center);
            {
                let (a, key) = (self.clone(), key.clone());
                copy.connect_clicked(move |_| {
                    WidgetExt::display(&a.window).clipboard().set_text(&ssh::public_key(&key));
                    a.toast("Clé publique copiée : collez-la dans GitHub, GitLab ou ~/.ssh/authorized_keys");
                });
            }
            row.add_suffix(&copy);
            let del = gtk::Button::from_icon_name("user-trash-symbolic");
            del.add_css_class("flat");
            del.set_valign(gtk::Align::Center);
            del.set_tooltip_text(Some("Supprimer la clé"));
            {
                let (a, key, name) = (self.clone(), key.clone(), name.clone());
                del.connect_clicked(move |_| {
                    let confirm = adap::AlertDialog::new(
                        Some("Supprimer la clé SSH ?"),
                        Some(&format!(
                            "« {name} » et sa clé publique seront supprimées de ~/.ssh. Vous ne pourrez plus vous connecter aux serveurs qui n'acceptent que cette clé."
                        )),
                    );
                    confirm.add_response("cancel", "Annuler");
                    confirm.add_response("delete", "Supprimer");
                    confirm.set_response_appearance("delete", adap::ResponseAppearance::Destructive);
                    confirm.set_close_response("cancel");
                    let (a2, key) = (a.clone(), key.clone());
                    confirm.choose(Some(&a.window), gio::Cancellable::NONE, move |r| {
                        if r == "delete" {
                            match ssh::delete(&key) {
                                Ok(()) => a2.toast("Clé supprimée"),
                                Err(e) => a2.toast(&format!("{e:#}")),
                            }
                            a2.fill_ssh();
                        }
                    });
                });
            }
            row.add_suffix(&del);
            self.ssh_list.append(&row);
        }
    }

    fn new_ssh(self: &Rc<Self>) {
        let dialog = adap::AlertDialog::new(Some("Nouvelle clé SSH"), Some("Une paire de clés est créée dans ~/.ssh."));
        let group = adap::PreferencesGroup::new();
        let kind = adap::ComboRow::new();
        kind.set_title("Type");
        kind.set_model(Some(&gtk::StringList::new(&["Ed25519 (recommandé)", "RSA 4096 bits"])));
        let default_name = (1..)
            .map(|i| if i == 1 { "id_ed25519".to_string() } else { format!("id_ed25519_{i}") })
            .find(|n| !ssh::dir().join(n).exists())
            .unwrap();
        let name = adap::EntryRow::new();
        name.set_title("Nom du fichier");
        name.set_text(&default_name);
        let comment = adap::EntryRow::new();
        comment.set_title("Commentaire");
        comment.set_text(&ssh::default_comment());
        let pass = adap::PasswordEntryRow::new();
        pass.set_title("Phrase secrète (facultative, recommandée)");
        for r in [kind.upcast_ref::<gtk::Widget>(), name.upcast_ref(), comment.upcast_ref(), pass.upcast_ref()] {
            group.add(r);
        }
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Annuler");
        dialog.add_response("create", "Générer");
        dialog.set_response_appearance("create", adap::ResponseAppearance::Suggested);
        dialog.set_close_response("cancel");
        let a = self.clone();
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |r| {
            if r != "create" {
                return;
            }
            let t = if kind.selected() == 1 { "rsa" } else { "ed25519" };
            match ssh::generate(t, &name.text(), &comment.text(), &pass.text()) {
                Ok(p) => a.toast(&format!("Clé créée : {}", p.display())),
                Err(e) => a.toast(&format!("{e:#}")),
            }
            if let Some(row) = a.sidebar.row_at_index(a.collections.borrow().len() as i32 + 2) {
                a.sidebar.select_row(Some(&row));
            }
            a.fill_ssh();
        });
    }

    // ───────────── Actions ─────────────

    fn setup(self: &Rc<Self>, app: &adap::Application) {
        {
            let a = Rc::downgrade(self);
            self.sidebar.connect_row_selected(move |_, row| {
                if let (Some(a), Some(row)) = (a.upgrade(), row) {
                    a.on_select(row.index());
                }
            });
        }
        {
            let a = Rc::downgrade(self);
            self.search.connect_search_changed(move |_| {
                if let Some(a) = a.upgrade() {
                    a.fill_items();
                }
            });
        }
        {
            let a = Rc::downgrade(self);
            self.kr_lock.connect_clicked(move |_| {
                let Some(a) = a.upgrade() else { return };
                let Some(c) = a.current.borrow().clone() else { return };
                let a2 = a.clone();
                spawn(async move {
                    let Some(s) = a2.secrets() else { return };
                    let res = if c.locked { s.unlock(&[c.path.clone()]).await.map(|_| ()) } else { s.lock(&[c.path.clone()]).await };
                    if let Err(e) = res {
                        a2.toast(&format!("{e:#}"));
                    }
                    a2.reload(Some(c.path.clone())).await;
                });
            });
        }
        let add = |name: &str, accels: &[&str], f: Box<dyn Fn(&Rc<App>)>| {
            let action = gio::SimpleAction::new(name, None);
            let a = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(a) = a.upgrade() {
                    f(&a);
                }
            });
            self.window.add_action(&action);
            if !accels.is_empty() {
                app.set_accels_for_action(&format!("win.{name}"), accels);
            }
        };
        add("new-password", &["<Control>n"], Box::new(|a| a.new_password()));
        add("new-keyring", &[], Box::new(|a| a.new_keyring()));
        add("new-ssh", &[], Box::new(|a| a.new_ssh()));
        add("find", &["<Control>f"], Box::new(|a| {
            a.search.grab_focus();
        }));
        add("keyring-unlock", &[], Box::new(|a| a.kr_lock.emit_clicked()));
        add("keyring-default", &[], Box::new(|a| {
            let Some(c) = a.current.borrow().clone() else { return };
            let a = a.clone();
            spawn(async move {
                let Some(s) = a.secrets() else { return };
                match s.set_default(&c.path).await {
                    Ok(()) => a.toast(&format!("« {} » est maintenant le trousseau par défaut", c.label)),
                    Err(e) => a.toast(&format!("{e:#}")),
                }
                a.reload(Some(c.path)).await;
            });
        }));
        add("keyring-delete", &[], Box::new(|a| {
            let Some(c) = a.current.borrow().clone() else { return };
            let confirm = adap::AlertDialog::new(
                Some(&format!("Supprimer le trousseau « {} » ?", c.label)),
                Some(&format!(
                    "Ses {} élément(s) seront définitivement supprimés. Les applications qui y gardaient leurs mots de passe les perdront.",
                    c.items.len()
                )),
            );
            confirm.add_response("cancel", "Annuler");
            confirm.add_response("delete", "Supprimer le trousseau");
            confirm.set_response_appearance("delete", adap::ResponseAppearance::Destructive);
            confirm.set_close_response("cancel");
            let a2 = a.clone();
            confirm.choose(Some(&a.window), gio::Cancellable::NONE, move |r| {
                if r != "delete" {
                    return;
                }
                let (a, c) = (a2.clone(), c.clone());
                spawn(async move {
                    let Some(s) = a.secrets() else { return };
                    match s.delete_collection(&c.path).await {
                        Ok(()) => a.toast("Trousseau supprimé"),
                        Err(e) => a.toast(&format!("{e:#}")),
                    }
                    *a.current.borrow_mut() = None;
                    a.reload(None).await;
                });
            });
        }));
    }
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let a = App::new(app);
        a.window.present();
        a.start();
        // L'état de l'application vit aussi longtemps que sa fenêtre.
        let keep = RefCell::new(Some(a.clone()));
        a.window.connect_destroy(move |_| {
            keep.borrow_mut().take();
        });
    });
    app.run()
}

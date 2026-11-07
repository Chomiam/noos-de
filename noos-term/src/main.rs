//! noos-term — terminal de NOOS (GTK 4 + libAdapta + VTE).
//!
//! Onglets dans la barre de titre, menu du clic droit (copier / coller / tout sélectionner),
//! copie à la sélection, taille du texte (Ctrl + / Ctrl − / Ctrl 0), thèmes de couleurs,
//! et une barre latérale d'alias : des commandes enregistrées (ex. « claude ») lancées
//! d'un clic dans l'onglet actif ou un nouvel onglet.
//!
//! `noos-term [--working-directory DIR] [-e COMMANDE…]`

#![allow(deprecated)] // titre de fenêtre / dossier courant : API VTE < 0.78 encore valables.

mod config;
mod themes;

use std::{
    cell::RefCell,
    rc::Rc,
};

use adap::prelude::*;
use config::{Alias, TermConfig};
use gtk::{gdk, gio, glib, pango};
use vte4::prelude::*;

const APP_ID: &str = "org.noos.Terminal";
const URL_REGEX: &str = r#"(https?|ftp|file)://[^\s<>"'`()\[\]{}]+"#;
/// PCRE2_MULTILINE (requis par VTE pour les expressions de correspondance).
const PCRE2_MULTILINE: u32 = 0x0000_0400;

struct State {
    cfg: RefCell<TermConfig>,
    /// Fenêtres ouvertes (référence forte : retirée à la fermeture de la fenêtre).
    windows: RefCell<Vec<Rc<Win>>>,
}

thread_local! {
    static STATE: Rc<State> = Rc::new(State { cfg: RefCell::new(TermConfig::load()), windows: RefCell::new(Vec::new()) });
}

fn state() -> Rc<State> {
    STATE.with(|s| s.clone())
}

fn cfg() -> TermConfig {
    state().cfg.borrow().clone()
}

/// Modifie les réglages, les enregistre et les applique à toutes les fenêtres.
fn update_cfg(f: impl FnOnce(&mut TermConfig)) {
    let st = state();
    f(&mut st.cfg.borrow_mut());
    st.cfg.borrow().save();
    apply_all();
}

fn windows() -> Vec<Rc<Win>> {
    state().windows.borrow().clone()
}

fn apply_all() {
    for w in windows() {
        w.apply();
    }
}

fn rgba(hex: &str, alpha: f64) -> gdk::RGBA {
    let mut c = gdk::RGBA::parse(hex).unwrap_or(gdk::RGBA::BLACK);
    c.set_alpha(alpha as f32);
    c
}

fn apply_terminal(term: &vte4::Terminal, cfg: &TermConfig) {
    let font = pango::FontDescription::from_string(&format!("{} {}", cfg.font_family, cfg.font_size));
    term.set_font(Some(&font));
    let dark = adap::StyleManager::default().is_dark();
    let theme = themes::resolve(&cfg.theme, dark);
    let (mut fg, mut bg, mut cursor) = (theme.fg.to_string(), theme.bg.to_string(), theme.cursor.to_string());
    if cfg.theme == "noos" {
        // Thème « NOOS » : fond, texte et curseur pris dans la palette de Personnalisation
        // (ex. Nuit saphir) ; seules les 16 couleurs ANSI restent celles de NOOS sombre/clair.
        let app = noos_core::NoosConfig::load().appearance;
        let v = noos_theme::palette::variant(&app, dark);
        fg = v.window_fg.to_string();
        bg = v.window_bg.to_string();
        cursor = if app.accent.is_empty() { noos_theme::palette::palette(&app.palette).accent.to_string() } else { app.accent };
    }
    let palette: Vec<gdk::RGBA> = theme.palette.iter().map(|c| rgba(c, 1.0)).collect();
    let refs: Vec<&gdk::RGBA> = palette.iter().collect();
    term.set_colors(Some(&rgba(&fg, 1.0)), Some(&rgba(&bg, cfg.opacity.clamp(0.5, 1.0))), &refs);
    term.set_color_cursor(Some(&rgba(&cursor, 1.0)));
    term.set_scrollback_lines(cfg.scrollback_lines as _);
    term.set_cursor_blink_mode(if cfg.cursor_blink { vte4::CursorBlinkMode::On } else { vte4::CursorBlinkMode::Off });
    term.set_bold_is_bright(true);
    term.set_audible_bell(false);
}

/// Terminal d'une page d'onglet (la page contient un ScrolledWindow).
fn page_terminal(page: &adap::TabPage) -> Option<vte4::Terminal> {
    page.child().downcast::<gtk::ScrolledWindow>().ok()?.child()?.downcast::<vte4::Terminal>().ok()
}

struct Win {
    window: adap::ApplicationWindow,
    tabs: adap::TabView,
    split: adap::OverlaySplitView,
    aliases: gtk::ListBox,
    toasts: adap::ToastOverlay,
}

impl Win {
    fn new(app: &adap::Application) -> Rc<Self> {
        let cfg = cfg();
        let window = adap::ApplicationWindow::builder()
            .application(app)
            .title("Terminal")
            .default_width(1000)
            .default_height(640)
            .build();
        window.add_css_class("noos-term");

        let tabs = adap::TabView::new();
        let tab_bar = adap::TabBar::new();
        tab_bar.set_view(Some(&tabs));
        tab_bar.set_autohide(false);
        tab_bar.set_hexpand(true);
        // Onglets à leur largeur naturelle : un onglet capture le clic-glisser (réordonner),
        // étirés sur toute la barre il ne resterait rien pour déplacer la fenêtre. L'espace
        // vide restant sert de poignée (glisser = déplacer, double-clic = agrandir).
        tab_bar.set_expand_tabs(false);

        let header = adap::HeaderBar::new();
        header.set_title_widget(Some(&tab_bar));
        let sidebar_btn = gtk::ToggleButton::new();
        sidebar_btn.set_icon_name("sidebar-show-symbolic");
        sidebar_btn.set_tooltip_text(Some("Alias (Ctrl+Maj+B)"));
        sidebar_btn.set_action_name(Some("win.toggle-sidebar"));
        header.pack_start(&sidebar_btn);
        let menu_btn = gtk::MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_tooltip_text(Some("Menu principal"));
        menu_btn.set_menu_model(Some(&main_menu()));
        header.pack_end(&menu_btn);
        let new_tab = gtk::Button::from_icon_name("tab-new-symbolic");
        new_tab.set_tooltip_text(Some("Nouvel onglet (Ctrl+Maj+T)"));
        new_tab.set_action_name(Some("win.new-tab"));
        header.pack_end(&new_tab);

        let aliases = gtk::ListBox::new();
        aliases.add_css_class("navigation-sidebar");
        aliases.set_selection_mode(gtk::SelectionMode::None);
        let split = adap::OverlaySplitView::new();
        split.set_sidebar(Some(&sidebar(&aliases)));
        split.set_sidebar_width_fraction(0.22);
        split.set_min_sidebar_width(200.0);
        split.set_max_sidebar_width(300.0);
        let toasts = adap::ToastOverlay::new();
        toasts.set_child(Some(&tabs));
        split.set_content(Some(&toasts));
        split.set_show_sidebar(cfg.sidebar);

        let view = adap::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&split));
        window.set_content(Some(&view));

        let win = Rc::new(Self { window, tabs, split, aliases, toasts });
        state().windows.borrow_mut().push(win.clone());
        {
            let w = Rc::downgrade(&win);
            win.window.connect_destroy(move |_| {
                if let Some(w) = w.upgrade() {
                    state().windows.borrow_mut().retain(|x| !Rc::ptr_eq(x, &w));
                }
            });
        }
        win.setup_actions(app);

        // Dernier onglet fermé → la fenêtre se ferme.
        {
            let w = Rc::downgrade(&win);
            win.tabs.connect_n_pages_notify(move |tv| {
                if tv.n_pages() == 0 {
                    if let Some(w) = w.upgrade() {
                        w.window.close();
                    }
                }
            });
        }
        {
            let w = Rc::downgrade(&win);
            win.tabs.connect_selected_page_notify(move |tv| {
                if let (Some(w), Some(page)) = (w.upgrade(), tv.selected_page()) {
                    w.window.set_title(Some(&page.title()));
                    if let Some(t) = page_terminal(&page) {
                        t.grab_focus();
                    }
                }
            });
        }
        win.apply();
        win
    }

    fn current(&self) -> Option<vte4::Terminal> {
        self.tabs.selected_page().and_then(|p| page_terminal(&p))
    }

    fn apply(self: &Rc<Self>) {
        let cfg = cfg();
        for i in 0..self.tabs.n_pages() {
            if let Some(t) = page_terminal(&self.tabs.nth_page(i)) {
                apply_terminal(&t, &cfg);
            }
        }
        self.split.set_show_sidebar(cfg.sidebar);
        if let Some(a) = self.window.lookup_action("toggle-sidebar") {
            a.change_state(&cfg.sidebar.to_variant());
        }
        if cfg.opacity < 0.99 {
            self.window.add_css_class("translucent");
        } else {
            self.window.remove_css_class("translucent");
        }
        self.fill_aliases(&cfg);
    }

    // ───────────── Onglets ─────────────

    /// Nouvel onglet : le shell de l'utilisateur, puis `command` s'il y en a une
    /// (tapée dans le shell : on retrouve l'invite à la fin de la commande).
    fn new_tab(self: &Rc<Self>, cwd: Option<String>, command: Option<String>) {
        let term = vte4::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        apply_terminal(&term, &cfg());
        if let Ok(regex) = vte4::Regex::for_match(URL_REGEX, PCRE2_MULTILINE) {
            let tag = term.match_add_regex(&regex, 0);
            term.match_set_cursor_name(tag, "pointer");
        }

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_child(Some(&term));
        let page = self.tabs.append(&scroll);
        page.set_title("Terminal");
        page.set_icon(Some(&gio::ThemedIcon::new("utilities-terminal-symbolic")));
        self.tabs.set_selected_page(&page);

        {
            let (page, win) = (page.clone(), Rc::downgrade(self));
            term.connect_window_title_changed(move |t| {
                let title = t.window_title().map(|s| s.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "Terminal".into());
                page.set_title(&title);
                page.set_tooltip(&title);
                if let Some(w) = win.upgrade() {
                    if w.tabs.selected_page().as_ref() == Some(&page) {
                        w.window.set_title(Some(&title));
                    }
                }
            });
        }
        {
            let (page, tabs) = (page.clone(), self.tabs.clone());
            term.connect_child_exited(move |_, _| tabs.close_page(&page));
        }
        term.connect_selection_changed(|t| {
            if cfg().copy_on_select && t.has_selection() {
                t.copy_clipboard_format(vte4::Format::Text);
            }
        });
        self.add_mouse(&term);

        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/bash".into());
        let term2 = term.clone();
        let shell2 = shell.clone();
        term.spawn_async(
            vte4::PtyFlags::DEFAULT,
            cwd.as_deref(),
            &[&shell],
            &["TERM_PROGRAM=noos-term"],
            glib::SpawnFlags::SEARCH_PATH,
            || {},
            -1,
            gio::Cancellable::NONE,
            move |res| match res {
                Ok(_) => {
                    if let Some(cmd) = command {
                        term2.feed_child(format!("{cmd}\n").as_bytes());
                    }
                }
                Err(e) => term2.feed(format!("\r\nImpossible de lancer {shell2} : {e}\r\n").as_bytes()),
            },
        );
        term.grab_focus();
    }

    /// Clic droit : menu presse-papiers ; Ctrl+clic : ouvrir le lien sous le pointeur.
    fn add_mouse(self: &Rc<Self>, term: &vte4::Terminal) {
        let right = gtk::GestureClick::new();
        right.set_button(3);
        right.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let w = Rc::downgrade(self);
            let term = term.clone();
            right.connect_pressed(move |g, _, x, y| {
                g.set_state(gtk::EventSequenceState::Claimed);
                let Some(w) = w.upgrade() else { return };
                if let Some(a) = w.window.lookup_action("copy").and_downcast::<gio::SimpleAction>() {
                    a.set_enabled(term.has_selection());
                }
                let (link, _) = term.check_match_at(x, y);
                let menu = context_menu(link.is_some());
                *LINK.with(|l| l.clone()).borrow_mut() = link.map(|s| s.to_string());
                let pop = gtk::PopoverMenu::from_model(Some(&menu));
                pop.set_parent(&term);
                pop.set_has_arrow(false);
                pop.set_halign(gtk::Align::Start);
                pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.connect_closed(|p| {
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.unparent());
                });
                noos_theme::fit_popover(&pop);
                pop.popup();
            });
        }
        term.add_controller(right);

        let left = gtk::GestureClick::new();
        left.set_button(1);
        {
            let (w, term) = (Rc::downgrade(self), term.clone());
            left.connect_released(move |g, _, x, y| {
                if !g.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                    return;
                }
                if let (Some(w), (Some(uri), _)) = (w.upgrade(), term.check_match_at(x, y)) {
                    gtk::UriLauncher::new(&uri).launch(Some(&w.window), gio::Cancellable::NONE, |_| {});
                }
            });
        }
        term.add_controller(left);
    }

    // ───────────── Alias ─────────────

    fn fill_aliases(self: &Rc<Self>, cfg: &TermConfig) {
        self.aliases.remove_all();
        for (i, alias) in cfg.aliases.iter().enumerate() {
            let row = gtk::ListBoxRow::new();
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            h.set_margin_top(4);
            h.set_margin_bottom(4);
            let icon = gtk::Image::from_icon_name(if alias.new_tab { "tab-new-symbolic" } else { "media-playback-start-symbolic" });
            h.append(&icon);
            let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
            v.set_hexpand(true);
            let name = gtk::Label::new(Some(&alias.name));
            name.set_xalign(0.0);
            name.set_ellipsize(pango::EllipsizeMode::End);
            v.append(&name);
            let cmd = gtk::Label::new(Some(&alias.command));
            cmd.set_xalign(0.0);
            cmd.add_css_class("dim-label");
            cmd.add_css_class("caption");
            cmd.add_css_class("monospace");
            cmd.set_ellipsize(pango::EllipsizeMode::End);
            v.append(&cmd);
            h.append(&v);
            let more = gtk::MenuButton::new();
            more.set_icon_name("view-more-symbolic");
            more.add_css_class("flat");
            more.set_valign(gtk::Align::Center);
            more.set_menu_model(Some(&alias_menu(i as i32)));
            h.append(&more);
            row.set_child(Some(&h));
            row.set_tooltip_text(Some(&format!("{}\n{}", alias.command, if alias.new_tab { "Nouvel onglet" } else { "Onglet actif" })));
            self.aliases.append(&row);
        }
    }

    fn run_alias(self: &Rc<Self>, index: usize, force_new_tab: Option<bool>) {
        let Some(alias) = cfg().aliases.get(index).cloned() else { return };
        let new_tab = force_new_tab.unwrap_or(alias.new_tab);
        match self.current().filter(|_| !new_tab) {
            Some(term) => {
                term.feed_child(format!("{}\n", alias.command).as_bytes());
                term.grab_focus();
            }
            None => {
                let cwd = self.current_dir();
                self.new_tab(cwd, Some(alias.command.clone()));
                if let Some(page) = self.tabs.selected_page() {
                    page.set_title(&alias.name);
                }
            }
        }
    }

    fn current_dir(&self) -> Option<String> {
        let uri = self.current()?.current_directory_uri()?;
        gio::File::for_uri(&uri).path().map(|p| p.to_string_lossy().to_string())
    }

    /// Ajout (index None) ou modification d'un alias.
    fn edit_alias(self: &Rc<Self>, index: Option<usize>) {
        let existing = index.and_then(|i| cfg().aliases.get(i).cloned()).unwrap_or(Alias { new_tab: true, ..Default::default() });
        let dialog = adap::AlertDialog::new(
            Some(if index.is_some() { "Modifier l'alias" } else { "Nouvel alias" }),
            Some("Une commande enregistrée, lancée d'un clic depuis la barre latérale."),
        );
        let group = adap::PreferencesGroup::new();
        let name = adap::EntryRow::new();
        name.set_title("Nom");
        name.set_text(&existing.name);
        let command = adap::EntryRow::new();
        command.set_title("Commande (ex. claude)");
        command.set_text(&existing.command);
        command.add_css_class("monospace");
        let new_tab = adap::SwitchRow::new();
        new_tab.set_title("Ouvrir dans un nouvel onglet");
        new_tab.set_active(existing.new_tab);
        group.add(&name);
        group.add(&command);
        group.add(&new_tab);
        dialog.set_extra_child(Some(&group));
        dialog.add_response("cancel", "Annuler");
        dialog.add_response("save", "Enregistrer");
        dialog.set_response_appearance("save", adap::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        dialog.choose(Some(&self.window), gio::Cancellable::NONE, move |resp| {
            if resp != "save" {
                return;
            }
            let alias = Alias {
                name: name.text().trim().to_string(),
                command: command.text().trim().to_string(),
                new_tab: new_tab.is_active(),
            };
            if alias.command.is_empty() {
                return;
            }
            let alias = Alias { name: if alias.name.is_empty() { alias.command.clone() } else { alias.name }, ..alias };
            update_cfg(|c| match index {
                Some(i) if i < c.aliases.len() => c.aliases[i] = alias,
                _ => c.aliases.push(alias),
            });
        });
    }

    // ───────────── Actions et raccourcis ─────────────

    fn setup_actions(self: &Rc<Self>, app: &adap::Application) {
        let simple = |name: &str, accels: &[&str], f: Box<dyn Fn(&Rc<Win>)>| {
            let action = gio::SimpleAction::new(name, None);
            let w = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(w) = w.upgrade() {
                    f(&w);
                }
            });
            self.window.add_action(&action);
            if !accels.is_empty() {
                app.set_accels_for_action(&format!("win.{name}"), accels);
            }
        };
        simple("new-tab", &["<Control><Shift>t"], Box::new(|w| w.new_tab(w.current_dir(), None)));
        simple("close-tab", &["<Control><Shift>w"], Box::new(|w| {
            if let Some(p) = w.tabs.selected_page() {
                w.tabs.close_page(&p);
            }
        }));
        simple("new-window", &["<Control><Shift>n"], Box::new(|w| {
            if let Some(app) = w.window.application().and_downcast::<adap::Application>() {
                let nw = Win::new(&app);
                nw.new_tab(w.current_dir(), None);
                nw.window.present();
            }
        }));
        simple("copy", &["<Control><Shift>c"], Box::new(|w| {
            if let Some(t) = w.current() {
                t.copy_clipboard_format(vte4::Format::Text);
            }
        }));
        simple("paste", &["<Control><Shift>v", "<Shift>Insert"], Box::new(|w| {
            if let Some(t) = w.current() {
                t.paste_clipboard();
            }
        }));
        simple("select-all", &["<Control><Shift>a"], Box::new(|w| {
            if let Some(t) = w.current() {
                t.select_all();
            }
        }));
        simple("copy-link", &[], Box::new(|w| {
            if let Some(link) = LINK.with(|l| l.borrow().clone()) {
                gtk::prelude::WidgetExt::display(&w.window).clipboard().set_text(&link);
            }
        }));
        simple("open-link", &[], Box::new(|w| {
            if let Some(link) = LINK.with(|l| l.borrow().clone()) {
                gtk::UriLauncher::new(&link).launch(Some(&w.window), gio::Cancellable::NONE, |_| {});
            }
        }));
        simple("zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"], Box::new(|_| {
            update_cfg(|c| c.font_size = (c.font_size + 1.0).min(48.0));
        }));
        simple("zoom-out", &["<Control>minus", "<Control>KP_Subtract"], Box::new(|_| {
            update_cfg(|c| c.font_size = (c.font_size - 1.0).max(6.0));
        }));
        simple("zoom-reset", &["<Control>0", "<Control>KP_0"], Box::new(|_| {
            update_cfg(|c| c.font_size = TermConfig::default().font_size);
        }));
        simple("fullscreen", &["F11"], Box::new(|w| w.window.set_fullscreened(!w.window.is_fullscreen())));
        simple("add-alias", &[], Box::new(|w| w.edit_alias(None)));
        simple("preferences", &["<Control>comma"], Box::new(|w| preferences(&w.window)));
        simple("about", &[], Box::new(|w| about(&w.window)));

        // Barre latérale (état coché).
        let sidebar = gio::SimpleAction::new_stateful("toggle-sidebar", None, &cfg().sidebar.to_variant());
        sidebar.connect_activate(|a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            update_cfg(|c| c.sidebar = on);
        });
        self.window.add_action(&sidebar);
        app.set_accels_for_action("win.toggle-sidebar", &["<Control><Shift>b"]);

        // Thème (choix unique).
        let theme = gio::SimpleAction::new_stateful("theme", Some(glib::VariantTy::STRING), &cfg().theme.to_variant());
        theme.connect_activate(|a, p| {
            if let Some(id) = p.and_then(|v| v.get::<String>()) {
                a.set_state(&id.to_variant());
                update_cfg(|c| c.theme = id);
            }
        });
        self.window.add_action(&theme);

        // Copie à la sélection.
        let cos = gio::SimpleAction::new_stateful("copy-on-select", None, &cfg().copy_on_select.to_variant());
        cos.connect_activate(|a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(false);
            a.set_state(&on.to_variant());
            update_cfg(|c| c.copy_on_select = on);
        });
        self.window.add_action(&cos);

        // Alias : actions avec l'index en paramètre.
        for (name, kind) in [("alias-run", 0), ("alias-run-tab", 1), ("alias-run-here", 2), ("alias-edit", 3), ("alias-delete", 4)] {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::INT32));
            let w = Rc::downgrade(self);
            action.connect_activate(move |_, p| {
                let (Some(w), Some(i)) = (w.upgrade(), p.and_then(|v| v.get::<i32>())) else { return };
                let i = i.max(0) as usize;
                match kind {
                    0 => w.run_alias(i, None),
                    1 => w.run_alias(i, Some(true)),
                    2 => w.run_alias(i, Some(false)),
                    3 => w.edit_alias(Some(i)),
                    _ => {
                        let name = cfg().aliases.get(i).map(|a| a.name.clone()).unwrap_or_default();
                        update_cfg(|c| {
                            if i < c.aliases.len() {
                                c.aliases.remove(i);
                            }
                        });
                        w.toasts.add_toast(adap::Toast::new(&format!("Alias « {name} » supprimé")));
                    }
                }
            });
            self.window.add_action(&action);
        }
        // Clic sur une ligne de la barre latérale : lancer l'alias.
        {
            let w = Rc::downgrade(self);
            self.aliases.connect_row_activated(move |_, row| {
                if let Some(w) = w.upgrade() {
                    w.run_alias(row.index().max(0) as usize, None);
                }
            });
        }
    }
}

thread_local! {
    /// Lien sous le pointeur au moment du clic droit (menu « Ouvrir / copier le lien »).
    static LINK: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
}

fn sidebar(list: &gtk::ListBox) -> gtk::Widget {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
    v.add_css_class("alias-sidebar");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    head.set_margin_start(12);
    head.set_margin_end(6);
    head.set_margin_top(8);
    head.set_margin_bottom(4);
    let title = gtk::Label::new(Some("Alias"));
    title.add_css_class("heading");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    head.append(&title);
    let add = gtk::Button::from_icon_name("list-add-symbolic");
    add.add_css_class("flat");
    add.set_tooltip_text(Some("Nouvel alias"));
    add.set_action_name(Some("win.add-alias"));
    head.append(&add);
    v.append(&head);
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroll.set_child(Some(list));
    v.append(&scroll);
    let hint = gtk::Label::new(Some("Clic : lancer · ⋯ : options"));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_margin_bottom(8);
    hint.set_margin_top(4);
    v.append(&hint);
    v.upcast()
}

fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    s1.append(Some("Nouvel onglet"), Some("win.new-tab"));
    s1.append(Some("Nouvelle fenêtre"), Some("win.new-window"));
    menu.append_section(None, &s1);

    let zoom = gio::Menu::new();
    zoom.append(Some("Agrandir le texte"), Some("win.zoom-in"));
    zoom.append(Some("Réduire le texte"), Some("win.zoom-out"));
    zoom.append(Some("Taille normale"), Some("win.zoom-reset"));
    menu.append_section(None, &zoom);

    let theme = gio::Menu::new();
    for (id, name) in themes::choices() {
        let item = gio::MenuItem::new(Some(name), None);
        item.set_action_and_target_value(Some("win.theme"), Some(&id.to_variant()));
        theme.append_item(&item);
    }
    let s3 = gio::Menu::new();
    s3.append_submenu(Some("Thème"), &theme);
    s3.append(Some("Barre latérale des alias"), Some("win.toggle-sidebar"));
    s3.append(Some("Copier à la sélection"), Some("win.copy-on-select"));
    s3.append(Some("Plein écran"), Some("win.fullscreen"));
    menu.append_section(None, &s3);

    let s4 = gio::Menu::new();
    s4.append(Some("Préférences"), Some("win.preferences"));
    s4.append(Some("À propos du Terminal"), Some("win.about"));
    menu.append_section(None, &s4);
    menu
}

fn context_menu(on_link: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    if on_link {
        let link = gio::Menu::new();
        link.append(Some("Ouvrir le lien"), Some("win.open-link"));
        link.append(Some("Copier le lien"), Some("win.copy-link"));
        menu.append_section(None, &link);
    }
    let clip = gio::Menu::new();
    clip.append(Some("Copier"), Some("win.copy"));
    clip.append(Some("Coller"), Some("win.paste"));
    clip.append(Some("Tout sélectionner"), Some("win.select-all"));
    menu.append_section(None, &clip);
    let other = gio::Menu::new();
    other.append(Some("Nouvel onglet"), Some("win.new-tab"));
    other.append(Some("Barre latérale des alias"), Some("win.toggle-sidebar"));
    menu.append_section(None, &other);
    menu
}

fn alias_menu(i: i32) -> gio::Menu {
    let menu = gio::Menu::new();
    let run = gio::Menu::new();
    for (label, action) in [("Lancer dans l'onglet actif", "win.alias-run-here"), ("Lancer dans un nouvel onglet", "win.alias-run-tab")] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some(action), Some(&i.to_variant()));
        run.append_item(&item);
    }
    menu.append_section(None, &run);
    let edit = gio::Menu::new();
    for (label, action) in [("Modifier…", "win.alias-edit"), ("Supprimer", "win.alias-delete")] {
        let item = gio::MenuItem::new(Some(label), None);
        item.set_action_and_target_value(Some(action), Some(&i.to_variant()));
        edit.append_item(&item);
    }
    menu.append_section(None, &edit);
    menu
}

fn preferences(parent: &adap::ApplicationWindow) {
    let cfg = cfg();
    let dialog = adap::PreferencesDialog::new();
    dialog.set_title("Préférences du terminal");
    let page = adap::PreferencesPage::new();

    let text = adap::PreferencesGroup::new();
    text.set_title("Texte");
    let font_row = adap::ActionRow::new();
    font_row.set_title("Police");
    font_row.set_subtitle("Une police à chasse fixe est recommandée");
    let font_btn = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    font_btn.set_level(gtk::FontLevel::Family);
    font_btn.set_font_desc(&pango::FontDescription::from_string(&cfg.font_family));
    font_btn.set_valign(gtk::Align::Center);
    font_btn.connect_font_desc_notify(|b| {
        if let Some(family) = b.font_desc().and_then(|d| d.family()) {
            let family = family.to_string();
            update_cfg(|c| c.font_family = family);
        }
    });
    font_row.add_suffix(&font_btn);
    text.add(&font_row);
    let size = adap::SpinRow::with_range(6.0, 48.0, 1.0);
    size.set_title("Taille du texte");
    size.set_subtitle("Ctrl + / Ctrl − / Ctrl 0");
    size.set_value(cfg.font_size);
    size.connect_value_notify(|r| {
        let v = r.value();
        update_cfg(|c| c.font_size = v);
    });
    text.add(&size);
    page.add(&text);

    let look = adap::PreferencesGroup::new();
    look.set_title("Apparence");
    let choices = themes::choices();
    let names: Vec<&str> = choices.iter().map(|c| c.1).collect();
    let theme = adap::ComboRow::new();
    theme.set_title("Thème");
    theme.set_model(Some(&gtk::StringList::new(&names)));
    theme.set_selected(choices.iter().position(|c| c.0 == cfg.theme).unwrap_or(0) as u32);
    theme.connect_selected_notify(move |r| {
        if let Some((id, _)) = choices.get(r.selected() as usize) {
            let id = id.to_string();
            update_cfg(|c| c.theme = id.clone());
            for w in windows() {
                if let Some(a) = w.window.lookup_action("theme").and_downcast::<gio::SimpleAction>() {
                    a.set_state(&id.to_variant());
                }
            }
        }
    });
    look.add(&theme);
    let opacity = adap::SpinRow::with_range(50.0, 100.0, 5.0);
    opacity.set_title("Opacité du fond (%)");
    opacity.set_value((cfg.opacity * 100.0).round());
    opacity.connect_value_notify(|r| {
        let v = r.value() / 100.0;
        update_cfg(|c| c.opacity = v);
    });
    look.add(&opacity);
    let blink = adap::SwitchRow::new();
    blink.set_title("Curseur clignotant");
    blink.set_active(cfg.cursor_blink);
    blink.connect_active_notify(|r| {
        let v = r.is_active();
        update_cfg(|c| c.cursor_blink = v);
    });
    look.add(&blink);
    page.add(&look);

    let behavior = adap::PreferencesGroup::new();
    behavior.set_title("Comportement");
    let cos = adap::SwitchRow::new();
    cos.set_title("Copier à la sélection");
    cos.set_subtitle("Le texte sélectionné va directement dans le presse-papiers");
    cos.set_active(cfg.copy_on_select);
    cos.connect_active_notify(|r| {
        let v = r.is_active();
        update_cfg(|c| c.copy_on_select = v);
    });
    behavior.add(&cos);
    let scroll = adap::SpinRow::with_range(1000.0, 1_000_000.0, 1000.0);
    scroll.set_title("Historique (lignes)");
    scroll.set_value(cfg.scrollback_lines as f64);
    scroll.connect_value_notify(|r| {
        let v = r.value() as i64;
        update_cfg(|c| c.scrollback_lines = v);
    });
    behavior.add(&scroll);
    page.add(&behavior);

    dialog.add(&page);
    dialog.present(Some(parent));
}

fn about(parent: &adap::ApplicationWindow) {
    let about = adap::AboutDialog::builder()
        .application_name("Terminal")
        .application_icon("utilities-terminal")
        .developer_name("NOOS")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Terminal de NOOS : onglets, thèmes et alias")
        .license_type(gtk::License::Gpl30)
        .build();
    about.present(Some(parent));
}

const CSS: &str = r#"
vte-terminal { padding: 6px 10px; }
/* Onglets dans la barre de titre : même hauteur que la barre des autres applis NOOS. */
window.noos-term headerbar { min-height: 46px; padding-top: 0; padding-bottom: 0; }
window.noos-term headerbar tabbar, window.noos-term headerbar tabbar .box {
  min-height: 0; padding: 0; margin: 0; border: none; box-shadow: none; background: none; }
window.noos-term headerbar tabbar tabbox { padding-top: 0; padding-bottom: 0; min-height: 0; }
window.noos-term headerbar tabbar tab { min-height: 30px; padding-top: 0; padding-bottom: 0; margin: 0 2px; }
window.noos-term.translucent, window.noos-term.translucent tabview, window.noos-term.translucent .split-content { background: transparent; }
.alias-sidebar { background-color: alpha(@window_fg_color, 0.03); }
"#;

/// Arguments : `--working-directory DIR`, `-e COMMANDE…` (le reste de la ligne).
fn parse(args: &[String]) -> (Option<String>, Option<String>) {
    let mut cwd = None;
    let mut cmd = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--working-directory" | "-d" => {
                cwd = args.get(i + 1).cloned();
                i += 1;
            }
            "-e" | "-x" | "--" => {
                let rest = args[i + 1..].join(" ");
                if !rest.is_empty() {
                    cmd = Some(rest);
                }
                break;
            }
            _ => {}
        }
        i += 1;
    }
    (cwd, cmd)
}

fn main() -> glib::ExitCode {
    let app = adap::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&gdk::Display::default().unwrap(), &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        // Thème NOOS (palette, mode sombre forcé, rechargement à chaud de noos.toml) ; le
        // thème « NOOS » du terminal suit le passage clair / sombre et la palette.
        let theming = noos_theme::Theming::init();
        theming.connect_changed(|_| apply_all());
        std::mem::forget(theming);
        adap::StyleManager::default().connect_dark_notify(|_| apply_all());
    });
    app.connect_activate(|app| {
        let w = Win::new(app);
        w.new_tab(None, None);
        w.window.present();
    });
    app.connect_command_line(|app, cl| {
        let args: Vec<String> = cl.arguments().iter().map(|a| a.to_string_lossy().to_string()).collect();
        let (cwd, cmd) = parse(&args);
        let cwd = cwd.or_else(|| cl.cwd().map(|p| p.to_string_lossy().to_string()));
        let w = Win::new(app);
        w.new_tab(cwd, cmd);
        w.window.present();
        glib::ExitCode::SUCCESS
    });
    app.run()
}

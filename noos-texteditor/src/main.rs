//! noos-texteditor — Éditeur de texte et de code de NOOS (GTK4 + libAdapta + GtkSourceView 5).
//!
//! Coloration syntaxique de plus de 150 langages (détection par le nom et le contenu), numéros
//! de ligne, mini-carte, recherche et remplacement (casse, mot entier, expressions régulières),
//! onglets, panneau de dossier, autocomplétion des mots, commentaires, opérations sur les lignes,
//! encodages et fins de ligne conservés, rechargement quand un autre programme modifie le
//! fichier, session restaurée, thèmes NOOS clair / sombre.
//!
//! Test : `NOOS_EDITOR_DEMO=1` ouvre des exemples dans plusieurs langages ;
//! `NOOS_EDITOR_SEARCH=mot` lance une recherche.

mod config;
mod doc;
mod prefs;
mod window;

use std::rc::Rc;

use adap::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "org.noos.TextEditor";

fn main() -> glib::ExitCode {
    let app = adap::Application::builder().application_id(APP_ID).flags(gio::ApplicationFlags::HANDLES_OPEN).build();
    app.connect_startup(|app| {
        std::mem::forget(noos_theme::Theming::init());
        sourceview5::init();
        // Thèmes de coloration NOOS (données de NOOS, ou du dépôt en développement).
        let sm = sourceview5::StyleSchemeManager::default();
        sm.prepend_search_path(&noos_core::paths::data_dir().join("sourceview").to_string_lossy());
        let css = gtk::CssProvider::new();
        css.load_from_string(
            ".noos-searchbar { padding: 8px 10px; border-bottom: 1px solid alpha(currentColor, 0.08); background: alpha(@window_fg_color, 0.03); }\n\
             .noos-searchbar togglebutton.search-opt { font-family: monospace; font-weight: 700; min-width: 30px; }\n\
             .noos-status { padding: 0 8px; min-height: 28px; font-size: 0.88em; border-top: 1px solid alpha(currentColor, 0.08); }\n\
             .noos-status > label { margin: 0 8px; }\n\
             .noos-status menubutton > button { padding: 2px 8px; min-height: 0; }\n\
             .noos-tree { background: alpha(@window_fg_color, 0.02); }\n\
             .noos-map { border-left: 1px solid alpha(currentColor, 0.06); }\n\
             entry.error, .error > text { color: @error_color; }\n",
        );
        if let Some(d) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&d, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        for (action, keys) in [
            ("win.new", &["<Control>n"][..]),
            ("win.open", &["<Control>o"]),
            ("win.save", &["<Control>s"]),
            ("win.save-as", &["<Control><Shift>s"]),
            ("win.close-tab", &["<Control>w"]),
            ("win.find", &["<Control>f"]),
            ("win.replace", &["<Control>h"]),
            ("win.find-next", &["F3", "<Control>g"]),
            ("win.find-prev", &["<Shift>F3", "<Control><Shift>g"]),
            ("win.goto", &["<Control>l"]),
            ("win.comment", &["<Control>slash", "<Control>KP_Divide"]),
            ("win.duplicate", &["<Control>d"]),
            ("win.delete-line", &["<Control><Shift>k"]),
            ("win.zoom-in", &["<Control>plus", "<Control>equal", "<Control>KP_Add"]),
            ("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]),
            ("win.zoom-reset", &["<Control>0", "<Control>KP_0"]),
            ("win.toggle-sidebar", &["F9"]),
            ("win.prefs", &["<Control>comma"]),
        ] {
            app.set_accels_for_action(action, keys);
        }
    });
    app.connect_activate(|app| {
        if let Some(w) = app.active_window() {
            w.present();
            return;
        }
        let w = new_window(app);
        if std::env::var_os("NOOS_EDITOR_DEMO").is_some() {
            demo(&w);
        } else {
            w.restore_session();
        }
        if let Ok(q) = std::env::var("NOOS_EDITOR_SEARCH") {
            let w2 = w.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(900), move || {
                gtk::prelude::WidgetExt::activate_action(&w2.window, "win.replace", None).ok();
                if let Some(e) = find_entry(&w2.window) {
                    e.set_text(&q);
                }
            });
        }
    });
    app.connect_open(|app, files, _| {
        let w = match app.active_window().and_then(|aw| WINDOWS.with(|ws| ws.borrow().iter().find(|w| w.window.upcast_ref::<gtk::Window>() == &aw).cloned())) {
            Some(w) => w,
            None => new_window(app),
        };
        for f in files {
            if let Some(p) = f.path() {
                w.open_path(&p);
            }
        }
        w.window.present();
    });
    app.run()
}

thread_local! {
    static WINDOWS: std::cell::RefCell<Vec<Rc<window::Win>>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn new_window(app: &adap::Application) -> Rc<window::Win> {
    let w = window::Win::new(app);
    WINDOWS.with(|ws| ws.borrow_mut().push(w.clone()));
    let weak = Rc::downgrade(&w);
    w.window.connect_destroy(move |_| {
        if let Some(w) = weak.upgrade() {
            WINDOWS.with(|ws| ws.borrow_mut().retain(|x| !Rc::ptr_eq(x, &w)));
        }
    });
    w.window.present();
    w
}

fn find_entry(root: &impl IsA<gtk::Widget>) -> Option<gtk::SearchEntry> {
    let mut stack = vec![root.as_ref().clone()];
    while let Some(w) = stack.pop() {
        if let Ok(e) = w.clone().downcast::<gtk::SearchEntry>() {
            if e.placeholder_text().is_some_and(|p| p == "Rechercher") {
                return Some(e);
            }
        }
        let mut c = w.first_child();
        while let Some(ch) = c {
            stack.push(ch.clone());
            c = ch.next_sibling();
        }
    }
    None
}

/// Exemples pour les captures (aperçu sans fichiers) : écrits dans un dossier temporaire.
fn demo(w: &Rc<window::Win>) {
    let dir = std::env::temp_dir().join("noos-editor-demo");
    let _ = std::fs::create_dir_all(dir.join("src"));
    let files = [
        ("src/main.rs", "//! Démonstration de l'éditeur NOOS.\nuse std::collections::HashMap;\n\n/// Compte les mots d'un texte.\nfn count_words(text: &str) -> HashMap<&str, usize> {\n    let mut map = HashMap::new();\n    for word in text.split_whitespace() {\n        *map.entry(word).or_insert(0) += 1; // TODO : ignorer la casse\n    }\n    map\n}\n\n#[derive(Debug, Clone)]\nstruct Config<'a> {\n    name: &'a str,\n    retries: u32,\n    ratio: f64,\n}\n\nfn main() {\n    let cfg = Config { name: \"noos\", retries: 3, ratio: 0.75 };\n    let words = count_words(\"le chat et le chien\");\n    println!(\"{cfg:?} → {} mots, actif = {}\", words.len(), true);\n}\n"),
        ("script.py", "#!/usr/bin/env python3\n\"\"\"Petit script d'exemple.\"\"\"\nimport json\n\n\ndef load(path: str) -> dict:\n    with open(path, encoding=\"utf-8\") as f:\n        return json.load(f)\n\n\nclass Cache:\n    def __init__(self, size=128):\n        self.size = size  # nombre d'entrées\n        self.data = {}\n\n    def get(self, key, default=None):\n        return self.data.get(key, default)\n"),
        ("style.css", ":root {\n  --accent: #7aa7ff;\n}\n\n.card:hover {\n  background: color-mix(in srgb, var(--accent) 12%, transparent);\n  transition: background 150ms ease-out;\n}\n"),
        ("README.md", "# Projet NOOS\n\nUn **éditeur** simple et *pratique*.\n\n- coloration de plus de 150 langages\n- recherche et `remplacement`\n\n```sh\ncargo build --release\n```\n"),
    ];
    for (name, text) in files {
        let _ = std::fs::write(dir.join(name), text);
    }
    w.set_folder(&dir);
    for (name, _) in files.iter().rev() {
        w.open_path(&dir.join(name));
    }
}

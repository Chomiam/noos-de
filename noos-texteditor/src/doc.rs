//! Un document (un onglet) : tampon GtkSourceView, vue avec numéros de ligne, mini-carte,
//! chargement / enregistrement (encodage et fins de ligne détectés), surveillance du fichier
//! sur le disque.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gio, glib};
use sourceview5::prelude::*;

use crate::config::Config;

pub struct Doc {
    pub root: gtk::Box,
    pub buffer: sourceview5::Buffer,
    pub view: sourceview5::View,
    pub map: sourceview5::Map,
    pub file: sourceview5::File,
    pub banner: adap::Banner,
    pub search: sourceview5::SearchContext,
    /// Encodage et fins de ligne du fichier (conservés à l'enregistrement).
    pub encoding: RefCell<Option<sourceview5::Encoding>>,
    pub newline: Cell<sourceview5::NewlineType>,
    /// Ignorer le prochain événement « modifié sur le disque » (notre propre enregistrement).
    pub saving: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    pub untitled: u32,
}

impl Doc {
    pub fn new(cfg: &Config, untitled: u32, search_settings: &sourceview5::SearchSettings) -> Rc<Self> {
        let buffer = sourceview5::Buffer::new(None::<&gtk::TextTagTable>);
        buffer.set_highlight_matching_brackets(true);
        buffer.set_highlight_syntax(true);
        let view = sourceview5::View::with_buffer(&buffer);
        view.set_monospace(true);
        view.set_auto_indent(true);
        view.set_indent_on_tab(true);
        view.set_smart_backspace(true);
        view.set_smart_home_end(sourceview5::SmartHomeEndType::Before);
        view.set_show_line_marks(false);
        view.set_top_margin(6);
        view.set_bottom_margin(240);
        view.set_left_margin(6);
        view.add_css_class("noos-code");
        view.set_hexpand(true);
        view.set_vexpand(true);

        let scroll = gtk::ScrolledWindow::builder().child(&view).hexpand(true).vexpand(true).build();
        let map = sourceview5::Map::new();
        map.set_view(&view);
        map.add_css_class("noos-map");
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        h.append(&scroll);
        h.append(&map);

        let banner = adap::Banner::new("Ce fichier a été modifié par un autre programme.");
        banner.set_button_label(Some("Recharger"));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&banner);
        root.append(&h);

        // Réglages de recherche partagés par tous les onglets de la fenêtre.
        let search = sourceview5::SearchContext::new(&buffer, Some(search_settings));
        search.set_highlight(false);

        // Autocomplétion : mots déjà présents dans le document.
        let words = sourceview5::CompletionWords::new(Some("Mots du document"));
        words.register(&buffer);
        view.completion().add_provider(&words);

        let doc = Rc::new(Self {
            root,
            buffer,
            view,
            map,
            file: sourceview5::File::new(),
            banner,
            search,
            encoding: RefCell::new(None),
            newline: Cell::new(sourceview5::NewlineType::Lf),
            saving: Cell::new(false),
            monitor: RefCell::new(None),
            untitled,
        });
        doc.apply(cfg);
        doc
    }

    pub fn apply(&self, cfg: &Config) {
        let v = &self.view;
        v.set_show_line_numbers(cfg.line_numbers);
        v.set_highlight_current_line(cfg.highlight_line);
        v.set_tab_width(cfg.tab_width);
        v.set_indent_width(-1);
        v.set_insert_spaces_instead_of_tabs(cfg.spaces);
        v.set_show_right_margin(cfg.right_margin > 0);
        if cfg.right_margin > 0 {
            v.set_right_margin_position(cfg.right_margin);
        }
        v.set_wrap_mode(if cfg.wrap { gtk::WrapMode::WordChar } else { gtk::WrapMode::None });
        self.map.set_visible(cfg.minimap);
        let sd = v.space_drawer();
        sd.set_types_for_locations(sourceview5::SpaceLocationFlags::ALL, sourceview5::SpaceTypeFlags::SPACE | sourceview5::SpaceTypeFlags::TAB);
        sd.set_enable_matrix(cfg.whitespace);
        v.completion().block_interactive();
        if cfg.completion {
            v.completion().unblock_interactive();
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.file.location().and_then(|f| f.path())
    }

    pub fn title(&self) -> String {
        match self.path() {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            None if self.untitled > 1 => format!("Sans titre {}", self.untitled),
            None => "Sans titre".into(),
        }
    }

    pub fn modified(&self) -> bool {
        self.buffer.is_modified()
    }

    pub fn is_empty_untitled(&self) -> bool {
        self.path().is_none() && !self.modified() && self.buffer.char_count() == 0
    }

    /// Langage d'après le nom et le contenu (premières lignes : `#!`, `<?xml`…).
    pub fn guess_language(&self) {
        let lm = sourceview5::LanguageManager::default();
        let name = self.path().map(|p| p.to_string_lossy().into_owned());
        let (start, mut end) = self.buffer.bounds();
        if end.offset() > 4096 {
            end = self.buffer.iter_at_offset(4096);
        }
        let head = self.buffer.text(&start, &end, false);
        let (ct, _) = gio::content_type_guess(name.as_deref(), head.as_bytes());
        let lang = lm.guess_language(name.as_deref(), Some(&ct));
        self.buffer.set_language(lang.as_ref());
    }

    /// Charge `path` dans le document (encodage détecté parmi les plus courants).
    pub fn load(self: &Rc<Self>, path: &Path, done: impl FnOnce(Result<(), String>) + 'static) {
        let gfile = gio::File::for_path(path);
        self.file.set_location(Some(&gfile));
        let loader = sourceview5::FileLoader::new(&self.buffer, &self.file);
        let doc = self.clone();
        let (fut, _progress) = loader.load_future(glib::Priority::DEFAULT);
        glib::spawn_future_local(async move {
            match fut.await {
                Ok(()) => {
                    *doc.encoding.borrow_mut() = Some(loader.encoding());
                    doc.newline.set(loader.newline_type());
                    doc.buffer.set_modified(false);
                    doc.buffer.place_cursor(&doc.buffer.start_iter());
                    doc.guess_language();
                    doc.watch();
                    done(Ok(()));
                }
                Err(e) => done(Err(e.message().to_string())),
            }
        });
    }

    /// Enregistre (vers `target` pour « Enregistrer sous »).
    pub fn save(self: &Rc<Self>, target: Option<&Path>, done: impl FnOnce(Result<(), String>) + 'static) {
        let saver = match target {
            Some(t) => sourceview5::FileSaver::with_target(&self.buffer, &self.file, &gio::File::for_path(t)),
            None => sourceview5::FileSaver::new(&self.buffer, &self.file),
        };
        if let Some(enc) = self.encoding.borrow().as_ref() {
            saver.set_encoding(Some(enc));
        }
        saver.set_newline_type(self.newline.get());
        // Le fichier a pu changer ailleurs : on écrase (l'utilisateur a été prévenu par le bandeau).
        saver.set_flags(sourceview5::FileSaverFlags::IGNORE_MODIFICATION_TIME);
        self.saving.set(true);
        let doc = self.clone();
        let retarget = target.is_some();
        let (fut, _progress) = saver.save_future(glib::Priority::DEFAULT);
        glib::spawn_future_local(async move {
            match fut.await {
                Ok(()) => {
                    doc.buffer.set_modified(false);
                    doc.banner.set_revealed(false);
                    if retarget {
                        doc.guess_language();
                        doc.watch();
                    }
                    // Les événements du moniteur arrivent juste après l'écriture.
                    let d = doc.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || d.saving.set(false));
                    done(Ok(()));
                }
                Err(e) => {
                    doc.saving.set(false);
                    done(Err(e.message().to_string()));
                }
            }
        });
    }

    /// Surveille le fichier : modifié par un autre programme → rechargé (si pas de
    /// modifications ici) ou bandeau « Recharger ».
    fn watch(self: &Rc<Self>) {
        let Some(loc) = self.file.location() else { return };
        let Ok(mon) = loc.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) else { return };
        let weak = Rc::downgrade(self);
        mon.connect_changed(move |_, _, _, ev| {
            let Some(doc) = weak.upgrade() else { return };
            if doc.saving.get() || !matches!(ev, gio::FileMonitorEvent::ChangesDoneHint | gio::FileMonitorEvent::Created) {
                return;
            }
            if doc.modified() {
                doc.banner.set_revealed(true);
            } else if let Some(p) = doc.path() {
                let d = doc.clone();
                let line = doc.buffer.iter_at_mark(&doc.buffer.get_insert()).line();
                doc.load(&p, move |_| {
                    let it = d.buffer.iter_at_line(line).unwrap_or(d.buffer.start_iter());
                    d.buffer.place_cursor(&it);
                });
            }
        });
        *self.monitor.borrow_mut() = Some(mon);
    }

    /// Ligne et colonne du curseur (à partir de 1), et longueur de la sélection.
    pub fn position(&self) -> (i32, i32, i32) {
        let it = self.buffer.iter_at_mark(&self.buffer.get_insert());
        let sel = self.buffer.selection_bounds().map(|(a, b)| b.offset() - a.offset()).unwrap_or(0);
        (it.line() + 1, self.view.visual_column(&it) as i32 + 1, sel)
    }

    pub fn goto_line(&self, line: i32) {
        let it = self.buffer.iter_at_line((line - 1).max(0)).unwrap_or(self.buffer.end_iter());
        self.buffer.place_cursor(&it);
        self.view.scroll_to_iter(&mut it.clone(), 0.2, true, 0.0, 0.4);
        self.view.grab_focus();
    }

    /// Lignes touchées par la sélection (ou ligne du curseur) : début de la première, fin de la dernière.
    fn line_range(&self) -> (gtk::TextIter, gtk::TextIter) {
        let (mut a, mut b) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let i = self.buffer.iter_at_mark(&self.buffer.get_insert());
            (i, i)
        });
        if b.starts_line() && b.offset() > a.offset() {
            b.backward_char();
        }
        a.set_line_offset(0);
        if !b.ends_line() {
            b.forward_to_line_end();
        }
        (a, b)
    }

    /// Commente / décommente les lignes (commentaire de ligne du langage, sinon de bloc).
    pub fn toggle_comment(&self) {
        let Some(lang) = self.buffer.language() else { return };
        let line = lang.metadata("line-comment-start").map(|s| s.to_string());
        let block = lang.metadata("block-comment-start").zip(lang.metadata("block-comment-end")).map(|(a, b)| (a.to_string(), b.to_string()));
        let (a, b) = self.line_range();
        let text = self.buffer.text(&a, &b, true).to_string();
        let new = if let Some(c) = line {
            let lines: Vec<&str> = text.split('\n').collect();
            let all = lines.iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with(&c));
            let indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
            lines
                .iter()
                .map(|l| {
                    if l.trim().is_empty() {
                        l.to_string()
                    } else if all {
                        let i = l.find(&c).unwrap_or(0);
                        let rest = &l[i + c.len()..];
                        format!("{}{}", &l[..i], rest.strip_prefix(' ').unwrap_or(rest))
                    } else {
                        format!("{}{c} {}", &l[..indent.min(l.len())], &l[indent.min(l.len())..])
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else if let Some((s, e)) = block {
            let t = text.trim();
            if t.starts_with(&s) && t.ends_with(&e) {
                text.replacen(&s, "", 1).trim_end().strip_suffix(&e).unwrap_or(&text).trim_start().to_string()
            } else {
                format!("{s} {text} {e}")
            }
        } else {
            return;
        };
        self.replace_range(a, b, &new);
    }

    fn replace_range(&self, mut a: gtk::TextIter, mut b: gtk::TextIter, text: &str) {
        let start = a.offset();
        self.buffer.begin_user_action();
        self.buffer.delete(&mut a, &mut b);
        let mut it = self.buffer.iter_at_offset(start);
        self.buffer.insert(&mut it, text);
        let s = self.buffer.iter_at_offset(start);
        self.buffer.select_range(&s, &it);
        self.buffer.end_user_action();
    }

    pub fn duplicate_lines(&self) {
        let (a, b) = self.line_range();
        let text = self.buffer.text(&a, &b, true).to_string();
        let mut end = b;
        self.buffer.begin_user_action();
        self.buffer.insert(&mut end, &format!("\n{text}"));
        self.buffer.end_user_action();
    }

    pub fn delete_lines(&self) {
        let (mut a, mut b) = self.line_range();
        if !b.forward_char() && a.backward_char() {}
        self.buffer.begin_user_action();
        self.buffer.delete(&mut a, &mut b);
        self.buffer.end_user_action();
    }

    pub fn sort_lines(&self) {
        let (a, b) = self.line_range();
        self.buffer.sort_lines(&mut a.clone(), &mut b.clone(), sourceview5::SortFlags::CASE_SENSITIVE, 0);
    }

    pub fn change_case(&self, case: sourceview5::ChangeCaseType) {
        if let Some((mut a, mut b)) = self.buffer.selection_bounds() {
            self.buffer.change_case(case, &mut a, &mut b);
        }
    }

    /// Supprime les espaces en fin de ligne.
    pub fn trim_trailing(&self) {
        let (start, end) = self.buffer.bounds();
        let text = self.buffer.text(&start, &end, true).to_string();
        let trimmed: String = text.split('\n').map(|l| l.trim_end_matches([' ', '\t'])).collect::<Vec<_>>().join("\n");
        if trimmed != text {
            let line = self.buffer.iter_at_mark(&self.buffer.get_insert()).line();
            self.buffer.begin_user_action();
            let (mut s, mut e) = self.buffer.bounds();
            self.buffer.delete(&mut s, &mut e);
            self.buffer.insert(&mut self.buffer.start_iter(), &trimmed);
            self.buffer.end_user_action();
            if let Some(it) = self.buffer.iter_at_line(line) {
                self.buffer.place_cursor(&it);
            }
        }
    }
}

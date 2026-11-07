//! Page « Créer une archive » : fichiers, nom et dossier, format (tuiles), compression,
//! mot de passe, volumes, exclusions.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};

use crate::engine::{CreateOptions, OutFormat};
use crate::jobs::human;

pub struct CreatePage {
    pub root: gtk::ScrolledWindow,
    sources: Rc<RefCell<Vec<PathBuf>>>,
    list: gtk::ListBox,
    summary: gtk::Label,
    name: adap::EntryRow,
    folder: Rc<RefCell<PathBuf>>,
    folder_row: adap::ActionRow,
    format: Rc<Cell<OutFormat>>,
    on_change: RefCell<Option<Rc<dyn Fn()>>>,
}

fn size_of(p: &Path) -> u64 {
    match std::fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Taux de compression typique (estimation grossière, fichiers ordinaires).
fn typical_ratio(f: OutFormat, level: u32) -> f64 {
    let l = f64::from(level) / 9.0;
    match f {
        OutFormat::Tar | OutFormat::Iso => 1.0,
        _ if level == 0 => 1.0,
        OutFormat::SevenZ | OutFormat::TarXz | OutFormat::TarLz => 0.62 - 0.12 * l,
        OutFormat::TarZst => 0.66 - 0.1 * l,
        OutFormat::TarBz2 => 0.66 - 0.06 * l,
        OutFormat::Zip | OutFormat::TarGz | OutFormat::Wim => 0.70 - 0.06 * l,
        OutFormat::TarLz4 => 0.78 - 0.06 * l,
    }
}

impl CreatePage {
    pub fn new(on_create: impl Fn(CreateOptions) + 'static) -> Rc<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 22);
        content.set_margin_top(18);
        content.set_margin_bottom(28);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let clamp = adap::Clamp::builder().maximum_size(820).child(&content).build();
        let root = gtk::ScrolledWindow::builder().child(&clamp).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();

        // ── Fichiers ──
        let files_group = adap::PreferencesGroup::new();
        files_group.set_title("Fichiers à archiver");
        let add_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let add_files = gtk::Button::from_icon_name("document-new-symbolic");
        add_files.set_tooltip_text(Some("Ajouter des fichiers"));
        add_files.add_css_class("flat");
        let add_folder = gtk::Button::from_icon_name("folder-new-symbolic");
        add_folder.set_tooltip_text(Some("Ajouter un dossier"));
        add_folder.add_css_class("flat");
        add_box.append(&add_files);
        add_box.append(&add_folder);
        files_group.set_header_suffix(Some(&add_box));
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        files_group.add(&list);
        let summary = gtk::Label::new(None);
        summary.add_css_class("dim-label");
        summary.set_xalign(0.0);
        summary.set_margin_top(6);
        files_group.add(&summary);
        content.append(&files_group);

        // ── Archive ──
        let g = adap::PreferencesGroup::new();
        g.set_title("Archive");
        let name = adap::EntryRow::new();
        name.set_title("Nom");
        g.add(&name);
        let folder_row = adap::ActionRow::new();
        folder_row.set_title("Dossier");
        folder_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
        let change = gtk::Button::with_label("Changer…");
        change.set_valign(gtk::Align::Center);
        folder_row.add_suffix(&change);
        g.add(&folder_row);
        content.append(&g);

        // ── Format ──
        let fmt_title = gtk::Label::new(Some("Format"));
        fmt_title.add_css_class("heading");
        fmt_title.set_xalign(0.0);
        content.append(&fmt_title);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_homogeneous(true);
        flow.set_min_children_per_line(2);
        flow.set_max_children_per_line(4);
        flow.set_column_spacing(8);
        flow.set_row_spacing(8);
        content.append(&flow);
        content.set_margin_top(18);
        let format = Rc::new(Cell::new(OutFormat::SevenZ));

        // ── Options ──
        let g = adap::PreferencesGroup::new();
        g.set_title("Options");
        let level = adap::ComboRow::new();
        level.set_title("Compression");
        level.set_model(Some(&gtk::StringList::new(&["Aucune (stockage)", "Rapide", "Normale", "Maximale", "Ultra"])));
        level.set_selected(2);
        g.add(&level);
        let pw = adap::PasswordEntryRow::new();
        pw.set_title("Mot de passe (facultatif)");
        g.add(&pw);
        let names = adap::SwitchRow::new();
        names.set_title("Chiffrer aussi les noms des fichiers");
        names.set_subtitle("Sans le mot de passe, on ne voit même pas la liste");
        names.set_active(true);
        g.add(&names);
        let split = adap::ComboRow::new();
        split.set_title("Découper en volumes");
        split.set_model(Some(&gtk::StringList::new(&["Non", "100 Mo (pièces jointes)", "700 Mo (CD)", "4 095 Mo (clé FAT32)", "4 480 Mo (DVD)", "Personnalisé…"])));
        g.add(&split);
        let split_custom = adap::SpinRow::with_range(1.0, 1_000_000.0, 1.0);
        split_custom.set_title("Taille d'un volume (Mo)");
        split_custom.set_value(1024.0);
        split_custom.set_visible(false);
        g.add(&split_custom);
        let solid = adap::SwitchRow::new();
        solid.set_title("Archive solide");
        solid.set_subtitle("Meilleure compression ; extraire un seul fichier est plus lent");
        solid.set_active(true);
        g.add(&solid);
        let excludes = adap::EntryRow::new();
        excludes.set_title("Exclure (motifs séparés par des espaces)");
        excludes.set_text(".git node_modules __pycache__ *.tmp .DS_Store Thumbs.db");
        g.add(&excludes);
        let label = adap::EntryRow::new();
        label.set_title("Nom du volume (ISO)");
        label.set_text("DONNEES");
        g.add(&label);
        content.append(&g);

        let estimate = gtk::Label::new(None);
        estimate.add_css_class("dim-label");
        let go = gtk::Button::with_label("Créer l'archive");
        go.add_css_class("suggested-action");
        go.add_css_class("pill");
        go.set_halign(gtk::Align::Center);
        content.append(&estimate);
        content.append(&go);

        let page = Rc::new(Self {
            root,
            sources: Rc::new(RefCell::new(vec![])),
            list,
            summary,
            name,
            folder: Rc::new(RefCell::new(glib::home_dir())),
            folder_row,
            format,
            on_change: RefCell::new(None),
        });

        // Tuiles de format.
        let tiles: Rc<RefCell<Vec<(OutFormat, gtk::ToggleButton)>>> = Rc::new(RefCell::new(vec![]));
        let mut group: Option<gtk::ToggleButton> = None;
        for f in OutFormat::ALL {
            let available = match f {
                OutFormat::SevenZ | OutFormat::Zip | OutFormat::Wim => crate::engine::tool_available("7z"),
                OutFormat::Iso => crate::engine::tool_available("xorriso"),
                _ => crate::engine::tool_available("bsdtar"),
            };
            let b = gtk::ToggleButton::new();
            b.add_css_class("fmt-tile");
            let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let t = gtk::Label::new(Some(&format!(".{}", f.ext())));
            t.add_css_class("fmt-ext");
            t.set_xalign(0.0);
            let d = gtk::Label::new(Some(f.description()));
            d.add_css_class("caption");
            d.add_css_class("dim-label");
            d.set_wrap(true);
            d.set_xalign(0.0);
            d.set_lines(2);
            d.set_ellipsize(gtk::pango::EllipsizeMode::End);
            v.append(&t);
            v.append(&d);
            b.set_child(Some(&v));
            b.set_sensitive(available);
            if !available {
                b.set_tooltip_text(Some("Outil manquant sur ce système"));
            }
            match &group {
                Some(g) => b.set_group(Some(g)),
                None => group = Some(b.clone()),
            }
            if f == OutFormat::SevenZ {
                b.set_active(true);
            }
            flow.append(&b);
            tiles.borrow_mut().push((f, b));
        }

        // Mise à jour : options valables pour le format, estimation, extension du nom.
        let refresh: Rc<dyn Fn()> = {
            let (p, level, pw, names, split, split_custom, solid, label, estimate, go) =
                (page.clone(), level.clone(), pw.clone(), names.clone(), split.clone(), split_custom.clone(), solid.clone(), label.clone(), estimate.clone(), go.clone());
            Rc::new(move || {
                let f = p.format.get();
                level.set_visible(f.supports_level());
                pw.set_visible(f.supports_password());
                names.set_visible(f == OutFormat::SevenZ && !pw.text().is_empty());
                split.set_visible(f.supports_split());
                split_custom.set_visible(f.supports_split() && split.selected() == 5);
                solid.set_visible(f == OutFormat::SevenZ);
                label.set_visible(f == OutFormat::Iso);
                let total: u64 = p.sources.borrow().iter().map(|s| size_of(s)).sum();
                let lvl = [0u32, 1, 5, 7, 9][level.selected().min(4) as usize];
                let est = (total as f64 * typical_ratio(f, lvl)) as u64;
                estimate.set_text(&if total == 0 {
                    "Ajoutez des fichiers ou des dossiers (bouton + ou glisser-déposer)".into()
                } else {
                    format!("{} à archiver → environ {} en .{} (estimation)", human(total), human(est), f.ext())
                });
                go.set_sensitive(total > 0 || !p.sources.borrow().is_empty());
                p.summary.set_visible(!p.sources.borrow().is_empty());
            })
        };
        *page.on_change.borrow_mut() = Some(refresh.clone());
        for (f, b) in tiles.borrow().iter() {
            let (p, refresh, f) = (page.clone(), refresh.clone(), *f);
            b.connect_toggled(move |b| {
                if b.is_active() {
                    p.format.set(f);
                    p.fix_name_extension();
                    refresh();
                }
            });
        }
        for w in [level.upcast_ref::<gtk::Widget>(), split.upcast_ref()] {
            let r = refresh.clone();
            w.connect_notify_local(Some("selected"), move |_, _| r());
        }
        {
            let r = refresh.clone();
            pw.connect_changed(move |_| r());
        }

        // Ajout de fichiers.
        {
            let p = page.clone();
            add_files.connect_clicked(move |b| {
                let fd = gtk::FileDialog::builder().title("Ajouter des fichiers").build();
                let p = p.clone();
                fd.open_multiple(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |r| {
                    if let Ok(files) = r {
                        let v: Vec<PathBuf> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path()).collect();
                        p.add(&v);
                    }
                });
            });
        }
        {
            let p = page.clone();
            add_folder.connect_clicked(move |b| {
                let fd = gtk::FileDialog::builder().title("Ajouter des dossiers").build();
                let p = p.clone();
                fd.select_multiple_folders(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |r| {
                    if let Ok(files) = r {
                        let v: Vec<PathBuf> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path()).collect();
                        p.add(&v);
                    }
                });
            });
        }
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        {
            let p = page.clone();
            drop.connect_drop(move |_, v, _, _| {
                let Ok(list) = v.get::<gdk::FileList>() else { return false };
                let v: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
                p.add(&v);
                true
            });
        }
        page.root.add_controller(drop);
        {
            let p = page.clone();
            change.connect_clicked(move |b| {
                let fd = gtk::FileDialog::builder().title("Dossier de l'archive").initial_folder(&gio::File::for_path(&*p.folder.borrow())).build();
                let p = p.clone();
                fd.select_folder(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |r| {
                    if let Some(d) = r.ok().and_then(|f| f.path()) {
                        p.folder_row.set_subtitle(&d.display().to_string());
                        *p.folder.borrow_mut() = d;
                    }
                });
            });
        }
        {
            let p = page.clone();
            go.connect_clicked(move |_| {
                let f = p.format.get();
                let mut name = p.name.text().trim().to_string();
                if name.is_empty() {
                    name = "Archive".into();
                }
                if !name.to_lowercase().ends_with(&format!(".{}", f.ext())) {
                    name = format!("{name}.{}", f.ext());
                }
                let split_mb = match split.selected() {
                    1 => 100,
                    2 => 700,
                    3 => 4095,
                    4 => 4480,
                    5 => split_custom.value() as u64,
                    _ => 0,
                };
                on_create(CreateOptions {
                    sources: p.sources.borrow().clone(),
                    output: p.folder.borrow().join(name),
                    format: f,
                    level: [0u32, 1, 5, 7, 9][level.selected().min(4) as usize],
                    password: Some(pw.text().to_string()).filter(|s| !s.is_empty() && f.supports_password()),
                    encrypt_names: names.is_active(),
                    split_mb: if f.supports_split() { split_mb } else { 0 },
                    solid: solid.is_active(),
                    excludes: excludes.text().split_whitespace().map(String::from).collect(),
                    label: label.text().to_string(),
                });
            });
        }
        page.render_list();
        refresh();
        page
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.borrow().as_ref() {
            f();
        }
    }

    /// Remplace l'extension du nom par celle du format choisi.
    fn fix_name_extension(&self) {
        let t = self.name.text().to_string();
        if t.is_empty() {
            return;
        }
        let base = crate::engine::stem(Path::new(&t));
        self.name.set_text(&format!("{base}.{}", self.format.get().ext()));
    }

    pub fn add(&self, paths: &[PathBuf]) {
        {
            let mut s = self.sources.borrow_mut();
            for p in paths {
                if !s.contains(p) {
                    s.push(p.clone());
                }
            }
        }
        // Nom et dossier proposés d'après le premier élément.
        if let Some(first) = self.sources.borrow().first() {
            if self.name.text().is_empty() {
                let base = if self.sources.borrow().len() == 1 {
                    first.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
                } else {
                    first.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into())
                };
                self.name.set_text(&format!("{base}.{}", self.format.get().ext()));
            }
            if let Some(parent) = first.parent() {
                *self.folder.borrow_mut() = parent.to_path_buf();
            }
        }
        self.folder_row.set_subtitle(&self.folder.borrow().display().to_string());
        self.render_list();
        self.changed();
    }

    fn render_list(self: &Self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let sources = self.sources.borrow().clone();
        if sources.is_empty() {
            let r = adap::ActionRow::new();
            r.set_title("Glissez des fichiers ou des dossiers ici");
            r.set_subtitle("ou utilisez les boutons « + » ci-dessus");
            r.add_prefix(&gtk::Image::from_icon_name("folder-download-symbolic"));
            self.list.append(&r);
            return;
        }
        let total: u64 = sources.iter().map(|s| size_of(s)).sum();
        self.summary.set_text(&format!("{} élément{} · {}", sources.len(), if sources.len() > 1 { "s" } else { "" }, human(total)));
        for p in sources {
            let r = adap::ActionRow::new();
            r.set_use_markup(false);
            r.set_title(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            r.set_subtitle(&format!("{} · {}", human(size_of(&p)), p.parent().map(|d| d.display().to_string()).unwrap_or_default()));
            r.add_prefix(&gtk::Image::from_icon_name(if p.is_dir() { "folder-symbolic" } else { "text-x-generic-symbolic" }));
            let rm = gtk::Button::from_icon_name("list-remove-symbolic");
            rm.add_css_class("flat");
            rm.set_valign(gtk::Align::Center);
            rm.set_tooltip_text(Some("Retirer"));
            let (sources, list, p2) = (self.sources.clone(), self.list.clone(), p.clone());
            let on_change = self.on_change.borrow().clone();
            let row = r.clone();
            rm.connect_clicked(move |_| {
                sources.borrow_mut().retain(|x| *x != p2);
                list.remove(&row);
                if let Some(f) = &on_change {
                    f();
                }
            });
            r.add_suffix(&rm);
            self.list.append(&r);
        }
    }
}

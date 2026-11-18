//! Galerie de widgets (mode édition) : une carte illustrée par widget, recherche et
//! catégories. Glisser une carte sur le bureau (n'importe quel écran) l'y pose ; un clic la
//! place au premier endroit libre de l'écran principal.

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, prelude::*};

use crate::{Shell, widgets};

pub const CSS: &str = r#"
.edit-gallery { padding: 14px; border-radius: 18px; color: @window_fg_color;
  border: 1px solid alpha(@window_fg_color, 0.10); box-shadow: 0 16px 44px rgba(0,0,0,0.45); }
.edit-gallery .gallery-cats button { padding: 2px 12px; min-height: 28px; border-radius: 99px; }
.edit-gallery .gallery-cats button:checked { background-color: @accent_bg_color; color: @accent_fg_color; }
.widget-card { padding: 12px; border-radius: 14px; background-color: alpha(@window_fg_color, 0.04); border: 1px solid alpha(@window_fg_color, 0.06);
  transition: background-color 150ms, transform 150ms; }
.widget-card:hover { background-color: alpha(@accent_bg_color, 0.14); border-color: alpha(@accent_bg_color, 0.5); }
.widget-card .widget-tile { min-width: 52px; min-height: 52px; border-radius: 14px; color: white; box-shadow: 0 4px 12px rgba(0,0,0,0.25); }
.widget-card .widget-name { font-weight: 700; }
.widget-card .widget-desc { font-size: 0.85em; opacity: 0.7; }
.widget-card .widget-add { font-size: 0.75em; font-weight: 700; color: @accent_bg_color; opacity: 0; transition: opacity 150ms; }
.widget-card:hover .widget-add { opacity: 1; }
.gallery-empty { padding: 30px; opacity: 0.6; }
.tile-clock { background-image: linear-gradient(135deg, #5b8def, #3a5bd9); }
.tile-worldclock { background-image: linear-gradient(135deg, #2bb3c0, #1b7f9e); }
.tile-calendar { background-image: linear-gradient(135deg, #f66151, #c64060); }
.tile-countdown { background-image: linear-gradient(135deg, #ff9f43, #f0643c); }
.tile-weather { background-image: linear-gradient(135deg, #4facfe, #2a7fd4); }
.tile-media { background-image: linear-gradient(135deg, #c061cb, #7d3cbf); }
.tile-photo { background-image: linear-gradient(135deg, #2ec27e, #158a6b); }
.tile-note { background-image: linear-gradient(135deg, #f8d14a, #e6a823); }
.tile-folders { background-image: linear-gradient(135deg, #6b8af7, #8f5cf0); }
.tile-vitals { background-image: linear-gradient(135deg, #3ec27f, #2a8fc9); }
"#;

pub struct Gallery {
    pub panel: gtk::Box,
    pub search: gtk::SearchEntry,
}

pub fn build(shell: &Rc<Shell>, on_added: impl Fn() + 'static) -> Gallery {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("edit-gallery");
    panel.set_size_request(760, -1);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Rechercher un widget (Ctrl+F)"));
    search.set_hexpand(true);
    head.append(&search);
    let cats = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    cats.add_css_class("gallery-cats");
    head.append(&cats);
    panel.append(&head);
    let hint = gtk::Label::new(Some("Glissez une carte sur le bureau, ou cliquez pour l'ajouter à un endroit libre."));
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    hint.set_xalign(0.0);
    panel.append(&hint);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_min_children_per_line(3);
    flow.set_max_children_per_line(3);
    flow.set_homogeneous(true);
    flow.set_row_spacing(8);
    flow.set_column_spacing(8);
    panel.append(&flow);
    let empty = gtk::Label::new(Some("Aucun widget ne correspond"));
    empty.add_css_class("gallery-empty");
    empty.set_visible(false);
    panel.append(&empty);
    let on_added = Rc::new(on_added);
    for spec in widgets::SPECS {
        let card = gtk::Button::new();
        card.add_css_class("flat");
        card.add_css_class("widget-card");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tile.add_css_class("widget-tile");
        tile.add_css_class(&format!("tile-{}", spec.kind));
        tile.set_valign(gtk::Align::Center);
        tile.set_hexpand(false);
        let icon = gtk::Image::from_icon_name(spec.icon);
        icon.set_pixel_size(26);
        icon.set_hexpand(true);
        tile.append(&icon);
        row.append(&tile);
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_valign(gtk::Align::Center);
        let n = gtk::Label::new(Some(spec.name));
        n.add_css_class("widget-name");
        n.set_xalign(0.0);
        col.append(&n);
        let d = gtk::Label::new(Some(spec.description));
        d.add_css_class("widget-desc");
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_max_width_chars(22);
        d.set_lines(2);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        col.append(&d);
        let a = gtk::Label::new(Some("＋ Ajouter"));
        a.add_css_class("widget-add");
        a.set_xalign(0.0);
        col.append(&a);
        row.append(&col);
        card.set_child(Some(&row));
        card.set_tooltip_text(Some(&format!("{} — glisser sur le bureau ou cliquer", spec.name)));
        // Glisser vers le bureau.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        let kind = spec.kind;
        drag.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&format!("noos-widget:{kind}").to_value())));
        {
            let tile = tile.clone();
            drag.connect_drag_begin(move |src, _| src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&tile))), 26, 26));
        }
        card.add_controller(drag);
        {
            let (shell, on_added) = (Rc::downgrade(shell), on_added.clone());
            card.connect_clicked(move |_| {
                if let Some(s) = shell.upgrade() {
                    crate::desk::add_widget(&s, kind, None, None);
                    on_added();
                }
            });
        }
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&card));
        // Données de filtrage : nom, description, catégorie.
        unsafe {
            child.set_data("noos-filter", format!("{} {} {}", spec.name, spec.description, spec.kind).to_lowercase());
            child.set_data("noos-cat", spec.category.to_string());
        }
        flow.insert(&child, -1);
    }
    let cat = Rc::new(RefCell::new("all".to_string()));
    let refilter = {
        let (flow, search, cat, empty) = (flow.clone(), search.clone(), cat.clone(), empty.clone());
        Rc::new(move || {
            let q = search.text().to_lowercase();
            let c = cat.borrow().clone();
            flow.set_filter_func(move |child| unsafe {
                let text = child.data::<String>("noos-filter").map(|p| p.as_ref().clone()).unwrap_or_default();
                let kc = child.data::<String>("noos-cat").map(|p| p.as_ref().clone()).unwrap_or_default();
                (c == "all" || kc == c) && q.split_whitespace().all(|w| text.contains(w))
            });
            // Le filtre ne s'applique qu'au prochain cycle : on compte d'après le catalogue.
            let q = search.text().to_lowercase();
            let c2 = cat.borrow().clone();
            let n = widgets::SPECS
                .iter()
                .filter(|s| (c2 == "all" || s.category == c2) && q.split_whitespace().all(|w| format!("{} {} {}", s.name, s.description, s.kind).to_lowercase().contains(w)))
                .count();
            empty.set_visible(n == 0);
        })
    };
    let mut first: Option<gtk::ToggleButton> = None;
    for (id, label) in widgets::CATEGORIES {
        let t = gtk::ToggleButton::with_label(label);
        t.add_css_class("flat");
        if let Some(f) = &first {
            t.set_group(Some(f));
        } else {
            first = Some(t.clone());
            t.set_active(true);
        }
        let (cat, refilter) = (cat.clone(), refilter.clone());
        t.connect_toggled(move |t| {
            if t.is_active() {
                *cat.borrow_mut() = id.to_string();
                refilter();
            }
        });
        cats.append(&t);
    }
    {
        let refilter = refilter.clone();
        search.connect_search_changed(move |_| refilter());
    }
    {
        // Entrée : ajoute le premier widget trouvé.
        let shell = Rc::downgrade(shell);
        let on_added = on_added.clone();
        search.connect_activate(move |e| {
            let q = e.text().to_lowercase();
            if let Some(spec) = widgets::SPECS.iter().find(|s| q.split_whitespace().all(|w| format!("{} {} {}", s.name, s.description, s.kind).to_lowercase().contains(w))) {
                if let Some(s) = shell.upgrade() {
                    crate::desk::add_widget(&s, spec.kind, None, None);
                    on_added();
                }
            }
        });
    }
    if let Ok(q) = std::env::var("NOOS_EDIT_SEARCH") {
        search.set_text(&q);
    }
    refilter();
    Gallery { panel, search }
}

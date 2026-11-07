//! Préférences : apparence (police, thème de coloration, affichage) et édition.

use std::rc::Rc;

use adap::prelude::*;

use crate::{config::Config, window::Win};

fn switch(title: &str, subtitle: &str, value: bool, w: &Rc<Win>, set: fn(&mut Config, bool)) -> adap::SwitchRow {
    let r = adap::SwitchRow::new();
    r.set_title(title);
    if !subtitle.is_empty() {
        r.set_subtitle(subtitle);
    }
    r.set_active(value);
    let w = w.clone();
    r.connect_active_notify(move |r| {
        set(&mut w.cfg.borrow_mut(), r.is_active());
        w.cfg.borrow().save();
        w.apply_all();
    });
    r
}

pub fn show(w: &Rc<Win>) {
    let cfg = w.cfg.borrow().clone();
    let dialog = adap::PreferencesDialog::new();

    // ── Apparence ──
    let page = adap::PreferencesPage::new();
    page.set_title("Apparence");
    page.set_icon_name(Some("applications-graphics-symbolic"));
    let g = adap::PreferencesGroup::new();
    g.set_title("Texte");
    let font = adap::ActionRow::new();
    font.set_title("Police");
    let fb = gtk::FontDialogButton::new(Some(gtk::FontDialog::new()));
    fb.set_font_desc(&gtk::pango::FontDescription::from_string(&cfg.font));
    fb.set_valign(gtk::Align::Center);
    {
        let w = w.clone();
        fb.connect_font_desc_notify(move |b| {
            if let Some(d) = b.font_desc() {
                w.cfg.borrow_mut().font = d.to_string();
                w.cfg.borrow().save();
                w.apply_all();
            }
        });
    }
    font.add_suffix(&fb);
    g.add(&font);
    // Thèmes de coloration : NOOS (suit le mode clair / sombre) puis ceux de GtkSourceView.
    let sm = sourceview5::StyleSchemeManager::default();
    let mut ids: Vec<(String, String)> = vec![("noos".into(), "NOOS (suit le mode clair / sombre)".into())];
    for id in sm.scheme_ids() {
        if id.starts_with("noos-") {
            continue;
        }
        if let Some(s) = sm.scheme(&id) {
            ids.push((id.to_string(), s.name().to_string()));
        }
    }
    let names: Vec<&str> = ids.iter().map(|(_, n)| n.as_str()).collect();
    let scheme = adap::ComboRow::new();
    scheme.set_title("Thème de coloration");
    scheme.set_model(Some(&gtk::StringList::new(&names)));
    scheme.set_selected(ids.iter().position(|(i, _)| *i == cfg.scheme).unwrap_or(0) as u32);
    {
        let w = w.clone();
        scheme.connect_selected_notify(move |r| {
            if let Some((id, _)) = ids.get(r.selected() as usize) {
                w.cfg.borrow_mut().scheme = id.clone();
                w.cfg.borrow().save();
                w.apply_all();
            }
        });
    }
    g.add(&scheme);
    page.add(&g);
    let g = adap::PreferencesGroup::new();
    g.set_title("Affichage");
    g.add(&switch("Numéros de ligne", "", cfg.line_numbers, w, |c, v| c.line_numbers = v));
    g.add(&switch("Surligner la ligne courante", "", cfg.highlight_line, w, |c, v| c.highlight_line = v));
    g.add(&switch("Mini-carte", "Vue d'ensemble du document à droite, cliquable", cfg.minimap, w, |c, v| c.minimap = v));
    g.add(&switch("Espaces et tabulations visibles", "", cfg.whitespace, w, |c, v| c.whitespace = v));
    g.add(&switch("Retour à la ligne automatique", "Les longues lignes s'affichent sur plusieurs lignes", cfg.wrap, w, |c, v| c.wrap = v));
    let margin = adap::SpinRow::with_range(0.0, 200.0, 1.0);
    margin.set_title("Marge droite");
    margin.set_subtitle("Repère vertical à cette colonne (0 : masqué)");
    margin.set_value(f64::from(cfg.right_margin));
    {
        let w = w.clone();
        margin.connect_value_notify(move |r| {
            w.cfg.borrow_mut().right_margin = r.value() as u32;
            w.cfg.borrow().save();
            w.apply_all();
        });
    }
    g.add(&margin);
    page.add(&g);
    dialog.add(&page);

    // ── Édition ──
    let page = adap::PreferencesPage::new();
    page.set_title("Édition");
    page.set_icon_name(Some("document-edit-symbolic"));
    let g = adap::PreferencesGroup::new();
    g.set_title("Indentation");
    let tab = adap::SpinRow::with_range(1.0, 16.0, 1.0);
    tab.set_title("Largeur des tabulations");
    tab.set_value(f64::from(cfg.tab_width));
    {
        let w = w.clone();
        tab.connect_value_notify(move |r| {
            w.cfg.borrow_mut().tab_width = r.value() as u32;
            w.cfg.borrow().save();
            w.apply_all();
        });
    }
    g.add(&tab);
    g.add(&switch("Insérer des espaces", "La touche Tab insère des espaces plutôt qu'une tabulation", cfg.spaces, w, |c, v| c.spaces = v));
    page.add(&g);
    let g = adap::PreferencesGroup::new();
    g.set_title("Confort");
    g.add(&switch("Autocomplétion", "Propose les mots déjà présents dans le document", cfg.completion, w, |c, v| c.completion = v));
    g.add(&switch("Enregistrement automatique", "Les fichiers déjà enregistrés le sont à nouveau après chaque modification", cfg.autosave, w, |c, v| c.autosave = v));
    g.add(&switch("Rouvrir les fichiers au démarrage", "Retrouver les onglets et le dossier de la dernière session", cfg.restore_session, w, |c, v| c.restore_session = v));
    page.add(&g);
    dialog.add(&page);
    dialog.present(Some(&w.window));
}

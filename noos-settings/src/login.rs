//! Personnalisation → Écran de connexion : thème de noos-login (`/etc/noos/login.toml`).
//!
//! Les modifications sont rassemblées puis enregistrées d'un coup (« Appliquer ») : le
//! fichier est système. Écriture directe si `/etc/noos` est modifiable (groupe `wheel`,
//! préparé par l'installateur), sinon via pkexec.

use std::{cell::RefCell, path::{Path, PathBuf}, rc::Rc};

use adap::prelude::*;
use gtk::{gio, glib};
use noos_core::{config::ColorScheme, login::{self, LoginConfig}};

use crate::ui::{App, background as in_background, card, combo, group, hero, page, spin, switch};

struct Draft {
    cfg: LoginConfig,
    /// Nouvelle image choisie (copiée dans /etc/noos à l'application).
    new_background: Option<PathBuf>,
}

/// Sessions installées (identifiant, nom) pour le choix de la session par défaut.
fn sessions() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["/usr/share/wayland-sessions", "/usr/local/share/wayland-sessions", "/usr/share/xsessions"] {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("desktop") {
                continue;
            }
            let id = p.file_stem().unwrap().to_string_lossy().to_string();
            let name = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| t.lines().find_map(|l| l.strip_prefix("Name=").map(String::from)))
                .unwrap_or_else(|| id.clone());
            if !out.iter().any(|(i, _): &(String, String)| *i == id) {
                out.push((id, name));
            }
        }
    }
    if !out.iter().any(|(i, _)| i == "noos") {
        out.insert(0, ("noos".into(), "NOOS".into()));
    }
    out
}

fn preview(cfg: &LoginConfig, bg: Option<&Path>) -> gtk::Overlay {
    let o = gtk::Overlay::new();
    let path = bg.map(Path::to_path_buf).or_else(|| (!cfg.background.is_empty()).then(|| PathBuf::from(&cfg.background)));
    let pic = match path.filter(|p| p.is_file()) {
        Some(p) => gtk::Picture::for_filename(p),
        None => {
            let p = gtk::Picture::new();
            p.add_css_class("no-wallpaper");
            p
        }
    };
    pic.set_content_fit(gtk::ContentFit::Cover);
    pic.set_size_request(260, 146);
    pic.set_can_shrink(true);
    o.set_child(Some(&pic));
    let v = gtk::Box::new(gtk::Orientation::Vertical, 0);
    v.set_valign(gtk::Align::Start);
    v.set_margin_top(22);
    let now = glib::DateTime::now_local().ok();
    let t = gtk::Label::new(now.as_ref().map(|n| format!("{:02}:{:02}", n.hour(), n.minute())).as_deref());
    t.add_css_class("login-preview-time");
    v.append(&t);
    let d = gtk::Label::new(Some("Écran de verrouillage"));
    d.add_css_class("login-preview-date");
    v.append(&d);
    o.add_overlay(&v);
    o.set_overflow(gtk::Overflow::Hidden);
    o.add_css_class("wallpaper-preview");
    o.set_valign(gtk::Align::Center);
    o
}

pub fn login_screen(app: &Rc<App>) -> adap::NavigationPage {
    let (p, content) = page(app, &["Personnalisation"], "Écran de connexion");
    let draft = Rc::new(RefCell::new(Draft { cfg: LoginConfig::load(), new_background: None }));
    let cfg = draft.borrow().cfg.clone();

    let preview_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    preview_slot.append(&preview(&cfg, None));
    let (h, actions) = hero(
        &preview_slot,
        "Écran de connexion",
        "Fond, horloge, comptes et couleurs de l'écran de connexion NOOS. S'applique à tous les utilisateurs de ce PC.",
    );
    let apply = gtk::Button::with_label("Appliquer");
    apply.add_css_class("suggested-action");
    apply.add_css_class("pill");
    apply.set_sensitive(false);
    actions.append(&apply);
    content.append(&h);

    let changed = {
        let apply = apply.clone();
        Rc::new(move || apply.set_sensitive(true))
    };

    // ── Fond ──
    let g = group("Arrière-plan");
    let bg_text = |c: &LoginConfig| {
        if c.background.is_empty() { "Dégradé aux couleurs de NOOS".to_string() } else { c.background.clone() }
    };
    let bg_row = card("preferences-desktop-wallpaper-symbolic", "Image", &bg_text(&cfg));
    let pick = gtk::Button::with_label("Parcourir");
    pick.set_valign(gtk::Align::Center);
    let mine = gtk::Button::with_label("Mon fond d'écran");
    mine.set_valign(gtk::Align::Center);
    mine.set_tooltip_text(Some("Reprendre le fond d'écran de votre bureau"));
    let none = gtk::Button::from_icon_name("edit-clear-symbolic");
    none.set_valign(gtk::Align::Center);
    none.add_css_class("flat");
    none.set_tooltip_text(Some("Sans image (dégradé)"));
    bg_row.add_suffix(&mine);
    bg_row.add_suffix(&pick);
    bg_row.add_suffix(&none);
    g.add(&bg_row);
    let set_bg = {
        let (draft, bg_row, slot, changed) = (draft.clone(), bg_row.clone(), preview_slot.clone(), changed.clone());
        Rc::new(move |path: Option<PathBuf>| {
            let mut d = draft.borrow_mut();
            match &path {
                Some(p) => {
                    bg_row.set_subtitle(&p.to_string_lossy());
                    d.new_background = Some(p.clone());
                }
                None => {
                    bg_row.set_subtitle("Dégradé aux couleurs de NOOS");
                    d.new_background = None;
                    d.cfg.background.clear();
                }
            }
            while let Some(c) = slot.first_child() {
                slot.remove(&c);
            }
            let mut shown = d.cfg.clone();
            if path.is_none() {
                shown.background.clear();
            }
            slot.append(&preview(&shown, path.as_deref()));
            changed();
        })
    };
    {
        let (app2, set_bg) = (app.clone(), set_bg.clone());
        pick.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            filter.add_pixbuf_formats();
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Fond de l'écran de connexion").filters(&filters).build();
            let set_bg = set_bg.clone();
            dialog.open(Some(&app2.window), gio::Cancellable::NONE, move |res| {
                if let Ok(Some(path)) = res.map(|f| f.path()) {
                    set_bg(Some(path));
                }
            });
        });
    }
    {
        let (app2, set_bg) = (app.clone(), set_bg.clone());
        mine.connect_clicked(move |_| {
            let w = app2.cfg().appearance.wallpaper;
            if w.is_empty() || !Path::new(&w).is_file() {
                app2.toast("Votre bureau n'a pas d'image de fond fixe (diaporama ou image du jour ?)");
            } else {
                set_bg(Some(PathBuf::from(w)));
            }
        });
    }
    {
        let set_bg = set_bg.clone();
        none.connect_clicked(move |_| set_bg(None));
    }
    let blur = switch("image-filter-symbolic", "Flou derrière le formulaire", "Comme Windows 11 : le fond se floute à l'ouverture du formulaire", cfg.blur);
    g.add(&blur);
    let dim = spin("display-brightness-symbolic", "Assombrissement du fond", "En pourcentage (0 à 80)", (0.0, 80.0, 5.0), (cfg.dim * 100.0).round());
    g.add(&dim);
    content.append(&g);

    // ── Comportement ──
    let g = group("Affichage");
    let lock = switch("system-lock-screen-symbolic", "Écran de verrouillage", "Grande horloge d'abord, le formulaire au clic ou à une touche", cfg.lock_screen);
    let h24 = switch("preferences-system-time-symbolic", "Horloge sur 24 heures", "", cfg.clock_24h);
    let users = switch("system-users-symbolic", "Liste des comptes", "Comptes en bas à gauche (sinon, saisie du nom d'utilisateur)", cfg.show_user_list);
    g.add(&lock);
    g.add(&h24);
    g.add(&users);
    let list = sessions();
    let names: Vec<String> = list.iter().map(|(_, n)| n.clone()).collect();
    let current = list.iter().position(|(i, _)| *i == cfg.default_session).unwrap_or(0);
    let default_session = combo("computer-symbolic", "Session par défaut", "Proposée aux comptes qui ne se sont jamais connectés", &names, current);
    g.add(&default_session);
    content.append(&g);

    // ── Couleurs ──
    let g = group("Couleurs");
    let scheme = combo(
        "weather-clear-night-symbolic",
        "Mode",
        "",
        &["Sombre", "Clair"].map(String::from),
        if cfg.color_scheme == ColorScheme::Light { 1 } else { 0 },
    );
    g.add(&scheme);
    let accent_row = card("applications-graphics-symbolic", "Couleurs de mon thème", &format!("Accent actuel de l'écran de connexion : {}", cfg.accent));
    let copy = gtk::Button::with_label("Reprendre");
    copy.set_valign(gtk::Align::Center);
    copy.set_tooltip_text(Some("Palette, accent et mode de votre bureau"));
    accent_row.add_suffix(&copy);
    g.add(&accent_row);
    content.append(&g);

    // ── Liaisons ──
    {
        let (draft, changed) = (draft.clone(), changed.clone());
        blur.connect_active_notify(move |r| {
            draft.borrow_mut().cfg.blur = r.is_active();
            changed();
        });
    }
    {
        let (draft, changed) = (draft.clone(), changed.clone());
        dim.connect_value_notify(move |r| {
            draft.borrow_mut().cfg.dim = r.value() / 100.0;
            changed();
        });
    }
    for (row, field) in [(&lock, 0), (&h24, 1), (&users, 2)] {
        let (draft, changed) = (draft.clone(), changed.clone());
        row.connect_active_notify(move |r| {
            let v = r.is_active();
            let mut d = draft.borrow_mut();
            match field {
                0 => d.cfg.lock_screen = v,
                1 => d.cfg.clock_24h = v,
                _ => d.cfg.show_user_list = v,
            }
            changed();
        });
    }
    {
        let (draft, changed) = (draft.clone(), changed.clone());
        default_session.connect_selected_notify(move |r| {
            if let Some((id, _)) = list.get(r.selected() as usize) {
                draft.borrow_mut().cfg.default_session = id.clone();
                changed();
            }
        });
    }
    {
        let (draft, changed) = (draft.clone(), changed.clone());
        scheme.connect_selected_notify(move |r| {
            draft.borrow_mut().cfg.color_scheme = if r.selected() == 1 { ColorScheme::Light } else { ColorScheme::Dark };
            changed();
        });
    }
    {
        let (draft, changed, app2, accent_row, scheme) = (draft.clone(), changed.clone(), app.clone(), accent_row.clone(), scheme.clone());
        copy.connect_clicked(move |_| {
            let a = app2.cfg().appearance;
            {
                let mut d = draft.borrow_mut();
                d.cfg.accent = a.accent.clone();
                d.cfg.palette = a.palette.clone();
                d.cfg.color_scheme = if a.color_scheme == ColorScheme::Light { ColorScheme::Light } else { ColorScheme::Dark };
            }
            scheme.set_selected(if a.color_scheme == ColorScheme::Light { 1 } else { 0 });
            accent_row.set_subtitle(&format!("Accent de l'écran de connexion : {} (palette {})", a.accent, a.palette));
            changed();
        });
    }
    {
        let (draft, app2) = (draft.clone(), app.clone());
        apply.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let (cfg, img) = {
                let d = draft.borrow();
                (d.cfg.clone(), d.new_background.clone())
            };
            let (app3, btn, draft) = (app2.clone(), btn.clone(), draft.clone());
            glib::spawn_future_local(async move {
                match in_background(move || save(cfg, img)).await {
                    Ok(saved) => {
                        let mut d = draft.borrow_mut();
                        d.cfg = saved;
                        d.new_background = None;
                        app3.toast("Écran de connexion mis à jour");
                    }
                    Err(e) => {
                        btn.set_sensitive(true);
                        app3.toast(&format!("Enregistrement impossible : {e}"));
                    }
                }
            });
        });
    }
    p
}

/// Enregistre la configuration (et copie l'image) dans /etc/noos.
fn save(mut cfg: LoginConfig, image: Option<PathBuf>) -> Result<LoginConfig, String> {
    let dir = Path::new(login::DIR);
    let target = image.as_ref().map(|img| {
        let ext = img.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_lowercase();
        dir.join(format!("{}.{ext}", login::BACKGROUND_STEM))
    });
    if let Some(t) = &target {
        cfg.background = t.to_string_lossy().to_string();
    }
    if login::dir_writable() {
        if let (Some(img), Some(t)) = (&image, &target) {
            // Anciennes images (autre extension) retirées.
            remove_old_backgrounds(dir, t);
            std::fs::copy(img, t).map_err(|e| format!("copie de l'image : {e}"))?;
        }
        cfg.save_in(dir).map_err(|e| format!("{e:#}"))?;
        return Ok(cfg);
    }
    // Sinon : pkexec (demande le mot de passe administrateur).
    let tmp = std::env::temp_dir().join(format!("noos-login-{}.toml", std::process::id()));
    std::fs::write(&tmp, cfg.to_toml().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut script = format!("install -d -m 2775 -g wheel {0} && install -m 664 -g wheel '{1}' {0}/login.toml", login::DIR, tmp.display());
    if let (Some(img), Some(t)) = (&image, &target) {
        script.push_str(&format!(
            " && rm -f {0}/{1}.* && install -m 644 '{2}' '{3}'",
            login::DIR,
            login::BACKGROUND_STEM,
            img.display().to_string().replace('\'', "'\\''"),
            t.display()
        ));
    }
    let status = std::process::Command::new("pkexec").args(["sh", "-c", &script]).status();
    let _ = std::fs::remove_file(&tmp);
    match status {
        Ok(s) if s.success() => Ok(cfg),
        Ok(s) if s.code() == Some(126) || s.code() == Some(127) => {
            Err("authentification refusée ou aucun agent polkit disponible".into())
        }
        Ok(s) => Err(format!("pkexec a échoué ({s})")),
        Err(e) => Err(format!("pkexec introuvable : {e}")),
    }
}

fn remove_old_backgrounds(dir: &Path, keep: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let stem_ok = p.file_stem().and_then(|s| s.to_str()) == Some(login::BACKGROUND_STEM);
            if stem_ok && p != keep {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

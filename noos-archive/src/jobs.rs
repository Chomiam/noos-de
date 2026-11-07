//! Fenêtres de tâches : progression (pourcentage, fichier, vitesse, temps restant,
//! annulation), mot de passe, options d'extraction.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
};

use adap::prelude::*;
use gtk::{gio, glib};

use crate::engine::{self, Archive, ExtractOptions, Job, Overwrite, Progress};

pub fn human(b: u64) -> String {
    let b = b as f64;
    let (v, u) = if b >= 1e9 {
        (b / 1e9, "Go")
    } else if b >= 1e6 {
        (b / 1e6, "Mo")
    } else if b >= 1e3 {
        (b / 1e3, "Ko")
    } else {
        return format!("{b} o");
    };
    format!("{v:.1} {u}").replace('.', ",")
}

fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    if s >= 3600 {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// Résultat d'une tâche, rendu au fil de l'interface.
pub type Finish = Box<dyn FnOnce(Result<PathBuf, String>)>;

/// Lance `work` dans un fil avec une fenêtre de progression. `total` (octets) sert à
/// afficher la vitesse et le temps restant.
pub fn run(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    total: u64,
    work: impl FnOnce(&Job, &dyn Fn(Progress)) -> Result<PathBuf, String> + Send + 'static,
    finish: Finish,
) {
    let dialog = adap::Dialog::new();
    dialog.set_title(title);
    dialog.set_content_width(480);
    dialog.set_can_close(false);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 10);
    v.set_margin_top(24);
    v.set_margin_bottom(20);
    v.set_margin_start(24);
    v.set_margin_end(24);
    let head = gtk::Label::new(Some(title));
    head.add_css_class("title-3");
    head.set_xalign(0.0);
    v.append(&head);
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("arc-progress");
    v.append(&bar);
    let pct = gtk::Label::new(Some("Préparation…"));
    pct.set_xalign(0.0);
    pct.add_css_class("numeric");
    let file = gtk::Label::new(None);
    file.set_xalign(0.0);
    file.add_css_class("dim-label");
    file.add_css_class("caption");
    file.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    v.append(&pct);
    v.append(&file);
    let cancel = gtk::Button::with_label("Annuler");
    cancel.add_css_class("pill");
    cancel.set_halign(gtk::Align::End);
    cancel.set_margin_top(8);
    v.append(&cancel);
    dialog.set_child(Some(&v));
    dialog.present(Some(parent));

    let job = Job::default();
    {
        let job = job.clone();
        cancel.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Annulation…");
            job.cancel();
        });
    }
    let (tx, rx) = async_channel::unbounded::<Progress>();
    {
        let job = job.clone();
        std::thread::spawn(move || {
            let send = |p: Progress| {
                let _ = tx.send_blocking(p);
            };
            let res = work(&job, &send);
            send(Progress::Done(res));
        });
    }
    let start = Instant::now();
    let last_pulse = Cell::new(Instant::now());
    let finish = RefCell::new(Some(finish));
    let parent = parent.as_ref().clone();
    glib::spawn_future_local(async move {
        while let Ok(p) = rx.recv().await {
            match p {
                Progress::Fraction(Some(f)) => {
                    bar.set_fraction(f);
                    let elapsed = start.elapsed().as_secs_f64();
                    let mut text = format!("{:.0} %", f * 100.0);
                    if total > 0 && f > 0.01 && elapsed > 1.0 {
                        let speed = total as f64 * f / elapsed;
                        let left = elapsed * (1.0 - f) / f;
                        text.push_str(&format!("  ·  {}/s  ·  environ {} restantes", human(speed as u64), duration(left)));
                    }
                    pct.set_text(&text);
                }
                Progress::Fraction(None) => {
                    if last_pulse.get().elapsed().as_millis() > 100 {
                        bar.pulse();
                        last_pulse.set(Instant::now());
                    }
                }
                Progress::File(f) => file.set_text(&f),
                Progress::Done(res) => {
                    dialog.set_can_close(true);
                    dialog.close();
                    // Fenêtre en arrière-plan : notification à la fin d'une longue tâche.
                    if start.elapsed().as_secs() > 5 {
                        if let Some(app) = gio::Application::default() {
                            let n = gio::Notification::new(if res.is_ok() { "Archives : terminé" } else { "Archives : échec" });
                            n.set_body(Some(&format!("{} — {}", head.text(), match &res {
                                Ok(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                Err(e) => e.clone(),
                            })));
                            app.send_notification(None, &n);
                        }
                    }
                    if let Some(f) = finish.borrow_mut().take() {
                        f(res);
                    }
                    let _ = &parent;
                    break;
                }
            }
        }
    });
}

/// Demande le mot de passe d'une archive.
pub fn ask_password(parent: &impl IsA<gtk::Widget>, name: &str, wrong: bool, done: impl Fn(String) + 'static) {
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_activates_default(true);
    let d = adap::AlertDialog::new(
        Some(if wrong { "Mot de passe incorrect" } else { "Archive protégée" }),
        Some(&format!("« {name} » est chiffrée. Saisissez son mot de passe.")),
    );
    d.set_extra_child(Some(&entry));
    d.add_responses(&[("cancel", "Annuler"), ("ok", "Déverrouiller")]);
    d.set_response_appearance("ok", adap::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let e2 = entry.clone();
    d.connect_response(None, move |_, r| {
        if r == "ok" {
            done(e2.text().to_string());
        }
    });
    d.present(Some(parent));
    glib::idle_add_local_once(move || {
        entry.grab_focus();
    });
}

/// Options d'extraction : destination, sous-dossier, arborescence, fichiers existants.
pub fn extract_dialog(parent: &impl IsA<gtk::Widget>, archive: &Archive, only: Vec<String>, done: impl Fn(ExtractOptions, bool) + 'static) {
    let base = archive.path.parent().map(Path::to_path_buf).unwrap_or_else(glib::home_dir);
    let dest = Rc::new(RefCell::new(base));
    let dialog = adap::Dialog::new();
    dialog.set_title("Extraire");
    dialog.set_content_width(520);
    let tv = adap::ToolbarView::new();
    tv.add_top_bar(&adap::HeaderBar::new());
    let page = adap::PreferencesPage::new();
    let g = adap::PreferencesGroup::new();
    g.set_title(&if only.is_empty() { format!("Tout le contenu ({} fichiers)", archive.files()) } else { format!("{} élément{} sélectionné{}", only.len(), if only.len() > 1 { "s" } else { "" }, if only.len() > 1 { "s" } else { "" }) });
    let where_row = adap::ActionRow::new();
    where_row.set_title("Destination");
    where_row.set_subtitle(&dest.borrow().display().to_string());
    where_row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    let choose = gtk::Button::with_label("Changer…");
    choose.set_valign(gtk::Align::Center);
    where_row.add_suffix(&choose);
    g.add(&where_row);
    {
        let (dest, row, dialog2) = (dest.clone(), where_row.clone(), dialog.clone());
        choose.connect_clicked(move |_| {
            let fd = gtk::FileDialog::builder().title("Dossier de destination").initial_folder(&gio::File::for_path(&*dest.borrow())).build();
            let (dest, row) = (dest.clone(), row.clone());
            let win = dialog2.root().and_downcast::<gtk::Window>();
            fd.select_folder(win.as_ref(), gio::Cancellable::NONE, move |r| {
                if let Some(p) = r.ok().and_then(|f| f.path()) {
                    row.set_subtitle(&p.display().to_string());
                    *dest.borrow_mut() = p;
                }
            });
        });
    }
    let sub = adap::SwitchRow::new();
    sub.set_title(&format!("Dans un dossier « {} »", engine::stem(&archive.path)));
    sub.set_subtitle("Évite de mélanger le contenu avec les fichiers déjà présents");
    sub.set_active(engine::smart_destination(archive, Path::new("/x")) != Path::new("/x"));
    g.add(&sub);
    let paths = adap::SwitchRow::new();
    paths.set_title("Conserver les dossiers");
    paths.set_subtitle("Désactivé : tous les fichiers à plat dans la destination");
    paths.set_active(true);
    g.add(&paths);
    let over = adap::ComboRow::new();
    over.set_title("Si un fichier existe déjà");
    over.set_model(Some(&gtk::StringList::new(&["Le remplacer", "Le garder (ignorer)", "Garder les deux (renommer)"])));
    g.add(&over);
    let open = adap::SwitchRow::new();
    open.set_title("Ouvrir le dossier à la fin");
    open.set_active(true);
    g.add(&open);
    page.add(&g);
    let pw = adap::PasswordEntryRow::new();
    if archive.encrypted() && archive.password.is_none() {
        let g2 = adap::PreferencesGroup::new();
        g2.set_title("Archive chiffrée");
        pw.set_title("Mot de passe");
        g2.add(&pw);
        page.add(&g2);
    }
    let go = gtk::Button::with_label("Extraire");
    go.add_css_class("suggested-action");
    go.add_css_class("pill");
    go.set_halign(gtk::Align::Center);
    go.set_margin_bottom(18);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.append(&page);
    body.append(&go);
    tv.set_content(Some(&body));
    dialog.set_child(Some(&tv));
    let archive = archive.clone();
    let d2 = dialog.clone();
    go.connect_clicked(move |_| {
        let base = dest.borrow().clone();
        let target = if sub.is_active() {
            let mut d = base.join(engine::stem(&archive.path));
            let mut i = 2;
            while d.exists() {
                d = base.join(format!("{} ({i})", engine::stem(&archive.path)));
                i += 1;
            }
            d
        } else {
            base
        };
        let o = ExtractOptions {
            dest: target,
            only: only.clone(),
            keep_paths: paths.is_active(),
            overwrite: match over.selected() {
                1 => Overwrite::Skip,
                2 => Overwrite::Rename,
                _ => Overwrite::Replace,
            },
            password: Some(pw.text().to_string()).filter(|p| !p.is_empty()).or_else(|| archive.password.clone()),
        };
        d2.close();
        done(o, open.is_active());
    });
    dialog.present(Some(parent));
}

/// Ouvre un dossier dans l'Explorateur.
pub fn show_folder(path: &Path) {
    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>);
}

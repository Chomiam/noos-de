//! Copies et déplacements avec suivi de la progression (comme la fenêtre de copie de Windows).
//!
//! Chaque opération tourne dans un fil de travail (GIO bloquant, annulable) et envoie son
//! avancement à l'interface : une carte par opération, en bas à droite de l'Explorateur, avec
//! pourcentage, octets copiés, vitesse, temps restant, fichier en cours et bouton Annuler.
//!
//! - Déplacement sur le même disque : simple renommage (instantané) ;
//! - entre deux disques (ou si le renommage est impossible) : copie puis suppression ;
//! - nom déjà pris : « nom (2).ext ».

use std::time::{Duration, Instant};

use adap::prelude::*;
use gtk::{gio, glib};

use crate::human_size;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Copy,
    Move,
}

enum Msg {
    /// Analyse terminée : nombre de fichiers et taille totale.
    Scanned { items: u64, bytes: u64 },
    Progress { bytes: u64, items: u64, current: String },
    Done { errors: Vec<String>, cancelled: bool },
}

/// Identifiant du système de fichiers (déplacement = copie entre deux disques).
pub fn fs_id(f: &gio::File) -> String {
    f.query_info("id::filesystem", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
        .ok()
        .and_then(|i| i.attribute_string("id::filesystem").map(|s| s.to_string()))
        .unwrap_or_default()
}

fn kind_of(f: &gio::File) -> gio::FileType {
    f.query_file_type(gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)
}

/// Taille totale et nombre de fichiers (dossiers parcourus).
fn scan(f: &gio::File, cancel: &gio::Cancellable, items: &mut u64, bytes: &mut u64) {
    if cancel.is_cancelled() {
        return;
    }
    let Ok(info) = f.query_info("standard::type,standard::size", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel))
    else {
        return;
    };
    if info.file_type() == gio::FileType::Directory {
        if let Ok(children) = f.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(cancel)) {
            for child in children.flatten() {
                scan(&f.child(child.name()), cancel, items, bytes);
            }
        }
    } else {
        *items += 1;
        *bytes += info.size().max(0) as u64;
    }
}

/// Premier nom libre dans `dest` : « nom », puis « nom (2).ext »…
fn free_name(dest: &gio::File, name: &str) -> gio::File {
    let mut target = dest.child(name);
    if !target.query_exists(gio::Cancellable::NONE) {
        return target;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(p) if p > 0 => (&name[..p], &name[p..]),
        _ => (name, ""),
    };
    let mut k = 2;
    while target.query_exists(gio::Cancellable::NONE) {
        target = dest.child(format!("{stem} ({k}){ext}"));
        k += 1;
    }
    target
}

/// Suppression définitive (dossiers compris).
pub fn delete_recursive(f: &gio::File, cancel: Option<&gio::Cancellable>) -> Result<(), glib::Error> {
    if kind_of(f) == gio::FileType::Directory && !f.uri().starts_with("trash:") {
        for info in f.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, cancel)? {
            delete_recursive(&f.child(info?.name()), cancel)?;
        }
    }
    f.delete(cancel)
}

/// Fil de travail : copie (récursive) avec retour de progression.
struct Worker {
    tx: async_channel::Sender<Msg>,
    cancel: gio::Cancellable,
    bytes: u64,
    items: u64,
    last_send: Instant,
}

impl Worker {
    fn report(&mut self, current: &str, force: bool) {
        if force || self.last_send.elapsed() >= Duration::from_millis(80) {
            self.last_send = Instant::now();
            let _ = self.tx.send_blocking(Msg::Progress { bytes: self.bytes, items: self.items, current: current.to_string() });
        }
    }

    fn copy(&mut self, src: &gio::File, dst: &gio::File) -> Result<(), glib::Error> {
        let name = src.basename().map(|b| b.to_string_lossy().to_string()).unwrap_or_default();
        if kind_of(src) == gio::FileType::Directory {
            dst.make_directory(Some(&self.cancel))?;
            let children = src.enumerate_children("standard::name", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, Some(&self.cancel))?;
            for child in children {
                let child = child?;
                self.copy(&src.child(child.name()), &dst.child(child.name()))?;
            }
            return Ok(());
        }
        let base = self.bytes;
        let items_done = self.items;
        let mut last = 0u64;
        let mut last_send = self.last_send;
        {
            let tx = self.tx.clone();
            let mut progress = |current: i64, _total: i64| {
                last = current.max(0) as u64;
                if last_send.elapsed() >= Duration::from_millis(80) {
                    last_send = Instant::now();
                    let _ = tx.send_blocking(Msg::Progress { bytes: base + last, items: items_done, current: name.clone() });
                }
            };
            src.copy(dst, gio::FileCopyFlags::NOFOLLOW_SYMLINKS | gio::FileCopyFlags::ALL_METADATA, Some(&self.cancel), Some(&mut progress))?;
        }
        let size = src
            .query_info("standard::size", gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS, gio::Cancellable::NONE)
            .map(|i| i.size().max(0) as u64)
            .unwrap_or(last);
        self.bytes = base + size.max(last);
        self.items += 1;
        self.last_send = last_send;
        self.report(&name, false);
        Ok(())
    }

    fn run(mut self, files: Vec<gio::File>, dest: gio::File, kind: Kind) {
        let (mut items, mut bytes) = (0, 0);
        for f in &files {
            scan(f, &self.cancel, &mut items, &mut bytes);
        }
        let _ = self.tx.send_blocking(Msg::Scanned { items, bytes });
        let dest_fs = fs_id(&dest);
        let mut errors = Vec::new();
        for src in &files {
            if self.cancel.is_cancelled() {
                break;
            }
            let Some(name) = src.basename().map(|b| b.to_string_lossy().to_string()) else { continue };
            let target = free_name(&dest, &name);
            let res = if kind == Kind::Move && fs_id(src) == dest_fs {
                // Même disque : renommage instantané ; sinon (cas rares) copie + suppression.
                let (mut i, mut b) = (0, 0);
                scan(src, &self.cancel, &mut i, &mut b);
                match src.move_(&target, gio::FileCopyFlags::NOFOLLOW_SYMLINKS, Some(&self.cancel), None) {
                    Ok(()) => {
                        self.bytes += b;
                        self.items += i;
                        self.report(&name, true);
                        Ok(())
                    }
                    Err(_) => self.copy(src, &target).and_then(|_| delete_recursive(src, Some(&self.cancel))),
                }
            } else {
                let r = self.copy(src, &target);
                if r.is_ok() && kind == Kind::Move {
                    delete_recursive(src, Some(&self.cancel))
                } else {
                    r
                }
            };
            if let Err(e) = res {
                if !e.matches(gio::IOErrorEnum::Cancelled) {
                    errors.push(format!("{name} : {}", e.message()));
                }
            }
        }
        let cancelled = self.cancel.is_cancelled();
        let _ = self.tx.send_blocking(Msg::Done { errors, cancelled });
    }
}

fn duration_label(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} min", s / 3600, (s % 3600) / 60),
    }
}

/// Panneau des opérations en cours (cartes empilées en bas à droite).
pub struct Panel {
    pub widget: gtk::Box,
}

impl Panel {
    pub fn new() -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 8);
        widget.add_css_class("transfers");
        widget.set_halign(gtk::Align::End);
        widget.set_valign(gtk::Align::End);
        widget.set_margin_end(16);
        widget.set_margin_bottom(16);
        Self { widget }
    }

    /// Lance une copie ou un déplacement ; `done(nombre, erreurs)` à la fin.
    pub fn start(&self, files: Vec<gio::File>, dest: gio::File, kind: Kind, dest_label: &str, done: impl Fn(usize, &[String]) + 'static) {
        let count = files.len();
        let cancel = gio::Cancellable::new();
        let (tx, rx) = async_channel::unbounded::<Msg>();

        // ── Carte ──
        let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
        card.add_css_class("transfer-card");
        card.set_size_request(380, -1);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = gtk::Image::from_icon_name(if kind == Kind::Copy { "edit-copy-symbolic" } else { "folder-move-symbolic" });
        icon.add_css_class("transfer-icon");
        top.append(&icon);
        let titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        titles.set_hexpand(true);
        let verb = if kind == Kind::Copy { "Copie" } else { "Déplacement" };
        let title = gtk::Label::new(Some(&format!("{verb} de {count} élément{} vers {dest_label}", if count > 1 { "s" } else { "" })));
        title.add_css_class("heading");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        titles.append(&title);
        let current = gtk::Label::new(Some("Analyse des fichiers…"));
        current.add_css_class("caption");
        current.add_css_class("dim-label");
        current.set_xalign(0.0);
        current.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        titles.append(&current);
        top.append(&titles);
        let cancel_btn = gtk::Button::from_icon_name("window-close-symbolic");
        cancel_btn.add_css_class("flat");
        cancel_btn.add_css_class("circular");
        cancel_btn.set_valign(gtk::Align::Start);
        cancel_btn.set_tooltip_text(Some("Annuler"));
        top.append(&cancel_btn);
        card.append(&top);
        let bar = gtk::ProgressBar::new();
        bar.set_pulse_step(0.08);
        card.append(&bar);
        let details = gtk::Label::new(None);
        details.add_css_class("caption");
        details.set_xalign(0.0);
        card.append(&details);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .transition_duration(180)
            .child(&card)
            .build();
        self.widget.append(&revealer);
        revealer.set_reveal_child(true);
        {
            let cancel = cancel.clone();
            cancel_btn.connect_clicked(move |b| {
                cancel.cancel();
                b.set_sensitive(false);
            });
        }

        // ── Fil de travail ──
        {
            let cancel = cancel.clone();
            std::thread::Builder::new()
                .name("transfert".into())
                .spawn(move || {
                    Worker { tx, cancel, bytes: 0, items: 0, last_send: Instant::now() }.run(files, dest, kind);
                })
                .expect("fil de transfert");
        }

        // ── Suivi dans l'interface ──
        let panel = self.widget.clone();
        glib::spawn_future_local(async move {
            let start = Instant::now();
            let (mut total_bytes, mut total_items) = (0u64, 0u64);
            let mut speed = 0.0f64; // octets/s, lissée
            let (mut last_t, mut last_b) = (start, 0u64);
            let pulse = {
                let bar = bar.clone();
                glib::timeout_add_local(Duration::from_millis(120), move || {
                    bar.pulse();
                    glib::ControlFlow::Continue
                })
            };
            let mut pulse = Some(pulse);
            while let Ok(msg) = rx.recv().await {
                match msg {
                    Msg::Scanned { items, bytes } => {
                        total_bytes = bytes;
                        total_items = items;
                        if let Some(p) = pulse.take() {
                            p.remove();
                        }
                        bar.set_fraction(0.0);
                        details.set_text(&format!("{} élément{} — {}", items, if items > 1 { "s" } else { "" }, human_size(bytes as i64)));
                    }
                    Msg::Progress { bytes, items, current: name } => {
                        current.set_text(&name);
                        let now = Instant::now();
                        let dt = now.duration_since(last_t).as_secs_f64();
                        if dt >= 0.25 {
                            let inst = (bytes.saturating_sub(last_b)) as f64 / dt;
                            speed = if speed == 0.0 { inst } else { speed * 0.7 + inst * 0.3 };
                            last_t = now;
                            last_b = bytes;
                        }
                        if total_bytes > 0 {
                            let frac = (bytes as f64 / total_bytes as f64).clamp(0.0, 1.0);
                            bar.set_fraction(frac);
                            let remaining = if speed > 1.0 {
                                format!(" — encore {}", duration_label((total_bytes.saturating_sub(bytes)) as f64 / speed))
                            } else {
                                String::new()
                            };
                            let rate = if speed > 1.0 { format!(" — {}/s", human_size(speed as i64)) } else { String::new() };
                            details.set_text(&format!(
                                "{:.0} % — {items}/{total_items} — {} sur {}{rate}{remaining}",
                                frac * 100.0,
                                human_size(bytes as i64),
                                human_size(total_bytes as i64)
                            ));
                        }
                    }
                    Msg::Done { errors, cancelled } => {
                        if let Some(p) = pulse.take() {
                            p.remove();
                        }
                        cancel_btn.set_visible(false);
                        bar.set_fraction(1.0);
                        let elapsed = start.elapsed().as_secs_f64();
                        if cancelled {
                            current.set_text("Annulé");
                            card.add_css_class("cancelled");
                        } else if errors.is_empty() {
                            current.set_text(&format!(
                                "Terminé en {} — {} élément{}",
                                duration_label(elapsed.max(1.0)),
                                total_items,
                                if total_items > 1 { "s" } else { "" }
                            ));
                            card.add_css_class("finished");
                            let avg = total_bytes as f64 / elapsed.max(0.001);
                            let verb = if kind == Kind::Copy { "copiés" } else { "déplacés" };
                            details.set_text(&if total_bytes > 0 && elapsed >= 0.5 {
                                format!("{} {verb} — {}/s en moyenne", human_size(total_bytes as i64), human_size(avg as i64))
                            } else {
                                format!("{} {verb}", human_size(total_bytes as i64))
                            });
                        } else {
                            current.set_text(&format!("{} erreur{}", errors.len(), if errors.len() > 1 { "s" } else { "" }));
                            details.set_text(&errors.join("\n"));
                            details.set_wrap(true);
                            card.add_css_class("failed");
                        }
                        done(count, &errors);
                        // La carte reste quelques secondes (plus longtemps en cas d'erreur).
                        let delay = if errors.is_empty() { 4 } else { 12 };
                        let (revealer, panel) = (revealer.clone(), panel.clone());
                        glib::timeout_add_local_once(Duration::from_secs(delay), move || {
                            revealer.set_reveal_child(false);
                            glib::timeout_add_local_once(Duration::from_millis(250), move || panel.remove(&revealer));
                        });
                        break;
                    }
                }
            }
        });
    }
}

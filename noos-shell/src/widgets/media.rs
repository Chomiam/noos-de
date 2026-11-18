//! Lecteur multimédia : ce que joue un lecteur MPRIS (Firefox, Spotify, VLC, mpv…) —
//! pochette (et fond flouté), titre, artiste, progression, précédent / lecture / suivant.
//!
//! Le lecteur est relu sur le bus de session chaque seconde (appels asynchrones), seulement
//! tant que le widget existe ; celui qui joue a la priorité.

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, every};

pub const CSS: &str = r#"
.kind-media { padding: 0; }
.desk-media { padding: 16px 18px; }
.desk-media-bg { filter: blur(26px) saturate(1.4); opacity: 0.55; }
.desk-media-art { border-radius: 12px; box-shadow: 0 6px 18px rgba(0,0,0,0.35); }
.desk-media-art.placeholder { background-image: linear-gradient(135deg, @accent_bg_color, shade(@accent_bg_color, 0.55)); color: white; }
.desk-media-title { font-weight: 700; font-size: 1.1em; }
.size-l .desk-media-title { font-size: 1.3em; }
.desk-media-artist { opacity: 0.75; }
.desk-media-player { font-size: 0.75em; font-weight: 700; opacity: 0.6; letter-spacing: 0.05em; }
.desk-media-time { font-size: 0.75em; opacity: 0.65; font-feature-settings: "tnum"; }
.desk-media progressbar trough { min-height: 4px; border-radius: 99px; }
.desk-media progressbar progress { min-height: 4px; border-radius: 99px; }
.desk-media-play { min-width: 40px; min-height: 40px; border-radius: 99px; background-color: @accent_bg_color; color: @accent_fg_color; }
.desk-media-play.big { min-width: 52px; min-height: 52px; }
.size-l .desk-media-title { font-size: 1.35em; }
.desk-media-btn { min-width: 32px; min-height: 32px; border-radius: 99px; }
"#;

#[derive(Default, Clone, PartialEq)]
struct Track {
    player: String,
    bus: String,
    title: String,
    artist: String,
    art: String,
    playing: bool,
    /// Microsecondes.
    length: i64,
    position: i64,
}

fn demo() -> bool {
    std::env::var_os("NOOS_MEDIA_DEMO").is_some()
}

/// Nom lisible du lecteur d'après son nom de bus.
fn player_name(bus: &str) -> String {
    let id = bus.trim_start_matches("org.mpris.MediaPlayer2.").split('.').next().unwrap_or("");
    match id {
        "vlc" => "VLC".into(),
        "mpv" => "mpv".into(),
        "" => "Lecteur".into(),
        other => super::capitalize(other),
    }
}

fn fmt_time(us: i64) -> String {
    let s = (us / 1_000_000).max(0);
    format!("{}:{:02}", s / 60, s % 60)
}

/// Lecteurs MPRIS présents sur le bus.
async fn players(conn: &gio::DBusConnection) -> Vec<String> {
    let Ok(v) = conn
        .call_future(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", "org.freedesktop.DBus", "ListNames", None, Some(glib::VariantTy::new("(as)").unwrap()), gio::DBusCallFlags::NONE, 2000)
        .await
    else {
        return vec![];
    };
    v.child_value(0).get::<Vec<String>>().unwrap_or_default().into_iter().filter(|n| n.starts_with("org.mpris.MediaPlayer2.")).collect()
}

async fn read(conn: &gio::DBusConnection, bus: &str) -> Option<Track> {
    let v = conn
        .call_future(
            Some(bus),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&("org.mpris.MediaPlayer2.Player",).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            gio::DBusCallFlags::NONE,
            2000,
        )
        .await
        .ok()?;
    let d = glib::VariantDict::new(Some(&v.child_value(0)));
    let status: String = d.lookup("PlaybackStatus").ok().flatten().unwrap_or_default();
    let position: i64 = d.lookup("Position").ok().flatten().unwrap_or(0);
    let meta = d.lookup_value("Metadata", None).map(|m| glib::VariantDict::new(Some(&m)));
    let get_s = |k: &str| meta.as_ref().and_then(|m| m.lookup::<String>(k).ok().flatten()).unwrap_or_default();
    let artist = meta
        .as_ref()
        .and_then(|m| m.lookup::<Vec<String>>("xesam:artist").ok().flatten())
        .map(|a| a.join(", "))
        .unwrap_or_default();
    let length = meta
        .as_ref()
        .and_then(|m| m.lookup_value("mpris:length", None))
        .and_then(|v| v.get::<i64>().or_else(|| v.get::<u64>().map(|u| u as i64)))
        .unwrap_or(0);
    Some(Track { player: player_name(bus), bus: bus.to_string(), title: get_s("xesam:title"), artist, art: get_s("mpris:artUrl"), playing: status == "Playing", length, position })
}

/// Pochette locale : fichier (file://) tel quel, image en ligne téléchargée dans le cache.
fn art_file(url: &str, done: impl FnOnce(Option<gio::File>) + 'static) {
    if url.is_empty() {
        return done(None);
    }
    if url.starts_with("file://") {
        return done(Some(gio::File::for_uri(url)));
    }
    if !url.starts_with("http") {
        return done(None);
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let path = noos_core::paths::cache_dir().join("media-art").join(format!("{h:016x}"));
    if path.exists() {
        return done(Some(gio::File::for_path(&path)));
    }
    let (tx, rx) = async_channel::bounded(1);
    let url = url.to_string();
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let ok = std::process::Command::new("curl").args(["-fsSL", "--max-time", "15", "-o"]).arg(&path).arg(&url).status().is_ok_and(|s| s.success());
        let _ = tx.send_blocking(ok.then_some(path));
    });
    glib::spawn_future_local(async move {
        done(rx.recv().await.ok().flatten().map(|p| gio::File::for_path(&p)));
    });
}

pub fn build(_ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let size = w.size().to_string();
    let (small, large) = (size == "s", size == "l");
    let art_px = by_size(w, 52, 84, 250);
    let o = gtk::Overlay::new();
    o.set_overflow(gtk::Overflow::Hidden);
    let bg = gtk::Picture::new();
    bg.set_content_fit(gtk::ContentFit::Cover);
    bg.add_css_class("desk-media-bg");
    bg.set_can_shrink(true);
    o.set_child(Some(&bg));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("desk-media");
    row.set_size_request(by_size(w, 280, 340, 290), -1);
    // Grande taille : carte d'album — pochette en haut, texte et commandes centrés dessous.
    if large {
        row.set_orientation(gtk::Orientation::Vertical);
        row.set_spacing(14);
    }
    o.add_overlay(&row);
    o.set_measure_overlay(&row, true);
    // Pochette (dégradé + note de musique tant qu'il n'y en a pas).
    let art = gtk::Overlay::new();
    art.add_css_class("desk-media-art");
    art.add_css_class("placeholder");
    art.set_overflow(gtk::Overflow::Hidden);
    art.set_size_request(art_px, art_px);
    art.set_valign(gtk::Align::Center);
    art.set_halign(gtk::Align::Center);
    let note = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    note.set_pixel_size(art_px / 2);
    art.set_child(Some(&note));
    let pic = gtk::Picture::new();
    pic.set_content_fit(gtk::ContentFit::Cover);
    art.add_overlay(&pic);
    row.append(&art);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 3);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let player = gtk::Label::new(None);
    player.add_css_class("desk-media-player");
    player.set_xalign(0.0);
    col.append(&player);
    let title = gtk::Label::new(Some("Rien en lecture"));
    title.add_css_class("desk-media-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(if small { 15 } else { 22 });
    if small {
        title.set_width_chars(15);
    }
    col.append(&title);
    let artist = gtk::Label::new(Some("Lancez de la musique ou une vidéo"));
    artist.add_css_class("desk-media-artist");
    artist.set_xalign(0.0);
    artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    artist.set_max_width_chars(if small { 18 } else { 26 });
    col.append(&artist);
    let bar = gtk::ProgressBar::new();
    bar.set_margin_top(6);
    col.append(&bar);
    let times = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let pos_l = gtk::Label::new(None);
    pos_l.add_css_class("desk-media-time");
    pos_l.set_hexpand(true);
    pos_l.set_xalign(0.0);
    let len_l = gtk::Label::new(None);
    len_l.add_css_class("desk-media-time");
    times.append(&pos_l);
    times.append(&len_l);
    col.append(&times);
    let ctl = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    ctl.set_halign(gtk::Align::Center);
    let prev = gtk::Button::from_icon_name("media-skip-backward-symbolic");
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    let next = gtk::Button::from_icon_name("media-skip-forward-symbolic");
    for (b, tip) in [(&prev, "Précédent"), (&next, "Suivant")] {
        b.add_css_class("flat");
        b.add_css_class("desk-media-btn");
        b.set_tooltip_text(Some(tip));
    }
    play.add_css_class("desk-media-play");
    ctl.append(&prev);
    ctl.append(&play);
    ctl.append(&next);
    if large {
        for l in [&player, &title, &artist] {
            l.set_xalign(0.5);
        }
        ctl.set_spacing(18);
        play.add_css_class("big");
    }
    row.append(&col);
    // Petite taille : l'essentiel — pochette, titre, artiste et lecture / pause à côté.
    if small {
        prev.set_visible(false);
        next.set_visible(false);
        ctl.set_valign(gtk::Align::Center);
        row.append(&ctl);
    } else {
        col.append(&ctl);
    }

    let state: Rc<RefCell<Track>> = Rc::new(RefCell::new(Track::default()));
    let shown_art = Rc::new(RefCell::new(String::new()));
    let apply = {
        let (title, artist, player, bar, pos_l, len_l, play, pic, bg, art, shown_art, ctl, times) =
            (title.clone(), artist.clone(), player.clone(), bar.clone(), pos_l.clone(), len_l.clone(), play.clone(), pic.clone(), bg.clone(), art.clone(), shown_art.clone(), ctl.clone(), times.clone());
        Rc::new(move |t: &Track| {
            let none = t.bus.is_empty();
            title.set_text(if none { "Rien en lecture" } else if t.title.is_empty() { "Titre inconnu" } else { &t.title });
            artist.set_text(if none { "Lancez de la musique ou une vidéo" } else { &t.artist });
            artist.set_visible(none || !t.artist.is_empty());
            player.set_text(&t.player.to_uppercase());
            player.set_visible(!none && !small);
            ctl.set_sensitive(!none);
            let has_len = t.length > 0;
            bar.set_visible(has_len && !small);
            times.set_visible(has_len && !small);
            if has_len {
                bar.set_fraction((t.position as f64 / t.length as f64).clamp(0.0, 1.0));
                pos_l.set_text(&fmt_time(t.position));
                len_l.set_text(&fmt_time(t.length));
            }
            play.set_icon_name(if t.playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
            play.set_tooltip_text(Some(if t.playing { "Pause" } else { "Lecture" }));
            if *shown_art.borrow() != t.art {
                *shown_art.borrow_mut() = t.art.clone();
                let (pic, bg, art) = (pic.clone(), bg.clone(), art.clone());
                art_file(&t.art, move |f| {
                    pic.set_file(f.as_ref());
                    bg.set_file(f.as_ref());
                    if f.is_some() {
                        art.remove_css_class("placeholder");
                    } else {
                        art.add_css_class("placeholder");
                    }
                });
            }
        })
    };
    if demo() {
        let t = Track { player: "Lecteur".into(), bus: "demo".into(), title: "Lumières de la ville".into(), artist: "Ensemble Aurore".into(), art: String::new(), playing: true, length: 228_000_000, position: 72_000_000 };
        *state.borrow_mut() = t.clone();
        apply(&t);
        let (st, ap) = (state.clone(), apply.clone());
        every(&o, 1, move || {
            let mut t = st.borrow_mut();
            if t.playing {
                t.position = (t.position + 1_000_000) % t.length;
            }
            ap(&t);
        });
        let st = state.clone();
        let a2 = apply.clone();
        play.connect_clicked(move |_| {
            let mut t = st.borrow_mut();
            t.playing = !t.playing;
            a2(&t);
        });
        return o.upcast();
    }
    let Ok(conn) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return o.upcast();
    };
    // Commandes.
    for (btn, method) in [(&prev, "Previous"), (&play, "PlayPause"), (&next, "Next")] {
        let (conn, state) = (conn.clone(), state.clone());
        btn.connect_clicked(move |_| {
            let bus = state.borrow().bus.clone();
            if bus.is_empty() {
                return;
            }
            conn.call(Some(&bus), "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player", method, None, None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE, |_| {});
        });
    }
    let busy = Rc::new(std::cell::Cell::new(false));
    let tick = {
        let (conn, state, apply, busy) = (conn.clone(), state.clone(), apply.clone(), busy.clone());
        let n = Rc::new(std::cell::Cell::new(0u32));
        move || {
            if busy.replace(true) {
                return;
            }
            let (conn, state, apply, busy) = (conn.clone(), state.clone(), apply.clone(), busy.clone());
            let rescan = n.get() % 3 == 0 || state.borrow().bus.is_empty();
            n.set(n.get() + 1);
            glib::spawn_future_local(async move {
                let current = state.borrow().bus.clone();
                let mut best: Option<Track> = None;
                if rescan {
                    // Celui qui joue d'abord, sinon le lecteur déjà suivi, sinon le premier.
                    for bus in players(&conn).await {
                        if let Some(t) = read(&conn, &bus).await {
                            let better = match &best {
                                None => true,
                                Some(b) => (t.playing && !b.playing) || (t.playing == b.playing && t.bus == current),
                            };
                            if better {
                                best = Some(t);
                            }
                        }
                    }
                } else {
                    best = read(&conn, &current).await;
                }
                let t = best.unwrap_or_default();
                if *state.borrow() != t {
                    *state.borrow_mut() = t.clone();
                    apply(&t);
                }
                busy.set(false);
            });
        }
    };
    tick();
    every(&o, 1, tick);
    o.upcast()
}

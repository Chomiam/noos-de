//! Cadre photo : les images d'un dossier (Images par défaut) en fondu enchaîné ; un clic
//! ouvre l'image affichée dans Photos.

use std::{cell::Cell, path::PathBuf, rc::Rc};

use gtk::{gio, glib, prelude::*};
use noos_core::config::DesktopWidget;

use super::{Ctx, by_size, choice, every, open_path, section, switch};

pub const CSS: &str = r#"
.kind-photo { padding: 6px; }
.desk-photo { border-radius: 13px; }
.desk-photo picture { border-radius: 13px; }
.desk-photo-caption { margin: 8px; padding: 3px 10px; border-radius: 99px; background-color: rgba(0,0,0,0.45); color: white; font-size: 0.82em; opacity: 0; transition: opacity 200ms; }
.desk-photo:hover .desk-photo-caption { opacity: 1; }
.desk-photo-empty { padding: 18px; }
"#;

fn folder(w: &DesktopWidget) -> PathBuf {
    w.option("folder").map(PathBuf::from).unwrap_or_else(noos_core::paths::pictures_dir)
}

/// Images du dossier (sans sous-dossiers), triées par nom.
fn images(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp" | "avif" | "heic")))
        .take(2000)
        .collect();
    v.sort();
    v
}

pub fn build(ctx: &Ctx, w: &DesktopWidget) -> gtk::Widget {
    let (pw, ph) = (by_size(w, 220, 330, 480), by_size(w, 150, 220, 320));
    let dir = folder(w);
    let mut list = images(&dir);
    if list.is_empty() {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
        b.add_css_class("desk-photo-empty");
        b.set_size_request(pw, ph);
        b.set_valign(gtk::Align::Center);
        let i = gtk::Image::from_icon_name("image-x-generic-symbolic");
        i.set_pixel_size(42);
        b.append(&i);
        let l = gtk::Label::new(Some(&format!("Aucune image dans « {} »", dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())));
        l.set_wrap(true);
        l.add_css_class("desk-dim");
        b.append(&l);
        let btn = gtk::Button::with_label("Choisir un dossier");
        btn.add_css_class("pill");
        btn.set_halign(gtk::Align::Center);
        let c = ctx.clone();
        btn.connect_clicked(move |_| c.open_settings());
        b.append(&btn);
        return b.upcast();
    }
    if w.option("shuffle") != Some("0") {
        for i in (1..list.len()).rev() {
            list.swap(i, glib::random_int_range(0, i as i32 + 1) as usize);
        }
    }
    let list = Rc::new(list);
    let o = gtk::Overlay::new();
    o.add_css_class("desk-photo");
    o.set_overflow(gtk::Overflow::Hidden);
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(1400);
    let pics = [gtk::Picture::new(), gtk::Picture::new()];
    for (i, p) in pics.iter().enumerate() {
        p.set_content_fit(gtk::ContentFit::Cover);
        p.set_size_request(pw, ph);
        stack.add_named(p, Some(&i.to_string()));
    }
    o.set_child(Some(&stack));
    let caption = gtk::Label::new(None);
    caption.add_css_class("desk-photo-caption");
    caption.set_halign(gtk::Align::Start);
    caption.set_valign(gtk::Align::End);
    caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    caption.set_max_width_chars(30);
    o.add_overlay(&caption);
    let index = Rc::new(Cell::new(0usize));
    let front = Rc::new(Cell::new(0usize));
    let show = {
        let (list, index, front, stack, caption) = (list.clone(), index.clone(), front.clone(), stack.clone(), caption.clone());
        Rc::new(move |next: bool| {
            if next {
                index.set((index.get() + 1) % list.len());
            }
            let path = &list[index.get()];
            let back = 1 - front.get();
            let pic = &stack.child_by_name(&back.to_string()).and_downcast::<gtk::Picture>();
            if let Some(pic) = pic {
                pic.set_file(Some(&gio::File::for_path(path)));
                stack.set_visible_child(pic);
            }
            front.set(back);
            caption.set_text(&path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        })
    };
    show(false);
    let minutes: u32 = w.option("interval").and_then(|v| v.parse().ok()).unwrap_or(5).max(1);
    {
        let show = show.clone();
        every(&o, minutes * 60, move || show(true));
    }
    // Clic : ouvrir l'image ; molette : image suivante / précédente.
    let click = gtk::GestureClick::new();
    {
        let (list, index) = (list.clone(), index.clone());
        click.connect_released(move |_, n, _, _| {
            if n == 1 {
                open_path(&list[index.get()]);
            }
        });
    }
    o.add_controller(click);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE);
    {
        let (show, index, list) = (show.clone(), index.clone(), list.clone());
        scroll.connect_scroll(move |_, _, dy| {
            if dy < 0.0 {
                index.set((index.get() + list.len() - 2) % list.len());
            }
            show(true);
            glib::Propagation::Stop
        });
    }
    o.add_controller(scroll);
    o.set_tooltip_text(Some("Clic : ouvrir · molette : image suivante"));
    o.upcast()
}

pub fn settings(ctx: &Ctx, w: &DesktopWidget, b: &gtk::Box) {
    section(b, "Dossier");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let dir = folder(w);
    let l = gtk::Label::new(Some(&dir.to_string_lossy()));
    l.set_hexpand(true);
    l.set_xalign(0.0);
    l.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    l.add_css_class("desk-dim");
    row.append(&l);
    let btn = gtk::Button::with_label("Changer…");
    let c = ctx.clone();
    btn.connect_clicked(move |btn| {
        let d = gtk::FileDialog::builder().title("Dossier d'images").initial_folder(&gio::File::for_path(&dir)).build();
        let c = c.clone();
        let win = btn.root().and_downcast::<gtk::Window>();
        d.select_folder(win.as_ref(), gio::Cancellable::NONE, move |r| {
            if let Some(p) = r.ok().and_then(|f| f.path()) {
                c.set_option("Dossier du cadre photo", "folder", &p.to_string_lossy());
            }
        });
    });
    row.append(&btn);
    b.append(&row);
    section(b, "Image suivante toutes les");
    let c = ctx.clone();
    choice(b, &[("1", "1 min"), ("5", "5 min"), ("15", "15 min"), ("60", "1 h")], w.option("interval").unwrap_or("5"), move |v| c.set_option("Intervalle du cadre photo", "interval", v));
    let c = ctx.clone();
    switch(b, "Ordre aléatoire", w.option("shuffle") != Some("0"), move |on| c.set_option("Ordre du cadre photo", "shuffle", if on { "1" } else { "0" }));
}

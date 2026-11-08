//! Volume (bouton haut-parleur de la barre) : volume général, périphériques de sortie et
//! d'entrée, mélangeur par application.
//!
//! Lecture : `pw-dump` (JSON de PipeWire, noms accentués corrects, contrairement à
//! `pactl -f json`). Actions : `wpctl` (volume en échelle cubique, comme les autres bureaux).

use std::{
    cell::{Cell, RefCell},
    process::Command,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use adap::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;

use crate::{Shell, apps};

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub description: String,
    /// Volume « perçu » (racine cubique du gain), 0 à 1,5.
    pub volume: f64,
    pub mute: bool,
    /// Flux d'application : nom, icône, binaire.
    pub app: Option<(String, String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioState {
    pub sinks: Vec<Node>,
    pub sources: Vec<Node>,
    pub streams: Vec<Node>,
    pub default_sink: String,
    pub default_source: String,
}

impl AudioState {
    pub fn sink(&self) -> Option<&Node> {
        self.sinks.iter().find(|n| n.name == self.default_sink).or(self.sinks.first())
    }
    pub fn source(&self) -> Option<&Node> {
        self.sources.iter().find(|n| n.name == self.default_source).or(self.sources.first())
    }
}

/// Analyse la sortie de `pw-dump`.
pub fn parse(json: &str) -> Option<AudioState> {
    let all: Vec<Value> = serde_json::from_str(json).ok()?;
    let mut st = AudioState::default();
    for o in &all {
        match o["type"].as_str() {
            Some("PipeWire:Interface:Metadata") if o["props"]["metadata.name"] == "default" => {
                for m in o["metadata"].as_array().into_iter().flatten() {
                    let name = m["value"]["name"].as_str().unwrap_or_default().to_string();
                    match m["key"].as_str() {
                        Some("default.audio.sink") => st.default_sink = name,
                        Some("default.audio.source") => st.default_source = name,
                        _ => {}
                    }
                }
            }
            Some("PipeWire:Interface:Node") => {
                let p = &o["info"]["props"];
                let class = p["media.class"].as_str().unwrap_or_default();
                if !matches!(class, "Audio/Sink" | "Audio/Source" | "Stream/Output/Audio") {
                    continue;
                }
                let props = o["info"]["params"]["Props"].as_array();
                let vol = props.and_then(|ps| ps.iter().find(|x| x.get("channelVolumes").is_some()));
                let volumes: Vec<f64> = vol
                    .and_then(|v| v["channelVolumes"].as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
                    .unwrap_or_default();
                let linear = if volumes.is_empty() { 1.0 } else { volumes.iter().cloned().fold(0.0, f64::max) };
                let s = |k: &str| p[k].as_str().unwrap_or_default().to_string();
                let node = Node {
                    id: o["id"].as_u64().unwrap_or(0) as u32,
                    name: s("node.name"),
                    description: [s("node.description"), s("node.nick"), s("node.name")]
                        .into_iter()
                        .find(|d| !d.is_empty())
                        .unwrap_or_default(),
                    volume: linear.cbrt(),
                    mute: vol.and_then(|v| v["mute"].as_bool()).unwrap_or(false),
                    app: (class == "Stream/Output/Audio").then(|| {
                        let name = [s("application.name"), s("node.description"), s("application.process.binary")]
                            .into_iter()
                            .find(|d| !d.is_empty())
                            .unwrap_or_else(|| "Application".into());
                        (name, s("application.icon-name"), s("application.process.binary"))
                    }),
                };
                match class {
                    "Audio/Sink" => st.sinks.push(node),
                    "Audio/Source" => st.sources.push(node),
                    _ => st.streams.push(node),
                }
            }
            _ => {}
        }
    }
    Some(st)
}

pub fn read() -> Option<AudioState> {
    let out = Command::new("pw-dump").output().ok()?;
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn wpctl(args: &[String]) {
    let _ = Command::new("wpctl").args(args).spawn().and_then(|mut c| c.wait());
}

pub fn set_volume(id: u32, v: f64) {
    let args = vec!["set-volume".into(), id.to_string(), format!("{:.3}", v.clamp(0.0, 1.5))];
    std::thread::spawn(move || wpctl(&args));
}

pub fn set_mute(id: u32, mute: bool) {
    let args = vec!["set-mute".into(), id.to_string(), if mute { "1" } else { "0" }.into()];
    std::thread::spawn(move || wpctl(&args));
}

pub fn set_default(id: u32) {
    let args = vec!["set-default".into(), id.to_string()];
    std::thread::spawn(move || wpctl(&args));
}

/// Icône de haut-parleur selon le volume.
pub fn volume_icon(volume: f64, mute: bool) -> &'static str {
    if mute || volume <= 0.001 {
        "audio-volume-muted-symbolic"
    } else if volume < 0.34 {
        "audio-volume-low-symbolic"
    } else if volume < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

/// Icône d'une application qui joue du son.
fn app_icon(icon: &str, binary: &str, name: &str) -> gio::Icon {
    let theme = gtk::IconTheme::for_display(&gdk::Display::default().unwrap());
    for candidate in [icon, binary, &name.to_lowercase()] {
        if !candidate.is_empty() && theme.has_icon(candidate) {
            return gio::ThemedIcon::new(candidate).upcast();
        }
    }
    for candidate in [binary, name] {
        if !candidate.is_empty() && apps::exists(candidate) {
            return apps::icon(candidate);
        }
    }
    gio::ThemedIcon::new("audio-x-generic-symbolic").upcast()
}

pub struct AudioFlyout {
    shell: Weak<Shell>,
    window: gtk::Window,
    output_dd: gtk::DropDown,
    input_dd: gtk::DropDown,
    master: Slider,
    mic: Slider,
    mixer: gtk::Box,
    state: RefCell<AudioState>,
    syncing: Cell<bool>,
    hidden_at: Cell<Option<Instant>>,
}

/// Ligne : bouton muet + curseur + pourcentage.
#[derive(Clone)]
struct Slider {
    root: gtk::Box,
    mute: gtk::Button,
    scale: gtk::Scale,
    label: gtk::Label,
    id: Rc<Cell<u32>>,
    muted: Rc<Cell<bool>>,
}

impl Slider {
    fn new(icon: Option<&gio::Icon>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let mute = gtk::Button::new();
        mute.add_css_class("flat");
        mute.add_css_class("circular");
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        scale.set_hexpand(true);
        scale.set_draw_value(false);
        let label = gtk::Label::new(Some("0"));
        label.add_css_class("numeric");
        label.set_width_chars(3);
        label.set_xalign(1.0);
        if let Some(icon) = icon {
            let img = gtk::Image::from_gicon(icon);
            img.set_pixel_size(24);
            root.append(&img);
        }
        root.append(&mute);
        root.append(&scale);
        root.append(&label);
        let s = Self { root, mute, scale, label, id: Rc::new(Cell::new(0)), muted: Rc::new(Cell::new(false)) };
        {
            let s2 = s.clone();
            s.scale.connect_value_changed(move |sc| {
                s2.label.set_text(&format!("{}", (sc.value() * 100.0).round()));
            });
        }
        s
    }

    fn set(&self, node: &Node) {
        self.id.set(node.id);
        self.muted.set(node.mute);
        self.scale.set_value(node.volume.min(1.0));
        self.label.set_text(&format!("{}", (node.volume * 100.0).round()));
        self.mute.set_icon_name(volume_icon(node.volume, node.mute));
        self.mute.set_tooltip_text(Some(if node.mute { "Réactiver le son" } else { "Couper le son" }));
    }

    fn connect(&self, syncing: Rc<dyn Fn() -> bool>, on_change: Rc<dyn Fn()>) {
        {
            let (s, syncing, on_change) = (self.clone(), syncing.clone(), on_change.clone());
            self.scale.connect_value_changed(move |sc| {
                if !syncing() {
                    set_volume(s.id.get(), sc.value());
                    if s.muted.get() && sc.value() > 0.0 {
                        s.muted.set(false);
                        set_mute(s.id.get(), false);
                    }
                    s.mute.set_icon_name(volume_icon(sc.value(), s.muted.get()));
                    on_change();
                }
            });
        }
        {
            let s = self.clone();
            self.mute.connect_clicked(move |b| {
                let m = !s.muted.get();
                s.muted.set(m);
                set_mute(s.id.get(), m);
                b.set_icon_name(volume_icon(s.scale.value(), m));
                on_change();
            });
        }
    }
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-title");
    l.set_xalign(0.0);
    l
}

fn device_dropdown() -> gtk::DropDown {
    let dd = gtk::DropDown::from_strings(&[]);
    dd.set_hexpand(true);
    dd.add_css_class("device-picker");
    dd
}

impl AudioFlyout {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-quicksettings"));
        window.set_layer(Layer::Top);
        crate::editmode::anchor_to_bar(&window, &shell.config());
        shell.flyouts.borrow_mut().push(window.downgrade());
        window.set_anchor(Edge::Right, true);
        window.set_margin(Edge::Right, 12);
        window.set_keyboard_mode(KeyboardMode::Exclusive);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("noos-flyout");
        root.add_css_class("quick");
        root.add_css_class("audio-flyout");
        root.set_size_request(400, -1);

        root.append(&section("Sortie"));
        let output_dd = device_dropdown();
        root.append(&output_dd);
        let master = Slider::new(None);
        root.append(&master.root);

        root.append(&section("Entrée"));
        let input_dd = device_dropdown();
        root.append(&input_dd);
        let mic = Slider::new(None);
        root.append(&mic.root);

        root.append(&section("Mélangeur de volume"));
        let mixer = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.append(&mixer);
        mixer.set_margin_bottom(16);
        window.set_child(Some(&root));

        let f = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            output_dd,
            input_dd,
            master,
            mic,
            mixer,
            state: RefCell::new(AudioState::default()),
            syncing: Cell::new(false),
            hidden_at: Cell::new(None),
        });

        let syncing: Rc<dyn Fn() -> bool> = {
            let w = Rc::downgrade(&f);
            Rc::new(move || w.upgrade().is_some_and(|f| f.syncing.get()))
        };
        let changed: Rc<dyn Fn()> = {
            let w = Rc::downgrade(&f);
            Rc::new(move || {
                if let Some(f) = w.upgrade() {
                    if let Some(shell) = f.shell.upgrade() {
                        shell.refresh_status_soon();
                    }
                }
            })
        };
        f.master.connect(syncing.clone(), changed.clone());
        f.mic.connect(syncing, changed);
        for (dd, sinks) in [(&f.output_dd, true), (&f.input_dd, false)] {
            let w = Rc::downgrade(&f);
            dd.connect_selected_notify(move |dd| {
                let Some(f) = w.upgrade() else { return };
                if f.syncing.get() {
                    return;
                }
                let st = f.state.borrow().clone();
                let list = if sinks { &st.sinks } else { &st.sources };
                if let Some(n) = list.get(dd.selected() as usize) {
                    set_default(n.id);
                    let w2 = Rc::downgrade(&f);
                    glib::timeout_add_local_once(Duration::from_millis(300), move || {
                        if let Some(f) = w2.upgrade() {
                            f.reload();
                        }
                    });
                }
            });
        }
        {
            let f2 = f.clone();
            crate::close_on_focus_loss(&f.window, move || f2.hide());
        }
        {
            let keys = gtk::EventControllerKey::new();
            let f2 = f.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                if key == gdk::Key::Escape {
                    f2.hide();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            f.window.add_controller(keys);
        }
        // Nouvelles applications qui jouent du son pendant que le panneau est ouvert.
        {
            let w = Rc::downgrade(&f);
            glib::timeout_add_local(Duration::from_millis(1500), move || {
                let Some(f) = w.upgrade() else { return glib::ControlFlow::Break };
                if f.window.is_visible() {
                    f.reload();
                }
                glib::ControlFlow::Continue
            });
        }
        f
    }

    /// Relit l'état PipeWire (dans un thread) puis met à jour le panneau.
    fn reload(self: &Rc<Self>) {
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = tx.send_blocking(read());
        });
        let w = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if let (Ok(Some(st)), Some(f)) = (rx.recv().await, w.upgrade()) {
                f.apply(st);
            }
        });
    }

    fn apply(self: &Rc<Self>, st: AudioState) {
        let old = self.state.replace(st.clone());
        self.syncing.set(true);
        let names = |l: &[Node]| l.iter().map(|n| n.description.clone()).collect::<Vec<_>>();
        if names(&old.sinks) != names(&st.sinks) || old.sinks.is_empty() {
            let d = names(&st.sinks);
            self.output_dd.set_model(Some(&gtk::StringList::new(&d.iter().map(String::as_str).collect::<Vec<_>>())));
        }
        if names(&old.sources) != names(&st.sources) || old.sources.is_empty() {
            let d = names(&st.sources);
            self.input_dd.set_model(Some(&gtk::StringList::new(&d.iter().map(String::as_str).collect::<Vec<_>>())));
        }
        if let Some(i) = st.sinks.iter().position(|n| n.name == st.default_sink) {
            self.output_dd.set_selected(i as u32);
        }
        if let Some(i) = st.sources.iter().position(|n| n.name == st.default_source) {
            self.input_dd.set_selected(i as u32);
        }
        // Ne pas faire sauter un curseur que l'utilisateur est en train de déplacer.
        let dragging = |s: &Slider| s.scale.state_flags().contains(gtk::StateFlags::ACTIVE);
        if let Some(n) = st.sink() {
            if !dragging(&self.master) {
                self.master.set(n);
            }
        }
        self.master.root.set_sensitive(st.sink().is_some());
        if let Some(n) = st.source() {
            if !dragging(&self.mic) {
                self.mic.set(n);
            }
        }
        self.mic.root.set_sensitive(st.source().is_some());

        let ids = |l: &[Node]| l.iter().map(|n| n.id).collect::<Vec<_>>();
        if ids(&old.streams) != ids(&st.streams) || self.mixer.first_child().is_none() {
            while let Some(c) = self.mixer.first_child() {
                self.mixer.remove(&c);
            }
            if st.streams.is_empty() {
                let l = gtk::Label::new(Some("Aucune application ne joue de son"));
                l.add_css_class("dim-label");
                l.set_xalign(0.0);
                self.mixer.append(&l);
            }
            let syncing: Rc<dyn Fn() -> bool> = {
                let w = Rc::downgrade(self);
                Rc::new(move || w.upgrade().is_some_and(|f| f.syncing.get()))
            };
            for n in &st.streams {
                let (name, icon, bin) = n.app.clone().unwrap_or_default();
                let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
                let title = gtk::Label::new(Some(&name));
                title.set_xalign(0.0);
                title.add_css_class("caption");
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                col.append(&title);
                let slider = Slider::new(Some(&app_icon(&icon, &bin, &name)));
                slider.set(n);
                slider.connect(syncing.clone(), Rc::new(|| {}));
                col.append(&slider.root);
                self.mixer.append(&col);
            }
        }
        self.syncing.set(false);
        self.window.set_default_size(1, 1);
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.hidden_at.get().is_some_and(|t| t.elapsed().as_millis() < 250) {
            return;
        }
        if self.window.is_visible() {
            self.hide();
        } else {
            self.reload();
            self.window.present();
        }
    }

    pub fn hide(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
            self.hidden_at.set(Some(Instant::now()));
        }
    }
}

/// Fixe le volume de la sortie par défaut (glisser sur l'icône de la barre).
pub fn set_default_volume(v: f64) {
    let v = format!("{:.3}", v.clamp(0.0, 1.0));
    std::thread::spawn(move || {
        let _ = Command::new("wpctl").args(["set-volume", "@DEFAULT_AUDIO_SINK@", &v]).status();
    });
}

/// Molette sur l'icône de volume : ±5 % sur la sortie par défaut.
pub fn nudge_default(delta: f64) {
    let step = format!("{:.0}%{}", delta.abs() * 100.0, if delta > 0.0 { "+" } else { "-" });
    std::thread::spawn(move || {
        let _ = Command::new("wpctl").args(["set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &step]).status();
    });
}

/// Volume de la sortie par défaut, pour l'icône de la barre : (volume, muet).
pub fn default_volume() -> Option<(f64, bool)> {
    let out = Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v = text.split_whitespace().nth(1)?.parse().ok()?;
    Some((v, text.contains("MUTED")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"[
      {"id": 40, "type": "PipeWire:Interface:Metadata", "props": {"metadata.name": "default"},
       "metadata": [{"key": "default.audio.sink", "value": {"name": "alsa_output.pci"}},
                    {"key": "default.audio.source", "value": {"name": "alsa_input.usb"}}]},
      {"id": 56, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Sink",
        "node.name": "alsa_output.pci", "node.description": "Audio interne Stéréo analogique"},
        "params": {"Props": [{"channelVolumes": [0.064, 0.064], "mute": false}]}}},
      {"id": 52, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Audio/Source",
        "node.name": "alsa_input.usb", "node.description": "Arctis Nova 7X Mono"},
        "params": {"Props": [{"volume": 1.0}, {"channelVolumes": [1.0], "mute": true}]}}},
      {"id": 90, "type": "PipeWire:Interface:Node", "info": {"props": {"media.class": "Stream/Output/Audio",
        "application.name": "Firefox", "application.icon-name": "firefox", "application.process.binary": "firefox"},
        "params": {"Props": [{"channelVolumes": [0.125, 0.125], "mute": false}]}}}
    ]"#;

    #[test]
    fn parses_pw_dump() {
        let st = parse(DUMP).unwrap();
        assert_eq!(st.sink().unwrap().description, "Audio interne Stéréo analogique");
        assert!((st.sink().unwrap().volume - 0.4).abs() < 0.01);
        assert!(st.source().unwrap().mute);
        assert_eq!(st.streams.len(), 1);
        assert_eq!(st.streams[0].app.as_ref().unwrap().0, "Firefox");
        assert!((st.streams[0].volume - 0.5).abs() < 0.01);
    }
}

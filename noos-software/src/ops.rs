//! File d'opérations (installer, supprimer, mettre à jour, dépôts) exécutées l'une après
//! l'autre dans un thread, avec journal en direct.

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::Future,
    rc::Rc,
    sync::Arc,
};

use adap::prelude::*;
use gtk::glib;
use noos_pkg::{Backend, Plan};

pub type Backends = Arc<Vec<Box<dyn Backend>>>;

/// Exécute `f` dans un thread et renvoie son résultat sur la boucle GTK.
pub fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl Future<Output = T> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    async move { rx.recv().await.expect("thread de travail interrompu") }
}

type PlanFn = Box<dyn FnOnce(&dyn Backend) -> anyhow::Result<Plan> + Send>;
type DoneFn = Box<dyn FnOnce(bool)>;

struct Job {
    title: String,
    backend: usize,
    plan: PlanFn,
    done: DoneFn,
}

pub struct Ops {
    backends: Backends,
    queue: RefCell<VecDeque<Job>>,
    running: Cell<bool>,
    pub bar: gtk::Revealer,
    spinner: gtk::Spinner,
    title: gtk::Label,
    line: gtk::Label,
    pub log: gtk::TextBuffer,
    toasts: adap::ToastOverlay,
    window: RefCell<Option<gtk::Window>>,
}

impl Ops {
    pub fn new(backends: Backends, toasts: &adap::ToastOverlay) -> Rc<Self> {
        let bar = gtk::Revealer::new();
        bar.set_transition_type(gtk::RevealerTransitionType::SlideUp);
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        h.add_css_class("toolbar");
        h.set_margin_start(12);
        h.set_margin_end(12);
        h.set_margin_top(6);
        h.set_margin_bottom(6);
        let spinner = gtk::Spinner::new();
        let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
        texts.set_hexpand(true);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.add_css_class("heading");
        let line = gtk::Label::new(None);
        line.set_xalign(0.0);
        line.add_css_class("dim-label");
        line.add_css_class("caption");
        line.set_ellipsize(gtk::pango::EllipsizeMode::End);
        texts.append(&title);
        texts.append(&line);
        let details = gtk::Button::with_label("Détails");
        details.set_valign(gtk::Align::Center);
        h.append(&spinner);
        h.append(&texts);
        h.append(&details);
        bar.set_child(Some(&h));

        let ops = Rc::new(Self {
            backends,
            queue: RefCell::new(VecDeque::new()),
            running: Cell::new(false),
            bar,
            spinner,
            title,
            line,
            log: gtk::TextBuffer::new(None),
            toasts: toasts.clone(),
            window: RefCell::new(None),
        });
        let o = ops.clone();
        details.connect_clicked(move |_| o.show_log());
        ops
    }

    pub fn set_window(&self, w: &impl IsA<gtk::Window>) {
        *self.window.borrow_mut() = Some(w.clone().upcast());
    }

    pub fn busy(&self) -> bool {
        self.running.get() || !self.queue.borrow().is_empty()
    }

    /// Ajoute une opération à la file. `done(succès)` est appelé sur la boucle GTK.
    pub fn enqueue(
        self: &Rc<Self>,
        title: impl Into<String>,
        backend: usize,
        plan: impl FnOnce(&dyn Backend) -> anyhow::Result<Plan> + Send + 'static,
        done: impl FnOnce(bool) + 'static,
    ) {
        let title = title.into();
        if self.busy() {
            self.toasts.add_toast(adap::Toast::new(&format!("En file d'attente : {title}")));
        }
        self.queue.borrow_mut().push_back(Job { title, backend, plan: Box::new(plan), done: Box::new(done) });
        self.next();
    }

    fn append_log(&self, text: &str) {
        let mut end = self.log.end_iter();
        self.log.insert(&mut end, text);
        self.log.insert(&mut end, "\n");
    }

    fn next(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let Some(job) = self.queue.borrow_mut().pop_front() else {
            self.bar.set_reveal_child(false);
            self.spinner.stop();
            return;
        };
        self.running.set(true);
        self.bar.set_reveal_child(true);
        self.spinner.start();
        self.title.set_text(&job.title);
        self.line.set_text("Préparation…");
        self.append_log(&format!("\n━━━ {} ━━━", job.title));

        let (tx, rx) = async_channel::unbounded::<String>();
        let backends = self.backends.clone();
        let idx = job.backend;
        let plan_fn = job.plan;
        let result = background(move || {
            let backend = backends[idx].as_ref();
            let plan = plan_fn(backend)?;
            plan.run(&|l: &str| {
                let _ = tx.send_blocking(l.to_string());
            })
        });

        let ops = self.clone();
        let ops2 = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(line) = rx.recv().await {
                ops2.line.set_text(&line);
                ops2.append_log(&line);
            }
        });
        let title = job.title;
        let done = job.done;
        glib::spawn_future_local(async move {
            let res = result.await;
            ops.running.set(false);
            match &res {
                Ok(()) => {
                    ops.append_log("✓ Terminé");
                    ops.toasts.add_toast(adap::Toast::new(&format!("Terminé : {title}")));
                }
                Err(e) => {
                    ops.append_log(&format!("✗ {e:#}"));
                    ops.error_dialog(&title, &format!("{e:#}"));
                }
            }
            done(res.is_ok());
            ops.next();
        });
    }

    fn error_dialog(self: &Rc<Self>, title: &str, err: &str) {
        let dialog = adap::AlertDialog::new(Some(&format!("Échec : {title}")), Some(err));
        dialog.add_responses(&[("log", "Voir le journal"), ("ok", "Fermer")]);
        dialog.set_default_response(Some("ok"));
        let ops = self.clone();
        dialog.connect_response(None, move |_, r| {
            if r == "log" {
                ops.show_log();
            }
        });
        dialog.present(self.window.borrow().as_ref());
    }

    pub fn show_log(&self) {
        let view = gtk::TextView::with_buffer(&self.log);
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_top_margin(12);
        view.set_bottom_margin(12);
        view.set_left_margin(12);
        view.set_right_margin(12);
        let scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).build();
        let tv = adap::ToolbarView::new();
        tv.add_top_bar(&adap::HeaderBar::new());
        tv.set_content(Some(&scroll));
        let dialog = adap::Dialog::builder().title("Journal des opérations").content_width(760).content_height(520).child(&tv).build();
        dialog.present(self.window.borrow().as_ref());
        glib::idle_add_local_once(move || {
            let mut end = view.buffer().end_iter();
            view.scroll_to_iter(&mut end, 0.0, false, 0.0, 1.0);
        });
    }
}

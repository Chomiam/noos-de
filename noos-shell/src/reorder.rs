//! Réordonner les icônes épinglées de la barre des tâches par glisser-déposer.
//!
//! - l'icône saisie suit le pointeur (légèrement soulevée et agrandie) ;
//! - quand elle dépasse la moitié d'une voisine épinglée, elle prend sa place dans la boîte
//!   et la voisine glisse jusqu'à l'emplacement libéré (technique « FLIP » : la voisine part
//!   de son ancienne position via `transform`, puis la transition CSS la ramène à 0) ;
//! - au relâchement, l'icône se pose en douceur, puis le nouvel ordre est enregistré dans
//!   `noos.toml` (`[taskbar] pinned`) une fois l'animation terminée.
//!
//! Tout passe par `transform` en CSS : la mise en page de la barre ne bouge qu'aux échanges.

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::Shell;

/// Distance (px) avant de considérer qu'il s'agit d'un glisser et non d'un clic.
const THRESHOLD: f64 = 8.0;
/// Durée des glissements (doit correspondre à la transition CSS).
const ANIM_MS: u64 = 180;
/// Classe CSS des boutons épinglés réordonnables.
pub const PINNED_CLASS: &str = "pinned";

thread_local! {
    /// Styles communs du glisser : transitions, et emplacement d'où partent les voisines.
    static CSS: gtk::CssProvider = {
        let css = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        }
        css.load_from_string(&common_style(0.0));
        css
    };
}

/// Feuille commune, rechargée seulement au début et à la fin d'un glisser (jamais à chaque
/// mouvement : cela recalculait le style de toute la barre et faisait sautiller les icônes).
fn common_style(slot: f64) -> String {
    format!(
        ".noos-taskbar button.task {{ transition: transform {ANIM_MS}ms cubic-bezier(0.2, 0.8, 0.2, 1); }}\n\
         .noos-taskbar button.task.dragging {{ background-color: alpha(@window_fg_color, 0.10); }}\n\
         .noos-taskbar button.task.from-left {{ transition: none; transform: translateX({neg:.1}px); }}\n\
         .noos-taskbar button.task.from-right {{ transition: none; transform: translateX({slot:.1}px); }}\n",
        neg = -slot,
    )
}

/// État d'un glisser en cours (un seul à la fois, pour toute la barre).
#[derive(Default)]
struct Drag {
    button: RefCell<Option<gtk::Button>>,
    active: Cell<bool>,
    /// Correction du décalage : chaque échange déplace l'emplacement de l'icône d'un cran.
    shift: Cell<f64>,
    slot: Cell<f64>,
    /// Style propre à l'icône saisie (seul lui change à chaque mouvement).
    own: RefCell<Option<gtk::CssProvider>>,
}

/// Rend un bouton épinglé réordonnable dans `tasks`. `app_id` sert à enregistrer l'ordre.
pub fn attach(button: &gtk::Button, app_id: &str, tasks: &gtk::Box, shell: &Weak<Shell>) {
    button.add_css_class(PINNED_CLASS);
    button.set_widget_name(app_id);
    // Un seul geste pour toute la zone, sur la boîte (qui ne bouge pas) : les décalages sont
    // mesurés dans un repère fixe. Mesurés sur l'icône, ils sautaient d'un emplacement à chaque
    // échange (l'icône a bougé sous le pointeur) et l'échange s'inversait en boucle.
    if !tasks.has_css_class("reorderable") {
        tasks.add_css_class("reorderable");
        install(tasks, shell);
    }
    if std::env::var_os("NOOS_REORDER_DEMO").is_some() && !DEMO.with(|d| d.replace(true)) {
        demo(tasks.clone(), shell.clone());
    }
}

fn install(tasks: &gtk::Box, shell: &Weak<Shell>) {
    let drag = gtk::GestureDrag::new();
    drag.set_button(gdk::BUTTON_PRIMARY);
    // Phase de capture : la boîte voit le geste avant le bouton, et ne le revendique qu'au-delà
    // du seuil (le clic du bouton est alors annulé ; sinon c'est un clic normal).
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state = Rc::new(Drag::default());
    {
        let (state, tasks) = (state.clone(), tasks.clone());
        drag.connect_drag_begin(move |_, x, y| {
            let hit = tasks.pick(x, y, gtk::PickFlags::DEFAULT);
            let mut w = hit;
            while let Some(cur) = w.clone() {
                if cur.parent().as_ref() == Some(tasks.upcast_ref()) {
                    break;
                }
                w = cur.parent();
            }
            *state.button.borrow_mut() = w.filter(|b| b.has_css_class(PINNED_CLASS)).and_downcast::<gtk::Button>();
        });
    }
    {
        let (state, tasks) = (state.clone(), tasks.clone());
        drag.connect_drag_update(move |g, dx, _dy| {
            let Some(button) = state.button.borrow().clone() else { return };
            if !state.active.get() {
                if dx.abs() < THRESHOLD {
                    return;
                }
                // C'est un glisser : le clic du bouton est annulé.
                g.set_state(gtk::EventSequenceState::Claimed);
            }
            update(&state, &button, &tasks, dx);
        });
    }
    {
        let (state, tasks, shell) = (state.clone(), tasks.clone(), shell.clone());
        drag.connect_drag_end(move |_, _, _| {
            if let Some(button) = state.button.borrow_mut().take() {
                finish(&state, &button, &tasks, &shell, true);
            }
        });
    }
    tasks.add_controller(drag);
}

thread_local! {
    static DEMO: Cell<bool> = const { Cell::new(false) };
}

/// Démonstration pour les captures de test (aperçu imbriqué, sans souris) : la première
/// épinglée glisse de 2,4 emplacements vers la droite puis revient d'un demi-emplacement
/// (le point d'échange ne doit pas osciller), puis est relâchée ; rien n'est enregistré.
fn demo(tasks: gtk::Box, shell: Weak<Shell>) {
    glib::timeout_add_local_once(Duration::from_millis(2500), move || {
        let Some(button) = tasks.first_child().filter(|w| w.has_css_class(PINNED_CLASS)).and_downcast::<gtk::Button>() else {
            eprintln!("noos-shell : démo de réordonnancement : aucune épinglée");
            return;
        };
        let state = Rc::new(Drag::default());
        let step = Rc::new(Cell::new(0u32));
        let swaps = Rc::new(Cell::new(0u32));
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let n = step.get();
            step.set(n + 1);
            let slot = button.width() as f64 + tasks.spacing() as f64;
            let before = pinned_order(&tasks);
            let dx = if n < 24 { slot * 2.4 * f64::from(n + 1) / 24.0 } else if n < 30 { slot * (2.4 - 0.5 * f64::from(n - 23) / 6.0) } else { -1.0 };
            if dx >= 0.0 {
                update(&state, &button, &tasks, dx);
                if pinned_order(&tasks) != before {
                    swaps.set(swaps.get() + 1);
                }
                return glib::ControlFlow::Continue;
            }
            eprintln!("noos-shell : démo de réordonnancement terminée, {} échanges, ordre {:?}", swaps.get(), pinned_order(&tasks));
            finish(&state, &button, &tasks, &shell, false);
            glib::ControlFlow::Break
        });
    });
}

/// Hystérésis : il faut dépasser nettement le milieu de la voisine pour échanger.
const SWAP_AT: f64 = 0.6;

/// Le pointeur s'est déplacé de `dx` (repère de la barre) depuis le début du glisser.
fn update(state: &Drag, button: &gtk::Button, tasks: &gtk::Box, dx: f64) {
    if !state.active.get() {
        state.active.set(true);
        state.shift.set(0.0);
        let slot = button.width() as f64 + tasks.spacing() as f64;
        state.slot.set(slot);
        CSS.with(|css| css.load_from_string(&common_style(slot)));
        let own = gtk::CssProvider::new();
        #[allow(deprecated)]
        button.style_context().add_provider(&own, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
        *state.own.borrow_mut() = Some(own);
        button.add_css_class("dragging");
    }
    let slot = state.slot.get();
    let mut offset = dx + state.shift.get();
    // Échanges avec les voisines épinglées tant qu'on dépasse nettement leur milieu.
    while offset > slot * SWAP_AT {
        let Some(next) = pinned_sibling(button, true) else { break };
        tasks.reorder_child_after(button, Some(&next));
        slide_in(&next, "from-right");
        state.shift.set(state.shift.get() - slot);
        offset -= slot;
    }
    while offset < -slot * SWAP_AT {
        let Some(prev) = pinned_sibling(button, false) else { break };
        tasks.reorder_child_after(&prev, Some(button));
        slide_in(&prev, "from-left");
        state.shift.set(state.shift.get() + slot);
        offset += slot;
    }
    // L'icône ne sort pas de la zone des épinglées.
    let offset = if pinned_sibling(button, false).is_none() { offset.max(-slot / 2.0) } else { offset };
    let offset = if pinned_sibling(button, true).is_none() { offset.min(slot / 2.0) } else { offset };
    if let Some(own) = state.own.borrow().as_ref() {
        own.load_from_string(&format!(
            ".noos-taskbar button.task.dragging {{ transition: none; transform: translate({offset:.1}px, -3px) scale(1.08); }}"
        ));
    }
}

/// Relâchement : l'icône se pose, puis l'ordre est enregistré (si `save`).
fn finish(state: &Drag, button: &gtk::Button, tasks: &gtk::Box, shell: &Weak<Shell>, save: bool) {
    if !state.active.replace(false) {
        return;
    }
    // Retrait du style propre et de la classe : la transition ramène l'icône à sa place.
    if let Some(own) = state.own.borrow_mut().take() {
        #[allow(deprecated)]
        button.style_context().remove_provider(&own);
    }
    button.remove_css_class("dragging");
    let order = pinned_order(tasks);
    let shell = shell.clone();
    glib::timeout_add_local_once(Duration::from_millis(ANIM_MS + 40), move || {
        CSS.with(|css| css.load_from_string(&common_style(0.0)));
        if !save {
            return;
        }
        let Some(shell) = shell.upgrade() else { return };
        shell.update_config(|c| {
            // Les épinglées absentes de la barre (appli désinstallée) gardent leur place à la
            // fin de la liste.
            let rest: Vec<String> = c.taskbar.pinned.iter().filter(|p| !order.contains(p)).cloned().collect();
            c.taskbar.pinned = order.iter().cloned().chain(rest).collect();
        });
        shell.refresh_tasks();
    });
}

/// Voisine épinglée immédiate (suivante ou précédente).
fn pinned_sibling(button: &gtk::Button, next: bool) -> Option<gtk::Widget> {
    let sib = if next { button.next_sibling() } else { button.prev_sibling() }?;
    sib.has_css_class(PINNED_CLASS).then_some(sib)
}

/// La voisine part de son ancienne position (classe sans transition) puis, une image plus
/// tard, la classe est retirée : la transition CSS l'amène à sa nouvelle place.
fn slide_in(widget: &gtk::Widget, class: &'static str) {
    widget.add_css_class(class);
    let frames = Rc::new(RefCell::new(0u8));
    widget.add_tick_callback(move |w, _| {
        *frames.borrow_mut() += 1;
        if *frames.borrow() < 2 {
            return glib::ControlFlow::Continue;
        }
        w.remove_css_class(class);
        glib::ControlFlow::Break
    });
}

/// Ordre des épinglées tel qu'affiché.
fn pinned_order(tasks: &gtk::Box) -> Vec<String> {
    let mut order = Vec::new();
    let mut child = tasks.first_child();
    while let Some(w) = child {
        if w.has_css_class(PINNED_CLASS) {
            order.push(w.widget_name().to_string());
        }
        child = w.next_sibling();
    }
    order
}

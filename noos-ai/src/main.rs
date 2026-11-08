//! noos-ai — démon de l'assistant IA local de NOOS (session de l'utilisateur).
//!
//! Démarré **à la demande** par D-Bus (`org.noos.Assistant1`, fichier de service de session)
//! et arrêté après quelques minutes sans requête : rien ne tourne tant que l'IA ne sert pas.
//! Tout reste sur le PC (modèle de langage par Ollama en local, Vulkan ou processeur).
//!
//! Méthodes (JSON en sortie) :
//! - `Status()` : matériel détecté, palier de modèle conseillé, moteurs installés, réglages ;
//! - `ListApps()` : applications qui publient des actions (`org.noos.Actions.*`) ;
//! - `Ask(texte)` : requête de l'utilisateur (phase 2 : commandes rapides puis modèle).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};

const BUS_NAME: &str = "org.noos.Assistant1";
const PATH: &str = "/org/noos/Assistant";
/// Arrêt après ce délai sans requête (relancé automatiquement par D-Bus au besoin).
const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);

const XML: &str = r#"<node>
  <interface name="org.noos.Assistant1">
    <method name="Status"><arg type="s" direction="out" name="json"/></method>
    <method name="ListApps"><arg type="s" direction="out" name="json"/></method>
    <method name="Ask">
      <arg type="s" direction="in" name="text"/>
      <arg type="s" direction="out" name="json"/>
    </method>
  </interface>
</node>"#;

struct Daemon {
    conn: RefCell<Option<gio::DBusConnection>>,
    last_call: Cell<Instant>,
}

impl Daemon {
    fn status(&self) -> Value {
        let hw = noos_core::ai::detect();
        let cfg = noos_core::NoosConfig::load().ai;
        let rec = noos_core::ai::recommended(&hw);
        json!({
            "hardware": {
                "summary": hw.summary(),
                "ram_mb": hw.ram_mb,
                "ram_available_mb": hw.ram_available_mb,
                "cpu_threads": hw.cpu_threads,
                "gpus": hw.gpus.iter().map(|g| json!({"name": g.name, "driver": g.driver, "discrete": g.discrete, "vram_mb": g.vram_mb})).collect::<Vec<_>>(),
                "on_battery": hw.on_battery,
            },
            "engines": {"ollama": hw.ollama, "ollama_vulkan": hw.ollama_vulkan, "whisper": hw.whisper, "vulkan_tools": hw.vulkan_tools},
            "recommended_tier": rec.map(|t| t.id),
            "settings": {"enabled": cfg.enabled, "assistant": cfg.assistant, "file_search": cfg.file_search, "dictation": cfg.dictation},
        })
    }

    fn list_apps(&self) -> Value {
        let Some(conn) = self.conn.borrow().clone() else { return json!([]) };
        let disabled = noos_core::NoosConfig::load().ai.disabled_apps;
        let apps: Vec<Value> = noos_actions::discover(&conn)
            .iter()
            .filter_map(|bus| noos_actions::list(&conn, bus).ok())
            .map(|a| {
                let enabled = !disabled.contains(&a.app);
                json!({"app": a.app, "name": a.name, "enabled": enabled, "actions": a.actions})
            })
            .collect();
        json!(apps)
    }

    fn ask(&self, text: &str) -> Value {
        let cfg = noos_core::NoosConfig::load().ai;
        if !cfg.enabled || !cfg.assistant {
            return json!({"ok": false, "message": "L'assistant est désactivé (Paramètres › IA)."});
        }
        // Phase 2 : commandes rapides (règles françaises) puis modèle local avec outils.
        json!({"ok": false, "message": format!("Je ne sais pas encore répondre à « {text} ».")})
    }

    fn call(&self, method: &str, params: &glib::Variant) -> String {
        self.last_call.set(Instant::now());
        match method {
            "Status" => self.status().to_string(),
            "ListApps" => self.list_apps().to_string(),
            "Ask" => {
                let (text,): (String,) = params.get().unwrap_or_default();
                self.ask(&text).to_string()
            }
            _ => json!({"ok": false, "message": "méthode inconnue"}).to_string(),
        }
    }
}

fn main() {
    if std::env::args().any(|a| a == "--status") {
        // Diagnostic en ligne de commande.
        let d = Daemon { conn: RefCell::new(None), last_call: Cell::new(Instant::now()) };
        println!("{}", serde_json::to_string_pretty(&d.status()).unwrap_or_default());
        return;
    }
    let main_loop = glib::MainLoop::new(None, false);
    let daemon = Rc::new(Daemon { conn: RefCell::new(None), last_call: Cell::new(Instant::now()) });
    {
        let (d, ml) = (daemon.clone(), main_loop.clone());
        let _ = gio::bus_own_name(
            gio::BusType::Session,
            BUS_NAME,
            gio::BusNameOwnerFlags::NONE,
            move |conn, _| {
                let node = gio::DBusNodeInfo::for_xml(XML).expect("XML org.noos.Assistant1");
                let iface = node.lookup_interface(BUS_NAME).expect("interface org.noos.Assistant1");
                let d2 = d.clone();
                let res = conn
                    .register_object(PATH, &iface)
                    .method_call(move |_, _, _, _, method, params, invocation| {
                        let reply = d2.call(method, &params);
                        invocation.return_value(Some(&(reply,).to_variant()));
                    })
                    .build();
                if let Err(e) = res {
                    eprintln!("noos-ai : publication impossible : {e}");
                }
                *d.conn.borrow_mut() = Some(conn);
            },
            |_, _| {},
            move |_, _| {
                eprintln!("noos-ai : déjà lancé");
                ml.quit();
            },
        );
    }
    {
        let (d, ml) = (daemon.clone(), main_loop.clone());
        glib::timeout_add_seconds_local(30, move || {
            if d.last_call.get().elapsed() > IDLE_EXIT {
                ml.quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
    main_loop.run();
}

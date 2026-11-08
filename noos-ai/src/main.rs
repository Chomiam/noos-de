//! noos-ai — démon de l'assistant IA local de NOOS (session de l'utilisateur).
//!
//! Démarré **à la demande** par D-Bus (`org.noos.Assistant1`, fichier de service de session)
//! et arrêté après quelques minutes sans requête : rien ne tourne tant que l'IA ne sert pas.
//! Tout reste sur le PC (modèle de langage par Ollama en local, Vulkan ou processeur).
//!
//! Méthodes (JSON en sortie) :
//! - `Status()` : matériel détecté, palier de modèle conseillé, moteurs installés, réglages ;
//! - `ListApps()` : applications qui publient des actions (`org.noos.Actions.*`) ;
//! - `Ask(texte)` : requête de l'utilisateur — commandes rapides (règles), sinon modèle local
//!   avec outils. Réponse `{kind: done|confirm|answer|error, message, action?, pending?, via}` ;
//! - `Confirm(n, oui)` : exécute (ou abandonne) une action en attente de confirmation ;
//! - `Models()`, `PullModel(palier)`, `PullStatus()`, `DeleteModel(palier)` : modèles locaux.

mod actions;
mod ollama;
mod rules;

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use noos_actions::{ActionInfo, Registry, Risk};
use serde_json::{Value, json};

const BUS_NAME: &str = "org.noos.Assistant1";
const PATH: &str = "/org/noos/Assistant";
/// Arrêt après ce délai sans requête (relancé automatiquement par D-Bus au besoin).
const IDLE_EXIT: Duration = Duration::from_secs(10 * 60);
/// Messages de conversation gardés pour le contexte du modèle.
const HISTORY: usize = 8;

const XML: &str = r#"<node>
  <interface name="org.noos.Assistant1">
    <method name="Status"><arg type="s" direction="out" name="json"/></method>
    <method name="ListApps"><arg type="s" direction="out" name="json"/></method>
    <method name="Ask">
      <arg type="s" direction="in" name="text"/>
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="Confirm">
      <arg type="u" direction="in" name="pending"/>
      <arg type="b" direction="in" name="accept"/>
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="Models"><arg type="s" direction="out" name="json"/></method>
    <method name="PullModel">
      <arg type="s" direction="in" name="tier"/>
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="PullStatus"><arg type="s" direction="out" name="json"/></method>
    <method name="DeleteModel">
      <arg type="s" direction="in" name="tier"/>
      <arg type="s" direction="out" name="json"/>
    </method>
    <method name="Reset"><arg type="s" direction="out" name="json"/></method>
  </interface>
</node>"#;

const SYSTEM_PROMPT: &str = "Tu es l'assistant du bureau NOOS (Linux), sur l'ordinateur de l'utilisateur. \
Réponds toujours en français, brièvement (une à trois phrases). \
Pour agir sur l'ordinateur, appelle UN outil adapté ; n'invente jamais d'outil ni de commande. \
Si aucun outil ne convient, réponds simplement à la question. Ne prétends pas avoir fait une action sans outil.";

/// Où exécuter une action : bureau (intégrée) ou application (bus `org.noos.Actions.X`).
#[derive(Clone)]
enum Target {
    System,
    App(String),
}

#[derive(Clone)]
struct Pending {
    target: Target,
    info: ActionInfo,
    params: Value,
}

struct Daemon {
    conn: RefCell<Option<gio::DBusConnection>>,
    last_call: Cell<Instant>,
    system: Rc<Registry>,
    ollama: Arc<ollama::Ollama>,
    pending: RefCell<HashMap<u32, Pending>>,
    next_id: Cell<u32>,
    history: RefCell<Vec<Value>>,
}

fn reply(kind: &str, message: impl Into<String>) -> Value {
    json!({"ok": kind != "error", "kind": kind, "message": message.into()})
}

/// Modèle à utiliser maintenant (palier choisi ou conseillé, allégé sur batterie / RAM juste).
fn pick_model(cfg: &noos_core::config::Ai) -> Result<&'static noos_core::ai::Tier, String> {
    let hw = noos_core::ai::detect();
    let mut tier = noos_core::ai::tier(&cfg.tier).or_else(|| noos_core::ai::recommended(&hw)).ok_or("Ce PC a trop peu de mémoire pour un modèle de langage : seules les commandes rapides fonctionnent.")?;
    if cfg.light_on_battery && hw.on_battery {
        tier = noos_core::ai::tier("light").unwrap_or(tier);
    }
    // Pas assez de mémoire libre : palier inférieur plutôt que de faire ramer le PC.
    while hw.ram_available_mb < tier.memory_mb && hw.best_gpu().and_then(|g| g.vram_mb).unwrap_or(0) < tier.memory_mb {
        let idx = noos_core::ai::TIERS.iter().position(|t| t.id == tier.id).unwrap_or(0);
        if idx == 0 {
            return Err(format!("Mémoire libre insuffisante ({} Mo) pour le modèle : fermez quelques applications.", hw.ram_available_mb));
        }
        tier = &noos_core::ai::TIERS[idx - 1];
    }
    Ok(tier)
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

    /// Applications actives (non coupées dans Paramètres › IA) et leurs actions.
    fn apps(&self) -> Vec<(noos_actions::AppActions, bool)> {
        let Some(conn) = self.conn.borrow().clone() else { return vec![] };
        let disabled = noos_core::NoosConfig::load().ai.disabled_apps;
        noos_actions::discover(&conn)
            .iter()
            .filter_map(|bus| noos_actions::list(&conn, bus).ok())
            .map(|a| {
                let on = !disabled.contains(&a.app);
                (a, on)
            })
            .collect()
    }

    fn list_apps(&self) -> Value {
        json!(self.apps().into_iter().map(|(a, on)| json!({"app": a.app, "name": a.name, "enabled": on, "actions": a.actions})).collect::<Vec<_>>())
    }

    /// Outils pour le modèle : actions du bureau + des applications actives.
    fn tools(&self) -> Vec<(String, Target, ActionInfo)> {
        let mut out: Vec<(String, Target, ActionInfo)> = self.system.list().actions.into_iter().map(|a| (a.id.replace(['.', '-'], "_"), Target::System, a)).collect();
        for (app, on) in self.apps() {
            if on {
                let bus = format!("{}{}", noos_actions::BUS_PREFIX, app.app);
                out.extend(app.actions.into_iter().map(|a| (a.id.replace(['.', '-'], "_"), Target::App(bus.clone()), a)));
            }
        }
        out
    }

    fn execute(&self, p: &Pending) -> Value {
        let res = match &p.target {
            Target::System => self.system.invoke(&p.info.id, &p.params),
            Target::App(bus) => match self.conn.borrow().clone() {
                Some(c) => noos_actions::invoke(&c, bus, &p.info.id, &p.params),
                None => Err("bus de session indisponible".into()),
            },
        };
        let mut v = match res {
            Ok(m) => reply("done", m),
            Err(e) => reply("error", e),
        };
        v["action"] = json!(p.info.id);
        v["title"] = json!(p.info.title);
        v
    }

    /// Exécute, ou met en attente si l'action touche au système.
    fn run_or_confirm(&self, p: Pending) -> Value {
        if let Err(e) = p.info.check(&p.params) {
            return reply("error", format!("Paramètres refusés : {e}"));
        }
        let confirm = noos_core::NoosConfig::load().ai.confirm_system;
        if p.info.risk == Risk::System && confirm {
            let id = self.next_id.get() + 1;
            self.next_id.set(id);
            let (title, desc) = (p.info.title.clone(), p.info.description.clone());
            let action = p.info.id.clone();
            self.pending.borrow_mut().insert(id, p);
            return json!({"ok": true, "kind": "confirm", "pending": id, "action": action, "title": title, "message": format!("{title} ? {desc}.")});
        }
        self.execute(&p)
    }

    fn confirm(&self, id: u32, accept: bool) -> Value {
        let Some(p) = self.pending.borrow_mut().remove(&id) else { return reply("error", "Cette demande a expiré.") };
        if !accept {
            let mut v = reply("done", "D'accord, j'annule.");
            v["action"] = json!(p.info.id);
            return v;
        }
        self.execute(&p)
    }

    fn remember(&self, user: &str, assistant: &str) {
        let mut h = self.history.borrow_mut();
        h.push(json!({"role": "user", "content": user}));
        h.push(json!({"role": "assistant", "content": assistant}));
        let n = h.len();
        if n > HISTORY {
            h.drain(..n - HISTORY);
        }
    }

    /// Première étape, sur le fil principal : règles. `Err(contexte)` → il faut le modèle.
    fn ask_rules(&self, text: &str) -> Result<Value, ()> {
        let cfg = noos_core::NoosConfig::load().ai;
        if !cfg.enabled || !cfg.assistant {
            return Ok(reply("error", "L'assistant est désactivé (Paramètres › IA)."));
        }
        let Some(intent) = rules::parse(text) else { return Err(()) };
        let Some(info) = self.system.list().actions.into_iter().find(|a| a.id == intent.action) else { return Err(()) };
        let mut v = self.run_or_confirm(Pending { target: Target::System, info, params: intent.params });
        v["via"] = json!("rules");
        if let Some(m) = v["message"].as_str() {
            self.remember(text, m);
        }
        Ok(v)
    }

    /// Réponse du modèle (déjà calculée hors du fil principal) → action ou texte.
    fn handle_model(&self, text: &str, msg: Result<Value, String>, tools: &[(String, Target, ActionInfo)]) -> Value {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => return reply("error", e),
        };
        let mut v = if let Some(call) = msg["tool_calls"].as_array().and_then(|c| c.first()) {
            let name = call["function"]["name"].as_str().unwrap_or("");
            let mut args = call["function"]["arguments"].clone();
            if let Some(s) = args.as_str() {
                args = serde_json::from_str(s).unwrap_or(json!({}));
            }
            if !args.is_object() {
                args = json!({});
            }
            match tools.iter().find(|(n, _, _)| n == name) {
                Some((_, target, info)) => self.run_or_confirm(Pending { target: target.clone(), info: info.clone(), params: args }),
                None => reply("error", format!("Le modèle a demandé une action inconnue ({name}).")),
            }
        } else {
            let content = msg["content"].as_str().unwrap_or("").trim().to_string();
            if content.is_empty() { reply("error", "Je n'ai pas de réponse.") } else { reply("answer", content) }
        };
        v["via"] = json!("model");
        if let Some(m) = v["message"].as_str() {
            self.remember(text, m);
        }
        v
    }

    fn models(&self) -> Value {
        let cfg = noos_core::NoosConfig::load().ai;
        let ok = noos_core::ai::detect().ollama;
        let installed = if ok { self.ollama.models(cfg.use_gpu).unwrap_or_default() } else { vec![] };
        json!({
            "engine": ok,
            "dir": ollama::models_dir(),
            "tiers": noos_core::ai::TIERS.iter().map(|t| {
                let size = installed.iter().find(|(n, _)| n == t.model).map(|(_, s)| *s);
                json!({"id": t.id, "model": t.model, "installed": size.is_some(), "size": size})
            }).collect::<Vec<_>>(),
        })
    }

    fn pull_status(&self) -> Value {
        match self.ollama.pull.lock().unwrap().clone() {
            None => json!({"active": false}),
            Some(p) => json!({"active": !p.done, "model": p.model, "status": p.status, "completed": p.completed, "total": p.total, "done": p.done, "error": p.error}),
        }
    }

    fn tier_model(id: &str) -> Result<&'static str, String> {
        noos_core::ai::tier(id).map(|t| t.model).ok_or_else(|| format!("palier inconnu : {id}"))
    }

    fn call_sync(&self, method: &str, params: &glib::Variant) -> Value {
        let use_gpu = noos_core::NoosConfig::load().ai.use_gpu;
        match method {
            "Status" => self.status(),
            "ListApps" => self.list_apps(),
            "Confirm" => {
                let (id, accept): (u32, bool) = params.get().unwrap_or((0, false));
                self.confirm(id, accept)
            }
            "Models" => self.models(),
            "PullStatus" => self.pull_status(),
            "PullModel" => {
                let (tier,): (String,) = params.get().unwrap_or_default();
                match Self::tier_model(&tier).and_then(|m| self.ollama.start_pull(m, use_gpu)) {
                    Ok(()) => reply("done", "Téléchargement lancé."),
                    Err(e) => reply("error", e),
                }
            }
            "DeleteModel" => {
                let (tier,): (String,) = params.get().unwrap_or_default();
                match Self::tier_model(&tier).and_then(|m| self.ollama.delete(m, use_gpu)) {
                    Ok(()) => reply("done", "Modèle supprimé."),
                    Err(e) => reply("error", e),
                }
            }
            "Reset" => {
                self.history.borrow_mut().clear();
                self.pending.borrow_mut().clear();
                reply("done", "Nouvelle conversation.")
            }
            _ => reply("error", "méthode inconnue"),
        }
    }
}

/// `Ask` : règles tout de suite ; sinon modèle dans un fil à part, réponse D-Bus différée.
fn ask(d: Rc<Daemon>, text: String, invocation: gio::DBusMethodInvocation) {
    let send = |inv: gio::DBusMethodInvocation, v: Value| inv.return_value(Some(&(v.to_string(),).to_variant()));
    if let Ok(v) = d.ask_rules(&text) {
        return send(invocation, v);
    }
    let cfg = noos_core::NoosConfig::load().ai;
    let tier = match pick_model(&cfg) {
        Ok(t) => t,
        Err(e) => return send(invocation, reply("error", e)),
    };
    let tools = d.tools();
    let tool_defs: Vec<Value> = tools.iter().map(|(_, _, a)| noos_actions::as_tool(a)).collect();
    let mut messages = vec![json!({"role": "system", "content": SYSTEM_PROMPT})];
    messages.extend(d.history.borrow().iter().cloned());
    messages.push(json!({"role": "user", "content": text}));
    let ollama = d.ollama.clone();
    let (model, use_gpu, keep) = (tier.model, cfg.use_gpu, cfg.unload_minutes);
    glib::spawn_future_local(async move {
        let res = gio::spawn_blocking(move || {
            if !ollama.has_model(model, use_gpu) {
                return Err(format!("Le modèle « {model} » n'est pas encore téléchargé : Paramètres › IA › Modèle."));
            }
            ollama.chat(model, &messages, &tool_defs, use_gpu, keep)
        })
        .await
        .unwrap_or_else(|_| Err("le moteur d'IA s'est arrêté".into()));
        d.last_call.set(Instant::now());
        let mut v = d.handle_model(&text, res, &tools);
        v["model"] = json!(model);
        send(invocation, v);
    });
}

fn new_daemon() -> Rc<Daemon> {
    Rc::new(Daemon {
        conn: RefCell::new(None),
        last_call: Cell::new(Instant::now()),
        system: actions::system(),
        ollama: Arc::new(ollama::Ollama::default()),
        pending: RefCell::new(HashMap::new()),
        next_id: Cell::new(0),
        history: RefCell::new(vec![]),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--status") {
        // Diagnostic en ligne de commande.
        println!("{}", serde_json::to_string_pretty(&new_daemon().status()).unwrap_or_default());
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--rules") {
        // Test du routeur : `noos-ai --rules "monte le son à 90 %"` (n'exécute rien).
        let t = args.get(i + 1).cloned().unwrap_or_default();
        match rules::parse(&t) {
            Some(it) => println!("{} {}", it.action, it.params),
            None => println!("(modèle)"),
        }
        return;
    }
    let main_loop = glib::MainLoop::new(None, false);
    let daemon = new_daemon();
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
                        d2.last_call.set(Instant::now());
                        if method == "Ask" {
                            let (text,): (String,) = params.get().unwrap_or_default();
                            return ask(d2.clone(), text, invocation);
                        }
                        let reply = d2.call_sync(method, &params).to_string();
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
            let pulling = d.ollama.pull.lock().unwrap().as_ref().is_some_and(|p| !p.done);
            if d.last_call.get().elapsed() > IDLE_EXIT && !pulling {
                ml.quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
    main_loop.run();
    // Mémoire rendue tout de suite (serveur système partagé) puis notre serveur arrêté.
    for t in noos_core::ai::TIERS {
        daemon.ollama.unload(t.model);
    }
    daemon.ollama.stop();
}

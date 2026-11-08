//! Actions exposées à l'assistant IA de NOOS.
//!
//! **Règle du projet** : chaque application NOOS déclare ce qu'elle sait faire sous forme
//! d'actions typées (identifiant `app.verbe`, titre et description en français, paramètres,
//! niveau de risque) et les sert sur le bus de session (`org.noos.Actions.<App>`, interface
//! `org.noos.Actions1`). Le démon `noos-ai` les découvre et les propose au modèle comme
//! outils ; l'IA n'agit jamais autrement (pas de commande shell générée).
//!
//! Côté application :
//! ```ignore
//! let reg = noos_actions::Registry::new("Files", "Explorateur");
//! reg.add(
//!     ActionInfo::new("files.open-folder", "Ouvrir un dossier", "Ouvre un dossier dans l'Explorateur", Risk::Read)
//!         .param("chemin", Kind::String, "Dossier à ouvrir", true),
//!     |p| { /* … */ Ok("Dossier ouvert".into()) },
//! );
//! noos_actions::serve(reg);
//! ```

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const BUS_PREFIX: &str = "org.noos.Actions.";
pub const PATH: &str = "/org/noos/Actions";
pub const IFACE: &str = "org.noos.Actions1";

const XML: &str = r#"<node>
  <interface name="org.noos.Actions1">
    <method name="ListActions"><arg type="s" direction="out" name="json"/></method>
    <method name="Invoke">
      <arg type="s" direction="in" name="id"/>
      <arg type="s" direction="in" name="params"/>
      <arg type="s" direction="out" name="result"/>
    </method>
  </interface>
</node>"#;

/// Niveau de risque : `System` demande toujours une confirmation à l'utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Lecture seule (chercher, afficher, ouvrir).
    Read,
    /// Modifie un réglage ou un fichier de l'utilisateur.
    Change,
    /// Touche au système (mise à jour, extinction, suppression…).
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    String,
    Integer,
    Number,
    Boolean,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub kind: Kind,
    pub description: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub risk: Risk,
    #[serde(default)]
    pub params: Vec<Param>,
}

impl ActionInfo {
    pub fn new(id: &str, title: &str, description: &str, risk: Risk) -> Self {
        Self { id: id.into(), title: title.into(), description: description.into(), risk, params: vec![] }
    }

    pub fn param(mut self, name: &str, kind: Kind, description: &str, required: bool) -> Self {
        self.params.push(Param { name: name.into(), kind, description: description.into(), required, choices: vec![] });
        self
    }

    /// Valeurs permises pour le dernier paramètre ajouté.
    pub fn choices(mut self, values: &[&str]) -> Self {
        if let Some(p) = self.params.last_mut() {
            p.choices = values.iter().map(|s| s.to_string()).collect();
        }
        self
    }

    /// Vérifie les paramètres (présence, type, valeurs permises).
    pub fn check(&self, params: &Value) -> Result<(), String> {
        for p in &self.params {
            let v = params.get(&p.name);
            match v {
                None | Some(Value::Null) if p.required => return Err(format!("paramètre « {} » manquant", p.name)),
                None | Some(Value::Null) => continue,
                Some(v) => {
                    let ok = match p.kind {
                        Kind::String => v.is_string(),
                        Kind::Integer => v.is_i64() || v.is_u64(),
                        Kind::Number => v.is_number(),
                        Kind::Boolean => v.is_boolean(),
                    };
                    if !ok {
                        return Err(format!("paramètre « {} » : type attendu {:?}", p.name, p.kind));
                    }
                    if !p.choices.is_empty() && !v.as_str().is_some_and(|s| p.choices.iter().any(|c| c == s)) {
                        return Err(format!("paramètre « {} » : valeurs permises {}", p.name, p.choices.join(", ")));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Actions d'une application, telles que publiées sur le bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppActions {
    /// Suffixe du nom de bus (`Files` → `org.noos.Actions.Files`).
    pub app: String,
    /// Nom affiché (« Explorateur »).
    pub name: String,
    pub actions: Vec<ActionInfo>,
}

/// Résultat d'une action : message lisible pour l'utilisateur.
pub type Outcome = Result<String, String>;
type Handler = Rc<dyn Fn(&Value) -> Outcome>;

/// Registre d'actions d'une application (fil principal GTK).
pub struct Registry {
    app: String,
    name: String,
    actions: RefCell<Vec<(ActionInfo, Handler)>>,
    conn: RefCell<Option<gio::DBusConnection>>,
}

impl Registry {
    pub fn new(app: &str, name: &str) -> Rc<Self> {
        Rc::new(Self { app: app.into(), name: name.into(), actions: RefCell::new(vec![]), conn: RefCell::new(None) })
    }

    pub fn add(&self, info: ActionInfo, handler: impl Fn(&Value) -> Outcome + 'static) {
        self.actions.borrow_mut().push((info, Rc::new(handler)));
    }

    pub fn list(&self) -> AppActions {
        AppActions { app: self.app.clone(), name: self.name.clone(), actions: self.actions.borrow().iter().map(|(i, _)| i.clone()).collect() }
    }

    pub fn invoke(&self, id: &str, params: &Value) -> Outcome {
        let found = self.actions.borrow().iter().find(|(i, _)| i.id == id).map(|(i, h)| (i.clone(), h.clone()));
        let (info, handler) = found.ok_or_else(|| format!("action inconnue : {id}"))?;
        info.check(params)?;
        handler(params)
    }

    pub fn bus_name(&self) -> String {
        format!("{BUS_PREFIX}{}", self.app)
    }
}

/// Sert le registre sur le bus de session (`org.noos.Actions.<App>`).
pub fn serve(reg: Rc<Registry>) {
    let name = reg.bus_name();
    let _ = gio::bus_own_name(
        gio::BusType::Session,
        &name,
        gio::BusNameOwnerFlags::REPLACE | gio::BusNameOwnerFlags::ALLOW_REPLACEMENT,
        move |conn, _| {
            let node = gio::DBusNodeInfo::for_xml(XML).expect("XML org.noos.Actions1");
            let iface = node.lookup_interface(IFACE).expect("interface org.noos.Actions1");
            let reg2 = reg.clone();
            let res = conn
                .register_object(PATH, &iface)
                .method_call(move |_, _, _, _, method, params, invocation| {
                    let reply = match method {
                        "ListActions" => serde_json::to_string(&reg2.list()).unwrap_or_default(),
                        "Invoke" => {
                            let (id, p): (String, String) = params.get().unwrap_or_default();
                            let p: Value = serde_json::from_str(&p).unwrap_or(json!({}));
                            match reg2.invoke(&id, &p) {
                                Ok(m) => json!({"ok": true, "message": m}).to_string(),
                                Err(e) => json!({"ok": false, "message": e}).to_string(),
                            }
                        }
                        _ => String::new(),
                    };
                    invocation.return_value(Some(&(reply,).to_variant()));
                })
                .build();
            if let Err(e) = res {
                eprintln!("noos-actions : publication impossible : {e}");
            }
            *reg.conn.borrow_mut() = Some(conn);
        },
        |_, _| {},
        |_, name| eprintln!("noos-actions : nom {name} indisponible"),
    );
}

// ───────────────────────────── Client (démon noos-ai) ─────────────────────────────

/// Applications qui publient des actions (noms de bus).
pub fn discover(conn: &gio::DBusConnection) -> Vec<String> {
    conn.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ListNames",
        None,
        Some(glib::VariantTy::new("(as)").unwrap()),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    )
    .ok()
    .and_then(|v| v.get::<(Vec<String>,)>())
    .map(|(names,)| names.into_iter().filter(|n| n.starts_with(BUS_PREFIX)).collect())
    .unwrap_or_default()
}

fn call(conn: &gio::DBusConnection, bus: &str, method: &str, args: Option<&glib::Variant>, timeout_ms: i32) -> Result<String, String> {
    conn.call_sync(Some(bus), PATH, IFACE, method, args, Some(glib::VariantTy::new("(s)").unwrap()), gio::DBusCallFlags::NONE, timeout_ms, gio::Cancellable::NONE)
        .map_err(|e| e.to_string())?
        .get::<(String,)>()
        .map(|(s,)| s)
        .ok_or_else(|| "réponse invalide".into())
}

pub fn list(conn: &gio::DBusConnection, bus: &str) -> Result<AppActions, String> {
    serde_json::from_str(&call(conn, bus, "ListActions", None, 2000)?).map_err(|e| e.to_string())
}

pub fn invoke(conn: &gio::DBusConnection, bus: &str, id: &str, params: &Value) -> Outcome {
    let reply = call(conn, bus, "Invoke", Some(&(id, params.to_string()).to_variant()), 120_000)?;
    let v: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
    let msg = v.get("message").and_then(Value::as_str).unwrap_or("").to_string();
    if v.get("ok").and_then(Value::as_bool).unwrap_or(false) { Ok(msg) } else { Err(msg) }
}

/// Description pour le modèle (format « outil » d'Ollama / OpenAI).
pub fn as_tool(a: &ActionInfo) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = vec![];
    for p in &a.params {
        let ty = match p.kind {
            Kind::String => "string",
            Kind::Integer => "integer",
            Kind::Number => "number",
            Kind::Boolean => "boolean",
        };
        let mut o = json!({"type": ty, "description": p.description});
        if !p.choices.is_empty() {
            o["enum"] = json!(p.choices);
        }
        props.insert(p.name.clone(), o);
        if p.required {
            required.push(p.name.clone());
        }
    }
    json!({
        "type": "function",
        "function": {
            "name": a.id.replace(['.', '-'], "_"),
            "description": format!("{} — {}", a.title, a.description),
            "parameters": {"type": "object", "properties": props, "required": required},
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry() {
        let reg = Registry::new("Test", "Test");
        reg.add(
            ActionInfo::new("son.regler", "Régler le volume", "Volume en pourcentage", Risk::Change).param("pourcentage", Kind::Integer, "0 à 150", true),
            |p| Ok(format!("Volume : {} %", p["pourcentage"])),
        );
        assert_eq!(reg.invoke("son.regler", &json!({"pourcentage": 90})), Ok("Volume : 90 %".into()));
        assert!(reg.invoke("son.regler", &json!({})).is_err());
        assert!(reg.invoke("son.regler", &json!({"pourcentage": "fort"})).is_err());
        assert!(reg.invoke("inconnue", &json!({})).is_err());
        let tool = as_tool(&reg.list().actions[0]);
        assert_eq!(tool["function"]["name"], "son_regler");
        assert_eq!(tool["function"]["parameters"]["required"][0], "pourcentage");
    }

    #[test]
    fn choices() {
        let a = ActionInfo::new("x", "x", "x", Risk::Read).param("mode", Kind::String, "", true).choices(&["clair", "sombre"]);
        assert!(a.check(&json!({"mode": "sombre"})).is_ok());
        assert!(a.check(&json!({"mode": "bleu"})).is_err());
    }
}

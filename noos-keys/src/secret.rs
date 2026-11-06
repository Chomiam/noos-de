//! Client du Secret Service (`org.freedesktop.secrets`) : oo7, gnome-keyring, KeePassXC…
//!
//! Session « plain » : les secrets transitent en clair sur le bus D-Bus de session (local à
//! l'utilisateur, comme pour la plupart des clients libsecret qui n'imposent pas le
//! chiffrement de transport).

use std::collections::HashMap;

use anyhow::{Context, anyhow, bail};
use gtk::{gio, glib, prelude::*};
use glib::variant::ObjectPath;

const BUS: &str = "org.freedesktop.secrets";
const SERVICE: &str = "/org/freedesktop/secrets";
const I_SERVICE: &str = "org.freedesktop.Secret.Service";
const I_COLLECTION: &str = "org.freedesktop.Secret.Collection";
const I_ITEM: &str = "org.freedesktop.Secret.Item";
const I_PROMPT: &str = "org.freedesktop.Secret.Prompt";
const I_PROPS: &str = "org.freedesktop.DBus.Properties";

#[derive(Clone, Debug)]
pub struct Collection {
    pub path: String,
    pub label: String,
    pub locked: bool,
    pub items: Vec<String>,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub path: String,
    pub label: String,
    pub attributes: HashMap<String, String>,
    pub created: u64,
    pub modified: u64,
}

#[derive(Clone)]
pub struct Secrets {
    conn: gio::DBusConnection,
    session: String,
}

fn opath(s: &str) -> anyhow::Result<ObjectPath> {
    ObjectPath::try_from(s).map_err(|_| anyhow!("chemin D-Bus invalide : {s}"))
}

impl Secrets {
    pub async fn connect() -> anyhow::Result<Self> {
        let conn = gio::bus_get_future(gio::BusType::Session).await.context("bus de session indisponible")?;
        let mut s = Self { conn, session: String::new() };
        let reply = s
            .call(SERVICE, I_SERVICE, "OpenSession", ("plain", "".to_variant()).to_variant(), "(vo)")
            .await
            .context("aucun service de secrets (oo7, gnome-keyring…) ne répond")?;
        s.session = reply.child_value(1).str().unwrap_or("/").to_string();
        Ok(s)
    }

    async fn call(&self, path: &str, iface: &str, method: &str, args: glib::Variant, reply: &str) -> anyhow::Result<glib::Variant> {
        self.conn
            .call_future(
                Some(BUS),
                path,
                iface,
                method,
                Some(&args),
                Some(glib::VariantTy::new(reply).unwrap()),
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await
            .map_err(|e| anyhow!("{method} : {}", e.message()))
    }

    async fn get_all(&self, path: &str, iface: &str) -> anyhow::Result<HashMap<String, glib::Variant>> {
        let v = self.call(path, I_PROPS, "GetAll", (iface,).to_variant(), "(a{sv})").await?;
        v.child_value(0).get::<HashMap<String, glib::Variant>>().context("propriétés illisibles")
    }

    pub async fn collections(&self) -> anyhow::Result<Vec<Collection>> {
        let props = self.get_all(SERVICE, I_SERVICE).await?;
        let paths: Vec<String> = props
            .get("Collections")
            .map(|v| (0..v.n_children()).filter_map(|i| v.child_value(i).str().map(String::from)).collect())
            .unwrap_or_default();
        let default = self
            .call(SERVICE, I_SERVICE, "ReadAlias", ("default",).to_variant(), "(o)")
            .await
            .ok()
            .and_then(|v| v.child_value(0).str().map(String::from))
            .unwrap_or_default();
        let mut out = Vec::new();
        for path in paths {
            let Ok(p) = self.get_all(&path, I_COLLECTION).await else { continue };
            let items = p
                .get("Items")
                .map(|v| (0..v.n_children()).filter_map(|i| v.child_value(i).str().map(String::from)).collect())
                .unwrap_or_default();
            out.push(Collection {
                label: p.get("Label").and_then(|v| v.get::<String>()).unwrap_or_else(|| path.rsplit('/').next().unwrap_or("").to_string()),
                locked: p.get("Locked").and_then(|v| v.get::<bool>()).unwrap_or(true),
                items,
                is_default: path == default,
                path,
            });
        }
        // Trousseau par défaut en premier, « session » (temporaire) en dernier.
        out.sort_by_key(|c| (!c.is_default, c.path.ends_with("/session"), c.label.to_lowercase()));
        Ok(out)
    }

    pub async fn items(&self, collection: &Collection) -> Vec<Item> {
        let mut out = Vec::new();
        for path in &collection.items {
            let Ok(p) = self.get_all(path, I_ITEM).await else { continue };
            out.push(Item {
                path: path.clone(),
                label: p.get("Label").and_then(|v| v.get::<String>()).unwrap_or_default(),
                attributes: p.get("Attributes").and_then(|v| v.get::<HashMap<String, String>>()).unwrap_or_default(),
                created: p.get("Created").and_then(|v| v.get::<u64>()).unwrap_or(0),
                modified: p.get("Modified").and_then(|v| v.get::<u64>()).unwrap_or(0),
            });
        }
        out.sort_by_key(|i| i.label.to_lowercase());
        out
    }

    pub async fn secret(&self, item: &str) -> anyhow::Result<String> {
        let v = self.call(item, I_ITEM, "GetSecret", (opath(&self.session)?,).to_variant(), "((oayays))").await?;
        let value = v.child_value(0).child_value(2).get::<Vec<u8>>().unwrap_or_default();
        Ok(String::from_utf8_lossy(&value).to_string())
    }

    fn secret_struct(&self, value: &str) -> anyhow::Result<glib::Variant> {
        Ok((opath(&self.session)?, Vec::<u8>::new(), value.as_bytes().to_vec(), "text/plain").to_variant())
    }

    pub async fn set_secret(&self, item: &str, value: &str) -> anyhow::Result<()> {
        let secret = self.secret_struct(value)?;
        self.call(item, I_ITEM, "SetSecret", glib::Variant::tuple_from_iter([secret]), "()").await?;
        Ok(())
    }

    pub async fn set_label(&self, item: &str, label: &str) -> anyhow::Result<()> {
        self.call(item, I_PROPS, "Set", (I_ITEM, "Label", label.to_variant()).to_variant(), "()").await?;
        Ok(())
    }

    /// Exécute une invite (mot de passe du trousseau…) affichée par le service lui-même.
    /// Renvoie `false` si l'utilisateur l'a annulée.
    async fn prompt(&self, prompt: &str) -> anyhow::Result<bool> {
        if prompt == "/" || prompt.is_empty() {
            return Ok(true);
        }
        let (tx, rx) = async_channel::bounded(1);
        let sub = self.conn.subscribe_to_signal(
            Some(BUS),
            Some(I_PROMPT),
            Some("Completed"),
            Some(prompt),
            None,
            gio::DBusSignalFlags::NONE,
            move |sig| {
                let dismissed = sig.parameters.child_value(0).get::<bool>().unwrap_or(true);
                let _ = tx.try_send(dismissed);
            },
        );
        self.call(prompt, I_PROMPT, "Prompt", ("",).to_variant(), "()").await?;
        let dismissed = rx.recv().await.unwrap_or(true);
        drop(sub);
        Ok(!dismissed)
    }

    fn paths_variant(paths: &[String]) -> anyhow::Result<glib::Variant> {
        let list: Vec<ObjectPath> = paths.iter().map(|p| opath(p)).collect::<anyhow::Result<_>>()?;
        Ok((list,).to_variant())
    }

    pub async fn unlock(&self, paths: &[String]) -> anyhow::Result<bool> {
        let v = self.call(SERVICE, I_SERVICE, "Unlock", Self::paths_variant(paths)?, "(aoo)").await?;
        let prompt = v.child_value(1).str().unwrap_or("/").to_string();
        self.prompt(&prompt).await
    }

    pub async fn lock(&self, paths: &[String]) -> anyhow::Result<()> {
        let v = self.call(SERVICE, I_SERVICE, "Lock", Self::paths_variant(paths)?, "(aoo)").await?;
        let prompt = v.child_value(1).str().unwrap_or("/").to_string();
        self.prompt(&prompt).await?;
        Ok(())
    }

    pub async fn create_item(
        &self,
        collection: &str,
        label: &str,
        attributes: HashMap<String, String>,
        value: &str,
    ) -> anyhow::Result<()> {
        let mut props: HashMap<String, glib::Variant> = HashMap::new();
        props.insert("org.freedesktop.Secret.Item.Label".into(), label.to_variant());
        props.insert("org.freedesktop.Secret.Item.Attributes".into(), attributes.to_variant());
        let args = glib::Variant::tuple_from_iter([props.to_variant(), self.secret_struct(value)?, false.to_variant()]);
        let v = self.call(collection, I_COLLECTION, "CreateItem", args, "(oo)").await?;
        let prompt = v.child_value(1).str().unwrap_or("/").to_string();
        if !self.prompt(&prompt).await? {
            bail!("annulé");
        }
        Ok(())
    }

    pub async fn delete_item(&self, item: &str) -> anyhow::Result<()> {
        let v = self.call(item, I_ITEM, "Delete", ().to_variant(), "(o)").await?;
        self.prompt(v.child_value(0).str().unwrap_or("/")).await?;
        Ok(())
    }

    pub async fn create_collection(&self, label: &str) -> anyhow::Result<()> {
        let mut props: HashMap<String, glib::Variant> = HashMap::new();
        props.insert("org.freedesktop.Secret.Collection.Label".into(), label.to_variant());
        let v = self.call(SERVICE, I_SERVICE, "CreateCollection", (props, "").to_variant(), "(oo)").await?;
        let prompt = v.child_value(1).str().unwrap_or("/").to_string();
        if !self.prompt(&prompt).await? {
            bail!("annulé");
        }
        Ok(())
    }

    pub async fn delete_collection(&self, path: &str) -> anyhow::Result<()> {
        let v = self.call(path, I_COLLECTION, "Delete", ().to_variant(), "(o)").await?;
        self.prompt(v.child_value(0).str().unwrap_or("/")).await?;
        Ok(())
    }

    pub async fn set_default(&self, path: &str) -> anyhow::Result<()> {
        self.call(SERVICE, I_SERVICE, "SetAlias", ("default", opath(path)?).to_variant(), "()").await?;
        Ok(())
    }
}

/// Description lisible d'un élément : (icône, sous-titre).
pub fn describe(item: &Item) -> (&'static str, String) {
    let a = &item.attributes;
    let get = |k: &str| a.get(k).filter(|v| !v.is_empty()).cloned();
    let schema = get("xdg:schema").unwrap_or_default();
    let app = get("application").unwrap_or_default().to_lowercase();
    let icon = if schema.contains("NetworkManager") || a.contains_key("connection-uuid") {
        "network-wireless-symbolic"
    } else if app.contains("chrom") || schema.contains("chrome") {
        "web-browser-symbolic"
    } else if a.contains_key("origin_url") || a.contains_key("server") || a.contains_key("url") || schema.contains("NetworkPassword") {
        "web-browser-symbolic"
    } else if schema.contains("Note") {
        "text-x-generic-symbolic"
    } else if a.contains_key("ssh") || schema.contains("ssh") {
        "network-server-symbolic"
    } else {
        "dialog-password-symbolic"
    };
    let user = get("username").or_else(|| get("user")).or_else(|| get("username_value")).or_else(|| get("account"));
    let place = get("origin_url").or_else(|| get("url")).or_else(|| get("server")).or_else(|| get("service")).or_else(|| get("application"));
    let sub = match (user, place) {
        (Some(u), Some(p)) => format!("{u} · {p}"),
        (Some(u), None) => u,
        (None, Some(p)) => p,
        (None, None) => schema,
    };
    (icon, sub)
}

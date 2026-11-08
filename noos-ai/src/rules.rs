//! Commandes rapides : phrases françaises courantes reconnues **sans modèle de langage**
//! (instantané, aucune ressource). Le modèle n'est sollicité que si rien ne correspond.

use serde_json::{Value, json};

/// Action reconnue : identifiant (voir `actions.rs`) et paramètres.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub action: &'static str,
    pub params: Value,
}

fn intent(action: &'static str, params: Value) -> Option<Intent> {
    Some(Intent { action, params })
}

/// Minuscules, sans accents ni ponctuation, espaces simples.
pub fn normalize(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        let c = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '\'' | '’' | '-' | ',' | '.' | '!' | '?' | ';' | ':' => ' ',
            c => c,
        };
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has(t: &str, words: &[&str]) -> bool {
    words.iter().any(|w| t.split(' ').any(|x| x == *w) || (w.contains(' ') && t.contains(w)))
}

/// Premier nombre de la phrase (« 90 », « 90% »).
fn number(t: &str) -> Option<i64> {
    t.split(|c: char| !c.is_ascii_digit()).find(|s| !s.is_empty()).and_then(|s| s.parse().ok())
}

/// Pages de Paramètres : mots-clés → page (`noos-settings <page>`).
const SETTINGS_PAGES: &[(&[&str], &str)] = &[
    (&["wifi", "wi fi", "sans fil"], "wifi"),
    (&["dns"], "dns"),
    (&["vpn", "wireguard"], "vpn"),
    (&["reseau", "internet", "connexion"], "network"),
    (&["ecran", "affichage", "resolution", "moniteur"], "display"),
    (&["alimentation", "batterie", "energie", "veille"], "power"),
    (&["notification", "notifications"], "notifications"),
    (&["fond d ecran", "fond", "arriere plan", "wallpaper"], "background"),
    (&["couleur", "couleurs", "mode sombre", "mode clair"], "colors"),
    (&["theme", "themes"], "themes"),
    (&["barre des taches", "barre", "dock"], "taskbar"),
    (&["fenetre", "fenetres", "ancrage"], "windows"),
    (&["verrouillage"], "lockscreen"),
    (&["connexion", "login"], "login"),
    (&["police", "polices"], "fonts"),
    (&["applications par defaut", "par defaut"], "default-apps"),
    (&["application", "applications"], "apps"),
    (&["ia", "intelligence artificielle", "assistant", "dictee"], "ai"),
    (&["jeu", "jeux"], "gaming"),
    (&["pare feu", "firewall", "securite", "confidentialite"], "firewall"),
    (&["mise a jour", "mises a jour"], "updates"),
    (&["bluetooth", "souris", "clavier", "peripheriques"], "devices"),
    (&["systeme", "a propos", "informations"], "about"),
];

/// Reconnaît une commande ; `None` → à confier au modèle.
pub fn parse(input: &str) -> Option<Intent> {
    let t = normalize(input);
    let t = t.as_str();
    if t.is_empty() {
        return None;
    }

    // ── Son ──
    let sound = has(t, &["son", "volume", "audio"]);
    if sound || has(t, &["mute", "muet", "silence"]) {
        if has(t, &["coupe", "couper", "mute", "muet", "silence"]) && !has(t, &["remets", "remet", "reactive", "reactiver", "retablis"]) {
            return intent("system.mute", json!({"etat": "on"}));
        }
        if has(t, &["remets", "remet", "reactive", "reactiver", "retablis", "remettre"]) {
            return intent("system.mute", json!({"etat": "off"}));
        }
        if let Some(n) = number(t) {
            return intent("system.volume.set", json!({"pourcentage": n.clamp(0, 150)}));
        }
        if has(t, &["monte", "monter", "augmente", "augmenter", "plus fort", "hausse"]) {
            return intent("system.volume.change", json!({"delta": 10}));
        }
        if has(t, &["baisse", "baisser", "diminue", "diminuer", "moins fort", "descends"]) {
            return intent("system.volume.change", json!({"delta": -10}));
        }
    }

    // ── Luminosité ──
    if has(t, &["luminosite", "lumineux", "lumiere"]) {
        if let Some(n) = number(t) {
            return intent("system.brightness.set", json!({"pourcentage": n.clamp(1, 100)}));
        }
        if has(t, &["monte", "augmente", "plus"]) {
            return intent("system.brightness.change", json!({"delta": 10}));
        }
        if has(t, &["baisse", "diminue", "moins"]) {
            return intent("system.brightness.change", json!({"delta": -10}));
        }
    }

    // ── Mode sombre / clair ──
    if has(t, &["mode sombre", "theme sombre", "passe en sombre"]) {
        return intent("appearance.scheme", json!({"mode": "sombre"}));
    }
    if has(t, &["mode clair", "theme clair", "passe en clair"]) {
        return intent("appearance.scheme", json!({"mode": "clair"}));
    }

    // ── Mises à jour ──
    if has(t, &["mets a jour", "met a jour", "mettre a jour", "faire les mises a jour", "lance les mises a jour", "update", "upgrade"])
        || (has(t, &["mise a jour", "mises a jour"]) && has(t, &["fais", "fait", "lance", "installe", "faire"]))
    {
        return intent("system.update", json!({}));
    }

    // ── Session ──
    if has(t, &["verrouille", "verrouiller", "verrouilles", "bloque l ecran"]) {
        return intent("session.lock", json!({}));
    }
    if has(t, &["redemarre", "redemarrer", "reboot"]) {
        return intent("session.reboot", json!({}));
    }
    if has(t, &["eteins", "eteindre", "eteint", "arrete l ordinateur", "arrete le pc", "shutdown"]) {
        return intent("session.poweroff", json!({}));
    }
    if has(t, &["mets en veille", "met en veille", "mise en veille", "veille prolongee"]) || t == "veille" {
        return intent("session.suspend", json!({}));
    }
    if has(t, &["deconnecte", "deconnecter", "deconnexion", "ferme la session"]) {
        return intent("session.logout", json!({}));
    }

    // ── Wi-Fi ──
    if has(t, &["wifi", "wi fi"]) && !has(t, &["parametres", "reglages"]) {
        if has(t, &["active", "activer", "allume", "allumer", "remets"]) && !has(t, &["desactive", "desactiver"]) {
            return intent("network.wifi", json!({"etat": "on"}));
        }
        if has(t, &["desactive", "desactiver", "coupe", "couper", "eteins"]) {
            return intent("network.wifi", json!({"etat": "off"}));
        }
    }

    // ── Capture d'écran ──
    if has(t, &["capture d ecran", "capture ecran", "screenshot", "capture"]) {
        return intent("system.screenshot", json!({}));
    }

    // ── Heure / date ──
    if has(t, &["quelle heure", "l heure"]) {
        return intent("info.time", json!({}));
    }
    if has(t, &["quel jour", "quelle date", "on est le"]) {
        return intent("info.date", json!({}));
    }

    // ── Ouvrir des paramètres ──
    let open = has(t, &["ouvre", "ouvrir", "affiche", "afficher", "montre", "va dans", "aller dans", "lance"]);
    if has(t, &["parametres", "parametre", "reglages", "reglage", "configuration"]) && (open || t.starts_with("parametres") || t.starts_with("reglages")) {
        let page = SETTINGS_PAGES.iter().find(|(kw, _)| has(t, kw)).map(|(_, p)| *p);
        return intent("settings.open", match page {
            Some(p) => json!({"page": p}),
            None => json!({}),
        });
    }

    // ── Ouvrir une application : « ouvre firefox », « lance le terminal » ──
    for verb in ["ouvre ", "ouvrir ", "lance ", "lancer ", "demarre ", "demarrer "] {
        if let Some(rest) = t.strip_prefix(verb).or_else(|| t.find(verb).map(|i| &t[i + verb.len()..])) {
            let name = rest.trim_start_matches("le ").trim_start_matches("la ").trim_start_matches("les ").trim_start_matches("l ").trim_start_matches("un ").trim();
            if !name.is_empty() && name.split(' ').count() <= 4 {
                return intent("app.open", json!({"nom": name}));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(t: &str) -> Option<(&'static str, Value)> {
        parse(t).map(|i| (i.action, i.params))
    }

    #[test]
    fn sound() {
        assert_eq!(a("Augmente le son à 90%"), Some(("system.volume.set", json!({"pourcentage": 90}))));
        assert_eq!(a("monte le volume"), Some(("system.volume.change", json!({"delta": 10}))));
        assert_eq!(a("baisse un peu le son"), Some(("system.volume.change", json!({"delta": -10}))));
        assert_eq!(a("Coupe le son"), Some(("system.mute", json!({"etat": "on"}))));
        assert_eq!(a("remets le son"), Some(("system.mute", json!({"etat": "off"}))));
        assert_eq!(a("volume 300"), Some(("system.volume.set", json!({"pourcentage": 150}))));
    }

    #[test]
    fn settings() {
        assert_eq!(a("Ouvre les paramètres"), Some(("settings.open", json!({}))));
        assert_eq!(a("ouvre les paramètres réseau"), Some(("settings.open", json!({"page": "network"}))));
        assert_eq!(a("affiche les réglages du wifi"), Some(("settings.open", json!({"page": "wifi"}))));
        assert_eq!(a("ouvre les paramètres de la barre des tâches"), Some(("settings.open", json!({"page": "taskbar"}))));
    }

    #[test]
    fn system() {
        assert_eq!(a("Mets à jour mon système"), Some(("system.update", json!({}))));
        assert_eq!(a("fais les mises à jour"), Some(("system.update", json!({}))));
        assert_eq!(a("verrouille l'écran"), Some(("session.lock", json!({}))));
        assert_eq!(a("Éteins l'ordinateur"), Some(("session.poweroff", json!({}))));
        assert_eq!(a("passe en mode sombre"), Some(("appearance.scheme", json!({"mode": "sombre"}))));
        assert_eq!(a("désactive le wifi"), Some(("network.wifi", json!({"etat": "off"}))));
        assert_eq!(a("quelle heure est-il ?"), Some(("info.time", json!({}))));
    }

    #[test]
    fn apps() {
        assert_eq!(a("Ouvre Firefox"), Some(("app.open", json!({"nom": "firefox"}))));
        assert_eq!(a("lance le terminal"), Some(("app.open", json!({"nom": "terminal"}))));
        assert_eq!(a("peux-tu ouvrir l'explorateur"), Some(("app.open", json!({"nom": "explorateur"}))));
    }

    #[test]
    fn unknown() {
        assert_eq!(a("trouve-moi la facture EDF de mars et résume-la"), None);
        assert_eq!(a(""), None);
    }
}

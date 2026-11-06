//! Dispositions clavier XKB (`/usr/share/X11/xkb/rules/evdev.xml`).

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub layout: String,
    pub variant: String,
    pub description: String,
}

impl Layout {
    pub fn id(&self) -> String {
        if self.variant.is_empty() { self.layout.clone() } else { format!("{}:{}", self.layout, self.variant) }
    }
}

/// Noms français des dispositions les plus courantes.
fn french(layout: &str, variant: &str, english: &str) -> String {
    let base = match layout {
        "fr" => "Français",
        "be" => "Belge",
        "ch" => "Suisse",
        "ca" => "Canadien",
        "us" => "Anglais (États-Unis)",
        "gb" => "Anglais (Royaume-Uni)",
        "de" => "Allemand",
        "es" => "Espagnol",
        "it" => "Italien",
        "br" => "Portugais (Brésil)",
        "pt" => "Portugais",
        "nl" => "Néerlandais",
        _ => return english.to_string(),
    };
    if variant.is_empty() {
        return base.to_string();
    }
    match (layout, variant) {
        ("fr", "bepo") => "Français (BÉPO)".into(),
        ("fr", "bepo_afnor") => "Français (BÉPO, AFNOR)".into(),
        ("fr", "afnor") => "Français (AZERTY AFNOR)".into(),
        ("fr", "oss") => "Français (variante)".into(),
        ("fr", "mac") => "Français (Macintosh)".into(),
        ("fr", "nodeadkeys") => "Français (sans touches mortes)".into(),
        ("ch", "fr") => "Suisse romand".into(),
        ("ca", "fr") | ("ca", "") => "Canadien français".into(),
        ("us", "intl") => "Anglais (US, international avec touches mortes)".into(),
        ("us", "altgr-intl") => "Anglais (US, international AltGr)".into(),
        _ => {
            // « French (Macintosh) » → « Français (Macintosh) ».
            let detail = english.split_once('(').map(|(_, d)| d.trim_end_matches(')')).unwrap_or(variant);
            format!("{base} ({detail})")
        }
    }
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim())
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

pub fn parse(xml: &str) -> Vec<Layout> {
    let mut out = Vec::new();
    let Some(list_start) = xml.find("<layoutList>") else { return out };
    let list_end = xml.find("</layoutList>").unwrap_or(xml.len());
    let body = &xml[list_start..list_end];
    for chunk in body.split("<layout>").skip(1) {
        let chunk = chunk.split("</layout>").next().unwrap_or(chunk);
        let (main, variants) = match chunk.find("<variantList>") {
            Some(i) => (&chunk[..i], &chunk[i..]),
            None => (chunk, ""),
        };
        let Some(name) = tag(main, "name") else { continue };
        let desc = unescape(tag(main, "description").unwrap_or(name));
        out.push(Layout { layout: name.into(), variant: String::new(), description: french(name, "", &desc) });
        for v in variants.split("<variant>").skip(1) {
            let (Some(vn), Some(vd)) = (tag(v, "name"), tag(v, "description")) else { continue };
            out.push(Layout { layout: name.into(), variant: vn.into(), description: french(name, vn, &unescape(vd)) });
        }
    }
    out.sort_by(|a, b| a.description.to_lowercase().cmp(&b.description.to_lowercase()));
    out
}

pub fn layouts() -> Vec<Layout> {
    std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.xml").map(|x| parse(&x)).unwrap_or_default()
}

/// Applique la disposition à la session de l'installateur (compositeur NOOS : config
/// `noos.toml` de root puis rechargement ; la saisie de test l'utilise aussitôt).
pub fn apply_live(layout: &str, variant: &str) {
    // Seulement dans le système live (root) : jamais dans la session d'un utilisateur.
    if !crate::install::is_root() {
        return;
    }
    let mut cfg = noos_core::NoosConfig::load();
    cfg.keyboard.layout = layout.to_string();
    cfg.keyboard.variant = variant.to_string();
    if cfg.save().is_ok() {
        let _ = noos_core::ipc::IpcClient::fire(&noos_core::ipc::Request::ReloadConfig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xkb() {
        let xml = "<xkbConfigRegistry><layoutList><layout><configItem><name>fr</name><shortDescription>fr</shortDescription><description>French</description></configItem><variantList><variant><configItem><name>bepo</name><description>French (BEPO)</description></configItem></variant><variant><configItem><name>mac</name><description>French (Macintosh)</description></configItem></variant></variantList></layout><layout><configItem><name>us</name><description>English (US)</description></configItem></layout></layoutList></xkbConfigRegistry>";
        let l = parse(xml);
        assert_eq!(l.len(), 4);
        assert!(l.iter().any(|x| x.description == "Français (BÉPO)" && x.id() == "fr:bepo"));
        assert!(l.iter().any(|x| x.description == "Anglais (États-Unis)"));
    }
}

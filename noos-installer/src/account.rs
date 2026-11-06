//! Validation du compte : identifiant, nom de la machine, robustesse du mot de passe.

/// Noms réservés par le système (comptes de service Fedora courants).
const RESERVED: &[&str] = &[
    "root", "bin", "daemon", "adm", "lp", "sync", "shutdown", "halt", "mail", "operator", "games", "ftp", "nobody",
    "dbus", "systemd-network", "systemd-resolve", "systemd-timesync", "systemd-oom", "systemd-coredump", "tss",
    "polkitd", "avahi", "sshd", "chrony", "greeter", "geoclue", "pipewire", "rtkit", "colord", "gdm", "flatpak",
    "wheel", "users", "admin", "sudo", "nm-openconnect", "unbound", "dnsmasq", "qemu", "libvirt",
];

/// Erreur d'identifiant (None = valide). Règles de `useradd` en mode strict (portable) :
/// minuscules, chiffres, `_` et `-`, commence par une lettre ou `_`, 32 caractères au plus.
pub fn username_error(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("Choisissez un identifiant.".into());
    }
    if name.len() > 32 {
        return Some("32 caractères au maximum.".into());
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')) {
        let what = match bad {
            ' ' => "les espaces".to_string(),
            c if c.is_uppercase() => "les majuscules".to_string(),
            c if !c.is_ascii() => format!("les accents et caractères spéciaux (« {c} »)"),
            c => format!("le caractère « {c} »"),
        };
        return Some(format!("Interdit dans un identifiant : {what}. Autorisés : a-z, 0-9, « - » et « _ »."));
    }
    let first = name.chars().next().unwrap();
    if !(first.is_ascii_lowercase() || first == '_') {
        return Some("L'identifiant doit commencer par une lettre minuscule.".into());
    }
    if name.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '_') {
        return Some("L'identifiant doit contenir au moins une lettre.".into());
    }
    if RESERVED.contains(&name) {
        return Some(format!("« {name} » est réservé par le système."));
    }
    None
}

/// Identifiant proposé à partir du nom complet : « Axel Valens » → « axel ».
pub fn suggest_username(full_name: &str) -> String {
    let first = full_name.split_whitespace().next().unwrap_or("");
    let s: String = first
        .chars()
        .filter_map(|c| {
            let c = match c {
                'à' | 'â' | 'ä' | 'á' | 'À' | 'Â' | 'Ä' | 'Á' => 'a',
                'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
                'î' | 'ï' | 'Î' | 'Ï' => 'i',
                'ô' | 'ö' | 'Ô' | 'Ö' => 'o',
                'ù' | 'û' | 'ü' | 'Ù' | 'Û' | 'Ü' => 'u',
                'ç' | 'Ç' => 'c',
                c => c.to_ascii_lowercase(),
            };
            (c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-').then_some(c)
        })
        .collect();
    let s = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-').to_string();
    s.chars().take(32).collect()
}

/// Nom de machine (RFC 1123) : lettres, chiffres et tirets, 63 caractères au plus.
pub fn hostname_error(h: &str) -> Option<String> {
    if h.is_empty() {
        return Some("Choisissez un nom pour cet ordinateur.".into());
    }
    if h.len() > 63 {
        return Some("63 caractères au maximum.".into());
    }
    if h.starts_with('-') || h.ends_with('-') {
        return Some("Le nom ne peut ni commencer ni finir par un tiret.".into());
    }
    if let Some(c) = h.chars().find(|c| !(c.is_ascii_alphanumeric() || *c == '-')) {
        return Some(format!("Caractère interdit dans le nom de l'ordinateur : « {c} »."));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Strength {
    Empty,
    VeryWeak,
    Weak,
    Fair,
    Strong,
    VeryStrong,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Strength::Empty => "",
            Strength::VeryWeak => "Très faible",
            Strength::Weak => "Faible",
            Strength::Fair => "Moyen",
            Strength::Strong => "Fort",
            Strength::VeryStrong => "Très fort",
        }
    }

    /// Valeur de la jauge (0 – 5).
    pub fn level(self) -> f64 {
        self as u8 as f64
    }

    pub fn css(self) -> &'static str {
        match self {
            Strength::Empty | Strength::VeryWeak => "pw-very-weak",
            Strength::Weak => "pw-weak",
            Strength::Fair => "pw-fair",
            Strength::Strong => "pw-strong",
            Strength::VeryStrong => "pw-very-strong",
        }
    }
}

const COMMON: &[&str] = &[
    "password", "motdepasse", "azerty", "qwerty", "123456", "admin", "bonjour", "soleil", "louloute", "doudou",
    "loulou", "chouchou", "marseille", "iloveyou", "welcome", "letmein", "dragon", "monkey", "football", "noos",
    "fedora", "linux", "secret", "abc123", "000000", "111111",
];

/// Estimation de l'entropie (bits), pénalisée pour les motifs courants.
pub fn entropy(pw: &str, context: &[&str]) -> f64 {
    if pw.is_empty() {
        return 0.0;
    }
    let mut pool = 0.0;
    if pw.chars().any(|c| c.is_ascii_lowercase()) {
        pool += 26.0;
    }
    if pw.chars().any(|c| c.is_ascii_uppercase()) {
        pool += 26.0;
    }
    if pw.chars().any(|c| c.is_ascii_digit()) {
        pool += 10.0;
    }
    if pw.chars().any(|c| c.is_ascii_punctuation() || c == ' ') {
        pool += 33.0;
    }
    if pw.chars().any(|c| !c.is_ascii()) {
        pool += 40.0;
    }
    // Caractères distincts : « aaaaaaaa » ne vaut pas 8 caractères aléatoires.
    let mut uniq: Vec<char> = pw.chars().collect();
    uniq.sort();
    uniq.dedup();
    let len = pw.chars().count() as f64;
    let effective = len.min(uniq.len() as f64 * 1.5);
    let mut bits = effective * (pool as f64).log2();
    let lower = pw.to_lowercase();
    for word in COMMON.iter().chain(context.iter()).filter(|w| w.len() >= 3) {
        if lower.contains(&word.to_lowercase()) {
            bits -= (word.len() as f64) * 3.5;
        }
    }
    // Suites (abcd, 1234, azer…).
    let seqs = ["abcdefghijklmnopqrstuvwxyz", "0123456789", "azertyuiop", "qwertyuiop", "qsdfghjklm", "asdfghjkl"];
    for s in seqs {
        for win in 4..=s.len().min(8) {
            for i in 0..=s.len() - win {
                if lower.contains(&s[i..i + win]) {
                    bits -= win as f64 * 2.0;
                    break;
                }
            }
        }
    }
    bits.max(1.0)
}

pub fn strength(pw: &str, context: &[&str]) -> Strength {
    if pw.is_empty() {
        return Strength::Empty;
    }
    let bits = entropy(pw, context);
    match bits {
        b if b < 28.0 => Strength::VeryWeak,
        b if b < 40.0 => Strength::Weak,
        b if b < 55.0 => Strength::Fair,
        b if b < 75.0 => Strength::Strong,
        _ => Strength::VeryStrong,
    }
}

/// Exigences du mode « mot de passe fort » non remplies.
pub fn strong_requirements(pw: &str) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if pw.chars().count() < 12 {
        missing.push("12 caractères minimum");
    }
    if !pw.chars().any(|c| c.is_lowercase()) || !pw.chars().any(|c| c.is_uppercase()) {
        missing.push("majuscules et minuscules");
    }
    if !pw.chars().any(|c| c.is_ascii_digit()) {
        missing.push("un chiffre");
    }
    if !pw.chars().any(|c| !c.is_alphanumeric()) {
        missing.push("un symbole");
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert!(username_error("axel").is_none());
        assert!(username_error("_svc-1").is_none());
        assert!(username_error("Axel").unwrap().contains("majuscules"));
        assert!(username_error("axel valens").unwrap().contains("espaces"));
        assert!(username_error("élodie").unwrap().contains("accents"));
        assert!(username_error("1axel").is_some());
        assert!(username_error("root").unwrap().contains("réservé"));
        assert!(username_error("a@b").unwrap().contains("@"));
        assert_eq!(suggest_username("Élodie Dupont"), "elodie");
        assert!(hostname_error("axel-pc").is_none());
        assert!(hostname_error("mon pc").is_some());
    }

    #[test]
    fn strengths() {
        assert_eq!(strength("", &[]), Strength::Empty);
        assert!(strength("azerty", &[]) <= Strength::VeryWeak);
        assert!(strength("motdepasse123", &[]) <= Strength::Weak);
        assert!(strength("Tr0ub4dor&3", &[]) >= Strength::Fair);
        assert!(strength("correct-Horse-battery-st4ple!", &[]) >= Strength::Strong);
        assert!(strength("axelvalens2024", &["axelvalens"]) < strength("Qz8!vR2#pL9@", &[]));
        assert!(strong_requirements("Abcdefgh1!xy").is_empty());
        assert_eq!(strong_requirements("abc").len(), 4);
    }
}

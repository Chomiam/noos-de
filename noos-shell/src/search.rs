//! Recherche façon Windows 11 : on clique dans la pastille « Rechercher » de la barre et on
//! tape directement dedans ; les résultats (applications, paramètres, fichiers, calcul)
//! s'affichent en direct dans un panneau contre la barre. Le menu Démarrer utilise le même
//! moteur (`query`) et la même liste (`Results`).
//!
//! Clavier : la barre est en `KeyboardMode::None` (un clic sur une icône ne doit pas prendre
//! le clavier à la fenêtre active). Le pointeur au-dessus de la pastille la passe en
//! `OnDemand` : le compositeur donne alors le clavier à la barre au clic (`focus_layer_under`).
//! La recherche finie (Échap, lancement, clic ailleurs), elle repasse en `None`.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use noos_core::ipc::{Request, WindowId};

use crate::{Shell, apps};

/// Taille du panneau de résultats (comme celui de Windows 11).
const PANEL_W: i32 = 600;
const PANEL_H: i32 = 540;
/// Nombre de résultats par groupe.
const MAX_APPS: usize = 6;
const MAX_SETTINGS: usize = 4;
const MAX_FILES: usize = 6;

// ───────────────────────────── Moteur (sans GTK) ─────────────────────────────

/// Minuscules sans accents : « Paramètres » et « parametres » se valent.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => out.push('a'),
            'ç' => out.push('c'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'î' | 'ï' | 'í' | 'ì' => out.push('i'),
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => out.push('o'),
            'ù' | 'û' | 'ü' | 'ú' => out.push('u'),
            'ÿ' => out.push('y'),
            'ñ' => out.push('n'),
            'œ' => out.push_str("oe"),
            'æ' => out.push_str("ae"),
            '’' => out.push('\''),
            _ => out.push(c),
        }
    }
    out
}

/// Pages de Paramètres : (libellé, mots-clés, page `noos-settings <page>`, icône).
/// À tenir à jour avec `PAGES` / `SEARCH` de noos-settings/src/main.rs.
pub const SETTINGS: &[(&str, &str, &str, &str)] = &[
    ("Système", "système", "system", "computer-symbolic"),
    ("Écran", "écran moniteur display résolution définition fréquence hz rafraîchissement échelle zoom dpi orientation rotation hdr plusieurs écrans disposition", "display", "video-display-symbolic"),
    ("Notifications", "notifications ne pas déranger alertes bannières", "notifications", "preferences-system-notifications-symbolic"),
    ("Alimentation et batterie", "alimentation énergie batterie veille extinction performances économie profil mode", "power", "battery-good-symbolic"),
    ("Informations système", "à propos processeur mémoire ram version noyau nom du pc renommer hostname", "about", "computer-symbolic"),
    ("Bluetooth et appareils", "bluetooth casque écouteurs manette associer appairer jumeler appareil sans fil", "bluetooth", "bluetooth-symbolic"),
    ("Imprimantes et scanners", "imprimante imprimer scanner numériser cups file d'attente pilote", "printers", "printer-symbolic"),
    ("Souris", "souris pointeur vitesse défilement pavé tactile", "mouse", "input-mouse-symbolic"),
    ("Clavier", "clavier disposition azerty qwerty", "keyboard", "input-keyboard-symbolic"),
    ("Réseau et Internet", "réseau internet ethernet connexion câble", "network", "network-wired-symbolic"),
    ("Wi-Fi", "wifi wi-fi sans fil réseau mot de passe", "wifi", "network-wireless-symbolic"),
    ("DNS", "dns serveur quad9 cloudflare adguard confidentialité chiffré tls", "dns", "network-server-symbolic"),
    ("VPN", "vpn wireguard tunnel conf mullvad proton", "vpn", "network-vpn-symbolic"),
    ("Adresse IP", "ip adresse fixe statique dhcp passerelle mac", "ip", "network-wired-symbolic"),
    ("Point d'accès mobile", "point d'accès hotspot partage connexion", "hotspot", "network-wireless-hotspot-symbolic"),
    ("Diagnostic réseau", "dépanner internet ne marche pas diagnostic ping", "network-diagnostics", "network-error-symbolic"),
    ("Partage de fichiers (Samba)", "partage fichiers dossier samba smb windows réseau local groupe de travail", "samba", "folder-remote-symbolic"),
    ("SSH et SFTP", "ssh sftp terminal à distance clé publique serveur", "ssh", "utilities-terminal-symbolic"),
    ("WebDAV", "webdav dav https fichiers téléphone serveur rclone", "webdav", "folder-remote-symbolic"),
    ("Personnalisation", "personnalisation apparence", "personalization", "applications-graphics-symbolic"),
    ("Arrière-plan", "fond d'écran arrière-plan image wallpaper bing wallhaven diaporama", "background", "preferences-desktop-wallpaper-symbolic"),
    ("Couleurs", "couleur accent sombre clair transparence mode", "colors", "preferences-color-symbolic"),
    ("Thèmes", "thème palette coins arrondis css boutons", "themes", "preferences-desktop-appearance-symbolic"),
    ("Barre des tâches", "barre des tâches dock hauteur alignement épingler recherche", "taskbar", "user-desktop-symbolic"),
    ("Fenêtres", "fenêtres ancrage ancrer tiling snap bords coins disposition animation aperçu", "windows", "window-restore-symbolic"),
    ("Écran de connexion", "écran de connexion login greeter session fond", "login", "system-users-symbolic"),
    ("Polices", "police font texte", "fonts", "preferences-desktop-font-symbolic"),
    ("Écran de verrouillage", "verrouillage verrouiller écran de veille animation shader assombrir super+l", "lockscreen", "system-lock-screen-symbolic"),
    ("Applications", "applications installées programmes", "apps", "view-app-grid-symbolic"),
    ("Applications par défaut", "applications par défaut ouvrir avec associations type de fichier navigateur", "default-apps", "view-app-grid-symbolic"),
    ("Bureaux virtuels", "bureaux virtuels espaces de travail vue des tâches super+tab multitâche", "desktops", "window-restore-symbolic"),
    ("Jeux", "jeux mode jeu gaming vrr freesync gsync variable", "gaming", "input-gaming-symbolic"),
    ("Confidentialité et sécurité", "confidentialité sécurité vie privée", "privacy", "security-high-symbolic"),
    ("Pare-feu", "pare-feu firewall ports sécurité nftables connexions bloquées autoriser bloquer", "firewall", "security-high-symbolic"),
    ("Mises à jour", "mise à jour updates logiciels", "updates", "software-update-available-symbolic"),
];

/// Pages de Paramètres correspondant à la recherche, les plus pertinentes d'abord.
pub fn match_settings(query: &str) -> Vec<usize> {
    let q = fold(query.trim());
    if q.chars().count() < 2 {
        return vec![];
    }
    let split = |s: &str| -> Vec<String> {
        s.split(|c: char| c.is_whitespace() || c == '\'' || c == '-').filter(|w| !w.is_empty()).map(str::to_string).collect()
    };
    let words = split(&q);
    let mut scored: Vec<(u8, usize)> = SETTINGS
        .iter()
        .enumerate()
        .filter_map(|(i, (label, keywords, _, _))| {
            let label = fold(label);
            let score = if label.starts_with(&q) {
                0
            } else if label.contains(&q) {
                1
            } else {
                // Chaque mot tapé commence un mot du libellé ou des mots-clés.
                let hay = split(&format!("{label} {}", fold(keywords)));
                if words.iter().all(|w| hay.iter().any(|h| h.starts_with(w.as_str()))) { 2 } else { return None }
            };
            Some((score, i))
        })
        .collect();
    scored.sort();
    scored.into_iter().map(|(_, i)| i).collect()
}

/// Calcul simple : `2+3*4`, `(1,5 + 2) / 3`, `2^10`, `12 × 3`. Un nombre seul n'est pas un calcul.
pub fn eval(input: &str) -> Option<f64> {
    let s = input.trim().trim_start_matches('=').trim_end_matches('=').trim();
    if s.is_empty() || !s.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut toks = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\u{a0}' | '\u{202f}' => i += 1,
            '0'..='9' | '.' | ',' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',') {
                    i += 1;
                }
                let n: String = chars[start..i].iter().map(|&c| if c == ',' { '.' } else { c }).collect();
                toks.push(Tok::Num(n.parse().ok()?));
            }
            '+' | '-' | '−' => {
                toks.push(Tok::Op(if c == '+' { '+' } else { '-' }));
                i += 1;
            }
            '*' | '×' | '·' => {
                toks.push(Tok::Op('*'));
                i += 1;
            }
            '/' | '÷' | ':' => {
                toks.push(Tok::Op('/'));
                i += 1;
            }
            '^' => {
                toks.push(Tok::Op('^'));
                i += 1;
            }
            '(' => {
                toks.push(Tok::Open);
                i += 1;
            }
            ')' => {
                toks.push(Tok::Close);
                i += 1;
            }
            _ => return None,
        }
    }
    // Au moins une opération entre deux valeurs (pas « -5 » ni « 42 »).
    let binary = toks.windows(2).any(|w| matches!(w, [Tok::Num(_) | Tok::Close, Tok::Op(_)]));
    if !binary {
        return None;
    }
    let mut p = Parser { toks, pos: 0 };
    let v = p.expr()?;
    (p.pos == p.toks.len() && v.is_finite()).then_some(v)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    Open,
    Close,
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<Tok> {
        self.toks.get(self.pos).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(Tok::Op(o @ ('+' | '-'))) = self.peek() {
            self.pos += 1;
            let r = self.term()?;
            v = if o == '+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(Tok::Op(o @ ('*' | '/'))) = self.peek() {
            self.pos += 1;
            let r = self.unary()?;
            v = if o == '*' { v * r } else { v / r };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.pos += 1;
                Some(-self.unary()?)
            }
            Some(Tok::Op('+')) => {
                self.pos += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.peek() == Some(Tok::Op('^')) {
            self.pos += 1;
            let e = self.unary()?;
            return Some(base.powf(e));
        }
        Some(base)
    }
    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Tok::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            Tok::Open => {
                self.pos += 1;
                let v = self.expr()?;
                // Parenthèse fermante oubliée en fin de saisie : tolérée (on tape encore).
                if self.peek() == Some(Tok::Close) {
                    self.pos += 1;
                } else if self.peek().is_some() {
                    return None;
                }
                Some(v)
            }
            _ => None,
        }
    }
}

/// Résultat à la française : virgule décimale, espaces fines entre les milliers.
pub fn format_number(v: f64, group: bool) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    if v.abs() >= 1e15 || (v != 0.0 && v.abs() < 1e-9) {
        return format!("{v:e}").replace('.', ",");
    }
    let s = format!("{:.10}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let (sign, digits) = int.strip_prefix('-').map(|d| ("-", d)).unwrap_or(("", int));
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if group && i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('\u{202f}');
        }
        grouped.push(c);
    }
    if frac.is_empty() { format!("{sign}{grouped}") } else { format!("{sign}{grouped},{frac}") }
}

/// Fichiers dont le nom correspond : récents d'abord, puis noms qui commencent par la
/// recherche, puis les moins profonds.
pub fn match_files(recent: &[PathBuf], index: &[PathBuf], query: &str, max: usize) -> Vec<(PathBuf, bool)> {
    let q = fold(query.trim());
    if q.chars().count() < 2 {
        return vec![];
    }
    let name_of = |p: &Path| p.file_name().map(|n| fold(&n.to_string_lossy())).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let mut scored: Vec<(u8, usize, usize, PathBuf, bool)> = Vec::new();
    for (list, is_recent) in [(recent, true), (index, false)] {
        for p in list {
            let name = name_of(p);
            let score = if name.starts_with(&q) {
                0
            } else if name.contains(&q) {
                1
            } else {
                continue;
            };
            if !seen.insert(p.clone()) {
                continue;
            }
            let score = if is_recent { score } else { score + 2 };
            scored.push((score, p.components().count(), name.len(), p.clone(), is_recent));
        }
    }
    scored.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    scored.into_iter().take(max).map(|(_, _, _, p, r)| (p, r)).collect()
}

/// Dossiers ignorés pendant l'indexation du dossier personnel.
const SKIP_DIRS: &[&str] = &["node_modules", "target", "__pycache__", "venv", "snap", "go", "Applications"];

/// Parcourt le dossier personnel (sans les dossiers cachés), en largeur, jusqu'à `max_depth`.
pub fn scan_home(home: &Path, max_depth: usize, max_entries: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(home.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let path = e.path();
            if ft.is_dir() {
                if depth + 1 < max_depth && !SKIP_DIRS.contains(&name.as_ref()) {
                    queue.push_back((path.clone(), depth + 1));
                }
            } else if !ft.is_file() {
                continue;
            }
            out.push(path);
            if out.len() >= max_entries {
                return out;
            }
        }
    }
    out
}

/// Index des fichiers du dossier personnel, reconstruit en arrière-plan (au plus toutes les 2 min).
struct FileIndex {
    paths: Mutex<Arc<Vec<PathBuf>>>,
    built: Mutex<Option<Instant>>,
    building: AtomicBool,
}

fn file_index() -> &'static FileIndex {
    static INDEX: OnceLock<FileIndex> = OnceLock::new();
    INDEX.get_or_init(|| FileIndex { paths: Mutex::new(Arc::new(vec![])), built: Mutex::new(None), building: AtomicBool::new(false) })
}

/// Lance la reconstruction de l'index s'il est trop ancien. Vrai si elle est en cours.
fn refresh_index() -> bool {
    let idx = file_index();
    let fresh = idx.built.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_secs(120));
    if fresh {
        return false;
    }
    if idx.building.swap(true, Ordering::SeqCst) {
        return true;
    }
    std::thread::spawn(|| {
        let home = glib::home_dir();
        let paths = scan_home(&home, 5, 60_000);
        let idx = file_index();
        *idx.paths.lock().unwrap() = Arc::new(paths);
        *idx.built.lock().unwrap() = Some(Instant::now());
        idx.building.store(false, Ordering::SeqCst);
    });
    true
}

fn indexing() -> bool {
    file_index().building.load(Ordering::SeqCst)
}

/// Fichiers récents (GtkRecentManager), existants seulement.
fn recent_files() -> Vec<PathBuf> {
    let mut items = gtk::RecentManager::default().items();
    items.sort_by_key(|i| std::cmp::Reverse(i.modified().to_unix()));
    items
        .into_iter()
        .filter_map(|i| gio::File::for_uri(&i.uri()).path())
        .filter(|p| p.exists())
        .take(200)
        .collect()
}

// ───────────────────────────── Résultats ─────────────────────────────

#[derive(Clone)]
pub enum Hit {
    Calc { expr: String, value: f64 },
    App(gio::AppInfo),
    Setting(usize),
    File { path: PathBuf, recent: bool },
}

impl Hit {
    fn group(&self) -> &'static str {
        match self {
            Hit::Calc { .. } => "Calcul",
            Hit::App(_) => "Applications",
            Hit::Setting(_) => "Paramètres",
            Hit::File { recent: true, .. } => "Récents",
            Hit::File { .. } => "Fichiers et dossiers",
        }
    }
}

/// Tous les résultats d'une recherche, groupés (calcul, applications, paramètres, fichiers).
/// Texte vide : fichiers récents.
pub fn query(text: &str) -> Vec<Hit> {
    let recent = recent_files();
    if text.trim().is_empty() {
        return recent.into_iter().take(MAX_FILES).map(|path| Hit::File { path, recent: true }).collect();
    }
    let mut hits = Vec::new();
    if let Some(value) = eval(text) {
        hits.push(Hit::Calc { expr: text.trim().to_string(), value });
    }
    hits.extend(apps::search(text).into_iter().take(MAX_APPS).map(Hit::App));
    hits.extend(match_settings(text).into_iter().take(MAX_SETTINGS).map(Hit::Setting));
    let index = file_index().paths.lock().unwrap().clone();
    // Dans les fichiers, les récents trouvés gardent leur étiquette mais vont dans un seul groupe.
    hits.extend(match_files(&recent, &index, text, MAX_FILES).into_iter().map(|(path, _)| Hit::File { path, recent: false }));
    hits
}

/// Ouvre un résultat. Vrai si le focus doit revenir à la fenêtre d'avant (rien n'a été lancé).
pub fn open(hit: &Hit) -> bool {
    match hit {
        Hit::App(app) => apps::launch_info(app),
        Hit::Setting(i) => apps::spawn(&format!("noos-settings {}", SETTINGS[*i].2)),
        Hit::File { path, .. } => {
            let uri = gio::File::for_path(path).uri();
            let ctx = gdk::Display::default().map(|d| d.app_launch_context());
            if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, ctx.as_ref()) {
                eprintln!("noos-shell : ouverture de {} impossible : {e}", path.display());
            }
        }
        Hit::Calc { value, .. } => {
            if let Some(d) = gdk::Display::default() {
                d.clipboard().set_text(&format_number(*value, false));
            }
            return true;
        }
    }
    false
}

fn home_relative(p: &Path) -> String {
    let home = glib::home_dir();
    match p.strip_prefix(&home) {
        Ok(r) if r.as_os_str().is_empty() => "Dossier personnel".into(),
        Ok(r) => format!("~/{}", r.display()),
        Err(_) => p.display().to_string(),
    }
}

fn hit_row(hit: &Hit) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("hit");
    let h = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let (icon, title, subtitle): (gtk::Image, String, String) = match hit {
        Hit::Calc { expr, value } => (
            gtk::Image::from_icon_name("accessories-calculator-symbolic"),
            format!("= {}", format_number(*value, true)),
            format!("{expr} · Entrée pour copier le résultat"),
        ),
        Hit::App(app) => (
            match app.icon() {
                Some(i) => gtk::Image::from_gicon(&i),
                None => gtk::Image::from_icon_name("application-x-executable"),
            },
            app.display_name().to_string(),
            app.description().map(|d| d.to_string()).unwrap_or_else(|| "Application".into()),
        ),
        Hit::Setting(i) => {
            let (label, _, _, icon) = SETTINGS[*i];
            (gtk::Image::from_icon_name(icon), label.to_string(), "Paramètres".into())
        }
        Hit::File { path, .. } => {
            let icon = if path.is_dir() {
                gio::ThemedIcon::new("folder").upcast::<gio::Icon>()
            } else {
                let (ct, _) = gio::content_type_guess(Some(path), None);
                gio::content_type_get_icon(&ct)
            };
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let parent = path.parent().map(home_relative).unwrap_or_default();
            (gtk::Image::from_gicon(&icon), name, parent)
        }
    };
    icon.set_pixel_size(if matches!(hit, Hit::App(_)) { 32 } else { 24 });
    icon.set_size_request(32, 32);
    h.append(&icon);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 1);
    v.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(&title));
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    if matches!(hit, Hit::Calc { .. }) {
        t.add_css_class("hit-calc");
    }
    v.append(&t);
    let s = gtk::Label::new(Some(&subtitle));
    s.set_xalign(0.0);
    s.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    s.add_css_class("hit-subtitle");
    v.append(&s);
    h.append(&v);
    row.set_child(Some(&h));
    unsafe { row.set_data("noos-group", hit.group()) };
    row
}

fn row_group(row: &gtk::ListBoxRow) -> Option<&'static str> {
    unsafe { row.data::<&'static str>("noos-group").map(|p| *p.as_ref()) }
}

/// Liste de résultats groupés, avec sélection au clavier (le panneau n'a pas le clavier : la
/// sélection est pilotée par le champ de saisie).
pub struct Results {
    pub scroll: gtk::ScrolledWindow,
    list: gtk::ListBox,
    placeholder: gtk::Label,
    hits: RefCell<Vec<Hit>>,
    /// Après l'ouverture d'un résultat (argument : rendre le focus à la fenêtre d'avant).
    on_opened: Box<dyn Fn(bool)>,
}

impl Results {
    pub fn new(on_opened: impl Fn(bool) + 'static) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.add_css_class("search-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(true);
        list.set_header_func(|row, before| {
            let group = row_group(row);
            if before.and_then(row_group) == group {
                row.set_header(None::<&gtk::Widget>);
                return;
            }
            let l = gtk::Label::new(group);
            l.set_xalign(0.0);
            l.add_css_class("search-group");
            row.set_header(Some(&l));
        });
        let placeholder = gtk::Label::new(None);
        placeholder.add_css_class("dim-label");
        placeholder.set_wrap(true);
        placeholder.set_justify(gtk::Justification::Center);
        placeholder.set_margin_top(48);
        placeholder.set_margin_start(24);
        placeholder.set_margin_end(24);
        list.set_placeholder(Some(&placeholder));
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let r = Rc::new(Self { scroll, list, placeholder, hits: RefCell::new(vec![]), on_opened: Box::new(on_opened) });
        {
            let r2 = Rc::downgrade(&r);
            r.list.connect_row_activated(move |_, row| {
                if let Some(r) = r2.upgrade() {
                    r.open_index(row.index());
                }
            });
        }
        r
    }

    pub fn set(&self, text: &str, hits: Vec<Hit>) {
        self.list.remove_all();
        for h in &hits {
            self.list.append(&hit_row(h));
        }
        self.placeholder.set_text(&if text.trim().is_empty() {
            "Tapez pour rechercher des applications, des paramètres et des fichiers, ou pour faire un calcul.".to_string()
        } else if indexing() {
            format!("Recherche de « {} »…", text.trim())
        } else {
            format!("Aucun résultat pour « {} »", text.trim())
        });
        *self.hits.borrow_mut() = hits;
        if let Some(first) = self.list.row_at_index(0) {
            self.list.select_row(Some(&first));
        }
        self.scroll.vadjustment().set_value(0.0);
    }

    /// Flèche haut / bas.
    pub fn move_selection(&self, delta: i32) {
        let n = self.hits.borrow().len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.list.selected_row().map(|r| r.index()).unwrap_or(-1);
        let next = (cur + delta).clamp(0, n - 1);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
            // Garder la ligne visible (le panneau n'a pas le focus : pas de défilement auto).
            if let Some(b) = row.compute_bounds(&self.list) {
                let adj = self.scroll.vadjustment();
                // L'en-tête de groupe fait partie de la ligne vue d'en haut.
                let top = if next == 0 { 0.0 } else { b.y() as f64 - 28.0 };
                let bottom = (b.y() + b.height()) as f64;
                if top < adj.value() {
                    adj.set_value(top);
                } else if bottom > adj.value() + adj.page_size() {
                    adj.set_value(bottom - adj.page_size());
                }
            }
        }
    }

    /// Entrée : ouvre la ligne sélectionnée (ou la première).
    pub fn open_selected(&self) -> bool {
        let i = self.list.selected_row().map(|r| r.index()).unwrap_or(0);
        self.open_index(i)
    }

    fn open_index(&self, i: i32) -> bool {
        let hit = self.hits.borrow().get(i.max(0) as usize).cloned();
        let Some(hit) = hit else { return false };
        let restore = open(&hit);
        (self.on_opened)(restore);
        true
    }
}

// ───────────────────────────── Champ de la barre et panneau ─────────────────────────────

/// Pastille « Rechercher » d'une barre.
struct Pill {
    bar: glib::WeakRef<gtk::Window>,
    pill: glib::WeakRef<gtk::Box>,
    entry: glib::WeakRef<gtk::Text>,
    monitor: gdk::Monitor,
    top: bool,
}

thread_local! {
    static PILLS: RefCell<Vec<Pill>> = const { RefCell::new(Vec::new()) };
}

/// Recherche en cours : barre et champ qui ont le clavier.
struct Active {
    bar: glib::WeakRef<gtk::Window>,
    entry: glib::WeakRef<gtk::Text>,
    /// Fenêtre active avant la recherche (on lui rend le clavier après Échap).
    previous: Option<WindowId>,
}

pub struct SearchPanel {
    shell: std::rc::Weak<Shell>,
    window: gtk::Window,
    results: Rc<Results>,
    active: RefCell<Option<Active>>,
    /// Recherche ouverte sans clic (variable de démo) : pas de clavier, ne pas la fermer.
    demo: Cell<bool>,
    /// Numéro de recherche (les mises à jour différées d'une recherche finie sont ignorées).
    generation: Cell<u64>,
}

impl SearchPanel {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let window = gtk::Window::builder().application(&shell.app).decorated(false).build();
        window.add_css_class("noos-layer");
        window.init_layer_shell();
        window.set_namespace(Some("noos-search"));
        window.set_layer(Layer::Top);
        // Le clavier reste dans la barre : le panneau n'en veut pas.
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_default_size(PANEL_W, PANEL_H);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("noos-flyout");
        root.add_css_class("search-flyout");
        root.set_size_request(PANEL_W, PANEL_H);
        let panel: Rc<RefCell<std::rc::Weak<SearchPanel>>> = Rc::new(RefCell::new(std::rc::Weak::new()));
        let results = {
            let panel = panel.clone();
            Results::new(move |restore| {
                if let Some(p) = panel.borrow().upgrade() {
                    p.end(restore);
                }
            })
        };
        root.append(&results.scroll);
        let hint = gtk::Label::new(Some("↑ ↓ pour choisir · Entrée pour ouvrir · Échap pour fermer"));
        hint.add_css_class("search-hint");
        hint.set_xalign(0.0);
        root.append(&hint);
        window.set_child(Some(&root));

        let p = Rc::new(Self {
            shell: Rc::downgrade(shell),
            window,
            results,
            active: RefCell::new(None),
            demo: Cell::new(false),
            generation: Cell::new(0),
        });
        *panel.borrow_mut() = Rc::downgrade(&p);
        p
    }

    fn is_on(&self, entry: &gtk::Text) -> bool {
        self.active.borrow().as_ref().and_then(|a| a.entry.upgrade()).is_some_and(|e| &e == entry)
    }

    /// Ouvre la recherche dans le champ d'une barre (clic dans la pastille).
    fn begin(self: &Rc<Self>, bar: &gtk::Window, pill: &gtk::Box, entry: &gtk::Text, monitor: &gdk::Monitor, top: bool, demo: bool) {
        if self.is_on(entry) {
            return;
        }
        self.end(false);
        let Some(shell) = self.shell.upgrade() else { return };
        // Les autres panneaux se ferment (ils perdent le clavier, sauf avec la démo).
        if let Some(s) = shell.start.borrow().as_ref() {
            s.hide();
        }
        let previous = shell.windows.borrow().iter().find(|w| w.focused).map(|w| w.id);
        *self.active.borrow_mut() = Some(Active { bar: bar.downgrade(), entry: entry.downgrade(), previous });
        self.demo.set(demo);
        self.generation.set(self.generation.get() + 1);
        bar.set_keyboard_mode(KeyboardMode::OnDemand);
        pill.add_css_class("searching");
        entry.grab_focus();
        self.place(bar, pill, monitor, top);
        if refresh_index() {
            self.refresh_when_indexed();
        }
        self.update();
        self.window.present();
        if !demo {
            // Compositeur qui ne donne pas le clavier à la barre : on se rabat sur le menu
            // Démarrer (qui le prend lui-même).
            let (p, bar, numero) = (Rc::downgrade(self), bar.downgrade(), self.generation.get());
            glib::timeout_add_local_once(Duration::from_millis(600), move || {
                let (Some(p), Some(bar)) = (p.upgrade(), bar.upgrade()) else { return };
                if p.generation.get() == numero && p.active.borrow().is_some() && !bar.is_active() {
                    eprintln!("noos-shell : la barre n'a pas reçu le clavier, recherche dans le menu Démarrer");
                    p.end(false);
                    if let Some(s) = p.shell.upgrade().and_then(|s| s.start.borrow().clone()) {
                        s.show(Some(""));
                    }
                }
            });
        }
    }

    /// Place le panneau contre la barre, centré sur la pastille.
    fn place(&self, bar: &gtk::Window, pill: &gtk::Box, monitor: &gdk::Monitor, top: bool) {
        let w = &self.window;
        w.set_monitor(Some(monitor));
        let cfg = self.shell.upgrade().map(|s| s.config()).unwrap_or_default();
        let at_top = crate::taskbar::bar_edge(&cfg, top) == "top";
        let mon_w = monitor.geometry().width();
        let center = pill.compute_bounds(bar).map(|b| (b.x() + b.width() / 2.0) as i32).unwrap_or(mon_w / 2);
        let left = (center - PANEL_W / 2).clamp(12, (mon_w - PANEL_W - 12).max(12));
        w.set_anchor(Edge::Top, at_top);
        w.set_anchor(Edge::Bottom, !at_top);
        w.set_anchor(Edge::Left, true);
        w.set_anchor(Edge::Right, false);
        w.set_margin(Edge::Left, left);
        w.set_margin(Edge::Top, if at_top { 8 } else { 0 });
        w.set_margin(Edge::Bottom, if at_top { 0 } else { 8 });
    }

    /// Texte du champ actif → résultats.
    fn update(&self) {
        let text = self.active.borrow().as_ref().and_then(|a| a.entry.upgrade()).map(|e| e.text().to_string());
        if let Some(text) = text {
            self.results.set(&text, query(&text));
        }
    }

    /// L'index des fichiers est en cours de construction : résultats rafraîchis à la fin.
    fn refresh_when_indexed(self: &Rc<Self>) {
        let (p, numero) = (Rc::downgrade(self), self.generation.get());
        glib::timeout_add_local(Duration::from_millis(150), move || {
            let Some(p) = p.upgrade() else { return glib::ControlFlow::Break };
            if p.generation.get() != numero || p.active.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            if indexing() {
                return glib::ControlFlow::Continue;
            }
            p.update();
            glib::ControlFlow::Break
        });
    }

    /// Ferme la recherche : champ vidé, clavier rendu, panneau caché.
    pub fn end(&self, restore_focus: bool) {
        let Some(active) = self.active.borrow_mut().take() else { return };
        self.generation.set(self.generation.get() + 1);
        self.demo.set(false);
        self.window.set_visible(false);
        // Pointeur encore sur la pastille : la barre garde `OnDemand` (un nouveau clic doit
        // pouvoir rouvrir la recherche) ; elle repassera en `None` quand il sortira.
        let mut hovered = false;
        if let Some(entry) = active.entry.upgrade() {
            entry.set_text("");
            if let Some(pill) = entry.parent() {
                pill.remove_css_class("searching");
                hovered = pill.state_flags().contains(gtk::StateFlags::PRELIGHT);
            }
        }
        if let Some(bar) = active.bar.upgrade() {
            GtkWindowExt::set_focus(&bar, None::<&gtk::Widget>);
            if !hovered {
                bar.set_keyboard_mode(KeyboardMode::None);
            }
        }
        if restore_focus {
            if let Some(id) = active.previous {
                apps::request(Request::Activate { id });
            }
        }
    }

    fn key(&self, key: gdk::Key) -> bool {
        match key {
            gdk::Key::Escape => self.end(true),
            gdk::Key::Down | gdk::Key::KP_Down => self.results.move_selection(1),
            gdk::Key::Up | gdk::Key::KP_Up => self.results.move_selection(-1),
            gdk::Key::Page_Down => self.results.move_selection(5),
            gdk::Key::Page_Up => self.results.move_selection(-5),
            _ => return false,
        }
        true
    }
}

/// Construit la pastille « Rechercher » d'une barre : un vrai champ de saisie.
/// Renvoie la pastille et le champ (caché sur un dock vertical : la pastille ouvre alors la
/// recherche du menu Démarrer).
pub fn pill(shell: &Rc<Shell>, bar: &gtk::Window, monitor: &gdk::Monitor, top: bool) -> (gtk::Box, gtk::Text) {
    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    pill.add_css_class("search-pill");
    pill.set_valign(gtk::Align::Center);
    pill.set_halign(gtk::Align::Center);
    pill.set_tooltip_text(Some("Rechercher des applications, des paramètres et des fichiers"));
    let icon = gtk::Image::from_icon_name("system-search-symbolic");
    pill.append(&icon);
    let entry = gtk::Text::new();
    entry.set_placeholder_text(Some("Rechercher"));
    entry.set_hexpand(false);
    entry.set_width_chars(12);
    entry.add_css_class("search-text");
    pill.append(&entry);

    let weak = Rc::downgrade(shell);
    let panel = move || weak.upgrade().and_then(|s| s.search.borrow().clone());

    // Pointeur au-dessus : la barre accepte le clavier au prochain clic.
    {
        let motion = gtk::EventControllerMotion::new();
        let (bar_w, e) = (bar.downgrade(), entry.downgrade());
        motion.connect_enter(move |_, _, _| {
            // Dock vertical (champ caché) : le menu Démarrer prendra le clavier lui-même.
            if let (Some(bar), Some(e)) = (bar_w.upgrade(), e.upgrade()) {
                if e.is_visible() {
                    bar.set_keyboard_mode(KeyboardMode::OnDemand);
                }
            }
        });
        let (p, bar_w, e) = (panel.clone(), bar.downgrade(), entry.downgrade());
        motion.connect_leave(move |_| {
            let (Some(bar), Some(e)) = (bar_w.upgrade(), e.upgrade()) else { return };
            if !p().is_some_and(|p| p.is_on(&e)) {
                bar.set_keyboard_mode(KeyboardMode::None);
            }
        });
        pill.add_controller(motion);
    }
    // Clic : la recherche commence (ou, sur un dock vertical, celle du menu Démarrer).
    {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_PRIMARY);
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (p, bar_w, pill_w, e, mon, sh) = (panel.clone(), bar.downgrade(), pill.downgrade(), entry.downgrade(), monitor.clone(), Rc::downgrade(shell));
        click.connect_pressed(move |_, _, _, _| {
            let (Some(p), Some(bar), Some(pill), Some(e)) = (p(), bar_w.upgrade(), pill_w.upgrade(), e.upgrade()) else { return };
            if e.is_visible() {
                p.begin(&bar, &pill, &e, &mon, top, false);
            } else if let Some(s) = sh.upgrade().and_then(|s| s.start.borrow().clone()) {
                s.toggle(Some(""));
            }
        });
        pill.add_controller(click);
    }
    {
        let p = panel.clone();
        entry.connect_changed(move |e| {
            if let Some(p) = p().filter(|p| p.is_on(e)) {
                p.update();
            }
        });
    }
    {
        let p = panel.clone();
        entry.connect_activate(move |e| {
            if let Some(p) = p().filter(|p| p.is_on(e)) {
                p.results.open_selected();
            }
        });
    }
    {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (p, e) = (panel.clone(), entry.downgrade());
        keys.connect_key_pressed(move |_, key, _, _| {
            let (Some(p), Some(e)) = (p(), e.upgrade()) else { return glib::Propagation::Proceed };
            if p.is_on(&e) && p.key(key) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        entry.add_controller(keys);
    }
    {
        // La barre perd le clavier (clic dans une fenêtre, menu Démarrer…) : recherche finie.
        let (p, e) = (panel.clone(), entry.downgrade());
        bar.connect_is_active_notify(move |b| {
            let (Some(p), Some(e)) = (p(), e.upgrade()) else { return };
            if !b.is_active() && p.is_on(&e) && !p.demo.get() {
                p.end(false);
            }
        });
    }
    PILLS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|p| p.bar.upgrade().is_some());
        v.push(Pill { bar: bar.downgrade(), pill: pill.downgrade(), entry: entry.downgrade(), monitor: monitor.clone(), top });
    });
    (pill, entry)
}

/// `NOOS_SEARCH_DEMO=texte[;down…]` : saisie simulée dans la pastille du dock (une lettre
/// toutes les 80 ms), puis flèches ; `start:texte` : dans le menu Démarrer.
pub fn demo(shell: &Rc<Shell>, spec: &str) {
    let mut parts = spec.split(';');
    let text = parts.next().unwrap_or("").to_string();
    let steps: Vec<String> = parts.map(str::to_string).collect();
    if let Some(t) = text.strip_prefix("start:") {
        if let Some(s) = shell.start.borrow().as_ref() {
            s.show(Some(t));
            for st in &steps {
                if st == "down" {
                    s.move_selection(1);
                }
            }
        }
        return;
    }
    let Some(panel) = shell.search.borrow().clone() else { return };
    let target = PILLS.with(|v| {
        let v = v.borrow();
        let found = v.iter().filter(|p| p.entry.upgrade().is_some_and(|e| e.is_visible() && e.is_mapped()));
        let mut found: Vec<&Pill> = found.collect();
        found.sort_by_key(|p| p.top);
        found.first().and_then(|p| Some((p.bar.upgrade()?, p.pill.upgrade()?, p.entry.upgrade()?, p.monitor.clone(), p.top)))
    });
    let Some((bar, pill, entry, monitor, top)) = target else {
        eprintln!("noos-shell : NOOS_SEARCH_DEMO : aucune pastille de recherche visible");
        return;
    };
    panel.begin(&bar, &pill, &entry, &monitor, top, true);
    let chars: Vec<char> = text.chars().collect();
    let i = Rc::new(Cell::new(0usize));
    glib::timeout_add_local(Duration::from_millis(80), move || {
        let n = i.get();
        if n < chars.len() {
            let pos = entry.text().chars().count() as i32;
            let mut p = pos;
            entry.insert_text(&chars[n].to_string(), &mut p);
            entry.set_position(p);
            i.set(n + 1);
            return glib::ControlFlow::Continue;
        }
        for st in &steps {
            match st.as_str() {
                "down" => panel.results.move_selection(1),
                "up" => panel.results.move_selection(-1),
                _ => {}
            }
        }
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plie_les_accents() {
        assert_eq!(fold("Paramètres Écran Œuvre"), "parametres ecran oeuvre");
    }

    #[test]
    fn calcule() {
        assert_eq!(eval("2+3*4"), Some(14.0));
        assert_eq!(eval("(1,5 + 2) / 7"), Some(0.5));
        assert_eq!(eval("2^10"), Some(1024.0));
        assert_eq!(eval("-2^2"), Some(-4.0));
        assert_eq!(eval("12 × 3"), Some(36.0));
        assert_eq!(eval("= 10 ÷ 4"), Some(2.5));
        assert_eq!(eval("(2+3"), Some(5.0));
        assert_eq!(eval("3*(4-1)"), Some(9.0));
    }

    #[test]
    fn ne_calcule_pas() {
        for s in ["42", "-5", "firefox", "1/0", "2+", "gimp 2", "", "3x4", "2++"] {
            assert_eq!(eval(s), None, "{s}");
        }
    }

    #[test]
    fn formate_a_la_francaise() {
        assert_eq!(format_number(1234567.0, true), "1\u{202f}234\u{202f}567");
        assert_eq!(format_number(1234567.0, false), "1234567");
        assert_eq!(format_number(0.5, true), "0,5");
        assert_eq!(format_number(-2.25, true), "-2,25");
        assert_eq!(format_number(1.0 / 3.0, true), "0,3333333333");
        assert_eq!(format_number(-0.0, true), "0");
    }

    #[test]
    fn trouve_les_parametres() {
        let label = |q: &str| match_settings(q).first().map(|&i| SETTINGS[i].0);
        assert_eq!(label("wifi"), Some("Wi-Fi"));
        assert_eq!(label("ecran"), Some("Écran"));
        assert_eq!(label("fond d'écran"), Some("Arrière-plan"));
        assert_eq!(label("pare"), Some("Pare-feu"));
        assert_eq!(label("batterie"), Some("Alimentation et batterie"));
        assert!(match_settings("x").is_empty());
        assert!(match_settings("zzzz").is_empty());
    }

    #[test]
    fn trouve_les_fichiers() {
        let recent = vec![PathBuf::from("/h/Documents/Rapport final.odt")];
        let index = vec![
            PathBuf::from("/h/Documents/Rapport final.odt"),
            PathBuf::from("/h/Images/vacances/rapport.png"),
            PathBuf::from("/h/rapports"),
            PathBuf::from("/h/notes.txt"),
        ];
        let r = match_files(&recent, &index, "rapport", 10);
        let names: Vec<String> = r.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["Rapport final.odt", "rapports", "rapport.png"]);
        assert!(r[0].1);
        assert!(match_files(&recent, &index, "r", 10).is_empty());
    }

    #[test]
    fn parcourt_le_dossier_personnel() {
        let dir = std::env::temp_dir().join(format!("noos-search-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Documents/a/b")).unwrap();
        std::fs::create_dir_all(dir.join(".cache")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("Documents/a/b/profond.txt"), "").unwrap();
        std::fs::write(dir.join("Documents/note.txt"), "").unwrap();
        std::fs::write(dir.join(".cache/cache.txt"), "").unwrap();
        let all = scan_home(&dir, 5, 100);
        let has = |n: &str| all.iter().any(|p| p.file_name().is_some_and(|f| f == n));
        assert!(has("note.txt") && has("profond.txt") && has("Documents") && has("node_modules"));
        assert!(!has("cache.txt") && !has(".cache"));
        let shallow = scan_home(&dir, 2, 100);
        assert!(!shallow.iter().any(|p| p.ends_with("profond.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

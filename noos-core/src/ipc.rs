//! Protocole IPC NOOS : une ligne JSON par message sur un socket Unix.
//!
//! Les clients (shell, réglages) envoient des [`Request`] ; le compositeur répond par des
//! [`Event`]. Un client qui envoie [`Request::Subscribe`] reçoit ensuite en continu les
//! changements (liste des fenêtres, écrans, ouverture du menu Démarrer…).
//!
//! On garde volontairement un protocole « maison » plutôt que wlr-foreign-toplevel :
//! il est trivial à étendre (gaming, HDR…) et lisible avec `socat` pour déboguer :
//! `echo '{"type":"get-outputs"}' | socat - UNIX-CONNECT:$NOOS_IPC_SOCKET`

use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::paths;

pub type WindowId = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Request {
    /// Recevoir les événements en continu.
    Subscribe,
    GetWindows,
    GetOutputs,
    Activate { id: WindowId },
    Close { id: WindowId },
    Minimize { id: WindowId },
    ToggleMaximize { id: WindowId },
    /// Ancre une fenêtre : maximize, left, right, top-left, top-right, bottom-left,
    /// bottom-right, left-third, center-third, right-third, left-two-thirds, right-two-thirds,
    /// restore.
    Snap { id: WindowId, zone: String },
    /// Minimise / restaure toutes les fenêtres (« Afficher le bureau »).
    ShowDesktop,
    /// Relire `noos.toml` (envoyé par noos-settings après sauvegarde).
    ReloadConfig,
    /// Lancer une commande avec l'environnement de la session.
    Spawn { command: String },
    /// Change la résolution / fréquence (mHz) d'un écran immédiatement.
    SetMode { output: String, width: i32, height: i32, refresh: i32 },
    /// Change l'échelle d'un écran immédiatement.
    SetScale { output: String, scale: f64 },
    /// Change l'orientation (`normal`, `90`, `180`, `270`).
    SetTransform { output: String, transform: String },
    /// Ouvre / ferme le menu Démarrer (équivalent de la touche Super).
    ToggleStartMenu,
    /// Ouvre / ferme la vue des tâches (Super+Tab).
    ToggleTaskView,
    /// Mode édition du bureau (disposition de la barre, widgets).
    ToggleEditMode,
    // ── Bureaux virtuels ──
    /// État des bureaux (réponse : `Desktops`).
    GetDesktops,
    /// Aller au bureau `index` (0 = premier).
    SwitchDesktop { index: usize },
    /// Bureau suivant (+1) / précédent (-1).
    SwitchDesktopRelative { delta: i32 },
    /// Nouveau bureau (nom par défaut « Bureau n »), devient le bureau actif si `switch`.
    AddDesktop { name: Option<String>, switch: bool },
    /// Supprime un bureau : ses fenêtres rejoignent le bureau voisin.
    RemoveDesktop { index: usize },
    RenameDesktop { index: usize, name: String },
    /// Déplace un bureau dans la liste (réordonner).
    MoveDesktop { from: usize, to: usize },
    /// Envoie une fenêtre sur un bureau (`follow` : on y va aussi).
    MoveWindowToDesktop { id: WindowId, index: usize, follow: bool },
    /// Fenêtre visible sur tous les bureaux.
    SetSticky { id: WindowId, sticky: bool },
    // ── Écrans multiples ──
    /// Envoie une fenêtre sur un autre écran (nom du connecteur).
    MoveWindowToOutput { id: WindowId, output: String },
    /// Capture de l'écran (premier écran) au format PNG.
    Screenshot { path: String },
    /// Verrouiller la session (lance `noos-lock`).
    Lock,
    /// Mise en veille (précédée du verrouillage si `[lock] before_suspend`).
    Suspend,
    /// Quitter la session.
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Windows { windows: Vec<WindowInfo> },
    Outputs { outputs: Vec<OutputInfo> },
    /// La touche Super a été pressée seule : ouvrir/fermer le menu Démarrer.
    ToggleStartMenu,
    /// Super+Tab : vue des tâches.
    ToggleTaskView,
    /// Mode édition du bureau.
    ToggleEditMode,
    /// Bureaux virtuels : noms et bureau actif (diffusé à chaque changement).
    Desktops { names: Vec<String>, current: usize },
    ConfigReloaded,
    Error { message: String },
    Ok,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WindowInfo {
    pub id: WindowId,
    pub title: String,
    pub app_id: String,
    pub focused: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    /// Bureau virtuel de la fenêtre (index).
    #[serde(default)]
    pub desktop: usize,
    /// Visible sur tous les bureaux.
    #[serde(default)]
    pub sticky: bool,
    /// Écran où se trouve la fenêtre (nom du connecteur, vide si inconnue).
    #[serde(default)]
    pub output: String,
    /// Position et taille (coordonnées logiques globales : x, y, largeur, hauteur).
    /// Renseignée seulement en réponse à `GetWindows` (pas dans les diffusions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<[i32; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct OutputInfo {
    pub name: String,
    pub make: String,
    pub model: String,
    pub width: i32,
    pub height: i32,
    /// mHz
    pub refresh: i32,
    pub scale: f64,
    pub modes: Vec<ModeInfo>,
    pub vrr_capable: bool,
    pub vrr_active: bool,
    /// L'EDID annonce le HDR (EOTF PQ) ET le pilote expose `HDR_OUTPUT_METADATA`.
    pub hdr_capable: bool,
    pub hdr_active: bool,
    /// Orientation actuelle (`normal`, `90`, `180`, `270`).
    #[serde(default)]
    pub transform: String,
    /// Le mode peut être changé à chaud.
    #[serde(default)]
    pub can_set_mode: bool,
    /// Format du framebuffer actuel (ex. `XR30` pour du 10 bits).
    pub format: String,
    /// Le compositeur tourne en fenêtre (mode développement imbriqué).
    pub nested: bool,
    /// Position dans l'espace des écrans (logique, coin supérieur gauche).
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    /// Taille logique (après échelle et rotation).
    #[serde(default)]
    pub logical_width: i32,
    #[serde(default)]
    pub logical_height: i32,
    /// Écran principal (barre des tâches complète, nouvelles fenêtres).
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModeInfo {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
    pub preferred: bool,
}

impl ModeInfo {
    pub fn label(&self) -> String {
        format!("{}×{} @ {:.2} Hz", self.width, self.height, self.refresh as f64 / 1000.0)
    }
}

/// Client IPC bloquant (à utiliser dans un thread côté GTK).
pub struct IpcClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl IpcClient {
    pub fn connect() -> std::io::Result<Self> {
        let stream = UnixStream::connect(paths::ipc_socket())?;
        let writer = stream.try_clone()?;
        Ok(Self { reader: BufReader::new(stream), writer })
    }

    pub fn send(&mut self, req: &Request) -> std::io::Result<()> {
        let mut line = serde_json::to_string(req).map_err(std::io::Error::other)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())
    }

    /// Lit le prochain événement (bloquant). `Ok(None)` = connexion fermée.
    pub fn recv(&mut self) -> std::io::Result<Option<Event>> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            if line.trim().is_empty() {
                continue;
            }
            return serde_json::from_str(&line).map(Some).map_err(std::io::Error::other);
        }
    }

    /// Envoie une requête et attend la réponse (avec délai d'attente).
    pub fn request(req: &Request) -> std::io::Result<Event> {
        let mut client = Self::connect()?;
        client.reader.get_ref().set_read_timeout(Some(Duration::from_secs(2)))?;
        client.send(req)?;
        client
            .recv()?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "connexion fermée"))
    }

    /// Envoie une requête sans attendre de réponse détaillée.
    pub fn fire(req: &Request) -> std::io::Result<()> {
        Self::request(req).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape() {
        let s = serde_json::to_string(&Request::Activate { id: 3 }).unwrap();
        assert_eq!(s, r#"{"type":"activate","id":3}"#);
        let e: Event = serde_json::from_str(r#"{"type":"toggle-start-menu"}"#).unwrap();
        assert_eq!(e, Event::ToggleStartMenu);
    }
}

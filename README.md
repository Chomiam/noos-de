# NOOS — prototype d'environnement de bureau Wayland en Rust

Disposition inspirée de Windows 11, fonctions gaming (VRR, scanout direct, 10 bits, détection HDR)
et, surtout, **un thème entièrement personnalisable** grâce à **GTK4 + libAdapta**.

> libAdapta est le fork « doux » de libadwaita maintenu par Linux Mint : même API et même rendu
> par défaut, mais il charge les thèmes. NOOS génère un vrai thème libAdapta, il n'y a donc
> aucun verrou sur l'apparence.

## Organisation du dépôt

| Dossier | Rôle |
|---|---|
| `noos-comp/` | **Compositeur** Wayland (Smithay) : DRM/KMS + libinput (session réelle) ou fenêtre imbriquée (winit), gestion des fenêtres façon Windows 11, VRR, IPC |
| `noos-shell/` | Barre des tâches centrée, menu Démarrer, réglages rapides, centre de notifications, zone des applis actives, fond d'écran (layer-shell) |
| `noos-file/` | Explorateur de fichiers (grille / détails, fil d'Ariane, corbeille, copier-coller) |
| `noos-settings/` | Paramètres : personnalisation, barre des tâches, écrans (HDR/VRR), jeux, entrées |
| `noos-software/` | **Logiciels** : vitrine Flathub, recherche multi-sources, mises à jour, dépôts |
| `noos-pkg/` | Bibliothèque + CLI `noos-pkg` : Flatpak, rpm-ostree, dnf, apt, pacman, AUR |
| `noos-keys/` | **Porte-clés** : mots de passe du Secret Service (oo7, gnome-keyring), trousseaux, clés SSH |
| `noos-term/` | **Terminal** (VTE) : onglets, menu clic droit, thèmes, taille du texte, barre latérale d'alias (ex. `claude`) |
| `noos-snip/` | **Outil Capture d'écran** (Win+Maj+S) : rectangle, fenêtre, plein écran, forme libre, annotation |
| `noos-login/` | **Écran de connexion** (greeter greetd) façon Windows 11 : verrouillage avec horloge, sessions en cartes, comptes, accessibilité (grand texte, contraste, clavier visuel, Orca) |
| `noos-installer/` | **Installateur** plein écran : carte des fuseaux, clavier, compte (mot de passe fort), disque Btrfs (@, @home, swap), installation réseau Fedora 45 + RPM Fusion, GRUB |
| `noos-plymouth/` | Thème de démarrage Plymouth (logo NOOS, faisceau animé sur le ∞) |
| `noos-iso/` | Construction de l'ISO live d'installation (BIOS + UEFI/Secure Boot) |
| `noos-theme/` | Moteur de thème : palettes, accent, génération du thème libAdapta, rechargement à chaud |
| `noos-core/` | Configuration (`~/.config/noos/noos.toml`) et protocole IPC partagés |
| `noos-adapta/` | Bindings Rust de libAdapta (`libadapta-sys` + `libadapta`) |
| `noos-session/` | Entrée GDM, lanceur de session, portails XDG, fichiers .desktop |
| `third_party/libadapta/` | Sources de libAdapta 1.5 (commit dans `.noos-upstream-commit`) |
| `data/` | Logo, bouton Démarrer, fond d'écran |
| `scripts/` | `dev.sh`, `install-user.sh`, `build-libadapta.sh`, `noos-ipc.py` |

## Environnement de développement (Fedora Silverblue)

Tout se compile dans le toolbox **`noos-dev`** (Fedora 45, mêmes bibliothèques que l'hôte) :

```bash
toolbox enter noos-dev
./scripts/build-libadapta.sh      # une fois : compile libAdapta dans target/
cargo build
./scripts/dev.sh                  # NOOS dans une fenêtre de votre session GNOME
```

Dans la fenêtre imbriquée, la touche Super est souvent interceptée par GNOME. Pour piloter NOOS :

```bash
./scripts/noos-ipc.py toggle-start-menu
./scripts/noos-ipc.py get-windows
./scripts/noos-ipc.py screenshot path=/tmp/noos.png
```

## Installer la vraie session

1. Compiler et installer dans `~/.local/opt/noos`, sans sudo :
   ```bash
   toolbox run -c noos-dev ./scripts/install-user.sh
   ```
2. Ajouter « NOOS » à l'écran de connexion. À faire une seule fois, **sur l'hôte** (pas dans le toolbox) :
   ```bash
   sudo ./noos-session/install-session.sh
   ```
   Le script écrit uniquement dans `/usr/local`, qui est modifiable sur Silverblue : pas de couche rpm-ostree, pas de redémarrage.
3. Se déconnecter, choisir **NOOS** via l'engrenage de GDM, puis se connecter.

- **Quitter :** Ctrl+Alt+Retour arrière (retour à GDM).
- **Journal :** `~/.local/state/noos/session.log`.
- **Mettre à jour :** relancer l'étape 1 suffit.

### Intégration à la session (portails, partage d'écran)

Au démarrage, NOOS active `noos-session.target`, lié à `graphical-session.target`. Sans cette cible,
`xdg-desktop-portal` refuse de démarrer, et les Flatpak perdent le thème, les icônes, le sélecteur de
fichiers et l'ouverture de liens entre applications.

Tout est écrit dans `$XDG_RUNTIME_DIR`, donc effacé à la déconnexion. La session GNOME n'est pas touchée.

- **Portails :** `gtk` pour presque tout, `wlr` pour le partage d'écran. `xdg-desktop-portal-wlr` est
  fourni dans `~/.local/opt/noos/libexec` : pas de couche rpm-ostree.
- **Capture :** le compositeur implémente `ext-image-copy-capture-v1` (shm et dmabuf).
- **Accès :** les Flatpak n'ont pas d'accès direct à la capture. Ils passent par le portail, qui
  demande l'accord de l'utilisateur.

## ISO d'installation (noos-iso)

Construit une ISO live Fedora 45 qui démarre sur l'installateur NOOS (rien n'est installé
sur l'hôte : compilation dans le toolbox, système live dans un conteneur podman rootless) :

```bash
./noos-iso/build.sh          # → noos-iso/out/noos-45-x86_64.iso (~780 Mo)
```

- **Démarrage** : GRUB en BIOS (El Torito) et UEFI (shim + GRUB signés de Fedora, Secure
  Boot), ISO hybride (graver sur clé USB avec `dd` ou Fedora Media Writer). Entrée « journal
  sur port série » pour le dépannage.
- **Live** : connexion automatique de root sur tty1 → `noos-comp --drm --kiosk noos-installer`.
  Journaux : `/tmp/noos-live.log`, `/tmp/noos-install.log`.
- **Système installé** : `@core` Fedora 45 + paquets de `noos-iso/packages-*.txt`, NOOS dans
  `/opt/noos`, greetd + noos-login, Plymouth NOOS, SELinux étiqueté à l'installation, journal
  copié dans `/var/log/noos-install.log`.
- Testé dans QEMU (UEFI et BIOS) : installation complète, connexion, bureau.

## Logiciels (noos-software / noos-pkg)

Une seule application pour toutes les sources, choisies automatiquement selon la distribution :

| Distribution | Backend système | Dépôts additionnels gérés |
|---|---|---|
| Fedora Atomic (Silverblue, Kinoite, Bazzite…) | `rpm-ostree` (superposition, redémarrage signalé) | RPM Fusion, COPR, fichier .repo, surcharges dnf5 |
| Fedora, RHEL, Alma, Rocky | `dnf` (dnf5 / dnf4) | RPM Fusion, COPR, .repo, EPEL + CRB (EL) |
| Debian, Ubuntu, Mint… | `apt` (sources une ligne et deb822) | PPA, Universe/Multiverse, contrib/non-free, backports, dépôt + clé |
| Arch, EndeavourOS, CachyOS… | `pacman` (+ AUR via paru/yay) | multilib, Chaotic-AUR, dépôt personnalisé |
| toutes | Flatpak (utilisateur / système) | Flathub, Flathub Beta, .flatpakrepo, retrait du filtre Fedora |

- Les opérations admin passent par **un seul** `pkexec` par opération. rpm-ostree et Flatpak passent par polkit.
- Sur Arch, NOOS n'utilise jamais `pacman -Sy` seul : il installe avec `-Syu` et liste les mises à jour avec `checkupdates`, pour éviter les mises à jour partielles.
- La page Explorer affiche la vitrine Flathub (API publique v2, en cache dans `~/.cache/noos-software`) : bannière, catégories, populaires, tendances, nouveautés et fiches détaillées.

En ligne de commande :

```bash
noos-pkg info
noos-pkg search firefox
noos-pkg updates
noos-pkg presets
noos-pkg repo-add -b dnf copr atim/starship --dry-run
```

**Testé :**
- dans des conteneurs Fedora 45, Debian 13, Ubuntu 24.04 et Arch : installation et suppression, mises à jour, ajout, activation et suppression de dépôts ;
- sur l'hôte Silverblue : rpm-ostree en lecture seule et en simulation.

**À faire :**
- un **agent polkit** propre à NOOS : dans la vraie session, il n'y en a pas encore pour afficher la demande de mot de passe ;
- zypper (openSUSE) ;
- les fiches détaillées des paquets non Flatpak.

## Paramètres (noos-settings)

L'organisation reprend celle de Windows 11 : compte et recherche dans la barre latérale, catégories en icônes colorées, pages à fil d'Ariane, réglages présentés en cartes.

| Page | Contenu |
|---|---|
| Système › Écran | Moniteurs, échelle, résolution, fréquence, orientation, VRR, HDR. Les changements s'appliquent à chaud, avec retour automatique aux anciens réglages au bout de 15 s sans confirmation |
| Système › Notifications | Ne pas déranger, son, durée des bannières, notification de test |
| Système › Alimentation | Profil d'alimentation (power-profiles-daemon / tuned-ppd), extinction de l'écran et veille après inactivité, inhibition en plein écran |
| Système › Informations | Processeur, mémoire, carte graphique, renommer le PC |
| Personnalisation | Thèmes rapides, arrière-plan (dont les images du jour Bing, avec changement quotidien automatique), couleurs, thèmes libAdapta, barre des tâches (hauteur 28 à 96 px, icônes des applis actives regroupées dans « ^ » ou étalées), polices |
| Confidentialité › Pare-feu | Détection firewalld / ufw / nftables / iptables, bouclier vert / rouge / orange, ouverture et fermeture de ports, installation d'un pare-feu |

Une page s'ouvre directement avec `noos-settings <page>` (`display`, `power`, `firewall`, `taskbar`, `colors`…).

## Terminal (noos-term)

- **Onglets** dans la barre de titre ; le dossier courant est repris à l'ouverture d'un nouvel onglet.
- **Clic droit :** copier, coller, tout sélectionner, ouvrir ou copier le lien. Ctrl+clic ouvre un lien.
- **Thèmes :** NOOS (suit le mode clair/sombre), Campbell, One Half, Dracula, Nord, Catppuccin, Tokyo Night, Gruvbox, Solarized.
- **Alias :** commandes enregistrées dans la barre latérale (Ctrl+Maj+B), lancées dans l'onglet actif ou un nouvel onglet. « Claude » (`claude`) est fourni par défaut.
- **Configuration :** `~/.config/noos/terminal.toml`.

| Raccourci | Action |
|---|---|
| Ctrl+Maj+T / W / N | Nouvel onglet / fermer l'onglet / nouvelle fenêtre |
| Ctrl+Maj+C / V / A | Copier / coller / tout sélectionner |
| Ctrl + / Ctrl − / Ctrl 0 | Taille du texte |
| Ctrl+PgUp / PgDn, Alt+1…9 | Changer d'onglet |
| Ctrl+, | Préférences |
| F11 | Plein écran |

## Élévation de privilèges (agent polkit)

noos-shell est l'agent d'authentification polkit de la session : quand une application
demande une action d'administration (écrire une ISO sur clé USB avec Popsicle via udisks,
`pkexec`, installer un paquet…), une fenêtre « Contrôle de compte d'utilisateur » s'affiche
par-dessus l'écran assombri (compte administrateur, mot de passe, Oui / Non). Le mot de passe
est vérifié par l'assistant setuid de polkit (`polkit-agent-helper-1`, PAM) ; NOOS ne le
conserve pas. Aperçu sans polkit : `NOOS_POLKIT_TEST=1 noos-shell`.

Dans un terminal, `pkexec` garde sa propre invite texte (comme sous GNOME) ; ajoutez
`--disable-internal-agent` pour passer par la fenêtre.

## Raccourcis

| Raccourci | Action |
|---|---|
| Super | Menu Démarrer |
| Super+E / Super+I / Super+Entrée | Explorateur / Paramètres / Terminal (noos-term) |
| Super+← / → | Moitié gauche / droite, puis 2/3 et 1/3 en répétant |
| Super+↑ / ↓ | Maximiser / restaurer-réduire ; depuis une moitié : quart du haut / du bas |
| Glisser une fenêtre contre un bord | Moitié (Ctrl : tiers), coin = quart, haut = maximiser |
| Glisser une fenêtre vers le haut | Barre de dispositions : moitiés, 2/3 + 1/3, tiers, moitié + quarts, quarts |
| Super+glisser | Déplacer une fenêtre depuis n'importe où |
| Super+D | Afficher le bureau |
| Super+F, F11 | Plein écran |
| Alt+Tab, Alt+F4 | Fenêtre suivante, fermer |
| Super+Maj+S, Impr. écran | Outil Capture d'écran |
| Super+Maj+R | Recharger la configuration |
| Ctrl+Alt+F1…F12 | Changer de terminal virtuel |

## Effets de transparence (Mica)

Quand les effets de transparence sont activés, le compositeur floute le fond d'écran sous la
barre des tâches, le menu Démarrer, les réglages rapides et le centre de notifications. C'est le
« Mica » de Windows 11 : le flou est calculé une seule fois par fond d'écran, réduit 8× sur le
processeur, puis affiché sous les panneaux avec des coins arrondis. Le coût par image est nul.
Ce qui passe derrière (les fenêtres) n'est pas flouté : ce serait l'« acrylique », plus coûteux.

## Personnalisation du thème

Le thème est écrit dans `~/.local/share/themes/NOOS/libadapta-1.5/` :

- `defaults-dark.css` et `defaults-light.css` : couleurs nommées générées (palette, accent, surcharges) ;
- `base.css` : feuille de style de libAdapta + ajustements NOOS (coins, boutons Windows 11, police, CSS libre).

La session exporte `ADAP_DEBUG_THEME_NAME=NOOS`, donc toutes les applis libAdapta de la session
suivent ce thème. Votre session GNOME n'est pas touchée. Les applis NOOS se mettent à jour à chaud
dès que `noos.toml` change.

Tout se règle dans **Paramètres → Personnalisation** ou à la main dans `~/.config/noos/noos.toml` :

```toml
[appearance]
color_scheme = "dark"        # dark | light | system
palette = "mica"             # nuit-saphir | mica | graphite | aurore | adapta
accent = "#4cc2ff"
corner_radius = 8
panel_opacity = 0.9
windows_buttons = true
custom_css = "headerbar { min-height: 40px; }"

[appearance.overrides]       # n'importe quelle couleur nommée libAdapta
window_bg_color = "#101014"

[gaming]
vrr = "fullscreen"           # off | always | fullscreen
direct_scanout = true
ten_bit = true

[[output]]
name = "DP-1"
scale = 1.25
mode = "2560x1440@165"
vrr = "always"
```

L'option `apply_to_all_apps` écrit aussi les couleurs dans `~/.config/gtk-4.0/gtk.css`, pour les
applis libadwaita classiques. Attention : ce fichier est également lu par vos autres sessions.

## État du prototype : ce qui marche, ce qui manque

**Fonctionne (testé en mode imbriqué) :**
- compositeur Smithay : xdg-shell, layer-shell, dmabuf, presse-papiers ;
- placement et snap des fenêtres, IPC ;
- barre des tâches, menu Démarrer, réglages rapides ;
- explorateur, paramètres, thème libAdapta généré à chaud ;
- Xwayland via xwayland-satellite.

**Testé en mode imbriqué :**
- notifications (`org.freedesktop.Notifications`) : bannières, centre de notifications, calendrier, ne pas déranger ;
- zone des applis actives (StatusNotifierItem + dbusmenu) : menu « ^ » ou icônes étalées ;
- Outil Capture d'écran ;
- capture `ext-image-copy-capture` et partage d'écran PipeWire (portail wlr).

**Écrit mais pas encore testé sur du vrai matériel :** le backend DRM (session GDM), le VRR
(`VRR_ENABLED`), le scanout direct en plein écran et le framebuffer 10 bits.

> Votre GPU actuel (Intel UHD 630, gen 9.5) n'expose pas le VRR dans le pilote i915 : la page
> Écrans affichera « VRR ✗ ». Un GPU AMD, ou Intel gen 11 et plus récent, est nécessaire pour l'essayer.

**À faire :**
- sortie **HDR** réelle : encodage PQ/BT.2020, `HDR_OUTPUT_METADATA`, protocole `wp-color-management` (aujourd'hui seule la détection EDID + pilote existe) ;
- multi-GPU ;
- changement de mode à chaud ;
- décorations serveur pour les applis sans CSD ;
- verrouillage d'écran (`ext-session-lock`) ;
- sélecteur d'écran / de fenêtre pour le partage d'écran (aujourd'hui : premier écran) ;
- agent polkit NOOS (demandes de mot de passe dans la vraie session) ;
- grabs de popups ;
- `wp-tearing-control`.

## Licences

- NOOS : GPL-3.0-or-later.
- `third_party/libadapta` : LGPL-2.1-or-later.
- `noos-adapta/` : dérivé de libadwaita-rs (MIT).
- Code adapté de l'exemple smallvil de Smithay : MIT.

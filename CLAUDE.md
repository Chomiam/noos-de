# CLAUDE.md — notes pour Claude Code sur le projet NOOS

NOOS est un environnement de bureau Wayland en Rust : compositeur Smithay maison, interface
façon Windows 11, applications GTK4 + **libAdapta**, pensé pour le jeu (VRR, HDR) et la
personnalisation du thème. Dépôt : https://github.com/Chomiam/noos-de (public, GPL-3.0).
Le `README.md` décrit les fonctionnalités ; ce fichier donne le contexte de travail.

## Préférences de l'utilisateur (Axel)

- **Écrire en français** (réponses, messages de commit, chaînes de l'interface, commentaires).
- **libAdapta, PAS libadwaita** (fork de libadwaita 1.5 par Linux Mint, pour la liberté de
  thème). L'utilisateur l'a répété plusieurs fois. Crate Rust `adap` (`noos-adapta/`), API
  libadwaita 1.5 uniquement. Thèmes dans `~/.local/share/themes/<nom>/libadapta-1.5/`.
- Apparence et comportement inspirés de **Windows 11** (barre centrée, menu Démarrer, Mica,
  contrôle de compte, écran de verrouillage…), boutons de fenêtre façon macOS à droite.
- Un dossier par composant ; tester visuellement (captures) avant d'annoncer « c'est fait ».
- Ne jamais demander ni réutiliser de mot de passe ou de jeton : l'utilisateur fait lui-même
  les étapes root et les connexions (GitHub `gh auth login`, COPR `~/.config/copr`). S'il en
  colle un dans la conversation, ne pas l'utiliser, lui conseiller de le régénérer.
- Pousser sur GitHub uniquement quand il le demande.

## Organisation

| Dossier | Rôle |
|---|---|
| `noos-comp/` | Compositeur Smithay (DRM/KMS + libinput, ou imbriqué winit), tiling, Mica (flou), capture d'écran, IPC, mode kiosque (`--kiosk "<cmd>"`) |
| `noos-shell/` | Barre des tâches, menu Démarrer, réglages rapides, notifications, zone de notification, fond d'écran, **agent polkit** (`src/polkit.rs`) |
| `noos-settings/`, `noos-file/`, `noos-term/`, `noos-snip/`, `noos-keys/`, `noos-software/` + `noos-pkg/` | Applications |
| `noos-photoview/` | « Photos » : visionneuse d'images (zoom centré sur le pointeur, ambilight, bande de miniatures, diaporama, EXIF) ; appli par défaut des images (`/etc/xdg/noos-mimeapps.list`) |
| `noos-login/` | Écran de connexion (greeter greetd), lancé par `noos-greeter` = `noos-comp --kiosk noos-login` |
| `noos-installer/` | Installateur plein écran (installation réseau Fedora 45, Btrfs @/@home, GRUB) |
| `noos-plymouth/` | Thème Plymouth (`generate.py` produit les PNG, déjà générés dans le dépôt) |
| `noos-iso/` | Construction de l'ISO live (`build.sh` → `noos-iso/out/noos-45-x86_64.iso`) |
| `packaging/rpm/` + `.copr/` | Paquet RPM `noos` et construction COPR |
| `tools/vm/` | VM de test QEMU pilotée par QMP (voir plus bas) |
| `noos-core/` | Configuration `~/.config/noos/noos.toml`, `/etc/noos/login.toml`, protocole IPC |
| `noos-theme/` | Palettes, génération du thème libAdapta « NOOS » |
| `noos-adapta/` | Bindings Rust de libAdapta ; sources C dans `third_party/libadapta/` |
| `noos-session/` | Lanceur de session, fichiers .desktop, portails |
| `noos-core/src/disk_image.rs` | Images disque : montage / démontage par udisks (sans mot de passe), détection (`losetup -J`) ; Explorateur : bandeau « Démonter », alerte avant suppression |
| `noos-texteditor/` | Éditeur de texte et de code (GtkSourceView 5 : 160+ langages, recherche / remplacement, onglets, dossier, mini-carte, thèmes NOOS `data/sourceview/`) |
| `noos-lock/` | Écran de verrouillage (ext-session-lock via gtk4-session-lock, PAM service `noos-lock`) : animation, horloge, mot de passe ; `--preview [animation]` sans verrouiller |
| `noos-shader/` | Animations GLSL format Shadertoy (GLArea, libepoxy chargée à l'exécution) : intégrées, bibliothèque `~/.local/share/noos/shaders`, API Shadertoy (clé de l'utilisateur dans `~/.config/noos/shadertoy-key`) |
| `noos-archive/` | « Archives » : parcourir, extraire, créer, tester, convertir (7-Zip, bsdtar, unar, xorriso) ; modes `--extract-here`, `--extract-to`, `--compress`, `--quick zip\|7z` pour le menu de l'Explorateur |
| `noos-save/` | « Sauvegarder et Restaurer » : instantanés Btrfs (snapper), restauration au redémarrage ; `noos-save-helper` (root, pkexec + actions dnf5) |
| `noos-grub/` | Thème GRUB (`generate.py` produit images et polices .pf2, déjà générées) ; `install.sh` le copie dans /boot |
| `noos-system/` | Réglages système livrés par le paquet : Wi-Fi/Ethernet sans économie d'énergie, WirePlumber sans mise en veille, sudo `pwfeedback`, profil fastfetch (`/etc/xdg/fastfetch`), fish (`vendor_conf.d` : pas de message d'accueil, fastfetch à l'ouverture) |

## Environnement de développement

- Hôte : Fedora 45 Silverblue (GNOME/GDM), Intel UHD 630, **8 Go de RAM** (attention aux VM).
- On compile dans le **toolbox `noos-dev`** (Fedora 45), pour que les binaires correspondent
  aux bibliothèques de l'hôte :
  ```bash
  toolbox create -c noos-dev --distro fedora --release 45
  toolbox run -c noos-dev sudo dnf builddep packaging/rpm/noos.spec   # dépendances de compilation
  toolbox run -c noos-dev sudo dnf install grim xdg-desktop-portal-wlr xwayland-satellite \
      qemu-system-x86-core edk2-ovmf qemu-img guestfs-tools libtimezonemap copr-cli gh rpm-build
  ```
  Si Claude tourne dans une sandbox Flatpak : `flatpak-spawn --host toolbox run -c noos-dev bash -c '…'`,
  et pour installer en root dans le toolbox : `flatpak-spawn --host podman exec -u root noos-dev dnf install …`.
- libAdapta est compilée (pas installée) dans `target/libadapta-build` par
  `scripts/build-libadapta.sh` ; `.cargo/config.toml` y pointe `PKG_CONFIG_PATH`.
  Les tests (`cargo test -p noos-login`…) ont besoin de
  `LD_LIBRARY_PATH=target/libadapta-build/src`.
- Construire : `cargo build --release -p <crate>` (profil release, LTO thin).
- Installer dans la vraie session de l'utilisateur (préfixe `~/.local/opt/noos`, sans sudo) :
  `toolbox run -c noos-dev ./scripts/install-user.sh` (remplacement atomique des binaires).
  Puis : `pkill -x noos-shell` (le chien de garde du compositeur le relance) ; un changement
  du compositeur demande une déconnexion/reconnexion.
- Journal de la session NOOS : `~/.local/state/noos/session.log`.

### Aperçu imbriqué (sans toucher à la vraie session)

```bash
toolbox run -c noos-dev bash -c 'WAYLAND_DISPLAY=wayland-1 NOOS_PROFILE=release ./scripts/dev.sh [--no-shell] [--kiosk noos-login]'
toolbox run -c noos-dev bash -c 'WAYLAND_DISPLAY=wayland-2 grim capture.png'   # capture de l'aperçu
```
On ne peut pas cliquer dans l'aperçu : utiliser les variables de test ci-dessous.

| Variable | Effet |
|---|---|
| `NOOS_LOGIN_PAGE=login` | noos-login ouvre directement le formulaire |
| `NOOS_LOGIN_SESSION_DIRS=…` | dossiers de sessions supplémentaires (le toolbox n'en a pas) |
| `NOOS_LOGIN_STATE=…` | fichier d'état du greeter (au lieu de /var/lib/noos-login) |
| `NOOS_INSTALLER_PAGE=n` / `install` | page n de l'installateur / installation simulée de bout en bout |
| `NOOS_INSTALL_DRYRUN=1` | installateur en simulation (commandes affichées, rien d'exécuté) |
| `NOOS_INSTALLER_DISK=/dev/sdX` | disque présélectionné |
| `NOOS_INSTALLER_SCROLL=bottom` | page de l'installateur défilée jusqu'en bas (ex. matériel détecté du récapitulatif) |
| `NOOS_POLKIT_TEST=1` | noos-shell affiche une fenêtre d'élévation d'exemple |
| `NOOS_SOFTWARE_PAGE=repos` | noos-software s'ouvre sur une page (`explore`, `installed`, `updates`, `repos`) |
| `NOOS_SOFTWARE_SEARCH=gimp` | noos-software lance une recherche au démarrage |
| `NOOS_REORDER_DEMO=1` | noos-shell fait glisser la 1re icône épinglée de 2 crans (sans enregistrer) |
| `NOOS_SAVE_DEMO=1` (+ `NOOS_SAVE_DEMO_PENDING=1`, `NOOS_SAVE_DEMO_BOOT=n`) | noos-save affiche des instantanés fictifs (restauration en attente, démarrage sur l'instantané n), sans rien exécuter |
| `NOOS_SAVE_PAGE=settings` / `NOOS_SAVE_EXPAND=n` / `NOOS_SAVE_COMPARE=n` / `NOOS_SAVE_SCROLL=px` | noos-save : page Réglages, instantané n déplié, fenêtre « Comparer » de n, défilement |
| `NOOS_NET_DEMO=1` (+ `NOOS_NET_MEASURE=1`) | noos-settings › Réseau : connexions, Wi-Fi, DNS, VPN fictifs, sans NetworkManager (+ mesure DNS lancée) ; pages directes `noos-settings network\|wifi\|dns\|vpn\|ip\|hotspot\|network-diagnostics` |
| `NOOS_VPN_DEMO=1` | noos-shell : profils VPN fictifs, bouclier VPN dans la barre et son menu ouvert |
| `NOOS_EDITOR_DEMO=1` (+ `NOOS_EDITOR_SEARCH=mot`) | noos-texteditor : dossier et fichiers d'exemple (Rust, Python, CSS, Markdown), recherche lancée ; lancer depuis le conteneur (gtksourceview5) |
| `NOOS_LOCK_DEMO_CARD=1` | noos-lock : carte de connexion affichée d'emblée (avec `--preview`) |
| `NOOS_SETTINGS_SCROLL=px` | noos-settings : défile la page ouverte (captures) |
| `NOOS_FILE_MENU_DEMO=x,y` | noos-file ouvre le menu contextuel de la vue en icônes en (x, y) ; y < 0 = depuis le bas |
| `NOOS_FILE_SELECT_DEMO=0,1,2` | noos-file sélectionne ces éléments puis rejoue un clic dans le vide (coin inférieur droit) |
| `NOOS_PHOTOVIEW_DEMO=<image>;info;zoom=2.5;next;hide;slideshow` | noos-photoview : ouvre une image, panneau d'infos, zoom, suivante, commandes masquées, diaporama |
| `NOOS_ARCHIVE_DEMO=<archive>\|create` | noos-archive : ouvre une archive / la page de création préremplie |
| `NOOS_FILE_DEMO=<dossier>;details;list;select=0,2;look;zoom=3;sort=size;copy=<source>;menu;mount;trash` | noos-file : ouvre un dossier, volet de détails, vue Détails, sélection, aperçu rapide, zoom, tri, copie suivie |

### Règles de sécurité (erreurs déjà commises)

- **Jamais `pkill -x noos-comp`** : tue la vraie session de l'utilisateur. Pour l'aperçu :
  `kill $(ps -eo pid,comm,args | awk '$2=="noos-comp" && /--winit/ {print $1}')`.
- **Jamais `pkill -f <motif>`** si le motif figure dans sa propre ligne de commande (la commande
  se tue elle-même, code 144). Chercher le PID avec `ps`/`awk`, puis `kill`.
- Ne jamais lancer une appli « unique » (noos-term, noos-settings) dans l'aperçu sans
  `dbus-run-session` : elle s'ouvrirait dans la vraie session.
- Variables de démo contenant « ; » : le compositeur lance la commande du kiosque avec `sh -c` ;
  mettre l'argument entre apostrophes (sinon le reste est exécuté comme des commandes).
- Ne jamais mettre de gros fichiers (disque de VM, ISO) dans `/tmp` : c'est de la RAM.

## ISO, VM de test, COPR

- **ISO** : `./noos-iso/build.sh` (≈ 15 min, sans sudo : compilation dans le toolbox, système
  live dans un conteneur podman `--privileged` rootless). Sortie : `noos-iso/out/`.
  `--no-stage` réutilise la compilation précédente.
- **VM** (dans le toolbox) : `tools/vm/vm.sh start noos-iso/out/noos-45-x86_64.iso uefi`, puis
  `tools/vm/vm.sh install`, `tools/vm/qmp.py shot x.png`, `tools/vm/vm.sh stop`,
  `tools/vm/vm.sh start "" uefi` (démarrer sur le disque installé), `tools/vm/vm.sh login`.
  Lire/modifier le disque de la VM éteinte : `guestfish` (`LIBGUESTFS_BACKEND=direct`,
  `mount-options subvol=@ /dev/sda3 /`) ; après `upload`, remettre l'étiquette SELinux
  (`setxattr security.selinux system_u:object_r:bin_t:s0 26 <fichier>`).
- **COPR** : projet https://copr.fedorainfracloud.org/coprs/chomiam/noos/ (Fedora 45 x86_64),
  paquet `noos` construit depuis GitHub (`main`) par `.copr/Makefile` →
  `packaging/rpm/make-srpm.sh` (dépendances Rust « vendored », version `0.1.0-0.<date>.git<commit>`).
  Relancer : `copr-cli build-package noos --name noos`. Tester un SRPM comme COPR :
  conteneur `fedora:45` + `dnf builddep` + `rpmbuild --rebuild`.
  Le paquet installe NOOS dans `/opt/noos` (+ `/usr/bin/noos-session`, `/usr/bin/noos-greeter`).
  L'installateur l'installe depuis COPR (`noos-iso/copr.txt`), sinon copie la version de l'ISO.

## Pièges de Fedora 45 et leçons apprises

- **kmscon** remplace la console texte et fournit `autovt@` : on l'exclut (`--exclude=kmscon`)
  et on remet agetty (`autovt@.service` → `getty@.service`, `getty@tty1` activé).
- Système live construit **sans dépendances faibles** : penser aux paquets chargés
  dynamiquement (invisibles pour `ldd`) : `systemd-pam` (sinon pas de session logind → libseat
  échoue), `libglvnd-gles`/`mesa-libEGL` (GTK), `glycin-loaders` (images).
- Les dépendances de libAdapta (appstream…) : `ldd` avec `LD_LIBRARY_PATH` vers libAdapta.
- **SELinux** : le live tourne avec `selinux=0` ; l'installateur étiquette le système installé
  avec `setfiles` (sinon systemd ne démarre pas en enforcing).
- **greetd** (paquet Fedora) : compte `greetd` (pas `greeter`) ; greetd lance lui-même
  `sh -c "… exec <commande>"` : passer la ligne Exec telle quelle, jamais un `sh -c` imbriqué.
- **polkit** : gio ne garde qu'une référence faible sur la connexion au bus système → la
  conserver, sinon polkitd oublie l'agent (« No authentication agent found »). Dans un
  terminal, `pkexec` garde son invite texte ; `--disable-internal-agent` pour tester la fenêtre.
- **Rendu logiciel (llvmpipe, VM sans 3D)** : attendre la fin du dessin (`glFinish`) avant
  d'afficher, sinon tuiles noires / restes d'images.
- **rpmbuild** exporte son propre `PKG_CONFIG_PATH` (prioritaire sur `.cargo/config.toml`) ;
  libAdapta a besoin de `sassc` et meson doit être en `--wrap-mode=nodownload`.
- dnf dans `--installroot` : télécharger dans le disque cible (`--setopt=cachedir=…`), pas en RAM.
- **Verrouillage** (`noos-comp/src/lock.rs`) : tant que le verrou est actif, seules les surfaces
  de verrouillage sont rendues et reçoivent les entrées (aucun raccourci sauf VT). Plus de
  surface vivante → noos-lock relancé (testé : tué pendant le verrou, l'écran reste noir puis
  se reverrouille). Déclencheurs : Super+L, IPC `lock` / `suspend`, `[lock] idle_minutes`.
- **Restauration Btrfs** : la racine est montée par son nom (`subvol=@`, fstab + rootflags), donc
  `snapper rollback` (qui change le sous-volume par défaut) est sans effet. `noos-save-helper
  rollback` renomme `@` en `@.avant-restauration-…`, crée `@` depuis l'instantané, déplace
  `.snapshots`, et choisit avec grubby le noyau le plus récent dont les modules existent dans
  l'instantané (/boot est hors instantanés). Premier instantané **après** `setfiles` (SELinux).
- **Démarrage sur un instantané** (GRUB) : `rootflags=subvol=@/.snapshots/N/snapshot
  systemd.volatile=overlay enforcing=0 noos.snapshot=N`. `/.snapshots` y est vide (sous-volume
  imbriqué) : `noos-snapshot-boot.service` monte `@/.snapshots` ; la racine est un overlay, donc
  le périphérique Btrfs se lit via `/home`.
- **Thème GRUB** : tester avec `grub2-mkrescue` + `qemu-system-x86_64 -display none` et
  `screendump` sur le moniteur ; les .pf2 doivent être chargés (`loadfont`) avant `set theme`.

## Reste à faire / idées

- Plymouth : thème NOOS non vérifié sur vrai matériel (en VM, mode texte faute de 3D).
- Menu GRUB du système installé : « Fedora Linux » au lieu de NOOS.
- Webhook GitHub → COPR pour reconstruire à chaque push (URL dans COPR → Settings → Integrations).
- Sélecteur d'écran pour le partage d'écran, VRR/HDR sur matériel réel.

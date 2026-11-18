# CLAUDE.md — notes pour Claude Code sur le projet NOOS

NOOS est un environnement de bureau Wayland en Rust : compositeur Smithay maison, interface
façon Windows 11, applications GTK4 + **libAdapta**, pensé pour le jeu (VRR, HDR) et la
personnalisation du thème. **Base : Debian sid (unstable)** depuis novembre 2026 (avant :
Fedora 45 + COPR, abandonnés). Dépôt : https://github.com/Chomiam/noos-de (public, GPL-3.0).
Le `README.md` est la vitrine (grand public, captures `docs/captures/*.webp`) ; le détail de
chaque composant est dans `docs/GUIDE.md` (à compléter à chaque nouvelle fonction) ; ce fichier
donne le contexte de travail.

## Préférences de l'utilisateur (Axel)

- **Écrire en français** (réponses, messages de commit, chaînes de l'interface, commentaires).
- **libAdapta, PAS libadwaita** (fork de libadwaita 1.5 par Linux Mint, pour la liberté de
  thème). L'utilisateur l'a répété plusieurs fois. Crate Rust `adap` (`noos-adapta/`), API
  libadwaita 1.5 uniquement. Thèmes dans `~/.local/share/themes/<nom>/libadapta-1.5/`.
- Apparence et comportement inspirés de **Windows 11** (barre centrée, menu Démarrer, Mica,
  contrôle de compte, écran de verrouillage…), boutons de fenêtre façon macOS à droite.
- Pensé pour le jeu : l'architecture **i386 est activée par défaut** (Steam, Wine) —
  installateur, postinst du paquet, préréglage « Applications 32 bits » de Logiciels.
- Un dossier par composant ; tester visuellement (captures) avant d'annoncer « c'est fait ».
- Ne jamais demander ni réutiliser de mot de passe ou de jeton : l'utilisateur fait lui-même
  les étapes root et les connexions (GitHub, secrets du dépôt comme `APT_SIGNING_KEY`). S'il en
  colle un dans la conversation, ne pas l'utiliser, lui conseiller de le régénérer.
- **On travaille directement sur `main`** (plus de branche de travail ; `debian-sid` supprimée
  le 16/11/2026). Chaque envoi sur `main` compile, teste et **publie** le paquet dans le dépôt
  APT : ne pousser que du code compilé ou testé autant que possible, et pas pendant qu'un run
  de `main` publie (`cancel-in-progress` l'annulerait).

## Organisation

| Dossier | Rôle |
|---|---|
| `noos-comp/` | Compositeur Smithay (DRM/KMS + libinput, ou imbriqué winit), tiling, Mica (flou), capture d'écran, IPC, mode kiosque (`--kiosk "<cmd>"`), **applis X11** : lance `Xwayland` (paquet Debian) et gère ses fenêtres lui-même (`src/xwayland.rs`, `X11Wm` de Smithay) |
| `noos-shell/` | Barre des tâches, menu Démarrer, **recherche** (`src/search.rs` : champ de la barre en `OnDemand` au survol, panneau `noos-search` ; applis, Paramètres, fichiers, calcul), réglages rapides, notifications, zone de notification, fond d'écran, **agent polkit** (`src/polkit.rs`) |
| `noos-settings/`, `noos-file/`, `noos-term/`, `noos-snip/`, `noos-keys/`, `noos-software/` + `noos-pkg/` | Applications |
| `noos-pkg/src/deb.rs` | Fichiers `.deb` : fiche (`dpkg-deb -f`), simulation sans root (`apt-get -s install <fichier>`), installation `apt-get install <chemin>` (pkexec) ; Logiciels est l'appli par défaut des `.deb` |
| `noos-pkg/src/details.rs` | Fiche d'un paquet (Logiciels › clic sur un résultat) : `Backend::details` (apt : `apt-cache show` traduit + `policy` + DEP-11 lu par un mini-lecteur YAML ; Flatpak : `remote-info` / `info` + `appstream.xml`) et `Backend::simulate` (`apt-get -s install\|remove`, runtime Flatpak) ; interface `noos-software/src/details.rs` ; plan d'évolution : `docs/plans/logiciels.md` |
| `noos-pkg/src/{space,odrs,commands,appimage_update,appimagehub,mirrors}.rs` | Logiciels : place occupée et données, avis ODRS (`[software] reviews`, désactivé par défaut), commande introuvable (fish `vendor_functions.d`, bash `profile.d`), mises à jour des AppImage (`.upd_info`, zsync), catalogue AppImageHub (comparaison), miroirs Debian (test de débit, changement) |
| `noos-pkg/src/{cleanup,history,permissions,hardware,packs,owner}.rs` | Logiciels : « Faire de la place », historique (journal apt + instantanés noos-save), permissions Flatpak (`flatpak override`), compléments pour le matériel, packs (`noos-pkg/data/packs.json`, aussi lu depuis GitHub), paquet d'un lanceur ; `lib.rs` : `offline` (installation au redémarrage, `noos-offline-update.service`), `auto_update_flatpak` |
| `noos-pkg/src/appimage.rs` | AppImage : lecture sans exécution (`unsquashfs -offset`), installation dans `~/Applications` + lanceur `appimage-<id>.desktop` (`X-NOOS-AppImage`) ; Logiciels ouvre les `.AppImage` (type MIME `application/vnd.appimage`) |
| `noos-pkg/` | Paquets : backend **apt** (`backends/apt.rs` : `apt-cache`, `dpkg-query`, `apt-get -s`, DEP-11) + **sources APT** (`backends/aptsources.rs` : une ligne et deb822, activer / supprimer / composants) + Flatpak ; `sources.rs` regroupe (Debian sid, NOOS, experimental, contrib/non-free manquants) (plus d'ufw : remplacé par `noos-firewall`) |
| `noos-firewall/` | **Pare-feu applicatif** : démon `noos-firewalld` (root, service `noos-firewall`) — table nftables `inet noos` + NFQUEUE (crate `nfq`, sans libnetfilter-queue) ; programme par `/proc/net/*` → inode → `/proc/<pid>` ; socket `/run/noos-firewall/control.sock` (`SO_PEERCRED` + `pkcheck` `org.noos.firewall.manage`) ; règles `/etc/noos/firewall/` ; CLI `noos-firewall` ; `install-host.sh` (installation depuis le dépôt) |
| `noos-firewall-prompt/` | Fenêtre d'autorisation (agent de session, `/etc/xdg/autostart`) ; page Paramètres = `noos-settings/src/security.rs` |
| `noos-settings/src/devices/` | **Bluetooth et appareils** : BlueZ par D-Bus (gio, bus système) avec agent d'appairage `org.bluez.Agent1` (`bluez.rs`, `bt_add.rs`) ; **Imprimantes et scanners** par les outils CUPS (`lpoptions`, `lpinfo`, `driverless`, `lpadmin` direct pour le groupe `lpadmin`, sinon pkexec) et `scanimage -L` (`cups.rs`, `printers.rs`) |
| `noos-share/` | **Partage de fichiers** : Samba (bloc NOOS à la fin de smb.conf, `[homes]` + dossiers, wsdd2, Avahi), SSH (`sshd_config.d/90-noos.conf`), WebDAV (`noos-webdav` = service utilisateur rclone) ; `/etc/noos/sharing.toml` écrit par `noos-share-helper` (pkexec, `org.noos.share.helper`) ; règles du pare-feu « Partage NOOS · … » en zone LAN. Pages `noos-settings/src/network/sharing.rs` ; Explorateur : `noos-file/src/network.rs` (page Réseau, montage gvfs, identifiants + trousseau) |
| `noos-photoview/` | « Photos » : visionneuse d'images (zoom centré sur le pointeur, ambilight, bande de miniatures, diaporama, EXIF) ; appli par défaut des images (`/etc/xdg/noos-mimeapps.list`) |
| `noos-login/` | Écran de connexion (greeter greetd, compte `_greetd`), lancé par `noos-greeter` = `noos-comp --kiosk noos-login` |
| `noos-installer/` | Installateur plein écran : Btrfs @/@home, **debootstrap sid** puis apt dans le chroot (avancement `APT::Status-Fd`), contrib/non-free au choix, i386, paquet `noos` du dépôt APT (sinon copie de l'ISO), GRUB (shim signé en UEFI, `EFI/debian`, entrée NVRAM « NOOS »), portable détecté (`hardware.rs`) → auto-cpufreq |
| `noos-plymouth/` | Thème Plymouth (`generate.py` produit les PNG, déjà générés dans le dépôt) |
| `noos-iso/` | ISO live Debian sid : `build.sh` (podman/docker) ou CI `iso.yml` → `noos-sid-amd64.iso` ; live-boot, GRUB signé `gcdx64.efi` + `/.disk/info` |
| `debian/` | Paquet Debian `noos` (debhelper 13, `rules`, `postinst` « une seule fois » via `/var/lib/noos/*`) |
| `packaging/apt/` | `build-deb.sh` (version `0.1.0~git<date>.<commit>`), `update-repo.sh` (dépôt signé), `generate-key.sh`, `noos.sources`, `index.html` de Pages |
| `.github/workflows/` | `build.yml` (debian:sid : `.deb` + tests ; publication `gh-pages` sur `main`), `iso.yml` (à la demande) |
| `tools/vm/` | VM de test QEMU pilotée par QMP (voir plus bas) |
| `noos-core/` | Configuration `~/.config/noos/noos.toml`, `/etc/noos/login.toml`, protocole IPC |
| `noos-theme/` | Palettes, génération du thème libAdapta « NOOS » |
| `noos-adapta/` | Bindings Rust de libAdapta ; sources C dans `third_party/libadapta/` |
| `noos-session/` | Lanceur de session, fichiers .desktop, portails |
| `noos-core/src/disk_image.rs` | Images disque : montage / démontage par udisks (sans mot de passe), détection (`losetup -J`) ; Explorateur : bandeau « Démonter », alerte avant suppression |
| `noos-texteditor/` | Éditeur de texte et de code (GtkSourceView 5 : 160+ langages, recherche / remplacement, onglets, dossier, mini-carte, thèmes NOOS `data/sourceview/`) |
| `noos-lock/` | Écran de verrouillage (ext-session-lock via gtk4-session-lock, PAM `noos-lock` = `@include common-auth`) : animation, horloge, mot de passe ; `--preview [animation]` sans verrouiller |
| `noos-shader/` | Animations GLSL format Shadertoy `mainImage` (GLArea, libepoxy chargée à l'exécution) : intégrées, bibliothèque `~/.local/share/noos/shaders` (`library:<id>`), catalogue NOOS = `data/shaders/catalog/*.frag` → `build-index.py` → `index.json` téléchargé depuis GitHub (repli : copie du paquet), import de fichiers / code collé (GLSL Sandbox converti par `adapt`) |
| `noos-archive/` | « Archives » : parcourir, extraire, créer, tester, convertir (7-Zip `7zip`, bsdtar `libarchive-tools`, unar, xorriso) ; modes `--extract-here`, `--extract-to`, `--compress`, `--quick zip\|7z` pour le menu de l'Explorateur |
| `noos-widget-vitals/` | Widget « Ressources » de la barre : processeur, carte graphique (AMD `gpu_busy_percent`, Intel fréquence, NVIDIA `nvidia-smi`), mémoire, disque, réseau ; noms PCI `/usr/share/misc/pci.ids` ; `[taskbar] show_vitals` ; `NOOS_VITALS_OPEN=1` ouvre le détail |
| `noos-disk/` | « Disques » (udisks2 par gio, bus système) : partitions, formatage (exFAT, NTFS, FAT32, ext4, Btrfs, XFS, F2FS), LUKS, SMART, test de vitesse, images, éjection. Bibliothèque `noos_disk` partagée avec le shell et l'Explorateur. `noos-disk-helper` (pkexec, `org.noos.disk.helper`) : montage permanent `/mnt/<nom>`, « M'en donner la propriété ». `noos-disk --selftest /dev/loopN` teste les opérations destructives sur une image |
| `noos-core/src/updates.rs` | Mises à jour en arrière-plan : timer utilisateur `noos-update-check.timer` → `noos-pkg check --auto` (sans root : `apt-get update` dans `~/.cache/noos/apt` au plus toutes les 6 h, puis `apt-get -s full-upgrade` sur les listes les plus récentes ; `flatpak remote-ls --updates`) → `~/.local/state/noos/updates.json`, lu par l'icône de la barre, Logiciels et Paramètres › Mises à jour |
| `noos-comp/src/desktops.rs` + `outputs.rs` | Bureaux virtuels et plusieurs écrans (`[[output]] position, primary`, Super+Maj+←/→). Shell : `taskview.rs` (Super+Tab). Le shell reconstruit les barres (`rebuild_bars`) quand le bord ou la barre d'état change |
| `noos-shell/src/editmode.rs` + `desk.rs`, `gallery.rs`, `layouts.rs`, `widgets/` | **Mode édition** (plan : `docs/plans/mode-edition.md`) : barre d'outils (Widgets, Barres, Mises en page, annuler / rétablir, visite guidée), 6 zones de barre (`NoosConfig::bar_zones()`, barre des tâches + **barre d'état** = `[statusbar]`, ancien `[topbar]` lu), widgets `[[desktop_widget]]` (`id`, `size`, `style`, table `options`) dans la surface `noos-desktop-widgets` (`desk.rs` : un seul geste sur le `gtk::Fixed` — jamais sur l'élément qui bouge, sinon clignotement ; aimantation `noos_core::layout::snap`). Toute modification passe par `Shell::edit_layout` (instantané `noos_core::layout::Layout`, pile `History`). Météo : `noos_core::weather` (Open-Meteo, `curl`) ; jours fériés : `noos_core::holidays` |
| `noos-save/` | « Sauvegarder et Restaurer » : instantanés Btrfs (snapper, `ALLOW_GROUPS=sudo`), avant / après chaque opération apt (`/etc/apt/apt.conf.d/80noos-save` → `noos-save-helper apt-pre/apt-post`, numéro « avant » dans `/run/noos-save/apt-pre`), restauration au redémarrage ; comparaison des paquets par `dpkg-query --admindir` |
| `noos-grub/` | Thème GRUB (`generate.py` produit images et polices .pf2, déjà générées) ; `install.sh` le copie dans `/boot/grub/themes/noos` |
| `noos-shell/src/power.rs` + `noos-core/src/power.rs` | **Énergie** (élément `power` de la barre, `[taskbar] show_power`) : batterie dessinée (UPower par D-Bus, `PropertiesChanged` ; repli `/sys/class/power_supply`), jauge du mode sur PC fixe, panneau (santé, consommation, modes). Modes : power-profiles-daemon, sinon **auto-cpufreq** (portables : PPD masqué, `pkexec /usr/libexec/noos-power-helper profile <mode>`, action `org.noos.power.helper`) |
| `noos-gaming/` + `noos-settings/src/gaming/` | **Mode jeu Steam** (Paramètres › Jeux) : session gamescope `noos-steam-session` (réglages `~/.config/noos/steam-session.conf`), bascule bureau ↔ Steam sans mot de passe `noos-session-switch` (pkexec `org.noos.gaming.switch`, unité `systemd-run` + `[initial_session]` ponctuel de greetd par `/run/noos-gaming/greetd.toml`), `noos-gaming-helper` (pkexec `org.noos.gaming.helper` : session, démarrage, Decky Loader), `steamos-session-select` dans `/usr/lib/noos/steamos` ; composants pour le jeu (apt + Flatpak par noos-pkg) |
| `noos-system/` | Réglages système livrés par le paquet : Wi-Fi/Ethernet sans économie d'énergie, WirePlumber sans mise en veille, sudo `pwfeedback`, PAM `noos-lock`, `GRUB_DISTRIBUTOR=NOOS` (`/etc/default/grub.d/90-noos.cfg`), `noos-boot-setup` (initramfs-tools), fastfetch, fish |

## Environnement de travail

- Poste actuel d'Axel : **Windows 11** (application Claude, dépôt dans
  `C:\Users\Chomiam\Documents\Projets\noos-de`, Git Bash). Pas de WSL ni de conteneur :
  **les compilations se font sur GitHub Actions** (`build.yml`, conteneur `debian:sid`).
  CLI GitHub installée et connectée par Axel (`"/c/Program Files/GitHub CLI/gh.exe"`, pas
  toujours dans le PATH de Git Bash) : `gh run list -b main`, `gh run watch <id>`,
  `gh run view <id> --log-failed` (les journaux et le résumé d'un run demandent d'être
  connecté ; filtrer les couleurs avec `sed 's/\x1b\[[0-9;]*m//g'`). À refaire sur une
  nouvelle machine (`winget install --id GitHub.cli`, puis `gh auth login` par Axel).
- Rust 2024 : `gen` est un mot réservé (ne pas l'utiliser comme nom de variable).
  En local sous Windows, seule la logique pure se teste : `noos-pkg` compile avec un
  `noos-core` réduit (paths + updates), car `noos-core` utilise des sockets Unix.
- Fins de ligne : `.gitattributes` impose LF (`core.autocrlf=false` dans le dépôt) ; ne jamais
  committer de CRLF dans les scripts shell, `debian/rules` ou les fichiers système.
- Cible : **Debian sid**. Rust vient de **rustup** (`scripts/install-build-deps.sh`), pas des
  paquets Debian (édition 2024, crates récentes) ; d'où `dpkg-buildpackage -d`.
- Sur une machine Debian sid (ou conteneur distrobox `noos-dev`) :
  ```bash
  ./scripts/install-build-deps.sh        # bibliothèques -dev, debhelper, rustup
  ./scripts/build-libadapta.sh           # libAdapta dans target/libadapta-build
  cargo build --release -p <crate>
  LD_LIBRARY_PATH=target/libadapta-build/src cargo test -p noos-login
  packaging/apt/build-deb.sh             # → packaging/apt/out/noos_<version>_amd64.deb
  ```
- libAdapta est compilée (pas installée) dans `target/libadapta-build` ; `.cargo/config.toml`
  y pointe `PKG_CONFIG_PATH` (debian/rules l'exporte aussi, sinon ignoré).
- Installer dans la vraie session de l'utilisateur (préfixe `~/.local/opt/noos`, sans sudo) :
  `./scripts/install-user.sh`, puis `pkill -x noos-shell` (le chien de garde du compositeur le
  relance) ; un changement du compositeur demande une déconnexion/reconnexion.
- Journal de la session NOOS : `~/.local/state/noos/session.log`.

### Dépôt APT (remplace COPR)

- https://chomiam.github.io/noos-de/ = branche `gh-pages` (GitHub Pages, source « Deploy from
  a branch », `gh-pages` / racine). Suite `sid`, composant `main`, `binary-amd64` + `binary-i386`
  (vide, évite l'avertissement apt quand i386 est activée), 5 dernières versions.
- `build.yml` publie à chaque envoi sur `main` si le secret **`APT_SIGNING_KEY`** (clé GPG
  privée sans phrase de passe) existe ; sinon avertissement et rien n'est publié.
- Clé : `packaging/apt/generate-key.sh` (à lancer par Axel), clé publique à committer dans
  `packaging/apt/noos-archive-keyring.asc` (à défaut, la CI la tire du secret). Le paquet
  n'installe `noos.sources` + la clé que si elle est présente au moment de la construction.
- L'installateur ajoute le dépôt (clé copiée depuis l'ISO) et installe `noos` ; sinon copie de
  `/opt/noos` de l'ISO.

### Aperçu imbriqué (sans toucher à la vraie session) — sur une machine Linux

```bash
WAYLAND_DISPLAY=wayland-1 NOOS_PROFILE=release ./scripts/dev.sh [--no-shell] [--kiosk noos-login]
WAYLAND_DISPLAY=wayland-2 grim capture.png   # capture de l'aperçu
```
On ne peut pas cliquer dans l'aperçu : utiliser les variables de test ci-dessous.
Scripts prêts à l'emploi (capture puis arrêt) : `tools/preview/run_app.sh capture.png 5 "commande"`
(une appli en kiosque) et `tools/preview/shell_shot.sh capture.png 5 [VAR=valeur…]` (shell
complet). À lancer sur l'hôte quand l'appli a besoin du bus système (udisks, NetworkManager).

Captures publiques (README) : `HOME=<faux dossier personnel>` + `NOOS_DEMO_NAME="Alex Martin"`
+ fond `docs/captures/fond-aurore.py` (jamais le fond d'écran, les fichiers ni le nom réels de
l'utilisateur ; masquer les numéros de série, ex. Disques), puis `magick x.png -quality 90 x.webp`.

Scénarios scriptés (compositeur + shell, configuration **isolée** copiée dans le dossier ;
étapes `ipc`, `shot`, `sleep`, `run <commande>`, `snap <app_id> <zone>`) :
`tools/preview/scenario.sh DOSSIER "ipc {json}\nsleep 1\nshot nom"` (requêtes IPC par
`tools/preview/ipc.py`) — ne jamais tester bureaux / écrans avec la vraie configuration : le
compositeur y écrit (`[desktops] names`).

| Variable | Effet |
|---|---|
| `NOOS_WINIT_OUTPUTS=2` (`3`) / `NOOS_WINIT_SIZE=2400x760` | compositeur imbriqué : fenêtre découpée en plusieurs écrans simulés / taille de la fenêtre |
| `NOOS_TASKVIEW_DEMO=1` / `NOOS_EDIT_DEMO=1` | noos-shell : vue des tâches / mode édition ouverts après 2,5 s (sans visite guidée automatique) |
| `NOOS_EDIT_OPEN=étape;étape…` (+ `NOOS_EDIT_STEP_MS=1500`) | avec `NOOS_EDIT_DEMO=1` : étapes jouées l'une après l'autre — `widgets`, `bars`, `layouts`, `settings:n` (réglages du widget n), `toast:texte`, `preset:windows\|classic\|statusbar\|gnome\|vertical\|minimal`, `add:<type>`, `remove:n`, `undo`, `redo`, `tour:n`, `guides:x,y` (widget 0 « déplacé », repères affichés) |
| `NOOS_EDIT_SEARCH=mot` / `NOOS_EDIT_TOUR=n` / `NOOS_EDIT_GUIDES=x,y` | galerie de widgets préremplie / visite guidée à l'étape n / repères d'alignement |
| `NOOS_TRAY_DEMO=n` / `NOOS_TRAY_OPEN=1\|ms` | noos-shell : n icônes fictives dans la zone des applications actives (sans D-Bus) / leur bulle ouverte après 3,5 s (ou le délai donné, ex. `8000` après `NOOS_POWER_OPEN=1` pour vérifier qu'elle passe devant les panneaux) |
| `NOOS_WEATHER_DEMO=1` / `NOOS_MEDIA_DEMO=1` | widgets Météo (Lyon fictif, sans réseau) / Lecteur multimédia (morceau fictif, sans lecteur MPRIS) |
| `NOOS_LOGIN_PAGE=login` | noos-login ouvre directement le formulaire |
| `NOOS_LOGIN_SESSION_DIRS=…` | dossiers de sessions supplémentaires (un conteneur n'en a pas) |
| `NOOS_LOGIN_STATE=…` | fichier d'état du greeter (au lieu de /var/lib/noos-login) |
| `NOOS_INSTALLER_PAGE=n` / `install` | page n de l'installateur / installation simulée de bout en bout |
| `NOOS_INSTALL_DRYRUN=1` | installateur en simulation (commandes affichées, rien d'exécuté) |
| `NOOS_INSTALLER_DISK=/dev/sdX` | disque présélectionné |
| `NOOS_INSTALLER_SCROLL=bottom` | page de l'installateur défilée jusqu'en bas (ex. matériel détecté du récapitulatif) |
| `NOOS_POLKIT_TEST=1` | noos-shell affiche une fenêtre d'élévation d'exemple |
| `NOOS_SOFTWARE_PAGE=repos` | noos-software s'ouvre sur une page (`explore`, `installed`, `updates`, `repos`) |
| `NOOS_SOFTWARE_SEARCH=gimp` | noos-software lance une recherche au démarrage |
| `NOOS_SOFTWARE_UPDATES_DEMO=1` / `NOOS_SOFTWARE_QUEUE_DEMO=1` | noos-software : mises à jour fictives (rien d'enregistré) / file d'attente fictive (`sleep`) ouverte |
| `NOOS_SOFTWARE_CLEANUP=1\|demo` / `NOOS_SOFTWARE_HISTORY=1` | noos-software : « Faire de la place » ouvert (analyse réelle ou éléments fictifs) / historique des opérations ouvert |
| `NOOS_SOFTWARE_SORT=size` / `NOOS_SOFTWARE_COMPARE=1` (avec `NOOS_SOFTWARE_DETAILS`) / `NOOS_SOFTWARE_REVIEWS=1` | noos-software : Installées triées par place occupée / comparaison apt-Flatpak-AppImage ouverte / avis ODRS affichés sans modifier la configuration |
| `NOOS_SOFTWARE_MIRRORS=1` (+ `NOOS_SOFTWARE_MIRRORS_TEST=1`) / `NOOS_SOFTWARE_QUEUE_DEMO=close` | noos-software : tableau des miroirs Debian ouvert (+ mesures lancées) / fenêtre fermée pendant la file fictive (travail en arrière-plan) |
| `NOOS_SOFTWARE_HARDWARE_DEMO=1` / `NOOS_SOFTWARE_COLLECTION=category/Graphics` | noos-software : compléments pour un matériel fictif (NVIDIA, Wi-Fi Intel, manette, portable) / une catégorie ouverte (avec « Aussi en paquets Debian ») |
| `NOOS_SOFTWARE_DETAILS=gimp` (`gimp@apt`, `gimp@flatpak`, `apt:htop`, `flatpak:org.gimp.GIMP`) + `NOOS_SOFTWARE_SCROLL=px` (seul : page Explorer défilée après 6 s) | noos-software ouvre la fiche d'un logiciel (onglet apt / Flatpak choisi par `@`), défilée de px au bout de 3,5 s ; les captures AppStream viennent du réseau (cache `~/.cache/noos-software/img`) : attendre ~12 s la première fois |
| `NOOS_SEARCH_DEMO=texte` (`'texte;down;down'`, `start:texte`) | noos-shell : saisie simulée dans la pastille de recherche du dock (puis flèches) / dans le menu Démarrer ; panneau de résultats ouvert |
| `NOOS_REORDER_DEMO=1` | noos-shell fait glisser la 1re icône épinglée de 2 crans (sans enregistrer) |
| `NOOS_SAVE_DEMO=1` (+ `NOOS_SAVE_DEMO_PENDING=1`, `NOOS_SAVE_DEMO_BOOT=n`) | noos-save affiche des instantanés fictifs (restauration en attente, démarrage sur l'instantané n), sans rien exécuter |
| `NOOS_SAVE_PAGE=settings` / `NOOS_SAVE_EXPAND=n` / `NOOS_SAVE_COMPARE=n` / `NOOS_SAVE_SCROLL=px` | noos-save : page Réglages, instantané n déplié, fenêtre « Comparer » de n, défilement |
| `NOOS_NET_DEMO=1` (+ `NOOS_NET_MEASURE=1`) | noos-settings › Réseau : connexions, Wi-Fi, DNS, VPN fictifs, sans NetworkManager (+ mesure DNS lancée) ; pages directes `noos-settings network\|wifi\|dns\|vpn\|ip\|hotspot\|network-diagnostics` |
| `NOOS_VPN_DEMO=1` | noos-shell : profils VPN fictifs, bouclier VPN dans la barre et son menu ouvert |
| `NOOS_EDITOR_DEMO=1` (+ `NOOS_EDITOR_SEARCH=mot`) | noos-texteditor : dossier et fichiers d'exemple, recherche lancée |
| `NOOS_LOCK_DEMO_CARD=1` | noos-lock : carte de connexion affichée d'emblée (avec `--preview`) ; `noos-lock --preview --screensaver` : écran de veille |
| `NOOS_DISK_DIALOG=format\|free-format\|init\|bench\|bench-run\|smart` | noos-disk : ouvre ce dialogue (avec `noos-disk /dev/sdX`) ; lancer l'aperçu sur l'**hôte** (bus système / udisks absents d'un conteneur) |
| `NOOS_DRIVES_OPEN=1` / `NOOS_AUTOPLAY_DEMO=1` | noos-shell : menu « Retirer en toute sécurité » ouvert / fenêtre « Lecture automatique » du premier disque externe |
| `NOOS_SHADER_CATALOG=<fichier ou URL>` | catalogue d'animations à utiliser (ex. `data/shaders/catalog/index.json` avant de pousser) |
| `NOOS_DEMO_NAME="Alex Martin"` | nom affiché (menu Démarrer, Paramètres) à la place du vrai : captures publiques |
| `NOOS_SETTINGS_SCROLL=px` (+ `NOOS_SETTINGS_SCROLL_DELAY=ms`) | noos-settings : défile la page ouverte (captures), au bout de 1500 ms par défaut (plus quand le contenu vient du réseau) |
| `NOOS_WALLHAVEN_SEARCH=mot` / `NOOS_WALLHAVEN_PAGES=n` / `NOOS_BING_PAGES=2` | noos-settings › Arrière-plan (`noos-settings background`) : recherche Wallhaven préremplie, n pages chargées d'affilée, jours précédents de Bing affichés |
| `NOOS_FILE_MENU_DEMO=x,y` | noos-file ouvre le menu contextuel de la vue en icônes en (x, y) ; y < 0 = depuis le bas |
| `NOOS_FILE_RUBBER_DEMO=x0,y0,x1,y1` | noos-file trace un cadre de sélection (vue en icônes) de (x0, y0) à (x1, y1) et le laisse affiché ; nombre d'éléments sélectionnés sur stderr |
| `NOOS_FILE_SELECT_DEMO=0,1,2` | noos-file sélectionne ces éléments puis rejoue un clic dans le vide (coin inférieur droit) |
| `NOOS_PHOTOVIEW_DEMO=<image>;info;zoom=2.5;next;hide;slideshow` | noos-photoview : ouvre une image, panneau d'infos, zoom, suivante, commandes masquées, diaporama |
| `NOOS_ARCHIVE_DEMO=<archive>\|create` | noos-archive : ouvre une archive / la page de création préremplie |
| `NOOS_FILE_DEMO=<dossier>;details;list;select=0,2;look;zoom=3;sort=size;copy=<source>;menu;mount;trash` | noos-file : ouvre un dossier, volet de détails, vue Détails, sélection, aperçu rapide, zoom, tri, copie suivie |
| `NOOS_VRR_DEMO=gpu:i915\|display\|supported` | noos-settings › Écran / Jeux : simule la cause d'indisponibilité du VRR (écran FreeSync 48–180 Hz fictif) |
| `NOOS_HDR_PREVIEW=roundtrip\|pq` | noos-comp imbriqué : écran simulé compatible HDR ; HDR actif = conversion PQ puis décodage inverse (image identique attendue) / signal PQ brut |
| `NOOS_FIREWALL_DEMO=1\|in\|done` | pare-feu fictif (sans démon) : page Paramètres › Pare-feu (`noos-settings firewall`) et fenêtre d'autorisation `noos-firewall-prompt` (sortie / entrée d'abord / écran « Révoquer ») ; `NOOS_FIREWALL_DIALOG=port\|app` ouvre le dialogue d'ajout |
| `NOOS_BT_DEMO=1\|add\|pair\|pin\|off\|none\|service` | noos-settings › Bluetooth et appareils (`noos-settings bluetooth`) sans BlueZ : appareils fictifs / assistant « Ajouter un appareil » / association avec code à confirmer / code PIN / désactivé / sans adaptateur / service arrêté |
| `NOOS_PRINT_DEMO=1\|printer\|add\|driver\|fail\|none\|missing` | noos-settings › Imprimantes et scanners (`noos-settings printers`) sans CUPS : imprimantes et scanners fictifs / fiche d'imprimante / assistant d'ajout / choix du pilote / échec / rien trouvé / CUPS absent |
| `NOOS_PRINT_OPEN=<file CUPS>` | noos-settings printers : ouvre la fiche d'une vraie imprimante (ex. `EPSON_ET_4800_Series`) |
| `NOOS_PKG_ELEVATE=sudo\|none` | noos-pkg : élévation par `sudo -n` ou aucune (tests), au lieu de pkexec |
| `NOOS_SHARE_DEMO=1\|nopass\|off\|missing` (+ `NOOS_SHARE_QR=1`) | noos-settings › Partage (`noos-settings samba\|ssh\|webdav`) : services, dossiers, connexions, clés fictifs (sans mot de passe / tout désactivé / paquets absents) ; premier QR code ouvert |
| `NOOS_POWER_DEMO=battery\|charging\|low\|critical\|full\|plugged\|desktop` (+ `NOOS_POWER_OPEN=1`) | noos-shell : batterie fictive (ou PC fixe) dans la barre, modes fictifs ; panneau Énergie ouvert après 2 s. noos-installer : portable simulé (sauf `desktop`) |
| `NOOS_GAMING_DEMO=1\|on\|installing` (+ `NOOS_GAMING_DIALOG=enable\|disable\|decky\|switch`) | noos-settings › Jeux (`noos-settings gaming`) : mode jeu Steam désactivé / activé (Decky, démarrage dans Steam, composants installés) / activation en cours, sans rien exécuter ; fenêtre de confirmation ouverte |
| `NOOS_FILE_NET_DEMO=1` | noos-file : appareils, partages et serveurs fictifs dans la page Réseau (`NOOS_FILE_DEMO='network:///;connect'` ou `;auth` : dialogue « Se connecter à un serveur » / fenêtre d'identifiants) |

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

## ISO, VM de test

- **ISO** : `./noos-iso/build.sh` (podman ou docker ; compilation dans `noos-dev` ou un
  conteneur `debian:sid` via `noos-iso/ci-stage.sh`, système live dans un conteneur
  `--privileged`). Sortie : `noos-iso/out/noos-sid-amd64.iso`. `--no-stage` réutilise la
  compilation. Sans Linux : GitHub › Actions › **ISO** › Run workflow (artefact `noos-iso`).
- **VM** (Linux, paquets `qemu-system-x86 ovmf qemu-utils`) : `tools/vm/vm.sh start
  noos-iso/out/noos-sid-amd64.iso uefi`, puis `tools/vm/vm.sh install`, `tools/vm/qmp.py shot
  x.png`, `tools/vm/vm.sh stop`, `tools/vm/vm.sh start "" uefi` (démarrer sur le disque
  installé), `tools/vm/vm.sh login`. OVMF Debian : `/usr/share/OVMF/OVMF_CODE_4M.fd`.
  Lire/modifier le disque de la VM éteinte : `guestfish` (`LIBGUESTFS_BACKEND=direct`,
  `mount-options subvol=@ /dev/sda3 /`).
- **VM sous Windows** (poste d'Axel) : QEMU de qemu.org (`winget install
  SoftwareFreedomConservancy.QEMU`) + « Plateforme de l'hyperviseur Windows » activée (WHPX).
  Mêmes commandes dans Git Bash (`vm.sh` détecte Windows : WHPX `kernel-irqchip=off`, sinon
  la VM se fige ; edk2 de QEMU ; fenêtre SDL ; QMP 127.0.0.1:4444, série 127.0.0.1:4445).
  **Jamais `system_reset` ni redémarrage depuis l'invité** : WHPX plante ou fige l'image ;
  toujours `vm.sh stop` puis `vm.sh start`. GRUB : touches US (qcodes), pas AZERTY. Console
  série : ajouter `console=tty0 console=ttyS0,115200` à la ligne `linux` (GRUB › e), puis
  `tools/vm/serial.py login atest <mdp>` et `tools/vm/serial.py run "commande"`.
- **VM sous Linux** : ISO de la CI (`gh run download <run ISO> -n noos-iso -D noos-iso/out`),
  `vm.sh start <iso> uefi` puis `vm.sh install` (compte « test ») ; console série = socket
  `$XDG_RUNTIME_DIR/noosser.sock` (`serial.py` le prend par défaut) après avoir ajouté
  `console=ttyS0,115200` à grub.cfg par guestfish (VM éteinte). Copier un binaire dans la VM :
  `python3 -m http.server --bind 127.0.0.1` sur l'hôte, `sudo curl http://10.0.2.2:PORT/…`
  dans l'invité. Un clic QMP au même endroit que le précédent peut ne pas être pris.
  L'utilisateur peut voir la fenêtre QEMU : lui demander de ne pas y cliquer pendant un
  parcours scripté (clics à coordonnées fixes).

## Debian sid : pièges et leçons apprises

- **À revalider depuis le passage à Debian** : installation complète en VM (UEFI + BIOS),
  live-boot + Plymouth, GRUB signé de l'ISO (`gcdx64.efi` cherche `/.disk/info` puis
  `/boot/grub/grub.cfg` ; les modules non signés ne se chargent pas avec Secure Boot), noms
  de paquets des listes `noos-iso/packages-*.txt` et de `debian/control` (sid bouge : un
  paquet absent est signalé et ignoré par l'installateur et l'ISO, mais pas par `Depends`).
- **Comptes et groupes** : administrateurs = groupe **`sudo`** (pas `wheel`) : installateur,
  snapper `ALLOW_GROUPS`, `/etc/noos` (groupe en écriture), polkit (règle Debian par défaut).
  greetd tourne sous **`_greetd`**.
- **Langue et clavier** : `/etc/locale.gen` + `locale-gen`, `/etc/default/locale` ; clavier
  dans `/etc/default/keyboard` (console-setup), lu par le compositeur et noos-login
  (`/etc/vconsole.conf` en repli, écrit aussi).
- **Démarrage** : initramfs-tools (`update-initramfs -u -k all`, modules précoces dans
  `/etc/initramfs-tools/modules`, bloc « >>> NOOS »), noyau `/boot/vmlinuz-*` +
  `/boot/initrd.img-*`, `quiet splash`. GRUB : `/boot/grub`, `grub-mkconfig` / `update-grub` ;
  Debian ajoute lui-même `rootflags=subvol=@` ; pas de BLS ni de grubby : noyau par défaut =
  `grub-set-default gnulinux-<version>-advanced-<uuid>` (lu dans grub.cfg, `GRUB_DEFAULT=saved`).
- **UEFI** : GRUB signé exige le dossier `EFI/debian` (`--bootloader-id=debian --no-nvram`),
  copie de secours `--removable`, entrée NVRAM « NOOS » → `\EFI\debian\shimx64.efi`.
- **Chroot d'installation** : `/usr/sbin/policy-rc.d` (exit 101) pendant l'installation, montages
  proc/sys/dev **après** debootstrap (il gère les siens), `DEBIAN_FRONTEND=noninteractive`,
  `--force-confold`, pas de recommandations (`90noos-install`), retirés à la fin. Les crochets
  apt de noos-save ne font rien dans un chroot (`systemd-detect-virt --chroot`).
- **i386** : `dpkg --add-architecture i386` avant le premier `apt-get update` ; Mesa 32 bits
  (`libgl1-mesa-dri:i386`, `mesa-vulkan-drivers:i386`) et `nvidia-driver-libs:i386` si NVIDIA.
- **NVIDIA** : `nvidia-driver` (non-free) compilé par DKMS pendant son installation
  (`linux-headers-amd64`) ; `nvidia-drm.modeset=1`. Codecs : tout est dans `main` (ffmpeg,
  GStreamer ugly/bad/libav, libde265) ; contrib / non-free = firmwares + NVIDIA.
- **apt sans root** : `apt-get update -o Dir::State::Lists=~/.cache/noos/apt/lists -o
  Dir::Cache=… -o Debug::NoLocking=1` fonctionne ; `apt-get -s` aussi. Sortie analysée en
  `LC_ALL=C.UTF-8` (exec.rs l'impose).
- **AppStream** : les identifiants des paquets viennent des fichiers DEP-11 de
  `/var/lib/apt/lists/*_dep11_Components-amd64.yml.gz`, téléchargés seulement si le paquet
  `appstream` est installé (`Depends`).
- **Paquet Debian** : unités systemd activées par postinst (pas par dh_installsystemd) ;
  libAdapta privée dans `/opt/noos/lib` (`dh_shlibdeps -l`, pas de shlibs publics) ; fichiers
  de `/etc` = conffiles automatiques ; sudoers.d mis en 0440 par dh_fixperms.
- **Live** : live-boot (`boot=live components`), squashfs dans `/live/filesystem.squashfs`,
  autologin root sur tty1 → `noos-live-installer`.
- **Applis X11** : `xwayland-satellite` n'existe pas dans Debian → le compositeur lance
  `Xwayland` (Depends `xwayland`) et joue le gestionnaire de fenêtres X11 (`xwayland.rs`).
  Focus clavier = `FocusTarget` (surface Wayland ou fenêtre X11) ; une fenêtre peut ne pas
  avoir de `toplevel()` xdg : jamais de `toplevel().unwrap()`. Menus X11 = fenêtres
  override-redirect (hors barre des tâches et focus). Position X synchronisée à chaque image
  (`sync_x11_geometry`). Electron en Wayland natif : une fenêtre créée cachée
  (`show: false` + `ready-to-show`) peut ne jamais s'afficher (Kudu) ; en X11 elle s'affiche.
- **greetd** lance lui-même `sh -c "… exec <commande>"` : passer la ligne Exec telle quelle,
  jamais un `sh -c` imbriqué. Trousseau : gnome-keyring + `libpam-gnome-keyring`.
  `/etc/pam.d/greetd` (paquet greetd) n'a pas `pam_env envfile=/etc/default/locale` : la
  session arrivait sans `LANG` (Chrome, Electron en anglais) → `noos-session` lit lui-même
  `/etc/default/locale` si `LANG` est vide.
- **greetd sur la console 1** (`vt = 1` + `greetd.service.d/90-noos-vt1.conf`, Conflicts
  getty@tty1), pas la 7 de Debian : la bascule 1 → 7 se perdait au démarrage (fin de Plymouth),
  greetd restait bloqué dans `VT_WAITACTIVE` et l'écran sur « login: » (installation réelle,
  reproduit en VM). postinst corrige les `vt = 7` déjà installés.
- **polkit** : gio ne garde qu'une référence faible sur la connexion au bus système → la
  conserver, sinon polkitd oublie l'agent (« No authentication agent found »). Dans un
  terminal, `pkexec` garde son invite texte ; `--disable-internal-agent` pour tester la fenêtre.
  Assistant Debian : `/usr/lib/polkit-1/polkit-agent-helper-1`, **plus setuid** depuis polkit
  ≥ 126 (sid : 128) : systemd le lance par `/run/polkit/agent-helper.socket` ; l'agent
  (`noos-shell/src/polkit.rs`) y écrit l'utilisateur puis le cookie, sinon « Incorrect
  permissions … (needs to be setuid root) » et aucune élévation ne passe.
- **Initramfs sans busybox** : le `mount` de klibc ne connaît pas `relatime` et le passe au
  système de fichiers → `efivarfs: Unknown parameter 'relatime'` / « mount … Invalid argument »
  (script `00_mount_efivarfs` de mdadm, tiré par udisks2). `busybox` en dépendance. Firmware
  Intel (DMC, GuC) = `firmware-intel-graphics`, séparé de `firmware-misc-nonfree`. L'ISO live
  embarque les firmwares Wi-Fi (installation réseau sur portable).
- **Rendu logiciel (llvmpipe, VM sans 3D)** : attendre la fin du dessin (`glFinish`) avant
  d'afficher, sinon tuiles noires / restes d'images.
- **Verrouillage** (`noos-comp/src/lock.rs`) : tant que le verrou est actif, seules les surfaces
  de verrouillage sont rendues et reçoivent les entrées (aucun raccourci sauf VT). Plus de
  surface vivante → noos-lock relancé. Déclencheurs : Super+L, IPC `lock` / `suspend`,
  `[lock] idle_minutes`. Inactivité (`noos-comp/src/idle.rs`) : voile noir progressif pendant
  `[lock] dim_seconds`, puis `noos-lock` (mode `lock`) ou `noos-lock --screensaver`.
- **Abonnements D-Bus sur le bus système** : garder la connexion (`udisks::watch` la retient),
  sinon GIO la ferme et les signaux n'arrivent plus, sans erreur (Explorateur non rafraîchi).
- **udisks2 depuis Rust (gio)** : construire les arguments avec `Variant::tuple_from_iter`
  (`noos_disk::ops::tup`). Ne jamais tester formatage / partitionnement sur un vrai disque :
  image + `udisksctl loop-setup` puis `noos-disk --selftest /dev/loopN`.
- **Restauration Btrfs** : la racine est montée par son nom (`subvol=@`), donc `snapper
  rollback` est sans effet. `noos-save-helper rollback` renomme `@` en
  `@.avant-restauration-…`, crée `@` depuis l'instantané, déplace `.snapshots`, et choisit le
  noyau le plus récent dont les modules existent dans l'instantané (/boot est séparé).
- **Démarrage sur un instantané** (GRUB, `42_noos_snapshots`) : `rootflags=subvol=@/.snapshots/N/snapshot
  systemd.volatile=overlay noos.snapshot=N` ; `noos-snapshot-boot.service` monte `@/.snapshots`.
- **Démarrage fluide** (`noos-boot-setup`, postinst + installateur) : pilote graphique dans
  l'initramfs, `UseSimpledrm=0` dans `/etc/plymouth/plymouthd.conf`, thème `noos` ;
  `plymouth-quit` garde la dernière image (`--retain-splash`) jusqu'au greeter.
- **GRUB : rectangle noir au lancement d'une entrée** = la console gfxterm ; corrigé par une
  console plein écran sans cadre (theme.txt) et `background_image` (`/etc/grub.d/05_noos_background`).
  Les .pf2 doivent être chargés (`loadfont`) avant `set theme`.
- **CSS du shell** : le style de la barre est chargé en `STYLE_PROVIDER_PRIORITY_USER + 10` ;
  une feuille ajoutée en plus doit être au-dessus (`USER + 20`), sinon ses couleurs sont ignorées.
  `@window_bg_color` du thème NOOS est **translucide** (Mica) : un fond opaque se règle dans
  `style.rs` (couleurs de la palette, ex. `banner_bg`), pas avec `alpha(@window_bg_color, …)`.
- **Transitions CSS GTK** : jamais de `transition` sur `box-shadow` entre deux listes d'ombres de
  longueurs différentes — interpolation invalide (`blur_radius >= 0`) puis **plantage** du shell
  (mode édition, 18/11/2026). `gtk::Fixed` alloue à ses enfants leur taille **minimale** ; un
  `f64::clamp(min, max)` avec `min > max` fait paniquer (taille d'écran encore inconnue).
- **Flou Mica** (`noos-comp/src/blur.rs`) : tout commit d'une surface de la couche
  « background » fait recalculer le flou de tout l'écran sur le processeur (≈ 170 ms sur
  l'UHD 630). Rien qui se rafraîchit ne doit vivre dans le fond d'écran : les widgets du bureau
  sont dans leur propre surface (`noos-desktop-widgets`, couche « bottom »).
- **Applis externes** (`noos-theme/src/thirdparty.rs`) : GNOME règle `button-layout` sur
  `appmenu:close` → NOOS y écrit `[appearance] button_layout`. Couleurs Qt : qt5ct / qt6ct
  (`QT_QPA_PLATFORMTHEME=qt6ct`), barre de titre Qt `QT_WAYLAND_DECORATION=adwaita`.
- **HDR10** (`noos-comp/src/hdr.rs`) : Smithay 65aa86f n'écrit jamais `HDR_OUTPUT_METADATA`,
  `Colorspace` ni `max bpc` → validation atomique séparée (`ALLOW_MODESET`, bloquante) ; le
  noyau garde ces valeurs, les validations de Smithay les reprennent. Le blob fait exactement
  32 octets (`struct hdr_output_metadata`). Bureau SDR → texture → shader PQ (un seul élément :
  pas de scanout direct en HDR). Captures et partage d'écran restent en SDR (éléments d'origine).
- **Pare-feu** (`noos-firewall`) : nftables ne reconnaît pas un exécutable → règles par programme
  appliquées par le démon au verdict, règles de ports = vraies règles nft. File d'entrée **sans**
  `bypass` (démon arrêté = entrées fermées), file de sortie **avec** (réseau intact). Sans fenêtre
  d'autorisation connectée, sorties autorisées après 3 s (`no_agent_action`) : sinon une mise à
  jour du paquet couperait Internet à la session en cours. `nft -c` sans root :
  `unshare -rn nft -c -f -`. Ne jamais tester le démon sur la vraie machine sans
  `noos-firewall check` d'abord ; secours : `sudo /opt/noos/bin/noos-firewall off`.
  Validé en VM (installation complète depuis l'ISO + paquet du dépôt, 17/11/2026) : service au
  démarrage, fenêtre d'autorisation en sortie et en entrée, polkit (accepté / refusé), bouclier,
  révocation, redémarrage du service (agent reconnecté en 2 s). Test d'entrée dans une VM :
  `ip netns add pair` + veth, serveur dans l'hôte, client dans `pair`.
- **Icônes d'applis** : des entrées .desktop pointent vers des XPM (`/usr/share/pixmaps/python3.14.xpm`)
  que GTK 4 n'affiche plus, et le thème cherche aussi dans `/usr/share/pixmaps` (un nom « existe »
  mais mène au XPM) → vérifier le fichier trouvé par `IconTheme::lookup_icon` (`shown_icon`).
  Jamais `gio::AppInfo::executable()` : plante sur une entrée sans `Exec` → `commandline()`.
- **Partage de fichiers** (`noos-share`) : smb.conf garde le fichier Debian, NOOS ajoute un
  bloc final qui rouvre `[global]` / `[homes]` (dernière valeur gagnante) ; `hosts allow` répété
  dans chaque section (les sections déjà lues ne reprennent pas le `[global]` rouvert, vu avec
  `testparm`). `wsdd2` = serveur WSD (unité liée à smbd) ; `wsdd` = client que gvfs lance pour
  découvrir les PC Windows (sans lui : « Failed to spawn the wsdd daemon »). OpenSSH 10 affiche
  `sshd -T` en `Port`, `PasswordAuthentication` (casse). Montages gvfs depuis noos-file :
  arrêter l'émission de `ask-password` / `ask-question` (sinon réponse « non traité »
  immédiate) ; dans `dbus-run-session`, gvfs bloque sur le trousseau. rclone : pas d'`union` à
  un seul membre (même dossier deux fois en `:ro`).
- **auto-cpufreq** (portables, installateur) : en conflit avec power-profiles-daemon (même
  gouverneur) → PPD masqué par `noos-power-helper laptop-setup`. Le paquet Debian ne crée pas
  `/opt/auto-cpufreq` : `auto-cpufreq --force` y écrit `override.pickle` et échoue sans lui ;
  `--force` refuse aussi de marcher si le démon ne tourne pas (l'assistant le démarre). Rien
  d'automatique pour une installation existante (postinst) : commande manuelle dans GUIDE.md.
- **Configuration illisible** : `NoosConfig::save()` garde une copie `noos.toml.illisible`
  avant d'écraser un fichier qui ne se lit plus.

## Reste à faire / idées

- Valider sur une vraie Debian sid : installateur, ISO (Secure Boot), dépôt APT, Logiciels (apt).
- Créer la clé du dépôt APT (`packaging/apt/generate-key.sh`) et le secret `APT_SIGNING_KEY`,
  vérifier que GitHub Pages sert la branche `gh-pages`.
- Plymouth : thème NOOS non vérifié sur vrai matériel (en VM, mode texte faute de 3D).
- Sélecteur d'écran pour le partage d'écran, VRR/HDR sur matériel réel.

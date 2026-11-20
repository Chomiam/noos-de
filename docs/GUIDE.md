# NOOS — guide détaillé

> Version grand public : [README](../README.md). Cette page rassemble le détail de chaque
> composant, la compilation, l'ISO, la configuration (`noos.toml`) et les raccourcis.
> Notes de travail (pièges, outils d'aperçu, variables de test) : [CLAUDE.md](../CLAUDE.md).

## Organisation du dépôt

| Dossier | Rôle |
|---|---|
| `noos-comp/` | **Compositeur** Wayland (Smithay) : DRM/KMS + libinput (session réelle) ou fenêtre imbriquée (winit), gestion des fenêtres façon Windows 11, VRR, IPC |
| `noos-shell/` | Barre des tâches centrée, menu Démarrer, réglages rapides, centre de notifications, zone des applis actives, fond d'écran (layer-shell) |
| `noos-file/` | Explorateur de fichiers : accueil, miniatures, volet de détails, aperçu rapide, glisser-déposer, copies suivies, corbeille |
| `noos-settings/` | Paramètres : personnalisation, barre des tâches, écrans (HDR/VRR), jeux, entrées |
| `noos-software/` | **Logiciels** : vitrine Flathub, recherche multi-sources, mises à jour, dépôts |
| `noos-pkg/` | Bibliothèque + CLI `noos-pkg` : apt et Flatpak, sources APT regroupées, rapprochement AppStream (DEP-11) |
| `noos-firewall/` | **Pare-feu applicatif** : démon `noos-firewalld` (root, nftables + NFQUEUE), CLI `noos-firewall`, bibliothèque partagée (règles, protocole) |
| `noos-firewall-prompt/` | Fenêtre d'autorisation du pare-feu (agent de session, GTK4 + libAdapta) |
| `noos-share/` | **Partage de fichiers** en réseau local : modèle et fichiers générés (Samba, sshd, Avahi, règles du pare-feu), état des services, `noos-share-helper` (pkexec), `noos-webdav` (service utilisateur, rclone) |
| `noos-gaming/` | **Mode jeu Steam** : session gamescope (`noos-steam-session`), bascule bureau ↔ Steam (`noos-session-switch`), `noos-gaming-helper` (pkexec), `steamos-session-select` et autres outils attendus par Steam |
| `noos-keys/` | **Porte-clés** : mots de passe du Secret Service (gnome-keyring, oo7), trousseaux, clés SSH |
| `noos-term/` | **Terminal** (VTE) : onglets, menu clic droit, thèmes, taille du texte, barre latérale d'alias (ex. `claude`) |
| `noos-snip/` | **Outil Capture d'écran** (Win+Maj+S) : rectangle, fenêtre, plein écran, forme libre, annotation |
| `noos-login/` | **Écran de connexion** (greeter greetd) façon Windows 11 : verrouillage avec horloge, sessions en cartes, comptes, accessibilité (grand texte, contraste, clavier visuel, Orca) |
| `noos-installer/` | **Installateur** plein écran : carte des fuseaux, clavier, compte (mot de passe fort), disque Btrfs (@, @home, swap), installation réseau Debian sid (debootstrap) + contrib / non-free, i386, GRUB |
| `noos-plymouth/` | Thème de démarrage Plymouth (logo NOOS, faisceau animé sur le ∞) |
| `noos-iso/` | Construction de l'ISO live d'installation Debian sid (BIOS + UEFI/Secure Boot) |
| `debian/` | Paquet Debian `noos` (dpkg-buildpackage, debhelper) |
| `packaging/apt/` | Construction du `.deb` et du dépôt APT signé (GitHub Pages, branche `gh-pages`) |
| `.github/workflows/` | CI : compilation + tests + `.deb` sur Debian sid, publication APT, ISO à la demande |
| `noos-theme/` | Moteur de thème : palettes, accent, génération du thème libAdapta, rechargement à chaud |
| `noos-core/` | Configuration (`~/.config/noos/noos.toml`) et protocole IPC partagés |
| `noos-adapta/` | Bindings Rust de libAdapta (`libadapta-sys` + `libadapta`) |
| `noos-session/` | Entrée GDM, lanceur de session, portails XDG, fichiers .desktop |
| `third_party/libadapta/` | Sources de libAdapta 1.5 (commit dans `.noos-upstream-commit`) |
| `data/` | Logo, bouton Démarrer, fond d'écran |
| `scripts/` | `install-build-deps.sh`, `dev.sh`, `install-user.sh`, `build-libadapta.sh`, `noos-ipc.py` |

## Environnement de développement (Debian sid)

Sur une Debian sid (ou dans un conteneur distrobox **`noos-dev`** Debian sid si l'hôte est
une autre distribution) :

```bash
distrobox create -n noos-dev -i docker.io/library/debian:sid   # facultatif
./scripts/install-build-deps.sh   # bibliothèques de compilation + Rust (rustup)
./scripts/build-libadapta.sh      # une fois : compile libAdapta dans target/
cargo build
./scripts/dev.sh                  # NOOS dans une fenêtre de votre session actuelle
```

La CI GitHub (`.github/workflows/build.yml`) compile tout, lance les tests et construit le
paquet `.deb` dans un conteneur `debian:sid` à chaque envoi.

Dans la fenêtre imbriquée, la touche Super est souvent interceptée par GNOME. Pour piloter NOOS :

```bash
./scripts/noos-ipc.py toggle-start-menu
./scripts/noos-ipc.py get-windows
./scripts/noos-ipc.py screenshot path=/tmp/noos.png
```

## Installer la vraie session

1. Compiler et installer dans `~/.local/opt/noos`, sans sudo :
   ```bash
   ./scripts/install-user.sh
   ```
2. Ajouter « NOOS » à l'écran de connexion. À faire une seule fois, **sur l'hôte** (pas dans un conteneur) :
   ```bash
   sudo ./noos-session/install-session.sh
   ```
   Le script écrit uniquement dans `/usr/local` (rien n'est mélangé aux paquets Debian). Inutile
   si le paquet `noos` est installé.
3. Se déconnecter, choisir **NOOS** dans l'écran de connexion, puis se connecter.

- **Quitter :** Ctrl+Alt+Retour arrière (retour à GDM).
- **Journal :** `~/.local/state/noos/session.log`.
- **Mettre à jour :** relancer l'étape 1 suffit.

### Intégration à la session (portails, partage d'écran)

Au démarrage, NOOS active `noos-session.target`, lié à `graphical-session.target`. Sans cette cible,
`xdg-desktop-portal` refuse de démarrer, et les Flatpak perdent le thème, les icônes, le sélecteur de
fichiers et l'ouverture de liens entre applications.

Tout est écrit dans `$XDG_RUNTIME_DIR`, donc effacé à la déconnexion. La session GNOME n'est pas touchée.

- **Portails :** `gtk` pour presque tout, `wlr` pour le partage d'écran (paquet Debian
  `xdg-desktop-portal-wlr`, lancé par NOOS lui-même).
- **Capture :** le compositeur implémente `ext-image-copy-capture-v1` (shm et dmabuf).
- **Accès :** les Flatpak n'ont pas d'accès direct à la capture. Ils passent par le portail, qui
  demande l'accord de l'utilisateur.

## ISO d'installation (noos-iso)

Construit une ISO live Debian sid qui démarre sur l'installateur NOOS (rien n'est installé
sur l'hôte : compilation dans le conteneur noos-dev ou un conteneur `debian:sid`, système live
dans un conteneur privilégié podman / docker) :

```bash
./noos-iso/build.sh          # → noos-iso/out/noos-sid-amd64.iso
```

Sans machine Linux : onglet **Actions › ISO › Run workflow** sur GitHub (artefact `noos-iso`).

- **Démarrage** : GRUB en BIOS (El Torito) et UEFI (shim + GRUB « CD » signés de Debian,
  Secure Boot ; repère `/.disk/info`), ISO hybride (graver sur clé USB avec `dd`). Live monté
  par live-boot (`boot=live`). Entrée « journal sur port série » pour le dépannage.
- **Live** : connexion automatique de root sur tty1 → `noos-comp --drm --kiosk noos-installer`.
  Journaux : `/tmp/noos-live.log`, `/tmp/noos-install.log`.
- **Système installé** : debootstrap de Debian sid + paquets de `noos-iso/packages-*.txt`
  (contrib / non-free au choix, architecture i386 activée pour Steam et Wine), paquet `noos`
  depuis le dépôt APT de NOOS (sinon copie de l'ISO), greetd (compte `_greetd`) + noos-login,
  Plymouth NOOS (initramfs-tools), journal copié dans `/var/log/noos-install.log`.
- À revalider dans QEMU (UEFI et BIOS) depuis le passage à Debian.

## Explorateur de fichiers (noos-file)

- **Accueil** façon Windows 11 : accès rapide aux dossiers, disques avec espace libre, fichiers
  récents (avec miniatures) ; « Dossier personnel » à part dans la barre latérale.
- **Barre de commandes** : Nouveau (dossier, document texte), couper / copier / coller, renommer,
  supprimer, Trier (nom, date, taille, type, ordre décroissant), Affichage (icônes / détails,
  zoom, fichiers cachés, volet de détails).
- **Miniatures** des images (gdk-pixbuf) et des vidéos (`ffmpegthumbnailer` s'il est installé),
  dans le cache partagé `~/.cache/thumbnails/large` ; zoom des icônes (Ctrl+molette, Ctrl+±).
- **Volet de détails** (Alt+P) : aperçu, taille, dimensions des images, date, emplacement,
  contenu des dossiers, début des fichiers texte. **Aperçu rapide** : Espace.
- **Glisser-déposer** vers un dossier, la vue, la barre latérale ou le fil d'Ariane (déplacer ;
  Ctrl : copier ; entre deux disques : copie). Déposer sur la Corbeille : mettre à la corbeille.
- **Copies et déplacements suivis** : une carte par opération (pourcentage, éléments, octets,
  vitesse, temps restant, fichier en cours, Annuler), sans bloquer l'interface.
- **Clic droit** comme sous Windows : l'élément sous le pointeur est sélectionné (seul) et son
  menu s'ouvre (Ouvrir, Ouvrir avec, Compresser, Renommer…) ; dans une sélection multiple qui
  le contient, elle est gardée ; dans le vide, menu du dossier (Nouveau, Coller, Trier…).
- **Cadre de sélection en direct** (`src/rubberband.rs`, icônes et détails) : un élément est
  surligné dès qu'il entre dans le cadre et désélectionné si le cadre rétrécit ; défilement
  automatique près du haut et du bas ; Ctrl ou Maj ajoute à la sélection existante.
- « Ouvrir avec », corbeille complète (restaurer, supprimer définitivement, vider), Suppr /
  Maj+Suppr, F2, Ctrl+A, Ctrl+F, Ctrl+1 / Ctrl+2, page « dossier vide ».
- **Réseau** (`src/network.rs`, barre latérale, adresse `network:///`) : appareils découverts par
  gvfs (`network:///` : PC Windows par WSD — programme `wsdd` du paquet du même nom —, Mac, NAS,
  SFTP, WebDAV annoncés en mDNS), partages ouverts (bouton Déconnecter), serveurs récents
  (`~/.local/share/noos/recent-servers`), lien vers les réglages de partage de ce PC.
  **Se connecter à un serveur** : `smb://`, `sftp://`, `dav(s)://`, `ftp://`, ou `\\PC\partage`
  tapé comme sous Windows (converti en `smb://PC/partage`) ; aussi depuis la barre d'adresse.
  Un emplacement distant est **monté avant d'être affiché** (`gio::MountOperation`) : fenêtre
  d'identifiants libAdapta (compte, domaine, mot de passe, invité si permis, **« Retenir le mot
  de passe »** = `PasswordSave::Permanently`, enregistré par gvfs dans le trousseau) et questions
  de gvfs (certificat inconnu : empreinte SHA-1). Piège : le gestionnaire par défaut de
  `GMountOperation` répond « non traité » aussitôt ; nos gestionnaires arrêtent l'émission du
  signal (`stop_signal_emission_by_name`). `ask-question` n'est pas généré par gtk-rs :
  `connect_local`. Dans un `dbus-run-session`, gvfs attend un trousseau verrouillé (pas de
  fenêtre) : tester les montages dans une vraie session.

## Photos (noos-photoview)

Visionneuse d'images, application par défaut des images sur NOOS :

- affichage immersif : fond « ambilight » (l'image floutée derrière elle), commandes qui
  s'effacent sans mouvement de souris ; plein écran (F11), diaporama (F5) ;
- zoom fluide centré sur le pointeur (molette, pincement, + / −), double-clic 100 % ↔ ajusté,
  déplacement à la souris ; rotation (R / Maj+R) et miroir (M) à l'affichage ;
- images du dossier (← / →), bande de miniatures (cache partagé avec l'Explorateur) ;
- informations (I) : dimensions, taille, format, EXIF (appareil, objectif, exposition,
  ouverture, ISO, focale, flash), position GPS avec lien vers OpenStreetMap ;
- copier l'image, définir comme fond d'écran, ouvrir avec, afficher dans l'Explorateur,
  corbeille, glisser-déposer pour ouvrir.

## Applications par défaut

Paramètres › Applications › Applications par défaut : navigateur, courriel, images, musique,
vidéos, PDF, texte, archives, dossiers (appliqué à tous les types de la catégorie), choix par
type de fichier avec recherche, et réinitialisation. Les choix vont dans `~/.config/mimeapps.list`.

## Sauvegarder et Restaurer (noos-save)

Instantanés Btrfs du système (snapper, configuration `root`) :

- **avant et après chaque opération apt** (crochets `DPkg::Pre-Invoke` / `Post-Invoke` de
  `/etc/apt/apt.conf.d/80noos-save`) ;
- **manuels**, jamais supprimés automatiquement, et **réguliers** (horaires, désactivés par
  défaut) avec un nombre conservé par période ;
- **restauration au prochain redémarrage** : l'état actuel est gardé (`@.avant-restauration-…`)
  et la restauration s'annule tant qu'on n'a pas redémarré ; le noyau par défaut est choisi
  pour correspondre à l'instantané (/boot est séparé ; `grub-set-default`, `GRUB_DEFAULT=saved`) ;
- **menu de démarrage « Restaurer depuis un instantané »** (`/etc/grub.d/42_noos_snapshots`,
  régénéré par `noos-snapshots-grub.path` à chaque instantané) : date, heure, nom et origine de
  chaque instantané ; il démarre en lecture seule avec une couche en mémoire
  (`systemd.volatile=overlay`, `noos.snapshot=N`), et l'application propose ensuite de le garder ;
- l'application sépare **manuels** et **automatiques** (apt, réguliers) par des badges et filtres,
  groupe par jour, détaille chaque instantané (noyau, paquets, nettoyage, présence au démarrage),
  compare paquets et fichiers (`dpkg-query --admindir`, `snapper status`), ouvre ses fichiers, renomme et marque
  « à conserver ».

Les dossiers personnels (`@home`) ne sont jamais concernés. Les opérations privilégiées passent
par `/opt/noos/libexec/noos-save-helper` (polkit `org.noos.save.helper`).

## Éditeur de texte (noos-texteditor)

Éditeur de texte et de code (GTK4 + libAdapta + GtkSourceView 5) : coloration de plus de 160
langages détectés par le nom et le contenu, numéros de ligne, mini-carte, onglets, panneau de
dossier, recherche et remplacement (casse, mot entier, expressions régulières, compteur),
autocomplétion des mots, commenter / dupliquer / déplacer / trier les lignes, casse, aller à la
ligne, encodage et fins de ligne conservés, rechargement si un autre programme modifie le
fichier, enregistrement automatique (option), session restaurée, thèmes NOOS clair / sombre.

## Archives (noos-archive)

Parcourir, extraire, créer, tester, modifier et convertir des archives de plus de 60 formats :
7z, zip et dérivés (jar, apk, epub, cbz…), rar (dont RAR5), tar.gz / .xz / .zst / .bz2 / .lz4 /
.lz, iso, dmg, wim, cab, msi, deb, rpm, cpio, squashfs, images de disques virtuels, arj, lzh,
StuffIt, ALZ… (moteurs : 7-Zip, libarchive, unar). Création : 7z (AES-256, noms chiffrés),
zip, tar.zst, tar.xz, tar.gz, tar.bz2, tar.lz4, tar.lz, tar, wim, image ISO ; niveau de
compression, volumes (CD, DVD, FAT32…), archive solide, exclusions. Progression avec vitesse
et temps restant, annulation, notification ; fichiers ouverts d'un double-clic depuis l'archive.

Explorateur : clic droit → Extraire ici, Extraire vers…, Compresser (ZIP, 7z, plus d'options),
Monter / Démonter une image disque. Dans une image montée, un bandeau propose de la démonter ;
supprimer une image encore montée affiche une alerte (« Démonter et continuer »).

## Écran de verrouillage (noos-lock)

Verrouillage de session (`ext-session-lock-v1`) : Super+L, menu Démarrer, inactivité, avant la
mise en veille. Animation en plein écran, grande horloge, mot de passe vérifié par PAM. Si
l'écran de verrouillage s'arrête, la session reste verrouillée et il est relancé.

Paramètres › Personnalisation › Écran de verrouillage : aperçu animé ; à l'inactivité,
**verrouiller ou seulement afficher l'écran de veille** (sans mot de passe), **assombrissement
progressif** de l'écran juste avant (annulé par la souris), **extinction de l'écran** ;
assombrissement de l'animation, images par seconde, qualité de rendu ; **animations NOOS**
intégrées (Aurore, Plasma, Nébuleuse, Infini, Vagues) ; **catalogue NOOS** (hébergé dans ce
dépôt, `data/shaders/catalog/`, sans compte ni clé, licence CC0) avec des clins d'œil aux
écrans de veille de Windows 95 / 98 (Champ d'étoiles, Mystique, Tuyauterie, Labyrinthe, Logos
volants, Courbes) et des créations (Horizon rétro, Lampe à lave, Vitrail, Tunnel, Lucioles,
Lumières de la ville) ; **bibliothèque** locale ; **import** d'un fichier ou de code collé
(format Shadertoy `mainImage`, ou GLSL Sandbox converti automatiquement).

## Bureaux virtuels, plusieurs écrans

**Bureaux virtuels** comme Windows : Super+Ctrl+← / → pour changer (glissement animé, nom du
bureau affiché), Super+Ctrl+D pour en créer un, Super+Ctrl+F4 pour le fermer,
Super+Ctrl+Maj+← / → pour emporter la fenêtre active. **Vue des tâches** (Super+Tab ou bouton
de la barre, la molette change de bureau) : fenêtres du bureau actif, miniatures de chaque
bureau, glisser une fenêtre sur un bureau pour l'y envoyer, renommer (double-clic), fermer,
ajouter. Menu des icônes de la barre : déplacer vers un bureau, afficher sur tous les bureaux.
Paramètres › **Bureaux virtuels** : liste (renommer, réordonner, supprimer), barre des tâches
limitée au bureau actif ou non, navigation circulaire, animation, indicateur, raccourcis.

**Plusieurs écrans** : Paramètres › Écran dispose les moniteurs à la souris (ils se collent
bord à bord), choisit l'écran principal ; Super+Maj+← / → envoie une fenêtre sur l'écran
voisin (en gardant son ancrage) ; barre des tâches sur tous les écrans ou seulement le
principal, chacune avec les fenêtres de son écran si on le souhaite.

## Mode édition du bureau

Clic droit sur le bureau (« Modifier le bureau » ou « Ajouter des widgets ») ou dans le vide de
la barre, Paramètres › Personnalisation › Barre des tâches › « Modifier la disposition ». Le
bureau s'assombrit et montre une grille de points ; une barre d'outils apparaît en haut (en bas
si la barre des tâches est en haut) avec, dessous, une ligne d'aide. Échap, Entrée ou
« Terminé » pour sortir. La première fois, une **visite guidée** en six étapes présente le mode
(bouton **?** pour la revoir). Plan détaillé et idées pour la suite : `docs/plans/mode-edition.md`.

- **Annuler / rétablir** : tout est annulable — déplacements, retraits, bord, barre d'état,
  coin des notifications, widgets (contenu d'une note compris), modèles. Ctrl+Z, Ctrl+Maj+Z ou
  Ctrl+Y, boutons ↶ ↷ (leur info-bulle dit ce qui sera annulé), et « Annuler » dans le message
  qui suit chaque action.
- **Barres** : dans la barre, chaque élément est entouré de pointillés ; on le glisse vers une
  autre zone (gauche, centre, droite de la barre des tâches ou de la barre d'état) — un **trait
  d'accent** montre où il s'insérera. Le « × » apparaît au survol ; clic droit sur un élément :
  « Vers la barre d'état · droite »… ou « Retirer de la barre ». Le panneau **Barres** choisit
  le bord de la barre des tâches sur des **schémas d'écran**, centre ou aligne à gauche les
  icônes, affiche la **barre d'état** et remet les **éléments retirés**.
- **Barre d'état** (ancienne « barre fine », section `[statusbar]` de `noos.toml`, l'ancienne
  `[topbar]` est relue) : seconde barre au bord opposé à la barre des tâches. Elle montre les
  bureaux virtuels (clic ou molette pour changer, « + » pour en créer) et tout ce qu'on y glisse :
  horloge, réseau, notifications… Le nom dit ce qu'elle montre, où qu'elle soit (en bas quand la
  barre des tâches est en haut).
- **Mises en page** : six modèles avec miniature — Windows 11, Classique (Démarrer à gauche),
  Barre d'état (façon macOS), GNOME (horloge au centre en haut), Verticale (barre des tâches à
  gauche), Épurée. Les modèles ne touchent pas aux widgets. « Mes mises en page » enregistre la
  disposition actuelle **avec ses widgets** (`~/.config/noos/layouts/<nom>.toml`) pour la
  réappliquer d'un clic ; **Exporter… / Importer…** un fichier `.toml` à partager ;
  « Réinitialiser » revient à la disposition d'origine des barres (annulable).
- **Notifications** : le cadre « Les notifications apparaîtront ici » se glisse vers un coin
  (ou flèches ↖ ↗ ↙ ↘).
- Bouton « Afficher le bureau » de la barre d'outils : réduit les fenêtres qui cachent les
  widgets.
- **Super+W** ouvre et ferme le mode édition (la touche des widgets de Windows 11).
- **Au clavier** : Tab / Maj+Tab choisit un widget (un clic aussi) ; flèches = une case de la
  grille (Maj : au pixel) ; + / − = taille ; Entrée = réglages ; Suppr = retirer ;
  Ctrl+Maj+← / → = écran voisin ; Échap = ne plus rien choisir, puis quitter. Les appuis répétés
  ne font qu'une étape d'annulation.

### Widgets du bureau

**Galerie** (bouton Widgets, touche W, Ctrl+F pour chercher) : une carte illustrée par widget,
recherche, catégories (Heure et dates, Infos, Multimédia, Pratique, Système). On **glisse une
carte sur le bureau** — sur n'importe quel écran — ou on clique pour la poser au premier endroit
libre (Entrée dans la recherche ajoute le premier résultat). Les widgets apparaissent avec une
petite animation.

En mode édition, chaque widget a une **barre de titre** au-dessus de lui (nom, ⚙ réglages,
× retirer) et une **poignée de taille** en bas à droite (glisser : petite, moyenne, grande). Il se
déplace en le saisissant n'importe où, **aimanté** à une grille de 16 px, aux marges et au centre
de l'écran (hors barres) et aux autres widgets (bords et centres alignés, côte à côte avec un
écart régulier) ; des **repères pointillés** montrent l'alignement trouvé. Alt maintenu :
déplacement libre. Un widget reste toujours entièrement à l'écran.

**Réglages** (⚙, ou clic droit sur le widget hors mode édition) : taille, **fond** (verre,
opaque, aucun), **écran** (plusieurs écrans), réglages propres au widget, Dupliquer, Retirer.

Les **trois tailles** ne font pas que grossir le widget : chacune a sa disposition (captures :
`docs/plans/captures-mode-edition/tailles-*.jpg`). Comme sur Windows 11, tous les widgets d'une
même taille ont **la même largeur** : petite 172 px, moyenne et grande 360 px (la grande gagne en
hauteur) ; deux petits côte à côte, avec l'écart de l'aimantation (16 px), font la largeur d'un
moyen, et les widgets s'alignent en colonnes (`grille.jpg`). La barre de titre d'un widget (nom,
⚙, ×) n'apparaît qu'au survol, pendant un déplacement ou quand ses réglages sont ouverts.

| Widget | Petite | Moyenne | Grande | Réglages |
|---|---|---|---|---|
| Horloge | heure + date courte (« Mer. 18 nov. ») ; cadran seul | heure + date | date avec l'année, semaine et jour de l'année, avancement de la journée ; cadran chiffré + heure numérique | numérique / **analogique** (trotteuse à la couleur d'accent), secondes, date |
| Horloges du monde | liste compacte | petit cadran clair le jour, sombre la nuit, heure, « Demain, +8 h » | + **barre jour / nuit de 24 h** par ville | villes (fuseaux du système, noms français des grandes villes), ordre, retrait |
| Calendrier | tuile **« aujourd'hui »** (jour, quantième, mois, prochain férié) | grille du mois façon Windows 11, jours fériés en couleur, prochain férié | grille avec numéros de semaine + liste **« À venir »** (4 prochains fériés) | numéros de semaine, prochain férié ; flèches, molette, « Aujourd'hui » |
| Compte à rebours | anneau + titre | anneau (jours) + titre, date, temps restant | anneau (**part écoulée**) + tuiles jours / heures / minutes | nom, date (calendrier) et heure |
| Météo | **tuile** : température, icône, ville, min / max | conditions, ressenti, min / max, humidité, vent | + prévisions sur 5 jours | ville (recherche en français), °C / °F |
| Lecteur multimédia | pochette, titre, lecture / pause | + lecteur, progression, précédent / suivant | **carte d'album** : grande pochette, texte et commandes centrés | — |
| Cadre photo | l'image seule | + nom au survol | + **barre** nom, date, ‹ › | dossier, intervalle (1 min à 1 h), ordre aléatoire |
| Note | trois tailles de papier ; texte indicatif « Écrivez ici… » tant qu'elle est vide | | | 5 couleurs |
| Dossiers | les icônes seules | icônes et noms | **liste** avec le nombre d'éléments | ajout / retrait de dossiers |
| Batterie | jauge ronde, pourcentage, état en un mot | batterie dessinée (verte en charge, orange / rouge quand elle faiblit), autonomie, mode d'alimentation | + santé, consommation, cycles, modèle, bouton « Modes d'alimentation… » (panneau Énergie) ; sur un PC fixe : jauge et nom du mode | — |
| Ressources | **4 jauges rondes** (processeur, graphique, mémoire, disque) | barre compacte (comme dans la barre des tâches) | une **carte par composant** : matériel, valeur, jauge, détail (espace libre, débits, température), réseau | — |

Météo : [Open-Meteo](https://open-meteo.com) (sans compte ni clé), fond dégradé selon le temps
et le jour / la nuit, actualisée toutes les 30 min, dernière réponse gardée pour le hors-ligne
(`~/.cache/noos/weather/`) ; seule la ville choisie est envoyée au service. Lecteur : MPRIS
(Firefox, Spotify, VLC, mpv…), le lecteur qui joue a la priorité. Cadre photo : clic = ouvrir
l'image, molette = suivante.

En mode édition, la barre d'outils se place **en haut ou en bas, du côté où elle cache le moins
de widgets** (en bas si la barre des tâches est en haut). Le panneau de réglages d'un widget
s'ouvre vers le haut ou vers le bas selon la place (hauteur limitée, défilement). Un widget
ajouté par un clic va au premier endroit libre, barres de titre, barre d'outils et cadre des
notifications compris ; sur un bureau plein, là où il recouvre le moins.

Les widgets vivent dans leur propre surface (`noos-desktop-widgets`, couche « bottom »), jamais
dans le fond d'écran (le flou Mica serait recalculé à chaque image). Configuration :
`[[desktop_widget]]` (`id`, `kind`, `output`, `x` / `y` en fraction de l'écran, `size`, `style`,
`text`, table `options`), les anciens fichiers restent valides.

## Applications actives (zone de notification)

Les icônes des applications qui tournent en arrière-plan (protocole StatusNotifierItem :
Steam, Discord, Nextcloud…) sont regroupées derrière la flèche « ^ » de la barre, comme sur
Windows 11, ou toutes visibles dans la barre (Paramètres › Barre des tâches,
`[taskbar] tray_expanded`). Code : `noos-shell/src/tray.rs`, bulle dans `taskbar.rs`.

- La bulle **s'adapte au nombre d'icônes** : une ligne jusqu'à 3, puis une grille presque
  carrée (4 → 2 × 2, 6 → 3 × 2, 9 → 3 × 3), 5 colonnes au plus.
- L'ouvrir **ferme les autres panneaux** (Énergie, son, réseau, réglages rapides,
  notifications, recherche) : ce sont des surfaces à part, posées au-dessus de la barre, qui
  cachaient la bulle.
- Clic : action de l'application ; clic droit : son menu ; clic du milieu : action secondaire.

## Recherche (barre des tâches et menu Démarrer)

La pastille « Rechercher » de la barre est un **vrai champ de saisie**, comme sous Windows 11 :
on clique et on tape tout de suite dedans. Les résultats s'affichent en direct dans un panneau
collé à la barre (au-dessus du dock en bas, au-dessous d'une barre en haut, y compris la barre
fine), centré sur la pastille :

- **Calcul** : `2+3*4`, `(12+3)*4,5`, `2^10`, `12 × 3`, `10 ÷ 4` (virgule ou point décimal) ;
  Entrée copie le résultat ;
- **Applications** (.desktop : nom, puis début des mots de la description et des mots-clés) ;
- **Paramètres** : pages de `noos-settings` (Wi-Fi, Pare-feu, Arrière-plan…), ouvertes par
  `noos-settings <page>` ;
- **Fichiers et dossiers** : fichiers récents (`recently-used.xbel`) puis dossier personnel
  (5 niveaux, sans les dossiers cachés ni `node_modules` / `target`…), indexé en arrière-plan
  au plus toutes les 2 minutes. Champ vide : fichiers récents.

Clavier : ↑ ↓ (et Page préc. / suiv.) pour choisir, Entrée pour ouvrir, Échap pour fermer (le
clavier revient à la fenêtre d'avant). Un clic ailleurs ferme aussi la recherche ; le champ est
alors vidé. Les accents ne comptent pas (« parametres » trouve « Paramètres »).

**Super** ouvre le menu Démarrer, dont le champ a le clavier : on tape aussitôt, avec les mêmes
résultats et les mêmes touches. Sur un dock vertical (gauche / droite), la pastille n'est
qu'une loupe et ouvre cette recherche du menu Démarrer.

Fonctionnement (`noos-shell/src/search.rs`) : la barre est en `KeyboardMode::None` (un clic sur
une icône ne prend pas le clavier à la fenêtre active). Le pointeur au-dessus de la pastille la
passe en `OnDemand`, si bien que le clic suivant lui donne le clavier (`focus_layer_under` du
compositeur, journal « clic : clavier donné au panneau ») ; la recherche finie, elle repasse en
`None`. Le panneau de résultats (`noos-search`) n'a jamais le clavier : le champ de la barre
pilote sa sélection. Si la barre n'a pas reçu le clavier 600 ms après le clic (autre
compositeur), la recherche se fait dans le menu Démarrer.

Démo sans clic : `NOOS_SEARCH_DEMO=texte` (saisie simulée dans la pastille du dock, une lettre
toutes les 80 ms), `NOOS_SEARCH_DEMO='texte;down;down'` (flèches), `NOOS_SEARCH_DEMO=start:texte`
(dans le menu Démarrer) :

```bash
tools/preview/shell_shot.sh recherche.png 6 NOOS_SEARCH_DEMO='(12+3)*4,5'
```

## Mises à jour

Un service de la session (`noos-update-check.timer`) recherche les mises à jour en
arrière-plan, **sans mot de passe** et à basse priorité, toutes les 2 heures à une fois par
semaine (Paramètres › Mises à jour : fréquence, interrupteur, « Vérifier maintenant », liste
de ce qui est disponible). Quand il y en a, une icône à deux flèches avec une **pastille
orange** apparaît dans la barre des tâches et ouvre directement la page Mises à jour de
Logiciels, qui a son bouton « Rechercher des mises à jour ». Rien n'est installé sans accord.

Les paquets Debian sont comparés sans droits root : `apt-get update` dans
`~/.cache/noos/apt` (toutes les 6 heures au plus, ou sur demande), puis simulation
`apt-get -s full-upgrade`. Le paquet `noos` lui-même vient du **dépôt APT de NOOS** :

- `.github/workflows/build.yml` construit le `.deb` (`packaging/apt/build-deb.sh`, version
  `0.1.0~git<date>.<commit>`) à chaque envoi ; sur `main`, `packaging/apt/update-repo.sh`
  l'ajoute à la branche `gh-pages` (5 dernières versions, index `apt-ftparchive`, `InRelease`
  signé par la clé du secret `APT_SIGNING_KEY`) ;
- GitHub Pages sert https://chomiam.github.io/noos-de/ ; le paquet installe
  `/etc/apt/sources.list.d/noos.sources` et `/usr/share/keyrings/noos-archive-keyring.asc` ;
- clé de signature : `packaging/apt/generate-key.sh` (une fois), clé publique committée dans
  `packaging/apt/noos-archive-keyring.asc`.

## Énergie : batterie et modes d'alimentation

Élément `power` de la barre des tâches (zone de droite, après réseau / son / réglages ;
déplaçable et retirable en mode édition, interrupteur « Énergie » dans Paramètres ›
Personnalisation › Barre des tâches, `[taskbar] show_power`). Code : `noos-shell/src/power.rs`,
logique partagée `noos-core/src/power.rs` (tests unitaires).

- **Portable** : batterie **dessinée** (cairo) selon le niveau et l'état — remplissage
  proportionnel, vert avec un éclair en charge, prise en badge quand elle est branchée sans
  charger, orange sous 20 %, rouge sous 10 % ; le niveau glisse en douceur quand il change.
  Info-bulle : « Batterie : 78 % · 2 h 15 restantes » ou « Pleine charge dans 40 min ».
  Notification « Batterie presque vide » à 10 % (une fois par décharge).
  Source : UPower par D-Bus (`DisplayDevice` : `Percentage`, `State`, `TimeToEmpty`,
  `TimeToFull`, `EnergyRate`), mis à jour par le signal `PropertiesChanged` (pas de boucle) ;
  la connexion au bus système est gardée (sinon GIO la ferme et les signaux se perdent).
  Sans UPower : lecture de `/sys/class/power_supply/BAT*` toutes les 60 s.
- **PC fixe** : jauge dont l'aiguille montre le mode actif (vert = Économie, neutre =
  Équilibré, orange = Performances), animée quand le mode change, y compris ailleurs
  (Paramètres). Masquée s'il n'y a ni batterie ni gestionnaire de modes.
- **Clic** : panneau dans le style des réglages rapides — pourcentage en grand, barre,
  autonomie, santé de la batterie (`Capacity` d'UPower : « 91 % · bonne »), consommation en
  watts, cycles, modèle ; trois tuiles Économie / Équilibré / Performances (avec l'avertissement
  `PerformanceDegraded` de power-profiles-daemon), raccourci « Paramètres d'alimentation »
  (`noos-settings power`). **Molette** sur l'icône : mode voisin.

Gestionnaires de modes, essayés dans l'ordre :

1. **power-profiles-daemon** (`org.freedesktop.UPower.PowerProfiles`, ancien nom
   `net.hadess.PowerProfiles`) : propriété `ActiveProfile` (écriture autorisée par polkit à la
   session active), suivie par `PropertiesChanged`.
2. **auto-cpufreq** (paquet `auto-cpufreq` de sid, installé sur les **portables** par
   l'installateur) : les deux démons se disputent le gouverneur du processeur, donc sur
   portable power-profiles-daemon est **masqué** et auto-cpufreq activé
   (`/usr/libexec/noos-power-helper laptop-setup`, `desktop-setup` pour revenir en arrière).
   Le menu pilote auto-cpufreq par `pkexec noos-power-helper profile <mode>` (action polkit
   `org.noos.power.helper`, sans mot de passe pour la session active, comme
   power-profiles-daemon) : Économie = `auto-cpufreq --force=powersave`, Équilibré =
   `--force=reset` (réglage automatique selon la charge et le secteur), Performances =
   `--force=performance`. Le choix forcé est lu dans `/opt/auto-cpufreq/override.pickle`
   (dossier surveillé) ; le paquet Debian ne crée pas `/opt/auto-cpufreq`, l'assistant le fait
   (sans lui, `--force` échoue). Paramètres › Alimentation utilise le même assistant.

Installation existante (portable) : rien n'est fait automatiquement par le paquet (le
postinst ne peut pas installer d'autre paquet, et masquer power-profiles-daemon ne doit pas se
faire en silence). À la main :

```bash
sudo apt install auto-cpufreq
sudo /usr/libexec/noos-power-helper laptop-setup
```

Détection d'un portable (installateur, `noos_core::power::is_laptop`) : batterie système dans
`/sys/class/power_supply` (pas celles des souris : `scope=Device`), ou châssis portable
(`/sys/class/dmi/id/chassis_type` 8, 9, 10, 14, 31, 32) ; jamais dans une machine virtuelle.
Le récapitulatif de l'installateur affiche « Ordinateur portable : économie d'énergie
automatique (auto-cpufreq) ».

Démo (captures) : `NOOS_POWER_DEMO=battery|charging|low|critical|full|plugged|desktop`
(+ `NOOS_POWER_OPEN=1` : panneau ouvert après 2 s) ; avec `noos-installer`, une valeur autre
que `desktop` simule un portable.

## Disques (`noos-disk`)

Gestionnaire de disques façon Windows, sur udisks2 : disques internes, clés USB, cartes mémoire
et images disque, avec une **carte des partitions** (taille réelle, espace occupé), la **santé**
(SMART / NVMe : température, heures, secteurs défectueux, autotest) et le **test de vitesse**
(débit le long du disque, temps d'accès, comparaison). **Formater** en exFAT (conseillé pour
les clés), **NTFS** (ntfs-3g / ntfsprogs fournis), FAT32, ext4, Btrfs, XFS ou F2FS ; créer et
supprimer des partitions (type Windows ou Linux selon le système de fichiers), initialiser un
disque (GPT / MBR), renommer, vérifier, réparer, déverrouiller un volume chiffré, créer et
restaurer une image disque, éjecter en toute sécurité.

**Montage sans se battre avec les permissions** : un disque interne se monte « pour de bon »
dans `/mnt/<nom>` (ligne fstab par UUID, `nofail` : le démarrage ne bloque pas si le disque
manque) ; NTFS / exFAT / FAT sont montés avec vos droits, et pour ext4 / Btrfs / XFS le
bouton **« M'en donner la propriété »** règle la propriété du volume (tout le contenu ou le
seul dossier racine). Les clés USB restent montées automatiquement avec vos droits.

Dans la barre des tâches, l'icône **« Retirer le périphérique en toute sécurité »** liste les
périphériques externes (ouvrir, éjecter). Au branchement, la fenêtre **Lecture automatique**
propose d'ouvrir les fichiers, d'importer photos et vidéos (Images › Importées), de lire les
médias, de gérer ou de formater le périphérique ; « Toujours faire ceci » mémorise le choix
(`[autoplay]`). Un son (créé pour NOOS, `data/sounds/generate.py`) accompagne le branchement
et le retrait (`[autoplay] sounds`). L'Explorateur affiche tous les volumes, montés ou non, avec un bouton
d'éjection.

## Réseau et Internet (Paramètres)

- **État** de la connexion avec débit en direct (graphique), adresses IP, DNS et VPN en cours ;
- **DNS** : fournisseurs proposés avec pays, juridiction, journalisation, filtrage et appréciation
  de confidentialité (Quad9, FDN, AdGuard, DNS4EU, Cloudflare, Google, OpenDNS…), mesure du temps
  de réponse, **DNS over TLS** (obligatoire / si possible), serveurs personnalisés, appliqué à un
  réseau ou à tous ;
- **VPN WireGuard** : import d'un fichier `.conf` (bouton ou glisser-déposer) avec aperçu,
  connexion, démarrage automatique, vérification de l'IP publique ; dans la barre des tâches, un
  bouclier apparaît quand un VPN est actif et son menu change de profil en un clic ;
- **Wi-Fi** (mot de passe enregistré, adresse MAC aléatoire), **adresse IP fixe**, **point
  d'accès mobile** et **diagnostic** guidé.
- **Partage de fichiers** (`src/network/sharing.rs`, crate `noos-share`) : trois pages, voir
  « Partage de fichiers en réseau local ».

## Partage de fichiers en réseau local (noos-share)

Paramètres › Réseau et Internet › **Partage de fichiers (Samba)**, **SSH et SFTP**, **WebDAV**
(`noos-settings samba|ssh|webdav`). Chaque page : interrupteur principal, adresses à utiliser
depuis un autre appareil (copier, **QR code**), dossiers partagés (ajout, retrait, lecture seule
ou écriture), appareils connectés (`ss`, noms par `getent`), état des services et du pare-feu.

| Service | Logiciel | Par défaut | Réglages |
|---|---|---|---|
| Samba (SMB) | `samba` ; annonces `wsdd2` (Windows, lié à smbd) et Avahi (Mac, Linux, téléphones) | **actif**, dossier personnel de chaque compte (`[homes]`) en lecture et écriture, **personne ne peut se connecter tant que l'utilisateur n'a pas choisi un mot de passe de partage** | dossiers supplémentaires (réservés au compte qui les ajoute), nom du PC, groupe de travail, visibilité WSD, chiffrement SMB 3 (si possible / toujours), mot de passe de partage |
| SSH / SFTP | `openssh-server` | désactivé (nouvelle installation) ; un serveur SSH déjà utilisé garde son état | port, connexion par mot de passe, clés autorisées (`~/.ssh/authorized_keys` : coller, importer un .pub, retirer ; empreintes SHA-256), empreintes des clés de ce PC |
| WebDAV | `rclone serve webdav` (service **utilisateur** `noos-webdav`) | désactivé | dossiers (un par entrée à la racine), nom d'utilisateur, mot de passe (htpasswd **bcrypt**), HTTPS (certificat auto-signé P-256 créé par `openssl`, empreintes SHA-256 et SHA-1), port (≥ 1024) |

**Sécurité.** Jamais d'invité (`map to guest = never`, `restrict anonymous = 2`, pas de SMB 1 ni
NetBIOS). Mot de passe Samba distinct de la session (`unix password sync = no`) : demandé par
l'interface, passé à `smbpasswd -s -a` par l'entrée standard du script privilégié, jamais sur une
ligne de commande. Réseau local seulement : `hosts allow` (plages privées, IPv6 lien local et
ULA) et règles de port du pare-feu NOOS en zone « Réseau local » (étiquettes « Partage NOOS · … »,
recréées à chaque application ; démon arrêté — installation, chroot — : `rules.json` modifié
directement). SSH : `PermitRootLogin no`, `KbdInteractiveAuthentication no` ; désactiver le mot
de passe sans clé demande confirmation.

**Fichiers.** Configuration système `/etc/noos/sharing.toml` (écrite seulement par
`/opt/noos/libexec/noos-share-helper`, action polkit `org.noos.share.helper`, auth_admin_keep) :
- smb.conf : bloc `# >>> NOOS … # <<< NOOS` **à la fin**, qui rouvre `[global]` et `[homes]`
  (Samba garde la dernière valeur : les réglages invités de Debian sont remplacés sans toucher au
  reste). Piège : les sections lues avant ce bloc ne reprennent pas un `hosts allow` du `[global]`
  rouvert → répété dans `[homes]` et chaque partage. Vérifié par `testparm` avant écriture ;
  copie d'origine `smb.conf.avant-noos`. Outil : `cargo run -p noos-share --example gen-smb --
  /usr/share/samba/smb.conf > smb-test.conf && testparm -s smb-test.conf` ;
- `/etc/ssh/sshd_config.d/90-noos.conf` (lu avant `sshd_config`, la première valeur l'emporte),
  vérifié par `sshd -t`, remis en place en cas d'échec ;
- annonces Avahi `/etc/avahi/services/noos-sftp.service`, `noos-webdav-<compte>.service` ;
- comptes ayant un mot de passe Samba : `/var/lib/noos/sharing/samba-users` (la base de Samba
  n'est pas lisible sans root) ;
- WebDAV : `~/.config/noos/webdav/` (`webdav.toml`, `htpasswd`, `cert.pem`, `key.pem` en 0600) ;
  configuration rclone dans `$XDG_RUNTIME_DIR` : `alias` (écriture) ou `union` du même dossier
  deux fois en `:ro` (rclone refuse une union à un seul membre) réunis par un `combine`.

**Script privilégié** : `noos-share-helper init [--fresh]` (postinst « une seule fois »,
installateur), `apply` (TOML sur l'entrée standard), `samba-password` / `samba-forget` (compte
de `PKEXEC_UID`), `hostname <nom>` (hostnamectl, ligne `127.0.1.1` de /etc/hosts, Samba, Avahi,
wsdd2), `webdav <port> https|http|off` (règle du pare-feu et annonce du compte appelant).
`noos-webdav --password` (entrée standard), `--fingerprint`, `--cert`, `--dry-run`.

**Démonstration** (captures, aucune commande) : `NOOS_SHARE_DEMO=1` (`nopass` : pas de mot de
passe, `off` : tout désactivé, `missing` : paquets absents), `NOOS_SHARE_QR=1` (premier QR code
ouvert), `NOOS_FILE_NET_DEMO=1` + `NOOS_FILE_DEMO='network:///;connect'` (ou `;auth`) pour
l'Explorateur.

## Logiciels (noos-software / noos-pkg)

NOOS est basé sur Debian sid : Logiciels gère les **paquets Debian (apt)** et les
**applications Flatpak**. Flathub est actif par défaut.

- **Un logiciel, plusieurs façons de l'installer** : les paquets apt et les applis Flatpak sont
  rapprochés par leur identifiant AppStream (données DEP-11 de `/var/lib/apt/lists`, paquet
  `appstream`, côté Debian ; `org.gimp.GIMP` côté Flatpak). Une seule ligne « GIMP » propose
  alors Flatpak · Flathub ou apt · Debian.
- **Sources lisibles** : les entrées de `sources.list` et `sources.list.d/*.{list,sources}`
  (formats une ligne et deb822) sont regroupées (Debian sid, NOOS, dépôts tiers) avec un état
  clair (Active, Inactive, Partielle, Non installée). Les **sources de test** (Debian
  experimental, `flathub-beta`) sont séparées et leur activation demande confirmation. Les
  entrées `deb-src` sont masquées.
- Ajout de sources : composants contrib / non-free / non-free-firmware, Debian experimental,
  dépôt NOOS, architecture i386 (Steam, Wine), dépôt APT (adresse, suite, composants, clé),
  fichier .sources, Flathub (système ou votre compte), Flathub bêta, .flatpakrepo.
- **Mises à jour sans mot de passe** : le timer `noos-update-check` télécharge les listes de
  paquets dans `~/.cache/noos/apt` (`apt-get update` sans root) puis simule `apt-get -s
  full-upgrade`.
- Les opérations admin passent par **un seul** `pkexec` par opération ; Flatpak passe par polkit.
- La page Explorer affiche la vitrine Flathub (API publique v2, en cache dans `~/.cache/noos-software`) : bannière, catégories, populaires, tendances, nouveautés et fiches détaillées.
- **AppImage** (`noos-pkg/src/appimage.rs`, `noos-software/src/appimage.rs`) : ouvrir un
  `.AppImage` (Explorateur, `noos-software fichier.AppImage`) affiche son nom, sa version et son
  icône, lus **sans exécuter le fichier** (`unsquashfs -offset` après le runtime ELF ; type 1 :
  `7z`). « Installer » la **déplace** dans `~/Applications`, la rend exécutable et crée
  `~/.local/share/applications/appimage-<id>.desktop` (clé `X-NOOS-AppImage`, icône dans
  `~/.local/share/noos/appimages/icons`) : elle apparaît dans le menu Démarrer. « Lancer sans
  installer » la rend seulement exécutable. Une AppImage déjà installée est lancée directement.
  Page Installées › AppImage : Ouvrir, Supprimer (corbeille + retrait du menu), Intégrer
  (fichiers copiés à la main dans `~/Applications`), Ajouter… Nouvelle version (même
  `StartupWMClass` ou nom) : l'ancienne est proposée à la corbeille. Outils Debian :
  `libfuse2t64` (runtime classique), `fuse3` (runtime statique), `squashfs-tools`,
  `desktop-file-utils` — dépendances du paquet et de l'ISO.
- **Paquets .deb** (`noos-pkg/src/deb.rs`, `noos-software/src/deb.rs`) : Logiciels est
  l'application par défaut des `.deb` (Archives reste dans « Ouvrir avec »). Fiche du paquet
  (`dpkg-deb -f` : version, architecture, taille, mainteneur, site), simulation sans mot de
  passe (`apt-get -s install <fichier>`) : dépendances ajoutées depuis les dépôts, mises à
  jour, conflits ; un paquet aux dépendances introuvables est refusé avec l'explication
  d'apt. Installation par la file d'opérations (pkexec, `apt-get install <chemin>`,
  `--reinstall` si la même version est installée) ; « Ouvrir » si le paquet a un lanceur.
- **Fiche d'un paquet** (`noos-pkg/src/details.rs`, `noos-software/src/details.rs`) : un clic
  sur une ligne de résultat de recherche ou de la page Installées (hors bouton) ouvre la fiche,
  pour les paquets apt (avec ou sans données AppStream) comme pour les Flatpak. Contenu : nom,
  résumé et description longue **traduits** (`apt-cache show` avec `Acquire::Languages=fr,en`,
  paragraphes « . » ; Flatpak : `appstream.xml` du remote), captures et icône AppStream
  (DEP-11 de `/var/lib/apt/lists` — images JPEG XL décodées par glycin — ou cache AppStream de
  Flatpak), versions installée / disponible, tailles téléchargée / installée (`apt-cache show`,
  `flatpak remote-info`), source et dépôt (`apt-cache policy`), catégorie Debian en français,
  licence (AppStream ou `/usr/share/doc/<paquet>/copyright` lisible par machine), développeur,
  mainteneur, commandes fournies, runtime Flatpak, dépendances / recommandations /
  suggestions. **Simulation** sans mot de passe : ce qu'installer ajoutera (`apt-get -s install`,
  runtime Flatpak absent) ou ce que supprimer retirera (`apt-get -s remove`). Boutons Installer,
  Supprimer, Ouvrir (lanceur `.desktop` du paquet). Un logiciel disponible en apt et en Flatpak
  a un onglet par façon de l'installer, pour comparer. Les fichiers DEP-11 sont lus par un petit
  lecteur YAML intégré (pas de dépendance). Aperçu : `NOOS_SOFTWARE_DETAILS=gimp@apt`,
  `apt:htop`, `flatpak:org.gimp.GIMP` (+ `NOOS_SOFTWARE_SCROLL=px`).
- **Recherche** : noms de paquets, puis noms, résumés, mots-clés et catégories AppStream
  (index DEP-11, en C et en français), puis descriptions traduites (`apt-cache search` avec
  `Acquire::Languages=fr,en`) : « retouche photo » trouve GIMP, digiKam, Photoflare. Les
  applications passent avant les simples paquets ; les sous-paquets (`-help-*`, `-l10n-*`,
  `-data`, `-doc`…) sont repliés sous leur logiciel (« 36 paquets associés »).
- **Fiche de partout** : `noos-software --details apt:gimp|flatpak:<id>|gimp`, `--app
  <lanceur.desktop>` (paquet retrouvé par `noos_pkg::owner` : `dpkg-query -S`, `X-Flatpak`,
  `X-NOOS-AppImage`), `--uninstall`, liens `appstream://<id>` et fichiers `.flatpakref`.
  Barre des tâches : « Désinstaller… » ; menu Démarrer : clic droit « Fiche dans Logiciels »
  / « Désinstaller… ».
- **File d'attente** : bouton « n en attente » de la barre d'en-tête (en cours, en attente,
  retirer une opération) ; les installations apt en attente partent en une seule commande
  (un seul mot de passe).
- **Mises à jour détaillées** : cases à cocher, groupes Applications / Composants du système,
  « Mettre à jour » par groupe (`apt-get install --only-upgrade`, `flatpak update <id>`),
  alerte « Redémarrage nécessaire » (noyau, libc, pilotes, noos…), clic → fiche avec les
  nouveautés (`apt-get changelog`). **Installer au redémarrage** : téléchargement
  (`--download-only`), lien `/system-update`, `noos-offline-update.service`
  (system-update.target) installe sans réseau puis redémarre (journal
  `/var/log/noos-offline-update.log`). Paramètres › Mises à jour : « Mettre à jour les
  applications Flatpak automatiquement » (`[updates] auto_flatpak`, désactivé par défaut ;
  règle polkit `50-noos-flatpak-update.rules`, mises à jour seulement).
- **Faire de la place** (Installées) : dépendances inutiles (`apt-get -s autoremove`), cache
  apt, anciens noyaux (le courant et le plus récent sont gardés), runtimes Flatpak inutilisés,
  données `~/.var/app` d'applis supprimées (corbeille) ; tailles mesurées sans root, une
  seule élévation.
- **Historique** (bouton horloge) : journal d'apt (y compris les commandes du terminal) et de
  Logiciels, par jour ; « Annuler » une installation ; « Revenir à l'état d'avant » ouvre
  Sauvegarder et Restaurer sur l'instantané pris juste avant (rapproché par la date).
- **Permissions Flatpak** (fiche) : accès accordés en français, accès larges d'abord (tout le
  dossier personnel, tous les périphériques, X11…), avant l'installation en lecture seule,
  ensuite des interrupteurs (`flatpak override --user`) et « Rétablir ».
- **Compléments pour le matériel** (bandeau d'Explorer) : microcode, pilote NVIDIA,
  firmwares (Intel, AMD, Wi-Fi), `steam-devices` (manette), `fprintd` (lecteur
  d'empreintes), `auto-cpufreq` (portable), plus `isenkram-lookup` s'il est installé ;
  seulement ce qui manque et existe dans les dépôts actifs ; « Tout installer ».
- **Sélection NOOS** : packs Bureautique, Création, Jeux, Multimédia, Développement
  (`noos-pkg/data/packs.json`, téléchargé depuis GitHub avec cache d'un jour, copie intégrée
  en repli) ; seul ce qui manque est installé. Les catégories montrent aussi les applications
  des paquets Debian (« Aussi en paquets Debian »).
- **Place occupée** : taille et données (`~/.var/app/<id>`, `~/.config|.local/share|.cache/<paquet>`)
  sous chaque logiciel d'Installées, tri par place occupée ; fiche : ouvrir le dossier de
  données, « Effacer… » (corbeille).
- **Arrière-plan** : fermer la fenêtre pendant une opération la cache, la file continue,
  Logiciels quitte une fois fini ; notification à la fin de chaque opération quand la fenêtre
  n'est pas au premier plan.
- **Comparer** (fiche d'un logiciel proposé en apt et en Flatpak) : version, tailles, mises à
  jour, isolement (accès larges Flatpak), conseil ; colonne AppImage d'après AppImageHub
  (`appimage.github.io/feed.json`, gardé un jour).
- **Avis** (odrs.gnome.org, comme GNOME Logiciels et KDE Discover) : désactivés par défaut
  (`[software] reviews`, « Activer » dans une fiche) ; note moyenne, répartition, avis dans la
  langue de la session, « Écrire un avis », vote « utile », signalement ; identifiant anonyme
  (SHA-1 local du compte et de `/etc/machine-id`).
- **Commande introuvable** (fish, bash) : `noos-pkg provides --hint` propose le paquet qui
  fournit la commande (champ `Commands` des listes apt, index `~/.cache/noos/commands.tsv`, et
  paquet du même nom) et peut ouvrir sa fiche.
- **Mises à jour des AppImage** : section ELF `.upd_info` (`zsync|…`, `gh-releases-zsync|…`)
  lue sans exécuter ; empreinte SHA-1 de l'en-tête zsync comparée au fichier ; mise à jour =
  téléchargement complet, intégration, ancienne version à la corbeille. Marqueur
  « ↻ Mises à jour intégrées » à l'installation et dans Installées, groupe AppImage dans
  Mises à jour (`noos-pkg appimage-update <fichier> [--apply]`).
- **Miroir Debian** (Sources) : liste officielle (`mirror-master.debian.org`, gardée 7 jours),
  tableau drapeau / pays / miroir / hébergeur / débit, « Tester » (1 Mio de `Packages.xz`),
  « Tester les premiers » (réseau mondial et pays de la session), « Utiliser » (sources
  réécrites, `apt-get update`).
- **Liste virtualisée** : la page Installées est un `GtkListView` (plus de limite de 300).

En ligne de commande :

```bash
noos-pkg info
noos-pkg search firefox
noos-pkg updates
noos-pkg presets
noos-pkg appimage ~/Téléchargements/App.AppImage   # nom, version, icône (sans l'exécuter)
noos-pkg appimage-install ~/Téléchargements/App.AppImage
noos-pkg deb ~/Téléchargements/paquet.deb          # fiche + simulation des dépendances
noos-pkg details -b apt gimp                       # fiche détaillée (Logiciels) + simulation
noos-pkg details -b flatpak org.gimp.GIMP
noos-pkg cleanup                                   # ce qui peut être nettoyé
noos-pkg hardware                                  # compléments recommandés pour le matériel
noos-pkg owner org.gnome.SimpleScan                # paquet d'une application
noos-pkg provides htop                             # paquet qui fournit une commande
noos-pkg appimage-update ~/Applications/App.AppImage   # nouvelle version ? (--apply : l'installer)
noos-pkg repo-add -b apt apt-repo https://repo.example.org/apt stable main clé=https://repo.example.org/key.asc --dry-run
```

**Testé :**
- tests unitaires (analyse des sources une ligne / deb822, `apt-cache policy`, simulations
  `apt-get -s`, DEP-11, regroupement des sources, `apt-cache show`, lecteur YAML DEP-11,
  AppStream XML, `flatpak remote-info`) dans la CI Debian sid ;
- fiche détaillée vérifiée dans l'aperçu (18/11/2026) : apt sans AppStream (`htop`), apt avec
  AppStream et captures JPEG XL (`gimp`, `simple-scan` installé : Ouvrir / Supprimer), Flatpak
  (`org.gimp.GIMP`, `app.fluxer.Fluxer` installé), onglets apt / Flatpak ;
- à revalider sur une vraie Debian sid : installation et suppression, mises à jour, ajout,
  activation et suppression de dépôts.

**À faire :**
- un **agent polkit** propre à NOOS : dans la vraie session, il n'y en a pas encore pour afficher la demande de mot de passe ;
- zypper (openSUSE) ;
- idées d'amélioration classées par impact / effort : `docs/plans/logiciels.md` (les 10
  premières sont faites ; à valider en VM : installation au redémarrage, règle polkit
  Flatpak, menu contextuel du menu Démarrer).

## Paramètres (noos-settings)

L'organisation reprend celle de Windows 11 : compte et recherche dans la barre latérale, catégories en icônes colorées, pages à fil d'Ariane, réglages présentés en cartes.

| Page | Contenu |
|---|---|
| Système › Écran | Moniteurs, échelle, résolution, fréquence, orientation, VRR, HDR. Les changements s'appliquent à chaud, avec retour automatique aux anciens réglages au bout de 15 s sans confirmation |
| Système › Notifications | Ne pas déranger, son, durée des bannières, notification de test |
| Système › Alimentation | Profil d'alimentation (power-profiles-daemon / tuned-ppd, ou auto-cpufreq sur portable), extinction de l'écran et veille après inactivité, inhibition en plein écran |
| Système › Informations | Processeur, mémoire, carte graphique, renommer le PC |
| Personnalisation | Thèmes rapides, arrière-plan (images du jour Bing, avec changement quotidien automatique et bouton « Jours précédents » : l'archive de Bing s'arrête à 15 jours ; fonds Wallhaven au hasard ou par recherche, 24 par page, bouton « Charger plus d'images » jusqu'à la dernière page — retour à la page 1 à chaque nouvelle recherche, graine conservée pour que le hasard ne repasse pas les mêmes images), couleurs, thèmes libAdapta, barre des tâches (hauteur 28 à 96 px, icônes des applis actives regroupées dans « ^ » ou étalées), polices |
| Bluetooth et appareils | Bluetooth (BlueZ) : appareils associés en tuiles (état, batterie, connecter / renommer / supprimer), assistant « Ajouter un appareil », visibilité ; liens vers Imprimantes et scanners, Souris, Clavier (voir « Bluetooth et appareils ») |
| Jeux | Mode jeu Steam (session gamescope, démarrage, Decky Loader, affichage de gamescope), composants pour le jeu, VRR / scanout direct / HDR du bureau (voir « Jeux et mode jeu Steam ») |
| Confidentialité › Pare-feu | Pare-feu applicatif NOOS : activation, mode, règles par application, ports, demandes en attente, journal (voir « Pare-feu ») |

Une page s'ouvre directement avec `noos-settings <page>` (`display`, `power`, `firewall`, `taskbar`, `colors`, `bluetooth`, `printers`…).

## Jeux et mode jeu Steam (Paramètres › Jeux)

Code : `noos-settings/src/gaming/` (`model.rs` : état, réglages, catalogue — testé ; `mod.rs` :
page), scripts système dans `noos-gaming/` (installés par `noos-gaming/install-system.sh`).

**Mode jeu Steam** — l'interface du Steam Deck (`steam -gamepadui -steamos3`) dans sa propre
session **gamescope**, sans le bureau, comme SteamOS, Bazzite ou ChimeraOS. Le grand
interrupteur de la bannière l'active en une fois (une seule demande de mot de passe,
`pkexec sh -c` de `noos-pkg`), après une fenêtre qui liste ce qui va être fait :

1. contrib et non-free ajoutés aux sources Debian s'il le faut (préréglage `debian-nonfree` de
   noos-pkg : **gamescope** et **steam-installer** sont dans contrib) ;
2. licence de Steam acceptée par l'utilisateur dans la fenêtre, puis transmise à debconf
   (`steam-installer steam/question select I AGREE`), sinon l'installation non interactive échoue ;
3. `apt-get install gamescope steam-installer mangoapp` (i386 déjà actif sur NOOS) ;
4. `noos-gaming-helper session on` : `/usr/local/share/wayland-sessions/noos-steam.desktop`
   (« Steam (mode jeu) »), que noos-login liste juste après NOOS (icône manette) ;
5. raccourci **« Mode jeu Steam »** (`~/.local/share/applications/org.noos.SteamGameMode.desktop`,
   copié sur le bureau) = `noos-settings steam-mode` : confirmation, puis bascule.

Désactiver retire la session, le raccourci et le démarrage direct ; Steam et les jeux restent.

**Session** — `/usr/libexec/noos-steam-session` (journal `~/.local/state/noos/steam-session.log`)
lit `~/.config/noos/steam-session.conf` (écrit par la page, lecture stricte : clés connues,
valeurs vérifiées, jamais de `source`) et lance
`gamescope -e --xwayland-count 2 [--prefer-output] [-W -H] [-r] [--adaptive-sync] [--hdr-enabled]
[--framerate-limit] [-F fsr|nis|…] [--mangoapp] -- steam -gamepadui -steamos3 -steampal [-steamdeck]`
avec les variables de gamescope-session (`STEAM_GAMESCOPE_*`, `STEAM_USE_MANGOAPP`…). La page
affiche la commande obtenue. Définitions et fréquences proposées = modes de l'écran (IPC
`GetOutputs`), disponibilité VRR / HDR expliquée par écran.

**Passer d'une session à l'autre sans mot de passe** — `/usr/libexec/noos-session-switch
steam|desktop`, par pkexec avec l'action `org.noos.gaming.switch` (**yes** pour la session
active : on rouvre une session pour son propre compte, et la session gamescope n'a pas d'agent
polkit) :

- il retrouve la session logind active de l'appelant (`PKEXEC_UID`, seat0), puis confie la suite
  à une unité passagère `systemd-run --unit=noos-session-switch` qui survit à la fin de session ;
- l'appelant ferme sa session proprement : le bureau par l'IPC `Quit` du compositeur
  (noos-settings, après la réussite de pkexec), Steam par `steam -shutdown` ;
- l'unité attend la fermeture (30 s, puis `loginctl terminate-session`), écrit
  `/run/noos-gaming/greetd.toml` (= `/etc/greetd/config.toml` + `[initial_session]` vers la
  session voulue), ajoute un complément `/run/systemd/system/greetd.service.d/zz-noos-session-switch.conf`
  (`ExecStart=greetd --config …`), relance greetd puis **retire aussitôt** le complément : la
  connexion automatique ne vaut qu'une fois, un redémarrage de greetd retrouve la configuration
  normale.

Dans Steam, **Marche/Arrêt › Passer au bureau** appelle `steamos-session-select plasma` : NOOS
fournit cet outil dans `/usr/lib/noos/steamos/` (en tête du `PATH` de la session seulement),
avec `steamos-update` (code 7 = pas de mise à jour : le système se met à jour par apt),
`steamos-select-branch` et `jupiter-biosupdate` (sans effet), comme ChimeraOS.

**Démarrage du PC** — écran de connexion, bureau NOOS ou mode jeu Steam :
`noos-gaming-helper boot login|desktop|steam` réécrit la section `[initial_session]` de
`/etc/greetd/config.toml` pour le compte appelant (connexion automatique de greetd, sans
authentification : c'est le principe). Lu par la page (`model::parse_boot`).

**Decky Loader** — `noos-gaming-helper decky install|remove` refait le script officiel
`decky-installer` **sans exécuter de script téléchargé** : version publiée (API GitHub), fichier
`PluginLoader` dont l'empreinte SHA-256 publiée par GitHub est vérifiée, installé dans
`~/homebrew/services` (emplacement attendu par le système de mise à jour intégré de Decky),
`.cef-enable-remote-debugging` dans le dossier de Steam, service `plugin_loader` (root, comme
l'original). La fenêtre de confirmation liste ces étapes et rappelle que Decky et ses greffons
tournent en root. La désinstallation garde `~/homebrew/plugins`.

**Composants pour le jeu** — tuiles installer / supprimer (état par `dpkg-query` et
`flatpak list`), par `noos-pkg` : paquets Debian (`apt_root`, 32 bits compris si i386 est actif)
**GameMode** (+ `libgamemode0:i386`), **MangoHud** (+ `:i386`), **vkBasalt** (+ `:i386`),
**Gamescope**, **Steam**, **Lutris**, **Protontricks** (les quatre derniers dans contrib : activé à la
demande), et Flatpak de Flathub (installation système si le remote système existe, sinon
utilisateur) **GOverlay** (`io.github.benjamimgois.goverlay`), **ProtonPlus**
(`com.vysp3r.ProtonPlus`), **ProtonUp-Qt** (`net.davidotek.pupgui2`), **Heroic**
(`com.heroicgameslauncher.hgl`), **Bottles** (`com.usebottles.bottles`). Une fois installé, la
tuile rappelle l'option de lancement Steam (`gamemoderun %command%`, `MANGOHUD=1 %command%`…).

**Démo** (captures, rien n'est exécuté) : `NOOS_GAMING_DEMO=1` (désactivé), `on` (activé,
Decky, démarrage dans Steam), `installing` (activation en cours) ; `NOOS_GAMING_DIALOG=enable|
disable|decky|switch` ouvre la fenêtre correspondante. Ex. :
`tools/preview/run_app.sh jeux.png 4 "env NOOS_GAMING_DEMO=on noos-settings gaming"`.

**Essai sur une vraie machine** (étapes root faites par l'administrateur) : activer depuis la
page, se déconnecter, choisir « Steam (mode jeu) » ; ou « Lancer le mode jeu ». Secours si la
session Steam ne s'ouvre pas ou si le PC démarre en boucle dedans : console (Ctrl+Alt+F2), retirer
la section `[initial_session]` de `/etc/greetd/config.toml`, puis `sudo systemctl restart greetd`
(`sudo rm /usr/local/share/wayland-sessions/noos-steam.desktop` retire aussi la session).

## Bluetooth et appareils (Paramètres)

Code : `noos-settings/src/devices/` (`bluez.rs` backend, `bt_add.rs` assistant et agent,
`cups.rs` backend d'impression, `printers.rs` pages).

**Bluetooth** — BlueZ par D-Bus (bus système, gio) : `GetManagedObjects` puis suivi par les
signaux (`InterfacesAdded/Removed`, `PropertiesChanged`, démarrage / arrêt de bluetoothd). Un seul
contrôleur par processus ; Paramètres enregistre un **agent d'appairage** `org.bluez.Agent1`
(`KeyboardDisplay`, agent par défaut tant que Paramètres vit) :

- code à 6 chiffres à confirmer (téléphones, casques récents), code PIN à saisir (anciens
  appareils, 0000 / 1234 proposé), code à taper sur un clavier, autorisation d'une connexion
  entrante (les appareils associés et de confiance sont acceptés sans question) ;
- « Ajouter un appareil » : recherche (`StartDiscovery`, arrêtée à la fermeture et pendant
  l'association), icône et type d'après la propriété `Icon` (ou la classe), force du signal,
  appareils sans nom masqués par défaut ; `Pair` → `Trusted` → `Connect` ; écrans « prêt » ou
  « échec » avec une explication claire des erreurs BlueZ (code refusé, hors de portée, profil
  audio manquant…) ;
- tuiles des appareils associés : connecté ou non, batterie (`Battery1`), menu « … » (connecter,
  renommer = `Alias`, supprimer = `RemoveDevice`) ;
- états : service arrêté (bouton « Démarrer le service », pkexec), BlueZ absent (« Installer »),
  aucun adaptateur, bloqué par rfkill (l'interrupteur débloque par `/dev/rfkill`).

Droits : groupe `bluetooth` (installateur, postinst une fois). Audio : `libspa-0.2-bluetooth`.

**Imprimantes et scanners** — outils de CUPS en `LC_ALL=C` : `lpstat -e / -d / -r`,
`lpoptions -p` (état, modèle, adresse, `marker-*` = niveaux d'encre), `lpq` (file d'attente),
`lpinfo -l -v` + `driverless` (détection réseau / USB), `lpinfo -m` (pilotes), `lpadmin`,
`cancel`, `cupsenable` / `cupsdisable`, `lp` (page de test `/usr/share/cups/data/testprint`).

- liste avec état (prête, impression, en pause, problème traduit : bac vide, bourrage, encre
  bientôt vide…), badge « Par défaut » ;
- fiche d'une imprimante : page de test, définir par défaut, problèmes, file d'attente (annuler
  un document ou tout), niveaux d'encre colorés, suspendre, partager, informations, **page Web de
  l'imprimante** (hôte de l'URI ; `dnssd://` résolu par `avahi-browse`, `adminurl` annoncé si
  présent) et **interface de CUPS** (`http://localhost:631/printers/<file>`), suppression ;
- « Ajouter un appareil » : imprimantes IPP Everywhere / AirPrint ajoutées **en un clic**
  (`lpadmin -m everywhere`), sinon choix du pilote (pilotes correspondants à l'identifiant IEEE
  1284 ou au modèle, recommandé présélectionné, recherche dans tous les pilotes) ; ajout par
  adresse (IP, nom d'hôte, URI) ; en cas d'échec sans pilote, « Choisir un pilote » ;
- **droits** : CUPS accepte l'administration des membres de `lpadmin` sans mot de passe ; sinon
  la commande est relancée par **pkexec** (jamais sudo) et une carte propose « M'ajouter au
  groupe » ;
- scanners : `scanimage -L` (eSCL / WSD par sane-airscan, USB), bouton « Numériser » (simple-scan,
  Skanpage, Skanlite ou XSane) ;
- CUPS absent : « Installer » (paquets de `debian/control`) ; arrêté : « Démarrer le service ».

Sur Debian, `lpadmin`, `lpinfo`, `cupsenable`… sont dans `/usr/sbin` (hors du PATH des
utilisateurs) : appelés par leur chemin complet. Les files créées automatiquement par
cups-browsed (`implicitclass://`) n'ont pas d'état dans `lpoptions` : lu par `lpstat -p`. Sans
`lpq` (cups-bsd), la file d'attente vient de `lpstat -o`. Un même scanner réseau vu par plusieurs
moteurs SANE (escl, epson2, airscan) n'est affiché qu'une fois.

Paquets (`debian/control`, `noos-iso/packages-system.txt`) : `cups`, `cups-browsed`, `cups-bsd`,
`cups-filters`, `avahi-daemon`, `avahi-utils`, `libnss-mdns`, `ipp-usb`, pilotes Gutenprint,
HP (hpcups) et Brother (brlaser), `sane-utils`, `sane-airscan` ; `printer-driver-all` et
`simple-scan` en Recommends. Services activés par l'installateur et par postinst (une fois,
`/var/lib/noos/devices`).

Démos (sans BlueZ ni CUPS) :

| Variable | Effet |
|---|---|
| `NOOS_BT_DEMO=1` | appareils associés fictifs (casque, souris, manette, téléphone) |
| `NOOS_BT_DEMO=add` / `pair` / `pin` | assistant ouvert (recherche simulée) / association de l'iPhone avec code à confirmer / de l'enceinte avec code PIN |
| `NOOS_BT_DEMO=off` / `none` / `service` | Bluetooth désactivé / aucun adaptateur / service arrêté |
| `NOOS_PRINT_DEMO=1` | imprimantes (par défaut, impression en cours avec encre basse, en pause) et scanners fictifs |
| `NOOS_PRINT_DEMO=printer` | fiche de l'imprimante Epson (file d'attente, niveaux d'encre) |
| `NOOS_PRINT_DEMO=add` / `driver` / `fail` / `none` | assistant d'ajout / choix du pilote d'une imprimante USB / échec d'un ajout sans pilote / aucune imprimante trouvée |
| `NOOS_PRINT_DEMO=missing` | CUPS non installé |
| `NOOS_PRINT_OPEN=<file CUPS>` | fiche d'une vraie imprimante (sans démo) |

```bash
tools/preview/run_app.sh bt.png 4 "env NOOS_BT_DEMO=pair noos-settings bluetooth"
tools/preview/run_app.sh imp.png 5 "env NOOS_PRINT_DEMO=1 noos-settings printers"
```

## Pare-feu (noos-firewall)

Pare-feu applicatif façon Windows Defender : chaque **nouvelle connexion** (sortante ou entrante)
qu'aucune règle ne couvre est retenue par le noyau pendant que la session décide ; ufw et firewalld
ne sont plus utilisés par NOOS (le service est en conflit avec eux : `Conflicts=` systemd).

**Fonctionnement**

- Table nftables `inet noos` générée en entier par le démon (`noos-firewall/src/nft.rs`) et
  appliquée atomiquement (`nft -f -`). Entrée : politique `drop`, établi / `lo` / interfaces de
  confiance (`virbr*`, `docker*`, `podman*`…) / ICMP utiles / DHCP / découverte du réseau local
  (mDNS, SSDP, réseau local seulement) / règles de ports, puis `queue num 1` **sans bypass** (démon
  arrêté = entrées fermées). Sortie : règles de ports puis `queue num 0 bypass` (démon arrêté =
  sorties autorisées, le réseau ne casse pas).
- `noos-firewalld` (root) lit la file NFQUEUE (crate `nfq`, netlink en Rust pur : pas de
  `libnetfilter-queue1`), analyse les en-têtes IPv4/IPv6 + TCP/UDP (`packet.rs`), retrouve le
  socket dans `/proc/net/{tcp,udp}{,6}` puis le processus (`/proc/<pid>/fd` → `socket:[inode]`) et
  le programme (`/proc/<pid>/exe`, cmdline, Flatpak par `/.flatpak-info`, AppImage par `$APPIMAGE`)
  — `procnet.rs`.
- Décision (`daemon.rs`) : règle par programme la plus précise (adresse + port > port > programme ;
  compte > tous ; blocage à égalité), sinon services du système (comptes < 1000) autorisés en
  sortie, sinon selon le **mode** : *Interactif* (demander), *Sorties autorisées* (façon Windows,
  entrées demandées), *Silencieux* (rien demandé), *Tout autoriser*. Une entrée vers un port où
  rien n'écoute est jetée sans demander. Les réémissions et connexions simultanées d'un même
  programme rejoignent la même demande.
- Sortie refusée : le paquet revient avec la marque `0x4e4f0001` (verdict *repeat*) et la table le
  rejette tout de suite (`reject … admin-prohibited`) : l'appli échoue immédiatement au lieu
  d'attendre.
- nftables ne reconnaît pas un exécutable : les **règles par programme** sont appliquées par le
  démon au verdict ; les **règles de ports** (ports, plages, listes, protocole, sens, zone « réseau
  local », interface, adresse distante) sont de vraies règles nftables.
- Sans réponse avant `timeout_secs` (30 s) : `default_action` (Bloquer) pour cette fois, retenue
  60 s pour ne pas redemander en boucle. **Aucune fenêtre d'autorisation connectée** (paquet tout
  juste installé, session pas encore ouverte) : après 3 s, sorties selon `no_agent_action`
  (Autoriser), entrées bloquées.
- Persistance : `/etc/noos/firewall/config.toml` et `rules.json` (root, 0644 ; copie `.illisible`
  si un fichier est abîmé). Règles livrées : `/opt/noos/bin/*` et `/usr/lib/apt/methods/*`
  autorisés en sortie.

**Sécurité du canal démon ↔ session** : socket Unix `/run/noos-firewall/control.sock` (0666), une
requête JSON par ligne (`proto.rs`). L'appelant est identifié par `SO_PEERCRED` (UID / PID donnés
par le noyau). Sans mot de passe : lire l'état, décider pour **ses propres** connexions sortantes,
modifier ses propres règles sortantes, bloquer une fois. Tout le reste (entrées, ports, réglages,
règles d'autres comptes ou de tous, services du système) passe par polkit :
`pkcheck --action-id org.noos.firewall.manage --process pid,début,uid --allow-user-interaction`
(`auth_admin_keep`, fenêtre de mot de passe de l'agent polkit du shell). Les administrateurs
(groupe `sudo`) voient les demandes et le journal de tous les comptes. Toutes les valeurs reçues
(ports, interfaces, adresses) sont validées avant d'entrer dans la table.

**Fenêtre d'autorisation** (`noos-firewall-prompt`, lancée par `/etc/xdg/autostart`) : nom et
icône de l'appli (fichier .desktop), chemin, hôte (résolution inverse), port + service
(`/etc/services`), protocole, sens, interface, compte ; « Retenir ce choix » + portée (programme,
programme + port, programme + adresse et port, ou port pour tous en entrée) ; Autoriser (bouclier
si le mot de passe sera demandé) / Bloquer ; compte à rebours. Après une décision permanente :
« Révoquer » ou « Gérer le pare-feu ».

**Paramètres › Confidentialité et sécurité › Pare-feu** : bouclier d'état, activer / désactiver,
bandeau si ufw / firewalld sont encore actifs (bouton pour les couper), autorisations en attente,
comportement (mode, tout bloquer en entrée, sans réponse, délai, sans fenêtre, services du système,
ping, découverte du réseau, interfaces de confiance, journal), **Applications** (activer, modifier,
révoquer, ajouter avec sélecteur de programme), **Ports** (ouvrir / fermer, plages et listes, TCP /
UDP, sens, zone, interface, adresse, préréglages Minecraft / Steam / SSH / KDE Connect / Sunshine),
**connexions bloquées** (regroupées, bouton « Autoriser »).

**Commandes** : `noos-firewall status | rules | log [N] | pending | ruleset | check | open PORTS
[tcp|udp|any] [lan] | close ID | delete ID | off` ; `noos-firewalld --print-ruleset`. Secours si le
réseau ne passe plus : `sudo /opt/noos/bin/noos-firewall off` (retire la table jusqu'au prochain
démarrage du service) ou `sudo systemctl disable --now noos-firewall`.

**Installer depuis le dépôt** (sans le paquet) :
```bash
cargo build --release -p noos-firewall -p noos-firewall-prompt -p noos-settings
sudo ./noos-firewall/install-host.sh       # --uninstall pour retirer
```

**Tests sans root** : `cargo test -p noos-firewall` (analyse des paquets, tables des sockets,
correspondance des règles, décisions du démon, génération de la table, persistance) ; la table
générée est vérifiée par `nft -c` dans un espace de noms réseau à soi (`unshare -rn`),
aussi par `noos-firewall check`. Aperçu : `NOOS_FIREWALL_DEMO=1` (page Paramètres et fenêtre
d'autorisation fictives ; `in` = demande entrante d'abord, `done` = écran « Révoquer »).

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
| Super | Menu Démarrer (taper aussitôt pour rechercher) |
| Super+E / Super+I / Super+Entrée | Explorateur / Paramètres / Terminal (noos-term) |
| Super+← / → | Moitié gauche / droite, puis 2/3 et 1/3 en répétant |
| Super+↑ / ↓ | Maximiser / restaurer-réduire ; depuis une moitié : quart du haut / du bas |
| Glisser une fenêtre contre un bord | Moitié (Ctrl : tiers), coin = quart, haut = maximiser |
| Glisser une fenêtre vers le haut | Barre de dispositions : moitiés, 2/3 + 1/3, tiers, moitié + quarts, quarts |
| Super+glisser | Déplacer une fenêtre depuis n'importe où |
| Super+D | Afficher le bureau |
| Super+W | Mode édition du bureau (widgets, barres, mises en page) ; Échap pour sortir |
| Super+F, F11 | Plein écran |
| Alt+Tab, Alt+F4 | Fenêtre suivante, fermer |
| Super+Maj+S, Impr. écran | Outil Capture d'écran |
| Super+Maj+R | Recharger la configuration |
| Ctrl+Alt+F1…F12 | Changer de terminal virtuel |

## HDR10 (compositeur + Paramètres › Système › Écran)

Activé par écran dans Paramètres › Système › Écran › « Utiliser le HDR » (appliqué tout de suite,
retour automatique au SDR au bout de 15 s sans confirmation), enregistré dans `noos.toml` :

```toml
[[output]]
name = "DP-1"
hdr = true              # absent = [gaming] hdr (« HDR par défaut », Paramètres › Jeux)
sdr_brightness = 203    # blanc du bureau en cd/m² (80 – 480, défaut 203 = BT.2408)
```

IPC : `{"type":"set-hdr","output":"DP-1","enabled":true,"sdr_brightness":250}` (immédiat, sans
toucher à `noos.toml`). `get-outputs` donne `hdr_capable`, `hdr_active`, `hdr_unavailable` (raison
lisible : EDID sans PQ, pilote sans `HDR_OUTPUT_METADATA` ou sans `BT2020_RGB`, mode imbriqué),
`sdr_brightness`, `peak_brightness` (EDID).

Fonctionnement (`noos-comp/src/hdr.rs`) — Smithay ne gère pas le HDR, le compositeur écrit lui-même
les propriétés du connecteur par une validation atomique (`ALLOW_MODESET`, l'écran clignote une
fois) : métadonnées HDR10 (`HDR_OUTPUT_METADATA`, EOTF PQ, primaires et luminances de l'EDID),
`Colorspace` = `BT2020_RGB`, `max bpc` = 10, framebuffer 10 bits. À la connexion d'un écran, les
propriétés sont remises en SDR si le HDR n'est pas demandé (un compositeur précédent a pu le
laisser actif). Le bureau est dessiné comme en SDR dans une texture, puis un shader convertit :
sRGB (gamma 2,2) → linéaire → BT.2020 → `sdr_brightness` cd/m² → PQ. Les captures d'écran et le
partage d'écran restent en SDR.

Limites : pas de protocole `wp_color_management_v1` (une appli ou un jeu HDR natif est traité
comme du SDR), pas de scanout direct en HDR (tout passe par la conversion), pas de mappage des
tons, journal `HDR : capacités du connecteur` dans `session.log`.

Aperçu imbriqué : `NOOS_HDR_PREVIEW=roundtrip` (écran simulé compatible HDR ; conversion PQ puis
décodage inverse, l'image doit rester identique) ou `NOOS_HDR_PREVIEW=pq` (signal PQ brut, délavé).

## Effets de transparence (Mica)

Quand les effets de transparence sont activés, le compositeur floute le fond d'écran sous la
barre des tâches, le menu Démarrer, le panneau de recherche, les réglages rapides et le centre de notifications. C'est le
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

L'option `apply_to_all_apps` étend les couleurs aux applis externes : `~/.config/gtk-4.0/gtk.css`
(libadwaita), `~/.config/gtk-3.0/gtk.css` (GTK3, thème adw-gtk3), palette **qt5ct / qt6ct**
pour Qt 5 et 6 (la session exporte `QT_QPA_PLATFORMTHEME=qt6ct`) et la couleur d'accent GNOME
la plus proche (seule couleur visible des applis Flatpak). La disposition des boutons
(`button_layout`) est aussi écrite dans `org.gnome.desktop.wm.preferences button-layout` :
les applis GTK3, GTK4, libadwaita et Qt (barre de titre « Adwaita »,
`QT_WAYLAND_DECORATION=adwaita`) ont alors réduire / agrandir / fermer, et pas seulement
fermer comme le veut le réglage par défaut de GNOME. Attention : ces réglages sont également lus par vos autres
sessions (GNOME…).

## État du prototype : ce qui marche, ce qui manque

**Fonctionne (testé en mode imbriqué) :**
- compositeur Smithay : xdg-shell, layer-shell, dmabuf, presse-papiers ;
- placement et snap des fenêtres, IPC ;
- barre des tâches, menu Démarrer, réglages rapides ;
- explorateur, paramètres, thème libAdapta généré à chaud ;
- Xwayland lancé et géré par le compositeur (gestionnaire de fenêtres X11 intégré, `noos-comp/src/xwayland.rs`).

**Testé en mode imbriqué :**
- notifications (`org.freedesktop.Notifications`) : bannières, centre de notifications, calendrier, ne pas déranger ;
- zone des applis actives (StatusNotifierItem + dbusmenu) : menu « ^ » ou icônes étalées ;
- Outil Capture d'écran ;
- capture `ext-image-copy-capture` et partage d'écran PipeWire (portail wlr).

**Écrit mais pas encore testé sur du vrai matériel :** le backend DRM (session greetd / GDM), le VRR
(`VRR_ENABLED`), le scanout direct en plein écran et le framebuffer 10 bits.

**Pourquoi le VRR est indisponible** : le compositeur distingue la cause et la transmet par IPC
(`vrr_reason`, `vrr_display`, `gpu_driver` de `OutputInfo`) ; Paramètres › Écran et › Jeux l'expliquent.
- `gpu` : le connecteur n'a pas la propriété DRM `vrr_capable` (ou le CRTC pas `VRR_ENABLED`) : carte ou
  pilote incapable. Cas des Intel HD/UHD Graphics avant la 11e génération : i915 n'attache
  `vrr_capable` que si `DISPLAY_VER >= 11` (Ice Lake, Iris Xe, Arc). L'UHD 630 (Coffee Lake, gen 9.5)
  n'a pas le circuit : aucun paquet ni réglage n'y change rien.
- `display` : `vrr_capable` existe mais vaut 0 : l'écran ne l'annonce pas (FreeSync désactivé dans son
  menu, câble/adaptateur, écran incompatible).
- `vrr_display` : plage lue dans l'EDID (`noos-core/src/edid.rs` : bloc AMD FreeSync de l'extension
  CTA, sinon descripteur « Display Range Limits »), indépendamment de la carte. Ex. AOC Q27G4 :
  FreeSync 48–180 Hz.
- Vérifier soi-même : `python3` + libdrm, ou `modetest -c` (paquet `libdrm-tests`) : chercher
  `vrr_capable` dans les propriétés du connecteur.

**À faire :**
- HDR : protocole `wp-color-management-v1` pour les applis et jeux HDR natifs, scanout direct en HDR, mappage des tons (voir « HDR10 ») ;
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

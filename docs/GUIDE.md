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

## Bureaux virtuels, plusieurs écrans, mode édition

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

**Mode édition** (clic droit sur le bureau ou la barre) : les éléments de la barre se
déplacent d'une zone à l'autre par glisser-déposer, se retirent d'un clic sur leur « × » et
reviennent par « Ajouter à la barre ». Le dock se place **en bas, en haut, à gauche ou à
droite** (les panneaux suivent) ; une **barre fine** optionnelle, au bord opposé, montre les
bureaux virtuels (clic ou molette pour changer, « + » pour en créer) et accueille les éléments
qu'on y dépose. Le cadre « Les notifications apparaîtront ici » se glisse vers l'un des quatre
coins. Des widgets se posent librement sur le bureau : horloge, ressources, note (son texte est
gardé), calendrier. Les mêmes réglages sont dans Paramètres › Personnalisation › Barre des
tâches.

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

En ligne de commande :

```bash
noos-pkg info
noos-pkg search firefox
noos-pkg updates
noos-pkg presets
noos-pkg appimage ~/Téléchargements/App.AppImage   # nom, version, icône (sans l'exécuter)
noos-pkg appimage-install ~/Téléchargements/App.AppImage
noos-pkg deb ~/Téléchargements/paquet.deb          # fiche + simulation des dépendances
noos-pkg repo-add -b apt apt-repo https://repo.example.org/apt stable main clé=https://repo.example.org/key.asc --dry-run
```

**Testé :**
- tests unitaires (analyse des sources une ligne / deb822, `apt-cache policy`, simulations
  `apt-get -s`, DEP-11, regroupement des sources) dans la CI Debian sid ;
- à revalider sur une vraie Debian sid : installation et suppression, mises à jour, ajout,
  activation et suppression de dépôts.

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
| Personnalisation | Thèmes rapides, arrière-plan (images du jour Bing, avec changement quotidien automatique et bouton « Jours précédents » : l'archive de Bing s'arrête à 15 jours ; fonds Wallhaven au hasard ou par recherche, 24 par page, bouton « Charger plus d'images » jusqu'à la dernière page — retour à la page 1 à chaque nouvelle recherche, graine conservée pour que le hasard ne repasse pas les mêmes images), couleurs, thèmes libAdapta, barre des tâches (hauteur 28 à 96 px, icônes des applis actives regroupées dans « ^ » ou étalées), polices |
| Confidentialité › Pare-feu | Pare-feu applicatif NOOS : activation, mode, règles par application, ports, demandes en attente, journal (voir « Pare-feu ») |

Une page s'ouvre directement avec `noos-settings <page>` (`display`, `power`, `firewall`, `taskbar`, `colors`…).

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

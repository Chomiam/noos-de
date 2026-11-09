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
| `noos-pkg/` | Bibliothèque + CLI `noos-pkg` : dnf et Flatpak, sources regroupées, rapprochement AppStream |
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
- « Ouvrir avec », corbeille complète (restaurer, supprimer définitivement, vider), Suppr /
  Maj+Suppr, F2, Ctrl+A, Ctrl+F, Ctrl+1 / Ctrl+2, page « dossier vide ».

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

- **avant et après chaque transaction dnf** (greffon actions de dnf5 : `noos-save.actions`) ;
- **manuels**, jamais supprimés automatiquement, et **réguliers** (horaires, désactivés par
  défaut) avec un nombre conservé par période ;
- **restauration au prochain redémarrage** : l'état actuel est gardé (`@.avant-restauration-…`)
  et la restauration s'annule tant qu'on n'a pas redémarré ; le noyau par défaut est choisi
  pour correspondre à l'instantané (/boot est séparé) ;
- **menu de démarrage « Restaurer depuis un instantané »** (`/etc/grub.d/42_noos_snapshots`,
  régénéré par `noos-snapshots-grub.path` à chaque instantané) : date, heure, nom et origine de
  chaque instantané ; il démarre en lecture seule avec une couche en mémoire
  (`systemd.volatile=overlay`, `noos.snapshot=N`), et l'application propose ensuite de le garder ;
- l'application sépare **manuels** et **automatiques** (dnf, réguliers) par des badges et filtres,
  groupe par jour, détaille chaque instantané (noyau, paquets, nettoyage, présence au démarrage),
  compare paquets et fichiers (`rpm`, `snapper status`), ouvre ses fichiers, renomme et marque
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

## Logiciels (noos-software / noos-pkg)

NOOS est basé sur Fedora : Logiciels gère les **paquets Fedora (dnf)** et les **applications
Flatpak**. Flathub et Fedora Flatpaks sont actifs par défaut.

- **Un logiciel, plusieurs façons de l'installer** : les paquets dnf et les applis Flatpak sont
  rapprochés par leur identifiant AppStream (`metainfo(org.gimp.GIMP…)` côté RPM, `org.gimp.GIMP`
  côté Flatpak). Une seule ligne « GIMP » propose alors Flatpak · Flathub, Flatpak · Fedora ou
  dnf · Fedora.
- **Sources lisibles** : les dépôts bruts sont regroupés (Fedora = `fedora` + `updates`, RPM Fusion
  libre = `rpmfusion-free` + `rpmfusion-free-updates`…) avec un état clair (Active, Inactive,
  Partielle, Non installée). Les **sources de test** (`*-updates-testing`, `flathub-beta`,
  `fedora-testing`) sont séparées et leur activation demande confirmation. Les dépôts de
  débogage et de sources sont masqués.
- Ajout de sources : RPM Fusion (libre / non libre), COPR, fichier .repo, Flathub (système ou
  votre compte), Flathub bêta, .flatpakrepo, retrait du filtre Fedora sur Flathub.
- Les opérations admin passent par **un seul** `pkexec` par opération ; Flatpak passe par polkit.
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

L'option `apply_to_all_apps` étend les couleurs aux applis externes : `~/.config/gtk-4.0/gtk.css`
(libadwaita), `~/.config/gtk-3.0/gtk.css` (GTK3, thème adw-gtk3), palette **qt5ct / qt6ct**
pour Qt 5 et 6 (la session exporte `QT_QPA_PLATFORMTHEME=qt6ct`) et la couleur d'accent GNOME
la plus proche (seule couleur visible des applis Flatpak). La disposition des boutons
(`button_layout`) est aussi écrite dans `org.gnome.desktop.wm.preferences button-layout` :
les applis GTK3, GTK4, libadwaita et Qt (barre de titre « Adwaita »,
`QT_WAYLAND_DECORATION=adwaita`) ont alors réduire / agrandir / fermer, et pas seulement
fermer comme le veut Fedora. Attention : ces réglages sont également lus par vos autres
sessions (GNOME…).

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

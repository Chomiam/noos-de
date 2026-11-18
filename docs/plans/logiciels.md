# Logiciels — état des lieux et idées

Novembre 2026. Code : `noos-software/src/` (interface GTK4 + libAdapta), `noos-pkg/src/`
(bibliothèque et CLI `noos-pkg` : backends apt et Flatpak, sources, AppImage, .deb, fiches).
Ce document est une **proposition** : rien de la partie 3 n'est fait, sauf mention contraire.

## 1. État des lieux

### Pages

| Page | Ce qu'elle fait | Limites constatées |
|---|---|---|
| **Explorer** | Vitrine Flathub (API v2, cache 6 h dans `~/.cache/noos-software`) : bannière « applis de la semaine », 10 catégories, populaires, tendances, nouveautés ; fiche Flathub (captures, description, licence, site). Champ de recherche **apt + Flatpak** (filtre par source), résultats fusionnés par identifiant AppStream (DEP-11) : une ligne « GIMP » propose Flatpak · Flathub ou apt · Debian. | La vitrine n'existe que pour Flathub (rien pour les paquets Debian, qui ont pourtant leurs catégories DEP-11). Recherche apt sur le **nom seulement** (`--names-only`), 150 paquets apt au plus : « gimp » remonte 40 `gimp-help-*` sous l'application. Pas de recherche par mot-clé (« retouche photo »). Résumés apt souvent en anglais dans la liste. |
| **Installées** | AppImage en tête (Ouvrir, Supprimer, Intégrer, Ajouter…), puis Flatpak, puis paquets apt **installés explicitement** (hors priorité required / important), filtre texte. | 300 lignes affichées par groupe (liste non virtualisée) ; ni taille, ni date d'installation, ni tri ; rien sur les données des applis (`~/.var/app`). |
| **Mises à jour** | Recherche sans mot de passe (`apt-get update` dans `~/.cache/noos/apt`, `apt-get -s full-upgrade`, `flatpak remote-ls --updates`), résultat partagé avec la barre et Paramètres (`updates.json`, timer `noos-update-check`), « Tout mettre à jour » par gestionnaire. | Pas de sélection par paquet, pas de notes de version ni de changelog, pas de distinction « système » / « applications », aucune mise à jour automatique, rien après l'installation (redémarrage nécessaire ? noyau ?). |
| **Sources** | Dépôts regroupés en sources lisibles (Debian sid, NOOS, tiers), états Active / Partielle / Inactive / Non installée, sources de test à part avec confirmation, ajout de dépôts APT (adresse, suite, composants, clé), contrib / non-free, i386, Flathub (système / compte), Flathub bêta, `.flatpakrepo`. | Pas de vérification de la signature ni de l'accessibilité d'un dépôt avant de l'ajouter ; pas de miroir le plus rapide ; pas de priorités (pinning). |
| **AppImage** | Ouverture d'un `.AppImage` : nom, version, icône lus **sans l'exécuter** ; installation dans `~/Applications` + lanceur du menu Démarrer ; nouvelle version → l'ancienne à la corbeille. | Pas de mises à jour (zsync / `X-AppImage-Update`), pas de catalogue, pas de bac à sable. |
| **.deb** | Fiche (`dpkg-deb -f`), simulation `apt-get -s install <fichier>` (dépendances, mises à jour, conflits), installation par la file, « Ouvrir ». | Aucune alerte sur le dépôt que le paquet ajoute souvent lui-même (Chrome, VS Code…). |

Commun à toutes les pages : **file d'opérations** séquentielle (`ops.rs`, un thread, journal en
direct, Ctrl+L), un `pkexec` par opération apt, Flatpak sans mot de passe en `--user`.

### Fait sur cette branche : fiche détaillée d'un paquet

Avant, une ligne de résultat de recherche (ou de la page Installées) n'ouvrait rien : seules les
tuiles de la vitrine Flathub avaient une fiche. Désormais, un clic sur la ligne ouvre une fiche
(`noos-software/src/details.rs`, données `Backend::details` / `Backend::simulate` de
`noos-pkg/src/details.rs`) :

- nom AppStream, résumé et description longue **traduits** (`apt-cache show` avec
  `Acquire::Languages=fr,en`, paragraphes « . » ; Flatpak : AppStream du remote) ;
- captures et icône AppStream (DEP-11 de Debian — images JPEG XL — ou `appstream.xml` de Flatpak) ;
- versions installée / disponible, tailles téléchargée / installée, source, dépôt
  (`deb.debian.org/debian · sid/main`), catégorie Debian en français, licence (AppStream ou
  `copyright` lisible par machine), développeur et mainteneur, commandes fournies, runtime ;
- dépendances / recommandations / suggestions ;
- **simulation** sans mot de passe de ce qu'installer ajoutera (`apt-get -s install`, runtime
  Flatpak manquant) ou de ce que supprimer retirera (`apt-get -s remove`) ;
- boutons Installer / Supprimer / Ouvrir ; un onglet par façon de l'installer quand le logiciel
  existe en apt et en Flatpak (comparaison directe).

## 2. Ce qui manque le plus (synthèse)

1. **Trouver** : la recherche ne comprend que les noms de paquets, et les sous-paquets noient
   l'application. Aucune vitrine pour ce que Debian propose.
2. **Avoir confiance** : on ne voit ni les permissions d'une appli Flatpak, ni les avis, ni ce qui
   a été installé hier et comment revenir en arrière.
3. **Entretenir** : rien pour faire de la place (orphelins, cache, runtimes inutilisés), rien
   d'automatique pour les mises à jour.

## 3. Idées classées par impact et effort

Impact : ★★★ change l'usage quotidien, ★★ utile souvent, ★ confort. Effort : **F** (≤ 1 jour),
**M** (quelques jours), **É** (une semaine ou plus, ou dépend d'un service externe).

### 3.1 Gros impact, effort faible ou moyen — à faire d'abord

| # | Idée | Impact | Effort | Comment |
|---|---|---|---|---|
| 1 | **Recherche qui trouve les applications** | ★★★ | F | Rechercher aussi dans les descriptions (`apt-cache search` sans `--names-only`, poids plus faible), dans les mots-clés et noms traduits AppStream (DEP-11 `Keywords`, `Name: fr`). Regrouper sous l'application les sous-paquets `-help-*`, `-l10n-*`, `-data`, `-doc`, `-dbgsym` (« et 38 paquets associés »). Filtre « Applications seulement » (composants `desktop-application`) activé par défaut. |
| 2 | **Nettoyage** (nouvelle section d'Installées ou de Paramètres › Stockage) | ★★★ | F–M | Paquets orphelins (`apt-get -s autoremove`), cache apt (`/var/cache/apt/archives`, `apt-get clean`), runtimes Flatpak inutilisés (`flatpak uninstall --unused`), données d'applis Flatpak désinstallées (`~/.var/app/<id>` sans appli), anciens noyaux (garder le courant et le précédent), AppImage dont le lanceur pointe dans le vide. Taille récupérable affichée avant d'agir, une seule élévation. |
| 3 | **Historique des opérations et retour en arrière** | ★★★ | M | Journal persistant (`~/.local/state/noos/software-history.json`) + lecture de `/var/log/apt/history.log` (opérations faites en terminal). Chaque opération apt a déjà un instantané « avant » de `noos-save` (`/run/noos-save/apt-pre`) : bouton « Revenir à l'état d'avant » qui ouvre noos-save sur cet instantané ; pour une simple installation, « Désinstaller ce qui a été ajouté » (paquets de la ligne `Install:`). |
| 4 | **Permissions Flatpak** (façon Flatseal) | ★★★ | M | Avant l'installation : `flatpak remote-info --show-metadata` → section « Accès » de la fiche (dossier personnel, réseau, périphériques, X11, bus système), avec avertissement pour les accès larges. Après : `flatpak info --show-permissions`, interrupteurs qui écrivent `flatpak override --user`, « Rétablir ». Une page dans la fiche des applis installées. |
| 5 | **Mises à jour automatiques** | ★★★ | M | Réglage dans Paramètres › Mises à jour : Flatpak en arrière-plan (le timer existant appelle `flatpak update -y --user` puis système via une règle polkit dédiée) ; apt : sécurité seulement par `unattended-upgrades` (opt-in, Debian sid n'a pas de suite security : à réserver aux paquets NOOS / Flatpak) ou « installer à l'arrêt » (`system-update.target`, comme GNOME Logiciels). Notification « Redémarrage nécessaire » si le noyau ou la libc change. |
| 6 | **Paquets recommandés selon le matériel** | ★★★ | M | Debian fournit `isenkram-lookup` (modalias → paquets) : firmwares manquants (`dmesg` « failed to load firmware »), pilote NVIDIA (`nvidia-detect`), microcode (`intel-microcode` / `amd64-microcode`), `steam-devices` si une manette est branchée, pilotes d'imprimante (lien avec Paramètres › Imprimantes). Bandeau « 3 compléments recommandés pour votre matériel » dans Explorer. Partage de code avec l'installateur (`hardware.rs`). |
| 7 | **File d'attente visible** | ★★ | F | `ops.rs` exécute déjà les opérations l'une après l'autre : afficher la file (bouton de la barre d'en-tête avec compteur), retirer une opération en attente, **regrouper** plusieurs installations apt en une seule commande (un seul mot de passe), reprise après refus de pkexec. |
| 8 | **Mises à jour détaillées** | ★★ | F | Sélection par paquet, groupes « Système » / « Applications » (paquets ayant un composant AppStream), notes de version (AppStream `Releases`, `apt-get changelog`), taille totale, mention « redémarrage nécessaire ». Clic sur une mise à jour = la nouvelle fiche. |
| 9 | **Ouvrir une fiche de partout** | ★★ | F | Argument `noos-software --details <id>` (la logique existe : `open_details_spec`, aujourd'hui seulement via `NOOS_SOFTWARE_DETAILS`) ; gestionnaire d'URI `appstream://` et `flatpak+https://` / `.flatpakref` ; menu Démarrer et recherche de la barre : « Désinstaller », « Fiche dans Logiciels ». |
| 10 | **Vitrine pour Debian** | ★★ | M | Catégories DEP-11 (`Categories:` des composants) mêlées à celles de Flathub, « Sélection NOOS » éditoriale : fichier JSON dans le dépôt (comme le catalogue d'animations `noos-shader`), téléchargé depuis GitHub avec repli sur la copie du paquet. Packs à installer en un clic (Bureautique, Création, Développement, Jeux : Steam + Lutris + Heroic + ProtonUp-Qt). |

### 3.2 Impact moyen

| # | Idée | Impact | Effort | Comment |
|---|---|---|---|---|
| 11 | **Notes et avis ODRS** | ★★ | M | API publique `odrs.gnome.org` (celle de GNOME Logiciels et KDE Discover) : note moyenne et avis dans la langue de l'utilisateur sur la fiche, signalement, rédaction d'un avis. Requêtes avec un identifiant anonyme (hachage local), **désactivées par défaut** (réglage « Afficher les avis »). |
| 12 | **command-not-found** | ★★ | M | Gestionnaire `fish_command_not_found` / `command_not_found_handle` (bash) qui appelle `noos-pkg provides <commande>` : index construit depuis le champ `Commands` (lu par la fiche) ou la base du paquet Debian `command-not-found` (`apt-file`). « gimp : commande introuvable. Installer avec Logiciels ? [o/N] » + Flatpak correspondant. |
| 13 | **Comparaison apt / Flatpak / AppImage** | ★★ | M (É pour AppImage) | Les onglets de la fiche comparent déjà apt et Flatpak. Ajouter un tableau : version, taille, fréquence des mises à jour, isolement, permissions, source vérifiée ; recommandation (« Flatpak : plus récent de 2 versions »). AppImage : catalogue AppImageHub / GitHub Releases, plus coûteux. |
| 14 | **Versions et épinglage** | ★★ | M | apt : « Garder cette version » (`apt-mark hold`), « Revenir à la version précédente » si elle est encore dans le cache ou un dépôt, choix de la suite (sid / experimental) pour un seul paquet (pinning `/etc/apt/preferences.d/`). Flatpak : branche stable / bêta, `flatpak update --commit` pour revenir en arrière. |
| 15 | **Espace et données des applis** | ★★ | F | Taille installée + données (`~/.var/app/<id>`, `~/.config/<app>` pour apt) dans Installées, tri par taille, « Effacer les données » d'une appli Flatpak. |
| 16 | **Notifications et travail en arrière-plan** | ★★ | F | `gio::Notification` à la fin d'une opération quand la fenêtre est cachée ; fermer la fenêtre pendant une installation la garde en arrière-plan (comme le shell le fait pour l'agent polkit). |
| 17 | **Sécurité des sources** | ★★ | M | À l'ajout d'un dépôt : vérifier qu'il répond et que la clé signe bien `InRelease` ; signaler les dépôts sans `Signed-By`, les applis Flatpak en fin de vie (`end-of-life` des métadonnées), les Flatpak non vérifiés ; `debsecan` pour les vulnérabilités connues des paquets installés. |
| 18 | **Listes virtualisées** | ★ | M | `gtk::ListView` + modèle pour Installées et les résultats (aujourd'hui `ListBox`, coupé à 300 / 120 lignes) : défilement fluide, tri, plus de limite. |

### 3.3 Plus tard / à étudier

| # | Idée | Impact | Effort | Remarque |
|---|---|---|---|---|
| 19 | Mises à jour des AppImage (zsync, `X-AppImage-Update`) | ★ | M | Seules certaines AppImage l'intègrent. |
| 20 | Logiciels d'autres distributions via **Distrobox** (AUR, paquets Fedora) | ★ | É | Conteneur + export du lanceur ; intéressant pour les joueurs, à concevoir avec soin. |
| 21 | Miroir Debian le plus rapide (test de débit) | ★ | F | `deb.debian.org` (CDN) suffit dans la plupart des cas. |
| 22 | Mode hors ligne explicite (bandeau, vitrine en cache) | ★ | F | La vitrine affiche aujourd'hui « Fiche indisponible ». |
| 23 | Paquets `.rpm` (via `alien`) | ★ | M | Rarement une bonne idée ; plutôt proposer le Flatpak équivalent. |

## 4. Proposition d'ordre

1. Recherche (1) + fiche ouverte de partout (9) : on trouve, on comprend, on installe.
2. Nettoyage (2) + file d'attente (7) + mises à jour détaillées (8) : entretien sans terminal.
3. Historique / retour arrière (3) et permissions Flatpak (4) : confiance.
4. Matériel (6), mises à jour automatiques (5), vitrine Debian (10).
5. Avis ODRS (11), command-not-found (12), comparaison (13), versions (14).

Chaque étape se teste dans l'aperçu (`tools/preview/run_app.sh`, variables `NOOS_SOFTWARE_*`) et,
pour ce qui touche apt en root, en VM.

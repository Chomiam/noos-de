# Mode édition du bureau — état des lieux et plan

Novembre 2026. Code : `noos-shell/src/editmode.rs` (barre d'outils, barres), `noos-shell/src/desk.rs`
(widgets posés sur le bureau), `noos-shell/src/widgets/` (un fichier par widget),
`noos-shell/src/layouts.rs` (mises en page), logique pure dans `noos-core/src/layout.rs`
(instantanés, annuler / rétablir, modèles, aimantation) et `noos-core/src/weather.rs`.

## 1. État des lieux (avant)

Aperçu imbriqué, configuration isolée, `NOOS_EDIT_DEMO=1` : barre d'outils sur une ligne
(« Mode édition », « Ajouter un widget », « Dock : » + 4 flèches, « Barre fine », « Ajouter à la
barre », ↺, « Terminé »), boucliers en pointillé sur les éléments de la barre, widgets
déplaçables par une pastille, cadre « Les notifications apparaîtront ici ».

Ce qui gêne concrètement :

| Domaine | Constat |
|---|---|
| Vocabulaire | « Barre fine » ne dit ni où elle est, ni à quoi elle sert. « Dock » (barre d'outils) et « Barre des tâches » (Paramètres) désignent la même chose. « Ajouter à la barre » ne liste en fait que les éléments *retirés* (vide la plupart du temps). |
| Découvrabilité | Le mode d'emploi n'existe que dans l'info-bulle du titre. Rien n'indique qu'on peut glisser les icônes de la barre ; aucun raccourci clavier hormis Échap. |
| Annulation | Aucune. Le bouton ↺ (icône « annuler ») **réinitialise** toute la barre sans confirmation ; un widget retiré l'est définitivement (le texte d'une note est perdu). |
| Bord de la barre | Quatre flèches (↓ ↑ ← →) sans schéma : on ne sait pas si la flèche montre le bord ou le sens du déplacement. |
| Glisser-déposer dans la barre | La zone survolée s'éclaire, mais rien ne montre *où* l'élément va s'insérer. Les « × » rouges recouvrent les icônes (horloge coupée, « × » de deux éléments voisins collés). Pas d'autre moyen que la souris (pas de menu « Déplacer vers… »). |
| Widgets | Menu en liste brute (4 noms, sans aperçu ni description), widget posé toujours au même endroit de l'écran principal. Déplacement seulement par une petite pastille ; pas d'alignement, pas de grille, pas de taille, aucun réglage (format de l'horloge, couleur de la note…). Le cadre en pointillé de l'horloge coupe le texte. |
| Recouvrements | La barre d'outils masque les widgets placés en haut ; le cadre des notifications peut recouvrir la barre. |
| Plusieurs écrans | Un widget ne peut pas changer d'écran ; rien n'indique quel écran est lequel. |
| Mises en page | Aucune disposition toute faite, impossible d'enregistrer / partager la sienne. |
| Catalogue | 4 widgets (horloge, ressources, note, calendrier GTK brut). |

Capture « avant » : `docs/plans/captures-mode-edition/avant.jpg`.

## 2. Vocabulaire

- **Barre des tâches** : la barre principale, partout (barre d'outils, Paramètres, guide).
  « Dock » disparaît de l'interface.
- **Barre d'état** (au lieu de « barre fine ») : la seconde barre, au bord opposé. Le nom dit
  *ce qu'elle montre* (bureaux virtuels, horloge, réseau, notifications : l'état du système),
  comme la barre d'état d'un téléphone ou la barre supérieure de GNOME / macOS. « Barre du
  haut » serait faux quand la barre des tâches est en haut (elle passe alors en bas) ;
  « barre secondaire » ne dit rien de son rôle. Clé de configuration `[statusbar]`,
  l'ancienne `[topbar]` est toujours lue.
- **Éléments retirés** au lieu de « Ajouter à la barre ».
- **Mises en page** : dispositions complètes (barres + notifications, et widgets pour les
  mises en page enregistrées).

## 3. Idées, classées

Impact : ★★★ fort, ★★ moyen, ★ confort. Effort : S (quelques heures), M (une journée),
L (plusieurs jours). ✅ = réalisé dans cette version.

### A. Plus simple à utiliser

| Idée | Impact | Effort | État |
|---|---|---|---|
| Nouvelle barre d'outils : boutons nommés (Widgets, Barres, Mises en page), ligne d'aide contextuelle sous la barre, apparition animée | ★★★ | M | ✅ |
| **Annuler / Rétablir** (Ctrl+Z, Ctrl+Maj+Z, Ctrl+Y) pour tout : déplacements, retraits, bord, barre d'état, notifications, widgets, mises en page ; message « … — Annuler » après chaque action | ★★★ | M | ✅ |
| Choix du bord par **schémas d'écran** (barre dessinée au bon bord) au lieu de flèches | ★★ | S | ✅ |
| **Indicateur d'insertion** (trait d'accent) pendant le glisser dans la barre | ★★ | S | ✅ |
| « × » discrets (au survol seulement), menu clic droit sur un élément en mode édition : « Déplacer vers… » (gauche / centre / droite, barre d'état), « Retirer » | ★★ | S | ✅ |
| Widgets déplacés **par tout leur corps** en mode édition (plus seulement la pastille), barre de titre au-dessus du widget sans recouvrir son contenu | ★★★ | M | ✅ |
| **Grille magnétique** + **repères d'alignement** (bords, centres, autres widgets, marges de l'écran) ; Alt maintenu = déplacement libre | ★★★ | M | ✅ |
| **Poignée de redimensionnement** (petit / moyen / grand, changement en direct) | ★★ | M | ✅ |
| Bureau assombri + grille de points en mode édition, nom de chaque écran affiché | ★★ | S | ✅ |
| **Visite guidée** au premier lancement (5 étapes, bouton « ? » pour la revoir) | ★★ | S | ✅ |
| Raccourcis : Échap / Entrée (terminer), Ctrl+Z / Ctrl+Y, W (widgets), Ctrl+F (recherche de widgets) | ★ | S | ✅ |
| Bouton « Afficher le bureau » (réduire les fenêtres qui cachent les widgets) | ★ | S | ✅ |
| Zoom arrière du bureau façon KDE Plasma (miniature de l'écran, barres autour) | ★★ | L | — |
| Navigation entièrement au clavier (Tab entre widgets, flèches pour déplacer) | ★ | M | — |

### B. Fonctions

| Idée | Impact | Effort | État |
|---|---|---|---|
| **Galerie de widgets** : cartes illustrées, description, recherche, catégories ; **glisser une carte sur le bureau** (n'importe quel écran) ou cliquer pour la placer à un endroit libre | ★★★ | M | ✅ |
| **Réglages par widget** (popover ⚙) : taille, style (verre / plein / sans fond), écran, réglages propres au widget, dupliquer, retirer — aperçu en direct | ★★★ | M | ✅ |
| **Mises en page prédéfinies** avec miniature dessinée : Windows 11, Classique, Barre d'état (façon macOS), GNOME, Verticale, Épurée | ★★★ | M | ✅ |
| **Mes mises en page** : enregistrer la disposition actuelle (barres + widgets), la réappliquer ; **exporter / importer** un fichier `.toml` | ★★ | M | ✅ |
| Réinitialiser (avec annulation possible) | ★ | S | ✅ |
| Plusieurs écrans : glisser un widget de la galerie sur n'importe quel écran, « Écran » dans les réglages du widget | ★★ | S | ✅ |
| Barre d'outils de chaque écran secondaire (aujourd'hui : écran principal) | ★ | M | — |
| Glisser un widget directement d'un écran à l'autre | ★ | L | — |
| Widgets dans la barre (météo, lecteur multimédia compacts) | ★★ | L | — |
| Mise en page par bureau virtuel | ★ | L | — |
| Raccourci global pour entrer dans le mode édition (compositeur) | ★ | S | — |

### C. Widgets

| Widget | Contenu | État |
|---|---|---|
| **Météo** | Open-Meteo (sans compte ni clé) : température, ressenti, conditions, min / max, vent, humidité, prévisions sur 5 jours (grand format) ; fond qui suit le temps et le jour / la nuit ; recherche de ville ; °C / °F ; cache hors ligne | ✅ |
| **Horloges du monde** | Plusieurs fuseaux : cadran analogique jour / nuit, heure, décalage, « Demain » / « Hier » ; recherche parmi les fuseaux du système | ✅ |
| **Lecteur multimédia** | MPRIS (Firefox, Spotify, VLC, mpv…) : pochette, fond flouté, titre, artiste, progression, précédent / lecture / suivant | ✅ |
| **Compte à rebours** | Événement nommé, date et heure, anneau de progression, « C'est aujourd'hui ! » | ✅ |
| **Cadre photo** | Images d'un dossier en fondu enchaîné, intervalle réglable, ouverture dans Photos | ✅ |
| **Dossiers** | Raccourcis vers des dossiers (Documents, Téléchargements…), ajout / retrait | ✅ |
| **Horloge** (revue) | Numérique ou **analogique**, secondes, date, trois tailles | ✅ |
| **Calendrier** (revu) | Grille du mois façon Windows 11, numéros de semaine, **prochain jour férié** (France) | ✅ |
| **Note** (revue) | Cinq couleurs, trois tailles | ✅ |
| Ressources | Existant, style verre | ✅ |
| Batterie, presse-papiers, agenda (calendriers en ligne), flux RSS, citations | — | — |

## 4. Choix techniques

- **Configuration** : `[[desktop_widget]]` gagne `id` (stable, généré au chargement pour les
  anciens fichiers), `size` (`s` / `m` / `l`, `m` par défaut), `style` (`glass` / `solid` /
  `clear`) et une table `options` libre par widget (ex. `city`, `zones`, `target`). Les anciens
  fichiers restent valides.
- **Instantané de mise en page** (`noos_core::layout::Layout`) : bord, alignement, zones des
  deux barres, éléments retirés et masqués, barre d'état, coin des notifications, widgets.
  Annuler / rétablir = pile d'instantanés (`History`, 50 niveaux) ; les modèles et les fichiers
  exportés sont des `Layout` sérialisés en TOML.
- **Aimantation** (`layout::snap`) : fonction pure testée ; grille de 16 px, repères à 10 px.
- **Rendu** : les widgets restent dans la surface `noos-desktop-widgets` (couche « bottom ») —
  jamais dans le fond d'écran (flou Mica recalculé à chaque image). Les repères et la grille
  sont une `DrawingArea` de cette même surface, visible seulement en mode édition.
- **Réseau** : `curl` (comme Bing / Wallhaven), dans un thread ; rien n'est envoyé hors de la
  ville choisie par l'utilisateur.

## 5. Vérification et variables de démo

Aperçu imbriqué, configuration isolée (`tools/preview/scenario.sh`), un et deux écrans
(`NOOS_WINIT_OUTPUTS=2`), barre des tâches en bas, en haut et à gauche. Captures « après » dans
`docs/plans/captures-mode-edition/` : `bureau.jpg` (tous les widgets, vraie météo),
`edition.jpg`, `galerie.jpg`, `barres.jpg`, `mises-en-page.jpg`, `reglages.jpg`,
`visite.jpg`, `annuler.jpg`, `deux-ecrans.jpg`, `barre-en-haut.jpg`, `verticale.jpg`.

| Variable | Effet |
|---|---|
| `NOOS_EDIT_DEMO=1` | mode édition ouvert après 2,5 s (sans visite guidée automatique) |
| `NOOS_EDIT_OPEN=étape;étape…` | étapes jouées toutes les `NOOS_EDIT_STEP_MS` (1500 ms) : `widgets`, `bars`, `layouts`, `settings:n`, `toast:texte`, `preset:<id>`, `add:<type>`, `remove:n`, `undo`, `redo`, `tour:n`, `guides:x,y` |
| `NOOS_EDIT_SEARCH=mot` | recherche préremplie dans la galerie |
| `NOOS_EDIT_TOUR=n` | visite guidée affichée à l'étape n |
| `NOOS_EDIT_GUIDES=x,y` | widget 0 « déplacé » en (x, y) pour montrer les repères |
| `NOOS_WEATHER_DEMO=1` / `NOOS_MEDIA_DEMO=1` | météo / lecture fictives (sans réseau ni lecteur) |

Non vérifié dans l'aperçu (on ne peut pas y cliquer) : les gestes à la souris eux-mêmes
(glisser un widget, une carte de la galerie, un élément de barre, la poignée de taille). Leur
logique est commune avec ce qui est montré (`NOOS_EDIT_GUIDES` passe par le même calcul
d'aimantation, `add:` par le même ajout que le dépôt d'une carte) ; à essayer dans la vraie
session.

## 6. Reste à faire

Les lignes « — » ci-dessus, et : tester sur vrai matériel (plusieurs écrans physiques, HiDPI),
traduire les noms de villes des fuseaux, widget d'agenda branché sur les calendriers en ligne
(GNOME Online Accounts / CalDAV).

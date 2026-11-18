#!/usr/bin/env bash
# Fichiers système du mode jeu Steam : ./install-system.sh [RACINE]
# (RACINE = /). Utilisé par le paquet Debian et la construction de l'ISO.
#   /usr/libexec/noos-gaming-helper   réglages root (pkexec, org.noos.gaming.helper)
#   /usr/libexec/noos-session-switch  bascule bureau ↔ mode jeu (pkexec, org.noos.gaming.switch)
#   /usr/libexec/noos-steam-session   la session gamescope + Steam
#   /usr/lib/noos/steamos/            outils attendus par Steam en mode -steamos3
#                                     (steamos-session-select…), ajoutés au PATH de la session
# Le fichier de session (wayland-sessions/noos-steam.desktop) n'est créé qu'à l'activation.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
install -Dm755 "$HERE/system/noos-gaming-helper" "$ROOT/usr/libexec/noos-gaming-helper"
install -Dm755 "$HERE/system/noos-session-switch" "$ROOT/usr/libexec/noos-session-switch"
install -Dm755 "$HERE/session/noos-steam-session" "$ROOT/usr/libexec/noos-steam-session"
install -Dm644 "$HERE/system/org.noos.gaming.policy" "$ROOT/usr/share/polkit-1/actions/org.noos.gaming.policy"
for f in "$HERE"/steamos/*; do
    install -Dm755 "$f" "$ROOT/usr/lib/noos/steamos/$(basename "$f")"
done

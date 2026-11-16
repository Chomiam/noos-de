#!/usr/bin/env bash
# Installe l'entrée « NOOS » dans l'écran de connexion existant (GDM, SDDM, LightDM) d'une
# Debian de développement, sans le paquet noos. À faire UNE seule fois, sur l'hôte (pas dans
# un conteneur distrobox) :
#
#     sudo ./noos-session/install-session.sh
#
# Tout va dans /usr/local : rien n'est mélangé aux fichiers des paquets Debian.
# Désinstallation : sudo ./noos-session/install-session.sh --uninstall
# (Avec le paquet noos installé, cette étape est inutile : la session est déjà fournie.)
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "À lancer avec sudo : sudo $0" >&2
    exit 1
fi
if [ -f /run/.containerenv ] || [ -f /.dockerenv ]; then
    echo "Ce script doit tourner sur l'hôte, pas dans un conteneur." >&2
    exit 1
fi

HERE="$(cd "$(dirname "$0")" && pwd)"
BIN=/usr/local/bin/noos-session
SESSION=/usr/local/share/wayland-sessions/noos.desktop
PORTALS=/usr/local/share/xdg-desktop-portal/noos-portals.conf

if [ "${1:-}" = "--uninstall" ]; then
    rm -fv "$BIN" "$SESSION" "$PORTALS"
    exit 0
fi

install -Dm755 "$HERE/noos-session" "$BIN"
install -Dm644 "$HERE/noos.desktop" "$SESSION"
install -Dm644 "$HERE/noos-portals.conf" "$PORTALS"

echo
echo "Session NOOS installée :"
echo "  $BIN"
echo "  $SESSION"
echo "  $PORTALS"
echo
echo "Déconnectez-vous et choisissez la session « NOOS » dans l'écran de connexion."
echo "Journal de session : ~/.local/state/noos/session.log"

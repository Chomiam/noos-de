#!/usr/bin/env bash
# Installe l'entrée « NOOS » dans l'écran de connexion (GDM). À faire UNE seule fois,
# sur l'hôte (pas dans un toolbox) :
#
#     sudo ./noos-session/install-session.sh
#
# Sur Fedora Silverblue, /usr est en lecture seule mais /usr/local (= /var/usrlocal) est
# modifiable : rien n'est superposé à l'image système, aucun redémarrage n'est nécessaire.
# Désinstallation : sudo ./noos-session/install-session.sh --uninstall
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
    echo "À lancer avec sudo : sudo $0" >&2
    exit 1
fi
if [ -f /run/.containerenv ]; then
    echo "Ce script doit tourner sur l'hôte, pas dans un toolbox." >&2
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
echo "Déconnectez-vous, cliquez sur votre nom puis sur l'engrenage en bas à droite de GDM"
echo "et choisissez « NOOS ». Journal de session : ~/.local/state/noos/session.log"

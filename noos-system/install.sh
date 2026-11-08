#!/usr/bin/env bash
# Réglages système de NOOS : ./install.sh [RACINE] — RACINE = / par défaut (le paquet RPM et la
# construction de l'ISO passent leur racine). Les fichiers vont dans /usr (défauts du paquet) :
# l'administrateur garde la main via /etc.
#
# - NetworkManager : Wi-Fi sans économie d'énergie
# - udev : pas de mise en veille automatique des cartes réseau (Ethernet, Wi-Fi)
# - WirePlumber : pas de mise en veille des nœuds audio (coupures de son au réveil)
# - sudo : « * » affichés pendant la saisie du mot de passe (pwfeedback)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(realpath -m "${1:-/}")"   # absolu : on change de dossier ensuite
cd "$HERE/root"
find . -type f | while read -r f; do
    case "$f" in
    ./etc/sudoers.d/*) install -Dm440 "$f" "$ROOT/${f#./}" ;;   # sudo refuse un autre mode
    ./usr/libexec/*|./usr/lib/kernel/install.d/*) install -Dm755 "$f" "$ROOT/${f#./}" ;;
    *) install -Dm644 "$f" "$ROOT/${f#./}" ;;
    esac
done

#!/usr/bin/env bash
# Fichiers système du pare-feu NOOS : ./install-system.sh [RACINE] [PRÉFIXE]
# (RACINE = / ; PRÉFIXE = /opt/noos). Utilisé par le paquet Debian, l'ISO et install-host.sh.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
PREFIX="${2:-/opt/noos}"
sed "s|/opt/noos/bin/|$PREFIX/bin/|g" "$HERE/system/noos-firewall.service" | install -Dm644 /dev/stdin "$ROOT/usr/lib/systemd/system/noos-firewall.service"
install -Dm644 "$HERE/system/org.noos.firewall.policy" "$ROOT/usr/share/polkit-1/actions/org.noos.firewall.policy"
sed "s|/opt/noos/bin/|$PREFIX/bin/|g" "$HERE/system/org.noos.FirewallPrompt.desktop" | install -Dm644 /dev/stdin "$ROOT/etc/xdg/autostart/org.noos.FirewallPrompt.desktop"
install -Dm644 "$HERE/system/modules-load.conf" "$ROOT/usr/lib/modules-load.d/noos-firewall.conf"

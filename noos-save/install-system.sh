#!/usr/bin/env bash
# Fichiers système de « Sauvegarder et Restaurer » : ./install-system.sh [RACINE] [PRÉFIXE]
# (RACINE = / ; PRÉFIXE = /opt/noos). Utilisé par le paquet RPM et la construction de l'ISO.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
PREFIX="${2:-/opt/noos}"
install -Dm755 "$HERE/noos-save-helper" "$ROOT$PREFIX/libexec/noos-save-helper"
install -Dm644 "$HERE/system/noos-save.actions" "$ROOT/etc/dnf/libdnf5-plugins/actions.d/noos-save.actions"
install -Dm644 "$HERE/system/org.noos.save.policy" "$ROOT/usr/share/polkit-1/actions/org.noos.save.policy"
install -Dm755 "$HERE/system/42_noos_snapshots" "$ROOT/etc/grub.d/42_noos_snapshots"
for u in noos-snapshots-grub.path noos-snapshots-grub.service noos-snapshot-boot.service; do
    install -Dm644 "$HERE/system/$u" "$ROOT/usr/lib/systemd/system/$u"
done

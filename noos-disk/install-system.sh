#!/usr/bin/env bash
# Fichiers système de « Disques » : ./install-system.sh [RACINE] [PRÉFIXE]
# (RACINE = / ; PRÉFIXE = /opt/noos). Utilisé par le paquet RPM et la construction de l'ISO.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
PREFIX="${2:-/opt/noos}"
install -Dm755 "$HERE/noos-disk-helper" "$ROOT$PREFIX/libexec/noos-disk-helper"
install -Dm644 "$HERE/system/org.noos.disk.policy" "$ROOT/usr/share/polkit-1/actions/org.noos.disk.policy"

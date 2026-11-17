#!/usr/bin/env bash
# Fichiers système du partage de fichiers : ./install-system.sh [RACINE] [PRÉFIXE]
# (RACINE = / ; PRÉFIXE = /opt/noos). Utilisé par le paquet Debian et la construction de l'ISO.
# Les binaires (noos-share-helper dans libexec, noos-webdav dans bin) sont copiés à part.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
PREFIX="${2:-/opt/noos}"
sed "s|/opt/noos/libexec/|$PREFIX/libexec/|g" "$HERE/system/org.noos.share.policy" \
    | install -Dm644 /dev/stdin "$ROOT/usr/share/polkit-1/actions/org.noos.share.policy"
sed "s|/opt/noos/bin/|$PREFIX/bin/|g" "$HERE/system/noos-webdav.service" \
    | install -Dm644 /dev/stdin "$ROOT/usr/lib/systemd/user/noos-webdav.service"

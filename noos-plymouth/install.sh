#!/usr/bin/env bash
# Installe le thème Plymouth NOOS (root) : ./install.sh [RACINE] — RACINE = / par défaut
# (l'installateur et la construction de l'ISO passent la racine du système cible).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="${1:-/}"
DEST="$ROOT/usr/share/plymouth/themes/noos"
[ -f "$HERE/noos/logo.png" ] || python3 "$HERE/generate.py"
install -d "$DEST"
install -m644 "$HERE"/noos/*.png "$HERE/noos/noos.plymouth" "$HERE/noos/noos.script" "$DEST/"
echo "Thème Plymouth NOOS installé dans $DEST"
echo "Activation : plymouth-set-default-theme -R noos   (dans le système cible)"

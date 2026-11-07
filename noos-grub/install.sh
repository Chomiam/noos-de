#!/usr/bin/env bash
# Installe le thème GRUB NOOS : ./install.sh [RACINE] [--mkconfig]
#
# - copie le thème dans RACINE/boot/grub2/themes/noos (GRUB lit /boot, pas /usr) ;
# - règle GRUB_THEME, GRUB_TERMINAL_OUTPUT=gfxterm et GRUB_GFXMODE dans /etc/default/grub ;
# - --mkconfig : régénère /boot/grub2/grub.cfg (grub2-mkconfig charge alors les polices .pf2
#   du thème). L'installateur, lui, régénère grub.cfg à la fin de son étape GRUB.
#
# Utilisé par l'installateur (chroot du système installé), le paquet RPM (%post) et à la main.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="/"
MKCONFIG=0
for a in "$@"; do
    case "$a" in
    --mkconfig) MKCONFIG=1 ;;
    *) ROOT="$a" ;;
    esac
done
SRC="$HERE/noos"
[ -f "$SRC/theme.txt" ] || SRC="$HERE"    # copie installée : fichiers à côté du script
DEST="$ROOT/boot/grub2/themes/noos"
install -d "$DEST"
for f in "$SRC"/theme.txt "$SRC"/*.png "$SRC"/*.pf2; do
    install -m644 "$f" "$DEST/"
done

DEFAULT="$ROOT/etc/default/grub"
install -d "$(dirname "$DEFAULT")"
touch "$DEFAULT"
set_key() {
    if grep -q "^$1=" "$DEFAULT"; then
        sed -i "s|^$1=.*|$1=\"$2\"|" "$DEFAULT"
    else
        echo "$1=\"$2\"" >>"$DEFAULT"
    fi
}
set_key GRUB_THEME "/boot/grub2/themes/noos/theme.txt"
set_key GRUB_TERMINAL_OUTPUT "gfxterm"
grep -q '^GRUB_GFXMODE=' "$DEFAULT" || set_key GRUB_GFXMODE "auto"

if [ "$MKCONFIG" = 1 ]; then
    if [ "$ROOT" = "/" ]; then
        grub2-mkconfig -o /boot/grub2/grub.cfg >/dev/null 2>&1
    else
        chroot "$ROOT" grub2-mkconfig -o /boot/grub2/grub.cfg >/dev/null 2>&1
    fi
fi
echo "Thème GRUB NOOS installé dans $DEST"

#!/usr/bin/env bash
# Compile NOOS en release et l'installe dans ~/.local/opt/noos (sans sudo).
# À lancer sur Debian sid (hôte, ou conteneur distrobox noos-dev aux mêmes bibliothèques) :
#
#     ./scripts/install-user.sh
#
# libAdapta (absente de Debian) est copiée dans le préfixe ; le reste (libseat,
# gtk4-layer-shell, xdg-desktop-portal-wlr, xwayland) vient des paquets Debian :
#     sudo apt install libseat1 libgtk4-layer-shell0 xdg-desktop-portal-wlr xwayland grim
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${NOOS_PREFIX:-$HOME/.local/opt/noos}"

if ! grep -q '^ID=debian' /etc/os-release 2>/dev/null; then
    echo "Attention : ce script est prévu pour Debian sid (hôte ou conteneur noos-dev)." >&2
fi

"$ROOT/scripts/build-libadapta.sh"

cd "$ROOT"
cargo build --release -p noos-comp -p noos-shell -p noos-file -p noos-settings -p noos-software -p noos-pkg -p noos-snip -p noos-term -p noos-keys

mkdir -p "$PREFIX/bin" "$PREFIX/lib" "$PREFIX/share/noos" "$PREFIX/share/applications"

# Remplacement atomique (copie à côté puis renommage) : les programmes NOOS en cours
# d'exécution gardent l'ancienne version. Écraser une bibliothèque en place (cp) modifie
# le code déjà chargé en mémoire et fait planter la barre des tâches pendant la mise à jour.
put() { # put SOURCE DESTINATION MODE
    cp -P "$1" "$2.new.$$"
    [ -L "$2.new.$$" ] || chmod "$3" "$2.new.$$"
    mv -f "$2.new.$$" "$2"
}

for b in noos-comp noos-shell noos-file noos-settings noos-software noos-pkg noos-snip noos-term noos-keys; do
    put "target/release/$b" "$PREFIX/bin/$b" 755
done

# Bibliothèques manquantes sur l'hôte (copiées avec leurs liens symboliques).
# (Uniquement les fichiers et liens, pas le dossier d'objets meson libadapta-1.so.0.p.)
rm -rf "$PREFIX/lib/libadapta-1.so.0.p"
for f in target/libadapta-build/src/libadapta-1.so*; do
    [ -d "$f" ] || put "$f" "$PREFIX/lib/$(basename "$f")" 755
done
# Hôte sans libseat / gtk4-layer-shell (session lancée hors du conteneur) : copie depuis ici.
MULTIARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || echo x86_64-linux-gnu)"
for lib in libseat.so.1 libgtk4-layer-shell.so.0; do
    [ -n "${NOOS_COPY_SYSTEM_LIBS:-}" ] || break
    src="$(readlink -f "/usr/lib/$MULTIARCH/$lib")"
    [ -f "$src" ] || continue
    real="$(basename "$src")"
    rm -f "$PREFIX/lib/$lib"          # ancien lien éventuellement cassé
    put "$src" "$PREFIX/lib/$real" 755
    [ "$real" = "$lib" ] || ln -sf "$real" "$PREFIX/lib/$lib"
done
for lib in "$PREFIX"/lib/*.so*; do
    [ -e "$lib" ] || { echo "ERREUR : lien cassé $lib" >&2; exit 1; }
done

# Partage d'écran : xdg-desktop-portal-wlr (paquet Debian), lancé par NOOS lui-même.
if [ ! -x /usr/libexec/xdg-desktop-portal-wlr ] && [ ! -x /usr/lib/xdg-desktop-portal-wlr/xdg-desktop-portal-wlr ]; then
    echo "Attention : xdg-desktop-portal-wlr absent (sudo apt install xdg-desktop-portal-wlr) :" >&2
    echo "            pas de partage d'écran dans la session NOOS." >&2
fi
if command -v grim >/dev/null; then
    put "$(command -v grim)" "$PREFIX/bin/grim" 755
fi
# Choix des portails pour XDG_CURRENT_DESKTOP=NOOS (ignoré par GNOME).
install -Dm644 noos-session/noos-portals.conf "${XDG_CONFIG_HOME:-$HOME/.config}/xdg-desktop-portal/noos-portals.conf"


cp -r data/. "$PREFIX/share/noos/"
rm -rf "$PREFIX/share/noos/wallpapers"   # ancien fond fourni par défaut (supprimé)
install -m644 noos-session/org.noos.Files.desktop "$PREFIX/share/applications/"
install -m644 noos-session/org.noos.Settings.desktop "$PREFIX/share/applications/"
install -m644 noos-session/org.noos.Software.desktop "$PREFIX/share/applications/"
install -m644 noos-session/org.noos.Snip.desktop "$PREFIX/share/applications/"
install -m644 noos-session/org.noos.Terminal.desktop "$PREFIX/share/applications/"
install -m644 noos-session/org.noos.Keys.desktop "$PREFIX/share/applications/"

echo
echo "NOOS installé dans $PREFIX"
echo "Si ce n'est pas déjà fait, installez l'entrée de session (une fois, sur l'hôte) :"
echo "    sudo $ROOT/noos-session/install-session.sh"

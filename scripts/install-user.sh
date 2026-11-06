#!/usr/bin/env bash
# Compile NOOS en release et l'installe dans ~/.local/opt/noos (sans sudo).
# À lancer dans le toolbox de développement (mêmes bibliothèques que l'hôte Fedora) :
#
#     toolbox run -c noos-dev ./scripts/install-user.sh
#
# Les bibliothèques absentes de Silverblue (libAdapta, libseat, gtk4-layer-shell) et
# xwayland-satellite sont copiées depuis le toolbox dans le préfixe.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${NOOS_PREFIX:-$HOME/.local/opt/noos}"

if [ ! -f /run/.containerenv ]; then
    echo "Attention : ce script est prévu pour le toolbox noos-dev (toolbox run -c noos-dev …)." >&2
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
for lib in libseat.so.1 libgtk4-layer-shell.so.0; do
    src="$(readlink -f "/usr/lib64/$lib")"
    real="$(basename "$src")"
    rm -f "$PREFIX/lib/$lib"          # ancien lien éventuellement cassé
    put "$src" "$PREFIX/lib/$real" 755
    # Sur Fedora, libseat.so.1 est le fichier lui-même : pas de lien à créer.
    [ "$real" = "$lib" ] || ln -sf "$real" "$PREFIX/lib/$lib"
done
for lib in "$PREFIX"/lib/*.so*; do
    [ -e "$lib" ] || { echo "ERREUR : lien cassé $lib" >&2; exit 1; }
done

# Partage d'écran : xdg-desktop-portal-wlr (absent de Silverblue), lancé par NOOS lui-même.
mkdir -p "$PREFIX/libexec"
if [ -x /usr/libexec/xdg-desktop-portal-wlr ]; then
    put /usr/libexec/xdg-desktop-portal-wlr "$PREFIX/libexec/xdg-desktop-portal-wlr" 755
else
    echo "Attention : xdg-desktop-portal-wlr absent du toolbox (dnf install xdg-desktop-portal-wlr) :" >&2
    echo "            pas de partage d'écran dans la session NOOS." >&2
fi
if command -v grim >/dev/null; then
    put "$(command -v grim)" "$PREFIX/bin/grim" 755
fi
# Choix des portails pour XDG_CURRENT_DESKTOP=NOOS (ignoré par GNOME).
install -Dm644 noos-session/noos-portals.conf "${XDG_CONFIG_HOME:-$HOME/.config}/xdg-desktop-portal/noos-portals.conf"

if command -v xwayland-satellite >/dev/null; then
    put "$(command -v xwayland-satellite)" "$PREFIX/bin/xwayland-satellite" 755
fi

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

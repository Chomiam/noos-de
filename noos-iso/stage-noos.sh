#!/usr/bin/env bash
# Prépare l'arborescence de NOOS pour l'ISO : compile en release (toolbox noos-dev) et
# assemble DEST/opt/noos :
#
#   bin/      programmes NOOS (dont noos-login et noos-installer)
#   lib/      libAdapta (absente des dépôts Fedora)
#   share/noos/
#     system/        fichiers à copier à la racine du système installé (session, greeter,
#                    thème Plymouth, ld.so.conf.d…)
#     install/packages.txt   paquets Fedora du système installé
#
#     toolbox run -c noos-dev ./noos-iso/stage-noos.sh [DEST]   (défaut : noos-iso/out/stage)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:-$ROOT/noos-iso/out/stage}"
PREFIX="$DEST/opt/noos"
BINS=(noos-comp noos-shell noos-file noos-settings noos-software noos-pkg noos-snip noos-term noos-keys noos-save noos-photoview noos-texteditor noos-login noos-installer)

"$ROOT/scripts/build-libadapta.sh"
cd "$ROOT"
args=()
for b in "${BINS[@]}"; do args+=(-p "$b"); done
cargo build --release "${args[@]}"

rm -rf "$DEST"
mkdir -p "$PREFIX/bin" "$PREFIX/lib" "$PREFIX/share/noos/install" "$PREFIX/share/applications"
for b in "${BINS[@]}"; do
    install -m755 "target/release/$b" "$PREFIX/bin/$b"
done
for f in target/libadapta-build/src/libadapta-1.so*; do
    [ -d "$f" ] || cp -P "$f" "$PREFIX/lib/"
done
cp -r data/. "$PREFIX/share/noos/"
install -m644 noos-session/org.noos.*.desktop "$PREFIX/share/applications/"

# ── Fichiers système (copiés tels quels à la racine du système installé) ──
SYS="$PREFIX/share/noos/system"
install -Dm755 noos-session/noos-session "$SYS/usr/local/bin/noos-session"
install -Dm644 noos-session/noos.desktop "$SYS/usr/local/share/wayland-sessions/noos.desktop"
install -Dm644 noos-session/noos-portals.conf "$SYS/usr/local/share/xdg-desktop-portal/noos-portals.conf"
install -Dm755 noos-login/noos-greeter "$SYS/usr/local/bin/noos-greeter"
install -Dm644 /dev/stdin "$SYS/etc/ld.so.conf.d/noos.conf" <<<"/opt/noos/lib"
"$ROOT/noos-plymouth/install.sh" "$SYS" >/dev/null
# Réglages système (réseau sans économie d'énergie, WirePlumber sans mise en veille).
"$ROOT/noos-system/install.sh" "$SYS"
# Sauvegarder et Restaurer : script privilégié, actions dnf5, règle polkit.
"$ROOT/noos-save/install-system.sh" "$SYS" /opt/noos
install -Dm755 "$ROOT/noos-save/noos-save-helper" "$PREFIX/libexec/noos-save-helper"
# Thème GRUB : sources dans /usr/share/noos-grub (l'installateur le copie dans /boot).
install -d "$SYS/usr/share/noos-grub"
install -m644 "$ROOT"/noos-grub/noos/* "$SYS/usr/share/noos-grub/"
install -m755 "$ROOT/noos-grub/install.sh" "$SYS/usr/share/noos-grub/install.sh"

# ── Paquets : listes du dépôt + bibliothèques dont dépendent les binaires ──
libs_pkgs() {
    # libAdapta est fournie par NOOS : ldd doit la trouver pour lister SES dépendances
    # (appstream…), sinon elles manqueraient dans l'ISO.
    for b in "${BINS[@]}"; do LD_LIBRARY_PATH="$ROOT/target/libadapta-build/src" ldd "target/release/$b"; done \
        | awk '/=> \// {print $3}' | sort -u \
        | while read -r lib; do rpm -qf --qf '%{NAME}\n' "$lib" 2>/dev/null || true; done \
        | grep -v '^glibc$' | sort -u
    # Bibliothèques introuvables : la liste serait incomplète.
    if LD_LIBRARY_PATH="$ROOT/target/libadapta-build/src" ldd target/release/noos-* | grep -q "not found"; then
        echo "ERREUR : bibliothèque introuvable pour un binaire NOOS" >&2
        exit 1
    fi
}
{
    echo "# Généré par noos-iso/stage-noos.sh — paquets du système installé"
    grep -hv '^\s*#' noos-iso/packages-common.txt noos-iso/packages-system.txt | sed '/^\s*$/d'
    echo "# Bibliothèques des binaires NOOS"
    libs_pkgs
} | awk '!seen[$0]++' >"$PREFIX/share/noos/install/packages.txt"
# Projet COPR d'où l'installateur télécharge le paquet « noos » (sinon : copie de l'ISO).
if [ -s noos-iso/copr.txt ]; then
    install -m644 noos-iso/copr.txt "$PREFIX/share/noos/install/copr"
fi
# Le système live a besoin des mêmes bibliothèques pour faire tourner l'installateur.
libs_pkgs >"$DEST/live-libs.txt"

echo "NOOS préparé dans $PREFIX ($(du -sh "$PREFIX" | cut -f1))"

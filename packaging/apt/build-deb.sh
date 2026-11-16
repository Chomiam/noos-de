#!/usr/bin/env bash
# Construit le paquet Debian de NOOS (debian/) sur Debian sid :
#
#     packaging/apt/build-deb.sh [DOSSIER_DE_SORTIE]     (défaut : packaging/apt/out)
#
# Version de développement : 0.1.0~git<date>.<commit> (toujours croissante d'un commit à
# l'autre, et inférieure à une future 0.1.0 publiée). debian/changelog est réécrit pour la
# construction puis restauré. Utilisé par la CI (.github/workflows/build.yml) ou en local
# (dépendances : scripts/install-build-deps.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$(realpath -m "${1:-$ROOT/packaging/apt/out}")"
cd "$ROOT"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

BASE="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
COMMIT="$(git rev-parse --short=8 HEAD)"
STAMP="$(git log -1 --format=%cd --date=format:%Y%m%d%H%M)"
# Paquet natif (format 3.0 native) : pas de révision Debian « -1 ».
VERSION="${BASE}~git${STAMP}.${COMMIT}"
DATE="$(git log -1 --format=%cd --date=rfc2822)"

cp debian/changelog debian/changelog.orig
trap 'mv -f debian/changelog.orig debian/changelog' EXIT
cat >debian/changelog <<CHANGELOG
noos ($VERSION) unstable; urgency=medium

  * Construction automatique du commit $COMMIT :
    $(git log -1 --format=%s | cut -c1-70)

 -- Axel Valens <axel.valens@gmail.com>  $DATE
CHANGELOG

echo "==> noos $VERSION"
# -d : Rust vient de rustup (absent des Build-Depends) ; -b : paquet binaire seulement.
dpkg-buildpackage -b -us -uc -d
mkdir -p "$OUT"
mv -f ../noos_"${VERSION}"_*.deb "$OUT"/
rm -f ../noos_"${VERSION}"_*.buildinfo ../noos_"${VERSION}"_*.changes
ls -l "$OUT"/noos_"${VERSION}"_*.deb

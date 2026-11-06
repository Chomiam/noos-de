#!/usr/bin/env bash
# Fabrique le paquet source (SRPM) de NOOS : sources (git archive) + dépendances Rust
# « vendored » (COPR construit sans réseau), version = date + commit.
#
#     packaging/rpm/make-srpm.sh [DOSSIER_DE_SORTIE]
#
# Utilisé par COPR via .copr/Makefile (méthode « make srpm »), ou en local pour tester.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$(realpath -m "${1:-$ROOT/packaging/rpm/out}")"
SPEC="$ROOT/packaging/rpm/noos.spec"
VERSION="$(sed -n 's/^Version:[[:space:]]*//p' "$SPEC")"
cd "$ROOT"

# Release : 0.<date>.git<commit> (toujours croissante d'un commit à l'autre).
COMMIT="$(git rev-parse --short=8 HEAD)"
STAMP="$(git log -1 --format=%cd --date=format:%Y%m%d%H%M)"
RELEASE="0.${STAMP}.git${COMMIT}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/SOURCES" "$OUT"

echo "==> Sources noos-$VERSION ($COMMIT)"
git archive --format=tar.gz --prefix="noos-$VERSION/" -o "$WORK/SOURCES/noos-$VERSION.tar.gz" HEAD

echo "==> Dépendances Rust (cargo vendor)"
mkdir -p "$WORK/vendor-root"
cargo vendor --locked --versioned-dirs "$WORK/vendor-root/vendor" > "$WORK/vendor-root/vendor-config.toml"
mv "$WORK/vendor-root/vendor-config.toml" "$WORK/vendor-root/vendor/vendor-config.toml"
# Chemin du dossier vendor relatif au dépôt décompressé.
sed -i "s|directory = \".*\"|directory = \"vendor\"|" "$WORK/vendor-root/vendor/vendor-config.toml"
tar -C "$WORK/vendor-root" -cJf "$WORK/SOURCES/noos-$VERSION-vendor.tar.xz" vendor

echo "==> SRPM"
# Release écrite en dur dans le spec du paquet source : COPR (comme rpmbuild --rebuild)
# relit le spec, une macro --define serait perdue et la version n'augmenterait plus.
sed "s/^Release:.*/Release:        ${RELEASE}%{?dist}/" "$SPEC" > "$WORK/noos.spec"
rpmbuild -bs "$WORK/noos.spec" \
    --define "_sourcedir $WORK/SOURCES" \
    --define "_srcrpmdir $OUT"
ls -1 "$OUT"/*.src.rpm

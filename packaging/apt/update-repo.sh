#!/usr/bin/env bash
# Ajoute des paquets au dépôt APT de NOOS (branche gh-pages, servie par GitHub Pages) et le
# signe :
#
#     packaging/apt/update-repo.sh DOSSIER_DU_DÉPÔT paquet.deb…
#
# Disposition (dépôt Debian classique, suite « sid », composant « main ») :
#
#     pool/main/n/noos/noos_<version>_amd64.deb     les 5 versions les plus récentes
#     dists/sid/main/binary-amd64/Packages(.gz,.xz)
#     dists/sid/{Release,InRelease,Release.gpg}     signés par la clé du dépôt
#     noos-archive-keyring.asc                      clé publique (Signed-By)
#     index.html                                    page d'explication
#
# La clé privée est fournie par la CI (secret APT_SIGNING_KEY, importé dans gpg) ; son
# identifiant peut être forcé par APT_SIGNING_KEY_ID.
set -euo pipefail

REPO="$(realpath -m "$1")"
shift
[ $# -gt 0 ] || { echo "usage : $0 DOSSIER_DU_DÉPÔT paquet.deb…" >&2; exit 2; }
HERE="$(cd "$(dirname "$0")" && pwd)"
SUITE=sid
COMP=main
ARCHES="amd64 i386"
KEEP=5

mkdir -p "$REPO/pool/$COMP/n/noos"
for deb in "$@"; do
    install -m644 "$deb" "$REPO/pool/$COMP/n/noos/"
done
# Seules les $KEEP dernières versions restent (GitHub Pages : 1 Go au total).
ls -1 "$REPO/pool/$COMP/n/noos"/noos_*.deb | sort -V | head -n -"$KEEP" | xargs -r rm -f

cd "$REPO"
for arch in $ARCHES; do
    dir="dists/$SUITE/$COMP/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages "pool/$COMP" >"$dir/Packages"
    gzip -9kf "$dir/Packages"
    xz -9kf "$dir/Packages"
done
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=NOOS \
    -o APT::FTPArchive::Release::Label=NOOS \
    -o APT::FTPArchive::Release::Suite=$SUITE \
    -o APT::FTPArchive::Release::Codename=$SUITE \
    -o "APT::FTPArchive::Release::Architectures=$ARCHES" \
    -o APT::FTPArchive::Release::Components=$COMP \
    -o "APT::FTPArchive::Release::Description=Dépôt APT de NOOS (bureau Wayland) pour Debian sid" \
    release "dists/$SUITE" >"dists/$SUITE/Release.tmp"
mv "dists/$SUITE/Release.tmp" "dists/$SUITE/Release"

KEY="${APT_SIGNING_KEY_ID:-$(gpg --list-secret-keys --with-colons | awk -F: '/^sec/ {print $5; exit}')}"
[ -n "$KEY" ] || { echo "aucune clé de signature dans gpg (secret APT_SIGNING_KEY ?)" >&2; exit 1; }
rm -f "dists/$SUITE/InRelease" "dists/$SUITE/Release.gpg"
gpg --batch --yes --pinentry-mode loopback --local-user "$KEY" --clearsign -o "dists/$SUITE/InRelease" "dists/$SUITE/Release"
gpg --batch --yes --pinentry-mode loopback --local-user "$KEY" -abs -o "dists/$SUITE/Release.gpg" "dists/$SUITE/Release"
gpg --armor --export "$KEY" >noos-archive-keyring.asc
touch .nojekyll
sed "s|@LATEST@|$(ls -1 pool/$COMP/n/noos/noos_*.deb | sort -V | tail -1 | xargs basename)|g; s|@DATE@|$(date -u '+%d/%m/%Y %H:%M UTC')|g" \
    "$HERE/index.html" >index.html
echo "Dépôt mis à jour : $(ls -1 pool/$COMP/n/noos | wc -l) version(s), clé $KEY"

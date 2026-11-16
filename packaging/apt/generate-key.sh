#!/usr/bin/env bash
# Crée la clé GPG de signature du dépôt APT de NOOS (une seule fois, par le propriétaire du
# dépôt GitHub). Fonctionne sous Linux et sous Git Bash (Windows : gpg est fourni avec Git).
#
#     packaging/apt/generate-key.sh
#
# Produit :
#   packaging/apt/noos-archive-keyring.asc   clé PUBLIQUE, à committer (paquet, ISO, Pages)
#   noos-apt-private.asc (dossier courant)   clé PRIVÉE : à coller dans GitHub › Settings ›
#                                            Secrets and variables › Actions › New repository
#                                            secret, nom APT_SIGNING_KEY ; puis À SUPPRIMER.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
GNUPGHOME="$(mktemp -d)"
export GNUPGHOME
trap 'rm -rf "$GNUPGHOME"' EXIT
gpg --batch --pinentry-mode loopback --passphrase '' \
    --quick-generate-key "NOOS APT <noos-apt@users.noreply.github.com>" ed25519 sign 5y
gpg --armor --export >"$HERE/noos-archive-keyring.asc"
gpg --armor --export-secret-keys >noos-apt-private.asc
echo "Clé publique : $HERE/noos-archive-keyring.asc (à committer)"
echo "Clé privée   : $PWD/noos-apt-private.asc → secret GitHub APT_SIGNING_KEY, puis supprimez ce fichier"

#!/usr/bin/env bash
# Préparation de NOOS pour l'ISO dans un conteneur debian:sid vierge (CI GitHub, ou
# noos-iso/build.sh sans conteneur noos-dev) : dépendances, Rust, clé du dépôt APT, stage.
set -euo pipefail
cd "$(dirname "$0")/.."
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends git ca-certificates curl gnupg >/dev/null
git config --global --add safe.directory "$PWD"
./scripts/install-build-deps.sh
. "$HOME/.cargo/env"
if [ ! -s packaging/apt/noos-archive-keyring.asc ] && [ -n "${APT_SIGNING_KEY:-}" ]; then
    printf '%s\n' "$APT_SIGNING_KEY" | gpg --batch --import
    gpg --armor --export >packaging/apt/noos-archive-keyring.asc
fi
./noos-iso/stage-noos.sh "$PWD/noos-iso/out/stage"

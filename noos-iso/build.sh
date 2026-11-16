#!/usr/bin/env bash
# Construit l'ISO d'installation de NOOS (Debian sid, installation réseau). Sans sudo sur l'hôte :
#
#   1. compilation et préparation de NOOS (stage-noos.sh) dans le conteneur de développement
#      « noos-dev » (distrobox Debian sid) s'il existe, sinon dans un conteneur debian:sid
#      jetable (dépendances installées par scripts/install-build-deps.sh) ;
#   2. système live + ISO dans un conteneur debian:sid « privileged » (build-inside.sh) :
#      rien n'est installé ni monté sur l'hôte.
#
#     ./noos-iso/build.sh            → noos-iso/out/noos-sid-amd64.iso
#     ./noos-iso/build.sh --no-stage → réutilise la préparation existante
#
# Moteur de conteneurs : podman (par défaut) ou docker (NOOS_CONTAINER=docker).
# La CI GitHub (.github/workflows/iso.yml) fait la même chose à la demande.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/noos-iso/out"
mkdir -p "$OUT"
ENGINE="${NOOS_CONTAINER:-$(command -v podman >/dev/null && echo podman || echo docker)}"

if [ "${1:-}" != "--no-stage" ]; then
    echo "==> Compilation et préparation de NOOS"
    if command -v distrobox >/dev/null && distrobox list 2>/dev/null | grep -qw noos-dev; then
        distrobox enter noos-dev -- "$ROOT/noos-iso/stage-noos.sh" "$OUT/stage"
    else
        "$ENGINE" run --rm -v "$ROOT:/src" -w /src docker.io/library/debian:sid \
            bash noos-iso/ci-stage.sh
    fi
fi

echo "==> Construction de l'ISO (conteneur Debian sid)"
"$ENGINE" run --rm --privileged \
    -v "$ROOT:/src:ro" \
    -v "$OUT:/out" \
    docker.io/library/debian:sid \
    bash /src/noos-iso/build-inside.sh

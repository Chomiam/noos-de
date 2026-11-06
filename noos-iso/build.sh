#!/usr/bin/env bash
# Construit l'ISO d'installation de NOOS (Fedora 45, installation réseau). Sans sudo :
#
#   1. compilation et préparation de NOOS dans le toolbox noos-dev (stage-noos.sh) ;
#   2. système live + ISO dans un conteneur Fedora 45 « privileged » de podman rootless
#      (build-inside.sh) : rien n'est installé ni monté sur l'hôte.
#
#     ./noos-iso/build.sh            → noos-iso/out/noos-45-x86_64.iso
#     ./noos-iso/build.sh --no-stage → réutilise la préparation existante
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/noos-iso/out"
mkdir -p "$OUT"

run_host() { if [ -f /run/.containerenv ] || [ -n "${FLATPAK_ID:-}" ]; then flatpak-spawn --host "$@"; else "$@"; fi; }

if [ "${1:-}" != "--no-stage" ]; then
    echo "==> Compilation et préparation de NOOS (toolbox noos-dev)"
    run_host toolbox run -c noos-dev "$ROOT/noos-iso/stage-noos.sh" "$OUT/stage"
fi

echo "==> Construction de l'ISO (conteneur Fedora 45)"
run_host podman run --rm --privileged \
    -v "$ROOT:/src:ro,z" \
    -v "$OUT:/out:z" \
    registry.fedoraproject.org/fedora:45 \
    bash /src/noos-iso/build-inside.sh

#!/usr/bin/env bash
# Dépendances de compilation de NOOS sur Debian sid (root, ou sudo si disponible), puis Rust
# stable par rustup dans ~/.cargo (l'édition 2024 et les crates récentes demandent souvent un
# Rust plus récent que celui de Debian).
#
#     ./scripts/install-build-deps.sh            → bibliothèques + outils + Rust
#     ./scripts/install-build-deps.sh --no-rust  → sans rustup (Rust déjà installé)
#
# Utilisé par la CI GitHub, noos-iso/build.sh et le conteneur de développement noos-dev.
set -euo pipefail
SUDO=""
[ "$(id -u)" = 0 ] || SUDO=sudo
export DEBIAN_FRONTEND=noninteractive
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq --no-install-recommends \
    build-essential pkgconf git curl ca-certificates xz-utils \
    meson ninja-build python3 sassc gobject-introspection libgirepository1.0-dev \
    libgtk-4-dev libappstream-dev libfribidi-dev \
    libseat-dev libinput-dev libudev-dev libgbm-dev libdrm-dev libegl-dev libgles-dev \
    libxkbcommon-dev libdisplay-info-dev libwayland-dev wayland-protocols \
    libgtk4-layer-shell-dev libvte-2.91-gtk4-dev libgtksourceview-5-dev libpam0g-dev \
    debhelper devscripts dpkg-dev fakeroot apt-utils gnupg
if [ "${1:-}" != "--no-rust" ] && ! command -v cargo >/dev/null; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
fi

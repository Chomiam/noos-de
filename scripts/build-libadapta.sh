#!/usr/bin/env bash
# Compile libAdapta (vendored dans third_party/libadapta) dans target/libadapta-build.
# Rien n'est installé : cargo trouve la bibliothèque via le .pc « uninstalled » de meson
# (voir .cargo/config.toml).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD="$ROOT/target/libadapta-build"

if [ ! -f "$BUILD/build.ninja" ]; then
    meson setup "$BUILD" "$ROOT/third_party/libadapta" \
        -Dintrospection=enabled -Dvapi=false -Dtests=false -Dexamples=false -Dgtk_doc=false
fi
ninja -C "$BUILD"

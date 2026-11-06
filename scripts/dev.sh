#!/usr/bin/env bash
# Lance NOOS dans une fenêtre de votre session actuelle (mode imbriqué, pour développer).
# À exécuter dans le toolbox de développement :  toolbox run -c noos-dev ./scripts/dev.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="${NOOS_PROFILE:-debug}"
export LD_LIBRARY_PATH="$ROOT/target/libadapta-build/src${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$ROOT/target/$PROFILE:$PATH"
export NOOS_DATA_DIR="$ROOT/data"
exec noos-comp --winit "$@"

#!/bin/bash
# scenario.sh DOSSIER "commandes" — compositeur imbriqué + shell ; les commandes (une par
# ligne) sont exécutées dans l'ordre : « ipc <json> », « shot <nom> », « sleep <s> ».
# Variables d'environnement transmises telles quelles (ex. NOOS_WINIT_OUTPUTS=2).
# Configuration isolée (copie de la vôtre dans DOSSIER/config) : le scénario peut modifier la
# configuration (bureaux, écrans) sans toucher à ~/.config/noos.
OUT=$1; STEPS=$2
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/../.."
mkdir -p "$OUT/config/noos"
[ -f "$OUT/config/noos/noos.toml" ] || cp "${XDG_CONFIG_HOME:-$HOME/.config}/noos/noos.toml" "$OUT/config/noos/noos.toml" 2>/dev/null || true
export XDG_CONFIG_HOME="$OUT/config"
LOG="$OUT/scenario.log"
WAYLAND_DISPLAY=wayland-1 NOOS_PROFILE=release dbus-run-session -- ./scripts/dev.sh >"$LOG" 2>&1 &
for i in $(seq 1 40); do
    sock=$(grep -ao "socket wayland prêt[^w]*wayland-[0-9]*" "$LOG" | grep -o "wayland-[0-9]*$" | head -1)
    [ -n "$sock" ] && break
    sleep 0.25
done
ipc=$(grep -ao "socket IPC prêt.*path[^/]*/[^ ]*sock" "$LOG" | grep -o "/[^ ]*sock$" | head -1)
sleep 4
while IFS= read -r line; do
    [ -z "$line" ] && continue
    cmd=${line%% *}; arg=${line#* }
    case "$cmd" in
        ipc) python3 "$HERE/ipc.py" "$ipc" "$arg" >>"$OUT/ipc.log" ;;
        shot) WAYLAND_DISPLAY=$sock grim "$OUT/$arg.png" && echo "capture $OUT/$arg.png" ;;
        sleep) sleep "$arg" ;;
    esac
done <<<"$STEPS"
kill $(ps -eo pid,comm,args | awk '$2=="noos-comp" && /--winit/ {print $1}') 2>/dev/null
wait 2>/dev/null

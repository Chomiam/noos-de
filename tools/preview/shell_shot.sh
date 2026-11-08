#!/bin/bash
# shell_shot.sh CAPTURE.png ATTENTE [VAR=valeur…] — compositeur imbriqué avec le shell complet
# (barre, panneaux), variables de démo passées au shell, capture, puis arrêt.
OUT=$1; WAIT=$2; shift 2
cd "$(dirname "$0")/../.."
LOG="${PREVIEW_LOG:-$(dirname "$OUT")/shell.log}"
env "$@" WAYLAND_DISPLAY=wayland-1 NOOS_PROFILE=release dbus-run-session -- ./scripts/dev.sh >"$LOG" 2>&1 &
for i in $(seq 1 40); do
    sock=$(grep -ao "socket wayland prêt[^w]*wayland-[0-9]*" "$LOG" | grep -o "wayland-[0-9]*$" | head -1)
    [ -n "$sock" ] && break
    sleep 0.25
done
sleep "$WAIT"
WAYLAND_DISPLAY=$sock grim "$OUT" && echo "capture $OUT"
kill $(ps -eo pid,comm,args | awk '$2=="noos-comp" && /--winit/ {print $1}') 2>/dev/null
wait 2>/dev/null

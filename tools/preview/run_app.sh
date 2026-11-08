#!/bin/bash
# run_app.sh CAPTURE.png ATTENTE "commande" — aperçu imbriqué en kiosque (une appli seule),
# capture, puis arrêt. Lancer sur l'hôte si l'appli a besoin du bus système (udisks,
# NetworkManager), sinon dans le toolbox.
OUT="$1"; WAIT="$2"; CMD="$3"
cd "$(dirname "$0")/../.."
LOG="${PREVIEW_LOG:-$(dirname "$OUT")/comp.log}"
WAYLAND_DISPLAY=wayland-1 NOOS_PROFILE=release ./scripts/dev.sh --no-shell --kiosk "dbus-run-session -- env GTK_A11Y=none $CMD" >"$LOG" 2>&1 &
for i in $(seq 1 40); do
    sock=$(grep -ao "socket wayland prêt[^w]*wayland-[0-9]*" "$LOG" | grep -o "wayland-[0-9]*$" | head -1)
    [ -n "$sock" ] && break
    sleep 0.25
done
sleep "$WAIT"
WAYLAND_DISPLAY=$sock grim "$OUT" && echo "capture $OUT"
kill $(ps -eo pid,comm,args | awk '$2=="noos-comp" && /--winit/ {print $1}') 2>/dev/null
wait 2>/dev/null

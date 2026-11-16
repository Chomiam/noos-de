#!/usr/bin/env bash
# VM de test NOOS (QEMU/KVM sans affichage), sur l'hôte Debian ou dans le conteneur noos-dev
# (paquets qemu-system-x86, ovmf, qemu-utils).
#
#   tools/vm/vm.sh start [ISO] [uefi|bios]   démarre la VM (sans ISO : démarre sur le disque)
#   tools/vm/vm.sh install                    parcours complet de l'installateur (écran d'accueil)
#   tools/vm/vm.sh login                      écran de connexion → bureau (compte de test)
#   tools/vm/vm.sh stop                       arrêt propre (ACPI) — jamais kill pendant une écriture
#   tools/vm/vm.sh reset-disk                 nouveau disque vierge + variables UEFI neuves
#
# Pilotage fin : tools/vm/qmp.py (shot, click, key, type, cmd).
# Sous Windows (Git Bash, QEMU de qemu.org + « Plateforme de l'hyperviseur Windows ») :
# accélération WHPX, micrologiciel edk2 fourni avec QEMU, QMP sur 127.0.0.1:4444 et console
# série sur 127.0.0.1:4445 (TCP, tools/vm/serial.py), fenêtre QEMU visible (NOOS_VM_DISPLAY=none
# pour la masquer). WHPX ne supporte pas la réinitialisation : jamais « cmd system_reset » ni
# redémarrage depuis l'invité (QEMU plante) ; toujours stop puis start.
# Fichiers (disque qcow2 de 40 Go, variables UEFI) : noos-iso/out/vm/ (sur le vrai disque :
# surtout pas dans /tmp, qui est en RAM). RAM de la VM : 2,5 Go (le PC hôte a 8 Go).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="${NOOS_VM_DIR:-$ROOT/noos-iso/out/vm}"
RUN="${XDG_RUNTIME_DIR:-/tmp}"
QMP="$RUN/noosvm.sock"
SER="$RUN/noosser.sock"
WINDOWS=0
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) WINDOWS=1 ;; esac
if [ "$WINDOWS" = 1 ]; then
    QEMU_DIR="${QEMU_DIR:-/c/Program Files/qemu}"
    export PATH="$QEMU_DIR:$PATH"
    export NOOS_VM_QMP="${NOOS_VM_QMP:-127.0.0.1:4444}"
    Q="python $ROOT/tools/vm/qmp.py"
else
    Q="python3 $ROOT/tools/vm/qmp.py"
fi
PASSWORD='Vm-Noos-Essai-2026!'   # compte de test « Test Noos » (test) — VM uniquement
mkdir -p "$DIR"

case "${1:-}" in
start)
    ISO="${2:-}"
    MODE="${3:-uefi}"
    [ -f "$DIR/disk.qcow2" ] || qemu-img create -f qcow2 "$DIR/disk.qcow2" 40G >/dev/null
    FW=()
    if [ "$MODE" = uefi ]; then
        if [ "$WINDOWS" = 1 ]; then
            CODE="$QEMU_DIR/share/edk2-x86_64-code.fd"
            [ -f "$DIR/VARS.fd" ] || cp "$QEMU_DIR/share/edk2-i386-vars.fd" "$DIR/VARS.fd"
        else
            # Debian (paquet ovmf) : OVMF_CODE_4M.fd / OVMF_VARS_4M.fd.
            CODE=/usr/share/OVMF/OVMF_CODE_4M.fd
            [ -f "$DIR/VARS.fd" ] || cp /usr/share/OVMF/OVMF_VARS_4M.fd "$DIR/VARS.fd"
        fi
        FW=(-drive "if=pflash,format=raw,readonly=on,file=$CODE"
            -drive "if=pflash,format=raw,file=$DIR/VARS.fd")
    fi
    # -vga std : pas de 3D (rendu logiciel llvmpipe, comme un PC sans pilote graphique).
    # usb-tablet : clics en coordonnées absolues.
    if [ "$WINDOWS" = 1 ]; then
        # kernel-irqchip=off : avec le contrôleur d'interruptions de WHPX, la VM se fige au démarrage.
        ACCEL=(-accel whpx,kernel-irqchip=off -m 8192 -smp 8)
        CTRL=(-display "${NOOS_VM_DISPLAY:-sdl}" -qmp "tcp:127.0.0.1:4444,server=on,wait=off"
              -serial "tcp:127.0.0.1:4445,server=on,wait=off")
    else
        rm -f "$QMP" "$SER"
        ACCEL=(-enable-kvm -cpu host -m 2560 -smp 4)
        CTRL=(-display none -qmp "unix:$QMP,server,nowait" -serial "unix:$SER,server,nowait")
    fi
    exec qemu-system-x86_64 "${ACCEL[@]}" -machine q35 "${FW[@]}"         -drive "file=$DIR/disk.qcow2,if=virtio,format=qcow2"         ${ISO:+-cdrom "$ISO"} -boot order=dc,menu=off         -vga std -device qemu-xhci -device usb-tablet         -nic user,model=virtio-net-pci "${CTRL[@]}"
    ;;
install)
    c() { $Q click "$@"; sleep 2.5; }
    c 1146 747                      # Bienvenue → Fuseau horaire
    c 1146 747                      # → Clavier
    c 1146 747                      # → Compte
    $Q type 'Test Noos'
    c 700 457; $Q type "$PASSWORD"
    c 700 512; $Q type "$PASSWORD"
    c 1146 747                      # → Disque
    sleep 4; c 1124 198              # coche le premier disque (/dev/vda), liste chargée
    c 1146 747                      # → Récapitulatif
    c 1146 747                      # Installer NOOS
    c 729 483                       # Effacer et installer
    $Q shot "$DIR/install-start.png"
    ;;
login)
    $Q click 640 500; sleep 2
    $Q type "$PASSWORD\n"
    ;;
stop)
    $Q cmd system_powerdown
    ;;
reset-disk)
    rm -f "$DIR/disk.qcow2" "$DIR/VARS.fd"
    echo "Disque et variables UEFI réinitialisés."
    ;;
*)
    sed -n '2,12p' "$0"
    exit 1
    ;;
esac

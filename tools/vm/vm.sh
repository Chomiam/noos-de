#!/usr/bin/env bash
# VM de test NOOS (QEMU/KVM sans affichage), à lancer dans le toolbox noos-dev.
#
#   tools/vm/vm.sh start [ISO] [uefi|bios]   démarre la VM (sans ISO : démarre sur le disque)
#   tools/vm/vm.sh install                    parcours complet de l'installateur (écran d'accueil)
#   tools/vm/vm.sh login                      écran de connexion → bureau (compte de test)
#   tools/vm/vm.sh stop                       arrêt propre (ACPI) — jamais kill pendant une écriture
#   tools/vm/vm.sh reset-disk                 nouveau disque vierge + variables UEFI neuves
#
# Pilotage fin : tools/vm/qmp.py (shot, click, key, type, cmd).
# Fichiers (disque qcow2 de 40 Go, variables UEFI) : noos-iso/out/vm/ (sur le vrai disque :
# surtout pas dans /tmp, qui est en RAM). RAM de la VM : 2,5 Go (le PC hôte a 8 Go).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="${NOOS_VM_DIR:-$ROOT/noos-iso/out/vm}"
RUN="${XDG_RUNTIME_DIR:-/tmp}"
QMP="$RUN/noosvm.sock"
SER="$RUN/noosser.sock"
Q="python3 $ROOT/tools/vm/qmp.py"
PASSWORD='Vm-Noos-Essai-2026!'   # compte de test « Test Noos » (test) — VM uniquement
mkdir -p "$DIR"

case "${1:-}" in
start)
    ISO="${2:-}"
    MODE="${3:-uefi}"
    [ -f "$DIR/disk.qcow2" ] || qemu-img create -f qcow2 "$DIR/disk.qcow2" 40G >/dev/null
    FW=()
    if [ "$MODE" = uefi ]; then
        [ -f "$DIR/VARS.fd" ] || cp /usr/share/edk2/ovmf/OVMF_VARS.fd "$DIR/VARS.fd"
        FW=(-drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.fd
            -drive "if=pflash,format=raw,file=$DIR/VARS.fd")
    fi
    rm -f "$QMP" "$SER"
    # -vga std : pas de 3D (rendu logiciel llvmpipe, comme un PC sans pilote graphique).
    # usb-tablet : clics en coordonnées absolues. Console série interactive sur $SER.
    exec qemu-system-x86_64 -enable-kvm -machine q35 -cpu host -m 2560 -smp 4 "${FW[@]}" \
        -drive "file=$DIR/disk.qcow2,if=virtio,format=qcow2" \
        ${ISO:+-cdrom "$ISO"} -boot order=dc,menu=off \
        -vga std -device qemu-xhci -device usb-tablet -display none \
        -qmp "unix:$QMP,server,nowait" -nic user,model=virtio-net-pci \
        -serial "unix:$SER,server,nowait"
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
    c 1124 198                      # coche le premier disque (/dev/vda)
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

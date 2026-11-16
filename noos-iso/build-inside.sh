#!/usr/bin/env bash
# Construction de l'ISO live NOOS — exécuté en root dans un conteneur Debian sid privilégié
# (lancé par noos-iso/build.sh ou par la CI GitHub, rien n'est fait sur l'hôte).
#
#   /src : le dépôt          /work : dossier de travail          /out : ISO produite
#
# 1. système live : debootstrap sid + paquets de NOOS (listes packages-*.txt + bibliothèques)
# 2. NOOS dans /opt/noos, connexion automatique de root sur tty1 → installateur plein écran
# 3. initramfs live-boot (initramfs-tools + Plymouth NOOS), squashfs zstd
# 4. GRUB : BIOS (El Torito) + UEFI (shim et GRUB signés de Debian : Secure Boot)
# 5. ISO hybride (CD, clé USB par dd) avec xorriso
set -euo pipefail

SUITE=sid
MIRROR=http://deb.debian.org/debian
LABEL="NOOS-SID"
SRC=/src
STAGE="${STAGE:-$SRC/noos-iso/out/stage}"
WORK=/work
ROOTFS="$WORK/rootfs"
ISO="$WORK/iso"
OUT="${OUT:-/out/noos-sid-amd64.iso}"
export DEBIAN_FRONTEND=noninteractive LC_ALL=C.UTF-8

log() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

[ -x "$STAGE/opt/noos/bin/noos-installer" ] || { echo "Lancez d'abord noos-iso/stage-noos.sh" >&2; exit 1; }

log "Outils de construction"
apt-get update -qq
apt-get install -y -qq debootstrap debian-archive-keyring squashfs-tools xorriso mtools dosfstools \
    grub-common grub-pc-bin grub-efi-amd64-bin grub-efi-amd64-signed shim-signed >/dev/null

log "Système live (debootstrap Debian $SUITE)"
rm -rf "$WORK" && mkdir -p "$ROOTFS" "$ISO"
debootstrap --arch=amd64 --components=main,contrib,non-free,non-free-firmware \
    --include=ca-certificates,locales,apt-utils "$SUITE" "$ROOTFS" "$MIRROR"
mount -t proc proc "$ROOTFS/proc"
mount --rbind /sys "$ROOTFS/sys"
mount --rbind /dev "$ROOTFS/dev"
cleanup() { umount -R "$ROOTFS/proc" "$ROOTFS/sys" "$ROOTFS/dev" 2>/dev/null || true; }
trap cleanup EXIT
cat >"$ROOTFS/etc/apt/sources.list.d/debian.sources" <<EOF
Types: deb
URIs: $MIRROR
Suites: $SUITE
Components: main contrib non-free non-free-firmware
Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg
EOF
rm -f "$ROOTFS/etc/apt/sources.list"
printf '#!/bin/sh\nexit 101\n' >"$ROOTFS/usr/sbin/policy-rc.d"
chmod 755 "$ROOTFS/usr/sbin/policy-rc.d"
chroot "$ROOTFS" apt-get update -qq
mapfile -t PKGS < <(cat "$SRC/noos-iso/packages-common.txt" "$SRC/noos-iso/packages-live.txt" "$STAGE/live-libs.txt" \
    | sed 's/#.*//; s/^\s*//; s/\s*$//; /^$/d' | sort -u)
# Paquets absents du miroir (renommés dans sid…) : signalés puis ignorés.
mapfile -t AVAIL < <(chroot "$ROOTFS" apt-cache policy "${PKGS[@]}" 2>/dev/null \
    | awk '/^[^ ].*:$/ {p=substr($0,1,length($0)-1)} /Candidate:/ && $2 != "(none)" {print p}')
for p in "${PKGS[@]}"; do
    printf '%s\n' "${AVAIL[@]}" | grep -qx "$p" || echo "ATTENTION : paquet absent de Debian $SUITE : $p" >&2
done
chroot "$ROOTFS" apt-get install -y --no-install-recommends \
    -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold "${AVAIL[@]}"

log "NOOS dans /opt/noos"
cp -a "$STAGE/opt/noos" "$ROOTFS/opt/"
# Fichiers système de NOOS aussi dans le live (Plymouth, ld.so.conf.d, clé du dépôt APT…).
cp -a "$STAGE/opt/noos/share/noos/system/." "$ROOTFS/"
chown -R root:root "$ROOTFS/etc/sudoers.d" 2>/dev/null || true
install -Dm755 "$SRC/noos-iso/noos-live-installer" "$ROOTFS/usr/local/bin/noos-live-installer"
chroot "$ROOTFS" ldconfig

log "Configuration du système live"
echo "noos-live" >"$ROOTFS/etc/hostname"
printf '127.0.0.1\tlocalhost\n127.0.1.1\tnoos-live\n' >"$ROOTFS/etc/hosts"
printf 'fr_FR.UTF-8 UTF-8\nen_US.UTF-8 UTF-8\n' >"$ROOTFS/etc/locale.gen"
chroot "$ROOTFS" locale-gen >/dev/null
echo "LANG=fr_FR.UTF-8" >"$ROOTFS/etc/default/locale"
cat >"$ROOTFS/etc/default/keyboard" <<'EOF'
XKBMODEL="pc105"
XKBLAYOUT="fr"
XKBVARIANT=""
XKBOPTIONS=""
BACKSPACE="guess"
EOF
printf 'KEYMAP=fr\nXKBLAYOUT=fr\n' >"$ROOTFS/etc/vconsole.conf"
ln -sf /usr/share/zoneinfo/Europe/Paris "$ROOTFS/etc/localtime"
echo "Europe/Paris" >"$ROOTFS/etc/timezone"
chroot "$ROOTFS" passwd -d root >/dev/null
# Connexion automatique de root sur tty1, puis l'installateur.
mkdir -p "$ROOTFS/etc/systemd/system/getty@tty1.service.d"
cat >"$ROOTFS/etc/systemd/system/getty@tty1.service.d/autologin.conf" <<'EOF'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --noclear %I $TERM
EOF
cat >>"$ROOTFS/root/.bash_profile" <<'EOF'

# NOOS live : installateur plein écran sur tty1.
if [ "$(tty)" = /dev/tty1 ] && [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${NOOS_LIVE_SHELL:-}" ]; then
    exec /usr/local/bin/noos-live-installer
fi
EOF
cat >"$ROOTFS/etc/motd" <<'EOF'
NOOS live — l'installateur tourne sur tty1 (Ctrl+Alt+F1).
Journaux : /tmp/noos-live.log et /tmp/noos-install.log
EOF
chroot "$ROOTFS" systemctl enable NetworkManager.service getty@tty1.service >/dev/null 2>&1 || true
chroot "$ROOTFS" systemctl set-default multi-user.target >/dev/null
# Pas de mise en veille pendant l'installation.
mkdir -p "$ROOTFS/etc/systemd/logind.conf.d"
printf '[Login]\nIdleAction=ignore\nHandleLidSwitch=ignore\n' >"$ROOTFS/etc/systemd/logind.conf.d/noos-live.conf"
chroot "$ROOTFS" plymouth-set-default-theme noos 2>/dev/null || true
rm -f "$ROOTFS/usr/sbin/policy-rc.d"

log "initramfs live (live-boot + Plymouth NOOS)"
KVER="$(ls "$ROOTFS/lib/modules" | sort -V | tail -1)"
chroot "$ROOTFS" update-initramfs -u -k "$KVER"
mkdir -p "$ISO/live" "$ISO/boot/grub" "$ISO/EFI/BOOT" "$ISO/.disk"
cp "$ROOTFS/boot/vmlinuz-$KVER" "$ISO/live/vmlinuz"
cp "$ROOTFS/boot/initrd.img-$KVER" "$ISO/live/initrd.img"
# Repère du support pour le GRUB signé de Debian (gcdx64.efi cherche /.disk/info).
echo "NOOS (Debian $SUITE) $(date +%Y%m%d)" >"$ISO/.disk/info"
cleanup
trap - EXIT

log "Image compressée du système (squashfs zstd)"
chroot "$ROOTFS" apt-get clean 2>/dev/null || rm -rf "$ROOTFS/var/cache/apt/archives"/*.deb
rm -rf "$ROOTFS/var/lib/apt/lists"/*_Packages "$ROOTFS/tmp"/* 2>/dev/null || true
mksquashfs "$ROOTFS" "$ISO/live/filesystem.squashfs" -comp zstd -Xcompression-level 15 -b 1M -noappend -quiet \
    -wildcards -e 'boot/initrd.img-*' 'boot/vmlinuz-*'

log "GRUB (BIOS + UEFI)"
GRUBCFG=$(cat <<EOF
set default=0
set timeout=5
insmod all_video
insmod gfxterm
insmod png
insmod gfxmenu
insmod part_gpt
insmod iso9660
search --no-floppy --set=root --file /.disk/info
loadfont (\$root)/boot/grub/fonts/unicode.pf2
set gfxmode=auto
terminal_output gfxterm
set menu_color_normal=light-gray/black
set menu_color_highlight=white/blue
# Thème NOOS (noos-grub), comme sur le système installé.
loadfont (\$root)/boot/grub/themes/noos/noos-16.pf2
loadfont (\$root)/boot/grub/themes/noos/noos-20.pf2
loadfont (\$root)/boot/grub/themes/noos/noos-semibold-20.pf2
loadfont (\$root)/boot/grub/themes/noos/noos-semibold-28.pf2
set theme=(\$root)/boot/grub/themes/noos/theme.txt

menuentry "Installer NOOS" --class noos {
    linux /live/vmlinuz boot=live components quiet splash
    initrd /live/initrd.img
}
menuentry "Installer NOOS (graphismes de base)" --class noos {
    linux /live/vmlinuz boot=live components nomodeset quiet splash
    initrd /live/initrd.img
}
menuentry "Installer NOOS (mode texte, dépannage)" {
    linux /live/vmlinuz boot=live components systemd.unit=multi-user.target
    initrd /live/initrd.img
}
menuentry "Installer NOOS (journal sur port série, dépannage)" {
    linux /live/vmlinuz boot=live components console=tty0 console=ttyS0,115200 noos.serial
    initrd /live/initrd.img
}
menuentry "Démarrer depuis le disque local" {
    exit
}
EOF
)
echo "$GRUBCFG" >"$ISO/boot/grub/grub.cfg"
echo "$GRUBCFG" >"$ISO/EFI/BOOT/grub.cfg"
mkdir -p "$ISO/boot/grub/fonts" "$ISO/boot/grub/themes/noos"
cp /usr/share/grub/unicode.pf2 "$ISO/boot/grub/fonts/"
cp "$SRC"/noos-grub/noos/* "$ISO/boot/grub/themes/noos/"
# UEFI : shim et GRUB « CD » signés de Debian (Secure Boot) ; ce GRUB cherche /.disk/info puis
# lit /boot/grub/grub.cfg.
cp /usr/lib/shim/shimx64.efi.signed "$ISO/EFI/BOOT/BOOTX64.EFI"
cp /usr/lib/grub/x86_64-efi-signed/gcdx64.efi.signed "$ISO/EFI/BOOT/grubx64.efi"
cp /usr/lib/shim/mmx64.efi.signed "$ISO/EFI/BOOT/mmx64.efi" 2>/dev/null || true
# Image FAT d'amorçage UEFI (sans montage : mtools).
EFIIMG="$ISO/boot/grub/efi.img"
truncate -s 12M "$EFIIMG"
mkfs.vfat -n NOOS_EFI "$EFIIMG" >/dev/null
mmd -i "$EFIIMG" ::/EFI ::/EFI/BOOT
mcopy -i "$EFIIMG" "$ISO/EFI/BOOT/BOOTX64.EFI" "$ISO/EFI/BOOT/grubx64.efi" "$ISO/EFI/BOOT/grub.cfg" ::/EFI/BOOT/
[ -f "$ISO/EFI/BOOT/mmx64.efi" ] && mcopy -i "$EFIIMG" "$ISO/EFI/BOOT/mmx64.efi" ::/EFI/BOOT/
# BIOS : image El Torito de GRUB (modules pour lire l'ISO et lancer Linux).
mkdir -p "$ISO/boot/grub/i386-pc"
cp /usr/lib/grub/i386-pc/*.mod /usr/lib/grub/i386-pc/*.lst "$ISO/boot/grub/i386-pc/"
grub-mkimage -O i386-pc-eltorito -p /boot/grub -o "$ISO/boot/grub/i386-pc/eltorito.img" \
    biosdisk iso9660 part_gpt part_msdos normal configfile search search_fs_file search_label linux all_video gfxterm echo test

log "ISO hybride"
mkdir -p "$(dirname "$OUT")"
xorriso -as mkisofs \
    -iso-level 3 -full-iso9660-filenames -joliet -rational-rock \
    -volid "$LABEL" -appid "NOOS Debian $SUITE" -publisher "NOOS" \
    -o "$OUT" \
    --grub2-mbr /usr/lib/grub/i386-pc/boot_hybrid.img \
    -partition_offset 16 --mbr-force-bootable \
    -append_partition 2 0xef "$EFIIMG" -appended_part_as_gpt \
    -c boot/boot.cat \
    -b boot/grub/i386-pc/eltorito.img -no-emul-boot -boot-load-size 4 -boot-info-table --grub2-boot-info \
    -eltorito-alt-boot -e '--interval:appended_partition_2:all::' -no-emul-boot \
    "$ISO"
log "ISO prête : $OUT ($(du -h "$OUT" | cut -f1))"

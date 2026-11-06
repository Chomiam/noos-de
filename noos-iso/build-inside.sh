#!/usr/bin/env bash
# Construction de l'ISO live NOOS — exécuté en root dans un conteneur Fedora 45 privilégié
# (lancé par noos-iso/build.sh, rien n'est fait sur l'hôte).
#
#   /src : le dépôt          /work : dossier de travail          /out : ISO produite
#
# 1. système live : dnf --installroot (Fedora 45 « core » + bibliothèques de NOOS)
# 2. NOOS dans /opt/noos, connexion automatique de root sur tty1 → installateur plein écran
# 3. initramfs dracut (dmsquash-live + Plymouth NOOS), squashfs zstd
# 4. GRUB : BIOS (El Torito) + UEFI (shim et GRUB signés de Fedora : Secure Boot)
# 5. ISO hybride (CD, clé USB par dd) avec xorriso
set -euo pipefail

RELEASE=45
LABEL="NOOS-$RELEASE"
SRC=/src
STAGE="$SRC/noos-iso/out/stage"
WORK=/work
ROOTFS="$WORK/rootfs"
ISO="$WORK/iso"
OUT="/out/noos-$RELEASE-x86_64.iso"

log() { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }

[ -x "$STAGE/opt/noos/bin/noos-installer" ] || { echo "Lancez d'abord noos-iso/stage-noos.sh" >&2; exit 1; }

log "Outils de construction"
dnf install -y -q xorriso squashfs-tools mtools dosfstools grub2-tools grub2-tools-extra grub2-pc-modules \
    grub2-efi-x64 grub2-efi-x64-modules shim-x64 >/dev/null

log "Système live (installation réseau des paquets Fedora $RELEASE)"
rm -rf "$WORK" && mkdir -p "$ROOTFS" "$ISO"
mapfile -t PKGS < <(cat "$SRC/noos-iso/packages-common.txt" "$SRC/noos-iso/packages-live.txt" "$STAGE/live-libs.txt" \
    | sed 's/#.*//; s/^\s*//; s/\s*$//; /^$/d' | sort -u)
mkdir -p "$ROOTFS"/{proc,sys,dev}
mount -t proc proc "$ROOTFS/proc"
mount --rbind /sys "$ROOTFS/sys"
mount --rbind /dev "$ROOTFS/dev"
cleanup() { umount -R "$ROOTFS/proc" "$ROOTFS/sys" "$ROOTFS/dev" 2>/dev/null || true; }
trap cleanup EXIT
dnf install -y --installroot="$ROOTFS" --releasever="$RELEASE" --use-host-config \
    --setopt=install_weak_deps=False --skip-unavailable --exclude=kmscon "${PKGS[@]}"
# Fedora 45 remplace la console texte par kmscon : il garderait l'écran (DRM) et
# court-circuiterait la connexion automatique sur tty1. Console classique (agetty).
chroot "$ROOTFS" rpm -e --nodeps kmscon 2>/dev/null || true
# Sans kmscon, Fedora 45 n'a plus d'autovt@ : consoles classiques sur demande + tty1.
ln -sf /usr/lib/systemd/system/getty@.service "$ROOTFS/etc/systemd/system/autovt@.service"
chroot "$ROOTFS" systemctl enable getty@tty1.service

log "NOOS dans /opt/noos"
cp -a "$STAGE/opt/noos" "$ROOTFS/opt/"
# Fichiers système de NOOS aussi dans le live (Plymouth, ld.so.conf.d…).
cp -a "$STAGE/opt/noos/share/noos/system/." "$ROOTFS/"
install -Dm755 "$SRC/noos-iso/noos-live-installer" "$ROOTFS/usr/local/bin/noos-live-installer"
chroot "$ROOTFS" ldconfig

log "Configuration du système live"
echo "noos-live" >"$ROOTFS/etc/hostname"
echo "LANG=fr_FR.UTF-8" >"$ROOTFS/etc/locale.conf"
printf 'KEYMAP=fr\nXKBLAYOUT=fr\nFONT=eurlatgr\n' >"$ROOTFS/etc/vconsole.conf"
ln -sf ../usr/share/zoneinfo/Europe/Paris "$ROOTFS/etc/localtime"
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
chroot "$ROOTFS" systemctl enable NetworkManager.service >/dev/null
chroot "$ROOTFS" systemctl set-default multi-user.target >/dev/null
# Pas de mise en veille pendant l'installation.
mkdir -p "$ROOTFS/etc/systemd/logind.conf.d"
printf '[Login]\nIdleAction=ignore\nHandleLidSwitch=ignore\n' >"$ROOTFS/etc/systemd/logind.conf.d/noos-live.conf"
chroot "$ROOTFS" plymouth-set-default-theme noos

log "initramfs live (dmsquash-live + Plymouth NOOS)"
KVER="$(ls "$ROOTFS/lib/modules" | sort -V | tail -1)"
chroot "$ROOTFS" dracut -f --no-hostonly --add "dmsquash-live plymouth" \
    --omit "crypt" "/boot/initramfs-live.img" "$KVER"
mkdir -p "$ISO/boot" "$ISO/LiveOS" "$ISO/EFI/BOOT" "$ISO/images"
cp "$ROOTFS/lib/modules/$KVER/vmlinuz" "$ISO/boot/vmlinuz"
cp "$ROOTFS/boot/initramfs-live.img" "$ISO/boot/initrd.img"
rm -f "$ROOTFS/boot/initramfs-live.img"
cleanup
trap - EXIT

log "Image compressée du système (squashfs zstd)"
rm -rf "$ROOTFS/var/cache/dnf"/* "$ROOTFS/var/cache/libdnf5"/* 2>/dev/null || true
mksquashfs "$ROOTFS" "$ISO/LiveOS/squashfs.img" -comp zstd -Xcompression-level 15 -b 1M -noappend -quiet

log "GRUB (BIOS + UEFI)"
GRUBCFG=$(cat <<EOF
set default=0
set timeout=5
insmod all_video
insmod gfxterm
insmod png
insmod part_gpt
insmod iso9660
search --no-floppy --set=root --label $LABEL
loadfont (\$root)/boot/grub2/fonts/unicode.pf2
set gfxmode=auto
terminal_output gfxterm
set menu_color_normal=light-gray/black
set menu_color_highlight=white/blue

menuentry "Installer NOOS" --class noos {
    linux /boot/vmlinuz root=live:CDLABEL=$LABEL rd.live.image selinux=0 quiet rhgb
    initrd /boot/initrd.img
}
menuentry "Installer NOOS (graphismes de base)" --class noos {
    linux /boot/vmlinuz root=live:CDLABEL=$LABEL rd.live.image selinux=0 nomodeset quiet rhgb
    initrd /boot/initrd.img
}
menuentry "Installer NOOS (mode texte, dépannage)" {
    linux /boot/vmlinuz root=live:CDLABEL=$LABEL rd.live.image selinux=0 systemd.unit=multi-user.target
    initrd /boot/initrd.img
}
menuentry "Installer NOOS (journal sur port série, dépannage)" {
    linux /boot/vmlinuz root=live:CDLABEL=$LABEL rd.live.image selinux=0 console=tty0 console=ttyS0,115200 noos.serial
    initrd /boot/initrd.img
}
menuentry "Démarrer depuis le disque local" {
    exit
}
EOF
)
# UEFI : shim et GRUB signés de Fedora (Secure Boot) ; GRUB lit grub.cfg à côté de lui.
mkdir -p "$ISO/boot/grub2"
echo "$GRUBCFG" >"$ISO/EFI/BOOT/grub.cfg"
echo "$GRUBCFG" >"$ISO/boot/grub2/grub.cfg"
cp /boot/efi/EFI/fedora/shimx64.efi "$ISO/EFI/BOOT/BOOTX64.EFI"
cp /boot/efi/EFI/fedora/grubx64.efi "$ISO/EFI/BOOT/grubx64.efi"
cp /boot/efi/EFI/fedora/mmx64.efi "$ISO/EFI/BOOT/mmx64.efi" 2>/dev/null || true
mkdir -p "$ISO/boot/grub2/fonts"
cp /usr/share/grub/unicode.pf2 "$ISO/boot/grub2/fonts/"
# Image FAT d'amorçage UEFI (sans montage : mtools).
EFIIMG="$ISO/images/efiboot.img"
truncate -s 12M "$EFIIMG"
mkfs.vfat -n NOOS_EFI "$EFIIMG" >/dev/null
mmd -i "$EFIIMG" ::/EFI ::/EFI/BOOT
mcopy -i "$EFIIMG" "$ISO/EFI/BOOT/BOOTX64.EFI" "$ISO/EFI/BOOT/grubx64.efi" "$ISO/EFI/BOOT/grub.cfg" ::/EFI/BOOT/
[ -f "$ISO/EFI/BOOT/mmx64.efi" ] && mcopy -i "$EFIIMG" "$ISO/EFI/BOOT/mmx64.efi" ::/EFI/BOOT/
# BIOS : image El Torito de GRUB (modules pour lire l'ISO et lancer Linux).
mkdir -p "$ISO/boot/grub2/i386-pc"
cp /usr/lib/grub/i386-pc/*.mod /usr/lib/grub/i386-pc/*.lst "$ISO/boot/grub2/i386-pc/"
grub2-mkimage -O i386-pc-eltorito -p /boot/grub2 -o "$ISO/boot/grub2/i386-pc/eltorito.img" \
    biosdisk iso9660 part_gpt part_msdos normal configfile search search_label linux all_video gfxterm echo test

log "ISO hybride"
mkdir -p /out
xorriso -as mkisofs \
    -iso-level 3 -full-iso9660-filenames -joliet -rational-rock \
    -volid "$LABEL" -appid "NOOS $RELEASE" -publisher "NOOS" \
    -o "$OUT" \
    --grub2-mbr /usr/lib/grub/i386-pc/boot_hybrid.img \
    -partition_offset 16 --mbr-force-bootable \
    -append_partition 2 0xef "$EFIIMG" -appended_part_as_gpt \
    -c boot/boot.cat \
    -b boot/grub2/i386-pc/eltorito.img -no-emul-boot -boot-load-size 4 -boot-info-table --grub2-boot-info \
    -eltorito-alt-boot -e '--interval:appended_partition_2:all::' -no-emul-boot \
    "$ISO"
log "ISO prête : $OUT ($(du -h "$OUT" | cut -f1))"

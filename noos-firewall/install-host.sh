#!/usr/bin/env bash
# Installe le pare-feu NOOS sur cette machine depuis le dépôt (sans passer par le paquet) :
#
#   cargo build --release -p noos-firewall -p noos-firewall-prompt -p noos-settings
#   sudo ./noos-firewall/install-host.sh            # binaires dans /opt/noos/bin + service
#   sudo ./noos-firewall/install-host.sh --uninstall
#
# À lancer en root (sudo). Le service démarre aussitôt ; si ufw ou firewalld sont actifs,
# systemd les arrête (Conflicts=). Sans fenêtre d'autorisation connectée, les sorties restent
# autorisées : rouvrir la session (ou lancer noos-firewall-prompt) pour recevoir les demandes.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
PREFIX=/opt/noos
[ "$(id -u)" = 0 ] || { echo "À lancer avec sudo : sudo $0 $*" >&2; exit 1; }

if [ "${1:-}" = --uninstall ]; then
    systemctl disable --now noos-firewall.service 2>/dev/null || true
    nft delete table inet noos 2>/dev/null || true
    rm -f /usr/lib/systemd/system/noos-firewall.service /usr/share/polkit-1/actions/org.noos.firewall.policy \
          /etc/xdg/autostart/org.noos.FirewallPrompt.desktop /usr/lib/modules-load.d/noos-firewall.conf
    rm -f "$PREFIX/bin/noos-firewalld" "$PREFIX/bin/noos-firewall" "$PREFIX/bin/noos-firewall-prompt"
    systemctl daemon-reload
    echo "Pare-feu NOOS retiré (règles gardées dans /etc/noos/firewall)."
    exit 0
fi

command -v nft >/dev/null || { echo "nftables manque : sudo apt install nftables" >&2; exit 1; }
for b in noos-firewalld noos-firewall noos-firewall-prompt; do
    [ -x "$ROOT/target/release/$b" ] || { echo "$b non compilé : cargo build --release -p noos-firewall -p noos-firewall-prompt" >&2; exit 1; }
done
# libAdapta : la fenêtre d'autorisation en a besoin (paquet noos ou copie du dépôt).
if [ ! -e "$PREFIX/lib/libadapta-1.so.0" ]; then
    install -d "$PREFIX/lib"
    cp -P "$ROOT"/target/libadapta-build/src/libadapta-1.so* "$PREFIX/lib/"
    echo "$PREFIX/lib" > /etc/ld.so.conf.d/noos.conf
    ldconfig
fi
install -d "$PREFIX/bin"
for b in noos-firewalld noos-firewall noos-firewall-prompt; do
    install -m755 "$ROOT/target/release/$b" "$PREFIX/bin/$b"
done
"$HERE/install-system.sh" / "$PREFIX"
# La table générée doit être acceptée par nft avant de démarrer quoi que ce soit.
"$PREFIX/bin/noos-firewall" check
systemctl daemon-reload
modprobe nfnetlink_queue || true
systemctl enable --now noos-firewall.service
sleep 1
systemctl --no-pager --lines=5 status noos-firewall.service || true
echo
echo "Pare-feu NOOS installé. Fenêtre d'autorisation : se déconnecter / reconnecter, ou lancer"
echo "  $PREFIX/bin/noos-firewall-prompt &"
echo "En cas de problème réseau : sudo $PREFIX/bin/noos-firewall off   (retire la table)"

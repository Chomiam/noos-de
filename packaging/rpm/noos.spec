# Paquet RPM de NOOS (COPR). Construit depuis le dépôt GitHub par .copr/Makefile, qui
# fournit les sources (git archive) et les dépendances Rust « vendored » (construction
# hors ligne). Version de développement : Release = date + commit, fournie par
# --define "noos_release …".

%global debug_package %{nil}
# libAdapta est une bibliothèque privée de NOOS (/opt/noos/lib) : ni Provides ni Requires.
%global __provides_exclude_from ^/opt/noos/lib/.*$
%global __requires_exclude ^libadapta-1\\.so.*$

%global noos_bins noos-comp noos-shell noos-file noos-settings noos-software noos-pkg noos-snip noos-term noos-keys noos-login noos-installer

Name:           noos
Version:        0.1.0
Release:        %{?noos_release}%{!?noos_release:1}%{?dist}
Summary:        Bureau Wayland NOOS (Smithay, GTK4 + libAdapta), façon Windows 11

License:        GPL-3.0-or-later AND LGPL-2.1-or-later
URL:            https://github.com/Chomiam/noos-de
Source0:        noos-%{version}.tar.gz
Source1:        noos-%{version}-vendor.tar.xz

ExclusiveArch:  x86_64 aarch64

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  meson
BuildRequires:  ninja-build
BuildRequires:  python3
BuildRequires:  sassc
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(appstream)
BuildRequires:  pkgconfig(fribidi)
BuildRequires:  pkgconfig(libseat)
BuildRequires:  pkgconfig(libinput)
BuildRequires:  pkgconfig(libudev)
BuildRequires:  pkgconfig(gbm)
BuildRequires:  pkgconfig(libdrm)
BuildRequires:  pkgconfig(egl)
BuildRequires:  pkgconfig(xkbcommon)
BuildRequires:  pkgconfig(libdisplay-info)
BuildRequires:  pkgconfig(gtk4-layer-shell-0)
BuildRequires:  pkgconfig(vte-2.91-gtk4)

# Session, écran de connexion, démarrage.
Requires:       greetd
Requires:       plymouth-plugin-script
# Image.Text du script Plymouth : sans ce greffon, plymouthd plante (SIGSEGV).
Requires:       plymouth-plugin-label
Requires:       polkit
Requires:       dbus-tools
Requires:       xdg-desktop-portal
Requires:       xdg-desktop-portal-gtk
Requires:       xdg-desktop-portal-wlr
Requires:       pipewire
Requires:       wireplumber
Requires:       pipewire-utils
Requires:       NetworkManager
Requires:       papirus-icon-theme
# Logiciels : applications Flatpak (Flathub + Fedora Flatpaks) et paquets dnf.
Requires:       flatpak
# Thème GTK3 qui suit les couleurs de la palette NOOS.
Requires:       adw-gtk3-theme
Recommends:     xwayland-satellite
Recommends:     grim
Recommends:     bluez
# Trousseau : oo7 + pam_oo7 (déverrouillé à la connexion par greetd, déjà prévu dans
# /usr/lib/pam.d/greetd) + gcr-prompter (fenêtre de saisie org.gnome.keyring.SystemPrompter,
# sans laquelle oo7 ne peut jamais demander le mot de passe).
Requires:       oo7-daemon
Requires:       pam_oo7
Requires:       gcr3
Recommends:     libtimezonemap
Recommends:     curl

%description
NOOS est un environnement de bureau Wayland écrit en Rust : compositeur Smithay, barre
des tâches et menu Démarrer façon Windows 11, Paramètres, explorateur de fichiers,
terminal, outil de capture, porte-clés, logithèque, écran de connexion (greetd) et
installateur, tous en GTK4 + libAdapta. Pensé pour le jeu (VRR, HDR) et entièrement
personnalisable.

%prep
%autosetup -n noos-%{version}
tar -xf %{SOURCE1}
# Dépendances Rust fournies avec les sources : construction hors ligne.
cat vendor/vendor-config.toml >> .cargo/config.toml

%build
export CARGO_HOME="$PWD/.cargo-home"
export RUSTFLAGS="%{?build_rustflags}"
meson setup target/libadapta-build third_party/libadapta --buildtype=release --wrap-mode=nodownload \
    -Dintrospection=disabled -Dvapi=false -Dtests=false -Dexamples=false -Dgtk_doc=false
ninja -C target/libadapta-build %{?_smp_mflags}
# rpmbuild exporte son propre PKG_CONFIG_PATH (qui l'emporte sur .cargo/config.toml) :
# on y ajoute le .pc « uninstalled » de libAdapta.
export PKG_CONFIG_PATH="$PWD/target/libadapta-build/meson-uninstalled${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
cargo build --release --offline $(for b in %{noos_bins}; do echo -n "-p $b "; done)

%install
install -d %{buildroot}/opt/noos/bin %{buildroot}/opt/noos/lib \
    %{buildroot}/opt/noos/share/noos %{buildroot}/opt/noos/share/applications
for b in %{noos_bins}; do
    install -m755 target/release/$b %{buildroot}/opt/noos/bin/$b
done
for f in target/libadapta-build/src/libadapta-1.so*; do
    [ -d "$f" ] || cp -P "$f" %{buildroot}/opt/noos/lib/
done
cp -r data/. %{buildroot}/opt/noos/share/noos/
install -m644 noos-session/org.noos.*.desktop %{buildroot}/opt/noos/share/applications/

# Session (GDM, greetd) et écran de connexion.
install -Dm755 noos-session/noos-session %{buildroot}%{_bindir}/noos-session
install -Dm755 noos-login/noos-greeter %{buildroot}%{_bindir}/noos-greeter
install -d %{buildroot}%{_datadir}/wayland-sessions
sed 's|/usr/local/bin/noos-session|%{_bindir}/noos-session|g' noos-session/noos.desktop \
    > %{buildroot}%{_datadir}/wayland-sessions/noos.desktop
install -Dm644 noos-session/noos-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/noos-portals.conf
install -d %{buildroot}%{_sysconfdir}/ld.so.conf.d
echo /opt/noos/lib > %{buildroot}%{_sysconfdir}/ld.so.conf.d/noos.conf

# Thème Plymouth (images générées, présentes dans le dépôt).
install -d %{buildroot}%{_datadir}/plymouth/themes/noos
install -m644 noos-plymouth/noos/*.png noos-plymouth/noos/noos.plymouth noos-plymouth/noos/noos.script \
    %{buildroot}%{_datadir}/plymouth/themes/noos/

%post
/sbin/ldconfig
# Flathub actif par défaut, à côté de Fedora Flatpaks (fichier embarqué : pas de réseau).
# Une seule fois : si l'utilisateur retire Flathub, une mise à jour ne le remet pas.
if [ ! -e /var/lib/noos/flathub-added ] && command -v flatpak >/dev/null 2>&1; then
    flatpak remote-add --system --if-not-exists flathub /opt/noos/share/noos/flathub.flatpakrepo >/dev/null 2>&1 || :
    flatpak remote-modify --system --no-filter --enable flathub >/dev/null 2>&1 || :
    mkdir -p /var/lib/noos && touch /var/lib/noos/flathub-added
fi

%postun -p /sbin/ldconfig

%files
%license LICENSE
%doc README.md
/opt/noos
%{_bindir}/noos-session
%{_bindir}/noos-greeter
%{_datadir}/wayland-sessions/noos.desktop
%{_datadir}/xdg-desktop-portal/noos-portals.conf
%{_datadir}/plymouth/themes/noos
%config(noreplace) %{_sysconfdir}/ld.so.conf.d/noos.conf

%changelog
* Fri Nov 06 2026 Axel Valens <axel.valens@gmail.com> - 0.1.0-1
- Premier paquet COPR de NOOS

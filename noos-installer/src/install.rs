//! Moteur d'installation : partitionnement Btrfs, installation réseau (dnf --installroot
//! sur les dépôts Fedora 45 + RPM Fusion), copie de NOOS, configuration et GRUB.
//!
//! Tourne dans un fil séparé ; chaque commande est journalisée ligne à ligne (stdout et
//! stderr) dans l'interface et dans `/tmp/noos-install.log` (copié ensuite dans
//! `/var/log/noos-install.log` du système installé).
//!
//! `NOOS_INSTALL_DRYRUN=1` : simulation (commandes affichées, rien n'est exécuté).

use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};

use crate::{
    disk::{self, MIB, Role},
    model::{self, Choices},
};

pub const TARGET: &str = "/mnt/noos-target";
const BTRFS_TOP: &str = "/mnt/noos-btrfs";
pub const LOG_FILE: &str = "/tmp/noos-install.log";
const RELEASE: &str = "45";
/// Fichiers de NOOS dans le système live (copiés tels quels dans le système installé).
const NOOS_PREFIX: &str = "/opt/noos";

pub const STEPS: &[&str] = &[
    "Vérifications",
    "Partitionnement du disque",
    "Formatage",
    "Sous-volumes Btrfs",
    "Téléchargement et installation de Fedora",
    "Dépôts RPM Fusion",
    "Installation de NOOS",
    "Configuration du système",
    "Chargeur d'amorçage GRUB",
    "Finalisation",
];

#[derive(Debug)]
pub enum Event {
    Step(usize),
    Line(String),
    /// Avancement dans l'étape en cours (0 – 1).
    Progress(f64),
    Done,
    Failed { step: usize, error: String },
}

struct Ctx {
    tx: async_channel::Sender<Event>,
    log: Mutex<Option<std::fs::File>>,
    dry: bool,
    step: Mutex<usize>,
}

impl Ctx {
    fn line(&self, s: impl Into<String>) {
        let s = s.into();
        if let Some(f) = self.log.lock().unwrap().as_mut() {
            let _ = writeln!(f, "{s}");
        }
        let _ = self.tx.send_blocking(Event::Line(s));
    }

    fn step(&self, i: usize) {
        *self.step.lock().unwrap() = i;
        self.line(format!("\n━━━ {} ━━━", STEPS[i]));
        let _ = self.tx.send_blocking(Event::Step(i));
    }

    fn progress(&self, p: f64) {
        let _ = self.tx.send_blocking(Event::Progress(p.clamp(0.0, 1.0)));
    }

    /// Exécute une commande en journalisant sa sortie. `on_line` reçoit chaque ligne.
    fn run_with(&self, program: &str, args: &[&str], stdin: Option<&str>, on_line: &dyn Fn(&str)) -> anyhow::Result<()> {
        let shown: Vec<String> = args.iter().map(|a| if a.contains(' ') { format!("'{a}'") } else { a.to_string() }).collect();
        self.line(format!("$ {program} {}", shown.join(" ")));
        if self.dry {
            std::thread::sleep(std::time::Duration::from_millis(120));
            return Ok(());
        }
        let mut child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C.UTF-8")
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("impossible de lancer {program}"))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input.as_bytes())?;
        }
        let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let err = child.stderr.take().unwrap();
        let (tx, tail2) = (self.tx.clone(), tail.clone());
        let log_err = self.log.lock().unwrap().as_ref().and_then(|f| f.try_clone().ok());
        let err_thread = std::thread::spawn(move || {
            let mut log = log_err;
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if let Some(f) = log.as_mut() {
                    let _ = writeln!(f, "{line}");
                }
                let mut t = tail2.lock().unwrap();
                t.push(line.clone());
                if t.len() > 12 {
                    t.remove(0);
                }
                let _ = tx.send_blocking(Event::Line(line));
            }
        });
        for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
            // dnf réécrit ses barres de progression avec \r : on garde la dernière.
            let line = line.rsplit('\r').next().unwrap_or(&line).to_string();
            on_line(&line);
            {
                let mut t = tail.lock().unwrap();
                t.push(line.clone());
                if t.len() > 12 {
                    t.remove(0);
                }
            }
            self.line(line);
        }
        let _ = err_thread.join();
        let status = child.wait()?;
        if !status.success() {
            let t = tail.lock().unwrap().join("\n");
            bail!("{program} a échoué ({status})\n{t}");
        }
        Ok(())
    }

    fn run(&self, program: &str, args: &[&str]) -> anyhow::Result<()> {
        self.run_with(program, args, None, &|_| {})
    }

    /// Comme `run`, mais un échec n'interrompt pas l'installation.
    fn try_run(&self, program: &str, args: &[&str]) {
        if let Err(e) = self.run(program, args) {
            self.line(format!("(ignoré) {e:#}"));
        }
    }

    fn chroot(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut a = vec![TARGET];
        a.extend_from_slice(args);
        self.run("chroot", &a)
    }

    fn write(&self, path: &str, content: &str) -> anyhow::Result<()> {
        let full = format!("{TARGET}{path}");
        self.line(format!("écriture de {path}"));
        if self.dry {
            return Ok(());
        }
        if let Some(dir) = Path::new(&full).parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&full, content).with_context(|| format!("écriture de {full}"))
    }
}

/// Lance l'installation dans un fil séparé.
pub fn start(choices: Choices) -> async_channel::Receiver<Event> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let ctx = Ctx {
            tx: tx.clone(),
            log: Mutex::new(std::fs::File::create(LOG_FILE).ok()),
            dry: std::env::var("NOOS_INSTALL_DRYRUN").is_ok_and(|v| v == "1"),
            step: Mutex::new(0),
        };
        if ctx.dry {
            ctx.line("Mode simulation (NOOS_INSTALL_DRYRUN=1) : aucune commande n'est exécutée.");
        }
        let res = install(&ctx, &choices);
        match res {
            Ok(()) => {
                let _ = tx.send_blocking(Event::Done);
            }
            Err(e) => {
                let step = *ctx.step.lock().unwrap();
                ctx.line(format!("\nERREUR : {e:#}"));
                cleanup(&ctx);
                let _ = tx.send_blocking(Event::Failed { step, error: format!("{e:#}") });
            }
        }
    });
    rx
}

fn cleanup(ctx: &Ctx) {
    ctx.line("Démontage du système cible…");
    ctx.try_run("umount", &["-R", "-l", TARGET]);
    ctx.try_run("umount", &["-l", BTRFS_TOP]);
}

fn uuid(ctx: &Ctx, dev: &str) -> anyhow::Result<String> {
    if ctx.dry {
        return Ok("00000000-0000-0000-0000-000000000000".into());
    }
    let out = Command::new("blkid").args(["-s", "UUID", "-o", "value", dev]).output()?;
    let u = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if u.is_empty() {
        bail!("UUID introuvable pour {dev}");
    }
    Ok(u)
}

/// Paquets du système : liste fournie par l'ISO + paquets selon le matériel et la langue.
fn packages(c: &Choices, uefi: bool) -> Vec<String> {
    let listed = std::fs::read_to_string(format!("{NOOS_PREFIX}/share/noos/install/packages.txt")).unwrap_or_default();
    let mut p: Vec<String> = listed
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if p.is_empty() {
        // Liste de secours (ISO incomplète) : de quoi démarrer NOOS.
        for x in [
            "@core", "kernel", "linux-firmware", "btrfs-progs", "NetworkManager-wifi", "plymouth", "plymouth-plugin-script",
            "plymouth-plugin-label", "adw-gtk3-theme",
            "greetd", "gtk4", "vte291-gtk4", "libseat", "gtk4-layer-shell", "pipewire", "wireplumber", "xdg-desktop-portal",
            "xdg-desktop-portal-gtk", "polkit", "papirus-icon-theme", "google-noto-sans-fonts", "mesa-dri-drivers",
        ] {
            p.push(x.into());
        }
    }
    p.push(model::langpack(&c.locale));
    p.extend(["grub2-tools", "grub2-common", "efibootmgr"].map(String::from));
    if uefi {
        p.extend(["grub2-efi-x64", "shim-x64", "dosfstools"].map(String::from));
    } else {
        p.push("grub2-pc".into());
    }
    p.sort();
    p.dedup();
    p
}

fn console_keymap(layout: &str, variant: &str) -> String {
    match (layout, variant) {
        ("fr", "bepo") => "fr-bepo".into(),
        ("fr", _) => "fr".into(),
        ("be", _) => "be-latin1".into(),
        ("ch", "fr") => "fr_CH".into(),
        ("ch", _) => "de_CH".into(),
        ("ca", _) => "cf".into(),
        ("de", _) => "de".into(),
        ("gb", _) => "uk".into(),
        ("br", _) => "br-abnt2".into(),
        (l, _) => l.into(),
    }
}

fn install(ctx: &Ctx, c: &Choices) -> anyhow::Result<()> {
    let Some(target_disk) = c.disk.clone() else { bail!("aucun disque choisi") };
    let dev = target_disk.path.as_str();
    let uefi = disk::uefi();
    let plan = disk::plan(target_disk.size, uefi, c.swap.then_some(c.swap_gib));
    let part = |role: Role| plan.iter().find(|p| p.role == role).map(|p| target_disk.part_path(p.number));

    // ── 0. Vérifications ──
    ctx.step(0);
    if !ctx.dry && unsafe_geteuid() != 0 {
        bail!("l'installateur doit tourner en root");
    }
    ctx.line(format!("Démarrage {} · disque {} ({})", if uefi { "UEFI" } else { "BIOS" }, dev, disk::human(target_disk.size)));
    ctx.run("curl", &["-sSfI", "--max-time", "20", "-o", "/dev/null", "https://mirrors.fedoraproject.org/"])
        .context("pas d'accès à Internet : l'installation réseau télécharge Fedora depuis les miroirs officiels")?;

    // ── 1. Partitionnement ──
    ctx.step(1);
    for p in &target_disk.parts {
        ctx.try_run("swapoff", &[&p.path]);
        ctx.try_run("umount", &["-l", &p.path]);
    }
    ctx.run("wipefs", &["-a", "-f", dev])?;
    let mut script = String::from("label: gpt\n");
    for p in &plan {
        let size = if p.role == Role::Swap || (p.role == Role::Root && !c.swap) {
            String::new()
        } else {
            format!("size={}MiB, ", p.size / MIB)
        };
        let (ty, name) = match p.role {
            Role::BiosBoot => ("21686148-6449-6E6F-744E-656564454649", "bios"),
            Role::Efi => ("C12A7328-F81F-11D2-BA4B-00A0C93EC93B", "EFI"),
            Role::Boot => ("0FC63DAF-8483-4772-8E79-3D69D8477DE4", "boot"),
            Role::Root => ("4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709", "NOOS"),
            Role::Swap => ("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F", "swap"),
        };
        script.push_str(&format!("{size}type={ty}, name=\"{name}\"\n"));
    }
    ctx.line(script.trim_end().to_string());
    ctx.run_with("sfdisk", &["--wipe", "always", "--wipe-partitions", "always", dev], Some(&script), &|_| {})?;
    ctx.try_run("udevadm", &["settle", "--timeout=15"]);

    // ── 2. Formatage ──
    ctx.step(2);
    let root = part(Role::Root).unwrap();
    let boot = part(Role::Boot).unwrap();
    if let Some(efi) = part(Role::Efi) {
        ctx.run("mkfs.fat", &["-F", "32", "-n", "EFI", &efi])?;
    }
    ctx.progress(0.25);
    ctx.run("mkfs.ext4", &["-F", "-q", "-L", "boot", &boot])?;
    ctx.progress(0.5);
    ctx.run("mkfs.btrfs", &["-f", "-L", "NOOS", &root])?;
    if let Some(swap) = part(Role::Swap) {
        ctx.run("mkswap", &["-L", "swap", &swap])?;
    }
    ctx.progress(1.0);

    // ── 3. Sous-volumes ──
    ctx.step(3);
    ctx.run("mkdir", &["-p", BTRFS_TOP, TARGET])?;
    ctx.run("mount", &[&root, BTRFS_TOP])?;
    ctx.run("btrfs", &["subvolume", "create", &format!("{BTRFS_TOP}/@")])?;
    ctx.run("btrfs", &["subvolume", "create", &format!("{BTRFS_TOP}/@home")])?;
    ctx.run("umount", &[BTRFS_TOP])?;
    const OPTS: &str = "compress=zstd:1,noatime";
    ctx.run("mount", &["-o", &format!("subvol=@,{OPTS}"), &root, TARGET])?;
    ctx.run("mkdir", &["-p", &format!("{TARGET}/home"), &format!("{TARGET}/boot")])?;
    ctx.run("mount", &["-o", &format!("subvol=@home,{OPTS}"), &root, &format!("{TARGET}/home")])?;
    ctx.run("mount", &[&boot, &format!("{TARGET}/boot")])?;
    if let Some(efi) = part(Role::Efi) {
        ctx.run("mkdir", &["-p", &format!("{TARGET}/boot/efi")])?;
        ctx.run("mount", &["-o", "umask=0077", &efi, &format!("{TARGET}/boot/efi")])?;
    }
    for d in ["proc", "sys", "dev", "run", "etc/kernel", "etc/default", "var/log"] {
        ctx.run("mkdir", &["-p", &format!("{TARGET}/{d}")])?;
    }
    ctx.run("mount", &["-t", "proc", "proc", &format!("{TARGET}/proc")])?;
    ctx.run("mount", &["--rbind", "/sys", &format!("{TARGET}/sys")])?;
    ctx.run("mount", &["--make-rslave", &format!("{TARGET}/sys")])?;
    ctx.run("mount", &["--rbind", "/dev", &format!("{TARGET}/dev")])?;
    ctx.run("mount", &["--make-rslave", &format!("{TARGET}/dev")])?;
    ctx.run("mount", &["-t", "tmpfs", "tmpfs", &format!("{TARGET}/run")])?;

    // Avant les paquets : fstab, cmdline du noyau (entrées BLS) et GRUB.
    let (root_uuid, boot_uuid) = (uuid(ctx, &root)?, uuid(ctx, &boot)?);
    let mut fstab = format!(
        "# /etc/fstab — créé par l'installateur NOOS\nUUID={root_uuid} /      btrfs subvol=@,{OPTS} 0 0\nUUID={root_uuid} /home  btrfs subvol=@home,{OPTS} 0 0\nUUID={boot_uuid} /boot  ext4  defaults 1 2\n"
    );
    if let Some(efi) = part(Role::Efi) {
        fstab.push_str(&format!("UUID={} /boot/efi vfat umask=0077,shortname=winnt 0 2\n", uuid(ctx, &efi)?));
    }
    if let Some(swap) = part(Role::Swap) {
        fstab.push_str(&format!("UUID={} none   swap  defaults 0 0\n", uuid(ctx, &swap)?));
    }
    ctx.write("/etc/fstab", &fstab)?;
    let cmdline = format!("root=UUID={root_uuid} rootflags=subvol=@ ro rhgb quiet");
    ctx.write("/etc/kernel/cmdline", &format!("{cmdline}\n"))?;
    ctx.write(
        "/etc/default/grub",
        "GRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR=\"NOOS\"\nGRUB_DEFAULT=saved\nGRUB_DISABLE_SUBMENU=true\nGRUB_TERMINAL_OUTPUT=\"gfxterm\"\nGRUB_CMDLINE_LINUX=\"rootflags=subvol=@ rhgb quiet\"\nGRUB_DISABLE_RECOVERY=\"true\"\nGRUB_ENABLE_BLSCFG=true\n",
    )?;
    ctx.run("systemd-machine-id-setup", &["--root", TARGET])?;

    // ── 4. Paquets (installation réseau) ──
    ctx.step(4);
    let pkgs = packages(c, uefi);
    ctx.line(format!("{} paquets demandés (et leurs dépendances)", pkgs.len()));
    let mut args: Vec<String> = vec![
        "install".into(),
        "-y".into(),
        format!("--installroot={TARGET}"),
        format!("--releasever={RELEASE}"),
        "--use-host-config".into(),
        "--setopt=keepcache=False".into(),
        // Un paquet absent d'un miroir ne doit pas bloquer toute l'installation.
        "--skip-unavailable".into(),
        // kmscon (console de Fedora 45) garderait l'écran de tty1 : greetd l'utilise.
        "--exclude=kmscon".into(),
        // Paquets téléchargés sur le disque cible, pas dans la RAM du système live.
        format!("--setopt=cachedir={TARGET}/var/cache/noos-install"),
    ];
    args.extend(pkgs);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    // « [ 12/345] Installing … » : avancement.
    let tx = ctx.tx.clone();
    ctx.run_with("dnf", &refs, None, &move |line: &str| {
        if let Some((n, m)) = dnf_counter(line) {
            let _ = tx.send_blocking(Event::Progress(n as f64 / m.max(1) as f64));
        }
    })?;

    // ── 5. RPM Fusion ──
    ctx.step(5);
    if c.rpmfusion {
        let free = format!("https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-{RELEASE}.noarch.rpm");
        let nonfree = format!("https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-{RELEASE}.noarch.rpm");
        // Paquets de dépôt téléchargés en HTTPS depuis le miroir officiel ; leurs clés GPG
        // ne sont pas encore connues du système cible.
        ctx.run(
            "dnf",
            &["install", "-y", &format!("--installroot={TARGET}"), &format!("--releasever={RELEASE}"), "--use-host-config", "--nogpgcheck", &free, &nonfree],
        )?;
    } else {
        ctx.line("RPM Fusion non demandé : étape ignorée.");
    }

    // ── 6. NOOS ──
    // Paquet « noos » du dépôt COPR : dernière version, et mises à jour avec dnf ensuite.
    // Sans accès au dépôt, copie de la version fournie avec l'ISO.
    ctx.step(6);
    let from_copr = match copr_project() {
        Some((owner, project)) => {
            ctx.write(&format!("/etc/yum.repos.d/_copr:copr.fedorainfracloud.org:{owner}:{project}.repo"), &copr_repo(&owner, &project))?;
            ctx.progress(0.2);
            let res = ctx.run(
                "dnf",
                &[
                    "install",
                    "-y",
                    &format!("--installroot={TARGET}"),
                    &format!("--releasever={RELEASE}"),
                    &format!("--setopt=cachedir={TARGET}/var/cache/noos-install"),
                    "noos",
                ],
            );
            match res {
                Ok(()) => true,
                Err(e) => {
                    ctx.line(format!("(attention) paquet NOOS indisponible sur COPR ({e:#}) : version de l'ISO"));
                    false
                }
            }
        }
        None => false,
    };
    if !from_copr {
        ctx.run("cp", &["-a", NOOS_PREFIX, &format!("{TARGET}/opt/")])?;
        ctx.progress(0.6);
        // Fichiers système préparés par la construction de l'ISO (session, greeter, plymouth…).
        let system = format!("{NOOS_PREFIX}/share/noos/system/.");
        if ctx.dry || Path::new(&system).exists() {
            ctx.run("cp", &["-a", &system, &format!("{TARGET}/")])?;
            // sudo refuse un fichier de sudoers.d qui n'appartient pas à root.
            ctx.try_run("chown", &["-R", "root:root", &format!("{TARGET}/etc/sudoers.d")]);
        } else {
            ctx.line("(attention) fichiers système NOOS absents de l'ISO");
        }
    }
    ctx.progress(1.0);

    // ── 7. Configuration ──
    ctx.step(7);
    ctx.write("/etc/locale.conf", &format!("LANG={}\n", c.locale))?;
    ctx.write(
        "/etc/vconsole.conf",
        &format!(
            "KEYMAP={}\nXKBLAYOUT={}\nXKBVARIANT={}\nFONT=eurlatgr\n",
            console_keymap(&c.kb_layout, &c.kb_variant),
            c.kb_layout,
            c.kb_variant
        ),
    )?;
    ctx.write(
        "/etc/X11/xorg.conf.d/00-keyboard.conf",
        &format!(
            "Section \"InputClass\"\n    Identifier \"system-keyboard\"\n    MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{}\"\n    Option \"XkbVariant\" \"{}\"\nEndSection\n",
            c.kb_layout, c.kb_variant
        ),
    )?;
    ctx.run("ln", &["-sf", &format!("../usr/share/zoneinfo/{}", c.timezone), &format!("{TARGET}/etc/localtime")])?;
    ctx.write("/etc/hostname", &format!("{}\n", c.hostname))?;
    ctx.progress(0.2);
    // Compte administrateur (groupe wheel) ; root verrouillé.
    // fish (fastfetch à l'ouverture d'un terminal) s'il est installé, sinon bash ; il devient
    // aussi le shell des comptes créés plus tard.
    let shell = if ctx.dry || Path::new(&format!("{TARGET}/usr/bin/fish")).exists() { "/usr/bin/fish" } else { "/bin/bash" };
    if shell == "/usr/bin/fish" {
        ctx.try_run("chroot", &[TARGET, "useradd", "-D", "-s", "/usr/bin/fish"]);
    }
    ctx.chroot(&["useradd", "-m", "-G", "wheel", "-c", &c.full_name, "-s", shell, &c.username])?;
    ctx.line(format!("$ chroot {TARGET} chpasswd   (mot de passe masqué)"));
    if !ctx.dry {
        let mut child = Command::new("chroot").args([TARGET, "chpasswd"]).stdin(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        child.stdin.take().unwrap().write_all(format!("{}:{}\n", c.username, c.password).as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("chpasswd : {}", String::from_utf8_lossy(&out.stderr));
        }
    }
    ctx.try_run("chroot", &[TARGET, "passwd", "-l", "root"]);
    ctx.progress(0.4);
    // Écran de connexion noos-login (greetd). Le paquet Fedora crée le compte « greetd »
    // (« greeter » ailleurs) : on prend celui qui existe.
    let greeter = greeter_user();
    ctx.line(format!("compte de l'écran de connexion : {greeter}"));
    ctx.write(
        "/etc/greetd/config.toml",
        &format!("[terminal]\nvt = 1\n\n[default_session]\ncommand = \"{}\"\nuser = \"{greeter}\"\n", greeter_command()),
    )?;
    ctx.run("mkdir", &["-p", &format!("{TARGET}/var/lib/noos-login")])?;
    ctx.chroot(&["chown", &format!("{greeter}:{greeter}"), "/var/lib/noos-login"])?;
    // Thème de l'écran de connexion : modifiable par les administrateurs (Paramètres).
    ctx.run("mkdir", &["-p", &format!("{TARGET}/etc/noos")])?;
    if !Path::new(&format!("{TARGET}/etc/noos/login.toml")).exists() {
        ctx.write("/etc/noos/login.toml", &noos_core::login::LoginConfig::default().to_toml()?)?;
    }
    ctx.try_run("chroot", &[TARGET, "chgrp", "-R", "wheel", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "chmod", "-R", "g+w", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "chmod", "g+s", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "ldconfig"]);
    ctx.progress(0.55);
    for unit in ["greetd.service", "NetworkManager.service", "bluetooth.service", "fstrim.timer", "systemd-oomd.service"] {
        ctx.try_run("systemctl", &["--root", TARGET, "enable", unit]);
    }
    ctx.try_run("systemctl", &["--root", TARGET, "disable", "gdm.service"]);
    // kmscon exclu : consoles texte classiques (Ctrl+Alt+F2…) via agetty.
    ctx.try_run("ln", &["-sf", "/usr/lib/systemd/system/getty@.service", &format!("{TARGET}/etc/systemd/system/autovt@.service")]);
    ctx.run("systemctl", &["--root", TARGET, "set-default", "graphical.target"])?;
    ctx.try_run("chroot", &[TARGET, "plymouth-set-default-theme", "noos"]);
    // Flathub actif par défaut, à côté de Fedora Flatpaks (fichier embarqué : pas de réseau).
    ctx.try_run(
        "chroot",
        &[TARGET, "flatpak", "remote-add", "--system", "--if-not-exists", "flathub", "/opt/noos/share/noos/flathub.flatpakrepo"],
    );
    ctx.try_run("chroot", &[TARGET, "flatpak", "remote-modify", "--system", "--no-filter", "--enable", "flathub"]);
    // Thème GRUB NOOS (grub.cfg est régénéré à l'étape GRUB).
    ctx.try_run("chroot", &[TARGET, "/usr/share/noos-grub/install.sh"]);
    // Instantanés Btrfs (snapper) : configuration de / et instantanés avant / après dnf.
    ctx.line("Configuration des instantanés du système (Sauvegarder et Restaurer)…");
    ctx.try_run("chroot", &[TARGET, "/opt/noos/libexec/noos-save-helper", "setup"]);
    // SELinux : le système live tourne sans SELinux (selinux=0), les fichiers créés ici
    // (comptes, /opt/noos, configuration) n'ont pas d'étiquette. Étiquetage complet hors
    // ligne avec la politique du système installé (sinon systemd ne démarre pas en
    // mode « enforcing »).
    ctx.line("Étiquetage SELinux du système installé…");
    ctx.chroot(&[
        "setfiles", "-F", "-e", "/proc", "-e", "/sys", "-e", "/dev", "-e", "/run", "-e", "/boot/efi",
        "/etc/selinux/targeted/contexts/files/file_contexts", "/",
    ])?;
    // Premier instantané, une fois le système étiqueté (sinon le restaurer donnerait un
    // système sans étiquettes SELinux) ; seuls ses fichiers de suivi sont étiquetés ensuite.
    ctx.try_run("chroot", &[TARGET, "/opt/noos/libexec/noos-save-helper", "create", "Installation de NOOS"]);
    ctx.try_run("chroot", &[TARGET, "sh", "-c", "restorecon -F /.snapshots /.snapshots/* /.snapshots/*/info.xml 2>/dev/null || true"]);
    ctx.progress(1.0);

    // ── 8. GRUB ──
    ctx.step(8);
    ctx.line("Régénération de l'initramfs (thème Plymouth NOOS inclus)…");
    ctx.chroot(&["dracut", "-f", "--regenerate-all", "--no-hostonly"])?;
    ctx.progress(0.5);
    if uefi {
        // Configuration GRUB « relais » de Fedora : la vraie est dans /boot/grub2.
        ctx.write(
            "/boot/efi/EFI/fedora/grub.cfg",
            &format!("search --no-floppy --fs-uuid --set=dev {boot_uuid}\nset prefix=($dev)/grub2\nexport $prefix\nconfigfile $prefix/grub.cfg\n"),
        )?;
        ctx.chroot(&["grub2-mkconfig", "-o", "/boot/grub2/grub.cfg"])?;
        let efi_num = plan.iter().find(|p| p.role == Role::Efi).map(|p| p.number).unwrap_or(1).to_string();
        ctx.run("efibootmgr", &["--create", "--disk", dev, "--part", &efi_num, "--label", "NOOS", "--loader", "\\EFI\\fedora\\shimx64.efi"])?;
    } else {
        ctx.chroot(&["grub2-install", "--target=i386-pc", dev])?;
        ctx.chroot(&["grub2-mkconfig", "-o", "/boot/grub2/grub.cfg"])?;
    }
    ctx.progress(1.0);

    // ── 9. Finalisation ──
    ctx.step(9);
    ctx.line("Installation terminée : copie du journal et démontage.");
    ctx.try_run("rm", &["-rf", &format!("{TARGET}/var/cache/noos-install")]);
    if !ctx.dry {
        let _ = std::fs::copy(LOG_FILE, format!("{TARGET}/var/log/noos-install.log"));
    }
    ctx.try_run("sync", &[]);
    ctx.run("umount", &["-R", TARGET])?;
    ctx.progress(1.0);
    Ok(())
}

/// Projet COPR de NOOS (« propriétaire/projet »), fourni par l'ISO.
fn copr_project() -> Option<(String, String)> {
    let text = std::fs::read_to_string(format!("{NOOS_PREFIX}/share/noos/install/copr")).ok()?;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let (owner, project) = line.split_once('/')?;
    Some((owner.to_string(), project.to_string()))
}

/// Fichier de dépôt COPR (identique à celui de `dnf copr enable`).
fn copr_repo(owner: &str, project: &str) -> String {
    let base = format!("https://download.copr.fedorainfracloud.org/results/{owner}/{project}");
    format!(
        "[copr:copr.fedorainfracloud.org:{owner}:{project}]\nname=Copr repo for {project} owned by {owner}\nbaseurl={base}/fedora-$releasever-$basearch/\ntype=rpm-md\nskip_if_unavailable=True\ngpgcheck=1\ngpgkey={base}/pubkey.gpg\nrepo_gpgcheck=0\nenabled=1\nenabled_metadata=1\n"
    )
}

/// Lanceur de l'écran de connexion : paquet RPM (/usr/bin) ou copie de l'ISO (/usr/local/bin).
fn greeter_command() -> &'static str {
    if Path::new(&format!("{TARGET}/usr/bin/noos-greeter")).exists() { "/usr/bin/noos-greeter" } else { "/usr/local/bin/noos-greeter" }
}

/// Compte système de l'écran de connexion dans le système cible.
fn greeter_user() -> String {
    let passwd = std::fs::read_to_string(format!("{TARGET}/etc/passwd")).unwrap_or_default();
    ["greetd", "greeter"]
        .into_iter()
        .find(|u| passwd.lines().any(|l| l.starts_with(&format!("{u}:"))))
        .map(String::from)
        .unwrap_or_else(|| "greetd".into())
}

/// « [ 12/345] … » (dnf5) → (12, 345).
pub fn dnf_counter(line: &str) -> Option<(u32, u32)> {
    let s = line.trim_start().strip_prefix('[')?;
    let (inner, _) = s.split_once(']')?;
    let (a, b) = inner.split_once('/')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

pub fn is_root() -> bool {
    unsafe_geteuid() == 0
}

fn unsafe_geteuid() -> u32 {
    // Sans dépendance à libc : /proc/self/status (« Uid: réel effectif … »).
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("Uid:").and_then(|v| v.split_whitespace().nth(1)?.parse().ok())))
        .unwrap_or(u32::MAX)
}

/// Journal complet (pour l'enregistrer ailleurs).
pub fn log_path() -> PathBuf {
    PathBuf::from(LOG_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_keymaps() {
        assert_eq!(dnf_counter("[ 12/345] Installing foo"), Some((12, 345)));
        assert_eq!(dnf_counter("Downloading"), None);
        assert_eq!(console_keymap("fr", ""), "fr");
        assert_eq!(console_keymap("ch", "fr"), "fr_CH");
    }
}

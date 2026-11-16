//! Moteur d'installation : partitionnement Btrfs, installation réseau de Debian sid
//! (debootstrap puis apt dans le système cible, contrib / non-free au choix), paquet NOOS
//! depuis le dépôt APT de NOOS (GitHub Pages), configuration et GRUB (shim signé en UEFI).
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
    hardware,
    disk::{self, MIB, Role},
    model::{self, Choices},
};

pub const TARGET: &str = "/mnt/noos-target";
const BTRFS_TOP: &str = "/mnt/noos-btrfs";
pub const LOG_FILE: &str = "/tmp/noos-install.log";
/// Suite Debian installée.
const SUITE: &str = "sid";
const MIRROR: &str = "http://deb.debian.org/debian";
/// Fichiers de NOOS dans le système live (copiés tels quels dans le système installé).
const NOOS_PREFIX: &str = "/opt/noos";

pub const STEPS: &[&str] = &[
    "Vérifications",
    "Partitionnement du disque",
    "Formatage",
    "Sous-volumes Btrfs",
    "Téléchargement et installation de Debian",
    "Pilotes graphiques et codecs",
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
            // apt / dpkg dans le système cible : aucune question, configuration par défaut.
            .env("DEBIAN_FRONTEND", "noninteractive")
            .env("DEBCONF_NONINTERACTIVE_SEEN", "true")
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
            // apt et debootstrap réécrivent leurs barres de progression avec \r : on garde la dernière.
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

/// Paquets du système : liste fournie par l'ISO + paquets selon le démarrage (UEFI / BIOS).
fn packages(uefi: bool) -> Vec<String> {
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
            "linux-image-amd64", "systemd-sysv", "dbus-user-session", "locales", "console-setup", "sudo", "btrfs-progs",
            "network-manager", "plymouth", "plymouth-label", "greetd", "libgtk-4-1", "libvte-2.91-gtk4-0", "libseat1",
            "libgtk4-layer-shell0", "pipewire", "wireplumber", "xdg-desktop-portal", "xdg-desktop-portal-gtk", "polkitd",
            "pkexec", "papirus-icon-theme", "fonts-noto-core", "mesa-utils", "libgl1-mesa-dri",
        ] {
            p.push(x.into());
        }
    }
    if uefi {
        p.extend(["grub-efi-amd64", "grub-efi-amd64-signed", "shim-signed", "efibootmgr", "dosfstools"].map(String::from));
    } else {
        p.push("grub-pc".into());
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

/// Composants Debian du système installé.
fn components(c: &Choices) -> &'static str {
    if c.nonfree { "main contrib non-free non-free-firmware" } else { "main" }
}

/// apt-get dans le système cible, avancement lu sur `APT::Status-Fd` (« pmstatus:…:42.5:… »).
fn apt_install(ctx: &Ctx, packages: &[String], from: f64, span: f64) -> anyhow::Result<()> {
    let mut a: Vec<String> = [
        TARGET, "apt-get", "install", "-y", "--no-install-recommends", "-o", "APT::Status-Fd=1",
        "-o", "Dpkg::Options::=--force-confdef", "-o", "Dpkg::Options::=--force-confold",
    ]
    .map(String::from)
    .to_vec();
    a.extend(packages.iter().cloned());
    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
    let tx = ctx.tx.clone();
    ctx.run_with("chroot", &refs, None, &move |line: &str| {
        if let Some(p) = apt_progress(line) {
            let _ = tx.send_blocking(Event::Progress(from + span * p));
        }
    })
}

/// Paquets connus des dépôts du système cible (`apt-cache policy` : candidat ≠ « (none) »).
fn available(ctx: &Ctx, packages: &[String]) -> (Vec<String>, Vec<String>) {
    if ctx.dry || packages.is_empty() {
        return (packages.to_vec(), vec![]);
    }
    let out = Command::new("chroot")
        .arg(TARGET)
        .args(["apt-cache", "policy"])
        .args(packages)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut ok = Vec::new();
    let mut current: Option<String> = None;
    for line in out.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            current = Some(line.trim_end_matches(':').to_string());
        } else if let (Some(name), Some(c)) = (&current, line.trim().strip_prefix("Candidate:")) {
            if c.trim() != "(none)" {
                ok.push(name.clone());
            }
        }
    }
    let (found, missing): (Vec<String>, Vec<String>) = packages.iter().cloned().partition(|p| ok.contains(p));
    (found, missing)
}

/// Comme `apt_install`, sans les paquets absents des miroirs, puis paquet par paquet si
/// l'ensemble échoue encore : renvoie les paquets non installés.
fn apt_install_lenient(ctx: &Ctx, packages: &[String], from: f64, span: f64) -> Vec<String> {
    let (packages, mut absent) = available(ctx, packages);
    if !absent.is_empty() {
        ctx.line(format!("(attention) absents des dépôts : {}", absent.join(" ")));
    }
    if packages.is_empty() || apt_install(ctx, &packages, from, span).is_ok() {
        return absent;
    }
    ctx.line("(attention) installation groupée impossible : paquet par paquet…");
    let mut missing = Vec::new();
    for (i, p) in packages.iter().enumerate() {
        let f = from + span * i as f64 / packages.len() as f64;
        if apt_install(ctx, std::slice::from_ref(p), f, 0.0).is_err() {
            missing.push(p.clone());
        }
    }
    absent.extend(missing);
    absent
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
    ctx.run("curl", &["-sSfI", "--max-time", "20", "-o", "/dev/null", "https://deb.debian.org/debian/"])
        .context("pas d'accès à Internet : l'installation réseau télécharge Debian depuis les miroirs officiels")?;
    // Matériel graphique : pilotes et codecs installés à l'étape 5, arguments du noyau ci-dessous.
    let hw = hardware::detect();
    let hw_plan = hardware::plan(&hw, c.nonfree);
    for line in &hw_plan.summary {
        ctx.line(format!("Matériel : {line}"));
    }

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

    // ── 4. Debian (installation réseau) ──
    // debootstrap : système de base minimal ; le reste avec apt dans le système cible. Les
    // paquets sont téléchargés sur le disque cible (/var/cache/apt), pas dans la RAM du live.
    ctx.step(4);
    let comps = components(c).replace(' ', ",");
    {
        let tx = ctx.tx.clone();
        ctx.run_with(
            "debootstrap",
            &[
                "--arch=amd64",
                &format!("--components={comps}"),
                "--include=ca-certificates,locales,apt-utils,curl,gnupg",
                "--keyring=/usr/share/keyrings/debian-archive-keyring.gpg",
                SUITE,
                TARGET,
                MIRROR,
            ],
            None,
            &move |line: &str| {
                let p = if line.starts_with("I: Retrieving") || line.starts_with("I: Validating") {
                    0.05
                } else if line.starts_with("I: Extracting") {
                    0.12
                } else if line.starts_with("I: Unpacking") {
                    0.18
                } else if line.starts_with("I: Configuring") {
                    0.24
                } else {
                    return;
                };
                let _ = tx.send_blocking(Event::Progress(p));
            },
        )?;
    }
    for d in ["proc", "sys", "dev", "run"] {
        ctx.run("mkdir", &["-p", &format!("{TARGET}/{d}")])?;
    }
    ctx.run("mount", &["-t", "proc", "proc", &format!("{TARGET}/proc")])?;
    ctx.run("mount", &["--rbind", "/sys", &format!("{TARGET}/sys")])?;
    ctx.run("mount", &["--make-rslave", &format!("{TARGET}/sys")])?;
    ctx.run("mount", &["--rbind", "/dev", &format!("{TARGET}/dev")])?;
    ctx.run("mount", &["--make-rslave", &format!("{TARGET}/dev")])?;
    ctx.run("mount", &["-t", "tmpfs", "tmpfs", &format!("{TARGET}/run")])?;
    ctx.write("/etc/fstab", &fstab)?;
    // Sources APT au format deb822 (comme le fera Logiciels), sources.list de debootstrap retiré.
    ctx.write(
        "/etc/apt/sources.list.d/debian.sources",
        &format!(
            "# Dépôts de Debian sid — installateur NOOS\nTypes: deb\nURIs: {MIRROR}\nSuites: {SUITE}\nComponents: {}\nSigned-By: /usr/share/keyrings/debian-archive-keyring.gpg\n",
            components(c)
        ),
    )?;
    ctx.try_run("rm", &["-f", &format!("{TARGET}/etc/apt/sources.list")]);
    // Pas de services démarrés dans le chroot pendant l'installation.
    ctx.write("/usr/sbin/policy-rc.d", "#!/bin/sh\n# Installateur NOOS : aucun service démarré dans le chroot.\nexit 101\n")?;
    ctx.try_run("chmod", &["755", &format!("{TARGET}/usr/sbin/policy-rc.d")]);
    // Recommandations désactivées pendant l'installation (liste NOOS complète et maîtrisée).
    ctx.write("/etc/apt/apt.conf.d/90noos-install", "APT::Install-Recommends \"false\";\n")?;
    // Architecture 32 bits (i386) active d'emblée : Steam, Wine et beaucoup de jeux en ont
    // besoin, et Debian ne l'active pas par défaut.
    ctx.chroot(&["dpkg", "--add-architecture", "i386"])?;
    ctx.chroot(&["apt-get", "update"])?;
    ctx.progress(0.3);
    let pkgs = packages(uefi);
    ctx.line(format!("{} paquets demandés (et leurs dépendances)", pkgs.len()));
    let missing = apt_install_lenient(ctx, &pkgs, 0.3, 0.7);
    if !missing.is_empty() {
        ctx.line(format!("(attention) paquets indisponibles : {}", missing.join(" ")));
        if missing.iter().any(|m| m.starts_with("linux-image") || m.starts_with("grub") || m == "systemd-sysv") {
            bail!("paquets indispensables indisponibles : {}", missing.join(" "));
        }
    }
    ctx.try_run("systemd-machine-id-setup", &["--root", TARGET]);

    // ── 5. Pilotes et codecs ──
    ctx.step(5);
    ctx.line(format!("Pilotes et codecs : {}", hw_plan.packages.join(" ")));
    if hw_plan.dkms {
        ctx.line("Pilote NVIDIA : le module est compilé par DKMS pendant son installation (quelques minutes).");
    }
    let missing = apt_install_lenient(ctx, &hw_plan.packages, 0.0, 1.0);
    if !missing.is_empty() {
        ctx.line(format!("(attention) pilotes et codecs non installés : {}", missing.join(" ")));
    }
    ctx.progress(1.0);

    // ── 6. NOOS ──
    // Paquet « noos » du dépôt APT de NOOS (GitHub Pages) : dernière version, puis mises à jour
    // avec apt. Sans accès au dépôt, copie de la version fournie avec l'ISO.
    ctx.step(6);
    let keyring = "/usr/share/keyrings/noos-archive-keyring.asc";
    let from_repo = if Path::new(keyring).exists() || ctx.dry {
        ctx.run("install", &["-Dm644", keyring, &format!("{TARGET}{keyring}")])?;
        ctx.write("/etc/apt/sources.list.d/noos.sources", &noos_sources(keyring))?;
        ctx.progress(0.2);
        let res = ctx.chroot(&["apt-get", "update"]).and_then(|_| apt_install(ctx, &["noos".to_string()], 0.2, 0.8));
        match res {
            Ok(()) => true,
            Err(e) => {
                ctx.line(format!("(attention) paquet NOOS indisponible sur le dépôt APT ({e:#}) : version de l'ISO"));
                false
            }
        }
    } else {
        ctx.line("(attention) clé du dépôt NOOS absente de l'ISO : version de l'ISO");
        false
    };
    if !from_repo {
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
    // Langue : locale générée (+ anglais américain, utilisé par certains outils).
    let mut gen = format!("{}\n", model::locale_gen_line(&c.locale));
    if c.locale != "en_US.UTF-8" {
        gen.push_str("en_US.UTF-8 UTF-8\n");
    }
    ctx.write("/etc/locale.gen", &gen)?;
    ctx.try_run("chroot", &[TARGET, "locale-gen"]);
    ctx.write("/etc/default/locale", &format!("LANG={}\n", c.locale))?;
    ctx.write("/etc/locale.conf", &format!("LANG={}\n", c.locale))?;
    // Clavier : console-setup (/etc/default/keyboard), lu aussi par NOOS et noos-login.
    ctx.write(
        "/etc/default/keyboard",
        &format!(
            "# Créé par l'installateur NOOS (voir keyboard(5))\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"{}\"\nXKBVARIANT=\"{}\"\nXKBOPTIONS=\"\"\nBACKSPACE=\"guess\"\n",
            c.kb_layout, c.kb_variant
        ),
    )?;
    ctx.write(
        "/etc/vconsole.conf",
        &format!("KEYMAP={}\nXKBLAYOUT={}\nXKBVARIANT={}\n", console_keymap(&c.kb_layout, &c.kb_variant), c.kb_layout, c.kb_variant),
    )?;
    ctx.write(
        "/etc/X11/xorg.conf.d/00-keyboard.conf",
        &format!(
            "Section \"InputClass\"\n    Identifier \"system-keyboard\"\n    MatchIsKeyboard \"on\"\n    Option \"XkbLayout\" \"{}\"\n    Option \"XkbVariant\" \"{}\"\nEndSection\n",
            c.kb_layout, c.kb_variant
        ),
    )?;
    ctx.run("ln", &["-sf", &format!("/usr/share/zoneinfo/{}", c.timezone), &format!("{TARGET}/etc/localtime")])?;
    ctx.write("/etc/timezone", &format!("{}\n", c.timezone))?;
    ctx.write("/etc/hostname", &format!("{}\n", c.hostname))?;
    ctx.write("/etc/hosts", &format!("127.0.0.1\tlocalhost\n127.0.1.1\t{0}\n\n::1\tlocalhost ip6-localhost ip6-loopback\nff02::1\tip6-allnodes\nff02::2\tip6-allrouters\n", c.hostname))?;
    ctx.progress(0.2);
    // Compte administrateur (groupe sudo) ; root verrouillé.
    // fish (fastfetch à l'ouverture d'un terminal) s'il est installé, sinon bash ; il devient
    // aussi le shell des comptes créés plus tard.
    let shell = if ctx.dry || Path::new(&format!("{TARGET}/usr/bin/fish")).exists() { "/usr/bin/fish" } else { "/bin/bash" };
    if shell == "/usr/bin/fish" {
        ctx.try_run("chroot", &[TARGET, "useradd", "-D", "-s", "/usr/bin/fish"]);
    }
    ctx.chroot(&["useradd", "-m", "-G", "sudo", "-c", &c.full_name, "-s", shell, &c.username])?;
    // Groupes facultatifs (présents selon les paquets installés).
    for group in ["audio", "video", "render", "input", "netdev", "plugdev", "bluetooth", "lpadmin"] {
        ctx.try_run("chroot", &[TARGET, "usermod", "-aG", group, &c.username]);
    }
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
    // Écran de connexion noos-login (greetd). Le paquet Debian crée le compte « _greetd ».
    let greeter = greeter_user();
    ctx.line(format!("compte de l'écran de connexion : {greeter}"));
    ctx.write(
        "/etc/greetd/config.toml",
        &format!("[terminal]\nvt = 7\n\n[default_session]\ncommand = \"{}\"\nuser = \"{greeter}\"\n", greeter_command()),
    )?;
    ctx.run("mkdir", &["-p", &format!("{TARGET}/var/lib/noos-login")])?;
    ctx.chroot(&["chown", &format!("{greeter}:{greeter}"), "/var/lib/noos-login"])?;
    // Thème de l'écran de connexion : modifiable par les administrateurs (Paramètres).
    ctx.run("mkdir", &["-p", &format!("{TARGET}/etc/noos")])?;
    if !Path::new(&format!("{TARGET}/etc/noos/login.toml")).exists() {
        ctx.write("/etc/noos/login.toml", &noos_core::login::LoginConfig::default().to_toml()?)?;
    }
    ctx.try_run("chroot", &[TARGET, "chgrp", "-R", "sudo", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "chmod", "-R", "g+w", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "chmod", "g+s", "/etc/noos"]);
    ctx.try_run("chroot", &[TARGET, "ldconfig"]);
    ctx.progress(0.55);
    for unit in ["greetd.service", "NetworkManager.service", "bluetooth.service", "fstrim.timer", "systemd-oomd.service", "power-profiles-daemon.service"] {
        ctx.try_run("systemctl", &["--root", TARGET, "enable", unit]);
    }
    // Recherche régulière des mises à jour (timer de session de chaque utilisateur).
    ctx.try_run("systemctl", &["--root", TARGET, "--global", "enable", "noos-update-check.timer"]);
    ctx.run("systemctl", &["--root", TARGET, "set-default", "graphical.target"])?;
    ctx.try_run("chroot", &[TARGET, "plymouth-set-default-theme", "noos"]);
    // Flathub actif par défaut (fichier embarqué : pas de réseau).
    ctx.try_run(
        "chroot",
        &[TARGET, "flatpak", "remote-add", "--system", "--if-not-exists", "flathub", "/opt/noos/share/noos/flathub.flatpakrepo"],
    );
    ctx.try_run("chroot", &[TARGET, "flatpak", "remote-modify", "--system", "--no-filter", "--enable", "flathub"]);
    // Thème GRUB NOOS (grub.cfg est régénéré à l'étape GRUB).
    ctx.try_run("chroot", &[TARGET, "/usr/share/noos-grub/install.sh"]);
    // Instantanés Btrfs (snapper) : configuration de / et instantanés avant / après apt.
    ctx.line("Configuration des instantanés du système (Sauvegarder et Restaurer)…");
    ctx.try_run("chroot", &[TARGET, "/opt/noos/libexec/noos-save-helper", "setup"]);
    ctx.progress(0.8);
    // Fin des réglages propres à l'installation.
    ctx.try_run("rm", &["-f", &format!("{TARGET}/usr/sbin/policy-rc.d"), &format!("{TARGET}/etc/apt/apt.conf.d/90noos-install")]);
    ctx.progress(1.0);

    // ── 8. GRUB ──
    ctx.step(8);
    // Arguments propres au matériel (pilote NVIDIA : KMS, nouveau écarté). Debian ajoute
    // lui-même rootflags=subvol=@ (racine Btrfs détectée par grub-mkconfig).
    let extra = hw_plan.kernel_args.join(" ");
    ctx.write(
        "/etc/default/grub",
        &format!(
            "# Créé par l'installateur NOOS — voir aussi /etc/default/grub.d/\nGRUB_DEFAULT=saved\nGRUB_TIMEOUT=3\nGRUB_DISTRIBUTOR=\"NOOS\"\nGRUB_DISABLE_SUBMENU=y\nGRUB_TERMINAL_OUTPUT=\"gfxterm\"\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash loglevel=3\"\nGRUB_CMDLINE_LINUX=\"{extra}\"\nGRUB_DISABLE_RECOVERY=\"true\"\nGRUB_DISABLE_OS_PROBER=\"false\"\n"
        ),
    )?;
    ctx.try_run("chroot", &[TARGET, "/usr/share/noos-grub/install.sh"]);
    // Démarrage fluide : pilote graphique de ce PC chargé tôt, Plymouth sans simpledrm
    // (l'initramfs est régénéré juste après).
    ctx.try_run("chroot", &[TARGET, "/usr/libexec/noos-boot-setup", "--no-initramfs"]);
    ctx.line("Régénération de l'initramfs (thème Plymouth NOOS inclus)…");
    ctx.chroot(&["update-initramfs", "-u", "-k", "all"])?;
    ctx.progress(0.5);
    if uefi {
        // GRUB signé de Debian derrière shim (Secure Boot) : dossier EFI « debian » imposé par
        // la signature ; entrée de démarrage « NOOS » créée ensuite, plus la copie de secours
        // EFI/BOOT/BOOTX64.EFI (micrologiciels qui ignorent les entrées).
        ctx.chroot(&["grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--bootloader-id=debian", "--no-nvram"])?;
        ctx.try_run("chroot", &[TARGET, "grub-install", "--target=x86_64-efi", "--efi-directory=/boot/efi", "--removable", "--no-nvram"]);
        ctx.chroot(&["grub-mkconfig", "-o", "/boot/grub/grub.cfg"])?;
        let efi_num = plan.iter().find(|p| p.role == Role::Efi).map(|p| p.number).unwrap_or(1).to_string();
        let loader = if ctx.dry || Path::new(&format!("{TARGET}/boot/efi/EFI/debian/shimx64.efi")).exists() {
            "\\EFI\\debian\\shimx64.efi"
        } else {
            "\\EFI\\debian\\grubx64.efi"
        };
        ctx.run("efibootmgr", &["--create", "--disk", dev, "--part", &efi_num, "--label", "NOOS", "--loader", loader])?;
    } else {
        ctx.chroot(&["grub-install", "--target=i386-pc", dev])?;
        ctx.chroot(&["grub-mkconfig", "-o", "/boot/grub/grub.cfg"])?;
    }
    // Premier instantané, une fois le système complet (grub.cfg compris).
    ctx.try_run("chroot", &[TARGET, "/opt/noos/libexec/noos-save-helper", "create", "Installation de NOOS"]);
    ctx.progress(1.0);

    // ── 9. Finalisation ──
    ctx.step(9);
    ctx.line("Installation terminée : copie du journal et démontage.");
    ctx.try_run("chroot", &[TARGET, "apt-get", "clean"]);
    if !ctx.dry {
        let _ = std::fs::copy(LOG_FILE, format!("{TARGET}/var/log/noos-install.log"));
    }
    ctx.try_run("sync", &[]);
    ctx.run("umount", &["-R", TARGET])?;
    ctx.progress(1.0);
    Ok(())
}

/// Fichier de sources du dépôt APT de NOOS (identique à celui du paquet `noos`).
fn noos_sources(keyring: &str) -> String {
    format!(
        "# Dépôt APT de NOOS (GitHub Pages) : bureau NOOS et ses applications.\nTypes: deb\nURIs: https://chomiam.github.io/noos-de\nSuites: sid\nComponents: main\nSigned-By: {keyring}\n"
    )
}

/// Lanceur de l'écran de connexion : paquet Debian (/usr/bin) ou copie de l'ISO (/usr/local/bin).
fn greeter_command() -> &'static str {
    if Path::new(&format!("{TARGET}/usr/bin/noos-greeter")).exists() { "/usr/bin/noos-greeter" } else { "/usr/local/bin/noos-greeter" }
}

/// Compte système de l'écran de connexion dans le système cible (« _greetd » sur Debian).
fn greeter_user() -> String {
    let passwd = std::fs::read_to_string(format!("{TARGET}/etc/passwd")).unwrap_or_default();
    ["_greetd", "greetd", "greeter"]
        .into_iter()
        .find(|u| passwd.lines().any(|l| l.starts_with(&format!("{u}:"))))
        .map(String::from)
        .unwrap_or_else(|| "_greetd".into())
}

/// Ligne d'état d'apt (`-o APT::Status-Fd=1`) → avancement 0 – 1 : téléchargement
/// (« dlstatus:n:10.0:… ») sur les premiers 40 %, installation (« pmstatus:paquet:42.5:… »)
/// sur le reste.
pub fn apt_progress(line: &str) -> Option<f64> {
    let (download, rest) = match line.strip_prefix("dlstatus:") {
        Some(r) => (true, r),
        None => (false, line.strip_prefix("pmstatus:")?),
    };
    let mut f = rest.splitn(3, ':');
    let _ = f.next()?;
    let pct: f64 = f.next()?.trim().parse().ok()?;
    let x = (pct / 100.0).clamp(0.0, 1.0);
    Some(if download { 0.4 * x } else { 0.4 + 0.6 * x })
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
        assert!((apt_progress("pmstatus:libfoo1:50.0:Installation de libfoo1").unwrap() - 0.7).abs() < 1e-9);
        assert!((apt_progress("dlstatus:3:10.0:Téléchargement").unwrap() - 0.04).abs() < 1e-9);
        assert_eq!(apt_progress("Setting up foo"), None);
        assert_eq!(model::locale_gen_line("fr_FR.UTF-8"), "fr_FR.UTF-8 UTF-8");
        assert_eq!(console_keymap("fr", ""), "fr");
        assert_eq!(console_keymap("ch", "fr"), "fr_CH");
    }
}

//! noos-share-helper — script privilégié du partage de fichiers (pkexec, action polkit
//! `org.noos.share.helper`, réservée aux administrateurs).
//!
//!   init [--fresh]            configuration initiale, si /etc/noos/sharing.toml n'existe pas
//!                             (postinst, installateur) : partage du dossier personnel, SSH tel
//!                             qu'il était (--fresh : désactivé)
//!   apply                     configuration (TOML) sur l'entrée standard : smb.conf (bloc NOOS),
//!                             sshd_config.d/90-noos.conf, services, annonces mDNS, pare-feu
//!   samba-password            mot de passe de partage du compte appelant (entrée standard)
//!   samba-forget              retire le compte appelant de la base de Samba
//!   hostname <nom>            nom de la machine (hostnamectl, /etc/hosts, Samba, Avahi, wsdd2)
//!   webdav <port> https|http  WebDAV du compte appelant : règle du pare-feu, annonce mDNS
//!   webdav off
//!
//! Les avertissements sont écrits sur la sortie standard, préfixés par « ! ».

use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, bail};
use noos_firewall::proto::Request;
use noos_share::{
    conf,
    model::{SAMBA_USERS, SYSTEM_CONFIG, SystemConfig, validate_hostname},
};

fn main() {
    if let Err(e) = run() {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

fn warn(msg: &str) {
    println!("! {msg}");
}

fn run() -> anyhow::Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("noos-share-helper doit être lancé par pkexec");
    }
    // Environnement minimal et prévisible (pkexec le nettoie déjà).
    unsafe {
        std::env::set_var("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
        std::env::set_var("LC_ALL", "C.UTF-8");
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["apply"] => {
            let mut cfg: SystemConfig = toml::from_str(&stdin()?).context("configuration illisible")?;
            cfg.validate()?;
            apply(&cfg)
        }
        ["init"] => init(false),
        ["init", "--fresh"] => init(true),
        ["samba-password"] => samba_password(),
        ["samba-forget"] => samba_forget(),
        ["hostname", name] => hostname(name),
        ["webdav", "off"] => webdav(None, false),
        ["webdav", port, proto] => {
            let p: u16 = port.parse().context("port invalide")?;
            if p < 1024 {
                bail!("port invalide (1024 au moins)");
            }
            webdav(Some(p), *proto == "https")
        }
        _ => bail!("usage : noos-share-helper apply | samba-password | samba-forget | hostname <nom> | webdav <port> https|http | webdav off"),
    }
}

fn stdin() -> anyhow::Result<String> {
    let mut s = String::new();
    std::io::stdin().take(1 << 20).read_to_string(&mut s)?;
    Ok(s)
}

/// Compte qui a lancé pkexec (ou sudo).
fn caller() -> anyhow::Result<String> {
    let uid: u32 = std::env::var("PKEXEC_UID").or_else(|_| std::env::var("SUDO_UID")).context("compte appelant inconnu")?.parse()?;
    if uid == 0 {
        bail!("le compte root n'a pas de partage");
    }
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        bail!("compte {uid} introuvable");
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().to_string();
    noos_share::model::validate_user(&name)
}

/// Commande ; échec = erreur avec sa sortie.
fn cmd(argv: &[&str]) -> anyhow::Result<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).output().with_context(|| format!("{} introuvable", argv[0]))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err.lines().filter(|l| !l.trim().is_empty()).take(6).collect::<Vec<_>>().join("\n");
        bail!("{} : {}", argv.join(" "), if first.is_empty() { format!("code {}", out.status) } else { first });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Commande dont l'échec n'est qu'un avertissement.
fn soft(argv: &[&str]) {
    if let Err(e) = cmd(argv) {
        warn(&format!("{e:#}"));
    }
}

fn systemctl(args: &[&str]) {
    let mut a = vec!["systemctl"];
    a.extend_from_slice(args);
    soft(&a);
}

fn is_active(unit: &str) -> bool {
    Command::new("systemctl").args(["is-active", "--quiet", unit]).status().is_ok_and(|s| s.success())
}

/// Écrit un fichier de façon atomique (mode 0644).
fn write(path: &str, content: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(dir) = Path::new(path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = format!("{path}.noos-tmp");
    std::fs::write(&tmp, content)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, path).with_context(|| format!("écriture de {path}"))?;
    Ok(())
}

/// Nom de la machine : /etc/hostname d'abord (juste aussi dans le chroot de l'installateur).
fn hostname_now() -> String {
    ["/etc/hostname", "/proc/sys/kernel/hostname"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| "noos".into())
}

// ───────────────────────────── init ─────────────────────────────

/// Première configuration. SSH : un serveur déjà utilisé garde son état, son port et son
/// réglage de mot de passe ; un openssh-server qui vient d'arriver avec NOOS (clés d'hôte
/// créées il y a moins de 30 min) ou une nouvelle installation (`--fresh`) reste désactivé.
fn init(fresh: bool) -> anyhow::Result<()> {
    if Path::new(SYSTEM_CONFIG).exists() {
        return Ok(());
    }
    let mut cfg = SystemConfig::default();
    if Path::new("/usr/sbin/sshd").exists() {
        let enabled = Command::new("systemctl").args(["is-enabled", "--quiet", "ssh.service"]).status().is_ok_and(|s| s.success());
        let new_keys = std::fs::metadata("/etc/ssh/ssh_host_ed25519_key")
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_secs() < 30 * 60));
        cfg.ssh.enabled = enabled && !fresh && !new_keys;
        if let Ok(out) = cmd(&["sshd", "-T"]) {
            let (port, pw) = parse_sshd_t(&out);
            cfg.ssh.port = port.unwrap_or(22);
            cfg.ssh.password_auth = pw.unwrap_or(true);
        }
    }
    apply(&cfg)
}

/// `sshd -T` : premier port et « passwordauthentication ».
pub fn parse_sshd_t(out: &str) -> (Option<u16>, Option<bool>) {
    let mut port = None;
    let mut pw = None;
    // Clés en minuscules jusqu'à OpenSSH 9, « Port », « PasswordAuthentication » ensuite.
    for l in out.lines() {
        let Some((k, v)) = l.split_once(' ') else { continue };
        match k.to_ascii_lowercase().as_str() {
            "port" if port.is_none() => port = v.trim().parse().ok(),
            "passwordauthentication" => pw = Some(v.trim().eq_ignore_ascii_case("yes")),
            _ => {}
        }
    }
    (port, pw)
}

// ───────────────────────────── apply ─────────────────────────────

fn apply(cfg: &SystemConfig) -> anyhow::Result<()> {
    write(SYSTEM_CONFIG, &format!("# Partage de fichiers (Paramètres › Réseau et Internet), écrit par NOOS.\n{}", cfg.to_toml()?))?;
    apply_samba(cfg)?;
    apply_ssh(cfg)?;
    let avahi = Path::new("/usr/sbin/avahi-daemon").exists();
    if (cfg.samba.enabled || cfg.ssh.enabled) && avahi {
        systemctl(&["enable", "--now", "avahi-daemon.service"]);
    }
    sync_firewall(&conf::system_rules(&cfg.samba, &cfg.ssh), &[conf::TAG_SAMBA, conf::TAG_DISCOVERY, conf::TAG_SSH]);
    Ok(())
}

fn apply_samba(cfg: &SystemConfig) -> anyhow::Result<()> {
    let installed = Path::new("/usr/sbin/smbd").exists();
    if !installed {
        if cfg.samba.enabled {
            warn("Samba n'est pas installé (paquet samba) : le partage SMB reste inactif");
        }
        return Ok(());
    }
    let old = std::fs::read_to_string(conf::SMB_CONF).unwrap_or_default();
    let new = conf::merge_block(&old, &conf::samba_block(&cfg.samba, &hostname_now()));
    if new != old {
        // Vérification par testparm avant de remplacer le fichier.
        let tmp = format!("{}.noos-check", conf::SMB_CONF);
        std::fs::write(&tmp, &new)?;
        let check = Command::new("testparm").args(["-s", "--suppress-prompt", &tmp]).stdout(Stdio::null()).stderr(Stdio::piped()).output();
        let _ = std::fs::remove_file(&tmp);
        if let Ok(out) = check {
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                bail!("configuration Samba refusée par testparm :\n{}", err.lines().filter(|l| l.contains("rror") || l.contains("nknown")).take(5).collect::<Vec<_>>().join("\n"));
            }
        }
        let backup = format!("{}.avant-noos", conf::SMB_CONF);
        if !Path::new(&backup).exists() && !old.is_empty() {
            std::fs::write(&backup, &old)?;
        }
        write(conf::SMB_CONF, &new)?;
    }
    if cfg.samba.enabled {
        systemctl(&["enable", "smbd.service"]);
        if is_active("smbd.service") {
            systemctl(&["reload", "smbd.service"]);
        } else {
            systemctl(&["start", "smbd.service"]);
        }
        if Path::new("/usr/sbin/wsdd2").exists() {
            if cfg.samba.discoverable {
                systemctl(&["enable", "--now", "wsdd2.service"]);
            } else {
                systemctl(&["disable", "--now", "wsdd2.service"]);
            }
        } else if cfg.samba.discoverable {
            warn("wsdd2 n'est pas installé : les PC Windows ne verront pas ce PC dans « Réseau » (accès par \\\\nom toujours possible)");
        }
    } else {
        if Path::new("/usr/sbin/wsdd2").exists() {
            systemctl(&["disable", "--now", "wsdd2.service"]);
        }
        systemctl(&["disable", "--now", "smbd.service"]);
    }
    Ok(())
}

fn apply_ssh(cfg: &SystemConfig) -> anyhow::Result<()> {
    if !Path::new("/usr/sbin/sshd").exists() {
        if cfg.ssh.enabled {
            warn("le serveur SSH n'est pas installé (paquet openssh-server)");
        }
        let _ = std::fs::remove_file(conf::AVAHI_SSH);
        return Ok(());
    }
    let old = std::fs::read_to_string(conf::SSHD_DROPIN).ok();
    write(conf::SSHD_DROPIN, &conf::sshd_dropin(&cfg.ssh))?;
    // `sshd -t` exige /run/sshd, que systemd ne crée que service lancé (RuntimeDirectory).
    if !Path::new("/run/sshd").exists() {
        use std::os::unix::fs::DirBuilderExt;
        let _ = std::fs::DirBuilder::new().mode(0o755).create("/run/sshd");
    }
    if let Err(e) = cmd(&["sshd", "-t"]) {
        match old {
            Some(o) => write(conf::SSHD_DROPIN, &o)?,
            None => std::fs::remove_file(conf::SSHD_DROPIN)?,
        }
        bail!("configuration SSH refusée : {e:#}");
    }
    if cfg.ssh.enabled {
        systemctl(&["enable", "ssh.service"]);
        systemctl(&["reload-or-restart", "ssh.service"]);
        write(conf::AVAHI_SSH, &conf::avahi_ssh(cfg.ssh.port))?;
    } else {
        systemctl(&["disable", "--now", "ssh.service"]);
        if Path::new("/usr/lib/systemd/system/ssh.socket").exists() {
            systemctl(&["disable", "--now", "ssh.socket"]);
        }
        let _ = std::fs::remove_file(conf::AVAHI_SSH);
    }
    Ok(())
}

/// Règles de port du pare-feu NOOS portant ces étiquettes : ajoutées / retirées pour
/// correspondre à `wanted` (en root : pas de demande polkit au démon). Démon arrêté
/// (installation, chroot) : fichier des règles modifié directement, lu à son démarrage.
fn sync_firewall(wanted: &[noos_firewall::model::PortRule], tags: &[&str]) {
    let existing = match noos_firewall::proto::rules() {
        Ok(r) => r.ports,
        Err(_) if !noos_firewall::proto::socket_path().exists() => {
            if let Err(e) = sync_firewall_offline(wanted, tags) {
                warn(&format!("pare-feu : {e:#}"));
            }
            return;
        }
        Err(e) => {
            if !wanted.is_empty() {
                warn(&format!("pare-feu NOOS injoignable ({e:#}) : règles d'accès au réseau local non créées"));
            }
            return;
        }
    };
    let (remove, add) = conf::rules_diff(&existing, wanted, tags);
    for id in remove {
        if let Err(e) = noos_firewall::proto::call(&Request::DeletePortRule { id }) {
            warn(&format!("pare-feu : {e:#}"));
        }
    }
    for r in add {
        if let Err(e) = noos_firewall::proto::call(&Request::SavePortRule(r)) {
            warn(&format!("pare-feu : {e:#}"));
        }
    }
}

fn sync_firewall_offline(wanted: &[noos_firewall::model::PortRule], tags: &[&str]) -> anyhow::Result<()> {
    let dir = noos_firewall::store::dir();
    let mut rules = noos_firewall::store::load_rules(&dir);
    let (remove, add) = conf::rules_diff(&rules.ports, wanted, tags);
    if remove.is_empty() && add.is_empty() {
        return Ok(());
    }
    rules.ports.retain(|r| !remove.contains(&r.id));
    for mut r in add {
        r.id = rules.alloc_id();
        rules.ports.push(r);
    }
    noos_firewall::store::save_rules(&dir, &rules)
}

// ───────────────────────────── Samba : mots de passe ─────────────────────────────

fn samba_users() -> Vec<String> {
    std::fs::read_to_string(SAMBA_USERS).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

fn save_samba_users(users: &[String]) -> anyhow::Result<()> {
    write(SAMBA_USERS, &users.iter().map(|u| format!("{u}\n")).collect::<String>())
}

fn samba_password() -> anyhow::Result<()> {
    let user = caller()?;
    let input = stdin()?;
    let pw = input.lines().next().unwrap_or("");
    if pw.is_empty() {
        bail!("mot de passe vide");
    }
    if !Path::new("/usr/bin/smbpasswd").exists() {
        bail!("Samba n'est pas installé (smbpasswd introuvable)");
    }
    // -s : mot de passe lu deux fois sur l'entrée standard, jamais sur la ligne de commande.
    let mut child = Command::new("smbpasswd").args(["-s", "-a", &user]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(format!("{pw}\n{pw}\n").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("smbpasswd : {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    soft(&["smbpasswd", "-e", &user]);
    let mut users = samba_users();
    if !users.contains(&user) {
        users.push(user);
        save_samba_users(&users)?;
    }
    Ok(())
}

fn samba_forget() -> anyhow::Result<()> {
    let user = caller()?;
    if Path::new("/usr/bin/smbpasswd").exists() {
        soft(&["smbpasswd", "-x", &user]);
    }
    let users: Vec<String> = samba_users().into_iter().filter(|u| *u != user).collect();
    save_samba_users(&users)
}

// ───────────────────────────── Nom de la machine ─────────────────────────────

/// Ligne `127.0.1.1` de /etc/hosts (Debian) : le nouveau nom, sinon sudo attend le DNS.
pub fn hosts_with(hosts: &str, name: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = hosts
        .lines()
        .map(|l| {
            if l.split_whitespace().next() == Some("127.0.1.1") {
                found = true;
                format!("127.0.1.1\t{name}")
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        let pos = out.iter().position(|l| l.starts_with("127.0.0.1")).map(|p| p + 1).unwrap_or(0);
        out.insert(pos, format!("127.0.1.1\t{name}"));
    }
    out.join("\n") + "\n"
}

fn hostname(name: &str) -> anyhow::Result<()> {
    let name = validate_hostname(name)?;
    cmd(&["hostnamectl", "set-hostname", &name])?;
    let hosts = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    write("/etc/hosts", &hosts_with(&hosts, &name))?;
    // Nom NetBIOS (Samba), annonces mDNS (Avahi) et WSD (wsdd2).
    let cfg = SystemConfig::load_from(Path::new(SYSTEM_CONFIG));
    if Path::new(conf::SMB_CONF).exists() && Path::new("/usr/sbin/smbd").exists() {
        let old = std::fs::read_to_string(conf::SMB_CONF).unwrap_or_default();
        write(conf::SMB_CONF, &conf::merge_block(&old, &conf::samba_block(&cfg.samba, &name)))?;
        if is_active("smbd.service") {
            systemctl(&["reload", "smbd.service"]);
        }
    }
    for unit in ["avahi-daemon.service", "wsdd2.service"] {
        if is_active(unit) {
            systemctl(&["try-restart", unit]);
        }
    }
    Ok(())
}

// ───────────────────────────── WebDAV ─────────────────────────────

fn webdav(port: Option<u16>, https: bool) -> anyhow::Result<()> {
    let user = caller()?;
    let file = conf::avahi_webdav_path(&user);
    match port {
        Some(p) if Path::new("/etc/avahi").exists() => write(&file, &conf::avahi_webdav(&user, p, https))?,
        _ => {
            let _ = std::fs::remove_file(&file);
        }
    }
    let tag = conf::tag_webdav(&user);
    sync_firewall(&conf::webdav_rules(&user, port), &[tag.as_str()]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sshd_t() {
        assert_eq!(parse_sshd_t("port 2222\nport 22\naddressfamily any\npasswordauthentication no\n"), (Some(2222), Some(false)));
        assert_eq!(parse_sshd_t("Port 2200\nPermitRootLogin no\nPasswordAuthentication yes\n"), (Some(2200), Some(true)));
        assert_eq!(parse_sshd_t(""), (None, None));
    }

    #[test]
    fn hosts() {
        let debian = "127.0.0.1\tlocalhost\n127.0.1.1\tancien\n\n::1     localhost ip6-localhost\n";
        assert_eq!(hosts_with(debian, "salon"), "127.0.0.1\tlocalhost\n127.0.1.1\tsalon\n\n::1     localhost ip6-localhost\n");
        let sans = "127.0.0.1 localhost\n::1 localhost\n";
        assert_eq!(hosts_with(sans, "salon"), "127.0.0.1 localhost\n127.0.1.1\tsalon\n::1 localhost\n");
    }
}

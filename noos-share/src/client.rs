//! Appels du script privilégié `noos-share-helper` (pkexec ; polkit garde l'autorisation
//! quelques minutes). Les secrets passent par l'entrée standard, jamais par la ligne de
//! commande (visible de tous dans /proc). Fonctions bloquantes : hors du fil de l'interface.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use anyhow::bail;

use crate::model::SystemConfig;

pub fn helper_path() -> String {
    std::env::var("NOOS_SHARE_HELPER").unwrap_or_else(|_| "/opt/noos/libexec/noos-share-helper".into())
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// Lance le script avec `args` et `stdin` ; rend les avertissements (lignes « ! » de sa sortie).
fn call(args: &[&str], stdin: &str) -> anyhow::Result<Vec<String>> {
    if crate::demo() {
        std::thread::sleep(std::time::Duration::from_millis(500));
        return Ok(vec![]);
    }
    let mut argv: Vec<String> = match (is_root(), std::env::var("NOOS_PKG_ELEVATE").as_deref()) {
        (true, _) | (_, Ok("none")) => vec![],
        (_, Ok("sudo")) => vec!["sudo".into(), "-n".into()],
        _ => vec!["pkexec".into()],
    };
    argv.push(helper_path());
    argv.extend(args.iter().map(|s| s.to_string()));
    let mut child = Command::new(&argv[0]).args(&argv[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    child.stdin.take().unwrap().write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    match out.status.code() {
        Some(0) => Ok(stdout.lines().filter_map(|l| l.strip_prefix("! ")).map(str::to_string).collect()),
        Some(126) | Some(127) if argv[0] == "pkexec" && stderr.trim().is_empty() => bail!("autorisation refusée ou annulée"),
        _ => {
            let msg = stderr.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n");
            bail!("{}", if msg.is_empty() { "échec du script privilégié".to_string() } else { msg })
        }
    }
}

/// Écrit la configuration système (Samba, SSH) et applique tout : fichiers, services, pare-feu.
pub fn apply(cfg: &SystemConfig) -> anyhow::Result<Vec<String>> {
    let mut c = cfg.clone();
    c.validate()?;
    call(&["apply"], &c.to_toml()?)
}

/// Mot de passe de partage Samba du compte appelant.
pub fn set_samba_password(password: &str) -> anyhow::Result<Vec<String>> {
    if password.is_empty() || password.contains('\n') {
        bail!("mot de passe invalide");
    }
    call(&["samba-password"], &format!("{password}\n"))
}

pub fn remove_samba_password() -> anyhow::Result<Vec<String>> {
    call(&["samba-forget"], "")
}

/// Nom de la machine (hostnamectl, /etc/hosts, Samba).
pub fn set_hostname(name: &str) -> anyhow::Result<Vec<String>> {
    let n = crate::model::validate_hostname(name)?;
    call(&["hostname", &n], "")
}

/// WebDAV de l'appelant : règle du pare-feu et annonce mDNS (`port` = None : retirées).
pub fn webdav_system(port: Option<u16>, https: bool) -> anyhow::Result<Vec<String>> {
    match port {
        Some(p) => call(&["webdav", &p.to_string(), if https { "https" } else { "http" }], ""),
        None => call(&["webdav", "off"], ""),
    }
}

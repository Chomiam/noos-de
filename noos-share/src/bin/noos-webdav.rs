//! noos-webdav — serveur WebDAV de l'utilisateur (service systemd utilisateur `noos-webdav`).
//! Lit `~/.config/noos/webdav/webdav.toml`, crée au besoin le certificat auto-signé, écrit la
//! configuration rclone dans `$XDG_RUNTIME_DIR` puis remplace ce processus par
//! `rclone serve webdav`.
//!
//!   noos-webdav            lance le serveur
//!   noos-webdav --cert     crée seulement le certificat (s'il manque)
//!   noos-webdav --dry-run  affiche la configuration rclone et la commande
//!   noos-webdav --password définit le mot de passe (lu sur l'entrée standard)
//!   noos-webdav --fingerprint  empreintes du certificat (SHA-256, SHA-1)

use std::{os::unix::process::CommandExt, path::PathBuf, process::Command};

use anyhow::{Context, bail};
use noos_share::webdav::{self, WebDav};

fn main() {
    if let Err(e) = run() {
        eprintln!("noos-webdav : {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let mut cfg = WebDav::load();
    cfg.validate()?;
    if arg == "--password" {
        let mut pw = String::new();
        std::io::stdin().read_line(&mut pw)?;
        return cfg.set_password(pw.trim_end_matches(['\n', '\r']));
    }
    if cfg.https || arg == "--cert" {
        ensure_cert()?;
    }
    if arg == "--cert" {
        return Ok(());
    }
    if arg == "--fingerprint" {
        let (a, b) = webdav::cert_fingerprints().context("pas de certificat")?;
        println!("SHA-256 {a}\nSHA-1   {b}");
        return Ok(());
    }
    if cfg.folders.is_empty() {
        bail!("aucun dossier à partager");
    }
    if !cfg.password_set() {
        bail!("aucun mot de passe défini (Paramètres › Réseau et Internet › WebDAV)");
    }
    let (conf, remote) = webdav::rclone_config(&cfg.folders);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| webdav::config_dir());
    let conf_path = runtime.join("noos-webdav-rclone.conf");
    let args = webdav::rclone_args(&cfg, &conf_path, &remote);
    if arg == "--dry-run" {
        println!("{conf}\nrclone {}", args.join(" "));
        return Ok(());
    }
    webdav::write_private(&conf_path, &conf)?;
    let err = Command::new("rclone").args(&args).exec();
    bail!("rclone : {err} (paquet rclone installé ?)")
}

/// Certificat auto-signé (10 ans, courbe P-256) au nom de la machine.
fn ensure_cert() -> anyhow::Result<()> {
    if webdav::cert_path().exists() && webdav::key_path().exists() {
        return Ok(());
    }
    let host = noos_share::status::hostname();
    let mut san = format!("DNS:{host}.local,DNS:{host},DNS:localhost,IP:127.0.0.1");
    for a in noos_share::status::local_addresses() {
        san.push_str(&format!(",IP:{}", a.ip));
    }
    std::fs::create_dir_all(webdav::config_dir())?;
    let key = webdav::key_path();
    let st = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-days", "3650"])
        .args(["-subj", &format!("/CN={host}.local/O=NOOS"), "-addext", &format!("subjectAltName={san}")])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(webdav::cert_path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("openssl introuvable")?;
    if !st.success() {
        bail!("création du certificat impossible (openssl)");
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

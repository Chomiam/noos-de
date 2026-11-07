//! noos-pkg — interface en ligne de commande de NOOS Logiciels.
//!
//! ```text
//! noos-pkg info                              distribution et backends détectés
//! noos-pkg search <texte> [-b backend]       recherche dans tous les backends
//! noos-pkg installed [-b backend]            paquets installés explicitement
//! noos-pkg updates [-b backend]              mises à jour disponibles
//! noos-pkg refresh [-b backend]              rafraîchit les métadonnées
//! noos-pkg install -b backend <id>…          installe
//! noos-pkg remove -b backend <id>…           supprime
//! noos-pkg upgrade [-b backend]              applique toutes les mises à jour
//! noos-pkg repos [-b backend]                dépôts configurés
//! noos-pkg presets                           dépôts additionnels proposés
//! noos-pkg repo-add -b backend <préréglage> [argument]
//! noos-pkg repo-enable|repo-disable|repo-remove -b backend <id>
//!
//! Options : --dry-run (affiche le plan sans l'exécuter), --json
//! Élévation : pkexec par défaut ; NOOS_PKG_ELEVATE=sudo|none
//! ```

use noos_pkg::{Backend, BackendKind, Package, SystemInfo};

struct Args {
    cmd: String,
    rest: Vec<String>,
    backend: Option<BackendKind>,
    dry_run: bool,
    json: bool,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut args = Args { cmd: String::new(), rest: vec![], backend: None, dry_run: false, json: false };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-b" | "--backend" => {
                let b = it.next().unwrap_or_default();
                args.backend = Some(BackendKind::parse(&b).ok_or_else(|| anyhow::anyhow!("backend inconnu : {b}"))?);
            }
            "--dry-run" | "-n" => args.dry_run = true,
            "--json" => args.json = true,
            _ if args.cmd.is_empty() => args.cmd = a,
            _ => args.rest.push(a),
        }
    }
    Ok(args)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("erreur : {e:#}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let args = parse_args()?;
    let all = noos_pkg::detect();
    let selected: Vec<&dyn Backend> =
        all.iter().map(|b| b.as_ref()).filter(|b| args.backend.is_none_or(|k| b.kind() == k)).collect();
    let one = || -> anyhow::Result<&dyn Backend> {
        let k = args.backend.ok_or_else(|| anyhow::anyhow!("précisez le backend avec -b"))?;
        selected.first().copied().ok_or_else(|| anyhow::anyhow!("backend {k} indisponible sur ce système"))
    };
    let log = |l: &str| println!("  {l}");
    let execute = |plan: noos_pkg::Plan| -> anyhow::Result<()> {
        if args.dry_run {
            for line in plan.describe() {
                println!("{line}");
            }
            return Ok(());
        }
        plan.run(&log)
    };

    match args.cmd.as_str() {
        "info" | "" => {
            let sys = SystemInfo::detect();
            println!("{} (id={}, version={})", sys.name, sys.id, sys.version_id);
            for b in &all {
                println!("  • {} ({})", b.kind().label(), b.kind().id());
            }
        }
        "search" => {
            let q = args.rest.join(" ");
            let mut results: Vec<Package> = Vec::new();
            for b in &selected {
                match b.search(&q) {
                    Ok(r) => results.extend(r),
                    Err(e) => eprintln!("[{}] {e:#}", b.kind()),
                }
            }
            print_packages(&results, args.json);
        }
        "installed" => {
            let mut results = Vec::new();
            for b in &selected {
                match b.installed() {
                    Ok(r) => results.extend(r),
                    Err(e) => eprintln!("[{}] {e:#}", b.kind()),
                }
            }
            print_packages(&results, args.json);
        }
        "updates" => {
            let mut all_updates = Vec::new();
            for b in &selected {
                match b.updates() {
                    Ok(r) => all_updates.extend(r),
                    Err(e) => eprintln!("[{}] {e:#}", b.kind()),
                }
            }
            if args.json {
                println!("{}", serde_json::to_string_pretty(&all_updates)?);
            } else {
                for u in &all_updates {
                    println!(
                        "{:<10} {:<40} {} → {}",
                        u.package.backend.map(|b| b.id()).unwrap_or(""),
                        u.package.name,
                        u.old_version,
                        u.new_version
                    );
                }
                println!("{} mise(s) à jour", all_updates.len());
            }
        }
        "refresh" => {
            for b in &selected {
                execute(b.refresh())?;
            }
        }
        "upgrade" => {
            for b in &selected {
                println!("== {}", b.kind().label());
                execute(b.upgrade_all())?;
            }
        }
        "install" | "remove" => {
            let b = one()?;
            let pkgs: Vec<Package> = args
                .rest
                .iter()
                .map(|id| Package { backend: Some(b.kind()), id: id.clone(), name: id.clone(), ..Default::default() })
                .collect();
            if pkgs.is_empty() {
                anyhow::bail!("aucun paquet indiqué");
            }
            execute(if args.cmd == "install" { b.install(&pkgs) } else { b.remove(&pkgs) })?;
        }
        "repos" => {
            for b in &selected {
                let repos = b.repos()?;
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&repos)?);
                    continue;
                }
                for r in repos {
                    println!(
                        "{:<10} [{}] {:<42} {}{}",
                        b.kind().id(),
                        if r.enabled { "x" } else { " " },
                        r.id,
                        r.name,
                        if r.builtin { "  (distribution)" } else { "" }
                    );
                }
            }
        }
        "presets" => {
            for b in &selected {
                for p in b.presets() {
                    println!(
                        "{:<10} {:<18} {}{}",
                        b.kind().id(),
                        p.id,
                        p.name,
                        p.argument.map(|a| format!("  <{a}>")).unwrap_or_default()
                    );
                }
            }
        }
        "repo-add" => {
            let b = one()?;
            let preset = args.rest.first().ok_or_else(|| anyhow::anyhow!("préréglage manquant (voir « presets »)"))?;
            let arg = (args.rest.len() > 1).then(|| args.rest[1..].join(" "));
            execute(b.add_preset(preset, arg.as_deref())?)?;
        }
        "repo-enable" | "repo-disable" | "repo-remove" => {
            let b = one()?;
            let id = args.rest.first().ok_or_else(|| anyhow::anyhow!("identifiant de dépôt manquant"))?;
            let repo = b.repos()?.into_iter().find(|r| &r.id == id).ok_or_else(|| anyhow::anyhow!("dépôt {id} introuvable"))?;
            let plan = match args.cmd.as_str() {
                "repo-enable" => b.set_repo_enabled(&repo, true)?,
                "repo-disable" => b.set_repo_enabled(&repo, false)?,
                _ => b.remove_repo(&repo)?,
            };
            execute(plan)?;
        }
        "fw-status" => {
            let st = noos_pkg::firewall::status();
            println!("{}", serde_json::to_string_pretty(&st)?);
        }
        "fw-rules" => {
            let b = noos_pkg::firewall::detect();
            let (ports, services, raw) = noos_pkg::firewall::read_rules_privileged(b)?;
            println!("ports: {:?}\nservices: {:?}\n---\n{raw}", ports.iter().map(|p| p.label()).collect::<Vec<_>>(), services);
        }
        "fw-enable" | "fw-disable" => execute(noos_pkg::firewall::set_active(noos_pkg::firewall::detect(), args.cmd == "fw-enable")?)?,
        "fw-open" | "fw-close" => {
            let port = args.rest.first().ok_or_else(|| anyhow::anyhow!("port manquant"))?;
            let proto = args.rest.get(1).map(String::as_str).unwrap_or("tcp");
            let b = noos_pkg::firewall::detect();
            let zone = noos_pkg::firewall::status().zone;
            let plan = if args.cmd == "fw-open" {
                let p = match proto {
                    "udp" => noos_pkg::firewall::Proto::Udp,
                    "both" => noos_pkg::firewall::Proto::Both,
                    _ => noos_pkg::firewall::Proto::Tcp,
                };
                noos_pkg::firewall::open_port(b, port, p, zone.as_deref())?
            } else {
                let rule = noos_pkg::firewall::PortRule { port: port.clone(), proto: if proto == "both" { String::new() } else { proto.to_string() } };
                noos_pkg::firewall::close_port(b, &rule, zone.as_deref())?
            };
            execute(plan)?;
        }
        "fw-install" => {
            let (name, plan) = noos_pkg::firewall::install_plan(&SystemInfo::detect()).ok_or_else(|| anyhow::anyhow!("distribution non reconnue"))?;
            println!("Pare-feu recommandé : {name}");
            execute(plan)?;
        }
        other => anyhow::bail!("commande inconnue : {other} (voir l'en-tête de noos-pkg.rs)"),
    }
    Ok(())
}

fn print_packages(pkgs: &[Package], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(pkgs).unwrap());
        return;
    }
    for p in pkgs {
        println!(
            "{:<10} {} {:<38} {:<22} {:<14} {}",
            p.backend.map(|b| b.id()).unwrap_or(""),
            if p.installed { "✓" } else { " " },
            truncate(&p.id, 38),
            truncate(&p.version, 22),
            truncate(&p.origin, 14),
            truncate(&p.summary, 60)
        );
    }
    println!("{} résultat(s)", pkgs.len());
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}

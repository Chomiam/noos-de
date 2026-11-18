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
//! noos-pkg appimage [fichier.AppImage]       lit une AppImage (sans l'exécuter) ; sans
//!                                            fichier : AppImage installées
//! noos-pkg appimage-install <fichier>        déplace dans ~/Applications + menu Démarrer
//! noos-pkg deb <fichier.deb>                 fiche du paquet et simulation (sans root)
//! noos-pkg details -b backend <id>           fiche détaillée (Logiciels) et simulation
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
        "appimage" => {
            use noos_pkg::appimage;
            if let Some(f) = args.rest.first() {
                let i = appimage::inspect(std::path::Path::new(f))?;
                println!("{} (type {}){}", i.name, i.kind, i.version.as_deref().map(|v| format!(" — version {v}")).unwrap_or_default());
                if let Some(c) = &i.comment {
                    println!("  {c}");
                }
                println!("  icône : {}", i.icon.as_ref().map_or("aucune".into(), |p| p.display().to_string()));
                println!("  lanceur embarqué : {}", if i.entry.is_some() { "oui" } else { "non" });
                println!("  installée : {}", i.installed.as_ref().map_or("non".into(), |a| a.launcher.display().to_string()));
            } else {
                for a in appimage::installed() {
                    println!("{} — {} ({})", a.name, a.path.display(), a.launcher.display());
                }
                for p in appimage::unintegrated() {
                    println!("(hors menu) {}", p.display());
                }
            }
        }
        "deb" => {
            let f = args.rest.first().ok_or_else(|| anyhow::anyhow!("précisez le fichier .deb"))?;
            let i = noos_pkg::deb::inspect(std::path::Path::new(f))?;
            println!("{} {} ({}) — {}", i.package, i.version, i.architecture, i.summary);
            if let Some(v) = &i.installed_version {
                println!("  installé : {v}");
            }
            match noos_pkg::deb::simulate(&i) {
                Ok(s) => println!("  dépendances : {:?} ; mises à jour : {:?} ; retraits : {:?}", s.extra, s.upgraded, s.removed),
                Err(e) => println!("  impossible à installer :\n{e:#}"),
            }
            for line in noos_pkg::deb::install_plan(&i).describe() {
                println!("  {line}");
            }
        }
        "appimage-install" => {
            let f = args.rest.first().ok_or_else(|| anyhow::anyhow!("précisez le fichier .AppImage"))?;
            let i = noos_pkg::appimage::inspect(std::path::Path::new(f))?;
            let out = noos_pkg::appimage::install(&i)?;
            println!("{} → {}", out.app.name, out.app.path.display());
            println!("  lanceur : {}", out.app.launcher.display());
            if let Some(p) = out.previous {
                println!("  ancienne version hors du menu : {}", p.display());
            }
        }
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
        "details" => {
            let b = one()?;
            let id = args.rest.first().ok_or_else(|| anyhow::anyhow!("précisez le paquet"))?;
            // La recherche donne le remote, l'installation et l'état du paquet.
            let pkg = b
                .search(id)
                .unwrap_or_default()
                .into_iter()
                .find(|p| &p.id == id)
                .unwrap_or_else(|| Package { backend: Some(b.kind()), id: id.clone(), name: id.clone(), ..Default::default() });
            let d = b.details(&pkg)?;
            if args.json {
                println!("{d:#?}");
            } else {
                println!("{} — {}\n\n{}\n", d.name, d.summary, d.description);
                println!("{d:#?}");
            }
            match b.simulate(&pkg, !pkg.installed) {
                Ok(e) => println!("simulation ({}) : {e:?}", if pkg.installed { "suppression" } else { "installation" }),
                Err(e) => println!("simulation impossible : {e:#}"),
            }
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
        "check" => {
            // Service noos-update-check : recherche sans droits root, état enregistré pour
            // l'icône de la barre des tâches. `--force` : métadonnées retéléchargées.
            let force = std::env::args().any(|a| a == "--force");
            if std::env::args().any(|a| a == "--auto") && !noos_core::NoosConfig::load().updates.auto_check {
                return Ok(());
            }
            let res: Vec<_> = noos_pkg::check_updates(&all, force).into_iter().map(|(k, r)| (k, r.map_err(|e| format!("{e:#}")))).collect();
            let st = noos_pkg::record_updates(&res);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&st)?);
            } else {
                for (name, n) in &st.by_backend {
                    println!("{name} : {n} mise(s) à jour");
                }
                for e in &st.errors {
                    eprintln!("{e}");
                }
                println!("{} mise(s) à jour au total", st.total);
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

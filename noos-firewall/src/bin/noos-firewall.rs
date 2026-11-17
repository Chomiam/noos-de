//! noos-firewall — pare-feu de NOOS en ligne de commande.
//!
//!   noos-firewall status            état du service et réglages
//!   noos-firewall rules             règles par programme et par port
//!   noos-firewall log [N]           dernières connexions bloquées
//!   noos-firewall pending           demandes en attente
//!   noos-firewall ruleset           table nftables générée depuis /etc/noos/firewall
//!   noos-firewall check             vérifie cette table avec `nft -c` (sans root)
//!   noos-firewall open PORTS [tcp|udp|any] [lan]   ouvre un port en entrée (polkit)
//!   noos-firewall close ID          supprime une règle de port (polkit)
//!   noos-firewall delete ID         supprime une règle de programme
//!   noos-firewall off               supprime la table nftables (root, secours)

use noos_firewall::{model::*, nft, proto, store};

fn fmt_conn(c: &Conn) -> String {
    let arrow = if c.direction == Direction::In { "←" } else { "→" };
    format!("{arrow} {}:{} {}", c.remote, c.port(), c.proto.label())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("status");
    match cmd {
        "status" => println!("{}", serde_json::to_string_pretty(&proto::status()?)?),
        "rules" => {
            let r = proto::rules()?;
            println!("Programmes :");
            for a in &r.apps {
                let user = a.user.map(|u| format!(" (compte {u})")).unwrap_or_default();
                let port = a.port.as_ref().map(|p| format!(" port {p}")).unwrap_or_default();
                let remote = a.remote.as_ref().map(|p| format!(" vers {p}")).unwrap_or_default();
                println!("  #{:<4} {:<8} {:<20} {}{port}{remote}{user}{}", a.id, a.action.label(), a.direction.label(), a.app, if a.enabled { "" } else { " [désactivée]" });
            }
            println!("Ports :");
            for p in &r.ports {
                println!(
                    "  #{:<4} {:<8} {:<20} {} {} {}{}",
                    p.id,
                    p.action.label(),
                    p.direction.label(),
                    p.ports,
                    p.proto.label(),
                    p.zone.label(),
                    p.interface.as_ref().map(|i| format!(" ({i})")).unwrap_or_default()
                );
            }
        }
        "log" => {
            let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(50);
            for e in proto::log(n)? {
                println!("{} {:<30} {} ×{} ({})", e.time, if e.program.comm.is_empty() { &e.program.key } else { &e.program.comm }, fmt_conn(&e.conn), e.count, e.reason);
            }
        }
        "pending" => {
            for p in proto::pending()? {
                println!("#{} {} {} (encore {} s)", p.id, p.program.key, fmt_conn(&p.conn), p.deadline_secs);
            }
        }
        "ruleset" | "check" => {
            let d = store::dir();
            let t = nft::ruleset(&store::load_config(&d), &store::load_rules(&d).ports);
            if cmd == "ruleset" {
                print!("{t}");
            } else {
                nft::check(&t)?;
                println!("Table valide (nft -c).");
            }
        }
        "open" => {
            let ports = args.get(1).ok_or_else(|| anyhow::anyhow!("port manquant"))?;
            let proto_ = match args.get(2).map(String::as_str) {
                Some("udp") => Proto::Udp,
                Some("any") | Some("both") => Proto::Any,
                _ => Proto::Tcp,
            };
            let zone = if args.iter().any(|a| a == "lan") { Zone::Lan } else { Zone::Any };
            let r = PortRule { enabled: true, direction: Direction::In, action: Action::Allow, proto: proto_, ports: validate_ports(ports)?, zone, ..Default::default() };
            println!("Règle {} créée.", proto::call(&proto::Request::SavePortRule(r))?);
        }
        "close" | "delete" => {
            let id: u64 = args.get(1).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow::anyhow!("numéro de règle manquant"))?;
            let req = if cmd == "close" { proto::Request::DeletePortRule { id } } else { proto::Request::DeleteAppRule { id } };
            proto::call(&req)?;
            println!("Règle {id} supprimée.");
        }
        "off" => {
            nft::remove()?;
            println!("Table inet noos supprimée (elle revient au prochain démarrage du service).");
        }
        _ => {
            eprintln!("commande inconnue : {cmd} (status, rules, log, pending, ruleset, check, open, close, delete, off)");
            std::process::exit(2);
        }
    }
    Ok(())
}

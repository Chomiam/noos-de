//! noos-firewalld — démon du pare-feu applicatif de NOOS (service systemd `noos-firewall`).
//!
//!   noos-firewalld                 démarre (root)
//!   noos-firewalld --print-ruleset affiche la table nftables qui serait appliquée

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--print-ruleset") {
        let d = noos_firewall::store::dir();
        let (c, r) = (noos_firewall::store::load_config(&d), noos_firewall::store::load_rules(&d));
        print!("{}", noos_firewall::nft::ruleset(&c, &r.ports));
        return;
    }
    let socket = noos_firewall::proto::socket_path();
    if let Err(e) = noos_firewall::daemon::run(&socket) {
        eprintln!("noos-firewalld : {e:#}");
        std::process::exit(1);
    }
}

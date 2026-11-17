//! Règle par programme applicable à une connexion : la plus précise l'emporte (adresse + port >
//! port > adresse > programme seul ; règle d'un compte > règle de tous les comptes) ; à
//! précision égale, le blocage gagne.

use crate::model::{Action, AppRule, Conn, Program, port_matches, remote_matches};

/// La clé `pattern` (exacte ou préfixe terminé par « * ») couvre le programme.
pub fn app_matches(pattern: &str, key: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => key.starts_with(prefix) && !key.is_empty(),
        None => pattern == key,
    }
}

fn specificity(r: &AppRule) -> u32 {
    let mut s = 0;
    if r.port.is_some() {
        s += 4;
    }
    if r.remote.is_some() {
        s += 2;
    }
    if !r.app.ends_with('*') {
        s += 8;
    }
    if r.user.is_some() {
        s += 1;
    }
    s * 2
}

pub fn rule_applies(r: &AppRule, p: &Program, c: &Conn) -> bool {
    r.enabled
        && app_matches(&r.app, &p.key)
        && r.direction.covers(c.direction)
        && r.proto.covers(c.proto)
        && r.user.is_none_or(|u| u == p.uid)
        && r.port.as_deref().is_none_or(|spec| port_matches(spec, c.port()))
        && r.remote.as_deref().is_none_or(|net| remote_matches(net, &c.remote))
}

pub fn find<'a>(rules: &'a [AppRule], p: &Program, c: &Conn) -> Option<&'a AppRule> {
    rules.iter().filter(|r| rule_applies(r, p, c)).max_by_key(|r| specificity(r) + u32::from(r.action == Action::Block))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Direction, Proto};

    fn prog(key: &str, uid: u32) -> Program {
        Program { key: key.into(), exe: key.into(), uid, ..Default::default() }
    }

    fn conn(dir: Direction, remote: &str, port: u16) -> Conn {
        Conn { direction: dir, proto: Proto::Tcp, ip_proto: 6, remote: remote.into(), remote_port: port, local_port: port, ..Default::default() }
    }

    fn r(id: u64, app: &str, action: Action) -> AppRule {
        AppRule { id, enabled: true, app: app.into(), direction: Direction::Out, action, ..Default::default() }
    }

    #[test]
    fn most_specific_wins() {
        let mut block443 = r(2, "/usr/bin/curl", Action::Block);
        block443.port = Some("443".into());
        let rules = vec![r(1, "/usr/bin/curl", Action::Allow), block443, r(3, "/opt/noos/bin/*", Action::Allow)];
        let p = prog("/usr/bin/curl", 1000);
        assert_eq!(find(&rules, &p, &conn(Direction::Out, "1.1.1.1", 80)).unwrap().id, 1);
        assert_eq!(find(&rules, &p, &conn(Direction::Out, "1.1.1.1", 443)).unwrap().id, 2);
        assert_eq!(find(&rules, &prog("/opt/noos/bin/noos-settings", 1000), &conn(Direction::Out, "1.1.1.1", 443)).unwrap().id, 3);
        // Entrée : pas couverte par des règles « sortant ».
        assert!(find(&rules, &p, &conn(Direction::In, "1.1.1.1", 80)).is_none());
    }

    #[test]
    fn block_wins_ties_and_users() {
        let mut mine = r(2, "/usr/bin/curl", Action::Allow);
        mine.user = Some(1000);
        let rules = vec![r(1, "/usr/bin/curl", Action::Block), mine.clone()];
        // Règle du compte 1000 plus précise que la règle globale.
        assert_eq!(find(&rules, &prog("/usr/bin/curl", 1000), &conn(Direction::Out, "1.1.1.1", 80)).unwrap().id, 2);
        assert_eq!(find(&rules, &prog("/usr/bin/curl", 1001), &conn(Direction::Out, "1.1.1.1", 80)).unwrap().id, 1);
        let rules = vec![r(1, "/usr/bin/curl", Action::Allow), r(2, "/usr/bin/curl", Action::Block)];
        assert_eq!(find(&rules, &prog("/usr/bin/curl", 0), &conn(Direction::Out, "1.1.1.1", 80)).unwrap().id, 2);
    }

    #[test]
    fn remote_and_disabled() {
        let mut lan = r(1, "/usr/bin/ssh", Action::Allow);
        lan.remote = Some("192.168.0.0/16".into());
        let mut off = r(2, "/usr/bin/ssh", Action::Allow);
        off.enabled = false;
        let rules = vec![lan, off];
        let p = prog("/usr/bin/ssh", 1000);
        assert!(find(&rules, &p, &conn(Direction::Out, "192.168.1.2", 22)).is_some());
        assert!(find(&rules, &p, &conn(Direction::Out, "8.8.8.8", 22)).is_none());
        assert!(!app_matches("*", ""));
    }
}

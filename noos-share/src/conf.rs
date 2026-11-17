//! Fichiers générés : bloc NOOS de smb.conf, fichier de sshd, services Avahi, règles du
//! pare-feu. Fonctions pures (testées) : le script privilégié ne fait qu'écrire leur résultat.

use noos_firewall::model::{Action, Direction, PortRule, Proto, Zone};

use crate::model::{Encrypt, Samba, Ssh};

pub const SMB_CONF: &str = "/etc/samba/smb.conf";
pub const SSHD_DROPIN: &str = "/etc/ssh/sshd_config.d/90-noos.conf";
pub const AVAHI_SSH: &str = "/etc/avahi/services/noos-sftp.service";
pub fn avahi_webdav_path(user: &str) -> String {
    format!("/etc/avahi/services/noos-webdav-{user}.service")
}

const BEGIN: &str = "# >>> NOOS";
const END: &str = "# <<< NOOS";

/// Réseaux locaux (mêmes plages que la zone « Réseau local » du pare-feu NOOS).
pub const LAN: &str = "127.0.0.1 ::1 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 169.254.0.0/16 fe80::/10 fc00::/7";

/// Bloc ajouté à la fin de smb.conf. Samba accepte de rouvrir `[global]` et `[homes]` : les
/// valeurs du bloc, lues en dernier, remplacent celles de Debian (invités, `[homes]` en lecture
/// seule) sans toucher au reste du fichier.
pub fn samba_block(s: &Samba, hostname: &str) -> String {
    let yn = |b: bool| if b { "yes" } else { "no" };
    let mut o = String::new();
    o.push_str(BEGIN);
    o.push_str(" — Paramètres › Réseau et Internet › Partage de fichiers.\n");
    o.push_str("# Bloc réécrit par NOOS à chaque modification : placez vos réglages au-dessus.\n");
    o.push_str("[global]\n");
    o.push_str(&format!("   netbios name = {}\n", crate::model::netbios_name(hostname)));
    o.push_str(&format!("   workgroup = {}\n", s.workgroup));
    o.push_str("   server string = NOOS\n");
    o.push_str("   server role = standalone server\n");
    // Jamais d'invité : un mot de passe inconnu est refusé, pas transformé en accès anonyme.
    o.push_str("   map to guest = never\n");
    o.push_str("   restrict anonymous = 2\n");
    o.push_str("   usershare allow guests = no\n");
    // Le mot de passe de partage reste distinct de celui de la session.
    o.push_str("   unix password sync = no\n");
    o.push_str("   pam password change = no\n");
    o.push_str("   server min protocol = SMB2_10\n");
    o.push_str(&format!("   server smb encrypt = {}\n", match s.encrypt { Encrypt::Desired => "desired", Encrypt::Required => "required" }));
    o.push_str(&format!("   hosts allow = {LAN}\n"));
    o.push_str("   hosts deny = ALL\n");
    o.push_str("   disable netbios = yes\n");
    o.push_str("   multicast dns register = yes\n");
    o.push_str("\n[homes]\n");
    o.push_str("   comment = Dossier personnel\n");
    o.push_str(&format!("   available = {}\n", yn(s.home)));
    o.push_str("   browseable = no\n");
    o.push_str(&format!("   read only = {}\n", yn(!s.home_writable)));
    o.push_str("   valid users = %S\n");
    o.push_str("   guest ok = no\n");
    // Les sections lues avant ce bloc (dont le [homes] de Debian) ne reprennent pas les valeurs
    // du [global] rouvert : restriction répétée ici (Samba vérifie aussi la valeur globale).
    o.push_str(&format!("   hosts allow = {LAN}\n   hosts deny = ALL\n"));
    o.push_str("   create mask = 0600\n");
    o.push_str("   directory mask = 0700\n");
    for sh in &s.shares {
        o.push_str(&format!("\n[{}]\n", sh.name));
        o.push_str("   comment = Partagé depuis NOOS\n");
        o.push_str(&format!("   path = {}\n", sh.path));
        o.push_str(&format!("   valid users = {}\n", sh.owner));
        o.push_str(&format!("   read only = {}\n", yn(!sh.writable)));
        o.push_str("   browseable = yes\n");
        o.push_str("   guest ok = no\n");
        o.push_str(&format!("   hosts allow = {LAN}\n   hosts deny = ALL\n"));
        o.push_str("   create mask = 0644\n");
        o.push_str("   directory mask = 0755\n");
    }
    o.push_str(END);
    o.push('\n');
    o
}

/// Remplace (ou ajoute) le bloc NOOS d'un fichier ; `block` vide = le retirer.
pub fn merge_block(existing: &str, block: &str) -> String {
    let start = existing.find(BEGIN);
    let end = start.and_then(|s| existing[s..].find(END).map(|e| s + e + END.len()));
    let (before, after) = match (start, end) {
        (Some(s), Some(e)) => (&existing[..s], existing[e..].trim_start_matches('\n')),
        _ => (existing, ""),
    };
    let mut out = before.trim_end_matches('\n').to_string();
    if !out.is_empty() {
        out.push_str(if block.is_empty() { "\n" } else { "\n\n" });
    }
    out.push_str(block);
    if !after.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(after);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Réglages de sshd. Les fichiers de `sshd_config.d` sont lus avant le reste et la première
/// valeur l'emporte : ces réglages priment sur ceux de Debian.
pub fn sshd_dropin(s: &Ssh) -> String {
    let yn = |b: bool| if b { "yes" } else { "no" };
    format!(
        "# NOOS — Paramètres › Réseau et Internet › SSH et SFTP (fichier réécrit par NOOS).\n\
         Port {}\n\
         PasswordAuthentication {}\n\
         KbdInteractiveAuthentication no\n\
         PermitEmptyPasswords no\n\
         PermitRootLogin no\n",
        s.port,
        yn(s.password_auth)
    )
}

/// Annonce mDNS (Avahi) d'un ou plusieurs services.
pub fn avahi_service(name: &str, services: &[(&str, u16, &[&str])]) -> String {
    let mut o = String::from("<?xml version=\"1.0\" standalone='no'?>\n<!DOCTYPE service-group SYSTEM \"avahi-service.dtd\">\n<!-- Écrit par NOOS (Paramètres › Réseau et Internet) -->\n<service-group>\n");
    o.push_str(&format!("  <name replace-wildcards=\"yes\">{}</name>\n", xml(name)));
    for (ty, port, txt) in services {
        o.push_str(&format!("  <service>\n    <type>{ty}</type>\n    <port>{port}</port>\n"));
        for t in *txt {
            o.push_str(&format!("    <txt-record>{}</txt-record>\n", xml(t)));
        }
        o.push_str("  </service>\n");
    }
    o.push_str("</service-group>\n");
    o
}

pub fn avahi_ssh(port: u16) -> String {
    avahi_service("%h", &[("_sftp-ssh._tcp", port, &[]), ("_ssh._tcp", port, &[])])
}

pub fn avahi_webdav(user: &str, port: u16, https: bool) -> String {
    let ty = if https { "_webdavs._tcp" } else { "_webdav._tcp" };
    avahi_service(&format!("WebDAV de {user} sur %h"), &[(ty, port, &["path=/"])])
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

// ───────────────────────────── Pare-feu ─────────────────────────────

/// Étiquettes des règles de port créées par NOOS (champ `note` du pare-feu).
pub const TAG_SAMBA: &str = "Partage NOOS · Samba";
pub const TAG_DISCOVERY: &str = "Partage NOOS · Découverte réseau";
pub const TAG_SSH: &str = "Partage NOOS · SSH et SFTP";
pub fn tag_webdav(user: &str) -> String {
    format!("Partage NOOS · WebDAV ({user})")
}

fn lan_in(proto: Proto, ports: &str, note: &str) -> PortRule {
    PortRule { id: 0, enabled: true, direction: Direction::In, action: Action::Allow, proto, ports: ports.into(), zone: Zone::Lan, interface: None, remote: None, note: note.into() }
}

/// Règles voulues pour Samba et SSH (entrées, réseau local seulement).
pub fn system_rules(samba: &Samba, ssh: &Ssh) -> Vec<PortRule> {
    let mut v = vec![];
    if samba.enabled {
        v.push(lan_in(Proto::Tcp, "139,445", TAG_SAMBA));
        // WSD (wsdd2 : 3702 en UDP et en TCP — fiche du PC lue par Windows ; 5357/tcp pour
        // les autres serveurs WSD), LLMNR (5355), mDNS (Avahi : 5353/udp).
        v.push(lan_in(Proto::Udp, "3702,5353,5355", TAG_DISCOVERY));
        v.push(lan_in(Proto::Tcp, "3702,5355,5357", TAG_DISCOVERY));
    } else if ssh.enabled {
        v.push(lan_in(Proto::Udp, "5353", TAG_DISCOVERY));
    }
    if ssh.enabled {
        v.push(lan_in(Proto::Tcp, &ssh.port.to_string(), TAG_SSH));
    }
    v
}

pub fn webdav_rules(user: &str, port: Option<u16>) -> Vec<PortRule> {
    port.map(|p| vec![lan_in(Proto::Tcp, &p.to_string(), &tag_webdav(user))]).unwrap_or_default()
}

/// Différence entre les règles en place et les règles voulues pour les étiquettes `tags` :
/// (identifiants à supprimer, règles à créer). Les règles identiques sont gardées.
pub fn rules_diff(existing: &[PortRule], wanted: &[PortRule], tags: &[&str]) -> (Vec<u64>, Vec<PortRule>) {
    let ours: Vec<&PortRule> = existing.iter().filter(|r| tags.contains(&r.note.as_str())).collect();
    let same = |a: &PortRule, b: &PortRule| a.direction == b.direction && a.action == b.action && a.proto == b.proto && a.ports == b.ports && a.zone == b.zone && a.note == b.note && a.enabled == b.enabled;
    let remove = ours.iter().filter(|r| !wanted.iter().any(|w| same(r, w))).map(|r| r.id).collect();
    let add = wanted.iter().filter(|w| !ours.iter().any(|r| same(r, w))).cloned().collect();
    (remove, add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Share;

    #[test]
    fn bloc_samba() {
        let mut s = Samba::default();
        s.shares.push(Share { name: "Photos".into(), path: "/home/axel/Images".into(), writable: false, owner: "axel".into() });
        let b = samba_block(&s, "salon");
        assert!(b.contains("netbios name = SALON"));
        assert!(b.contains("map to guest = never"));
        assert!(b.contains("[homes]\n   comment = Dossier personnel\n   available = yes"));
        assert!(b.contains("[Photos]\n   comment = Partagé depuis NOOS\n   path = /home/axel/Images\n   valid users = axel\n   read only = yes"));
        assert!(!b.contains("guest ok = yes"));
        assert_eq!(b.matches("hosts deny = ALL").count(), 3);
        s.home = false;
        assert!(samba_block(&s, "salon").contains("available = no"));
    }

    #[test]
    fn fusion() {
        let debian = "[global]\n   workgroup = WORKGROUP\n\n[print$]\n   path = /var/lib/samba/printers\n";
        let b1 = samba_block(&Samba::default(), "pc");
        let once = merge_block(debian, &b1);
        assert!(once.starts_with(debian.trim_end()));
        assert!(once.ends_with("# <<< NOOS\n"));
        // Deuxième passage : un seul bloc, remplacé.
        let mut s = Samba::default();
        s.workgroup = "MAISON".into();
        let twice = merge_block(&once, &samba_block(&s, "pc"));
        assert_eq!(twice.matches(BEGIN).count(), 1);
        assert!(twice.contains("workgroup = MAISON"));
        // Réglages ajoutés après le bloc par l'utilisateur : conservés.
        let with_tail = format!("{twice}\n[perso]\n   path = /srv\n");
        let again = merge_block(&with_tail, &b1);
        assert!(again.contains("[perso]") && again.matches(BEGIN).count() == 1);
        // Retrait.
        let removed = merge_block(&once, "");
        assert!(!removed.contains(BEGIN));
        assert!(removed.contains("[print$]"));
    }

    #[test]
    fn sshd() {
        let t = sshd_dropin(&Ssh { enabled: true, port: 2222, password_auth: false });
        assert!(t.contains("Port 2222\nPasswordAuthentication no\n"));
        assert!(t.contains("PermitRootLogin no"));
    }

    #[test]
    fn avahi() {
        let x = avahi_webdav("a&b", 8443, true);
        assert!(x.contains("_webdavs._tcp") && x.contains("<port>8443</port>") && x.contains("a&amp;b"));
    }

    #[test]
    fn regles() {
        let samba = Samba::default();
        let ssh = Ssh { enabled: true, port: 22, password_auth: true };
        let wanted = system_rules(&samba, &ssh);
        assert_eq!(wanted.len(), 4);
        assert!(wanted.iter().any(|r| r.proto == Proto::Tcp && r.ports.split(',').any(|p| p == "3702")));
        assert!(wanted.iter().all(|r| r.zone == Zone::Lan && r.direction == Direction::In));
        let mut existing: Vec<PortRule> = wanted.iter().cloned().enumerate().map(|(i, mut r)| {
            r.id = i as u64 + 1;
            r
        }).collect();
        existing.push(PortRule { id: 99, note: "Minecraft".into(), ports: "25565".into(), ..Default::default() });
        let tags = [TAG_SAMBA, TAG_DISCOVERY, TAG_SSH];
        assert_eq!(rules_diff(&existing, &wanted, &tags), (vec![], vec![]));
        // SSH coupé : seule sa règle part ; la règle de l'utilisateur n'est jamais touchée.
        let (rm, add) = rules_diff(&existing, &system_rules(&samba, &Ssh::default()), &tags);
        assert_eq!(rm, vec![4]);
        assert!(add.is_empty());
        // Port changé : remplacement.
        let (rm, add) = rules_diff(&existing, &system_rules(&samba, &Ssh { port: 2222, ..ssh }), &tags);
        assert_eq!(rm, vec![4]);
        assert_eq!(add[0].ports, "2222");
    }
}

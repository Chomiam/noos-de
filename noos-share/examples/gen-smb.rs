//! Génère smb.conf (Debian + bloc NOOS) pour vérification par testparm : `gen-smb <smb.conf de base>`.
fn main() {
    let base = std::fs::read_to_string(std::env::args().nth(1).expect("smb.conf de base")).unwrap();
    let mut s = noos_share::model::Samba::default();
    s.shares.push(noos_share::model::Share { name: "Photos de vacances".into(), path: "/tmp".into(), writable: false, owner: "axel".into() });
    s.encrypt = noos_share::model::Encrypt::Required;
    print!("{}", noos_share::conf::merge_block(&base, &noos_share::conf::samba_block(&s, "salon-de-jeux")));
}

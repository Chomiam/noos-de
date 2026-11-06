//! AUR via un assistant (paru ou yay). Les PKGBUILD de l'AUR ne sont pas vérifiés :
//! l'interface doit l'indiquer clairement avant toute installation.

use super::pacman::{parse_arrow_updates, parse_ss};
use crate::{
    Backend, BackendKind, Package, Plan, Preset, Repo, Update,
    exec::{capture_codes, have},
};

pub struct Aur {
    helper: &'static str,
}

impl Aur {
    pub fn detect() -> Option<Self> {
        ["paru", "yay"].into_iter().find(|h| have(h)).map(|helper| Self { helper })
    }

    /// Options non interactives communes à paru et yay, élévation via pkexec.
    fn noninteractive(&self) -> Vec<String> {
        let mut v: Vec<String> = vec!["--noconfirm".into(), "--sudo".into(), "pkexec".into()];
        match self.helper {
            "paru" => v.push("--skipreview".into()),
            _ => v.extend(["--answerdiff", "None", "--answerclean", "None", "--answeredit", "None"].map(String::from)),
        }
        v
    }
}

impl Backend for Aur {
    fn kind(&self) -> BackendKind {
        BackendKind::Aur
    }

    fn search(&self, query: &str) -> anyhow::Result<Vec<Package>> {
        let q = query.trim();
        if q.len() < 2 {
            return Ok(vec![]);
        }
        let out = capture_codes(&[self.helper, "-Ssa", q], &[0, 1])?;
        let mut pkgs = parse_ss(&out, BackendKind::Aur);
        crate::rank(q, &mut pkgs);
        pkgs.truncate(80);
        Ok(pkgs)
    }

    fn installed(&self) -> anyhow::Result<Vec<Package>> {
        // Paquets « étrangers » = hors dépôts (AUR ou installés à la main).
        let out = capture_codes(&["pacman", "-Qm"], &[0, 1])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(n, v)| Package {
                backend: Some(BackendKind::Aur),
                id: n.into(),
                name: n.into(),
                version: v.into(),
                installed: true,
                origin: "aur".into(),
                ..Default::default()
            })
            .collect())
    }

    fn updates(&self) -> anyhow::Result<Vec<Update>> {
        Ok(parse_arrow_updates(&capture_codes(&[self.helper, "-Qua"], &[0, 1])?, BackendKind::Aur))
    }

    fn refresh(&self) -> Plan {
        Plan::new()
    }

    fn install(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> = vec![self.helper.into(), "-S".into(), "--needed".into()];
        argv.extend(self.noninteractive());
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().user_owned(argv)
    }

    fn remove(&self, packages: &[Package]) -> Plan {
        let mut argv: Vec<String> = ["pacman", "-Rns", "--noconfirm"].map(String::from).to_vec();
        argv.extend(packages.iter().map(|p| p.id.clone()));
        Plan::new().root_owned(argv)
    }

    fn upgrade_all(&self) -> Plan {
        let mut argv: Vec<String> = vec![self.helper.into(), "-Sua".into()];
        argv.extend(self.noninteractive());
        Plan::new().user_owned(argv)
    }

    fn repos(&self) -> anyhow::Result<Vec<Repo>> {
        Ok(vec![])
    }

    fn set_repo_enabled(&self, _: &Repo, _: bool) -> anyhow::Result<Plan> {
        anyhow::bail!("l'AUR n'a pas de dépôts à gérer")
    }

    fn remove_repo(&self, _: &Repo) -> anyhow::Result<Plan> {
        anyhow::bail!("l'AUR n'a pas de dépôts à gérer")
    }

    fn presets(&self) -> Vec<Preset> {
        vec![]
    }

    fn add_preset(&self, preset: &str, _: Option<&str>) -> anyhow::Result<Plan> {
        anyhow::bail!("préréglage inconnu : {preset}")
    }
}

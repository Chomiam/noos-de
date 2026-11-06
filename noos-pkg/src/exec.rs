//! Exécution des commandes : capture, flux de sortie en direct, élévation de privilèges.
//!
//! Une opération est un [`Plan`] : une suite d'étapes, certaines en root. Les étapes root
//! consécutives sont regroupées dans **un seul** `pkexec sh -c '…'` pour ne demander le
//! mot de passe qu'une fois.

use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};

/// Rappel recevant chaque ligne de sortie d'une opération.
pub type Log<'a> = &'a (dyn Fn(&str) + Sync);

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Commande (argv), éventuellement en root.
    Run { argv: Vec<String>, root: bool },
    /// Écrit un fichier (en root) avec le contenu donné.
    WriteFile { path: String, content: String },
    /// Supprime un fichier (en root).
    RemoveFile { path: String },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn user(mut self, argv: &[&str]) -> Self {
        self.steps.push(Step::Run { argv: argv.iter().map(|s| s.to_string()).collect(), root: false });
        self
    }
    pub fn root(mut self, argv: &[&str]) -> Self {
        self.steps.push(Step::Run { argv: argv.iter().map(|s| s.to_string()).collect(), root: true });
        self
    }
    pub fn root_owned(mut self, argv: Vec<String>) -> Self {
        self.steps.push(Step::Run { argv, root: true });
        self
    }
    pub fn user_owned(mut self, argv: Vec<String>) -> Self {
        self.steps.push(Step::Run { argv, root: false });
        self
    }
    pub fn write(mut self, path: &str, content: &str) -> Self {
        self.steps.push(Step::WriteFile { path: path.into(), content: content.into() });
        self
    }
    pub fn remove(mut self, path: &str) -> Self {
        self.steps.push(Step::RemoveFile { path: path.into() });
        self
    }
    pub fn extend(mut self, other: Plan) -> Self {
        self.steps.extend(other.steps);
        self
    }

    /// Représentation lisible (mode `--dry-run`).
    pub fn describe(&self) -> Vec<String> {
        self.steps
            .iter()
            .map(|s| match s {
                Step::Run { argv, root } => format!("{}{}", if *root { "# " } else { "$ " }, join_quoted(argv)),
                Step::WriteFile { path, content } => format!("# écrire {path} ({} lignes)", content.lines().count()),
                Step::RemoveFile { path } => format!("# supprimer {path}"),
            })
            .collect()
    }

    pub fn run(&self, log: Log) -> anyhow::Result<()> {
        let mut root_batch: Vec<String> = Vec::new();
        for step in &self.steps {
            match step {
                Step::Run { argv, root: false } => {
                    flush_root(&mut root_batch, log)?;
                    stream(argv, log)?;
                }
                Step::Run { argv, root: true } => root_batch.push(join_quoted(argv)),
                Step::WriteFile { path, content } => {
                    let parent = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().to_string());
                    if let Some(parent) = parent {
                        root_batch.push(format!("mkdir -p {}", quote(&parent)));
                    }
                    let mut marker = String::from("NOOS_EOF");
                    while content.contains(&marker) {
                        marker.push('_');
                    }
                    root_batch.push(format!("cat > {} <<'{marker}'\n{content}\n{marker}", quote(path)));
                }
                Step::RemoveFile { path } => root_batch.push(format!("rm -f -- {}", quote(path))),
            }
        }
        flush_root(&mut root_batch, log)
    }
}

fn flush_root(batch: &mut Vec<String>, log: Log) -> anyhow::Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    for item in batch.iter() {
        let first = item.lines().next().unwrap_or("");
        let shown = match first.split_once(" <<'") {
            Some((cmd, _)) => format!("{} (écriture du fichier)", cmd.trim_start_matches("cat > ")),
            None => first.to_string(),
        };
        log(&format!("→ (admin) {shown}"));
    }
    let script = format!("set -e\n{}", batch.join("\n"));
    batch.clear();
    let mut argv = privilege_prefix();
    argv.extend(["sh".to_string(), "-c".to_string(), script]);
    run_streaming(&argv, log)
}

/// Préfixe d'élévation : rien si on est déjà root, `sudo -n` si demandé, sinon `pkexec`.
pub fn privilege_prefix() -> Vec<String> {
    if is_root() {
        return vec![];
    }
    match std::env::var("NOOS_PKG_ELEVATE").as_deref() {
        Ok("sudo") => vec!["sudo".into(), "-n".into()],
        Ok("none") => vec![],
        _ => vec!["pkexec".into()],
    }
}

pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|euid| euid == "0"))
        })
        .unwrap_or(false)
}

pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+,".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

pub fn join_quoted(argv: &[String]) -> String {
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

pub fn have(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Exécute et capture stdout (stderr ignoré). Échec si le code de retour n'est pas dans `ok_codes`.
pub fn capture_codes(argv: &[&str], ok_codes: &[i32]) -> anyhow::Result<String> {
    let out = Command::new(argv[0])
        .args(&argv[1..])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("lancement de {}", argv[0]))?;
    let code = out.status.code().unwrap_or(-1);
    if !ok_codes.contains(&code) {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("{} a échoué ({code}) : {}", argv.join(" "), err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn capture(argv: &[&str]) -> anyhow::Result<String> {
    capture_codes(argv, &[0])
}

/// Exécute en transmettant stdout et stderr ligne par ligne à `log`.
pub fn stream(argv: &[String], log: Log) -> anyhow::Result<()> {
    log(&format!("→ {}", join_quoted(argv)));
    run_streaming(argv, log)
}

fn run_streaming(argv: &[String], log: Log) -> anyhow::Result<()> {
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("lancement de {}", argv[0]))?;

    let stderr = child.stderr.take().unwrap();
    let last_err = Arc::new(Mutex::new(String::new()));
    let last_err2 = last_err.clone();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let tx2 = tx.clone();
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            *last_err2.lock().unwrap() = line.clone();
            let _ = tx2.send(line);
        }
    });
    let stdout = child.stdout.take().unwrap();
    let out_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    for line in rx {
        // Les barres de progression utilisent \r : on garde le dernier segment.
        let line = line.rsplit('\r').next().unwrap_or("").to_string();
        if !line.trim().is_empty() {
            log(&line);
        }
    }
    let _ = out_thread.join();
    let _ = err_thread.join();
    let status = child.wait()?;
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        let hint = match (argv[0].as_str(), code) {
            ("pkexec", 126) => " — authentification refusée ou annulée".to_string(),
            ("pkexec", 127) => " — pas d'agent d'authentification polkit dans la session".to_string(),
            _ => String::new(),
        };
        bail!("échec (code {code}){hint} : {}", last_err.lock().unwrap());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("abc-1.2"), "abc-1.2");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn plan_describe() {
        let p = Plan::new().root(&["dnf", "install", "-y", "htop"]).write("/etc/x.repo", "a\nb");
        let d = p.describe();
        assert_eq!(d[0], "# dnf install -y htop");
        assert!(d[1].contains("/etc/x.repo"));
    }

    #[test]
    fn root_batch_runs_locally_when_root_disabled() {
        // NOOS_PKG_ELEVATE=none : exécute le lot root sans élévation (tests).
        unsafe { std::env::set_var("NOOS_PKG_ELEVATE", "none") };
        let dir = std::env::temp_dir().join(format!("noos-pkg-test-{}", std::process::id()));
        let file = dir.join("t.txt");
        let lines = Mutex::new(Vec::new());
        Plan::new()
            .write(file.to_str().unwrap(), "bonjour 'monde'")
            .root(&["cat", file.to_str().unwrap()])
            .run(&|l| lines.lock().unwrap().push(l.to_string()))
            .unwrap();
        assert!(lines.lock().unwrap().iter().any(|l| l == "bonjour 'monde'"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

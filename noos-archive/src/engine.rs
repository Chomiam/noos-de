//! Moteur d'archives : trois outils éprouvés, choisis selon le format.
//!
//! - **7-Zip** (`7z`) : 7z, zip et dérivés (jar, docx, epub, apk…), iso/udf, dmg, wim, cab,
//!   msi, rpm, deb, cpio, squashfs, images de disques virtuels, arj, lzh… ; création 7z, zip,
//!   wim, tar (mot de passe AES-256, volumes) ; ajout et suppression de fichiers ;
//! - **libarchive** (`bsdtar`) : tar et tar compressés (gz, bz2, xz, zst, lz4, lz, lzma) ;
//! - **unar** (`lsar` / `unar`) : RAR (dont RAR5), StuffIt, ALZ, ARC, ZOO, ACE… et en secours.
//!
//! Images ISO créées par `xorriso`. Chaque opération longue rapporte sa progression (`Job`)
//! et peut être annulée.

use std::{
    io::{BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::Value;

// ───────────────────────────── Formats ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    SevenZip,
    Tar,
    Unar,
}

/// Format d'archive à créer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutFormat {
    SevenZ,
    Zip,
    TarGz,
    TarXz,
    TarZst,
    TarBz2,
    TarLz4,
    TarLz,
    Tar,
    Wim,
    Iso,
}

impl OutFormat {
    pub const ALL: [OutFormat; 11] = [
        OutFormat::SevenZ,
        OutFormat::Zip,
        OutFormat::TarZst,
        OutFormat::TarXz,
        OutFormat::TarGz,
        OutFormat::TarBz2,
        OutFormat::TarLz4,
        OutFormat::TarLz,
        OutFormat::Tar,
        OutFormat::Wim,
        OutFormat::Iso,
    ];

    pub fn ext(self) -> &'static str {
        match self {
            OutFormat::SevenZ => "7z",
            OutFormat::Zip => "zip",
            OutFormat::TarGz => "tar.gz",
            OutFormat::TarXz => "tar.xz",
            OutFormat::TarZst => "tar.zst",
            OutFormat::TarBz2 => "tar.bz2",
            OutFormat::TarLz4 => "tar.lz4",
            OutFormat::TarLz => "tar.lz",
            OutFormat::Tar => "tar",
            OutFormat::Wim => "wim",
            OutFormat::Iso => "iso",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            OutFormat::SevenZ => "La meilleure compression, mot de passe AES-256 et noms chiffrés",
            OutFormat::Zip => "Compatible partout (Windows, macOS, téléphones)",
            OutFormat::TarZst => "Très rapide et très efficace (Zstandard)",
            OutFormat::TarXz => "Compression maximale pour Linux",
            OutFormat::TarGz => "Le classique de Linux, lisible partout",
            OutFormat::TarBz2 => "Ancien standard, bonne compression",
            OutFormat::TarLz4 => "Ultra rapide, compression légère",
            OutFormat::TarLz => "Lzip : compression maximale, archivage à long terme",
            OutFormat::Tar => "Sans compression : regroupe seulement les fichiers",
            OutFormat::Wim => "Image Windows (déduplication des fichiers identiques)",
            OutFormat::Iso => "Image de CD / DVD / clé (ISO 9660 + Joliet + Rock Ridge)",
        }
    }

    pub fn supports_password(self) -> bool {
        matches!(self, OutFormat::SevenZ | OutFormat::Zip)
    }

    pub fn supports_split(self) -> bool {
        matches!(self, OutFormat::SevenZ | OutFormat::Zip)
    }

    pub fn supports_level(self) -> bool {
        !matches!(self, OutFormat::Tar | OutFormat::Iso)
    }
}

pub use noos_core::archive::{is_archive, lower_name, stem, EXTENSIONS, TAR_SUFFIXES};

/// Formats confiés d'abord à unar (absents du 7-Zip de Fedora ou mieux gérés par unar).
const UNAR_EXTS: &[&str] = &["rar", "sit", "sitx", "alz", "arc", "ark", "zoo", "ace", "cpt", "sea", "lbr", "pak", "pma", "dd", "nsa", "adf", "dms", "lzx"];

/// Moteurs à essayer, dans l'ordre, pour lire `path`.
fn engines_for(path: &Path) -> Vec<Engine> {
    let n = lower_name(path);
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
    if TAR_SUFFIXES.iter().any(|s| n.ends_with(s)) {
        vec![Engine::Tar, Engine::SevenZip, Engine::Unar]
    } else if UNAR_EXTS.contains(&ext.as_str()) || n.ends_with(".cbr") {
        vec![Engine::Unar, Engine::SevenZip, Engine::Tar]
    } else {
        vec![Engine::SevenZip, Engine::Unar, Engine::Tar]
    }
}

// ───────────────────────────── Contenu ─────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// Chemin dans l'archive, séparé par « / », sans « / » final.
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub packed: Option<u64>,
    pub modified: String,
    pub encrypted: bool,
    pub method: String,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone)]
pub struct Archive {
    pub path: PathBuf,
    pub engine: Engine,
    pub format: String,
    pub entries: Vec<Entry>,
    pub physical_size: u64,
    pub solid: Option<bool>,
    pub method: String,
    /// Les noms eux-mêmes étaient chiffrés (mot de passe demandé pour lister).
    pub header_encrypted: bool,
    pub password: Option<String>,
}

impl Archive {
    pub fn total_size(&self) -> u64 {
        self.entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum()
    }
    pub fn files(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_dir).count()
    }
    pub fn dirs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_dir).count()
    }
    pub fn encrypted(&self) -> bool {
        self.header_encrypted || self.entries.iter().any(|e| e.encrypted)
    }
    /// Modifiable sur place (ajout / suppression de fichiers) par 7-Zip.
    pub fn writable(&self) -> bool {
        self.engine == Engine::SevenZip && matches!(self.format.to_lowercase().as_str(), "7z" | "zip" | "tar" | "wim")
    }
    /// Contenu d'un dossier de l'archive (« » = racine), dossiers implicites compris.
    pub fn children(&self, dir: &str) -> Vec<Entry> {
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let mut out: Vec<Entry> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for e in &self.entries {
            let Some(rest) = e.path.strip_prefix(&prefix) else { continue };
            if rest.is_empty() {
                continue;
            }
            match rest.split_once('/') {
                None => {
                    if seen.insert(rest.to_string()) {
                        out.push(e.clone());
                    } else if let Some(x) = out.iter_mut().find(|x| x.name() == rest) {
                        // Entrée explicite d'un dossier vu d'abord implicitement.
                        x.modified = e.modified.clone();
                    }
                }
                Some((first, _)) => {
                    if seen.insert(first.to_string()) {
                        out.push(Entry { path: format!("{prefix}{first}"), is_dir: true, ..Default::default() });
                    }
                }
            }
        }
        for d in out.iter_mut().filter(|d| d.is_dir) {
            let p = format!("{}/", d.path);
            d.size = self.entries.iter().filter(|e| !e.is_dir && e.path.starts_with(&p)).map(|e| e.size).sum();
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase())));
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    /// Mot de passe nécessaire (ou incorrect) pour lire le contenu.
    Password,
    Other(String),
}

fn output(cmd: &mut Command) -> Result<(bool, String, String), String> {
    let out = cmd.env("LC_ALL", "C.UTF-8").stdin(Stdio::null()).output().map_err(|e| format!("{e}"))?;
    Ok((out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned()))
}

/// Mot de passe passé à 7-Zip : jamais vide (sinon il le demanderait sur le terminal).
fn pw_arg(pw: Option<&str>) -> String {
    format!("-p{}", pw.filter(|p| !p.is_empty()).unwrap_or("\u{1}noos"))
}

/// Ouvre une archive (liste son contenu).
pub fn open(path: &Path, password: Option<&str>) -> Result<Archive, OpenError> {
    let mut last = String::from("format non reconnu");
    for engine in engines_for(path) {
        let res = match engine {
            Engine::SevenZip => list_7z(path, password),
            Engine::Tar => list_tar(path),
            Engine::Unar => list_unar(path, password),
        };
        match res {
            Ok(mut a) => {
                a.password = password.map(String::from);
                return Ok(a);
            }
            Err(OpenError::Password) => return Err(OpenError::Password),
            Err(OpenError::Other(e)) => last = e,
        }
    }
    Err(OpenError::Other(last))
}

fn kv(line: &str) -> Option<(&str, &str)> {
    line.split_once(" = ").or_else(|| line.strip_suffix(" =").map(|k| (k, "")))
}

fn list_7z(path: &Path, password: Option<&str>) -> Result<Archive, OpenError> {
    let (ok, out, err) = output(Command::new("7z").args(["l", "-slt", &pw_arg(password), "--"]).arg(path)).map_err(OpenError::Other)?;
    let all = format!("{out}\n{err}");
    if all.contains("Wrong password") || all.contains("Can not open encrypted archive") {
        return Err(OpenError::Password);
    }
    if !ok && !out.contains("----------") {
        let msg = all.lines().find(|l| l.contains("ERROR") || l.contains("Can not open")).unwrap_or("illisible par 7-Zip").trim().to_string();
        return Err(OpenError::Other(msg));
    }
    let (head, body) = out.split_once("\n----------\n").unwrap_or((&out, ""));
    let mut a = Archive {
        path: path.to_path_buf(),
        engine: Engine::SevenZip,
        format: String::new(),
        entries: vec![],
        physical_size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        solid: None,
        method: String::new(),
        header_encrypted: password.is_some_and(|p| !p.is_empty()) && out.contains("Encrypted = +"),
        password: None,
    };
    for line in head.lines().rev() {
        if let Some((k, v)) = kv(line) {
            match k {
                "Type" if a.format.is_empty() => a.format = v.to_string(),
                "Solid" => a.solid = Some(v == "+"),
                "Method" if a.method.is_empty() => a.method = v.to_string(),
                "Physical Size" => a.physical_size = v.parse().unwrap_or(a.physical_size),
                _ => {}
            }
        }
    }
    for block in body.split("\n\n") {
        let mut e = Entry::default();
        let mut attrs = String::new();
        for line in block.lines() {
            if let Some((k, v)) = kv(line) {
                match k {
                    "Path" => e.path = v.replace('\\', "/").trim_end_matches('/').to_string(),
                    "Size" => e.size = v.parse().unwrap_or(0),
                    "Packed Size" => e.packed = v.parse().ok(),
                    "Modified" => e.modified = v.split('.').next().unwrap_or(v).to_string(),
                    "Attributes" => attrs = v.to_string(),
                    "Folder" => e.is_dir = v == "+",
                    "Encrypted" => e.encrypted = v == "+",
                    "Method" => e.method = v.to_string(),
                    _ => {}
                }
            }
        }
        if attrs.starts_with('D') || attrs.contains(" d") {
            e.is_dir = true;
        }
        if !e.path.is_empty() {
            a.entries.push(e);
        }
    }
    if a.format.is_empty() {
        a.format = path.extension().map(|x| x.to_string_lossy().to_uppercase()).unwrap_or_default();
    }
    // Archive solide : les fichiers sont compressés ensemble, par blocs ; la taille compressée
    // d'un fichier n'a pas de sens (7-Zip donne celle du bloc au premier, rien aux autres).
    if a.solid == Some(true) {
        for e in &mut a.entries {
            e.packed = None;
        }
    }
    Ok(a)
}

fn list_tar(path: &Path) -> Result<Archive, OpenError> {
    let (ok, out, err) = output(Command::new("bsdtar").args(["-tvf"]).arg(path)).map_err(OpenError::Other)?;
    if !ok {
        return Err(OpenError::Other(err.lines().next().unwrap_or("illisible par libarchive").to_string()));
    }
    let mut entries = Vec::new();
    for line in out.lines() {
        // -rw-r--r--  0 user group     8 Nov  7 23:50 dossier/a.txt
        let mut idx = 0;
        let mut fields = Vec::new();
        let bytes = line.as_bytes();
        while fields.len() < 8 && idx < bytes.len() {
            while idx < bytes.len() && bytes[idx] == b' ' {
                idx += 1;
            }
            let start = idx;
            while idx < bytes.len() && bytes[idx] != b' ' {
                idx += 1;
            }
            fields.push(&line[start..idx]);
        }
        if fields.len() < 8 {
            continue;
        }
        let name = line[idx..].trim_start();
        let name = name.split(" -> ").next().unwrap_or(name).split(" link to ").next().unwrap_or(name);
        entries.push(Entry {
            path: name.trim_end_matches('/').to_string(),
            is_dir: fields[0].starts_with('d'),
            size: fields[4].parse().unwrap_or(0),
            packed: None,
            modified: format!("{} {} {}", fields[6], fields[5], fields[7]),
            encrypted: false,
            method: String::new(),
        });
    }
    let n = lower_name(path);
    let format = TAR_SUFFIXES.iter().filter(|s| n.ends_with(*s)).max_by_key(|s| s.len()).map(|s| s.trim_start_matches('.').to_string()).unwrap_or("tar".into());
    Ok(Archive {
        path: path.to_path_buf(),
        engine: Engine::Tar,
        format,
        entries,
        physical_size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        solid: Some(true),
        method: String::new(),
        header_encrypted: false,
        password: None,
    })
}

fn list_unar(path: &Path, password: Option<&str>) -> Result<Archive, OpenError> {
    let mut cmd = Command::new("lsar");
    cmd.arg("-j");
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        cmd.args(["-p", p]);
    }
    let (ok, out, err) = output(cmd.arg(path)).map_err(OpenError::Other)?;
    let v: Value = serde_json::from_str(&out).map_err(|_| OpenError::Other(if ok { "réponse de lsar illisible".into() } else { err.trim().to_string() }))?;
    if v.get("lsarError").and_then(Value::as_i64).is_some_and(|e| e != 0) && v.get("lsarContents").is_none() {
        return Err(OpenError::Other("illisible par unar".into()));
    }
    let entries: Vec<Entry> = v
        .get("lsarContents")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|f| {
                    let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    let n = |k: &str| f.get(k).and_then(Value::as_u64);
                    Entry {
                        path: s("XADFileName").trim_end_matches('/').to_string(),
                        is_dir: f.get("XADIsDirectory").and_then(Value::as_bool).unwrap_or(false),
                        size: n("XADFileSize").unwrap_or(0),
                        packed: n("XADCompressedSize"),
                        modified: s("XADLastModificationDate").chars().take(16).collect(),
                        encrypted: f.get("XADIsEncrypted").and_then(Value::as_bool).unwrap_or(false),
                        method: s("XADCompressionName"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if entries.is_empty() && out.contains("XADIsEncrypted") && password.is_none() {
        return Err(OpenError::Password);
    }
    Ok(Archive {
        path: path.to_path_buf(),
        engine: Engine::Unar,
        format: v.get("lsarFormatName").and_then(Value::as_str).unwrap_or("Archive").to_string(),
        entries,
        physical_size: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        solid: v.pointer("/lsarProperties/XADIsSolid").and_then(Value::as_bool),
        method: String::new(),
        header_encrypted: false,
        password: None,
    })
}

// ───────────────────────────── Tâches ─────────────────────────────

/// Avancement d'une tâche, envoyé au fil de l'interface.
#[derive(Debug, Clone)]
pub enum Progress {
    /// 0.0 à 1.0 (None : indéterminé).
    Fraction(Option<f64>),
    File(String),
    /// Fin : Ok(chemin résultat) ou Err(message lisible).
    Done(Result<PathBuf, String>),
}

/// Tâche en cours : processus annulable.
#[derive(Clone, Default)]
pub struct Job {
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Lance `cmd` ; chaque morceau de sortie passe par `parse` (pourcentage, nom de fichier).
    fn run(&self, mut cmd: Command, send: &dyn Fn(Progress), parse: &dyn Fn(&str) -> (Option<f64>, Option<String>)) -> Result<(), String> {
        cmd.env("LC_ALL", "C.UTF-8").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("lancement impossible : {e}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *self.child.lock().unwrap() = Some(child);
        // stderr lu à part (messages d'erreur, et noms de fichiers de bsdtar -v).
        let err_text = Arc::new(Mutex::new(String::new()));
        let err_thread = stderr.map(|e| {
            let err_text = err_text.clone();
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = BufReader::new(e).read_to_string(&mut s);
                *err_text.lock().unwrap() = s;
            })
        });
        if let Some(mut out) = stdout {
            // 7-Zip réécrit sa ligne de progression avec des retours arrière : on découpe sur
            // \r, \n et \x08.
            let mut buf = [0u8; 4096];
            let mut pending = String::new();
            while let Ok(n) = out.read(&mut buf) {
                if n == 0 {
                    break;
                }
                pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(i) = pending.find(['\r', '\n', '\u{8}']) {
                    let chunk: String = pending.drain(..=i).collect();
                    let chunk = chunk.trim_matches(['\r', '\n', '\u{8}', ' ']);
                    if chunk.is_empty() {
                        continue;
                    }
                    let (f, file) = parse(chunk);
                    if f.is_some() {
                        send(Progress::Fraction(f));
                    }
                    if let Some(file) = file {
                        send(Progress::File(file));
                    }
                }
            }
        }
        let status = self.child.lock().unwrap().as_mut().map(|c| c.wait());
        if let Some(t) = err_thread {
            let _ = t.join();
        }
        if self.is_cancelled() {
            return Err("Opération annulée".into());
        }
        let err = err_text.lock().unwrap().clone();
        match status {
            Some(Ok(s)) if s.success() => Ok(()),
            _ => {
                let msg = err
                    .lines()
                    .chain(err.lines())
                    .map(str::trim)
                    .find(|l| l.contains("ERROR") || l.contains("Error") || l.contains("rror:") || l.contains("Wrong password") || l.contains("Cannot"))
                    .or_else(|| err.lines().map(str::trim).rfind(|l| !l.is_empty()))
                    .unwrap_or("échec de l'outil d'archivage")
                    .to_string();
                Err(if msg.contains("Wrong password") { "Mot de passe incorrect".into() } else { msg })
            }
        }
    }
}

/// La même commande, sortie d'erreur fusionnée dans la sortie standard (bsdtar -v écrit les
/// noms de fichiers sur stderr).
fn merged(cmd: &Command) -> Command {
    let mut sh = Command::new("sh");
    if let Some(d) = cmd.get_current_dir() {
        sh.current_dir(d);
    }
    sh.arg("-c").arg("exec \"$@\" 2>&1").arg("sh").arg(cmd.get_program()).args(cmd.get_args());
    sh
}

/// Pourcentage écrit par 7-Zip (« 42% 13 - dossier/fichier »).
fn parse_7z(chunk: &str) -> (Option<f64>, Option<String>) {
    let Some(pct) = chunk.find('%') else { return (None, None) };
    let num = chunk[..pct].trim();
    let f = num.parse::<f64>().ok().map(|p| p / 100.0);
    let file = chunk[pct + 1..].split_once(" - ").or_else(|| chunk[pct + 1..].split_once(" + ")).map(|(_, f)| f.trim().to_string()).filter(|s| !s.is_empty());
    (f, file)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overwrite {
    Replace,
    Skip,
    Rename,
}

#[derive(Debug, Clone)]
pub struct ExtractOptions {
    pub dest: PathBuf,
    /// Chemins de l'archive à extraire (vide = tout).
    pub only: Vec<String>,
    pub keep_paths: bool,
    pub overwrite: Overwrite,
    pub password: Option<String>,
}

/// Extrait l'archive. `total` sert à estimer la progression (outils sans pourcentage).
pub fn extract(job: &Job, a: &Archive, o: &ExtractOptions, send: &dyn Fn(Progress)) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&o.dest).map_err(|e| format!("dossier de destination : {e}"))?;
    let total = if o.only.is_empty() { a.entries.len() } else { o.only.len().max(1) };
    let pw = o.password.clone().or_else(|| a.password.clone());
    match a.engine {
        Engine::SevenZip => {
            let mut cmd = Command::new("7z");
            cmd.arg(if o.keep_paths { "x" } else { "e" })
                .arg(&a.path)
                .arg(format!("-o{}", o.dest.display()))
                .arg(pw_arg(pw.as_deref()))
                .arg(match o.overwrite {
                    Overwrite::Replace => "-aoa",
                    Overwrite::Skip => "-aos",
                    Overwrite::Rename => "-aou",
                })
                .args(["-bsp1", "-bso0", "-bse2", "-y", "--"]);
            for p in &o.only {
                cmd.arg(p);
            }
            job.run(cmd, send, &parse_7z)?;
        }
        Engine::Tar => {
            let mut cmd = Command::new("bsdtar");
            cmd.args(["-xvf"]).arg(&a.path).arg("-C").arg(&o.dest);
            if o.overwrite == Overwrite::Skip {
                cmd.arg("-k");
            }
            if !o.keep_paths {
                // Sans arborescence : chaque chemin réduit à son nom de fichier.
                cmd.args(["-s", "|.*/||"]);
            }
            for p in &o.only {
                cmd.arg(p);
            }
            let done = std::sync::atomic::AtomicUsize::new(0);
            // « x chemin » pour chaque entrée extraite.
            job.run(merged(&cmd), send, &|chunk| {
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                (Some((n as f64 / total as f64).min(1.0)), Some(chunk.trim_start_matches("x ").to_string()))
            })?;
        }
        Engine::Unar => {
            let mut cmd = Command::new("unar");
            cmd.args(["-o"]).arg(&o.dest).arg(match o.overwrite {
                Overwrite::Replace => "-f",
                Overwrite::Skip => "-s",
                Overwrite::Rename => "-r",
            });
            cmd.arg("-D");
            if let Some(p) = pw.filter(|p| !p.is_empty()) {
                cmd.args(["-p", &p]);
            }
            cmd.arg(&a.path);
            let done = std::sync::atomic::AtomicUsize::new(0);
            job.run(cmd, send, &|chunk| {
                if chunk.ends_with("OK.") || chunk.contains("... ") {
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    (Some((n as f64 / total as f64).min(1.0)), Some(chunk.split("  ").next().unwrap_or(chunk).trim().to_string()))
                } else {
                    (None, None)
                }
            })?;
        }
    }
    send(Progress::Fraction(Some(1.0)));
    Ok(o.dest.clone())
}

/// Vérifie l'intégrité (lecture complète, sommes de contrôle).
pub fn test(job: &Job, a: &Archive, send: &dyn Fn(Progress)) -> Result<(), String> {
    match a.engine {
        Engine::SevenZip => {
            let mut cmd = Command::new("7z");
            cmd.arg("t").arg(&a.path).arg(pw_arg(a.password.as_deref())).args(["-bsp1", "-bso0", "-bse2", "-y"]);
            job.run(cmd, send, &parse_7z)
        }
        Engine::Tar => {
            let mut cmd = Command::new("bsdtar");
            cmd.arg("-tf").arg(&a.path);
            let total = a.entries.len().max(1);
            let done = std::sync::atomic::AtomicUsize::new(0);
            job.run(cmd, send, &|c| {
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                (Some((n as f64 / total as f64).min(1.0)), Some(c.to_string()))
            })
        }
        Engine::Unar => {
            let mut cmd = Command::new("lsar");
            cmd.arg("-t");
            if let Some(p) = a.password.as_deref().filter(|p| !p.is_empty()) {
                cmd.args(["-p", p]);
            }
            cmd.arg(&a.path);
            job.run(cmd, send, &|c| (None, Some(c.to_string())))
        }
    }
}

// ───────────────────────────── Création ─────────────────────────────

#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub sources: Vec<PathBuf>,
    pub output: PathBuf,
    pub format: OutFormat,
    /// 0 (stockage) à 9 (ultra).
    pub level: u32,
    pub password: Option<String>,
    pub encrypt_names: bool,
    /// Taille des volumes en Mo (0 = un seul fichier).
    pub split_mb: u64,
    pub solid: bool,
    pub excludes: Vec<String>,
    /// Nom de volume (ISO).
    pub label: String,
}

/// Dossier commun des sources (les chemins sont stockés relativement à lui).
fn common_parent(sources: &[PathBuf]) -> PathBuf {
    sources.first().and_then(|p| p.parent()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"))
}

fn names(sources: &[PathBuf]) -> Vec<String> {
    sources.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
}

/// Nombre de fichiers des sources (progression de bsdtar).
fn count_files(sources: &[PathBuf]) -> usize {
    fn walk(p: &Path, n: &mut usize) {
        *n += 1;
        if p.is_dir() && !p.is_symlink() {
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    walk(&e.path(), n);
                }
            }
        }
    }
    let mut n = 0;
    for s in sources {
        walk(s, &mut n);
    }
    n.max(1)
}

pub fn create(job: &Job, o: &CreateOptions, send: &dyn Fn(Progress)) -> Result<PathBuf, String> {
    if o.sources.is_empty() {
        return Err("aucun fichier à archiver".into());
    }
    if o.output.exists() && o.format != OutFormat::Iso {
        std::fs::remove_file(&o.output).map_err(|e| format!("remplacement de l'archive existante : {e}"))?;
    }
    let parent = common_parent(&o.sources);
    let level = o.level.min(9);
    match o.format {
        OutFormat::SevenZ | OutFormat::Zip | OutFormat::Wim => {
            let mut cmd = Command::new("7z");
            cmd.current_dir(&parent).arg("a").arg(match o.format {
                OutFormat::Zip => "-tzip",
                OutFormat::Wim => "-twim",
                _ => "-t7z",
            });
            if o.format != OutFormat::Wim {
                cmd.arg(format!("-mx={level}"));
            }
            if let Some(p) = o.password.as_deref().filter(|p| !p.is_empty()) {
                cmd.arg(format!("-p{p}"));
                match o.format {
                    OutFormat::SevenZ if o.encrypt_names => {
                        cmd.arg("-mhe=on");
                    }
                    OutFormat::Zip => {
                        cmd.arg("-mem=AES256");
                    }
                    _ => {}
                }
            }
            if o.format == OutFormat::SevenZ {
                cmd.arg(if o.solid { "-ms=on" } else { "-ms=off" });
            }
            if o.split_mb > 0 {
                cmd.arg(format!("-v{}m", o.split_mb));
            }
            for x in &o.excludes {
                cmd.arg(format!("-xr!{x}"));
            }
            cmd.args(["-bsp1", "-bso0", "-bse2", "-y", "--"]).arg(&o.output).args(names(&o.sources));
            job.run(cmd, send, &parse_7z)?;
        }
        OutFormat::Iso => {
            let mut cmd = Command::new("xorriso");
            let label: String = o.label.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(32).collect::<String>().to_uppercase();
            cmd.args(["-as", "mkisofs", "-r", "-J", "-joliet-long", "-V", if label.is_empty() { "NOOS" } else { &label }, "-o"]).arg(&o.output);
            for x in &o.excludes {
                cmd.args(["-m", x]);
            }
            cmd.arg("-graft-points");
            for s in &o.sources {
                let name = s.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                // Un seul dossier : son contenu à la racine de l'image.
                if o.sources.len() == 1 && s.is_dir() {
                    cmd.arg(format!("/={}", s.display()));
                } else {
                    cmd.arg(format!("/{name}={}", s.display()));
                }
            }
            job.run(cmd, send, &|chunk| {
                // « xorriso : UPDATE :  41.27% done »
                chunk.find("% done").and_then(|i| chunk[..i].rsplit(' ').next()?.parse::<f64>().ok()).map(|p| (Some(p / 100.0), None)).unwrap_or((None, None))
            })?;
        }
        tar => {
            let mut cmd = Command::new("bsdtar");
            cmd.args(["-c", "-v", "--format", "pax", "-f"]).arg(&o.output);
            let (flag, opt) = match tar {
                OutFormat::TarGz => ("-z", Some(format!("gzip:compression-level={}", level.max(1)))),
                OutFormat::TarBz2 => ("-j", Some(format!("bzip2:compression-level={}", level.max(1)))),
                OutFormat::TarXz => ("-J", Some(format!("xz:compression-level={level}"))),
                // zstd : 1 à 19 (on étale 0–9 sur 1–19).
                OutFormat::TarZst => ("--zstd", Some(format!("zstd:compression-level={}", (level * 2).clamp(1, 19)))),
                OutFormat::TarLz4 => ("--lz4", Some(format!("lz4:compression-level={}", level.max(1)))),
                OutFormat::TarLz => ("--lzip", Some(format!("lzip:compression-level={level}"))),
                _ => ("", None),
            };
            if !flag.is_empty() {
                cmd.arg(flag);
            }
            if let Some(opt) = opt {
                cmd.args(["--options", &opt]);
            }
            for x in &o.excludes {
                cmd.args(["--exclude", x]);
            }
            cmd.arg("-C").arg(&parent).args(names(&o.sources));
            let total = count_files(&o.sources);
            let done = std::sync::atomic::AtomicUsize::new(0);
            // « a chemin » pour chaque entrée ajoutée.
            job.run(merged(&cmd), send, &|chunk| {
                if let Some(f) = chunk.strip_prefix("a ") {
                    let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    (Some((n as f64 / total as f64).min(0.99)), Some(f.to_string()))
                } else {
                    (None, None)
                }
            })?;
        }
    }
    send(Progress::Fraction(Some(1.0)));
    // Volumes : le premier fichier (archive.7z.001).
    let first = if o.split_mb > 0 && o.format.supports_split() { PathBuf::from(format!("{}.001", o.output.display())) } else { o.output.clone() };
    Ok(if first.exists() { first } else { o.output.clone() })
}

/// Ajoute des fichiers à une archive modifiable (7z, zip, tar, wim).
pub fn add(job: &Job, a: &Archive, files: &[PathBuf], into: &str, send: &dyn Fn(Progress)) -> Result<(), String> {
    if !a.writable() {
        return Err(format!("les archives {} ne peuvent pas être modifiées : convertissez-la d'abord", a.format));
    }
    // 7-Zip range les fichiers selon leur chemin relatif : on passe par un dossier temporaire
    // reproduisant le dossier de destination dans l'archive.
    let tmp = std::env::temp_dir().join(format!("noos-archive-add-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let target = if into.is_empty() { tmp.clone() } else { tmp.join(into) };
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    for f in files {
        let name = f.file_name().ok_or("nom invalide")?;
        // Lien symbolique : rien n'est copié, 7-Zip suit le lien (-l) et lit l'original.
        std::os::unix::fs::symlink(f, target.join(name)).map_err(|e| e.to_string())?;
    }
    let top = if into.is_empty() { names(files) } else { vec![into.split('/').next().unwrap_or(into).to_string()] };
    let mut cmd = Command::new("7z");
    cmd.current_dir(&tmp).arg("a").arg(&a.path).arg("-l").arg(pw_arg(a.password.as_deref())).args(["-bsp1", "-bso0", "-bse2", "-y", "--"]).args(top);
    let res = job.run(cmd, send, &parse_7z);
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// Supprime des éléments d'une archive modifiable.
pub fn delete(job: &Job, a: &Archive, paths: &[String], send: &dyn Fn(Progress)) -> Result<(), String> {
    if !a.writable() {
        return Err(format!("les archives {} ne peuvent pas être modifiées", a.format));
    }
    let mut cmd = Command::new("7z");
    cmd.arg("d").arg(&a.path).arg(pw_arg(a.password.as_deref())).args(["-r", "-bsp1", "-bso0", "-bse2", "-y", "--"]).args(paths);
    job.run(cmd, send, &parse_7z)
}

/// Convertit vers un autre format (extraction dans un dossier temporaire, puis création).
pub fn convert(job: &Job, a: &Archive, o: &CreateOptions, send: &dyn Fn(Progress)) -> Result<PathBuf, String> {
    let tmp = std::env::temp_dir().join(format!("noos-archive-convert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let half = |p: Progress| match p {
        Progress::Fraction(Some(f)) => send(Progress::Fraction(Some(f * 0.5))),
        other => send(other),
    };
    extract(job, a, &ExtractOptions { dest: tmp.clone(), only: vec![], keep_paths: true, overwrite: Overwrite::Replace, password: a.password.clone() }, &half)?;
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    sources.sort();
    let second = |p: Progress| match p {
        Progress::Fraction(Some(f)) => send(Progress::Fraction(Some(0.5 + f * 0.5))),
        other => send(other),
    };
    let res = create(job, &CreateOptions { sources, ..o.clone() }, &second);
    let _ = std::fs::remove_dir_all(&tmp);
    res
}

/// Destination « intelligente » pour « Extraire ici » : un sous-dossier au nom de l'archive,
/// sauf si l'archive contient déjà un unique dossier à sa racine.
pub fn smart_destination(a: &Archive, base: &Path) -> PathBuf {
    let roots: std::collections::HashSet<&str> = a.entries.iter().map(|e| e.path.split('/').next().unwrap_or("")).collect();
    let single_dir = roots.len() == 1 && a.entries.iter().any(|e| e.path.contains('/') || e.is_dir);
    if single_dir {
        return base.to_path_buf();
    }
    let mut d = base.join(stem(&a.path));
    let mut i = 2;
    while d.exists() {
        d = base.join(format!("{} ({i})", stem(&a.path)));
        i += 1;
    }
    d
}

/// Outils présents (fonctionnalités disponibles).
pub fn tool_available(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems() {
        assert_eq!(stem(Path::new("/a/photos.tar.gz")), "photos");
        assert_eq!(stem(Path::new("x.7z.001")), "x");
        assert_eq!(stem(Path::new("doc.zip")), "doc");
        assert!(is_archive(Path::new("a.TAR.ZST")));
        assert!(!is_archive(Path::new("a.txt")));
    }

    #[test]
    fn progress_7z() {
        assert_eq!(parse_7z("42% 13 - dossier/fichier.txt"), (Some(0.42), Some("dossier/fichier.txt".into())));
        assert_eq!(parse_7z("  7%"), (Some(0.07), None));
    }

    #[test]
    fn children() {
        let a = Archive {
            path: "x.zip".into(),
            engine: Engine::SevenZip,
            format: "zip".into(),
            entries: vec![
                Entry { path: "a/b.txt".into(), size: 5, ..Default::default() },
                Entry { path: "a/c/d.txt".into(), size: 7, ..Default::default() },
                Entry { path: "e.txt".into(), size: 1, ..Default::default() },
            ],
            physical_size: 0,
            solid: None,
            method: String::new(),
            header_encrypted: false,
            password: None,
        };
        let root = a.children("");
        assert_eq!(root.len(), 2);
        assert!(root[0].is_dir && root[0].name() == "a" && root[0].size == 12);
        assert_eq!(a.children("a").len(), 2);
    }

    /// Cycle réel (outils installés) : `cargo test -- --ignored cycle`.
    #[test]
    #[ignore]
    fn cycle() {
        let dir = std::env::temp_dir().join("noos-archive-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/sous")).unwrap();
        std::fs::write(dir.join("src/a.txt"), "bonjour").unwrap();
        std::fs::write(dir.join("src/sous/b.md"), "# titre").unwrap();
        let job = Job::default();
        for f in [OutFormat::SevenZ, OutFormat::Zip, OutFormat::TarZst, OutFormat::TarXz, OutFormat::TarGz, OutFormat::TarLz4, OutFormat::TarBz2, OutFormat::Iso] {
            let out = dir.join(format!("t.{}", f.ext()));
            let o = CreateOptions {
                sources: vec![dir.join("src")],
                output: out.clone(),
                format: f,
                level: 5,
                password: if f == OutFormat::SevenZ { Some("secret".into()) } else { None },
                encrypt_names: true,
                split_mb: 0,
                solid: true,
                excludes: vec![],
                label: "TEST".into(),
            };
            let made = create(&job, &o, &|_| {}).unwrap_or_else(|e| panic!("{f:?} : {e}"));
            let opened = open(&made, None);
            let a = if f == OutFormat::SevenZ {
                assert_eq!(opened.unwrap_err(), OpenError::Password, "7z chiffré sans mot de passe");
                open(&made, Some("secret")).unwrap()
            } else {
                opened.unwrap_or_else(|e| panic!("{f:?} : {e:?}"))
            };
            assert!(a.files() >= 2, "{f:?} : {} fichiers", a.files());
            let dest = dir.join(format!("x-{}", f.ext()));
            extract(&job, &a, &ExtractOptions { dest: dest.clone(), only: vec![], keep_paths: true, overwrite: Overwrite::Replace, password: a.password.clone() }, &|_| {}).unwrap();
            let found = walkdir_has(&dest, "b.md");
            assert!(found, "{f:?} : b.md extrait");
            println!("{f:?} : {} ({} fichiers) ok", a.format, a.files());
        }
    }

    fn walkdir_has(d: &Path, name: &str) -> bool {
        std::fs::read_dir(d).map(|rd| rd.flatten().any(|e| e.file_name() == name || (e.path().is_dir() && walkdir_has(&e.path(), name)))).unwrap_or(false)
    }
}

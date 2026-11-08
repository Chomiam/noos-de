//! Client Ollama local (HTTP par `curl`, pas de pile HTTP embarquée).
//!
//! On réutilise un `ollama serve` déjà lancé (service système sur 11434) ; sinon le démon
//! lance le sien, pour l'utilisateur, sur un port à part avec ses modèles dans
//! `~/.local/share/noos/ai/models`, et l'arrête en quittant. Le modèle est déchargé de la
//! mémoire après `[ai] unload_minutes` (`keep_alive`).

use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const SYSTEM_URL: &str = "http://127.0.0.1:11434";
const OWN_URL: &str = "http://127.0.0.1:11535";

pub fn models_dir() -> std::path::PathBuf {
    noos_core::paths::xdg_data_home().join("noos/ai/models")
}

fn curl(url: &str, body: Option<&Value>, timeout_s: u32) -> Result<Value, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-f", "-m", &timeout_s.to_string(), url]);
    if body.is_some() {
        cmd.args(["-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-"]);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("curl : {e}"))?;
    if let Some(b) = body {
        let _ = child.stdin.take().map(|mut s| s.write_all(b.to_string().as_bytes()));
    } else {
        drop(child.stdin.take());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("le moteur local ne répond pas (code {})", out.status.code().unwrap_or(-1)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("réponse illisible : {e}"))
}

fn alive(url: &str) -> bool {
    curl(&format!("{url}/api/version"), None, 2).is_ok()
}

/// Progression d'un téléchargement de modèle.
#[derive(Debug, Clone, Default)]
pub struct Pull {
    pub model: String,
    pub status: String,
    pub completed: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Ollama {
    url: Mutex<Option<String>>,
    child: Mutex<Option<Child>>,
    pub pull: Arc<Mutex<Option<Pull>>>,
}

impl Ollama {
    /// Adresse d'un serveur prêt (le lance au besoin, ≈ 1 à 3 s).
    pub fn ensure(&self, use_gpu: bool) -> Result<String, String> {
        if let Some(u) = self.url.lock().unwrap().clone() {
            if alive(&u) {
                return Ok(u);
            }
        }
        for u in [SYSTEM_URL, OWN_URL] {
            if alive(u) {
                *self.url.lock().unwrap() = Some(u.into());
                return Ok(u.into());
            }
        }
        if !noos_core::ai::detect().ollama {
            return Err("Le moteur d'IA (ollama) n'est pas installé.".into());
        }
        let dir = models_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut cmd = Command::new("ollama");
        cmd.arg("serve")
            .env("OLLAMA_HOST", OWN_URL.trim_start_matches("http://"))
            .env("OLLAMA_MODELS", &dir)
            .env("OLLAMA_NOPRUNE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if use_gpu {
            // Moteur Vulkan (paquet ollama-vulkan) : AMD, Intel, NVIDIA.
            cmd.env("OLLAMA_VULKAN", "1");
        }
        let child = cmd.spawn().map_err(|e| format!("ollama : {e}"))?;
        *self.child.lock().unwrap() = Some(child);
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(15) {
            if alive(OWN_URL) {
                *self.url.lock().unwrap() = Some(OWN_URL.into());
                return Ok(OWN_URL.into());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("Le moteur d'IA ne démarre pas.".into())
    }

    /// Modèles présents (noms Ollama).
    pub fn models(&self, use_gpu: bool) -> Result<Vec<(String, u64)>, String> {
        let url = self.ensure(use_gpu)?;
        let v = curl(&format!("{url}/api/tags"), None, 5)?;
        Ok(v["models"].as_array().into_iter().flatten().map(|m| (m["name"].as_str().unwrap_or("").to_string(), m["size"].as_u64().unwrap_or(0))).collect())
    }

    pub fn has_model(&self, model: &str, use_gpu: bool) -> bool {
        self.models(use_gpu).is_ok_and(|m| m.iter().any(|(n, _)| n == model || n == &format!("{model}:latest")))
    }

    pub fn delete(&self, model: &str, use_gpu: bool) -> Result<(), String> {
        let url = self.ensure(use_gpu)?;
        let st = Command::new("curl")
            .args(["-s", "-f", "-m", "30", "-X", "DELETE", &format!("{url}/api/delete"), "-d", &json!({"model": model}).to_string()])
            .status()
            .map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("suppression impossible".into()) }
    }

    /// Lance le téléchargement d'un modèle en arrière-plan (suivi par `self.pull`).
    pub fn start_pull(self: &Arc<Self>, model: &str, use_gpu: bool) -> Result<(), String> {
        if self.pull.lock().unwrap().as_ref().is_some_and(|p| !p.done) {
            return Err("Un téléchargement est déjà en cours.".into());
        }
        let url = self.ensure(use_gpu)?;
        *self.pull.lock().unwrap() = Some(Pull { model: model.into(), status: "Préparation…".into(), ..Default::default() });
        let (state, model) = (self.pull.clone(), model.to_string());
        std::thread::spawn(move || {
            let set = |f: &dyn Fn(&mut Pull)| {
                if let Some(p) = state.lock().unwrap().as_mut() {
                    f(p);
                }
            };
            let child = Command::new("curl")
                .args(["-s", "-N", "-X", "POST", &format!("{url}/api/pull"), "-d", &json!({"model": model, "stream": true}).to_string()])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut child = match child {
                Ok(c) => c,
                Err(e) => return set(&|p| (p.error, p.done) = (Some(e.to_string()), true)),
            };
            let mut ok = false;
            for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(e) = v["error"].as_str() {
                    let e = e.to_string();
                    set(&|p| p.error = Some(e.clone()));
                    break;
                }
                let status = v["status"].as_str().unwrap_or("").to_string();
                ok |= status == "success";
                let (c, t) = (v["completed"].as_u64(), v["total"].as_u64());
                set(&|p| {
                    p.status = status.clone();
                    if let (Some(c), Some(t)) = (c, t) {
                        (p.completed, p.total) = (c, t);
                    }
                });
            }
            let _ = child.wait();
            set(&|p| {
                p.done = true;
                if !ok && p.error.is_none() {
                    p.error = Some("téléchargement interrompu".into());
                }
            });
        });
        Ok(())
    }

    /// Une requête de conversation avec outils (non diffusée en continu).
    pub fn chat(&self, model: &str, messages: &[Value], tools: &[Value], use_gpu: bool, keep_alive_min: u32) -> Result<Value, String> {
        let url = self.ensure(use_gpu)?;
        let mut body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "keep_alive": format!("{keep_alive_min}m"),
            "options": {"temperature": 0.2, "num_ctx": 4096},
        });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if !use_gpu {
            body["options"]["num_gpu"] = json!(0);
        }
        let v = curl(&format!("{url}/api/chat"), Some(&body), 180)?;
        Ok(v["message"].clone())
    }

    /// Décharge tout de suite le modèle (mémoire libérée).
    pub fn unload(&self, model: &str) {
        if let Some(u) = self.url.lock().unwrap().clone() {
            let _ = curl(&format!("{u}/api/generate"), Some(&json!({"model": model, "keep_alive": 0})), 5);
        }
    }

    pub fn stop(&self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

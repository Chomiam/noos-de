//! Client du protocole IPC de greetd (`$GREETD_SOCK`) : messages JSON précédés de leur
//! longueur (u32, ordre natif).
//!
//! Déroulé : `create_session` → (questions PAM : `auth_message` → réponse)* → `success`
//! → `start_session` → `success` → le greeter se termine et greetd lance la session.

use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    CreateSession { username: String },
    PostAuthMessageResponse { response: Option<String> },
    StartSession { cmd: Vec<String>, env: Vec<String> },
    CancelSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Visible,
    Secret,
    Info,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Success,
    Error { error_type: String, description: String },
    AuthMessage { auth_message_type: AuthKind, auth_message: String },
}

pub struct Greetd {
    stream: UnixStream,
}

impl Greetd {
    /// None hors de greetd (aperçu / démonstration).
    pub fn connect() -> Option<anyhow::Result<Self>> {
        let path = std::env::var_os("GREETD_SOCK")?;
        Some(UnixStream::connect(&path).map(|stream| Self { stream }).context("connexion à greetd"))
    }

    pub fn call(&mut self, req: &Request) -> anyhow::Result<Response> {
        let body = serde_json::to_vec(req)?;
        self.stream.write_all(&(body.len() as u32).to_ne_bytes())?;
        self.stream.write_all(&body)?;
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).context("greetd ne répond plus")?;
        let len = u32::from_ne_bytes(len) as usize;
        if len > 1 << 20 {
            bail!("réponse de greetd trop longue");
        }
        let mut buf = vec![0; len];
        self.stream.read_exact(&mut buf)?;
        serde_json::from_slice(&buf).context("réponse de greetd illisible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let r = serde_json::to_string(&Request::PostAuthMessageResponse { response: Some("x".into()) }).unwrap();
        assert_eq!(r, r#"{"type":"post_auth_message_response","response":"x"}"#);
        let r = serde_json::to_string(&Request::CancelSession).unwrap();
        assert_eq!(r, r#"{"type":"cancel_session"}"#);
        let a: Response =
            serde_json::from_str(r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: "}"#).unwrap();
        assert!(matches!(a, Response::AuthMessage { auth_message_type: AuthKind::Secret, .. }));
        let e: Response = serde_json::from_str(r#"{"type":"error","error_type":"auth_error","description":"x"}"#).unwrap();
        assert!(matches!(e, Response::Error { .. }));
    }
}

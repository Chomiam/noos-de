//! Animations GLSL au format Shadertoy (`mainImage(out vec4, in vec2)`) pour NOOS.
//!
//! - [`view::ShaderView`] : rendu dans un `GtkGLArea` (uniformes Shadertoy, images par
//!   seconde plafonnées, rendu à résolution réduite) ;
//! - [`builtin`] : animations NOOS intégrées ;
//! - [`store`] : bibliothèque locale des animations téléchargées ;
//! - [`api`] : API Shadertoy (recherche, téléchargement ; clé personnelle de l'utilisateur).
//!
//! Une animation se désigne par une chaîne : `builtin:<id>` ou `shadertoy:<id>`.

pub mod api;
pub mod builtin;
pub mod store;
pub mod view;

/// Code d'une animation prête à compiler.
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub author: String,
    pub image: String,
    pub common: String,
}

/// Résout `builtin:<id>` / `shadertoy:<id>`.
pub fn resolve(spec: &str) -> Option<Source> {
    if let Some(id) = spec.strip_prefix("builtin:") {
        let b = builtin::get(id)?;
        return Some(Source { name: b.name.into(), author: "NOOS".into(), image: b.code.into(), common: String::new() });
    }
    if let Some(id) = spec.strip_prefix("shadertoy:") {
        let s = store::get(id)?;
        return Some(Source { name: s.name, author: s.author, image: s.image, common: s.common });
    }
    None
}

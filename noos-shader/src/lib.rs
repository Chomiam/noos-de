//! Animations GLSL pour NOOS (écran de verrouillage, écran de veille), au format Shadertoy
//! (`mainImage(out vec4, in vec2)`) ; le format GLSL Sandbox (`main` + `gl_FragColor`) est
//! converti à l'import.
//!
//! - [`view::ShaderView`] : rendu dans un `GtkGLArea` (uniformes Shadertoy, images par
//!   seconde plafonnées, rendu à résolution réduite) ;
//! - [`builtin`] : animations NOOS intégrées ;
//! - [`catalog`] : catalogue d'animations NOOS hébergé dans le dépôt GitHub (sans compte ni clé) ;
//! - [`store`] : bibliothèque locale (animations du catalogue et importées).
//!
//! Une animation se désigne par une chaîne : `builtin:<id>` ou `library:<id>`
//! (`shadertoy:<id>` : ancien nom, toujours lu).

pub mod builtin;
pub mod catalog;
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

/// Résout `builtin:<id>` / `library:<id>`.
pub fn resolve(spec: &str) -> Option<Source> {
    if let Some(id) = spec.strip_prefix("builtin:") {
        let b = builtin::get(id)?;
        return Some(Source { name: b.name.into(), author: "NOOS".into(), image: b.code.into(), common: String::new() });
    }
    if let Some(id) = spec.strip_prefix("library:").or_else(|| spec.strip_prefix("shadertoy:")) {
        let s = store::get(id)?;
        return Some(Source { name: s.name, author: s.author, image: s.image, common: s.common });
    }
    None
}

/// Convertit du code GLSL Sandbox (`void main()`, `gl_FragColor`, uniformes `time`,
/// `resolution`, `mouse`) au format `mainImage` ; le code Shadertoy est rendu tel quel.
pub fn adapt(code: &str) -> String {
    if code.contains("mainImage") {
        return code.to_string();
    }
    let skip = |l: &str| {
        let t = l.trim();
        t.starts_with("#extension")
            || t.starts_with("#version")
            || t.starts_with("varying vec2 surfacePosition")
            || (t.starts_with("uniform")
                && ["time", "resolution", "mouse", "backbuffer", "surfaceSize"].iter().any(|n| t.trim_end_matches(';').split_whitespace().last() == Some(*n)))
    };
    let body: String = code.lines().filter(|l| !skip(l)).collect::<Vec<_>>().join("\n");
    let body = body.replace("gl_FragColor", "noos_glFragColor");
    // `void main()` / `void main( void )` → fonction ordinaire.
    let mut out = String::new();
    let mut rest = body.as_str();
    while let Some(i) = rest.find("void main") {
        let after = &rest[i + 9..];
        if after.trim_start().starts_with('(') {
            out.push_str(&rest[..i]);
            out.push_str("void noos_sandbox_main");
            rest = after;
        } else {
            out.push_str(&rest[..i + 9]);
            rest = after;
        }
    }
    out.push_str(rest);
    let out = out.replace("noos_sandbox_main( void )", "noos_sandbox_main()").replace("noos_sandbox_main(void)", "noos_sandbox_main()");
    format!(
        "#define time iTime\n#define resolution iResolution.xy\n#define mouse (iMouse.xy / max(iResolution.xy, vec2(1.0)))\n\
#define surfacePosition ((gl_FragCoord.xy - 0.5 * iResolution.xy) / iResolution.y)\n#define texture2D texture\n\
vec4 noos_glFragColor;\n{out}\n\
void mainImage(out vec4 noos_c, in vec2 noos_f) {{ noos_glFragColor = vec4(0.0, 0.0, 0.0, 1.0); noos_sandbox_main(); noos_c = noos_glFragColor; }}\n"
    )
}

/// Le code ressemble-t-il à une animation utilisable ?
pub fn looks_valid(code: &str) -> bool {
    code.contains("mainImage") || (code.contains("void main") && code.contains("gl_FragColor"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn sandbox() {
        let code = "#ifdef GL_ES\nprecision mediump float;\n#endif\n#extension GL_OES_standard_derivatives : enable\nuniform float time;\nuniform vec2 resolution;\nvoid main( void ) {\n vec2 p = gl_FragCoord.xy / resolution;\n gl_FragColor = vec4(p, sin(time), 1.0);\n}\n";
        let a = super::adapt(code);
        assert!(a.contains("void noos_sandbox_main()"));
        assert!(!a.contains("uniform float time"));
        assert!(a.contains("noos_glFragColor = vec4(p"));
        assert!(!a.contains("#extension"));
        assert!(super::looks_valid(code));
        let st = "void mainImage(out vec4 c, in vec2 f) { c = vec4(1.0); }";
        assert_eq!(super::adapt(st), st);
    }
}

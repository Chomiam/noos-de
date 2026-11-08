//! Animations NOOS intégrées (créées pour NOOS, au format Shadertoy `mainImage`).

pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub code: &'static str,
}

pub const BUILTINS: &[Builtin] = &[
    Builtin {
        id: "aurora",
        name: "Aurore",
        description: "Rubans d'aurore boréale sur un ciel étoilé",
        code: r#"
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    for (int i = 0; i < 5; i++) { v += a * noise(p); p *= 2.03; a *= 0.5; }
    return v;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * 0.06;
    vec3 sky = mix(vec3(0.015, 0.02, 0.05), vec3(0.04, 0.05, 0.12), uv.y);
    // Étoiles scintillantes.
    vec2 g = floor(fragCoord / 3.0);
    float s = hash(g);
    sky += vec3(smoothstep(0.996, 1.0, s) * (0.6 + 0.4 * sin(iTime * 2.0 + s * 60.0)));
    // Rubans.
    vec3 col = vec3(0.0);
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float y = 0.15 + 0.12 * fi + 0.12 * fbm(vec2(p.x * 1.4 + t * (1.0 + fi * 0.4), fi * 3.1 + t));
        float d = abs(p.y - y);
        float band = exp(-d * (18.0 - fi * 4.0)) * (0.55 + 0.45 * fbm(vec2(p.x * 6.0 - t * 4.0, fi)));
        float curtain = smoothstep(y - 0.02, y + 0.35, p.y) * smoothstep(y + 0.5, y, p.y) * 0.25;
        vec3 c = mix(vec3(0.20, 0.95, 0.65), vec3(0.48, 0.65, 1.0), fi / 2.0);
        c = mix(c, vec3(0.78, 0.6, 1.0), smoothstep(0.1, 0.5, p.y - y));
        col += c * (band + curtain * fbm(vec2(p.x * 3.0 + t * 2.0, p.y * 2.0)));
    }
    vec3 rgb = sky + col * 0.9;
    rgb *= 1.0 - 0.35 * length(uv - 0.5);
    fragColor = vec4(pow(rgb, vec3(0.9)), 1.0);
}
"#,
    },
    Builtin {
        id: "plasma",
        name: "Plasma",
        description: "Plasma doux aux couleurs de NOOS",
        code: r#"
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y * 3.0;
    float t = iTime * 0.25;
    float v = sin(p.x + t) + sin(p.y * 1.3 - t * 1.2) + sin((p.x + p.y) * 0.7 + t * 0.8);
    v += sin(length(p + vec2(sin(t * 0.7), cos(t * 0.9)) * 1.5) * 2.0 - t * 2.0);
    v *= 0.25;
    vec3 a = vec3(0.48, 0.65, 1.0);
    vec3 b = vec3(0.78, 0.61, 1.0);
    vec3 c = vec3(0.95, 0.55, 0.66);
    vec3 col = mix(a, b, 0.5 + 0.5 * sin(v * 3.14159 + 0.0));
    col = mix(col, c, 0.5 + 0.5 * sin(v * 3.14159 + 2.1));
    col *= 0.35 + 0.25 * sin(v * 6.2831 + t);
    fragColor = vec4(col, 1.0);
}
"#,
    },
    Builtin {
        id: "nebula",
        name: "Nébuleuse",
        description: "Champ d'étoiles en mouvement à travers une nébuleuse",
        code: r#"
float hash(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1, 0)), u.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), u.x), u.y);
}
float fbm(vec2 p) { float v = 0.0, a = 0.5; for (int i = 0; i < 6; i++) { v += a * noise(p); p *= 2.0; a *= 0.5; } return v; }
vec3 stars(vec2 p, float scale, float speed) {
    p *= scale;
    p.x += iTime * speed;
    vec2 g = floor(p), f = fract(p) - 0.5;
    float h = hash(g);
    vec2 o = vec2(hash(g + 1.7), hash(g + 3.3)) - 0.5;
    float d = length(f - o * 0.7);
    float star = smoothstep(0.06, 0.0, d) * step(0.82, h);
    return vec3(star) * mix(vec3(0.7, 0.8, 1.0), vec3(1.0, 0.85, 0.9), hash(g + 9.1));
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * 0.02;
    float n = fbm(p * 2.2 + vec2(t, -t * 0.6));
    float m = fbm(p * 4.0 - vec2(t * 1.3, t) + n);
    vec3 neb = mix(vec3(0.05, 0.02, 0.12), vec3(0.35, 0.18, 0.55), m);
    neb = mix(neb, vec3(0.15, 0.35, 0.65), smoothstep(0.45, 0.8, n));
    neb *= smoothstep(0.25, 0.85, n * m * 1.8);
    vec3 col = vec3(0.01, 0.01, 0.03) + neb * 0.8;
    col += stars(p, 18.0, 0.02) + stars(p, 34.0, 0.05) * 0.7 + stars(p, 60.0, 0.1) * 0.5;
    fragColor = vec4(col, 1.0);
}
"#,
    },
    Builtin {
        id: "infinity",
        name: "Infini",
        description: "Le symbole NOOS, lumineux, incliné à 45°",
        code: r#"
vec2 lemniscate(float t) {
    float s = sin(t), c = cos(t);
    float d = 1.0 + s * s;
    vec2 q = vec2(c, s * c) / d;
    float a = 0.7853981;
    return mat2(cos(a), sin(a), -sin(a), cos(a)) * q;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y * 2.6;
    float glow = 0.0;
    float hue = 0.0;
    for (int i = 0; i < 260; i++) {
        float t = float(i) / 260.0 * 6.2831853;
        vec2 q = lemniscate(t + iTime * 0.15);
        float d = length(p - q);
        float w = 0.0009 / (d * d + 0.00045);
        glow += w;
        hue += w * (0.5 + 0.5 * sin(t + iTime * 0.6));
    }
    hue /= max(glow, 1e-4);
    vec3 blue = vec3(0.48, 0.65, 1.0), violet = vec3(0.78, 0.61, 1.0), pink = vec3(0.95, 0.55, 0.66);
    vec3 c = mix(mix(blue, violet, smoothstep(0.0, 0.5, hue)), pink, smoothstep(0.5, 1.0, hue));
    vec3 col = c * glow * 0.042;
    col += vec3(0.02, 0.02, 0.05) + c * 0.06 * exp(-length(p) * 1.2);
    col = 1.0 - exp(-col * 1.6);
    fragColor = vec4(col, 1.0);
}
"#,
    },
    Builtin {
        id: "waves",
        name: "Vagues",
        description: "Lignes ondulantes apaisantes",
        code: r#"
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec3 col = mix(vec3(0.04, 0.05, 0.10), vec3(0.08, 0.06, 0.16), uv.y);
    for (int i = 0; i < 14; i++) {
        float fi = float(i);
        float y = 0.12 + fi * 0.055 + 0.03 * sin(uv.x * (5.0 + fi * 0.4) + iTime * (0.35 + fi * 0.03) + fi)
                + 0.015 * sin(uv.x * 13.0 - iTime * 0.6 + fi * 2.0);
        float d = abs(uv.y - y);
        float line = smoothstep(0.004, 0.0, d) * 0.9 + exp(-d * 90.0) * 0.25;
        vec3 c = mix(vec3(0.48, 0.65, 1.0), vec3(0.95, 0.55, 0.66), fi / 13.0);
        col += c * line * (0.4 + 0.6 * uv.x);
    }
    fragColor = vec4(col, 1.0);
}
"#,
    },
];

pub fn get(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.id == id)
}

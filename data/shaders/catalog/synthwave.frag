// name: Horizon rétro
// description: Soleil couchant à rayures, montagnes et grille néon qui défile
// tags: néon, rétro, paysage
// author: NOOS
// license: CC0-1.0
float h2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(float x) { float i = floor(x), f = fract(x); return mix(h2(vec2(i, 0.0)), h2(vec2(i + 1.0, 0.0)), f * f * (3.0 - 2.0 * f)); }
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float hz = -0.05;
    vec3 col;
    if (p.y > hz) {
        float y = p.y - hz;
        col = mix(vec3(0.95, 0.35, 0.55), vec3(0.08, 0.03, 0.2), smoothstep(0.0, 0.5, y));
        vec2 g = floor(fragCoord / 2.0);
        col += vec3(step(0.997, h2(g))) * smoothstep(0.15, 0.4, y);
        // Soleil.
        vec2 sp = p - vec2(0.0, hz + 0.16);
        float sun = smoothstep(0.205, 0.2, length(sp));
        float stripes = step(0.0, sin((sp.y + iTime * 0.02) * 90.0) + smoothstep(-0.2, 0.15, sp.y) * 2.2 - 1.0);
        vec3 sc = mix(vec3(1.0, 0.25, 0.55), vec3(1.0, 0.88, 0.3), smoothstep(-0.2, 0.2, sp.y));
        col = mix(col, sc, sun * stripes);
        col += vec3(1.0, 0.4, 0.6) * 0.25 * exp(-length(sp) * 4.0);
        // Montagnes.
        float m = 0.05 + 0.09 * noise(p.x * 6.0 + 3.0) + 0.04 * noise(p.x * 17.0);
        float mm = 0.03 + 0.05 * noise(p.x * 9.0 + 11.0);
        col = mix(col, vec3(0.1, 0.03, 0.2), step(y, m));
        col = mix(col, vec3(0.05, 0.01, 0.1), step(y, mm));
    } else {
        float d = hz - p.y;
        float z = 0.25 / d;
        vec2 g = vec2(p.x * z, z + iTime * 1.6);
        vec2 f = abs(fract(g) - 0.5);
        float w = fwidth(g.y) * 1.5 + 0.01;
        float line = smoothstep(w, 0.0, 0.5 - f.y) + smoothstep(fwidth(g.x) * 1.5 + 0.01, 0.0, 0.5 - f.x);
        col = vec3(0.04, 0.0, 0.08) + vec3(1.0, 0.25, 0.85) * min(line, 1.0) * exp(-z * 0.08);
        col += vec3(0.9, 0.2, 0.6) * 0.3 * exp(-d * 18.0);
    }
    fragColor = vec4(col, 1.0);
}

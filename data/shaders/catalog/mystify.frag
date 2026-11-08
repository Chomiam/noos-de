// name: Mystique
// description: Deux polygones lumineux rebondissent et laissent des échos colorés
// tags: lignes, rétro, néon
// homage: Écran de veille « Mystify Your Mind » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float tri(float x) { return abs(fract(x) - 0.5) * 2.0; }
float h1(float n) { return fract(sin(n) * 43758.5453123); }
float seg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float k = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * k);
}
vec3 hue(float h) { return clamp(abs(mod(h * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0, 0.0, 1.0); }
vec2 vert(int poly, int k, float t, vec2 box) {
    float s = float(poly * 4 + k);
    vec2 sp = vec2(0.045 + 0.05 * h1(s * 3.7), 0.05 + 0.05 * h1(s * 5.1));
    vec2 ph = vec2(h1(s * 7.3), h1(s * 1.9));
    return (vec2(tri(t * sp.x + ph.x), tri(t * sp.y + ph.y)) - 0.5) * 2.0 * box;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 box = vec2(0.5 * iResolution.x / iResolution.y, 0.5) * 0.94;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float px = 1.0 / iResolution.y;
    vec3 col = vec3(0.0);
    for (int poly = 0; poly < 2; poly++) {
        vec3 c = hue(fract(iTime * 0.03 + float(poly) * 0.45));
        for (int e = 0; e < 6; e++) {
            float t = iTime - float(e) * 0.55;
            float fade = 1.0 - float(e) / 6.0;
            for (int k = 0; k < 4; k++) {
                vec2 a = vert(poly, k, t, box);
                vec2 b = vert(poly, (k + 1) % 4, t, box);
                float d = seg(p, a, b);
                col += c * fade * (smoothstep(px * 1.6, 0.0, d) + 0.18 * smoothstep(px * 7.0, 0.0, d));
            }
        }
    }
    fragColor = vec4(min(col, vec3(1.0)), 1.0);
}

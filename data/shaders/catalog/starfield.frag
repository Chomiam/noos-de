// name: Champ d'étoiles
// description: Voyage à travers les étoiles, avec traînées de vitesse
// tags: espace, rétro
// homage: Écran de veille « Starfield Simulation » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
float seg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float k = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * k);
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    vec3 col = vec3(0.0);
    float px = 1.0 / iResolution.y;
    for (int i = 0; i < 180; i++) {
        float fi = float(i);
        vec2 dir = vec2(h1(fi * 1.31) - 0.5, h1(fi * 2.17 + 3.1) - 0.5) * 2.2;
        float speed = 0.10 + 0.10 * h1(fi * 5.3);
        float z = fract(h1(fi * 7.7) - iTime * speed);
        float z2 = min(z + 0.035, 1.0);
        vec2 a = dir * 0.12 / (z + 0.04);
        vec2 b = dir * 0.12 / (z2 + 0.04);
        float near = 1.0 - z;
        float d = seg(p, a, b);
        float w = px * (0.6 + 2.2 * near * near);
        float bright = smoothstep(1.0, 0.55, z) * near;
        vec3 tint = mix(vec3(0.75, 0.85, 1.0), vec3(1.0, 0.92, 0.8), h1(fi * 9.1));
        col += tint * bright * (smoothstep(w, 0.0, d) + 0.25 * smoothstep(w * 5.0, 0.0, d));
    }
    fragColor = vec4(col, 1.0);
}

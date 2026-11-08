// name: Logos volants
// description: Des symboles ∞ de NOOS ondulent vers vous à travers la nuit
// tags: rétro, espace, noos
// homage: Écran de veille « Flying Windows » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
float logo(vec2 q, float th) {
    // Deux boucles ovales qui se croisent au centre : ∞.
    float r = 0.4;
    vec2 s = vec2(1.0, 1.15);
    float a = abs(length((q - vec2(-0.34, 0.0)) * s) - r);
    float b = abs(length((q - vec2(0.34, 0.0)) * s) - r);
    return min(a, b) - th;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    vec3 col = vec3(0.0);
    float alpha = 0.0;
    for (int i = 0; i < 28; i++) {
        float fi = float(i);
        float z = fract(h1(fi * 3.7) - iTime * (0.06 + 0.03 * h1(fi * 1.3)));
        vec2 dir = (vec2(h1(fi * 5.1), h1(fi * 8.3)) - 0.5) * 2.4;
        vec2 c = dir * 0.16 / (z + 0.05);
        float size = 0.035 / (z + 0.05);
        vec2 q = (p - c) / size;
        q.y += 0.12 * sin(q.x * 3.0 + iTime * 3.0 + fi);
        float d = logo(q, 0.075) * size;
        float px = 1.5 / iResolution.y;
        float m = smoothstep(px, -px, d) * smoothstep(1.0, 0.7, z) * smoothstep(0.0, 0.03, z);
        float g = clamp(q.x * 0.5 + 0.5, 0.0, 1.0);
        vec3 lc = mix(vec3(0.49, 0.36, 1.0), vec3(0.89, 0.33, 0.61), g);
        lc = mix(lc, vec3(1.0, 0.62, 0.26), smoothstep(0.75, 1.0, g));
        lc *= 0.75 + 0.35 * cos(q.y * 6.0 + iTime);
        col = mix(col, lc, m);
        col += lc * 0.06 * smoothstep(size * 0.6, 0.0, abs(d)) * (1.0 - z);
    }
    fragColor = vec4(col, 1.0);
}

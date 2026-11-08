// name: Vitrail
// description: Un vitrail vivant dont les cellules changent doucement de couleur
// tags: géométrie, couleurs, lumière
// author: NOOS
// license: CC0-1.0
vec2 h22(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = fragCoord / iResolution.y * 5.0;
    vec2 i = floor(p), f = fract(p);
    float d1 = 8.0, d2 = 8.0;
    vec2 best = vec2(0.0);
    for (int y = -1; y <= 1; y++)
        for (int x = -1; x <= 1; x++) {
            vec2 g = vec2(float(x), float(y));
            vec2 o = h22(i + g);
            o = 0.5 + 0.4 * sin(iTime * 0.25 + 6.2831 * o);
            float d = length(g + o - f);
            if (d < d1) { d2 = d1; d1 = d; best = i + g; }
            else if (d < d2) d2 = d;
        }
    vec2 r = h22(best);
    vec3 c = 0.5 + 0.5 * cos(6.2831 * (r.x + iTime * 0.02 + vec3(0.0, 0.33, 0.67)));
    c = mix(c, vec3(0.3, 0.45, 1.0), 0.25);
    float lead = smoothstep(0.03, 0.07, d2 - d1);
    float glow = 0.75 + 0.35 * sin(iTime * 0.6 + r.y * 6.0) * (1.0 - d1);
    vec2 uv = fragCoord / iResolution.xy;
    float beam = 0.7 + 0.5 * smoothstep(0.9, 0.0, length(uv - vec2(0.3 + 0.1 * sin(iTime * 0.1), 0.85)));
    vec3 col = c * glow * beam * lead + vec3(0.02) * (1.0 - lead);
    fragColor = vec4(col, 1.0);
}

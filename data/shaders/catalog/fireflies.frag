// name: Lucioles
// description: Des lucioles dansent au-dessus d'une prairie, au crépuscule
// tags: nature, doux, nuit
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
float h2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    vec3 col = mix(vec3(0.02, 0.04, 0.08), vec3(0.08, 0.12, 0.22), uv.y);
    col += vec3(0.35, 0.2, 0.3) * smoothstep(0.5, 0.0, abs(uv.y - 0.35)) * 0.25;
    // Herbes : trois rangées de brins qui ondulent au vent.
    float x = fragCoord.x / iResolution.y;
    for (int k = 2; k >= 0; k--) {
        float fk = float(k);
        float sx = x * (55.0 + fk * 30.0) + fk * 13.0;
        float id = floor(sx);
        float fx = fract(sx) - 0.5;
        float hgt = (0.10 + 0.16 * h2(vec2(id, fk))) * (1.0 - fk * 0.22);
        float yy = uv.y / hgt;
        float lean = 0.45 * sin(iTime * 0.7 + id * 0.9 + fk) * yy * yy;
        float halfw = 0.5 * (1.0 - yy);
        float inside = step(yy, 1.0) * smoothstep(halfw, halfw - 0.08, abs(fx - lean));
        vec3 g = mix(vec3(0.015, 0.05, 0.035), vec3(0.03, 0.09, 0.06), fk / 2.0);
        col = mix(col, g, inside);
    }
    // Lucioles.
    for (int i = 0; i < 40; i++) {
        float fi = float(i);
        vec2 c = vec2((h1(fi * 1.1) - 0.5) * iResolution.x / iResolution.y, h1(fi * 2.3) * 0.6 - 0.35);
        c += 0.08 * vec2(sin(iTime * (0.3 + 0.3 * h1(fi)) + fi), cos(iTime * (0.25 + 0.3 * h1(fi * 3.0)) + fi * 2.0));
        float blink = smoothstep(0.3, 1.0, sin(iTime * (0.8 + h1(fi * 5.0)) + fi * 4.0));
        float d = length(p - c);
        col += vec3(0.85, 1.0, 0.4) * blink * (0.004 / (d * d + 0.0004)) * 0.06;
    }
    fragColor = vec4(col, 1.0);
}

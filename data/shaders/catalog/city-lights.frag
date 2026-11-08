// name: Lumières de la ville
// description: Bokeh de lumières nocturnes, flou et chaleureux
// tags: doux, nuit, couleurs
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float ar = iResolution.x / iResolution.y;
    vec3 col = mix(vec3(0.01, 0.01, 0.04), vec3(0.05, 0.02, 0.08), fragCoord.y / iResolution.y);
    for (int i = 0; i < 60; i++) {
        float fi = float(i);
        float layer = h1(fi * 4.7);
        float x = fract(h1(fi * 1.3) + iTime * (0.004 + 0.012 * layer)) * (ar + 0.4) - 0.5 * (ar + 0.4);
        float y = (h1(fi * 2.9) - 0.5) * 0.9 + 0.03 * sin(iTime * 0.2 + fi);
        float r = 0.02 + 0.09 * layer * layer;
        float d = length(p - vec2(x, y));
        float disk = smoothstep(r, r * (0.85 - 0.3 * layer), d);
        float ring = smoothstep(r * 0.1, 0.0, abs(d - r * 0.92)) * 0.4;
        vec3 c = mix(vec3(1.0, 0.65, 0.25), vec3(1.0, 0.3, 0.45), h1(fi * 6.1));
        c = mix(c, vec3(0.35, 0.6, 1.0), step(0.75, h1(fi * 8.3)));
        float a = (0.18 + 0.25 * (1.0 - layer)) * (0.7 + 0.3 * sin(iTime * 0.7 + fi * 3.0));
        col += c * (disk + ring) * a;
    }
    fragColor = vec4(col, 1.0);
}

// name: Lampe à lave
// description: De grosses gouttes de cire chaude montent, fusionnent et redescendent
// tags: rétro, doux, couleurs
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float field = 0.0;
    for (int i = 0; i < 10; i++) {
        float fi = float(i);
        float sp = 0.05 + 0.05 * h1(fi * 3.1);
        vec2 c = vec2((h1(fi * 1.7) - 0.5) * iResolution.x / iResolution.y * 0.8 + 0.08 * sin(iTime * 0.3 + fi),
                      sin(iTime * sp * 2.0 + fi * 2.4) * 0.45);
        float r = 0.06 + 0.07 * h1(fi * 5.9);
        field += r * r / dot(p - c, p - c);
    }
    vec3 bg = mix(vec3(0.12, 0.02, 0.15), vec3(0.3, 0.05, 0.25), fragCoord.y / iResolution.y);
    float m = smoothstep(0.95, 1.05, field);
    vec3 wax = mix(vec3(1.0, 0.35, 0.1), vec3(1.0, 0.75, 0.2), clamp(field * 0.25, 0.0, 1.0));
    wax = mix(wax, vec3(1.0, 0.2, 0.45), smoothstep(0.3, -0.5, p.y));
    vec3 col = mix(bg, wax, m);
    col += vec3(1.0, 0.4, 0.2) * 0.12 * smoothstep(0.4, 1.0, field) * (1.0 - m);
    col *= 1.0 - 0.4 * length(p * vec2(0.6, 1.0));
    fragColor = vec4(col, 1.0);
}

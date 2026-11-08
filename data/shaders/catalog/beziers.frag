// name: Courbes
// description: Rubans de Bézier qui ondulent et laissent un sillage
// tags: lignes, rétro, néon
// homage: Écran de veille « Bézier » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float tri(float x) { return abs(fract(x) - 0.5) * 2.0; }
float h1(float n) { return fract(sin(n) * 43758.5453123); }
float seg(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float k = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * k);
}
vec2 pt(int c, int k, float t, vec2 box) {
    float s = float(c * 4 + k) + 1.0;
    return (vec2(tri(t * (0.03 + 0.04 * h1(s * 3.1)) + h1(s * 7.7)), tri(t * (0.035 + 0.04 * h1(s * 4.3)) + h1(s * 2.3))) - 0.5) * 2.0 * box;
}
vec2 bez(vec2 a, vec2 b, vec2 c, vec2 d, float u) {
    float v = 1.0 - u;
    return v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c + u * u * u * d;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 box = vec2(0.5 * iResolution.x / iResolution.y, 0.5) * 0.92;
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float px = 1.0 / iResolution.y;
    vec3 col = vec3(0.0);
    for (int c = 0; c < 2; c++) {
        vec3 tint = c == 0 ? vec3(0.3, 0.75, 1.0) : vec3(1.0, 0.45, 0.75);
        for (int e = 0; e < 5; e++) {
            float t = iTime - float(e) * 0.8;
            vec2 A = pt(c, 0, t, box), B = pt(c, 1, t, box), C = pt(c, 2, t, box), D = pt(c, 3, t, box);
            vec2 prev = A;
            float dmin = 1e3;
            for (int i = 1; i <= 18; i++) {
                vec2 cur = bez(A, B, C, D, float(i) / 18.0);
                dmin = min(dmin, seg(p, prev, cur));
                prev = cur;
            }
            float fade = 1.0 - float(e) / 5.0;
            col += tint * fade * (smoothstep(px * 1.8, 0.0, dmin) + 0.15 * smoothstep(px * 9.0, 0.0, dmin));
        }
    }
    fragColor = vec4(min(col, vec3(1.0)), 1.0);
}

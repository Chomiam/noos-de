// name: Tuyauterie
// description: Des tuyaux brillants poussent en zigzag puis l'écran repart à zéro
// tags: rétro, 3d, géométrie
// homage: Écran de veille « 3D Pipes » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float h1(float n) { return fract(sin(n) * 43758.5453123); }
vec2 dirv(int d) { return d == 0 ? vec2(1.0, 0.0) : d == 1 ? vec2(0.0, 1.0) : d == 2 ? vec2(-1.0, 0.0) : vec2(0.0, -1.0); }
vec3 shade(vec3 n, vec3 base) {
    vec3 l = normalize(vec3(-0.5, 0.6, 0.65));
    float dif = max(dot(n, l), 0.0);
    float spe = pow(max(dot(reflect(-l, n), vec3(0.0, 0.0, 1.0)), 0.0), 24.0);
    return base * (0.18 + 0.85 * dif) + vec3(0.9) * spe;
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    const float H = 14.0;
    float cs = iResolution.y / H;
    float W = floor(iResolution.x / cs);
    vec2 q = fragCoord / cs;
    const int STEPS = 90;
    float cycle = 22.0;
    float seed = floor(iTime / cycle);
    float local = mod(iTime, cycle);
    int grown = int(local * 6.0);
    float fadeOut = smoothstep(cycle, cycle - 1.5, local);
    vec3 col = mix(vec3(0.01, 0.01, 0.03), vec3(0.03, 0.03, 0.07), fragCoord.y / iResolution.y);
    float r = 0.28;
    for (int pi = 0; pi < 5; pi++) {
        float fp = float(pi) + seed * 17.0;
        vec3 base = clamp(abs(mod(h1(fp * 3.3) * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0, 0.0, 1.0) * 0.85 + 0.1;
        vec2 pos = floor(vec2(h1(fp * 1.7) * W, h1(fp * 2.9) * H));
        int dir = int(h1(fp * 4.1) * 4.0);
        int start = int(h1(fp * 5.7) * 25.0);
        for (int s = 0; s < STEPS; s++) {
            if (s + start > grown) break;
            float fs = float(s);
            int ndir = dir;
            if (h1(fp * 11.0 + fs * 1.37) < 0.28) ndir = (dir + (h1(fp * 13.0 + fs * 2.11) < 0.5 ? 1 : 3)) % 4;
            vec2 np = pos + dirv(ndir);
            if (np.x < 0.0 || np.x >= W || np.y < 0.0 || np.y >= H) { ndir = (ndir + 2) % 4; np = pos + dirv(ndir); }
            vec2 a = pos + 0.5, b = np + 0.5;
            vec2 ba = b - a;
            float k = clamp(dot(q - a, ba) / dot(ba, ba), 0.0, 1.0);
            vec2 off = q - (a + ba * k);
            float d = length(off);
            if (d < r) {
                vec2 perp = vec2(-ba.y, ba.x);
                float o = dot(off, perp) / r;
                vec3 n = normalize(vec3(perp * o, sqrt(max(1.0 - o * o, 0.0))));
                col = shade(n, base);
            }
            if (ndir != dir || s == 0) {
                float dj = length(q - a);
                if (dj < r * 1.35) {
                    vec2 o2 = (q - a) / (r * 1.35);
                    col = shade(vec3(o2, sqrt(max(1.0 - dot(o2, o2), 0.0))), base);
                }
            }
            dir = ndir;
            pos = np;
        }
    }
    fragColor = vec4(col * fadeOut, 1.0);
}

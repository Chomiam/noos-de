// name: Labyrinthe
// description: Promenade sans fin dans des couloirs de briques
// tags: rétro, 3d
// homage: Écran de veille « 3D Maze » de Windows 95 / 98
// author: NOOS
// license: CC0-1.0
float h2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
bool wall(vec2 c) {
    if (c.y == 0.0) return false;
    if (abs(c.y) == 1.0) return h2(c) > 0.45;
    return abs(c.y) > 2.0 || h2(c + 7.0) > 0.4;
}
vec3 brick(vec2 uv) {
    uv *= vec2(4.0, 8.0);
    uv.x += 0.5 * mod(floor(uv.y), 2.0);
    vec2 f = fract(uv);
    float m = step(0.06, f.x) * step(0.1, f.y);
    vec3 b = mix(vec3(0.55, 0.22, 0.14), vec3(0.72, 0.33, 0.2), h2(floor(uv)));
    return mix(vec3(0.75, 0.72, 0.66), b, m);
}
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 sc = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * 0.9;
    vec2 ro = vec2(t, 0.5 + 0.12 * sin(t * 0.37));
    float ang = 0.25 * sin(t * 0.21);
    vec2 fw = vec2(cos(ang), sin(ang));
    vec2 rt = vec2(-fw.y, fw.x);
    vec2 rd = normalize(fw + rt * sc.x * 1.2);
    // Lancer de rayon sur la grille (DDA).
    vec2 cell = floor(ro);
    vec2 stp = sign(rd);
    vec2 dd = abs(1.0 / rd);
    vec2 side = (stp * (cell - ro) + stp * 0.5 + 0.5) * dd;
    float dist = 30.0;
    float sideHit = 0.0;
    for (int i = 0; i < 40; i++) {
        if (side.x < side.y) { side.x += dd.x; cell.x += stp.x; sideHit = 0.0; }
        else { side.y += dd.y; cell.y += stp.y; sideHit = 1.0; }
        if (wall(cell)) { dist = sideHit < 0.5 ? side.x - dd.x : side.y - dd.y; break; }
    }
    float perp = dist * dot(rd, fw);
    float hgt = 0.5 / perp;
    vec3 col;
    if (abs(sc.y) < hgt) {
        vec2 hit = ro + rd * dist;
        float u = sideHit < 0.5 ? hit.y : hit.x;
        float v = sc.y / hgt * 0.5 + 0.5;
        col = brick(vec2(u, v)) * (sideHit < 0.5 ? 1.0 : 0.75);
    } else {
        float rowDist = 0.5 / abs(sc.y);
        vec2 fp = ro + rd * rowDist / dot(rd, fw);
        if (sc.y < 0.0) {
            vec2 g = floor(fp * 2.0);
            col = mix(vec3(0.16, 0.16, 0.2), vec3(0.3, 0.3, 0.36), mod(g.x + g.y, 2.0));
        } else {
            col = vec3(0.12, 0.14, 0.24) + 0.05 * step(0.9, fract(fp.x)) ;
        }
        dist = rowDist;
    }
    col *= exp(-dist * 0.12);
    fragColor = vec4(col, 1.0);
}

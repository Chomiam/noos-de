// name: Tunnel
// description: Plongée hypnotique dans un tunnel de damiers, façon démoscène
// tags: rétro, hypnotique, géométrie
// author: NOOS
// license: CC0-1.0
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    p += 0.08 * vec2(sin(iTime * 0.5), cos(iTime * 0.37));
    float a = atan(p.y, p.x);
    float r = length(p);
    vec2 uv = vec2(0.35 / r + iTime * 0.6, a / 3.14159 * 4.0 + iTime * 0.1);
    vec2 c = floor(uv * vec2(2.0, 2.0));
    float chk = mod(c.x + c.y, 2.0);
    vec3 col = mix(vec3(0.15, 0.1, 0.5), vec3(0.85, 0.3, 0.7), chk);
    col = mix(col, vec3(0.2, 0.8, 1.0), 0.3 + 0.3 * sin(uv.x * 0.5));
    vec2 f = abs(fract(uv * 2.0) - 0.5);
    col += vec3(0.8, 0.9, 1.0) * smoothstep(0.47, 0.5, max(f.x, f.y)) * 0.5;
    col *= smoothstep(0.0, 0.35, r);
    fragColor = vec4(col, 1.0);
}

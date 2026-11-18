// Shared finish for every visualizer: camera, colour, vignette, grain, fade.
uniform sampler2D u_scene;

void main() {
    vec2 p = px();
    vec2 uv0 = p / u_size;
    vec2 uv = clamp((uv0 - 0.5) / F_post1.z + 0.5 + F_cam.xy, vec2(0.0), vec2(1.0));
    vec3 col = tex_tl(u_scene, uv).rgb;

    col = (col - 0.5) * F_post1.x + 0.5 + F_post0.w;
    float l = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(l), col, F_post1.y);

    vec2 q = (p - u_size * 0.5) / (0.5 * length(u_size));
    col *= 1.0 - F_post0.x * 0.7 * smoothstep(0.3, 1.1, length(q));

    float gp = max(1.0, min(u_size.x, u_size.y) / 720.0);
    float g = hash21(ivec2(p / gp), uint(F_phase.w) * 7919u + 13u) - 0.5;
    col += g * F_post0.y * 0.08;
    col *= F_post0.z;
    o_color = vec4(clamp(col, vec3(0.0), vec3(1.0)), 1.0);
}

// Velouna visualizer prelude (API 1). Velouna puts "#version" and the
// precision lines in front of this; the plugin's shader follows.
//
// Coordinates: px() is the pixel position in the current target with the
// origin top left (y down), like most image tools. Textures written by a
// pass are read back the same way with tex_tl() / fetch_tl().

uniform vec4 u_f[38];
uniform vec2 u_size;    // target size in pixels
uniform vec2 u_origin;  // target offset in the framebuffer (only the final pass)
uniform vec4 u_step;    // ops count, decay, ops row, - (simulation passes)
uniform sampler2D u_ops;
out vec4 o_color;

#define F_res    u_f[0]   // w, h, 1/w, 1/h (output size)
#define F_time   u_f[1]   // t, duration, seed (0..1000), m = min(w, h)
#define F_au0    u_f[2]   // sub, bass, drums, mix
#define F_au1    u_f[3]   // voice, keys (mids), pads (presence), high
#define F_au2    u_f[4]   // intensity, section gain, build 0..1, drop flash
#define F_pulse  u_f[5]   // kick, snare, hat, keys
#define F_phase  u_f[6]   // beat, bar, flow, frame
#define F_shape0 u_f[14]  // params 1-4
#define F_shape1 u_f[15]  // params 5-8
#define F_react0 u_f[16]  // kick, snare, hats, bass
#define F_react1 u_f[17]  // voice, keys, dynamics, camera
#define F_post0  u_f[18]  // vignette, grain, fade, brightness
#define F_post1  u_f[19]  // contrast, saturation, zoom, -
#define F_cam    u_f[20]  // offset x, offset y, -, ripple count
#define F_rip(i) u_f[22 + (i)]

// Palette: 0 background, 1 background 2, 2-4 colors a-c, 5 accent, 6 text.
vec3 C(int i) { return u_f[7 + i].rgb; }

vec2 px() { return vec2(gl_FragCoord.x - u_origin.x, u_size.y - (gl_FragCoord.y - u_origin.y)); }
vec4 tex_tl(sampler2D s, vec2 uv) { return texture(s, vec2(uv.x, 1.0 - uv.y)); }
vec4 fetch_tl(sampler2D s, ivec2 p) {
    ivec2 sz = textureSize(s, 0);
    if (p.x < 0 || p.y < 0 || p.x >= sz.x || p.y >= sz.y) return vec4(0.0);
    return texelFetch(s, ivec2(p.x, sz.y - 1 - p.y), 0);
}
// Operation i of this step: head, a, b.
vec4 op_tex(int i, int k) { return texelFetch(u_ops, ivec2(i * 3 + k, int(u_step.z)), 0); }

uvec2 pcg2d(uvec2 v0) {
    uvec2 v = v0 * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> uvec2(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> uvec2(16u));
    return v;
}
float hash21(ivec2 p, uint s) {
    uvec2 h = pcg2d(uvec2(p) + uvec2(s, s * 7u));
    return float(h.x) * (1.0 / 4294967296.0);
}
vec2 hash22(ivec2 p, uint s) {
    uvec2 h = pcg2d(uvec2(p) + uvec2(s, s * 7u));
    return vec2(h) * (1.0 / 4294967296.0);
}
vec2 grad2(ivec2 ip) {
    uvec2 h = pcg2d(uvec2(ip));
    float a = float(h.x) * (6.2831853 / 4294967296.0);
    return vec2(cos(a), sin(a));
}
// Gradient noise, about -0.7..0.7
float gnoise(vec2 p) {
    ivec2 i = ivec2(floor(p));
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float a = dot(grad2(i), f);
    float b = dot(grad2(i + ivec2(1, 0)), f - vec2(1.0, 0.0));
    float c = dot(grad2(i + ivec2(0, 1)), f - vec2(0.0, 1.0));
    float d = dot(grad2(i + ivec2(1, 1)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
mat2 rot(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, s, -s, c);
}
float fbm(vec2 p, int oct) {
    float s = 0.0;
    float a = 0.5;
    vec2 q = p;
    mat2 r = rot(0.62);
    for (int i = 0; i < oct; i++) {
        s += a * gnoise(q);
        q = r * q * 2.03 + vec2(17.3, 9.1);
        a *= 0.5;
    }
    return s;
}
vec2 seed_off() {
    float s = F_time.z;
    return vec2(fract(s * 0.6180339) * 97.0 + s * 0.013, fract(s * 0.4142135) * 89.0 - s * 0.017);
}
// Ripple i: centre in centred units (y down)
vec2 rip_center(int i) { return (F_rip(i).xy * F_time.w - F_res.xy * 0.5) / F_time.w; }

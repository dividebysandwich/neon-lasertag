// Neon accents and their glow halos for the OpenGL headset renderer (src/glvr): the GL port of
// assets/shaders/neon.wgsl (keep the two in step). Compiled with VERTEX or FRAGMENT defined,
// and GLOW for the halos; both eyes in one pass (GL_OVR_multiview2).
//
// There is no bloom here (one pass straight into the headset's image): the HDR colour is
// mapped to the display's range in the shader, keeping its hue, so the strips look bright and
// saturated instead of clipping to white; the halos (blended ONE, ONE) stand in for the bloom.

uniform mat4 u_view_proj[2];
uniform mat4 u_model;
// The material: rgb fixed colour (modes 1, 2), a intensity; x mode (0 team by position and
// vertex tint, 1 fixed colour, 2 fixed colour flowing along uv.x), w half length of the arena (m).
uniform vec4 u_colour;
uniform vec4 u_params;
// The frame: time (s), team colours, event pulses (xyz origin, w start time, negative: unused;
// rgb colour).
uniform float u_time;
uniform vec3 u_red;
uniform vec3 u_blue;
uniform vec4 u_pulse_pos[8];
uniform vec4 u_pulse_colour[8];
// x: halo shape (0 strip, 1 volume: fades edge on), y: flow speed (m/s, mode 2).
uniform vec4 u_extra;
uniform vec3 u_eye[2];

#ifdef VERTEX
layout(num_views = 2) in;
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_tint;
out vec3 v_world;
out vec3 v_normal;
out vec2 v_uv;
out vec4 v_tint;
out vec3 v_eye;

void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_world = world.xyz;
    v_normal = mat3(u_model) * a_normal;
    v_uv = a_uv;
    v_tint = a_tint;
    v_eye = u_eye[VIEW];
    gl_Position = u_view_proj[VIEW] * world;
}
#endif

#ifdef FRAGMENT
in vec3 v_world;
in vec3 v_normal;
in vec2 v_uv;
in vec4 v_tint;
in vec3 v_eye;
layout(location = 0) out vec4 o_colour;

float hash2(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise, about 0..1.
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash2(i);
    float b = hash2(i + vec2(1.0, 0.0));
    float c = hash2(i + vec2(0.0, 1.0));
    float d = hash2(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Streaks of light along the accents (see neon.wgsl).
float streaks(vec3 world, float t) {
    vec2 block = floor(world.xz / 8.0);
    float band = floor(world.y / 0.6);
    vec2 key = block * 1.7 + vec2(band * 5.3, band * 2.9);
    float slot = 3.5;
    float phase = hash2(key + 0.5) * 9.0;
    float n = floor(t / slot + phase);
    float local = fract(t / slot + phase) * slot;
    if (hash2(key + n * 1.31) > 0.22) {
        return 0.0;
    }
    float dir = hash2(key + n * 2.17) > 0.5 ? 1.0 : -1.0;
    float along = dir * (world.x + world.z - (block.x + block.y) * 8.0 - 8.0);
    float head = -10.0 + local * 26.0;
    float d = head - along;
    if (d < 0.0) {
        return 0.0;
    }
    return exp(-d * 0.9) * (1.0 + 2.0 * exp(-d * 8.0));
}

float world_xz_noise(vec3 world, float t) {
    float n = vnoise(world.xz * 0.11 + vec2(t * 0.05, t * 0.02)) * 0.65
        + vnoise(world.xz * 0.27 - vec2(t * 0.08, -t * 0.04)) * 0.35;
    return smoothstep(0.5, 0.9, n);
}

vec3 neon_colour(vec3 world) {
    float t = u_time;
    float half_len = u_params.w;
    vec3 base;
    if (u_params.x < 0.5) {
        float k = smoothstep(-0.12, 0.12, world.z);
        base = mix(u_blue, u_red, k);
        base = mix(base, v_tint.rgb, v_tint.a);
        // Waves from each base towards the centre: a sharp front with a fading tail.
        float from_base = half_len - abs(world.z);
        float jitter = hash2(floor(world.xz * 0.5)) * 0.6;
        float w = fract((from_base - t * 9.0) / 22.0 + jitter * 0.05);
        float wave = smoothstep(0.0, 0.03, w) * exp(-w * 14.0);
        // (Dimmed accents, whose tint is darker than full colour, get a dimmer white too.)
        float level = mix(1.0, max(max(v_tint.r, v_tint.g), v_tint.b), v_tint.a);
        base = base * (1.0 + 3.5 * wave) + vec3(wave * 0.6 * level);
        // A slow shimmer so the accents feel alive.
        base *= 0.85 + 0.15 * sin(t * 2.0 + world.x * 0.7 + world.y * 1.3 + jitter * 6.0);
        base *= 0.55 + 0.75 * vnoise(world.xz * 0.09 + vec2(t * 0.035, -t * 0.025));
        float s = streaks(world, t) * (0.5 + 0.5 * level);
        base = base * (1.0 + 3.0 * s) + vec3(s * 0.35 * level);
    } else if (u_params.x < 1.5) {
        base = u_colour.rgb;
    } else {
        // Pulses running along the strip, a few per metre.
        float w = fract(v_uv.x * 2.2 - t * u_extra.y);
        float pulse = smoothstep(0.0, 0.06, w) * exp(-w * 5.0);
        base = u_colour.rgb * (0.45 + 2.2 * pulse);
    }
    vec3 col = base * u_colour.a;
    for (int i = 0; i < 8; i++) {
        vec4 p = u_pulse_pos[i];
        if (p.w < 0.0) {
            continue;
        }
        float age = t - p.w;
        if (age < 0.0 || age > 2.5) {
            continue;
        }
        float r = age * 14.0;
        float d = distance(world, p.xyz) - r;
        float ring = exp(-d * d / 1.2) * (1.0 - age / 2.5);
        col += u_pulse_colour[i].rgb * ring * 8.0;
    }
    return col;
}

// HDR to the display's range: the brightest channel rolls off (1 - e^-x) with the others
// scaled alongside (the hue kept), and very bright light goes towards white as a bloomed
// core would.
vec3 tone(vec3 c) {
    c = max(c, vec3(0.0));
    float m = max(max(c.r, c.g), c.b);
    if (m <= 1e-5) {
        return vec3(0.0);
    }
    float mapped = 1.0 - exp(-m);
    vec3 hue = c * (mapped / m);
    float white = smoothstep(2.5, 14.0, m) * 0.55;
    return mix(hue, vec3(mapped), white);
}

void main() {
    vec3 col = neon_colour(v_world);
#ifdef GLOW
    // Halo: fades out away from the accent (uv.y 0 at the strip, 1 at the outer edge).
    float v = clamp(v_uv.y, 0.0, 1.0);
    float a = (1.0 - v) * (1.0 - v) * 0.35;
    if (u_extra.x > 0.5 && u_extra.x < 1.5) {
        float facing = abs(dot(normalize(v_normal), normalize(v_eye - v_world)));
        a *= facing * facing;
    }
    if (u_extra.x > 1.5) {
        a *= world_xz_noise(v_world, u_time);
    }
    o_colour = vec4(tone(col * a), a);
#else
    o_colour = vec4(tone(col), 1.0);
#endif
}
#endif

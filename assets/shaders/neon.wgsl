// Neon accents and their glow halos (src/materials.rs). The GL port for the foveated headset
// renderer is assets/shaders/gl/neon.glsl: keep the two in step.
//
// Mode 0 (the arena): accents are coloured by which half of the arena they are in (Red at +z,
// Blue at -z), unless the mesh's vertex colour says otherwise: its alpha mixes towards its rgb
// (the arena's middle has zones of other colours, reload stations are white). Waves of intense
// colour run from each base towards the centre, and event pulses (target hits, tag-outs) ripple
// out from where they happened.
// Mode 1: a fixed colour (bolts, blasters). Mode 2: a fixed colour with pulses of light
// flowing along the strip (uv.x: metres along it).
// Halos fade with uv.y (0 at the strip, 1 at the outer edge); volume halos (light cones) also
// fade where they are seen edge on; forcefields (shape 3) are translucent sheets of their own;
// cloaks (shape 4) are a faint shimmer, brightest at the silhouette.
//
// With reflections on (src/reflections.rs) the accents are drawn deferred: their colour goes
// into the G-buffer as unlit, so the screen-space reflections on the glossy surfaces show them.

#import bevy_pbr::mesh_view_bindings::view
#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_UNLIT_BIT},
}
#else
#import bevy_pbr::forward_io::VertexOutput
#endif

struct Neon {
    // rgb: fixed colour (modes 1, 2), a: intensity.
    color: vec4<f32>,
    // x: mode, y: glow (0 accent, 1 halo), z: time (s), w: half length of the arena (m).
    params: vec4<f32>,
    red: vec4<f32>,
    blue: vec4<f32>,
    // Event pulses: xyz origin, w start time (negative: unused).
    pulse_pos: array<vec4<f32>, 8>,
    // rgb colour of each pulse.
    pulse_color: array<vec4<f32>, 8>,
    // x: halo shape (0 strip, 1 volume, 2 haze, 3 forcefield, 4 cloak), y: flow speed (m/s,
    // mode 2), z: 1 for the arena's own lights (dimmed in a blackout), w: how bright those are.
    extra: vec4<f32>,
    // Shots near the eye: xyz, w strength (0: none). They light up the dimmed lights near them.
    shots: array<vec4<f32>, 8>,
};

// How much the shots light up a point (0..1): each within SHOT_REACH metres, more nearer.
const SHOT_REACH: f32 = 4.5;
fn shot_light(p: vec3<f32>) -> f32 {
    var l = 0.0;
    for (var i = 0; i < 8; i++) {
        let s = neon.shots[i];
        if (s.w > 0.0) {
            let k = max(1.0 - distance(p, s.xyz) / SHOT_REACH, 0.0);
            l += s.w * k * k;
        }
    }
    return min(l, 1.0);
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> neon: Neon;

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Smooth value noise, about 0..1.
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Now and then a pulse of light streaks along the accents: the arena is cut into 8 m blocks
// and 0.6 m height bands; in each, every few seconds there's a chance of a streak racing
// across it (along x + z, so along walls of either direction), a bright head and a tail.
fn streaks(world: vec3<f32>, t: f32) -> f32 {
    let block = floor(world.xz / 8.0);
    let band = floor(world.y / 0.6);
    let key = block * 1.7 + vec2<f32>(band * 5.3, band * 2.9);
    let slot = 3.5;
    let phase = hash2(key + 0.5) * 9.0;
    let n = floor(t / slot + phase);
    let local = fract(t / slot + phase) * slot;
    if (hash2(key + n * 1.31) > 0.22) {
        return 0.0;
    }
    let dir = select(-1.0, 1.0, hash2(key + n * 2.17) > 0.5);
    let along = dir * (world.x + world.z - (block.x + block.y) * 8.0 - 8.0);
    let head = -10.0 + local * 26.0;
    let d = head - along;
    if (d < 0.0) {
        return 0.0;
    }
    return exp(-d * 0.9) * (1.0 + 2.0 * exp(-d * 8.0));
}

fn neon_color(world: vec3<f32>, uv: vec2<f32>, tint: vec4<f32>) -> vec3<f32> {
    let t = neon.params.z;
    let half_len = neon.params.w;
    var base: vec3<f32>;
    if (neon.params.x < 0.5) {
        let k = smoothstep(-0.12, 0.12, world.z);
        base = mix(neon.blue.rgb, neon.red.rgb, k);
        base = mix(base, tint.rgb, tint.a);
        // Waves from each base towards the centre: a sharp front with a fading tail.
        let from_base = half_len - abs(world.z);
        let jitter = hash2(floor(world.xz * 0.5)) * 0.6;
        let w = fract((from_base - t * 9.0) / 22.0 + jitter * 0.05);
        let wave = smoothstep(0.0, 0.03, w) * exp(-w * 14.0);
        // (Dimmed accents, whose tint is darker than full colour, get a dimmer white too.)
        let level = mix(1.0, max(max(tint.r, tint.g), tint.b), tint.a);
        base = base * (1.0 + 3.5 * wave) + vec3<f32>(wave * 0.6 * level);
        // A slow shimmer so the accents feel alive, and slowly drifting brighter and darker
        // patches so the arena isn't lit evenly.
        base *= 0.85 + 0.15 * sin(t * 2.0 + world.x * 0.7 + world.y * 1.3 + jitter * 6.0);
        base *= 0.55 + 0.75 * vnoise(world.xz * 0.09 + vec2<f32>(t * 0.035, -t * 0.025));
        // Streaks (on the dimmed accents too, a little weaker there).
        let s = streaks(world, t) * (0.5 + 0.5 * level);
        base = base * (1.0 + 3.0 * s) + vec3<f32>(s * 0.35 * level);
    } else if (neon.params.x < 1.5) {
        base = neon.color.rgb;
    } else {
        // Pulses running along the strip, a few per metre.
        let w = fract(uv.x * 2.2 - t * neon.extra.y);
        let pulse = smoothstep(0.0, 0.06, w) * exp(-w * 5.0);
        base = neon.color.rgb * (0.45 + 2.2 * pulse);
    }
    var col = base * neon.color.a;
#ifndef OVERLAY
    // (Not on overlays: the VR menus don't flash with what happens in the arena.)
    for (var i = 0; i < 8; i++) {
        let p = neon.pulse_pos[i];
        if (p.w < 0.0) {
            continue;
        }
        let age = t - p.w;
        if (age < 0.0 || age > 2.5) {
            continue;
        }
        let r = age * 14.0;
        let d = distance(world, p.xyz) - r;
        let ring = exp(-d * d / 1.2) * (1.0 - age / 2.5);
        col += neon.pulse_color[i].rgb * ring * 8.0;
    }
#endif
    return col;
}

fn world_xz_noise(world: vec3<f32>, t: f32) -> f32 {
    let n = vnoise(world.xz * 0.11 + vec2<f32>(t * 0.05, t * 0.02)) * 0.65
        + vnoise(world.xz * 0.27 - vec2<f32>(t * 0.08, -t * 0.04)) * 0.35;
    return smoothstep(0.5, 0.9, n);
}

// Distance from the middle of the hexagon of a lattice (cells 1 across): 0 at its middle, 0.5
// on its edges; and the cell's id.
fn hex(p: vec2<f32>) -> vec3<f32> {
    let s = vec2<f32>(1.0, 1.7320508);
    let h = s * 0.5;
    let a = p - s * floor(p / s) - h;
    let pb = p - h;
    let b = pb - s * floor(pb / s) - h;
    var g = b;
    if (dot(a, a) < dot(b, b)) {
        g = a;
    }
    let q = abs(g);
    let d = max(q.x * 0.5 + q.y * 0.8660254, q.x);
    return vec3<f32>(d, p - g);
}

// A forcefield (halo shape 3): uv 0..1 across its width (x) and height (y). Translucent: a
// faint sheet with a hexagonal lattice, cells sparkling now and then, bands of light rolling
// up it, crawling noise, a bright frame, and brighter where seen edge on.
fn shield(world: vec3<f32>, uv: vec2<f32>, n: vec3<f32>, to_eye: vec3<f32>, t: f32) -> vec2<f32> {
    let p = vec2<f32>(world.x + world.z, world.y) * 3.2;
    let hx = hex(p);
    let lines = smoothstep(0.40, 0.49, hx.x);
    let cell = hx.yz;
    let spark = step(0.94, hash2(cell * 0.37 + floor(t * 5.0 + hash2(cell) * 3.0)))
        * (1.0 - smoothstep(0.0, 0.45, hx.x));
    let bands = pow(0.5 + 0.5 * sin(uv.y * 9.0 - t * 3.2), 10.0)
        + 0.6 * pow(0.5 + 0.5 * sin(uv.y * 3.0 + t * 1.3 + uv.x * 2.0), 16.0);
    let crawl = vnoise(p * 0.7 + vec2<f32>(t * 0.6, -t * 1.1));
    let flicker = 0.85 + 0.15 * sin(t * 37.0 + crawl * 6.0);
    let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    let frame = 1.0 - smoothstep(0.0, 0.08, edge);
    let facing = abs(dot(normalize(n), to_eye));
    var a = 0.07 + 0.10 * crawl + lines * (0.22 + 0.25 * crawl) + spark * 0.55 + bands * 0.22;
    a = (a * mix(1.8, 0.85, facing) + frame * 0.6) * flicker;
    return vec2<f32>(clamp(a, 0.0, 1.0), 1.0 + bands * 1.5 + spark * 2.0);
}

// A cloaked player (halo shape 4): next to nothing face on, a shimmering rim at the silhouette,
// with ripples crawling up it. Alpha.
fn cloak(world: vec3<f32>, n: vec3<f32>, to_eye: vec3<f32>, t: f32) -> f32 {
    let facing = abs(dot(normalize(n), to_eye));
    let rim = pow(1.0 - facing, 2.5);
    let p = vec2<f32>(world.x + world.z, world.y) * 7.0;
    let crawl = vnoise(p + vec2<f32>(t * 0.7, -t * 1.6));
    let ripple = 0.5 + 0.5 * sin(world.y * 22.0 - t * 5.0 + crawl * 4.0);
    return clamp((0.03 + 0.55 * rim) * (0.5 + 0.9 * crawl) * (0.75 + 0.25 * ripple), 0.0, 1.0);
}

// Overlays (src/materials.rs NeonKey: the VR menus) are drawn over the world: their depth is
// squeezed into the sliver next to the near plane (reversed depth, 1 there), keeping their
// own order, so only what is within OVERLAY_DEPTH of the near plane comes in front.
#ifdef PREPASS_PIPELINE
@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    var pbr = pbr_input_new();
    pbr.material.base_color = vec4<f32>(shade(in).rgb, 1.0);
    pbr.material.flags |= STANDARD_MATERIAL_FLAGS_UNLIT_BIT;
    pbr.frag_coord = in.position;
    pbr.world_position = in.world_position;
    pbr.world_normal = normalize(in.world_normal);
    pbr.N = pbr.world_normal;
    return deferred_output(in, pbr);
}
#else ifdef OVERLAY
const OVERLAY_DEPTH: f32 = 0.01;

struct OverlayOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> OverlayOut {
    var o: OverlayOut;
    o.color = shade(in);
    o.depth = 1.0 - (1.0 - in.position.z) * OVERLAY_DEPTH;
    return o;
}
#else
@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in);
}
#endif

fn shade(in: VertexOutput) -> vec4<f32> {
    var uv = vec2<f32>(0.5, 0.5);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    var tint = vec4<f32>(0.0);
#ifdef VERTEX_COLORS
    tint = in.color;
#endif
    // (The arena's own lights dim in a blackout: extra.z marks them, extra.w is how bright.)
    // (Shots light them up again around them.)
    var lit = neon.extra.w;
    if (neon.extra.z > 0.5 && lit < 0.99) {
        lit = max(lit, shot_light(in.world_position.xyz));
    }
    let col = neon_color(in.world_position.xyz, uv, tint) * mix(1.0, lit, neon.extra.z);
    if (neon.params.y > 0.5 && neon.extra.x > 3.5) {
        let to_eye = normalize(view.world_position - in.world_position.xyz);
        let a = cloak(in.world_position.xyz, in.world_normal, to_eye, neon.params.z);
        return vec4<f32>(col * a, a);
    }
    if (neon.params.y > 0.5 && neon.extra.x > 2.5) {
        let to_eye = normalize(view.world_position - in.world_position.xyz);
        let f = shield(in.world_position.xyz, uv, in.world_normal, to_eye, neon.params.z);
        return vec4<f32>(col * f.y * f.x, f.x);
    }
    if (neon.params.y > 0.5) {
        let v = clamp(uv.y, 0.0, 1.0);
        var a = (1.0 - v) * (1.0 - v) * 0.35;
        if (neon.extra.x > 0.5 && neon.extra.x < 1.5) {
            let to_eye = normalize(view.world_position - in.world_position.xyz);
            let facing = abs(dot(normalize(in.world_normal), to_eye));
            a *= facing * facing;
        }
        if (neon.extra.x > 1.5 && neon.extra.x < 2.5) {
            // Haze: drifting patches.
            let w = world_xz_noise(in.world_position.xyz, neon.params.z);
            a *= w;
        }
        return vec4<f32>(col * a, a);
    }
    return vec4<f32>(col, 1.0);
}

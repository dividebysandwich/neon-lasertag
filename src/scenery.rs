//! Atmosphere around the arena, all built from the three materials every renderer draws (dark
//! StandardMaterial bodies, `AccentMaterial` neon, additive `GlowMaterial` halos):
//!
//! - a city of procedural towers from ~25 m to ~200 m out (boxes, setbacks, prisms, tapered and
//!   twin towers, spires), with rows of little lit windows, neon bands, edge lines and crowns
//!   with their halos;
//! - a big spectrum display above each rear border wall (6 stacks of segments, low to high
//!   frequencies from the viewer's left, with falling peaks), driven by `MusicSpectrum`;
//! - soft light cones: from the arena's corners and long walls (pulsing with the bass) and
//!   searchlights on towers sweeping the sky.
//!
//! Static geometry is merged into one mesh per material; the displays swap prebuilt meshes.
//! Rebuilt (deterministically from the seed) whenever the arena's seed changes.
//! For screenshots, `LT_VIEW=x,y,z,tx,ty,tz` fixes the camera (or the headset's tracking space).

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::prelude::*;
use lt_sim::kit::{BORDER_H, BORDER_T};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::assets::MeshData;
use crate::game::Game;
use crate::materials::{AccentMaterial, BLUE, GlowMaterial, NeonUniform, RED, ZONE_COLORS};
use crate::music::{BANDS, MusicSpectrum};

const PINK: Vec3 = Vec3::new(1.0, 0.12, 0.6);
/// Scenery neon: the zone colours, then red and blue.
const NEON: [Vec3; 7] = [ZONE_COLORS[0], ZONE_COLORS[1], ZONE_COLORS[2], ZONE_COLORS[3], ZONE_COLORS[4], RED, BLUE];
const PURPLE: usize = 0;
const CYAN: usize = 3;
const ORANGE: usize = 4;
const C_RED: usize = 5;
const C_BLUE: usize = 6;
/// The display's stacks, low to high.
const STACK_COLORS: [Vec3; BANDS] = [ZONE_COLORS[0], PINK, ZONE_COLORS[4], ZONE_COLORS[1], ZONE_COLORS[2], ZONE_COLORS[3]];
const WARM: Vec3 = Vec3::new(1.0, 0.48, 0.16);
const COOL: Vec3 = Vec3::new(0.3, 0.55, 1.0);

// The spectrum displays: above the rear border walls, facing the centre.
const SEGMENTS: usize = 16;
const DISP_W: f32 = 19.0;
const DISP_H: f32 = 9.5;
const DISP_Y0: f32 = 9.0;
/// The displays stand this far behind the rear walls' inner faces.
const DISP_BACK: f32 = BORDER_T + 1.8;
const DISP_DEPTH: f32 = 0.6;
const STACK_W: f32 = 2.3;
const STACK_PITCH: f32 = 2.95;
const SEG_MARGIN: f32 = 0.45;
const SEG_PITCH: f32 = (DISP_H - 2.0 * SEG_MARGIN) / SEGMENTS as f32;
const SEG_H: f32 = SEG_PITCH * 0.74;
/// Unlit segments: halo quads at this v (faint).
const DIM_V: f32 = 0.74;
/// Resting intensities of the cones pulsing with the bass: corners (red, blue), long walls.
const CORNER_CONE: [f32; 2] = [0.4, 0.5];
const SIDE_CONE: f32 = 0.45;

#[derive(Component)]
pub struct Scenery;

/// Searchlights turning around the vertical.
#[derive(Component)]
struct Sweep {
    tilt: Quat,
    speed: f32,
    phase: f32,
}

#[derive(Resource)]
struct SceneryMats {
    body: Handle<StandardMaterial>,
    warm: Handle<AccentMaterial>,
    cool: Handle<AccentMaterial>,
    accent: [Handle<AccentMaterial>; 7],
    glow: [Handle<GlowMaterial>; 7],
    stack: [Handle<AccentMaterial>; BANDS],
    stack_glow: [Handle<GlowMaterial>; BANDS],
    /// Cones from the arena's corners (red half, blue half) and long walls.
    corner_cones: [Handle<GlowMaterial>; 2],
    side_cones: Handle<GlowMaterial>,
    sky_cones: Handle<GlowMaterial>,
    sweep_cones: [Handle<GlowMaterial>; 4],
    /// Materials pulsing with the bass, with their resting intensity.
    pulsing: Vec<(Handle<GlowMaterial>, f32)>,
}

/// The displays' stacks: prebuilt meshes per number of lit segments, and their state.
#[derive(Resource, Default)]
struct Displays {
    /// Per stack: lit meshes for 1..=SEGMENTS segments, halo meshes for 0..=SEGMENTS.
    lit: Vec<Vec<Handle<Mesh>>>,
    halo: Vec<Vec<Handle<Mesh>>>,
    /// Per stack: lit entity, halo entity, peak entity.
    stacks: Vec<(Entity, Entity, Entity)>,
    shown: Vec<(usize, Option<usize>)>,
    peak: [f32; BANDS],
    hold: [f32; BANDS],
}

pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Displays>()
            .add_systems(Startup, make_materials)
            .add_systems(Update, (build, (update_displays, sweep, pulse)).chain())
            .add_systems(PostUpdate, debug_view.before(TransformSystems::Propagate));
    }
}

fn make_materials(
    mut commands: Commands,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let body = std.add(StandardMaterial {
        base_color: Color::linear_rgb(0.01, 0.01, 0.014),
        perceptual_roughness: 0.7,
        reflectance: 0.3,
        ..default()
    });
    let mut accent = |c: Vec3, i: f32| accents.add(AccentMaterial { neon: NeonUniform::fixed(c, i) });
    let warm = accent(WARM, 0.5);
    let cool = accent(COOL, 0.5);
    let neon_accent = NEON.map(|c| accent(c, 3.0));
    let stack = STACK_COLORS.map(|c| accent(c, 4.0));
    let mut glow = |c: Vec3, i: f32| glows.add(GlowMaterial { neon: NeonUniform::fixed(c, i) });
    let neon_glow = NEON.map(|c| glow(c, 1.3));
    let stack_glow = STACK_COLORS.map(|c| glow(c, 2.2));
    let mut volume = |c: Vec3, i: f32| glows.add(GlowMaterial { neon: NeonUniform::fixed(c, i).volume() });
    let corner_cones = [volume(RED, CORNER_CONE[0]), volume(BLUE, CORNER_CONE[1])];
    let side_cones = volume(ZONE_COLORS[CYAN], SIDE_CONE);
    let sky_cones = volume(ZONE_COLORS[PURPLE], 0.7);
    let sweep_cones = [1, 2, 3, 4].map(|i| volume(ZONE_COLORS[i], 0.6));
    let pulsing = vec![
        (corner_cones[0].clone(), CORNER_CONE[0]),
        (corner_cones[1].clone(), CORNER_CONE[1]),
        (side_cones.clone(), SIDE_CONE),
    ];
    commands.insert_resource(SceneryMats {
        body,
        warm,
        cool,
        accent: neon_accent,
        glow: neon_glow,
        stack,
        stack_glow,
        corner_cones,
        side_cones,
        sky_cones,
        sweep_cones,
        pulsing,
    });
}

// ---------------------------------------------------------------------------------------------
// Mesh building.

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A quad (corners in order around it), wound to face `n`.
fn quad(m: &mut MeshData, p: [Vec3; 4], v: [f32; 4], n: Vec3) {
    let base = m.pos.len() as u32;
    let flip = (p[1] - p[0]).cross(p[2] - p[0]).dot(n) < 0.0;
    for (i, q) in p.iter().enumerate() {
        m.pos.push(q.to_array());
        m.nrm.push(n.to_array());
        m.uv.push([(i == 1 || i == 2) as u8 as f32, v[i]]);
    }
    if flip {
        m.idx.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    } else {
        m.idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// A quad with uv v = 0 along its first edge (p0, p1) and 1 along the other (p2, p3).
fn halo_quad(m: &mut MeshData, p: [Vec3; 4], n: Vec3) {
    quad(m, p, [0.0, 0.0, 1.0, 1.0], n);
}

/// A box: centre and half-extent vectors.
fn cuboid(m: &mut MeshData, c: Vec3, a: [Vec3; 3]) {
    for k in 0..3 {
        let (u, w) = (a[(k + 1) % 3], a[(k + 2) % 3]);
        for s in [1.0, -1.0] {
            let f = c + a[k] * s;
            let n = (a[k] * s).normalize_or_zero();
            quad(m, [f - u - w, f + u - w, f + u + w, f - u + w], [0.0; 4], n);
        }
    }
}

fn aabox(m: &mut MeshData, min: Vec3, max: Vec3) {
    let h = (max - min) / 2.0;
    cuboid(m, (min + max) / 2.0, [Vec3::X * h.x, Vec3::Y * h.y, Vec3::Z * h.z]);
}

/// Two unit vectors perpendicular to `d` and each other.
fn basis(d: Vec3) -> (Vec3, Vec3) {
    let s = if d.y.abs() < 0.99 { d.cross(Vec3::Y).normalize() } else { Vec3::X };
    (s, s.cross(d).normalize())
}

/// A square beam from `a` to `b`, `t` thick.
fn beam(m: &mut MeshData, a: Vec3, b: Vec3, t: f32) {
    let d = b - a;
    let (s, u) = basis(d.normalize());
    cuboid(m, (a + b) / 2.0, [d / 2.0, s * t / 2.0, u * t / 2.0]);
}

/// A flat disc (accent lens) facing `n`.
fn disc(m: &mut MeshData, c: Vec3, n: Vec3, r: f32, seg: usize) {
    let (s, u) = basis(n);
    for i in 0..seg {
        let a0 = i as f32 / seg as f32 * TAU;
        let a1 = (i + 1) as f32 / seg as f32 * TAU;
        let p0 = c + (s * a0.cos() + u * a0.sin()) * r;
        let p1 = c + (s * a1.cos() + u * a1.sin()) * r;
        quad(m, [c, p0, p1, c], [0.0; 4], n);
    }
}

/// A flat ring halo facing `n`, bright at `r0`, faded at `r1`.
fn halo_ring(m: &mut MeshData, c: Vec3, n: Vec3, r0: f32, r1: f32, seg: usize) {
    let (s, u) = basis(n);
    for i in 0..seg {
        let a0 = i as f32 / seg as f32 * TAU;
        let a1 = (i + 1) as f32 / seg as f32 * TAU;
        let d0 = s * a0.cos() + u * a0.sin();
        let d1 = s * a1.cos() + u * a1.sin();
        halo_quad(m, [c + d0 * r0, c + d1 * r0, c + d1 * r1, c + d0 * r1], n);
    }
}

/// A point light's glow seen from anywhere: three crossed rings.
fn lamp_halo(m: &mut MeshData, c: Vec3, r0: f32, r1: f32) {
    for n in [Vec3::X, Vec3::Y, Vec3::Z] {
        halo_ring(m, c, n, r0, r1, 10);
    }
}

/// A rectangle's halo in the plane z: bright along the rectangle's edges, fading `w` outward.
fn halo_rect(m: &mut MeshData, min: Vec2, max: Vec2, z: f32, w: f32, n: Vec3) {
    let p = |x: f32, y: f32| Vec3::new(x, y, z);
    let inner = [p(min.x, min.y), p(max.x, min.y), p(max.x, max.y), p(min.x, max.y)];
    let outer = [p(min.x - w, min.y - w), p(max.x + w, min.y - w), p(max.x + w, max.y + w), p(min.x - w, max.y + w)];
    for i in 0..4 {
        let j = (i + 1) % 4;
        halo_quad(m, [inner[i], inner[j], outer[j], outer[i]], n);
    }
}

/// An open light cone from `apex` along `dir`: radius r0 there to r1 at `len`; uv v 0 at the
/// emitter, 1 at the far end; normals outward.
pub(crate) fn cone(m: &mut MeshData, apex: Vec3, dir: Vec3, len: f32, r0: f32, r1: f32, seg: usize) {
    let (s, u) = basis(dir);
    let slope = (r1 - r0) / len;
    let base = m.pos.len() as u32;
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * TAU;
        let radial = s * a.cos() + u * a.sin();
        let n = (radial - dir * slope).normalize();
        for (r, l, v) in [(r0, 0.0, 0.0), (r1, len, 1.0)] {
            m.pos.push((apex + dir * l + radial * r).to_array());
            m.nrm.push(n.to_array());
            m.uv.push([i as f32 / seg as f32, v]);
        }
    }
    for i in 0..seg as u32 {
        let (a, b, c, d) = (base + i * 2, base + i * 2 + 1, base + i * 2 + 3, base + i * 2 + 2);
        m.idx.extend([a, b, c, a, c, d]);
    }
}

/// One straight-sided piece of a tower: a polygon (world xz) extruded from y0 to y1, scaled
/// by `top` towards its centre at the top.
struct Tier {
    c: Vec2,
    poly: Vec<Vec2>,
    y0: f32,
    y1: f32,
    top: f32,
}

impl Tier {
    fn rect(c: Vec2, w: f32, d: f32, yaw: f32, y0: f32, y1: f32, top: f32) -> Tier {
        let r = Vec2::from_angle(yaw);
        let poly = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, z)| c + r.rotate(Vec2::new(x * w / 2.0, z * d / 2.0)))
            .to_vec();
        Tier { c, poly, y0, y1, top }
    }

    fn ngon(c: Vec2, n: usize, r: f32, yaw: f32, y0: f32, y1: f32, top: f32) -> Tier {
        let poly = (0..n).map(|i| c + Vec2::from_angle(yaw + i as f32 / n as f32 * TAU) * r).collect();
        Tier { c, poly, y0, y1, top }
    }

    fn n(&self) -> usize {
        self.poly.len()
    }

    fn corner(&self, i: usize, v: f32) -> Vec3 {
        let p = self.poly[i % self.n()];
        let q = self.c + (p - self.c) * lerp(1.0, self.top, v);
        Vec3::new(q.x, lerp(self.y0, self.y1, v), q.y)
    }

    fn at(&self, i: usize, u: f32, v: f32) -> Vec3 {
        self.corner(i, v).lerp(self.corner(i + 1, v), u)
    }

    fn normal(&self, i: usize) -> Vec3 {
        let (a, b, t) = (self.corner(i, 0.0), self.corner(i + 1, 0.0), self.corner(i, 1.0));
        let n = (b - a).cross(t - a).normalize();
        let mid = (a + b) / 2.0;
        if n.dot(mid - Vec3::new(self.c.x, mid.y, self.c.y)) < 0.0 { -n } else { n }
    }

    /// Up along face i.
    fn up(&self, i: usize) -> Vec3 {
        (self.corner(i, 1.0) - self.corner(i, 0.0)).normalize()
    }

    fn width(&self, i: usize, v: f32) -> f32 {
        self.corner(i, v).distance(self.corner(i + 1, v))
    }

    fn v_of(&self, y: f32) -> f32 {
        (y - self.y0) / (self.y1 - self.y0)
    }

    fn top_centre(&self) -> Vec3 {
        Vec3::new(self.c.x, self.y1, self.c.y)
    }

    /// Distance from the centre to the faces at the top (roughly).
    fn top_radius(&self) -> f32 {
        let r = self.poly.iter().map(|p| p.distance(self.c)).fold(f32::MAX, f32::min);
        r * self.top
    }
}

/// Everything static, merged by material.
#[derive(Default)]
struct Parts {
    /// Half the arena's width and length (m).
    half: Vec2,
    body: MeshData,
    warm: MeshData,
    cool: MeshData,
    accent: [MeshData; 7],
    glow: [MeshData; 7],
    corner_cones: [MeshData; 2],
    side_cones: MeshData,
    sky_cones: MeshData,
}

/// How a tower is dressed.
struct Style {
    color: usize,
    crown_color: usize,
    warm: bool,
    mixed: f32,
    lit: f32,
    floor: f32,
    col: f32,
    win_w: f32,
    win_h: f32,
    /// Halo widths (m): bigger far away, to read at a distance.
    halo: f32,
    /// Offsets off the faces, growing with distance (depth precision).
    off: f32,
}

fn tier_body(p: &mut Parts, t: &Tier) {
    for i in 0..t.n() {
        let n = t.normal(i);
        quad(&mut p.body, [t.corner(i, 0.0), t.corner(i + 1, 0.0), t.corner(i + 1, 1.0), t.corner(i, 1.0)], [0.0; 4], n);
    }
    let c = t.top_centre();
    for i in 0..t.n() {
        quad(&mut p.body, [c, t.corner(i, 1.0), t.corner(i + 1, 1.0), c], [0.0; 4], Vec3::Y);
    }
}

/// Rows of windows on every face of `t`, some lit; none where `skip` (y ranges) says.
fn windows(p: &mut Parts, rng: &mut StdRng, t: &Tier, st: &Style, skip: &[(f32, f32)]) {
    let rows = ((t.y1 - t.y0 - 0.8) / st.floor).floor() as usize;
    for i in 0..t.n() {
        let n = t.normal(i);
        let w = t.width(i, 0.0).min(t.width(i, 1.0));
        let cols = ((w - 0.6) / st.col).floor() as usize;
        if cols == 0 {
            continue;
        }
        for r in 0..rows {
            let yc = t.y0 + 0.4 + (r as f32 + 0.5) * st.floor;
            let (ya, yb) = (yc - st.win_h / 2.0, yc + st.win_h / 2.0);
            if skip.iter().any(|&(a, b)| yb > a && ya < b) {
                continue;
            }
            let busy = rng.random::<f32>();
            let chance = st.lit * if busy < 0.2 { 2.4 } else if busy < 0.5 { 0.15 } else { 0.8 };
            let (va, vb) = (t.v_of(ya), t.v_of(yb));
            for c in 0..cols {
                if rng.random::<f32>() > chance {
                    continue;
                }
                let wv = t.width(i, (va + vb) / 2.0);
                let uc = (c as f32 + 0.5) / cols as f32;
                let du = st.win_w / 2.0 / wv;
                let o = n * st.off;
                let q = [t.at(i, uc - du, va) + o, t.at(i, uc + du, va) + o, t.at(i, uc + du, vb) + o, t.at(i, uc - du, vb) + o];
                let warm = st.warm != (rng.random::<f32>() < st.mixed);
                quad(if warm { &mut p.warm } else { &mut p.cool }, q, [0.0; 4], n);
            }
        }
    }
}

/// A neon band around `t` at height y, `bh` tall, with halos above and below.
fn band(p: &mut Parts, t: &Tier, y: f32, bh: f32, color: usize, st: &Style) {
    let (va, vb) = (t.v_of(y - bh / 2.0), t.v_of(y + bh / 2.0));
    for i in 0..t.n() {
        let n = t.normal(i);
        let up = t.up(i);
        let o = n * st.off * 2.0;
        let a = [t.at(i, 0.0, va) + o, t.at(i, 1.0, va) + o];
        let b = [t.at(i, 0.0, vb) + o, t.at(i, 1.0, vb) + o];
        quad(&mut p.accent[color], [a[0], a[1], b[1], b[0]], [0.0; 4], n);
        // Lips, so it reads from above and below.
        let d = n * st.off;
        quad(&mut p.accent[color], [b[0], b[1], b[1] + d, b[0] + d], [0.0; 4], up);
        quad(&mut p.accent[color], [a[0], a[1], a[1] + d, a[0] + d], [0.0; 4], -up);
        let h = st.halo;
        let g = n * st.off;
        halo_quad(&mut p.glow[color], [b[0] + g, b[1] + g, b[1] + g + up * h, b[0] + g + up * h], n);
        halo_quad(&mut p.glow[color], [a[0] + g, a[1] + g, a[1] + g - up * h, a[0] + g - up * h], n);
    }
}

/// A crown: a band at the top edge and a halo rising from it.
fn crown(p: &mut Parts, t: &Tier, color: usize, st: &Style) {
    let bh = (0.35 + st.halo * 0.12).min(t.y1 - t.y0);
    band(p, t, t.y1 - bh / 2.0, bh, color, st);
    let h = st.halo * 1.1;
    for i in 0..t.n() {
        let n = t.normal(i);
        let o = n * st.off * 3.0;
        let a = [t.corner(i, 1.0) + o, t.corner(i + 1, 1.0) + o];
        let rise = Vec3::Y * h + n * h * 0.3;
        halo_quad(&mut p.glow[color], [a[0], a[1], a[1] + rise, a[0] + rise], n);
    }
}

/// Neon lines up the edges of `t` (every `step`th vertex), with halos along the faces beside.
fn edge_lines(p: &mut Parts, t: &Tier, step: usize, color: usize, st: &Style) {
    let th = 0.12 + st.halo * 0.05;
    for i in (0..t.n()).step_by(step) {
        let prev = (i + t.n() - 1) % t.n();
        let (n0, n1) = (t.normal(prev), t.normal(i));
        let out = (n0 + n1).normalize() * st.off * 2.0;
        let (a, b) = (t.corner(i, 0.0) + out, t.corner(i, 1.0) + out);
        beam(&mut p.accent[color], a, b, th);
        let w = st.halo * 0.7;
        // Along face i (towards vertex i+1) and face i-1 (towards vertex i-1).
        for (n, towards) in [(n1, t.corner(i + 1, 0.0)), (n0, t.corner(prev, 0.0))] {
            let along = (towards - t.corner(i, 0.0)).normalize() * w;
            let g = n * st.off * 2.0;
            halo_quad(&mut p.glow[color], [a + g, b + g, b + g + along, a + g + along], n);
        }
    }
}

/// A spire on top of `t` with a warning light.
fn spire(p: &mut Parts, rng: &mut StdRng, t: &Tier, st: &Style) {
    let base = t.top_centre();
    let len = rng.random_range(6.0..(10.0 + (t.y1 * 0.35)));
    let th = (t.top_radius() * 0.12).clamp(0.25, 0.9);
    beam(&mut p.body, base, base + Vec3::Y * len * 0.55, th);
    beam(&mut p.body, base, base + Vec3::Y * len, th * 0.4);
    let tip = base + Vec3::Y * (len + 0.2);
    let s = (0.18 + st.halo * 0.04) * Vec3::ONE;
    aabox(&mut p.accent[ORANGE], tip - s, tip + s);
    lamp_halo(&mut p.glow[ORANGE], tip, s.x, 0.8 + st.halo * 0.6);
    if rng.random_bool(0.5) {
        let mid = base + Vec3::Y * len * 0.55;
        aabox(&mut p.accent[ORANGE], mid - s, mid + s);
        lamp_halo(&mut p.glow[ORANGE], mid, s.x, 0.6 + st.halo * 0.4);
    }
}

/// A tower at `at`, `d` metres from the arena. `team`: the colour index of the side it is on.
fn tower(p: &mut Parts, rng: &mut StdRng, at: Vec2, d: f32, team: usize, roofs: &mut Vec<(Vec3, f32)>) {
    let t = (d / 180.0).clamp(0.0, 1.0);
    let mut h = lerp(10.0, 55.0, t) * rng.random_range(0.6..1.5);
    if d > 80.0 && rng.random_bool(0.1) {
        h *= 1.8;
    }
    let fp = lerp(7.0, 18.0, t) * rng.random_range(0.7..1.25);
    let yaw = if rng.random_bool(0.5) { rng.random_range(0.0..TAU) } else { at.to_angle() + FRAC_PI_2 };
    let pick = |rng: &mut StdRng| {
        if rng.random_bool(0.2) { team } else { rng.random_range(0..5) }
    };
    let color = pick(rng);
    let st = Style {
        color,
        crown_color: if rng.random_bool(0.3) { pick(rng) } else { color },
        warm: rng.random_bool(0.55),
        mixed: rng.random_range(0.0..0.3),
        lit: rng.random_range(0.08..0.4),
        floor: rng.random_range(3.2..4.2),
        col: rng.random_range(2.2..3.6),
        win_w: rng.random_range(1.0..1.8),
        win_h: rng.random_range(1.2..1.9),
        halo: 0.9 + d / 55.0,
        off: 0.03 + d * 0.0007,
    };
    let aspect = rng.random_range(0.6..1.0);
    let ngon_sides = [6, 8, 8, 12, 16][rng.random_range(0..5)];
    let mut tiers = Vec::new();
    let kind = rng.random_range(0..100);
    match kind {
        // A plain box.
        0..25 => tiers.push(Tier::rect(at, fp, fp * aspect, yaw, 0.0, h, 1.0)),
        // Setbacks: boxes or prisms, smaller as they rise.
        25..55 => {
            let n = rng.random_range(2..=4);
            let prism = kind >= 45;
            let mut y = 0.0;
            let mut s = 1.0;
            for k in 0..n {
                let part = if k == n - 1 { h - y } else { h * rng.random_range(0.25..0.5) };
                let y1 = (y + part).min(h).max(y + 3.0);
                tiers.push(if prism {
                    Tier::ngon(at, ngon_sides, fp / 2.0 * s, yaw, y, y1, 1.0)
                } else {
                    Tier::rect(at, fp * s, fp * aspect * s, yaw, y, y1, 1.0)
                });
                y = y1;
                s *= rng.random_range(0.6..0.82);
            }
        }
        // A prism.
        55..72 => tiers.push(Tier::ngon(at, ngon_sides, fp / 2.0, yaw, 0.0, h, 1.0)),
        // Tapered.
        72..88 => {
            let top = rng.random_range(0.5..0.85);
            tiers.push(if rng.random_bool(0.5) {
                Tier::rect(at, fp, fp * aspect, yaw, 0.0, h, top)
            } else {
                Tier::ngon(at, ngon_sides.min(8), fp / 2.0, yaw, 0.0, h, top)
            });
        }
        // Twins with a sky bridge.
        _ => {
            let dir = Vec2::from_angle(yaw);
            let off = dir * fp * 0.42;
            let w = fp * 0.42;
            let h2 = h * rng.random_range(0.8..1.0);
            let prism = rng.random_bool(0.4);
            for (c, hh) in [(at + off, h), (at - off, h2)] {
                tiers.push(if prism {
                    Tier::ngon(c, 8, w / 2.0, yaw, 0.0, hh, 1.0)
                } else {
                    Tier::rect(c, w, w * 1.2, yaw, 0.0, hh, 1.0)
                });
            }
            let yb = h2 * rng.random_range(0.45..0.75);
            let a = Vec3::new(at.x + off.x, yb, at.y + off.y);
            let b = Vec3::new(at.x - off.x, yb, at.y - off.y);
            beam(&mut p.body, a, b, w * 0.45);
            let (s, u) = basis((b - a).normalize());
            let side = if s.y.abs() < 0.5 { s } else { u };
            for sg in [1.0, -1.0] {
                let o = side * (w * 0.225 + st.off * 2.0) * sg;
                beam(&mut p.accent[st.color], a + o, b + o, 0.12 + st.halo * 0.05);
            }
        }
    }
    let top = tiers.iter().map(|t| t.y1).fold(0.0, f32::max);
    for tier in &tiers {
        tier_body(p, tier);
        let mut skip = Vec::new();
        let is_top = tier.y1 >= top - 0.01 || kind >= 88;
        if is_top && rng.random_bool(0.8) {
            crown(p, tier, st.crown_color, &st);
            skip.push((tier.y1 - 1.5, tier.y1));
        } else if !is_top && rng.random_bool(0.6) {
            // The setback's edge.
            band(p, tier, tier.y1 - 0.3, 0.4, st.color, &st);
            skip.push((tier.y1 - 1.2, tier.y1));
        }
        let height = tier.y1 - tier.y0;
        if height > 12.0 && rng.random_bool(0.45) {
            for _ in 0..rng.random_range(1..=2) {
                let y = tier.y0 + rng.random_range(0.2..0.8) * height;
                band(p, tier, y, 0.3 + st.halo * 0.06, st.color, &st);
                skip.push((y - 1.0, y + 1.0));
            }
        }
        if tier.n() <= 8 && rng.random_bool(0.3) {
            edge_lines(p, tier, if tier.n() > 4 { 2 } else { 1 }, st.color, &st);
        }
        windows(p, rng, tier, &st, &skip);
        if is_top {
            if h > 22.0 && rng.random_bool(0.3) {
                spire(p, rng, tier, &st);
            } else {
                roofs.push((tier.top_centre(), d));
            }
        }
    }
}

/// Distance from `p` to the arena's border (outside it).
fn arena_distance(half: Vec2, p: Vec2) -> f32 {
    let q = p.abs() - (half + Vec2::splat(BORDER_T));
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0)
}

fn city(p: &mut Parts, rng: &mut StdRng) -> Vec<(Vec3, f32)> {
    let mut towers: Vec<(Vec2, f32)> = Vec::new();
    let mut roofs = Vec::new();
    let (k1, k2) = (rng.random_range(2..5) as f32, rng.random_range(5..9) as f32);
    let (ph1, ph2) = (rng.random_range(0.0..TAU), rng.random_range(0.0..TAU));
    for _ in 0..6000 {
        if towers.len() >= 170 {
            break;
        }
        let at = Vec2::new(rng.random_range(-210.0..210.0), rng.random_range(-210.0..210.0));
        let d = arena_distance(p.half, at);
        if !(12.0..200.0).contains(&d) || at.length() < 25.0 {
            continue;
        }
        // Clusters and open stretches.
        let a = at.to_angle();
        let density = 0.55 + 0.45 * (a * k1 + ph1).sin() * (a * k2 + ph2 + d * 0.02).sin();
        if rng.random::<f32>() > density {
            continue;
        }
        let size = lerp(7.0, 18.0, (d / 180.0).clamp(0.0, 1.0)) * 1.25;
        let gap = 4.0 + d * 0.04;
        if towers.iter().any(|(q, s)| q.distance(at) < (size + s) / 2.0 + gap) {
            continue;
        }
        towers.push((at, size));
        let team = if at.y > 0.0 { C_RED } else { C_BLUE };
        tower(p, rng, at, d, team, &mut roofs);
    }
    info!("scenery: {} towers", towers.len());
    roofs
}

/// A light fixture at `at` shining along `dir`: a dark housing, a lens and its glow.
fn emitter(p: &mut Parts, at: Vec3, dir: Vec3, r: f32, color: usize) {
    let (s, u) = basis(dir);
    cuboid(&mut p.body, at - dir * r * 0.9, [dir * r * 0.8, s * r * 1.3, u * r * 1.3]);
    disc(&mut p.accent[color], at + dir * 0.02, dir, r, 12);
    halo_ring(&mut p.glow[color], at + dir * 0.03, dir, r, r * 2.5, 12);
}

/// Cones from the arena's corners and long walls, and searchlights on the roofs.
fn cones(p: &mut Parts, rng: &mut StdRng, roofs: &[(Vec3, f32)]) -> Vec<(Vec3, Quat, f32, f32, usize)> {
    let (hx, hz) = (p.half.x + BORDER_T / 2.0, p.half.y + BORDER_T / 2.0);
    for sx in [1.0, -1.0] {
        for sz in [1.0f32, -1.0] {
            // Beams crossing over the arena, rising, so nobody stands in them: the lower edge
            // stays above the border wall's top.
            let at = Vec3::new(sx * hx, BORDER_H + 1.5, sz * hz);
            beam(&mut p.body, Vec3::new(at.x, BORDER_H - 0.2, at.z), at - Vec3::Y * 0.3, 0.25);
            let aim = Vec3::new(-sx * 10.0, 13.5, sz * 4.0);
            let dir = (aim - at).normalize();
            let team = if sz > 0.0 { 0 } else { 1 };
            cone(&mut p.corner_cones[team], at, dir, 40.0, 0.3, 4.5, 20);
            emitter(p, at, dir, 0.3, if sz > 0.0 { C_RED } else { C_BLUE });
        }
        for z in [-16.0, 0.0, 16.0] {
            let at = Vec3::new(sx * (hx + 0.3), BORDER_H + 1.0, z);
            let aim = Vec3::new(-sx * 12.0, 11.5, z * 0.4 + sx * 6.0);
            let dir = (aim - at).normalize();
            cone(&mut p.side_cones, at, dir, 26.0, 0.25, 3.0, 18);
            emitter(p, at, dir, 0.25, CYAN);
        }
    }
    // Searchlights: on the nearer roofs, some fixed, four sweeping.
    let mut near: Vec<&(Vec3, f32)> = roofs.iter().filter(|(r, d)| *d < 110.0 && r.y > 18.0).collect();
    near.sort_by(|a, b| a.1.total_cmp(&b.1));
    // Spread them around rather than taking the nearest few.
    let mut picked: Vec<Vec3> = Vec::new();
    for (r, _) in near {
        if picked.iter().all(|q| Vec2::new(q.x - r.x, q.z - r.z).length() > 45.0) {
            picked.push(*r);
        }
        if picked.len() >= 8 {
            break;
        }
    }
    let mut sweeping = Vec::new();
    for (i, at) in picked.iter().enumerate() {
        let lens = *at + Vec3::Y * 0.8;
        aabox(&mut p.body, *at - Vec3::new(0.6, 0.0, 0.6), *at + Vec3::new(0.6, 0.6, 0.6));
        if i % 2 == 0 && sweeping.len() < 4 {
            let k = sweeping.len();
            disc(&mut p.accent[k + 1], lens, Vec3::Y, 0.45, 12);
            lamp_halo(&mut p.glow[k + 1], lens, 0.45, 2.5);
            let tilt = Quat::from_rotation_x(rng.random_range(0.25..0.55));
            let speed = rng.random_range(0.12..0.25) * if rng.random_bool(0.5) { 1.0 } else { -1.0 };
            sweeping.push((lens, tilt, speed, rng.random_range(0.0..TAU), k));
        } else {
            let away = Vec3::new(at.x, 0.0, at.z).normalize_or_zero();
            let dir = (Vec3::Y + away * rng.random_range(0.15..0.6)).normalize();
            cone(&mut p.sky_cones, lens, dir, 140.0, 0.45, 9.0, 20);
            disc(&mut p.accent[PURPLE], lens, Vec3::Y, 0.45, 12);
            lamp_halo(&mut p.glow[PURPLE], lens, 0.45, 2.5);
        }
    }
    sweeping
}

/// The x of stack b on the display at end `s` (+1: +z): the lowest band on the viewer's left.
fn stack_x(s: f32, b: usize) -> f32 {
    s * (2.5 - b as f32) * STACK_PITCH
}

fn front_z(half_len: f32, s: f32) -> f32 {
    s * (half_len + DISP_BACK - DISP_DEPTH / 2.0)
}

fn seg_y(j: usize) -> f32 {
    DISP_Y0 + SEG_MARGIN + j as f32 * SEG_PITCH + (SEG_PITCH - SEG_H) / 2.0
}

/// Segment j of stack b at end s, as a box standing out of the display.
fn segment(m: &mut MeshData, half_len: f32, s: f32, b: usize, j: usize) {
    let (x, y, z) = (stack_x(s, b), seg_y(j), front_z(half_len, s));
    aabox(
        m,
        Vec3::new(x - STACK_W / 2.0, y, z.min(z - s * 0.06)),
        Vec3::new(x + STACK_W / 2.0, y + SEG_H, z.max(z - s * 0.06)),
    );
}

/// The displays' frames (static) and their stacks' meshes.
/// Per stack: lit meshes (1..=SEGMENTS lit), halo meshes (0..=SEGMENTS lit), the peak segment (row 0).
type StackMeshes = (Vec<Vec<Handle<Mesh>>>, Vec<Vec<Handle<Mesh>>>, Vec<Handle<Mesh>>);

fn displays(p: &mut Parts, meshes: &mut Assets<Mesh>) -> StackMeshes {
    for s in [1.0f32, -1.0] {
        let z = s * (p.half.y + DISP_BACK);
        let m = 0.4;
        aabox(
            &mut p.body,
            Vec3::new(-DISP_W / 2.0 - m, DISP_Y0 - m, z - DISP_DEPTH / 2.0),
            Vec3::new(DISP_W / 2.0 + m, DISP_Y0 + DISP_H + m, z + DISP_DEPTH / 2.0),
        );
        for x in [-6.0, 6.0] {
            aabox(&mut p.body, Vec3::new(x - 0.4, 0.0, z - 0.3), Vec3::new(x + 0.4, DISP_Y0, z + 0.3));
        }
        let color = if s > 0.0 { C_RED } else { C_BLUE };
        let fz = front_z(p.half.y, s) - s * 0.03;
        let (x0, x1, y0, y1) = (-DISP_W / 2.0 - m, DISP_W / 2.0 + m, DISP_Y0 - m, DISP_Y0 + DISP_H + m);
        let t = 0.14;
        let n = Vec3::Z * -s;
        for (a, b) in [
            (Vec3::new(x0, y0, fz), Vec3::new(x1, y0, fz)),
            (Vec3::new(x0, y1, fz), Vec3::new(x1, y1, fz)),
            (Vec3::new(x0, y0, fz), Vec3::new(x0, y1, fz)),
            (Vec3::new(x1, y0, fz), Vec3::new(x1, y1, fz)),
        ] {
            beam(&mut p.accent[color], a, b, t);
        }
        halo_rect(&mut p.glow[color], Vec2::new(x0 - t, y0 - t), Vec2::new(x1 + t, y1 + t), fz, 1.8, n);
        // A line under the stacks.
        beam(&mut p.accent[color], Vec3::new(x0 + 0.6, DISP_Y0 + 0.2, fz), Vec3::new(x1 - 0.6, DISP_Y0 + 0.2, fz), 0.06);
    }
    let mut lit = Vec::new();
    let mut halo = Vec::new();
    for b in 0..BANDS {
        let mut l = Vec::new();
        let mut h = Vec::new();
        for k in 0..=SEGMENTS {
            if k > 0 {
                let mut m = MeshData::default();
                for s in [1.0, -1.0] {
                    for j in 0..k {
                        segment(&mut m, p.half.y, s, b, j);
                    }
                }
                l.push(meshes.add(m.to_mesh()));
            }
            let mut m = MeshData::default();
            for s in [1.0f32, -1.0] {
                let n = Vec3::Z * -s;
                let z = front_z(p.half.y, s) - s * 0.015;
                let x = stack_x(s, b);
                for j in 0..SEGMENTS {
                    let y = seg_y(j);
                    let q = [
                        Vec3::new(x - STACK_W / 2.0, y, z),
                        Vec3::new(x + STACK_W / 2.0, y, z),
                        Vec3::new(x + STACK_W / 2.0, y + SEG_H, z),
                        Vec3::new(x - STACK_W / 2.0, y + SEG_H, z),
                    ];
                    quad(&mut m, q, [DIM_V; 4], n);
                }
                if k > 0 {
                    let min = Vec2::new(x - STACK_W / 2.0, seg_y(0));
                    let max = Vec2::new(x + STACK_W / 2.0, seg_y(k - 1) + SEG_H);
                    halo_rect(&mut m, min, max, front_z(p.half.y, s) - s * 0.04, 0.45, n);
                }
            }
            h.push(meshes.add(m.to_mesh()));
        }
        lit.push(l);
        halo.push(h);
    }
    let peak = (0..BANDS)
        .map(|b| {
            let mut m = MeshData::default();
            for s in [1.0, -1.0] {
                segment(&mut m, p.half.y, s, b, 0);
            }
            meshes.add(m.to_mesh())
        })
        .collect();
    (lit, halo, peak)
}

fn triangles(m: &MeshData) -> usize {
    m.idx.len() / 3
}

fn build(
    mut commands: Commands,
    game: Option<Res<Game>>,
    mats: Res<SceneryMats>,
    mut displays_res: ResMut<Displays>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut built: Local<Option<u64>>,
    old: Query<Entity, With<Scenery>>,
) {
    let Some(game) = game else { return };
    let seed = game.level.seed;
    if *built == Some(seed) {
        return;
    }
    *built = Some(seed);
    for e in &old {
        commands.entity(e).despawn();
    }
    let mut rng = StdRng::seed_from_u64(seed ^ 0x5CE7_E41A_C17E_5EED);
    let mut p = Parts::default();
    let dm = game.level.dims();
    p.half = Vec2::new(dm.half_width(), dm.half_length());
    // The ground around the arena.
    quad(
        &mut p.body,
        [Vec3::new(-400.0, -0.05, -400.0), Vec3::new(400.0, -0.05, -400.0), Vec3::new(400.0, -0.05, 400.0), Vec3::new(-400.0, -0.05, 400.0)],
        [0.0; 4],
        Vec3::Y,
    );
    let roofs = city(&mut p, &mut rng);
    let sweeping = cones(&mut p, &mut rng, &roofs);
    let (lit, halo, peak) = displays(&mut p, &mut meshes);

    let mut tris = 0;
    let mut draws = 0;
    let mut spawn = |commands: &mut Commands, data: &MeshData, name: &str, mat: &dyn Fn(&mut EntityCommands)| {
        if data.is_empty() {
            return;
        }
        tris += triangles(data);
        draws += 1;
        let mut e = commands.spawn((Scenery, Mesh3d(meshes.add(data.to_mesh())), Transform::IDENTITY, Name::new(format!("scenery {name}"))));
        mat(&mut e);
    };
    spawn(&mut commands, &p.body, "body", &|e| {
        e.insert(MeshMaterial3d(mats.body.clone()));
    });
    spawn(&mut commands, &p.warm, "warm windows", &|e| {
        e.insert(MeshMaterial3d(mats.warm.clone()));
    });
    spawn(&mut commands, &p.cool, "cool windows", &|e| {
        e.insert(MeshMaterial3d(mats.cool.clone()));
    });
    for c in 0..NEON.len() {
        spawn(&mut commands, &p.accent[c], "neon", &|e| {
            e.insert(MeshMaterial3d(mats.accent[c].clone()));
        });
        spawn(&mut commands, &p.glow[c], "halo", &|e| {
            e.insert(MeshMaterial3d(mats.glow[c].clone()));
        });
    }
    for t in 0..2 {
        spawn(&mut commands, &p.corner_cones[t], "corner cones", &|e| {
            e.insert(MeshMaterial3d(mats.corner_cones[t].clone()));
        });
    }
    spawn(&mut commands, &p.side_cones, "side cones", &|e| {
        e.insert(MeshMaterial3d(mats.side_cones.clone()));
    });
    spawn(&mut commands, &p.sky_cones, "sky cones", &|e| {
        e.insert(MeshMaterial3d(mats.sky_cones.clone()));
    });
    let mut sweep_cone = MeshData::default();
    cone(&mut sweep_cone, Vec3::ZERO, Vec3::Y, 150.0, 0.45, 8.0, 20);
    let sweep_mesh = meshes.add(sweep_cone.to_mesh());
    for (at, tilt, speed, phase, k) in sweeping {
        tris += triangles(&sweep_cone);
        draws += 1;
        commands.spawn((
            Scenery,
            Sweep { tilt, speed, phase },
            Mesh3d(sweep_mesh.clone()),
            MeshMaterial3d(mats.sweep_cones[k].clone()),
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(phase) * tilt),
            Name::new("searchlight"),
        ));
    }
    // The displays' stacks.
    let d = &mut *displays_res;
    d.stacks.clear();
    d.shown = vec![(usize::MAX, None); BANDS];
    d.peak = [0.0; BANDS];
    d.hold = [0.0; BANDS];
    for b in 0..BANDS {
        let lit_e = commands
            .spawn((Scenery, Mesh3d(lit[b][0].clone()), MeshMaterial3d(mats.stack[b].clone()), Transform::IDENTITY, Visibility::Hidden))
            .id();
        let halo_e = commands
            .spawn((Scenery, Mesh3d(halo[b][0].clone()), MeshMaterial3d(mats.stack_glow[b].clone()), Transform::IDENTITY, Visibility::Inherited))
            .id();
        // The peak: the stack's bottom segment, moved up to its row.
        let peak_e = commands
            .spawn((Scenery, Mesh3d(peak[b].clone()), MeshMaterial3d(mats.stack[b].clone()), Transform::IDENTITY, Visibility::Hidden))
            .id();
        d.stacks.push((lit_e, halo_e, peak_e));
        draws += 3;
        tris += 12 * 2 * SEGMENTS + 2 * 2 * (SEGMENTS + 4) + 12 * 2;
    }
    d.lit = lit;
    d.halo = halo;
    info!("scenery: {tris} triangles (displays at most), {draws} draws");
}

/// Lights the displays' segments to the spectrum, with peaks that hold and fall.
fn update_displays(
    time: Res<Time>,
    spectrum: Res<MusicSpectrum>,
    mut d: ResMut<Displays>,
    mut q: Query<(&mut Mesh3d, &mut Visibility, &mut Transform)>,
) {
    if d.stacks.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let d = &mut *d;
    for b in 0..BANDS {
        let level = spectrum.bands[b].clamp(0.0, 1.0);
        let k = ((level * SEGMENTS as f32).round() as usize).min(SEGMENTS);
        if k as f32 >= d.peak[b] {
            d.peak[b] = k as f32;
            d.hold[b] = 0.7;
        } else if d.hold[b] > 0.0 {
            d.hold[b] -= dt;
        } else {
            d.peak[b] = (d.peak[b] - 5.0 * dt).max(0.0);
        }
        let p = d.peak[b].ceil() as usize;
        let peak = (p > k && p > 0).then(|| p - 1);
        if d.shown[b] == (k, peak) {
            continue;
        }
        d.shown[b] = (k, peak);
        let (lit_e, halo_e, peak_e) = d.stacks[b];
        if let Ok((mut mesh, mut vis, _)) = q.get_mut(lit_e) {
            vis.set_if_neq(if k > 0 { Visibility::Inherited } else { Visibility::Hidden });
            if k > 0 {
                mesh.set_if_neq(Mesh3d(d.lit[b][k - 1].clone()));
            }
        }
        if let Ok((mut mesh, _, _)) = q.get_mut(halo_e) {
            mesh.set_if_neq(Mesh3d(d.halo[b][k].clone()));
        }
        if let Ok((_, mut vis, mut tf)) = q.get_mut(peak_e) {
            vis.set_if_neq(if peak.is_some() { Visibility::Inherited } else { Visibility::Hidden });
            if let Some(j) = peak {
                tf.translation.y = j as f32 * SEG_PITCH;
            }
        }
    }
}

fn sweep(time: Res<Time>, mut q: Query<(&Sweep, &mut Transform)>) {
    let t = time.elapsed_secs_wrapped();
    for (s, mut tf) in &mut q {
        tf.rotation = Quat::from_rotation_y(s.phase + s.speed * t) * s.tilt;
    }
}

fn pulse(spectrum: Res<MusicSpectrum>, mats: Res<SceneryMats>, mut glows: ResMut<Assets<GlowMaterial>>) {
    let k = 0.55 + 0.9 * spectrum.bass;
    for (h, base) in &mats.pulsing {
        if let Some(mut m) = glows.get_mut(h) {
            m.neon.color.w = base * k;
        }
    }
}

/// Debugging: `LT_VIEW=x,y,z,tx,ty,tz` puts the flat screen's camera (or the headset's
/// tracking space, turned to look along the same heading) at x,y,z looking at tx,ty,tz.
fn debug_view(
    mut cams: Query<&mut Transform, With<Camera3d>>,
    rig: Option<Res<crate::vr::XrRig>>,
    mut all: Query<&mut Transform, Without<Camera3d>>,
    mut view: Local<Option<Option<(Vec3, Vec3)>>>,
) {
    let view = view.get_or_insert_with(|| {
        let n: Vec<f32> = std::env::var("LT_VIEW").ok()?.split(',').filter_map(|t| t.parse().ok()).collect();
        (n.len() >= 6).then(|| (Vec3::new(n[0], n[1], n[2]), Vec3::new(n[3], n[4], n[5])))
    });
    let Some((at, target)) = *view else { return };
    if let Some(rig) = rig
        && let Ok(mut tf) = all.get_mut(rig.0)
    {
        let flat = Vec3::new(target.x - at.x, 0.0, target.z - at.z);
        *tf = Transform::from_xyz(at.x, at.y - 1.65, at.z).looking_to(flat, Vec3::Y);
        return;
    }
    for mut tf in &mut cams {
        *tf = Transform::from_translation(at).looking_at(target, Vec3::Y);
    }
}

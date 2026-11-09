//! Contours: rolling hills around the arena, their contour lines glowing (with little lights
//! along them), and spheres, cones and domes ringed with glowing lines resting on them.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::prelude::*;
use rand::Rng;
use rand::rngs::StdRng;

use super::shapes::Lathe;
use super::spires::lasers;
use super::{
    C_PINK, C_RED, C_WHITE, CYAN, ORANGE, PURPLE, Parts, aabox, arena_distance, halo_quad, lerp,
    quad,
};

/// The hills reach this far out (m) from the middle, in cells this big.
const EXTENT: f32 = 380.0;
const CELL: f32 = 7.6;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The hills: a sum of waves, flat (just below the arena's floor) near the arena.
struct Terrain {
    half: Vec2,
    /// Direction times frequency, phase, amplitude.
    waves: Vec<(Vec2, f32, f32)>,
    amp: f32,
}

impl Terrain {
    fn new(rng: &mut StdRng, half: Vec2) -> Terrain {
        let waves = (0..7)
            .map(|_| {
                let len = rng.random_range(45.0..170.0);
                let dir = Vec2::from_angle(rng.random_range(0.0..TAU));
                (dir * TAU / len, rng.random_range(0.0..TAU), len)
            })
            .collect();
        Terrain {
            half,
            waves,
            amp: rng.random_range(16.0..28.0),
        }
    }

    fn height(&self, p: Vec2) -> f32 {
        let d = arena_distance(self.half, p);
        let ramp = smoothstep(16.0, 75.0, d);
        if ramp <= 0.0 {
            return -0.05;
        }
        let (mut n, mut total) = (0.0, 0.0);
        for (w, ph, a) in &self.waves {
            n += a * (w.dot(p) + ph).sin();
            total += a;
        }
        let n = (0.5 + 0.5 * n / total * 1.6).clamp(0.0, 1.0);
        // Higher further out.
        let far = 0.45 + 0.55 * (d / 260.0).min(1.0);
        -0.05 + ramp * self.amp * far * n * n
    }
}

/// How thick things are drawn at `r` metres from the middle (lines, halos, offsets).
fn widths(r: f32) -> (f32, f32, f32) {
    (0.14 + r * 0.0028, 0.7 + r * 0.011, 0.05 + r * 0.0012)
}

pub(super) fn contours(p: &mut Parts, rng: &mut StdRng) {
    let terrain = Terrain::new(rng, p.half);
    let n = (2.0 * EXTENT / CELL) as usize;
    let at = |i: usize, j: usize| Vec2::new(-EXTENT + i as f32 * CELL, -EXTENT + j as f32 * CELL);
    let mut h = vec![0.0f32; (n + 1) * (n + 1)];
    for j in 0..=n {
        for i in 0..=n {
            h[j * (n + 1) + i] = terrain.height(at(i, j));
        }
    }
    let hh = |i: usize, j: usize| h[j.min(n) * (n + 1) + i.min(n)];
    // The ground (smooth normals from the neighbours).
    let ground = &mut p.pale;
    let base = ground.pos.len() as u32;
    for j in 0..=n {
        for i in 0..=n {
            let q = at(i, j);
            let dx = hh(i + 1, j) - hh(i.saturating_sub(1), j);
            let dz = hh(i, j + 1) - hh(i, j.saturating_sub(1));
            let nrm = Vec3::new(-dx, 2.0 * CELL, -dz).normalize();
            ground.pos.push([q.x, hh(i, j), q.y]);
            ground.nrm.push(nrm.to_array());
            ground.uv.push([0.0, 0.0]);
        }
    }
    let row = n as u32 + 1;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = base + j * row + i;
            // (Facing up: +x then +z is clockwise seen from above.)
            ground
                .idx
                .extend([a, a + row, a + 1, a + 1, a + row, a + row + 1]);
        }
    }
    // The contour lines.
    let step = rng.random_range(2.1..2.9);
    let (main, every, other) = match rng.random_range(0..4) {
        0 | 1 => (C_PINK, 5, C_RED),
        2 => (C_PINK, 4, ORANGE),
        _ => (PURPLE, 4, C_PINK),
    };
    let mut segments = 0;
    for j in 0..n {
        for i in 0..n {
            let v = [hh(i, j), hh(i + 1, j), hh(i + 1, j + 1), hh(i, j + 1)];
            let lo = v.iter().copied().fold(f32::MAX, f32::min);
            let hi = v.iter().copied().fold(f32::MIN, f32::max);
            let k0 = (lo / step).ceil().max(1.0) as i32;
            let k1 = (hi / step).floor() as i32;
            if k1 < k0 {
                continue;
            }
            let c = [at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)];
            for k in k0..=k1 {
                let level = k as f32 * step;
                let mut cross = Vec::new();
                for e in 0..4 {
                    let (a, b) = (v[e], v[(e + 1) % 4]);
                    if (a < level) != (b < level) {
                        let t = (level - a) / (b - a);
                        let q = c[e].lerp(c[(e + 1) % 4], t);
                        cross.push(Vec3::new(q.x, level, q.y));
                    }
                }
                let pairs: &[(usize, usize)] = match cross.len() {
                    2 => &[(0, 1)],
                    4 => {
                        // A saddle: join by the middle's side.
                        let mid = v.iter().sum::<f32>() / 4.0;
                        if (mid < level) == (v[0] < level) {
                            &[(0, 3), (1, 2)]
                        } else {
                            &[(0, 1), (2, 3)]
                        }
                    }
                    _ => &[],
                };
                let col = if k % every == 0 { other } else { main };
                for &(a, b) in pairs {
                    segment(p, cross[a], cross[b], col);
                    segments += 1;
                    if rng.random_bool(0.035) {
                        let m = (cross[a] + cross[b]) / 2.0;
                        let (w, _, off) = widths(Vec2::new(m.x, m.z).length());
                        let s = w * 0.9;
                        let c = m + Vec3::Y * (off + s);
                        aabox(
                            &mut p.accent[C_WHITE],
                            c - Vec3::splat(s),
                            c + Vec3::splat(s),
                        );
                    }
                }
            }
        }
    }
    // Shapes resting on the hills.
    let mut placed: Vec<(Vec2, f32)> = Vec::new();
    let want = rng.random_range(10..20);
    for _ in 0..3000 {
        if placed.len() >= want {
            break;
        }
        let q = Vec2::new(
            rng.random_range(-260.0..260.0),
            rng.random_range(-260.0..260.0),
        );
        let d = arena_distance(p.half, q);
        let k = (d / 220.0).clamp(0.0, 1.0);
        let r = lerp(5.0, 16.0, k) * rng.random_range(0.6..2.0);
        if d - r < 20.0 || d > 250.0 {
            continue;
        }
        if placed
            .iter()
            .any(|(o, s)| o.distance(q) < r + s + 6.0 + d * 0.03)
        {
            continue;
        }
        placed.push((q, r));
        shape(p, rng, &terrain, q, r);
    }
    info!(
        "scenery: contours, {segments} line segments, {} shapes",
        placed.len()
    );
    lasers(p, rng);
}

/// A contour line's segment from a to b (on the ground), with halos either side.
fn segment(p: &mut Parts, a: Vec3, b: Vec3, col: usize) {
    let m = (a + b) / 2.0;
    let (w, halo, off) = widths(Vec2::new(m.x, m.z).length());
    let along = (b - a).normalize_or(Vec3::X);
    // Across, in the ground's slope (as well as one segment knows it): flat.
    let across = Vec3::Y.cross(along).normalize_or(Vec3::Z);
    let up = Vec3::Y * off;
    // (Lengthened a little, so the segments meet.)
    let (a, b) = (a - along * w * 0.5 + up, b + along * w * 0.5 + up);
    let x = across * w * 0.5;
    quad(
        &mut p.accent[col],
        [a - x, b - x, b + x, a + x],
        [0.0; 4],
        Vec3::Y,
    );
    for s in [1.0, -1.0] {
        let (e0, e1) = (a + x * s, b + x * s);
        let o = across * halo * s;
        halo_quad(&mut p.glow[col], [e0, e1, e1 + o, e0 + o], Vec3::Y);
    }
}

/// A sphere, cone or dome at q, about r across, ringed with lines.
fn shape(p: &mut Parts, rng: &mut StdRng, terrain: &Terrain, q: Vec2, r: f32) {
    let ground = terrain.height(q);
    let dist = q.length();
    let (w, halo, off) = widths(dist);
    let col = [C_PINK, C_PINK, C_RED, ORANGE, PURPLE, CYAN][rng.random_range(0..6)];
    let kind = rng.random_range(0..100);
    // Rings this far apart (m, along the surface).
    let spacing = (r / rng.random_range(5.0..9.0)).max(0.8);
    let (lathe, rings): (Lathe, Vec<usize>) = if kind < 65 {
        // A sphere, sunk into the hill, its rings round an axis mostly facing the arena.
        let c = Vec3::new(q.x, ground + r * rng.random_range(0.35..0.85), q.y);
        let axis = match rng.random_range(0..4) {
            0 => Vec3::Y,
            1 => Vec3::new(
                rng.random_range(-1.0..1.0),
                rng.random_range(-1.0..1.0),
                rng.random_range(-1.0..1.0),
            )
            .normalize_or(Vec3::Y),
            _ => {
                let to = -Vec3::new(q.x, 0.0, q.y).normalize_or(Vec3::Z);
                let side = Vec3::Y.cross(to);
                (to + side * rng.random_range(-0.5..0.5) + Vec3::Y * rng.random_range(-0.3..0.4))
                    .normalize()
            }
        };
        let steps = ((PI * r / spacing).round() as usize).clamp(8, 16);
        // Finer than the rings, so the sphere is round.
        let sub = (18 / steps).max(1);
        let total = steps * sub;
        let prof = (0..=total)
            .map(|i| {
                let a = i as f32 / total as f32 * PI;
                Vec2::new(r * a.sin(), -r * a.cos())
            })
            .collect();
        let rings = (1..steps).map(|i| i * sub).collect();
        (Lathe::new(c, axis, 28, 0.0, prof), rings)
    } else if kind < 85 {
        // A cone.
        let height = r * rng.random_range(1.2..2.6);
        let steps = ((height / spacing).round() as usize).clamp(4, 14);
        let prof = (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                Vec2::new((r * (1.0 - t)).max(0.05), height * t)
            })
            .collect();
        let base = Vec3::new(q.x, ground - 1.0, q.y);
        (Lathe::vertical(base, 28, 0.0, prof), (0..steps).collect())
    } else {
        // A dome.
        let steps = ((FRAC_PI_2 * r / spacing).round() as usize).clamp(4, 9);
        let prof = (0..=steps)
            .map(|i| {
                let a = FRAC_PI_2 + i as f32 / steps as f32 * FRAC_PI_2;
                Vec2::new(r * a.sin(), -r * a.cos())
            })
            .collect();
        let base = Vec3::new(q.x, ground - 0.5, q.y);
        (Lathe::vertical(base, 28, 0.0, prof), (0..steps).collect())
    };
    lathe.body(&mut p.pale, true);
    for i in rings {
        lathe.ring(&mut p.accent[col], &mut p.glow[col], i, w, halo * 0.6, off);
    }
}

//! Collision against the arena: axis-aligned boxes and ramps (wedges climbing along one of
//! the four grid directions). Rays for bolts and grenades, a kinematic cylinder for players.
//!
//! The arena is mostly static (found through a grid over XZ), plus a handful of dynamic boxes:
//! the moving doors and panels (`level::Mover`), which the sim replaces every tick. They are
//! few, so every query simply tests all of them.

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::kit::{self, Shape};

/// A collider in world space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Collider {
    Box {
        min: Vec3,
        max: Vec3,
    },
    /// A wedge over `min..max` whose top climbs from `min.y` to `max.y` in direction `dir`
    /// (0: +Z, 1: +X, 2: -Z, 3: -X).
    Ramp {
        min: Vec3,
        max: Vec3,
        dir: u8,
    },
}

impl Collider {
    /// `shape` (in a piece's frame) placed at `pos` turned by `rot` quarter turns.
    pub fn place(shape: &Shape, pos: Vec3, rot: u8) -> Collider {
        match *shape {
            Shape::Box { min, max } => {
                let (a, b) = kit::rotate_box(min, max, rot);
                Collider::Box {
                    min: a + pos,
                    max: b + pos,
                }
            }
            Shape::Ramp { min, max } => {
                let (a, b) = kit::rotate_box(min, max, rot);
                Collider::Ramp {
                    min: a + pos,
                    max: b + pos,
                    dir: rot % 4,
                }
            }
        }
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        match *self {
            Collider::Box { min, max } | Collider::Ramp { min, max, .. } => (min, max),
        }
    }

    /// 0..1 along the climbing direction of a ramp at (x, z).
    fn ramp_t(min: Vec3, max: Vec3, dir: u8, x: f32, z: f32) -> f32 {
        let t = match dir {
            0 => (z - min.z) / (max.z - min.z),
            1 => (x - min.x) / (max.x - min.x),
            2 => (max.z - z) / (max.z - min.z),
            _ => (max.x - x) / (max.x - min.x),
        };
        t.clamp(0.0, 1.0)
    }

    /// The top surface at (x, z), if (x, z) is over the collider.
    pub fn top_at(&self, x: f32, z: f32) -> Option<f32> {
        let (min, max) = self.bounds();
        if x < min.x || x > max.x || z < min.z || z > max.z {
            return None;
        }
        Some(self.top_clamped(x, z))
    }

    /// The top at the point of the footprint nearest (x, z).
    pub fn top_clamped(&self, x: f32, z: f32) -> f32 {
        match *self {
            Collider::Box { max, .. } => max.y,
            Collider::Ramp { min, max, dir } => {
                min.y + (max.y - min.y) * Self::ramp_t(min, max, dir, x, z)
            }
        }
    }

    /// Ray `o + d t` for t in [0, t_max]: the entry distance and the surface normal.
    pub fn ray(&self, o: Vec3, d: Vec3, t_max: f32) -> Option<(f32, Vec3)> {
        match *self {
            Collider::Box { min, max } => ray_box(o, d, min, max, t_max),
            Collider::Ramp { min, max, dir } => {
                // Clip against the wedge's planes (Cyrus-Beck): the box faces, then the slope.
                let (lo, hi) = (min, max);
                let rise = hi.y - lo.y;
                // Slope plane: y - lo.y <= rise * t(x,z), written as n·p <= c.
                let (n, c) = match dir {
                    0 => {
                        let s = rise / (hi.z - lo.z);
                        (Vec3::new(0.0, 1.0, -s), lo.y - s * lo.z)
                    }
                    1 => {
                        let s = rise / (hi.x - lo.x);
                        (Vec3::new(-s, 1.0, 0.0), lo.y - s * lo.x)
                    }
                    2 => {
                        let s = rise / (hi.z - lo.z);
                        (Vec3::new(0.0, 1.0, s), lo.y + s * hi.z)
                    }
                    _ => {
                        let s = rise / (hi.x - lo.x);
                        (Vec3::new(s, 1.0, 0.0), lo.y + s * hi.x)
                    }
                };
                let planes = [
                    (Vec3::NEG_X, -lo.x),
                    (Vec3::X, hi.x),
                    (Vec3::NEG_Y, -lo.y),
                    (Vec3::Y, hi.y),
                    (Vec3::NEG_Z, -lo.z),
                    (Vec3::Z, hi.z),
                    (n, c),
                ];
                clip_planes(o, d, t_max, &planes)
            }
        }
    }
}

/// Ray against a convex volume given as half-spaces n·p <= c.
fn clip_planes(o: Vec3, d: Vec3, t_max: f32, planes: &[(Vec3, f32)]) -> Option<(f32, Vec3)> {
    let mut t0 = 0.0f32;
    let mut t1 = t_max;
    let mut normal = Vec3::ZERO;
    for &(n, c) in planes {
        let dn = n.dot(d);
        let dist = c - n.dot(o);
        if dn.abs() < 1e-9 {
            if dist < 0.0 {
                return None;
            }
            continue;
        }
        let t = dist / dn;
        if dn < 0.0 {
            // Entering.
            if t > t0 {
                t0 = t;
                normal = n;
            }
        } else if t < t1 {
            t1 = t;
        }
        if t0 > t1 {
            return None;
        }
    }
    if normal == Vec3::ZERO {
        // Started inside.
        return None;
    }
    Some((t0, normal.normalize()))
}

pub fn ray_box(o: Vec3, d: Vec3, min: Vec3, max: Vec3, t_max: f32) -> Option<(f32, Vec3)> {
    let mut t0 = 0.0f32;
    let mut t1 = t_max;
    let mut normal = Vec3::ZERO;
    for i in 0..3 {
        if d[i].abs() < 1e-9 {
            if o[i] < min[i] || o[i] > max[i] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d[i];
        let (mut a, mut b) = ((min[i] - o[i]) * inv, (max[i] - o[i]) * inv);
        let mut n = Vec3::ZERO;
        n[i] = -d[i].signum();
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        if a > t0 {
            t0 = a;
            normal = n;
        }
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    if normal == Vec3::ZERO {
        return None;
    }
    Some((t0, normal))
}

/// Closest points between segments p1-q1 and p2-q2: returns the squared distance and the
/// parameter along the first segment.
pub fn segment_segment(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f32, f32) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-9 && e <= 1e-9 {
        return (r.length_squared(), 0.0);
    }
    if a <= 1e-9 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > 1e-9 {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    let c1 = p1 + d1 * s;
    let c2 = p2 + d2 * t;
    ((c1 - c2).length_squared(), s)
}

/// Player body for movement: a vertical cylinder.
pub const BODY_RADIUS: f32 = 0.32;
pub const BODY_HEIGHT: f32 = 1.75;
/// The highest ledge a player walks onto without noticing.
pub const STEP: f32 = 0.4;
pub const GRAVITY: f32 = 9.81;

/// The arena geometry: static colliders with a uniform grid over XZ to find nearby ones
/// quickly, and the dynamic ones (moving pieces, set every tick) tested one by one.
#[derive(Clone, Debug, Default)]
pub struct World {
    pub colliders: Vec<Collider>,
    /// The moving pieces' colliders where they are now (`set_dynamic`).
    pub dynamic: Vec<Collider>,
    origin: Vec2,
    cell: f32,
    dims: (usize, usize),
    grid: Vec<Vec<u32>>,
}

impl World {
    pub fn new(colliders: Vec<Collider>) -> World {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for c in &colliders {
            let (a, b) = c.bounds();
            min = min.min(Vec2::new(a.x, a.z));
            max = max.max(Vec2::new(b.x, b.z));
        }
        if colliders.is_empty() {
            min = Vec2::ZERO;
            max = Vec2::ONE;
        }
        let cell = 2.0;
        let nx = (((max.x - min.x) / cell).ceil() as usize).max(1);
        let nz = (((max.y - min.y) / cell).ceil() as usize).max(1);
        let mut w = World {
            colliders,
            dynamic: Vec::new(),
            origin: min,
            cell,
            dims: (nx, nz),
            grid: vec![Vec::new(); nx * nz],
        };
        for (i, c) in w.colliders.iter().enumerate() {
            let (a, b) = c.bounds();
            let (x0, z0) = w.cell_of(a.x, a.z);
            let (x1, z1) = w.cell_of(b.x, b.z);
            for z in z0..=z1 {
                for x in x0..=x1 {
                    w.grid[z * nx + x].push(i as u32);
                }
            }
        }
        w
    }

    /// Replaces the dynamic colliders (the moving pieces at the current tick).
    pub fn set_dynamic(&mut self, colliders: Vec<Collider>) {
        self.dynamic = colliders;
    }

    /// The dynamic colliders whose XZ bounds overlap the rectangle.
    fn near_dynamic(&self, min: Vec2, max: Vec2) -> impl Iterator<Item = &Collider> {
        self.dynamic.iter().filter(move |c| {
            let (a, b) = c.bounds();
            a.x <= max.x && b.x >= min.x && a.z <= max.y && b.z >= min.y
        })
    }

    fn cell_of(&self, x: f32, z: f32) -> (usize, usize) {
        let cx = ((x - self.origin.x) / self.cell)
            .floor()
            .clamp(0.0, (self.dims.0 - 1) as f32);
        let cz = ((z - self.origin.y) / self.cell)
            .floor()
            .clamp(0.0, (self.dims.1 - 1) as f32);
        (cx as usize, cz as usize)
    }

    /// Indices of colliders whose grid cells overlap the XZ rectangle.
    fn near(&self, min: Vec2, max: Vec2, out: &mut Vec<u32>) {
        out.clear();
        let (x0, z0) = self.cell_of(min.x, min.y);
        let (x1, z1) = self.cell_of(max.x, max.y);
        for z in z0..=z1 {
            for x in x0..=x1 {
                out.extend_from_slice(&self.grid[z * self.dims.0 + x]);
            }
        }
        out.sort_unstable();
        out.dedup();
    }

    /// The nearest hit of the segment `from`→`to`: distance fraction 0..1, point and normal.
    pub fn ray(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3, Vec3)> {
        let d = to - from;
        let len = d.length();
        if len < 1e-6 {
            return None;
        }
        let dir = d / len;
        let mut idx = Vec::new();
        let lo = from.min(to);
        let hi = from.max(to);
        self.near(Vec2::new(lo.x, lo.z), Vec2::new(hi.x, hi.z), &mut idx);
        let mut best: Option<(f32, Vec3)> = None;
        let near = idx.iter().map(|&i| &self.colliders[i as usize]);
        for c in near.chain(self.near_dynamic(Vec2::new(lo.x, lo.z), Vec2::new(hi.x, hi.z))) {
            let lim = best.map_or(len, |b| b.0);
            if let Some((t, n)) = c.ray(from, dir, lim) {
                best = Some((t, n));
            }
        }
        best.map(|(t, n)| (t / len, from + dir * t, n))
    }

    /// Is the straight line between two points clear?
    pub fn clear(&self, a: Vec3, b: Vec3) -> bool {
        self.ray(a, b).is_none()
    }

    /// The highest surface under (x, z) no higher than `below`, or `None` over a void.
    pub fn ground(&self, x: f32, z: f32, below: f32) -> Option<f32> {
        let mut idx = Vec::new();
        let p = Vec2::new(x, z);
        self.near(p, p, &mut idx);
        let mut best: Option<f32> = None;
        for c in idx
            .iter()
            .map(|&i| &self.colliders[i as usize])
            .chain(self.near_dynamic(p, p))
        {
            if let Some(top) = c.top_at(x, z)
                && top <= below
                && best.is_none_or(|b| top > b)
            {
                best = Some(top);
            }
        }
        best
    }

    /// Moves a standing player (feet at `feet`) by `delta` in XZ, then applies gravity.
    /// Returns the new feet position and vertical speed.
    pub fn move_body(&self, feet: Vec3, delta: Vec2, vy: f32, dt: f32) -> (Vec3, f32) {
        let mut p = feet;
        // Substeps keep a fast move from tunnelling through thin walls.
        let steps = ((delta.length() / (BODY_RADIUS * 0.5)).ceil() as usize).clamp(1, 8);
        let mut idx = Vec::new();
        for _ in 0..steps {
            p.x += delta.x / steps as f32;
            p.z += delta.y / steps as f32;
            let r = BODY_RADIUS + 0.05;
            let (lo, hi) = (Vec2::new(p.x - r, p.z - r), Vec2::new(p.x + r, p.z + r));
            self.near(lo, hi, &mut idx);
            let dynamic: Vec<&Collider> = self.near_dynamic(lo, hi).collect();
            for _ in 0..3 {
                let mut moved = false;
                for c in idx
                    .iter()
                    .map(|&i| &self.colliders[i as usize])
                    .chain(dynamic.iter().copied())
                {
                    let (min, max) = c.bounds();
                    if min.y >= p.y + BODY_HEIGHT {
                        continue;
                    }
                    // Nearest point of the footprint to the body's axis.
                    let nx = p.x.clamp(min.x, max.x);
                    let nz = p.z.clamp(min.z, max.z);
                    let top = c.top_clamped(nx, nz);
                    if top <= p.y + STEP {
                        continue;
                    }
                    let dx = p.x - nx;
                    let dz = p.z - nz;
                    let d2 = dx * dx + dz * dz;
                    if d2 >= BODY_RADIUS * BODY_RADIUS {
                        continue;
                    }
                    if d2 > 1e-8 {
                        let d = d2.sqrt();
                        let push = BODY_RADIUS - d;
                        p.x += dx / d * push;
                        p.z += dz / d * push;
                    } else {
                        // Axis inside the footprint: out through the nearest side.
                        let opts = [
                            (p.x - min.x + BODY_RADIUS, Vec2::new(-1.0, 0.0)),
                            (max.x - p.x + BODY_RADIUS, Vec2::new(1.0, 0.0)),
                            (p.z - min.z + BODY_RADIUS, Vec2::new(0.0, -1.0)),
                            (max.z - p.z + BODY_RADIUS, Vec2::new(0.0, 1.0)),
                        ];
                        let (d, n) = opts
                            .iter()
                            .copied()
                            .fold(opts[0], |a, b| if b.0 < a.0 { b } else { a });
                        p.x += n.x * d;
                        p.z += n.y * d;
                    }
                    moved = true;
                }
                if !moved {
                    break;
                }
            }
        }
        // Vertical: stand on the highest surface within a step, otherwise fall.
        let ground = self.ground(p.x, p.z, p.y + STEP).unwrap_or(-50.0);
        let mut vy = vy;
        // Walking (not falling): follow the floor up and down within a step.
        if vy == 0.0 && p.y - ground <= STEP * 0.5 {
            p.y = ground;
            return (p, 0.0);
        }
        vy -= GRAVITY * dt;
        p.y += vy * dt;
        if p.y <= ground {
            p.y = ground;
            vy = 0.0;
        }
        (p, vy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground_box() -> Collider {
        Collider::Box {
            min: Vec3::new(-20.0, -1.0, -20.0),
            max: Vec3::new(20.0, 0.0, 20.0),
        }
    }

    #[test]
    fn ray_hits_box_face() {
        let b = Collider::Box {
            min: Vec3::new(-1.0, 0.0, -0.1),
            max: Vec3::new(1.0, 3.0, 0.1),
        };
        let (t, n) = b.ray(Vec3::new(0.0, 1.0, 5.0), Vec3::NEG_Z, 10.0).unwrap();
        assert!((t - 4.9).abs() < 1e-4);
        assert_eq!(n, Vec3::Z);
        assert!(b.ray(Vec3::new(0.0, 4.0, 5.0), Vec3::NEG_Z, 10.0).is_none());
    }

    #[test]
    fn ray_hits_ramp_slope() {
        // Climbs along +Z from y=0 at z=-3 to y=3 at z=3.
        let r = Collider::Ramp {
            min: Vec3::new(-1.0, 0.0, -3.0),
            max: Vec3::new(1.0, 3.0, 3.0),
            dir: 0,
        };
        // Straight down at z=0: the slope is at y=1.5.
        let (t, n) = r.ray(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 10.0).unwrap();
        assert!((t - 3.5).abs() < 1e-4, "{t}");
        assert!(n.y > 0.8 && n.z < 0.0);
        // Above the slope near the low end, a horizontal ray at y=2 passes z=-2 freely.
        assert!(
            r.ray(Vec3::new(5.0, 2.0, -2.0), Vec3::NEG_X, 10.0)
                .is_none()
        );
        for dir in 0..4u8 {
            let q = crate::kit::turn(dir);
            let (a, b) =
                crate::kit::rotate_box(Vec3::new(-1.0, 0.0, -3.0), Vec3::new(1.0, 3.0, 3.0), dir);
            let c = Collider::Ramp {
                min: a,
                max: b,
                dir,
            };
            let p = q * Vec3::new(0.0, 0.0, 1.5);
            let top = c.top_at(p.x, p.z).unwrap();
            assert!((top - 2.25).abs() < 1e-4, "dir {dir}: {top}");
            let (t, _) = c.ray(Vec3::new(p.x, 5.0, p.z), Vec3::NEG_Y, 10.0).unwrap();
            assert!((t - (5.0 - 2.25)).abs() < 1e-3, "dir {dir}: {t}");
        }
    }

    #[test]
    fn body_blocked_by_wall_and_climbs_ramp() {
        let wall = Collider::Box {
            min: Vec3::new(-1.0, 0.0, -0.1),
            max: Vec3::new(1.0, 3.0, 0.1),
        };
        let w = World::new(vec![ground_box(), wall]);
        let mut p = Vec3::new(0.0, 0.0, 2.0);
        let mut vy = 0.0;
        for _ in 0..120 {
            (p, vy) = w.move_body(p, Vec2::new(0.0, -0.1), vy, 1.0 / 60.0);
        }
        assert!(p.z >= 0.1 + BODY_RADIUS - 1e-3, "{p}");

        let ramp = Collider::Ramp {
            min: Vec3::new(-1.0, 0.0, -3.0),
            max: Vec3::new(1.0, 3.0, 3.0),
            dir: 0,
        };
        let deck = Collider::Box {
            min: Vec3::new(-3.0, 2.8, 3.0),
            max: Vec3::new(3.0, 3.0, 7.0),
        };
        let w = World::new(vec![ground_box(), ramp, deck]);
        let mut p = Vec3::new(0.0, 0.0, -5.0);
        let mut vy = 0.0;
        for _ in 0..150 {
            (p, vy) = w.move_body(p, Vec2::new(0.0, 0.07), vy, 1.0 / 60.0);
        }
        assert!((p.y - 3.0).abs() < 1e-3 && p.z > 4.0, "{p}");
        // Beside the ramp's high end, the ramp is a wall.
        let mut p = Vec3::new(3.0, 0.0, 2.0);
        for _ in 0..100 {
            (p, vy) = w.move_body(p, Vec2::new(-0.07, 0.0), vy, 1.0 / 60.0);
        }
        assert!(p.x >= 1.0 + BODY_RADIUS - 1e-3 && p.y < 0.01, "{p}");
    }

    #[test]
    fn body_falls_through_hole() {
        let deck = Collider::Box {
            min: Vec3::new(-3.0, 2.8, -3.0),
            max: Vec3::new(3.0, 3.0, 3.0),
        };
        let w = World::new(vec![ground_box(), deck]);
        let mut p = Vec3::new(0.0, 3.0, 0.0);
        let mut vy = 0.0;
        for _ in 0..120 {
            (p, vy) = w.move_body(p, Vec2::new(0.1, 0.0), vy, 1.0 / 60.0);
        }
        assert!(p.y.abs() < 1e-3, "{p}");
    }
}

//! Shapes for the spires and contours sceneries: lathes (a profile turned around an axis, with
//! rings around them and ribs up them), floating rings, lamps and lasers across the sky.

use std::f32::consts::TAU;

use bevy::prelude::*;

use super::{aabox, basis, halo_quad, halo_ring, quad};
use crate::assets::MeshData;

/// A profile (radius, height along the axis), bottom to top, turned around `axis` from `base`
/// with `sides` faces (a polygon when few, smooth when many).
pub(super) struct Lathe {
    pub base: Vec3,
    pub axis: Vec3,
    pub sides: usize,
    /// The first meridian's angle.
    pub phase: f32,
    pub prof: Vec<Vec2>,
    s: Vec3,
    u: Vec3,
}

impl Lathe {
    pub fn new(base: Vec3, axis: Vec3, sides: usize, phase: f32, prof: Vec<Vec2>) -> Lathe {
        let axis = axis.normalize();
        let (s, u) = basis(axis);
        Lathe {
            base,
            axis,
            sides,
            phase,
            prof,
            s,
            u,
        }
    }

    pub fn vertical(base: Vec3, sides: usize, phase: f32, prof: Vec<Vec2>) -> Lathe {
        let mut l = Lathe::new(base, Vec3::Y, sides, phase, prof);
        // (Turned around the vertical: x and z across it.)
        l.s = Vec3::X;
        l.u = Vec3::Z;
        l
    }

    /// Outward at meridian j.
    pub fn radial(&self, j: usize) -> Vec3 {
        let a = self.phase + (j % self.sides) as f32 / self.sides as f32 * TAU;
        self.s * a.cos() + self.u * a.sin()
    }

    /// Across meridian j (towards j + 1).
    fn across(&self, j: usize) -> Vec3 {
        self.axis.cross(self.radial(j)).normalize()
    }

    pub fn point(&self, j: usize, i: usize) -> Vec3 {
        let p = self.prof[i];
        self.base + self.axis * p.y + self.radial(j) * p.x
    }

    /// Up the profile at i (in the profile's plane: radius, height).
    fn tangent2(&self, i: usize) -> Vec2 {
        let a = self.prof[i.saturating_sub(1)];
        let b = self.prof[(i + 1).min(self.prof.len() - 1)];
        (b - a).normalize_or(Vec2::Y)
    }

    /// Up the surface at meridian j, profile point i.
    fn tangent(&self, j: usize, i: usize) -> Vec3 {
        let t = self.tangent2(i);
        self.radial(j) * t.x + self.axis * t.y
    }

    /// Out of the surface at meridian j, profile point i (smooth).
    fn normal(&self, j: usize, i: usize) -> Vec3 {
        let t = self.tangent2(i);
        (self.radial(j) * t.y - self.axis * t.x).normalize_or(self.radial(j))
    }

    /// The surface: faceted (a normal per face) or smooth; a cap on top if it ends wide.
    pub fn body(&self, m: &mut MeshData, smooth: bool) {
        let n = self.prof.len();
        for i in 0..n - 1 {
            if self.prof[i].x < 1e-3 && self.prof[i + 1].x < 1e-3 {
                continue;
            }
            for j in 0..self.sides {
                let q = [
                    self.point(j, i),
                    self.point(j + 1, i),
                    self.point(j + 1, i + 1),
                    self.point(j, i + 1),
                ];
                // Out of the face (the diagonals' cross product is fine where an edge is a
                // point, at an apex).
                let mut face = (q[2] - q[0])
                    .cross(q[3] - q[1])
                    .normalize_or(self.radial(j));
                let mid = (q[0] + q[1] + q[2] + q[3]) / 4.0;
                let centre = self.base + self.axis * self.axis.dot(mid - self.base);
                if face.dot(mid - centre) < 0.0 {
                    face = -face;
                }
                if smooth {
                    let base = m.pos.len() as u32;
                    for (k, (jj, ii)) in [(j, i), (j + 1, i), (j + 1, i + 1), (j, i + 1)]
                        .into_iter()
                        .enumerate()
                    {
                        m.pos.push(q[k].to_array());
                        m.nrm.push(self.normal(jj, ii).to_array());
                        m.uv.push([0.0, 0.0]);
                    }
                    if (q[2] - q[0]).cross(q[3] - q[1]).dot(face) > 0.0 {
                        m.idx
                            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                    } else {
                        m.idx
                            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
                    }
                } else {
                    quad(m, q, [0.0; 4], face);
                }
            }
        }
        let top = n - 1;
        if self.prof[top].x > 1e-3 {
            let c = self.base + self.axis * self.prof[top].y;
            for j in 0..self.sides {
                quad(
                    m,
                    [c, self.point(j, top), self.point(j + 1, top), c],
                    [0.0; 4],
                    self.axis,
                );
            }
        }
    }

    /// A neon ring round profile point i, `w` wide, standing `off` off the surface, with halos
    /// `halo` wide up and down the surface.
    pub fn ring(
        &self,
        accent: &mut MeshData,
        glow: &mut MeshData,
        i: usize,
        w: f32,
        halo: f32,
        off: f32,
    ) {
        for j in 0..self.sides {
            let (n0, n1) = (self.normal(j, i), self.normal(j + 1, i));
            let (t0, t1) = (self.tangent(j, i), self.tangent(j + 1, i));
            let (p0, p1) = (self.point(j, i) + n0 * off, self.point(j + 1, i) + n1 * off);
            let h = w / 2.0;
            let n = (n0 + n1).normalize_or(n0);
            quad(
                accent,
                [p0 - t0 * h, p1 - t1 * h, p1 + t1 * h, p0 + t0 * h],
                [0.0; 4],
                n,
            );
            if halo > 0.0 {
                for sg in [1.0, -1.0] {
                    let (a0, a1) = (p0 + t0 * h * sg, p1 + t1 * h * sg);
                    halo_quad(glow, [a0, a1, a1 + t1 * halo * sg, a0 + t0 * halo * sg], n);
                }
            }
        }
    }

    /// A neon rib up meridian j from profile point i0 to i1, with halos across the surface.
    #[allow(clippy::too_many_arguments)]
    pub fn rib(
        &self,
        accent: &mut MeshData,
        glow: &mut MeshData,
        j: usize,
        i0: usize,
        i1: usize,
        w: f32,
        halo: f32,
        off: f32,
    ) {
        let x = self.across(j);
        let h = w / 2.0;
        for i in i0..i1 {
            let (n0, n1) = (self.normal(j, i), self.normal(j, i + 1));
            let (a, b) = (self.point(j, i) + n0 * off, self.point(j, i + 1) + n1 * off);
            let n = (n0 + n1).normalize_or(n0);
            quad(
                accent,
                [a - x * h, a + x * h, b + x * h, b - x * h],
                [0.0; 4],
                n,
            );
            if halo > 0.0 {
                for sg in [1.0, -1.0] {
                    let (e0, e1) = (a + x * h * sg, b + x * h * sg);
                    halo_quad(glow, [e0, e1, e1 + x * halo * sg, e0 + x * halo * sg], n);
                }
            }
        }
    }

    /// Little lights round profile point i, every `every`th meridian.
    pub fn dots(&self, accent: &mut MeshData, i: usize, every: usize, size: f32, off: f32) {
        for j in (0..self.sides).step_by(every.max(1)) {
            // (A square facing out: they are small.)
            let n = self.normal(j, i);
            let c = self.point(j, i) + n * off * 1.5;
            let (x, t) = (self.across(j) * size, self.tangent(j, i) * size);
            quad(
                accent,
                [c - x - t, c + x - t, c + x + t, c - x + t],
                [0.0; 4],
                n,
            );
        }
    }
}

/// A ring floating in the air: a neon tube of radius `t` round `axis` at `c`, `r` across, with
/// halos fading out from it in its plane and along its axis.
pub(super) fn halo_hoop(
    accent: &mut MeshData,
    glow: &mut MeshData,
    c: Vec3,
    axis: Vec3,
    r: f32,
    t: f32,
    halo: f32,
) {
    let seg = ((r * 2.0) as usize).clamp(32, 96);
    let tube = 4;
    let (s, u) = basis(axis);
    let at = |a: f32, b: f32| {
        let radial = s * a.cos() + u * a.sin();
        let n = radial * b.cos() + axis * b.sin();
        (c + radial * r + n * t, n)
    };
    let base = accent.pos.len() as u32;
    for i in 0..=seg {
        for k in 0..=tube {
            let (p, n) = at(
                i as f32 / seg as f32 * TAU,
                k as f32 / tube as f32 * TAU + TAU / 8.0,
            );
            accent.pos.push(p.to_array());
            accent.nrm.push(n.to_array());
            accent.uv.push([0.0, 0.0]);
        }
    }
    let row = tube as u32 + 1;
    for i in 0..seg as u32 {
        for k in 0..tube as u32 {
            let a = base + i * row + k;
            let (b, cc, d) = (a + row, a + row + 1, a + 1);
            let p = |v: u32| Vec3::from(accent.pos[v as usize]);
            let out = (p(b) - p(a))
                .cross(p(cc) - p(a))
                .dot(Vec3::from(accent.nrm[a as usize]));
            if out > 0.0 {
                accent.idx.extend([a, b, cc, a, cc, d]);
            } else {
                accent.idx.extend([a, cc, b, a, d, cc]);
            }
        }
    }
    // Flat halos in its plane (in and out), and bands up and down its axis.
    halo_ring(glow, c, axis, r + t, r + t + halo, seg);
    halo_ring(glow, c, axis, r - t, (r - t - halo).max(r * 0.5), seg);
    for sg in [1.0, -1.0] {
        for i in 0..seg {
            let a0 = i as f32 / seg as f32 * TAU;
            let a1 = (i + 1) as f32 / seg as f32 * TAU;
            let d0 = s * a0.cos() + u * a0.sin();
            let d1 = s * a1.cos() + u * a1.sin();
            let (p0, p1) = (c + d0 * r + axis * t * sg, c + d1 * r + axis * t * sg);
            let up = axis * halo * 0.8 * sg;
            halo_quad(glow, [p0, p1, p1 + up, p0 + up], d0);
        }
    }
}

/// A small lamp: a bright cube and its glow seen from anywhere.
pub(super) fn lamp(accent: &mut MeshData, glow: &mut MeshData, c: Vec3, size: f32, halo: f32) {
    aabox(accent, c - Vec3::splat(size), c + Vec3::splat(size));
    if halo > 0.0 {
        for n in [Vec3::X, Vec3::Y, Vec3::Z] {
            halo_ring(glow, c, n, size, size + halo, 6);
        }
    }
}

/// A laser across the sky from `a` to `b`: a thin bright line with a glow round it.
pub(super) fn laser(
    accent: &mut MeshData,
    glow: &mut MeshData,
    a: Vec3,
    b: Vec3,
    t: f32,
    halo: f32,
) {
    super::beam(accent, a, b, t);
    let (s, u) = basis((b - a).normalize());
    for d in [s, -s, u, -u] {
        let n = if d.dot(s).abs() > 0.5 { u } else { s };
        halo_quad(glow, [a, b, b + d * halo, a + d * halo], n);
    }
}

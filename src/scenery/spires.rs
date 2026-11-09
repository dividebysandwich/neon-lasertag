//! Spires: tall towers on flared and conical bases, ribbed with neon from the ground up, rings
//! of light with little lamps round their joints, glowing hoops floating round their tops,
//! lamps scattered over the ground and lasers across the sky.

use std::f32::consts::{FRAC_PI_4, TAU};

use bevy::prelude::*;
use rand::Rng;
use rand::rngs::StdRng;

use super::shapes::{Lathe, halo_hoop, lamp, laser};
use super::{
    C_BLUE, C_PINK, C_RED, C_WHITE, CYAN, ORANGE, PURPLE, Parts, arena_distance, beam, lerp,
};

/// The colours of one spires scenery.
struct Colours {
    /// Ribs (the first most often).
    ribs: [usize; 2],
    rings: usize,
    dots: usize,
    hoops: usize,
}

pub(super) fn spires(p: &mut Parts, rng: &mut StdRng) {
    let c = match rng.random_range(0..4) {
        0 => Colours {
            ribs: [C_PINK, PURPLE],
            rings: C_RED,
            dots: ORANGE,
            hoops: C_RED,
        },
        1 => Colours {
            ribs: [C_PINK, C_PINK],
            rings: ORANGE,
            dots: C_RED,
            hoops: C_RED,
        },
        2 => Colours {
            ribs: [CYAN, C_BLUE],
            rings: C_PINK,
            dots: ORANGE,
            hoops: C_PINK,
        },
        _ => Colours {
            ribs: [PURPLE, C_PINK],
            rings: CYAN,
            dots: ORANGE,
            hoops: ORANGE,
        },
    };
    let mut placed: Vec<(Vec2, f32)> = Vec::new();
    let want = rng.random_range(22..35);
    for _ in 0..6000 {
        if placed.len() >= want {
            break;
        }
        let at = Vec2::new(
            rng.random_range(-230.0..230.0),
            rng.random_range(-230.0..230.0),
        );
        let d = arena_distance(p.half, at);
        if !(22.0..220.0).contains(&d) || at.length() < 30.0 {
            continue;
        }
        let k = (d / 200.0).clamp(0.0, 1.0);
        let r = lerp(6.0, 15.0, k) * rng.random_range(0.7..1.3);
        if d < r + 8.0 {
            continue;
        }
        let gap = 3.0 + d * 0.03;
        if placed.iter().any(|(q, s)| q.distance(at) < r + s + gap) {
            continue;
        }
        placed.push((at, r));
        tower(p, rng, at, d, r, &c);
    }
    info!("scenery: {} spires", placed.len());
    // Lamps scattered over the ground.
    let mut n = 0;
    for _ in 0..3000 {
        if n >= 160 {
            break;
        }
        let at = Vec2::new(
            rng.random_range(-220.0..220.0),
            rng.random_range(-220.0..220.0),
        );
        let d = arena_distance(p.half, at);
        if d < 4.0 || placed.iter().any(|(q, s)| q.distance(at) < s + 1.0) {
            continue;
        }
        n += 1;
        let size = 0.1 + d * 0.0015;
        let col = if rng.random_bool(0.7) {
            c.dots
        } else {
            C_WHITE
        };
        lamp(
            &mut p.accent[col],
            &mut p.glow[col],
            Vec3::new(at.x, size + 0.02, at.y),
            size,
            size * 4.0,
        );
    }
    lasers(p, rng);
}

/// One or two lasers across the sky, from far beyond the arena.
pub(super) fn lasers(p: &mut Parts, rng: &mut StdRng) {
    for _ in 0..rng.random_range(1..=2) {
        let a0 = rng.random_range(0.0..TAU);
        let a1 = a0 + std::f32::consts::PI + rng.random_range(-0.8..0.8);
        let from = Vec3::new(a0.cos() * 300.0, 0.0, a0.sin() * 300.0);
        let to = Vec3::new(
            a1.cos() * 320.0,
            rng.random_range(260.0..420.0),
            a1.sin() * 320.0,
        );
        let col = if rng.random_bool(0.7) { C_BLUE } else { CYAN };
        laser(&mut p.accent[col], &mut p.glow[col], from, to, 0.45, 5.0);
    }
}

/// A tower at `at`, `d` metres from the arena, its base `r` across.
fn tower(p: &mut Parts, rng: &mut StdRng, at: Vec2, d: f32, r: f32, c: &Colours) {
    let k = (d / 200.0).clamp(0.0, 1.0);
    let h = lerp(32.0, 105.0, k) * rng.random_range(0.6..1.4);
    let w = 0.1 + d * 0.0022;
    let halo = 0.5 + d / 70.0;
    let off = 0.03 + d * 0.0007;
    let base = Vec3::new(at.x, 0.0, at.y);
    let mut prof = Vec::new();
    // The profile's points where the base ends and where it steps in.
    let mut joints = Vec::new();
    let kind = rng.random_range(0..100);
    let (sides, phase) = match kind {
        0..20 => (4, FRAC_PI_4 + rng.random_range(-0.3..0.3)),
        _ => ([8, 12, 16, 20, 24][rng.random_range(0..5)], 0.0),
    };
    let hs = h * rng.random_range(0.18..0.34);
    let r1 = r * rng.random_range(0.2..0.34);
    if (20..65).contains(&kind) {
        // A flared skirt.
        let bend = rng.random_range(1.6..2.6);
        for i in 0..=5 {
            let t = i as f32 / 5.0;
            prof.push(Vec2::new(r1 + (r - r1) * (1.0 - t).powf(bend), hs * t));
        }
    } else {
        prof.push(Vec2::new(r, 0.0));
        prof.push(Vec2::new(r1, hs));
    }
    let skirt_top = prof.len() - 1;
    joints.push(skirt_top);
    // A collar on top of it.
    let collar = (1.4 + d * 0.01).min(h * 0.05);
    let rc = r1 * rng.random_range(1.05..1.2);
    prof.push(Vec2::new(rc, hs));
    prof.push(Vec2::new(rc, hs + collar));
    joints.push(prof.len() - 1);
    // The shaft, perhaps with a setback (or a second, smaller flare) on the way up.
    let mut rs = r1 * rng.random_range(0.7..0.9);
    let mut y = hs + collar;
    prof.push(Vec2::new(rs, y));
    if rng.random_bool(0.6) {
        let ym = lerp(y, h, rng.random_range(0.35..0.65));
        prof.push(Vec2::new(rs * rng.random_range(0.85..1.0), ym));
        if rng.random_bool(0.5) {
            // A small flare.
            prof.push(Vec2::new(rs * 1.6, ym));
            let y2 = ym + h * rng.random_range(0.06..0.12);
            rs *= rng.random_range(0.55..0.75);
            prof.push(Vec2::new(rs, y2));
            joints.push(prof.len() - 2);
            y = y2;
        } else {
            rs *= rng.random_range(0.6..0.8);
            prof.push(Vec2::new(rs, ym));
            joints.push(prof.len() - 1);
            y = ym;
        }
    }
    let rt = rs * rng.random_range(0.55..0.95);
    let top_y = h.max(y + 6.0);
    prof.push(Vec2::new(rt, top_y));
    let top = prof.len() - 1;
    let lathe = Lathe::vertical(base, sides, phase, prof);
    lathe.body(&mut p.pale, false);
    // Ribs up the base (every face's edge, or every other on the many-sided), and perhaps up
    // the shaft.
    let rib = if rng.random_bool(0.8) {
        c.ribs[0]
    } else {
        c.ribs[1]
    };
    let step = if sides > 16 { 2 } else { 1 };
    for j in (0..sides).step_by(step) {
        lathe.rib(
            &mut p.accent[rib],
            &mut p.glow[rib],
            j,
            0,
            skirt_top,
            w,
            halo * 0.6,
            off,
        );
    }
    if rng.random_bool(0.55) {
        let step = if sides <= 4 { 1 } else { (sides / 4).max(2) };
        let from = skirt_top + 3;
        for j in (0..sides).step_by(step) {
            lathe.rib(
                &mut p.accent[rib],
                &mut p.glow[rib],
                j,
                from,
                top,
                w * 0.8,
                halo * 0.5,
                off,
            );
        }
    }
    // Rings with lamps round the foot and the joints.
    let ring_w = w * 1.6;
    lathe.ring(
        &mut p.accent[c.rings],
        &mut p.glow[c.rings],
        0,
        ring_w,
        halo,
        off * 2.0,
    );
    lathe.dots(&mut p.accent[c.dots], 0, 1, w * 1.2, off * 2.0);
    for &i in &joints {
        lathe.ring(
            &mut p.accent[c.rings],
            &mut p.glow[c.rings],
            i,
            ring_w,
            halo,
            off * 2.0,
        );
        if rng.random_bool(0.6) {
            lathe.dots(&mut p.accent[c.dots], i, 1, w * 1.1, off * 2.0);
        }
    }
    // Bright lamps round the collar.
    if rng.random_bool(0.7) {
        let i = joints[1];
        let every = (sides / 4).max(1);
        for j in (0..sides).step_by(every) {
            let at = lathe.point(j, i) + lathe.radial(j) * (w * 3.0);
            lamp(
                &mut p.accent[C_WHITE],
                &mut p.glow[C_WHITE],
                at,
                w * 2.2,
                halo * 1.6,
            );
        }
    }
    // The top: a mast with a lamp at its tip.
    let tip_y = top_y + h * rng.random_range(0.05..0.18);
    let t0 = base + Vec3::Y * top_y;
    let t1 = base + Vec3::Y * tip_y;
    beam(&mut p.pale, t0, t1, (rt * 0.25).clamp(0.2, 0.8));
    let tip = if rng.random_bool(0.5) {
        C_WHITE
    } else {
        ORANGE
    };
    lamp(
        &mut p.accent[tip],
        &mut p.glow[tip],
        t1 + Vec3::Y * w * 2.0,
        w * 2.5,
        halo * 2.0,
    );
    // A hoop floating round the top.
    if rng.random_bool(0.55) {
        let hr = (rt * 3.0).max(r * rng.random_range(0.8..2.4));
        let hy = top_y + h * rng.random_range(-0.15..0.25);
        halo_hoop(
            &mut p.accent[c.hoops],
            &mut p.glow[c.hoops],
            base + Vec3::Y * hy,
            Vec3::Y,
            hr,
            0.2 + d * 0.003,
            1.5 + d / 40.0,
        );
    }
}

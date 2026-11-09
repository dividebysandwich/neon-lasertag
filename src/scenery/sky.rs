//! The sky: black, or a gradient glowing at the horizon and fading out to black as it rises,
//! on a dome around everything (vertex coloured, `NeonUniform::vertex_coloured`). Above the
//! gradient's top the clear colour (black) shows.

use bevy::prelude::*;
use rand::Rng;
use rand::rngs::StdRng;

use crate::assets::MeshData;

/// The dome's radius (m): beyond the ground and everything on it, inside the far plane.
const RADIUS: f32 = 800.0;
/// How far the dome reaches below the horizon (degrees): hidden by the ground.
const BELOW: f32 = -12.0;
const SEGMENTS: usize = 64;

/// A gradient's colours from the horizon up (linear, at full strength); black follows the last.
const GRADIENTS: [(&str, &[Vec3]); 8] = [
    (
        "sunset",
        &[
            Vec3::new(1.0, 0.72, 0.12),
            Vec3::new(1.0, 0.3, 0.02),
            Vec3::new(0.55, 0.02, 0.03),
        ],
    ),
    (
        "ember",
        &[
            Vec3::new(1.0, 0.25, 0.02),
            Vec3::new(0.6, 0.02, 0.06),
            Vec3::new(0.18, 0.0, 0.12),
        ],
    ),
    (
        "dusk",
        &[
            Vec3::new(0.75, 0.15, 0.9),
            Vec3::new(0.2, 0.08, 0.8),
            Vec3::new(0.02, 0.03, 0.35),
        ],
    ),
    (
        "aurora",
        &[
            Vec3::new(0.2, 1.0, 0.35),
            Vec3::new(0.02, 0.6, 0.6),
            Vec3::new(0.02, 0.1, 0.45),
        ],
    ),
    (
        "ice",
        &[
            Vec3::new(0.3, 0.9, 1.0),
            Vec3::new(0.05, 0.3, 0.9),
            Vec3::new(0.02, 0.04, 0.3),
        ],
    ),
    (
        "rose",
        &[
            Vec3::new(1.0, 0.3, 0.55),
            Vec3::new(0.6, 0.04, 0.45),
            Vec3::new(0.15, 0.02, 0.3),
        ],
    ),
    (
        "toxic",
        &[
            Vec3::new(0.8, 1.0, 0.1),
            Vec3::new(0.15, 0.7, 0.1),
            Vec3::new(0.0, 0.2, 0.15),
        ],
    ),
    (
        "infra",
        &[
            Vec3::new(1.0, 0.08, 0.1),
            Vec3::new(0.4, 0.02, 0.25),
            Vec3::new(0.08, 0.0, 0.2),
        ],
    ),
];

/// A sky: its stops (elevation in degrees, colour) from the horizon up, the last black.
pub(super) struct Sky {
    pub name: &'static str,
    stops: Vec<(f32, Vec3)>,
}

/// A sky for this arena: one time in four black (`LT_SKY=<name>|black` picks one).
pub(super) fn pick(rng: &mut StdRng) -> Option<Sky> {
    let forced = std::env::var("LT_SKY").ok();
    let i = match forced.as_deref() {
        Some("black") => return None,
        Some(name) => GRADIENTS.iter().position(|g| g.0 == name),
        None => None,
    };
    let i = match i {
        Some(i) => i,
        None => {
            if rng.random_bool(0.25) {
                return None;
            }
            rng.random_range(0..GRADIENTS.len())
        }
    };
    let (name, colours) = GRADIENTS[i];
    // How high it reaches and how strong it is at the horizon.
    let height = rng.random_range(16.0..34.0);
    let strength = rng.random_range(0.22..0.42);
    let mut stops = Vec::new();
    let n = colours.len();
    for (k, c) in colours.iter().enumerate() {
        // Most of the colour close to the horizon; dimmer with every stop.
        let t = (k as f32 / n as f32).powf(1.6);
        let fade = 1.0 - 0.55 * k as f32 / n as f32;
        stops.push((t * height, *c * strength * fade));
    }
    stops.push((height, Vec3::ZERO));
    Some(Sky { name, stops })
}

impl Sky {
    fn colour(&self, elev: f32) -> Vec3 {
        let s = &self.stops;
        if elev <= s[0].0 {
            return s[0].1;
        }
        for w in s.windows(2) {
            let ((e0, c0), (e1, c1)) = (w[0], w[1]);
            if elev <= e1 {
                let t = (elev - e0) / (e1 - e0);
                // Smooth between the stops.
                return c0.lerp(c1, t * t * (3.0 - 2.0 * t));
            }
        }
        Vec3::ZERO
    }

    /// The dome, wound to face its centre.
    pub(super) fn dome(&self, m: &mut MeshData) {
        let top = self.stops.last().map_or(30.0, |s| s.0);
        // Rings: closer together near the horizon, where the colour changes fastest.
        let mut elevs = vec![BELOW, 0.0];
        let steps = 14;
        for k in 1..=steps {
            elevs.push(top * (k as f32 / steps as f32).powf(1.4));
        }
        let base = m.pos.len() as u32;
        for &e in &elevs {
            let (se, ce) = e.to_radians().sin_cos();
            let c = self.colour(e);
            for i in 0..=SEGMENTS {
                let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let d = Vec3::new(a.cos() * ce, se, a.sin() * ce);
                m.pos.push((d * RADIUS).to_array());
                m.nrm.push((-d).to_array());
                m.uv.push([i as f32 / SEGMENTS as f32, 0.0]);
                m.col.push([c.x, c.y, c.z, 1.0]);
            }
        }
        let row = SEGMENTS as u32 + 1;
        for r in 0..elevs.len() as u32 - 1 {
            for i in 0..SEGMENTS as u32 {
                let a = base + r * row + i;
                let b = a + 1;
                let (c, d) = (b + row, a + row);
                // Seen from inside (counter-clockwise from the centre).
                m.idx.extend([a, b, c, a, c, d]);
            }
        }
    }
}

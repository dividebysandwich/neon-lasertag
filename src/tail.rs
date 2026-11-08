//! The avatars' tails: a chain of rigid pieces from the pelvis's back (`assets::TailModel`,
//! tools/models/avatars.py), swung about their joints by a damped spring each, so a tail lags
//! behind as its player runs, turns, jumps and stops, and flops about a little before it settles.
//! A canine's wags for a while after its player tags someone out. Cosmetic only, as the rest of
//! the body (src/rig.rs): nothing is networked, and every client swings its own.
//!
//! Each joint is a point simulated in the world: pulled towards where its piece would hold it
//! at rest from the piece before (as that piece is now), kept its piece's length from that
//! joint, and never bent too far from rest.

use bevy::prelude::*;

use crate::assets::Model;

/// A tail's pieces (each from its joint along -Z, +Y its top) and its joints at rest in the
/// pelvis's frame (`rig::Legs::pelvis`): one per piece and the tip.
#[derive(Clone, Default)]
pub struct TailModel {
    pub pieces: Vec<Model>,
    pub joints: Vec<Transform>,
    pub swing: Swing,
}

impl TailModel {
    /// The joints' poses at rest (in the pelvis's frame), as `Tail::update` gives them.
    pub fn rest(&self) -> Vec<Transform> {
        self.joints
            .iter()
            .take(self.pieces.len())
            .copied()
            .collect()
    }
}

/// How a tail swings: its joints' springs (Hz) and damping (of critical), and whether it wags.
#[derive(Clone, Copy, Debug)]
pub struct Swing {
    pub freq: f32,
    pub damping: f32,
    pub wags: bool,
}

impl Default for Swing {
    fn default() -> Self {
        Swing {
            freq: 4.0,
            damping: 0.4,
            wags: false,
        }
    }
}

/// Each animal's tail swing: long ones loose, stubs stiff, the canines (wolf, fox) wag.
pub fn swing(animal: &str) -> Swing {
    let (freq, damping, wags) = match animal {
        "wolf" => (3.2, 0.3, true),
        "fox" => (2.8, 0.3, true),
        "cat" => (2.4, 0.22, false),
        "dragon" => (1.9, 0.45, false),
        "boar" => (4.0, 0.15, false),
        "deer" => (7.0, 0.25, false),
        "rabbit" => (9.0, 0.2, false),
        "bear" => (9.0, 0.25, false),
        "owl" => (6.0, 0.35, false),
        _ => return Swing::default(),
    };
    Swing {
        freq,
        damping,
        wags,
    }
}

/// The simulation's step (s), however long the frames.
const STEP: f32 = 1.0 / 240.0;
/// At most this many steps a frame (a long hitch is not caught up).
const MAX_STEPS: u32 = 24;
/// Air drag (1/s): running, a tail streams back a little.
const DRAG: f32 = 1.5;
/// How far a joint may bend from its rest (rad).
const MAX_BEND: f32 = 1.2;
/// Wagging after a tag-out: how long (s), how fast (Hz) and how far either side at the root
/// (rad); it eases in and out over `WAG_EASE` s.
pub const WAG_TIME: f32 = 4.0;
const WAG_FREQ: f32 = 3.2;
const WAG_SWING: f32 = 0.35;
const WAG_EASE: f32 = 0.35;

/// One avatar's tail as it swings.
#[derive(Clone, Debug, Default)]
pub struct Tail {
    /// The joints in the world (the root's following the pelvis) and where they were a step
    /// before, the time not yet stepped, and each piece's turn as last stepped.
    pts: Vec<Vec3>,
    prev: Vec<Vec3>,
    left: f32,
    rots: Vec<Quat>,
    /// Wagging: seconds left, how much (0..1, eased), and the wag's phase (rad).
    wag_left: f32,
    wag: f32,
    phase: f32,
}

impl Tail {
    /// Wags (if this tail does) for `WAG_TIME` from now.
    pub fn wag(&mut self) {
        self.wag_left = WAG_TIME;
    }

    /// Advances by `dt` with the pelvis at `pelvis` (world) and poses the pieces (world).
    pub fn update(&mut self, model: &TailModel, dt: f32, pelvis: &Transform) -> Vec<Transform> {
        let n = model.pieces.len();
        if n == 0 || model.joints.len() < n + 1 {
            return Vec::new();
        }
        let lengths: Vec<f32> = (0..n)
            .map(|k| {
                (model.joints[k + 1].translation - model.joints[k].translation)
                    .length()
                    .max(1e-3)
            })
            .collect();
        let root = pelvis.transform_point(model.joints[0].translation);
        // New, or carried off (respawned, a new arena): at rest.
        if self.pts.len() != n + 1 || self.pts[0].distance(root) > 1.0 {
            let rest: Vec<Vec3> = model
                .joints
                .iter()
                .take(n + 1)
                .map(|j| pelvis.transform_point(j.translation))
                .collect();
            self.pts = rest.clone();
            self.prev = rest;
            self.rots = (0..n)
                .map(|k| pelvis.rotation * model.joints[k].rotation)
                .collect();
            self.left = 0.0;
        }
        // Wagging, eased in and out.
        self.wag_left = (self.wag_left - dt).max(0.0);
        let want = if model.swing.wags && self.wag_left > 0.0 {
            1.0
        } else {
            0.0
        };
        self.wag += (want - self.wag) * (1.0 - (-dt / WAG_EASE).exp());
        if self.wag > 1e-3 {
            self.phase = (self.phase + std::f32::consts::TAU * WAG_FREQ * dt)
                .rem_euclid(std::f32::consts::TAU);
        }
        let wag = Quat::from_rotation_y(WAG_SWING * self.wag * self.phase.sin());
        let w = std::f32::consts::TAU * model.swing.freq;
        let (stiff, damp) = (w * w, 2.0 * model.swing.damping * w);
        // The root's frame: the pelvis's, swung side to side about its upright if wagging.
        let root_rot = pelvis.rotation * wag * model.joints[0].rotation;
        // (The pelvis's travel this frame spread over the steps, so the root moves smoothly.)
        let from = self.pts[0];
        self.left += dt;
        let steps = ((self.left / STEP) as u32).min(MAX_STEPS);
        self.left = if steps == MAX_STEPS {
            0.0
        } else {
            self.left - steps as f32 * STEP
        };
        let root_step = if steps > 0 {
            (root - from) / steps as f32
        } else {
            Vec3::ZERO
        };
        for s in 0..steps {
            self.pts[0] = from.lerp(root, (s + 1) as f32 / steps as f32);
            let mut parent = root_rot;
            for (k, &length) in lengths.iter().enumerate() {
                // At rest from the piece before as it is now.
                let rest = if k == 0 {
                    root_rot
                } else {
                    parent * (model.joints[k - 1].rotation.inverse() * model.joints[k].rotation)
                };
                let along = rest * Vec3::NEG_Z;
                let target = self.pts[k] + along * length;
                let p = self.pts[k + 1];
                // Damped as it moves against the root (and dragged through the air).
                let vel = p - self.prev[k + 1];
                let vel = root_step + (vel - root_step) * (1.0 - damp * STEP).max(0.0);
                let vel = vel * (1.0 - DRAG * STEP);
                let mut next = p + vel + (target - p) * stiff * STEP * STEP;
                // Its length from the joint before, bent no further than MAX_BEND.
                let mut dir = (next - self.pts[k]).normalize_or(along);
                let bend = dir.angle_between(along);
                if bend > MAX_BEND {
                    let axis = along.cross(dir).normalize_or(rest * Vec3::X);
                    dir = Quat::from_axis_angle(axis, MAX_BEND) * along;
                }
                next = self.pts[k] + dir * length;
                self.prev[k + 1] = p;
                self.pts[k + 1] = next;
                parent = Quat::from_rotation_arc(along, dir) * rest;
                self.rots[k] = parent;
            }
        }
        // (Shifted by the pelvis's travel not stepped yet, so the root stays on it.)
        let behind = root - self.pts[0];
        (0..n)
            .map(|k| Transform::from_translation(self.pts[k] + behind).with_rotation(self.rots[k]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight tail of three 0.1 m pieces straight back from the pelvis.
    fn model(wags: bool) -> TailModel {
        let back = Quat::from_rotation_y(std::f32::consts::PI);
        TailModel {
            pieces: vec![Model::default(); 3],
            joints: (0..4)
                .map(|k| Transform::from_xyz(0.0, 0.0, 0.1 * k as f32).with_rotation(back))
                .collect(),
            swing: Swing {
                freq: 3.0,
                damping: 0.3,
                wags,
            },
        }
    }

    fn tip(t: &Tail) -> Vec3 {
        *t.pts.last().unwrap()
    }

    #[test]
    fn rests_lags_and_settles() {
        let m = model(false);
        let mut t = Tail::default();
        let dt = 1.0 / 90.0;
        let mut pelvis = Transform::from_xyz(0.0, 1.0, 0.0);
        for _ in 0..90 {
            t.update(&m, dt, &pelvis);
        }
        assert!(tip(&t).distance(Vec3::new(0.0, 1.0, 0.3)) < 1e-3);
        // Setting off sideways (+X): the tail trails, still its length.
        for _ in 0..10 {
            pelvis.translation.x += 6.0 * dt;
            let poses = t.update(&m, dt, &pelvis);
            assert_eq!(poses.len(), 3);
        }
        assert!(tip(&t).x < pelvis.translation.x - 0.05, "{}", tip(&t));
        // Running on, it comes round behind again, streaming back a little.
        for _ in 0..180 {
            pelvis.translation.x += 6.0 * dt;
            t.update(&m, dt, &pelvis);
        }
        let lag = pelvis.translation.x - tip(&t).x;
        assert!((0.0..0.03).contains(&lag), "{lag}");
        let rest = pelvis.translation + Vec3::new(0.0, 0.0, 0.3);
        for w in t.pts.windows(2) {
            assert!((w[0].distance(w[1]) - 0.1).abs() < 1e-3);
        }
        // Stopped: it settles back.
        for _ in 0..360 {
            t.update(&m, dt, &pelvis);
        }
        assert!(tip(&t).distance(rest) < 2e-3);
    }

    #[test]
    fn turning_swings_it_round_late() {
        let m = model(false);
        let mut t = Tail::default();
        let dt = 1.0 / 90.0;
        let mut pelvis = Transform::from_xyz(0.0, 1.0, 0.0);
        t.update(&m, dt, &pelvis);
        pelvis.rotation = Quat::from_rotation_y(1.0);
        t.update(&m, dt, &pelvis);
        let rest = pelvis.transform_point(Vec3::new(0.0, 0.0, 0.3));
        assert!(tip(&t).distance(rest) > 0.05);
    }

    #[test]
    fn only_canines_wag() {
        let dt = 1.0 / 90.0;
        let pelvis = Transform::from_xyz(0.0, 1.0, 0.0);
        for wags in [false, true] {
            let m = model(wags);
            let mut t = Tail::default();
            t.wag();
            let mut widest = 0.0f32;
            for _ in 0..180 {
                t.update(&m, dt, &pelvis);
                widest = widest.max(tip(&t).x.abs());
            }
            assert_eq!(widest > 0.05, wags, "{widest}");
        }
    }
}

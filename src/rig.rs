//! The avatars' bodies: rigid segments (neck, arms, pelvis, legs, boots) posed every frame
//! between the parts the game already knows about (the head, the chest hanging from the neck,
//! the gloves) by two-bone IK and a procedural gait. Cosmetic only: `lt_sim::hitbox` never sees
//! any of it, and nothing new is networked; speed, crouching and jumps are read off the feet and
//! head the snapshots already carry.
//!
//! Segment meshes (`assets::BodyModels`) hang from their joint down -Y with the front -Z;
//! [`segment`] turns and stretches one between two joints.

use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The neck, from the head's pivot down into the vest's collar (its mesh's length).
pub const NECK: f32 = 0.2;
/// The shoulder joints in the chest's frame (top centre, front -Z); X mirrored on the left.
pub const SHOULDER: Vec3 = Vec3::new(0.2, -0.08, 0.0);
pub const UPPER_ARM: f32 = 0.27;
pub const FOREARM: f32 = 0.25;
/// The wrist in a glove's frame: inside the cuff, behind the palm.
pub const WRIST: Vec3 = Vec3::new(-0.002, 0.0, 0.075);
/// The pelvis's top below the chest's top (tucked into the vest's hem, 0.5 m down).
pub const PELVIS_DROP: f32 = 0.46;
/// The hip joints in the pelvis's frame (top centre); X mirrored on the left.
pub const HIP: Vec3 = Vec3::new(0.095, -0.12, 0.0);
/// Bone lengths of an average player's legs (the meshes are a little longer: [`THIGH_MESH`]).
pub const THIGH: f32 = 0.36;
pub const SHIN: f32 = 0.35;
pub const THIGH_MESH: f32 = 0.38;
pub const SHIN_MESH: f32 = 0.37;
/// The ankle above the sole.
pub const ANKLE: f32 = 0.08;
/// How far a limb stretches beyond its length before the hand or foot lets go.
pub const STRETCH: f32 = 1.1;
/// The hips' height above the feet of a player standing straight with eyes at `EYE`.
pub const STAND_HIP: f32 = lt_sim::sim::EYE - lt_sim::hitbox::CHEST_DROP - PELVIS_DROP + HIP.y;
/// Half the distance between the feet, standing.
const STANCE: f32 = 0.11;
/// How far the hips may turn from the chest.
const MAX_TWIST: f32 = 0.9;
/// An idle foot's step: its duration and height.
const STEP_TIME: f32 = 0.22;
const STEP_LIFT: f32 = 0.07;

fn wrap(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The middle joint (elbow, knee) and the end (wrist, ankle) of a two-bone limb from `root`
/// reaching for `target`, bending towards `pole`. Out of reach it stretches by up to
/// [`STRETCH`], then the end falls short of the target.
pub fn two_bone(root: Vec3, target: Vec3, l1: f32, l2: f32, pole: Vec3) -> (Vec3, Vec3) {
    let d = target - root;
    let dir = d.normalize_or(Vec3::NEG_Y);
    let s = (d.length() / (l1 + l2)).clamp(1.0, STRETCH);
    let (l1, l2) = (l1 * s, l2 * s);
    let dist = d.length().clamp((l1 - l2).abs() + 1e-3, l1 + l2);
    let a = (l1 * l1 - l2 * l2 + dist * dist) / (2.0 * dist);
    let h = (l1 * l1 - a * a).max(0.0).sqrt();
    let bend = (pole - dir * pole.dot(dir)).normalize_or(dir.any_orthonormal_vector());
    (root + dir * a + bend * h, root + dir * dist)
}

/// Places a segment mesh hanging from its joint down -Y (front -Z, `nominal` long) from `a` to
/// `b`, its X axis as near `side` as the bone allows, stretched along Y to fit.
pub fn segment(a: Vec3, b: Vec3, nominal: f32, side: Vec3) -> Transform {
    let d = a - b;
    let y = d.normalize_or(Vec3::Y);
    let x = (side - y * side.dot(y)).normalize_or(y.any_orthonormal_vector());
    let z = x.cross(y);
    Transform {
        translation: a,
        rotation: Quat::from_mat3(&Mat3::from_cols(x, y, z)),
        scale: Vec3::new(1.0, (d.length() / nominal).clamp(0.5, 1.5), 1.0),
    }
}

/// The neck from the head's pivot (`lt_sim::sim::neck_of`) down to the chest's top.
pub fn neck(pivot: Vec3, chest_top: Vec3, head_rot: Quat) -> Transform {
    segment(pivot, chest_top, NECK, head_rot * Vec3::X)
}

/// An arm's upper arm and forearm, from the shoulder of the chest at `chest` (`side` -1 left,
/// +1 right) to the glove at `hand`.
pub fn arm(chest: &Transform, side: f32, hand: &Transform) -> [Transform; 2] {
    let right = chest.rotation * Vec3::X;
    let fwd = chest.rotation * Vec3::NEG_Z;
    let shoulder = chest.transform_point(SHOULDER * Vec3::new(side, 1.0, 1.0));
    let wrist = hand.transform_point(WRIST);
    // Elbows out, down and back.
    let pole = chest.rotation * Vec3::new(side * 0.7, -1.0, 0.5);
    let (elbow, wrist) = two_bone(shoulder, wrist, UPPER_ARM, FOREARM, pole);
    // The sleeve's sides face out where the arm allows (degenerate pointing straight ahead).
    let up = (shoulder - elbow).normalize_or(Vec3::Y);
    [
        segment(shoulder, elbow, UPPER_ARM, fwd.cross(up) + right * 0.2),
        segment(elbow, wrist, FOREARM, hand.rotation * Vec3::X),
    ]
}

/// The pose of the lower body, relative to the feet the [`Gait`] was given.
#[derive(Clone, Copy, Debug, Default)]
pub struct Legs {
    pub pelvis: Transform,
    pub thigh: [Transform; 2],
    pub shin: [Transform; 2],
    pub boot: [Transform; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct Foot {
    /// Planted ankle (world) and the boot's yaw.
    at: Vec3,
    yaw: f32,
    /// An idle step in progress (0..1; 1 when standing): from where.
    step: f32,
    from: Vec3,
    from_yaw: f32,
}

/// One avatar's walking legs: a stride cycle driven by how fast its pelvis moves, idle feet
/// that stay planted and shuffle when the hips turn or wander, knees bending as the head drops
/// (crouching, for real or with a key) and tucked legs in the air.
#[derive(Component, Clone, Debug, Default)]
pub struct Gait {
    ready: bool,
    root: Vec3,
    pelvis: Vec3,
    /// Smoothed horizontal velocity (of the pelvis, so walking in a VR room counts) and
    /// vertical velocity (of the feet).
    vel: Vec3,
    vy: f32,
    air: f32,
    air_hold: f32,
    hip_yaw: f32,
    /// Moving backwards (with hysteresis, so strafing doesn't flip the hips back and forth).
    backward: bool,
    phase: f32,
    /// The hips' standing height above the feet (tall VR players have longer legs).
    stand: f32,
    feet: [Foot; 2],
}

impl Gait {
    /// The hips' facing (the legs' and boots' yaw).
    pub fn hip_yaw(&self) -> f32 {
        self.hip_yaw
    }

    /// Advances by `dt` and poses the legs: `root` is the feet (world), `chest_top` the chest's
    /// top relative to them, `chest_yaw` its facing.
    pub fn update(&mut self, dt: f32, root: Vec3, chest_top: Vec3, chest_yaw: f32) -> Legs {
        let dt = dt.clamp(1e-4, 0.1);
        let fresh = !self.ready || root.distance(self.root) > 2.0;
        if fresh {
            *self = Gait {
                ready: true,
                root,
                pelvis: root + chest_top,
                hip_yaw: chest_yaw,
                stand: STAND_HIP,
                ..default()
            };
        }
        let k = 1.0 - (-dt * 8.0).exp();
        let pelvis_w = root + chest_top;
        let v = (pelvis_w - self.pelvis) / dt;
        self.vel = self.vel.lerp(Vec3::new(v.x, 0.0, v.z), k);
        self.vy += ((root.y - self.root.y) / dt - self.vy) * k;
        self.root = root;
        self.pelvis = pelvis_w;
        let speed = self.vel.length();
        let moving = smoothstep(0.3, 1.2, speed);

        // In the air: rising or falling faster than running up a ramp would.
        self.air_hold = if self.vy.abs() > 0.8 + 0.5 * speed {
            0.2
        } else {
            self.air_hold - dt
        };
        let air_want = if self.air_hold > 0.0 { 1.0 } else { 0.0 };
        self.air += (air_want - self.air) * (1.0 - (-dt * 12.0).exp());

        // The hips follow the chest's facing, lagging (faster when moving), never far; moving,
        // they turn halfway into the travel (or away from it, backpedalling), so a strafe runs
        // half forward and the chest keeps aiming.
        let dir = self.vel.normalize_or_zero();
        let travel = wrap((-dir.x).atan2(-dir.z) - chest_yaw);
        if moving > 0.0 {
            let hyst = if self.backward { -0.25 } else { 0.25 };
            self.backward = travel.abs() > FRAC_PI_2 + hyst;
        }
        let along = if self.backward {
            wrap(travel + PI)
        } else {
            travel
        };
        let hip_want = chest_yaw + (0.5 * along * moving).clamp(-MAX_TWIST, MAX_TWIST);
        self.hip_yaw += wrap(hip_want - self.hip_yaw) * (1.0 - (-dt * (2.0 + 6.0 * moving)).exp());
        self.hip_yaw = chest_yaw + wrap(self.hip_yaw - chest_yaw).clamp(-MAX_TWIST, MAX_TWIST);
        let hips = Quat::from_rotation_y(self.hip_yaw);
        // The travel in the hips' frame: forward (negative backwards) and to the right.
        let local = hips.inverse() * dir;
        let (fwd, lat) = (-local.z, local.x);
        let back_k = if self.backward { moving } else { 0.0 };

        // How high the hips would hang from the chest, and how tall this player stands.
        let want = chest_top.y - PELVIS_DROP + HIP.y;
        self.stand = (self.stand - 0.01 * dt)
            .max(want)
            .clamp(STAND_HIP * 0.85, STAND_HIP * 1.25);
        let scale = self.stand / STAND_HIP;
        let (thigh, shin) = (THIGH * scale, SHIN * scale);
        let leg = thigh + shin;
        let crouch = ((self.stand - want) / (0.5 * scale)).clamp(0.0, 1.0);
        // Below a deep squat the pelvis rides up into the vest instead.
        let hip_y = want.max(ANKLE + 0.42 * leg).min(want + 0.36);

        // Stride: cadence and length grow with speed; a run lifts the knees higher. Sideways
        // and backwards the steps are quicker and shorter.
        let cadence = (0.9 + 0.22 * speed).min(3.2) * (1.0 + 0.6 * lat.abs() + 0.25 * back_k);
        if moving > 0.0 {
            self.phase = (self.phase + TAU * cadence * dt).rem_euclid(TAU);
        }
        let amp = (speed / (TAU * cadence)).min(0.42 * scale);
        let run = smoothstep(5.0, 7.0, speed) * (1.0 - back_k);
        let lift = (0.09 + 0.12 * run) * scale * (1.0 - 0.3 * back_k);

        let back = 0.12 * crouch;
        let bob = 0.02 * moving * (2.0 * self.phase).cos();
        // (Leaning back a little, backpedalling.)
        let pelvis = Transform {
            translation: Vec3::new(chest_top.x, hip_y - HIP.y + bob, chest_top.z)
                + hips * Vec3::new(0.0, 0.0, back),
            rotation: hips * Quat::from_rotation_x(-0.3 * crouch + 0.1 * back_k),
            scale: Vec3::ONE,
        };
        let mut legs = Legs {
            pelvis,
            ..default()
        };
        // Side-stepping, the feet stand wider so the trailing one closes up to the leading one
        // rather than crossing through it.
        let width = (STANCE + 0.06 * crouch).max(lat.abs() * amp * moving + 0.06);
        let ground = root.y + ANKLE;
        let rest: [Vec3; 2] = [-1.0, 1.0].map(|side| {
            let p = root + chest_top + hips * Vec3::new(side * width, 0.0, -0.02);
            Vec3::new(p.x, ground, p.z)
        });
        let planted = rest.map(|at| Foot {
            at,
            yaw: self.hip_yaw,
            step: 1.0,
            ..default()
        });
        // Idle: planted feet, one stepping at a time when it has wandered or turned too far
        // (moving, they follow the stride cycle).
        if fresh || moving > 0.5 {
            self.feet = planted;
        } else if self.feet.iter().all(|f| f.step >= 1.0) {
            let off = |f: &Foot, r: Vec3| {
                let d = (f.at - r).xz().length() / 0.14;
                d.max(wrap(f.yaw - self.hip_yaw).abs() / 0.6)
            };
            let (i, worst) = (0..2)
                .map(|i| (i, off(&self.feet[i], rest[i])))
                .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            if worst > 1.0 {
                let f = &mut self.feet[i];
                f.from = f.at;
                f.from_yaw = f.yaw;
                f.step = 0.0;
            }
        }
        let hip_w = |side: f32| root + pelvis.transform_point(HIP * Vec3::new(side, 1.0, 1.0));
        for (i, side) in [-1.0f32, 1.0].into_iter().enumerate() {
            let f = &mut self.feet[i];
            if f.step < 1.0 {
                f.step = (f.step + dt / STEP_TIME).min(1.0);
                let t = smoothstep(0.0, 1.0, f.step);
                f.at = f.from.lerp(rest[i], t) + Vec3::Y * STEP_LIFT * (PI * f.step).sin();
                f.yaw = f.from_yaw + wrap(self.hip_yaw - f.from_yaw) * t;
            } else {
                f.at.y = ground;
            }
            let phi = self.phase + i as f32 * PI;
            let swing = amp * phi.sin();
            let stride = rest[i]
                + hips * Vec3::new(lat * swing, 0.0, -fwd * swing)
                + Vec3::Y * lift * phi.cos().max(0.0) * moving.min(speed / 1.5);
            let mut at = f.at.lerp(stride, moving);
            // In the air: feet drawn up under the hips, one ahead of the other.
            let tuck =
                hip_w(side) + Vec3::Y * -(0.78 * leg) + hips * Vec3::new(0.0, 0.0, 0.1 * side);
            at = at.lerp(tuck, self.air);

            let pole = hips * Vec3::new(side * 0.15, 0.0, -1.0);
            let hip = hip_w(side);
            let (knee, ankle) = two_bone(hip, at, thigh, shin, pole);
            let right = hips * Vec3::X;
            legs.thigh[i] = segment(hip - root, knee - root, THIGH_MESH, right);
            legs.shin[i] = segment(knee - root, ankle - root, SHIN_MESH, right);
            let yaw = f.yaw + wrap(self.hip_yaw - f.yaw) * moving;
            // Heel strike toe up and push-off toe down along the stride (toe first stepping
            // back), rolling onto the outer edge stepping sideways.
            let rock = moving * phi.sin() * (amp / 0.3).min(1.0);
            let pitch = 0.3 * fwd * rock - 0.5 * self.air;
            let roll = -0.15 * lat * rock;
            legs.boot[i] = Transform {
                translation: ankle - root,
                rotation: Quat::from_rotation_y(yaw)
                    * Quat::from_rotation_x(pitch)
                    * Quat::from_rotation_z(roll),
                scale: Vec3::ONE,
            };
        }
        legs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn two_bone_keeps_lengths_and_reaches() {
        let root = Vec3::new(0.1, 1.0, 0.0);
        let target = Vec3::new(0.2, 0.5, -0.3);
        let (mid, end) = two_bone(root, target, 0.4, 0.35, Vec3::NEG_Z);
        assert!((mid.distance(root) - 0.4).abs() < EPS);
        assert!((end.distance(mid) - 0.35).abs() < EPS);
        assert!(end.distance(target) < EPS);
        // Bent towards the pole.
        assert!((mid - (root + target) / 2.0).dot(Vec3::NEG_Z) > 0.0);
    }

    #[test]
    fn two_bone_stretches_then_falls_short() {
        let root = Vec3::ZERO;
        let (_, end) = two_bone(root, Vec3::new(0.0, -0.78, 0.0), 0.4, 0.35, Vec3::NEG_Z);
        assert!((end.y + 0.78).abs() < EPS, "stretches 4%");
        let (_, end) = two_bone(root, Vec3::new(0.0, -2.0, 0.0), 0.4, 0.35, Vec3::NEG_Z);
        assert!((end.y + 0.75 * STRETCH).abs() < EPS, "then lets go");
    }

    #[test]
    fn segment_spans_its_joints() {
        let a = Vec3::new(0.3, 1.2, 0.1);
        let b = Vec3::new(0.4, 0.9, -0.1);
        let t = segment(a, b, 0.3, Vec3::X);
        assert!(t.translation.distance(a) < EPS);
        // The mesh's far end (0, -nominal, 0) lands on b.
        assert!(t.transform_point(Vec3::new(0.0, -0.3, 0.0)).distance(b) < EPS);
        // X stays as near the hint as it can.
        assert!((t.rotation * Vec3::X).dot(Vec3::X) > 0.9);
    }

    fn chest(eye: f32) -> Vec3 {
        Vec3::new(0.0, eye - lt_sim::hitbox::CHEST_DROP, 0.08)
    }

    fn run(g: &mut Gait, from: Vec3, vel: Vec3, eye: f32, secs: f32) -> Legs {
        let mut legs = Legs::default();
        let n = (secs / (1.0 / 90.0)) as usize;
        for k in 0..n {
            let root = from + vel * (k as f32 / 90.0);
            legs = g.update(1.0 / 90.0, root, chest(eye), 0.0);
        }
        legs
    }

    #[test]
    fn standing_legs_reach_the_floor_nearly_straight() {
        let mut g = Gait::default();
        let legs = run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 1.0);
        for i in 0..2 {
            assert!((legs.boot[i].translation.y - ANKLE).abs() < 0.01);
            let knee = legs.shin[i].translation;
            let hip = legs.thigh[i].translation;
            // Knees soft, not bent: within 10 cm of the hip-ankle line.
            let ankle = legs.boot[i].translation;
            let d = (ankle - hip).normalize();
            let off = (knee - hip) - d * (knee - hip).dot(d);
            assert!(off.length() < 0.1, "knee {off:?} off the line");
        }
    }

    #[test]
    fn crouching_bends_the_knees_forward_feet_planted() {
        let mut g = Gait::default();
        run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 1.0);
        let legs = run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE - 0.6, 1.0);
        for i in 0..2 {
            assert!((legs.boot[i].translation.y - ANKLE).abs() < 0.01);
            let hip = legs.thigh[i].translation;
            let knee = legs.shin[i].translation;
            let ankle = legs.boot[i].translation;
            assert!(
                knee.z < hip.z - 0.1 && knee.z < ankle.z - 0.1,
                "knee forward"
            );
            assert!(hip.y > ANKLE + 0.2, "hips above the floor");
        }
    }

    #[test]
    fn idle_feet_stay_planted_and_walking_strides() {
        let mut g = Gait::default();
        let a = run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 0.5);
        let b = run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 0.5);
        for i in 0..2 {
            assert!(a.boot[i].translation.distance(b.boot[i].translation) < 1e-3);
        }
        // Walking forward: the feet swing well ahead of and behind the hips, alternately.
        let vel = Vec3::new(0.0, 0.0, -4.0);
        let mut spread: f32 = 0.0;
        let mut lifted = false;
        let mut root = Vec3::ZERO;
        for _ in 0..180 {
            root += vel / 90.0;
            let legs = g.update(1.0 / 90.0, root, chest(lt_sim::sim::EYE), 0.0);
            let d = legs.boot[0].translation.z - legs.boot[1].translation.z;
            spread = spread.max(d.abs());
            lifted |= legs.boot.iter().any(|b| b.translation.y > ANKLE + 0.04);
        }
        assert!(spread > 0.4, "stride {spread}");
        assert!(lifted);
        assert!(g.phase > 0.0);
    }

    #[test]
    fn turning_in_place_steps_the_feet_round() {
        let mut g = Gait::default();
        run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 0.5);
        for _ in 0..360 {
            g.update(1.0 / 90.0, Vec3::ZERO, chest(lt_sim::sim::EYE), 1.5);
        }
        assert!(wrap(g.hip_yaw - 1.5).abs() < 0.1);
        for f in &g.feet {
            assert!(
                wrap(f.yaw - g.hip_yaw).abs() < 0.61,
                "foot turned with the hips"
            );
        }
    }

    /// Walks at `vel` for 2 s facing -Z; returns the gait and the closest the feet came to
    /// each other across the hips (their gap along the hips' X axis: negative when crossed).
    fn walk(vel: Vec3) -> (Gait, f32) {
        let mut g = Gait::default();
        run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 0.3);
        let mut root = Vec3::ZERO;
        let mut gap = f32::MAX;
        for _ in 0..180 {
            root += vel / 90.0;
            let legs = g.update(1.0 / 90.0, root, chest(lt_sim::sim::EYE), 0.0);
            let x = Quat::from_rotation_y(g.hip_yaw) * Vec3::X;
            gap = gap.min((legs.boot[1].translation - legs.boot[0].translation).dot(x));
        }
        (g, gap)
    }

    #[test]
    fn strafing_turns_the_hips_and_never_crosses_the_feet() {
        for speed in [2.0, 4.0, 8.0] {
            for vel in [Vec3::new(speed, 0.0, 0.0), Vec3::new(-speed, 0.0, 0.0)] {
                let (g, gap) = walk(vel);
                assert!(gap > 0.08, "feet {gap} apart strafing at {vel}");
                // Turned halfway into the travel.
                let want = if vel.x > 0.0 { -PI / 4.0 } else { PI / 4.0 };
                assert!(
                    (g.hip_yaw - want).abs() < 0.1,
                    "hips {} at {vel}",
                    g.hip_yaw
                );
            }
        }
        // Diagonally: the hips turn halfway to the travel.
        let (g, gap) = walk(Vec3::new(2.8, 0.0, -2.8));
        assert!(gap > 0.08);
        assert!((g.hip_yaw + PI / 8.0).abs() < 0.1, "hips {}", g.hip_yaw);
    }

    #[test]
    fn backpedalling_keeps_the_hips_forward() {
        let (g, gap) = walk(Vec3::new(0.0, 0.0, 4.0));
        assert!(g.backward);
        assert!(g.hip_yaw.abs() < 0.1);
        assert!(gap > 0.08);
        // Backwards and to the side: hips turned away from the travel, not round to it.
        let (g, _) = walk(Vec3::new(2.8, 0.0, 2.8));
        assert!(g.backward);
        assert!((g.hip_yaw - PI / 8.0).abs() < 0.1, "hips {}", g.hip_yaw);
    }

    #[test]
    fn jumping_tucks_the_legs() {
        let mut g = Gait::default();
        run(&mut g, Vec3::ZERO, Vec3::ZERO, lt_sim::sim::EYE, 0.5);
        let legs = run(
            &mut g,
            Vec3::ZERO,
            Vec3::new(0.0, 3.5, 0.0),
            lt_sim::sim::EYE,
            0.2,
        );
        let root_y = 3.5 * 0.2;
        for b in &legs.boot {
            assert!(
                b.translation.y > ANKLE + 0.1,
                "feet {} above the root {root_y}",
                b.translation.y
            );
        }
    }
}

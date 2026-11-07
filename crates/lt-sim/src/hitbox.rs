//! Where a player can be hit: the floating parts of the avatar (src/actors.rs draws them from
//! the same poses). The head is one sphere for every animal (ears and antlers don't count), the
//! chest a box turned with the body, the gloves small spheres, the blaster a box along its
//! muzzle.

use glam::{Quat, Vec3};

use crate::collide::ray_box;
use crate::types::PlayerCmd;

/// How far below the eyes the floating chest's top is, looking straight ahead (it hangs from
/// the neck: `sim::chest_top`).
pub const CHEST_DROP: f32 = 0.3;
/// How much bigger than its model a head is drawn (src/actors.rs), grown about the skull top
/// (this high above the eyes), so it reaches down towards the chest rather than up.
pub const HEAD_SCALE: f32 = 1.5;
pub const HEAD_CROWN: f32 = 0.15;
/// The head sphere of the model at scale 1 (its centre from the eyes, in the head's frame), and
/// grown with the drawn head.
const MODEL_HEAD_CENTRE: Vec3 = Vec3::new(0.0, 0.02, 0.06);
const MODEL_HEAD_RADIUS: f32 = 0.15;
pub const HEAD_RADIUS: f32 = MODEL_HEAD_RADIUS * HEAD_SCALE;
pub const HEAD_CENTRE: Vec3 = Vec3::new(
    MODEL_HEAD_CENTRE.x * HEAD_SCALE,
    HEAD_CROWN + (MODEL_HEAD_CENTRE.y - HEAD_CROWN) * HEAD_SCALE,
    MODEL_HEAD_CENTRE.z * HEAD_SCALE,
);
/// The chest box, half extents (x across, y, z front to back).
pub const CHEST_HALF: Vec3 = Vec3::new(0.22, 0.25, 0.125);
pub const HAND_RADIUS: f32 = 0.075;
/// The blaster box in the muzzle's frame (the muzzle at the origin, the gun along +Z behind it).
pub const GUN_MIN: Vec3 = Vec3::new(-0.035, -0.11, -0.01);
pub const GUN_MAX: Vec3 = Vec3::new(0.035, 0.035, 0.29);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Head,
    Chest,
    Hand,
    Gun,
}

fn unit(q: Quat) -> Quat {
    if q.is_finite() && q.length_squared() > 0.5 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// Distance along the ray `o` + t·`d` (`d` unit length) to a sphere, within `len`.
fn ray_sphere(o: Vec3, d: Vec3, c: Vec3, r: f32, len: f32) -> Option<f32> {
    let m = o - c;
    let b = m.dot(d);
    let cc = m.length_squared() - r * r;
    if cc > 0.0 && b > 0.0 {
        return None;
    }
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()).max(0.0);
    (t <= len).then_some(t)
}

/// Distance to a box turned by `rot` about `centre` (its extents `min`..`max` in its frame).
fn ray_obb(
    o: Vec3,
    d: Vec3,
    centre: Vec3,
    rot: Quat,
    min: Vec3,
    max: Vec3,
    len: f32,
) -> Option<f32> {
    let inv = rot.inverse();
    let lo = inv * (o - centre);
    let ld = inv * d;
    if lo.cmpge(min).all() && lo.cmple(max).all() {
        return Some(0.0);
    }
    ray_box(lo, ld, min, max, len).map(|(t, _)| t)
}

/// The first part of a player (feet at `feet`, eyes at `eye`, its latest `cmd`) the segment
/// `a`..`b` hits: the fraction along the segment and the part.
pub fn first_hit(feet: Vec3, eye: Vec3, cmd: &PlayerCmd, a: Vec3, b: Vec3) -> Option<(f32, Part)> {
    let len = (b - a).length();
    if len < 1e-6 {
        return None;
    }
    let d = (b - a) / len;
    // Quick reject: nothing of the player is more than ~1.3 m from the chest.
    let centre = eye - Vec3::Y * 0.5;
    let m = a - centre;
    let t = (-m.dot(d)).clamp(0.0, len);
    if (m + d * t).length_squared() > 1.3 * 1.3 {
        return None;
    }
    let mut best: Option<(f32, Part)> = None;
    let mut take = |t: Option<f32>, part: Part| {
        if let Some(t) = t
            && best.is_none_or(|(b, _)| t < b)
        {
            best = Some((t, part));
        }
    };
    let head_rot = unit(cmd.head.rot);
    take(
        ray_sphere(a, d, eye + head_rot * HEAD_CENTRE, HEAD_RADIUS, len),
        Part::Head,
    );
    let body = Quat::from_rotation_y(if cmd.yaw.is_finite() { cmd.yaw } else { 0.0 });
    let chest = crate::sim::chest_top(eye, head_rot) - Vec3::Y * CHEST_HALF.y;
    take(
        ray_obb(a, d, chest, body, -CHEST_HALF, CHEST_HALF, len),
        Part::Chest,
    );
    for h in &cmd.hands {
        let p = feet + h.pos;
        if h.pos != Vec3::ZERO && p.is_finite() && (p - eye).length() < 1.3 {
            take(ray_sphere(a, d, p, HAND_RADIUS, len), Part::Hand);
        }
    }
    // (Dual pistols: the second too.)
    for (hand, muzzle) in [(cmd.gun_hand, cmd.muzzle), (cmd.gun2_hand, cmd.muzzle2)] {
        if hand.is_none() {
            continue;
        }
        let p = feet + muzzle.pos;
        if muzzle.pos != Vec3::ZERO && p.is_finite() && (p - eye).length() < 1.5 {
            take(
                ray_obb(a, d, p, unit(muzzle.rot), GUN_MIN, GUN_MAX, len),
                Part::Gun,
            );
        }
    }
    best.map(|(t, p)| (t / len, p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Hand, Pose};

    fn cmd() -> PlayerCmd {
        PlayerCmd {
            head: Pose {
                pos: Vec3::Y * 1.65,
                rot: Quat::IDENTITY,
            },
            hands: [
                Pose {
                    pos: Vec3::new(-0.3, 1.0, -0.2),
                    rot: Quat::IDENTITY,
                },
                Pose {
                    pos: Vec3::new(0.3, 1.3, -0.4),
                    rot: Quat::IDENTITY,
                },
            ],
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: Vec3::new(0.3, 1.35, -0.6),
                rot: Quat::IDENTITY,
            },
            ..Default::default()
        }
    }

    fn shot(y: f32, x: f32) -> Option<Part> {
        let eye = Vec3::Y * 1.65;
        first_hit(
            Vec3::ZERO,
            eye,
            &cmd(),
            Vec3::new(x, y, -5.0),
            Vec3::new(x, y, 5.0),
        )
        .map(|h| h.1)
    }

    #[test]
    fn parts() {
        assert_eq!(shot(1.65, 0.0), Some(Part::Head));
        assert_eq!(shot(1.1, 0.0), Some(Part::Chest));
        assert_eq!(shot(1.0, -0.3), Some(Part::Hand));
        assert_eq!(
            shot(1.3, 0.3),
            Some(Part::Gun),
            "the blaster is in front of the glove"
        );
        // The grown head reaches down towards the chest.
        assert_eq!(shot(1.45, 0.0), Some(Part::Head));
        // Between the chest and the head, beside the head, below the chest: misses.
        assert_eq!(shot(1.365, 0.0), None);
        assert_eq!(shot(1.65, 0.25), None);
        assert_eq!(shot(0.5, 0.0), None);
        // Antler height above the head sphere misses.
        assert_eq!(shot(1.95, 0.0), None);
    }

    #[test]
    fn head_is_the_skull_only() {
        // The sphere ends just above the skull top, which the grown head keeps at the crown.
        let top = HEAD_CENTRE.y + HEAD_RADIUS;
        assert!(top > HEAD_CROWN && top < HEAD_CROWN + 0.04, "{top}");
        // Rabbit ears and deer antlers (from 0.25 m above the eyes up) never count.
        for y in [0.25, 0.32, 0.4, 0.5] {
            for z in [-0.1, 0.0, 0.1] {
                let hit = first_hit(
                    Vec3::ZERO,
                    Vec3::Y * 1.65,
                    &cmd(),
                    Vec3::new(-5.0, 1.65 + y, z),
                    Vec3::new(5.0, 1.65 + y, z),
                );
                assert_eq!(hit, None, "ear height {y}, depth {z}");
            }
        }
    }
}

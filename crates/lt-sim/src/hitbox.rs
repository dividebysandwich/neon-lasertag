//! Where a player can be hit: the floating parts of the avatar (src/actors.rs draws them from
//! the same poses). The head is one sphere for every animal (ears and antlers don't count), the
//! chest a box turned with the body, the gloves small spheres, the blaster a box along its
//! muzzle.

use glam::{Quat, Vec3};

use crate::collide::ray_box;
use crate::types::PlayerCmd;

/// How far below the eyes the floating chest's top is (src/actors.rs CHEST_DROP).
pub const CHEST_DROP: f32 = 0.3;
pub const HEAD_RADIUS: f32 = 0.15;
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
        ray_sphere(
            a,
            d,
            eye + head_rot * Vec3::new(0.0, 0.02, 0.06),
            HEAD_RADIUS,
            len,
        ),
        Part::Head,
    );
    let body = Quat::from_rotation_y(if cmd.yaw.is_finite() { cmd.yaw } else { 0.0 });
    let chest = eye - Vec3::Y * (CHEST_DROP + CHEST_HALF.y);
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
    if cmd.gun_hand.is_some() {
        let p = feet + cmd.muzzle.pos;
        if cmd.muzzle.pos != Vec3::ZERO && p.is_finite() && (p - eye).length() < 1.5 {
            take(
                ray_obb(a, d, p, unit(cmd.muzzle.rot), GUN_MIN, GUN_MAX, len),
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
        // Between the chest and the head, beside the head, below the chest: misses.
        assert_eq!(shot(1.45, 0.0), None);
        assert_eq!(shot(1.65, 0.25), None);
        assert_eq!(shot(0.5, 0.0), None);
        // Antler height above the head sphere misses.
        assert_eq!(shot(1.95, 0.0), None);
    }
}

//! The avatars' eyes: every head gets the shared eyeball and two eyelids (assets/models/
//! avatars.glb `eyeball`, `eyelid`) in each of its eye frames, and this module turns the balls
//! to look about and the lids to blink. The eyes glance at whoever is looking at them (the
//! camera) when they are near and ahead, else at the nearest other head close by, else they
//! wander in saccades; lids rest at the animal's openness, follow the gaze up and down a
//! little, and snap shut now and then.

use bevy::prelude::*;

use crate::assets::{HeadModel, Models};
use crate::materials::Palette;

/// Furthest the eyes follow the viewer (m), and another head.
const WATCH_VIEWER: f32 = 14.0;
const WATCH_OTHER: f32 = 6.0;
/// How far the eyes turn at most (radians): sideways, and up/down.
const MAX_YAW: f32 = 0.42;
const MAX_PITCH: f32 = 0.3;
/// How quickly the eyes reach their target (per second), and the lids theirs.
const GAZE_RATE: f32 = 22.0;
const LID_RATE: f32 = 30.0;
/// Seconds between saccades, and between blinks; how long a blink takes.
const SACCADE: (f32, f32) = (0.6, 2.8);
const BLINK_EVERY: (f32, f32) = (2.0, 6.5);
const BLINK_SECS: f32 = 0.17;
/// Lids shut: their edges meet this far below the eye's forward point (radians;
/// tools/models/avatars.py LID_SHUT).
const LID_SHUT: f32 = -5.0 * std::f32::consts::PI / 180.0;

pub struct EyesPlugin;

impl Plugin for EyesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, animate);
    }
}

/// One eye's parts under a head: the ball, the upper and the lower lid, and its frame in the
/// head (`HeadModel::eyes`).
struct Eye {
    ball: Entity,
    lids: [Entity; 2],
    frame: Transform,
}

/// A head's pair of eyes and their state (on the head's model entity).
#[derive(Component)]
pub struct EyeRig {
    eyes: Vec<Eye>,
    /// How wide open the lids rest (`HeadModel::lids`).
    rest: [f32; 2],
    /// Where the eyes look (yaw, pitch; radians) and where they are heading.
    gaze: Vec2,
    target: Vec2,
    /// Seconds to the next saccade (a new wandering target) and to the next blink.
    saccade: f32,
    blink_in: f32,
    /// A blink under way: seconds into it (none when negative).
    blink: f32,
    /// The lids' angles.
    lids: [f32; 2],
    rng: u64,
}

impl EyeRig {
    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, (lo, hi): (f32, f32)) -> f32 {
        lo + (hi - lo) * self.rand()
    }
}

/// Gives the head model at `head` (an entity in the head's frame) its eyes, animated from now
/// on; nothing for a model without eye frames.
pub fn spawn_eyes(
    commands: &mut Commands,
    head: Entity,
    animal: &HeadModel,
    models: &Models,
    palette: &Palette,
    team: Option<lt_sim::Team>,
    seed: u64,
) {
    let Some(frames) = animal.eyes else {
        return;
    };
    let mut rig = EyeRig {
        eyes: Vec::new(),
        rest: animal.lids,
        gaze: Vec2::ZERO,
        target: Vec2::ZERO,
        saccade: 0.0,
        blink_in: 0.0,
        blink: -1.0,
        lids: animal.lids,
        rng: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
    };
    rig.saccade = rig.range(SACCADE);
    rig.blink_in = rig.range(BLINK_EVERY) * 0.5;
    for frame in frames {
        let part = |commands: &mut Commands, model| {
            let e = commands
                .spawn((frame, Visibility::default(), ChildOf(head)))
                .id();
            crate::actors::spawn_model(commands, e, model, palette, team);
            e
        };
        let ball = part(commands, animal.eyeball.as_ref().unwrap_or(&models.eyeball));
        let lids = [
            part(commands, &models.eyelid),
            part(commands, &models.eyelid),
        ];
        rig.eyes.push(Eye { ball, lids, frame });
    }
    commands.entity(head).insert(rig);
}

/// Where a lid's edge stands over the eye during a blink `t` seconds in: shut fast, open slower.
fn blink_curve(t: f32) -> f32 {
    let p = (t / BLINK_SECS).clamp(0.0, 1.0);
    let s = if p < 0.35 {
        p / 0.35
    } else {
        1.0 - (p - 0.35) / 0.65
    };
    s * s * (3.0 - 2.0 * s)
}

fn animate(
    time: Res<Time>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut rigs: Query<(Entity, &GlobalTransform, &mut EyeRig)>,
    mut transforms: Query<&mut Transform, Without<EyeRig>>,
) {
    let dt = time.delta_secs().min(0.1);
    let viewer = cams.iter().next().map(|c| c.translation());
    let heads: Vec<(Entity, Vec3)> = rigs.iter().map(|(e, g, _)| (e, g.translation())).collect();
    for (me, global, mut rig) in &mut rigs {
        // Something to look at: the viewer, or the nearest other head, close by and in front.
        let here = global.translation();
        let ahead = global.rotation() * Vec3::NEG_Z;
        let in_view = |p: Vec3, reach: f32| {
            let d = p - here;
            let len = d.length();
            len > 0.3 && len < reach && d.dot(ahead) > len * 0.45
        };
        let mut look = viewer.filter(|&v| in_view(v, WATCH_VIEWER));
        if look.is_none() {
            look = heads
                .iter()
                .filter(|(e, p)| *e != me && in_view(*p, WATCH_OTHER))
                .min_by(|a, b| {
                    a.1.distance_squared(here)
                        .total_cmp(&b.1.distance_squared(here))
                })
                .map(|(_, p)| *p);
        }
        rig.saccade -= dt;
        if let Some(p) = look {
            // Both eyes converge a little on it; the frame in between is the head's.
            let local = global.affine().inverse().transform_point3(p);
            let d = local.normalize_or_zero();
            if d != Vec3::ZERO {
                let yaw = (-d.x).atan2(-d.z);
                let pitch = d.y.asin();
                rig.target = Vec2::new(yaw, pitch);
            }
            if rig.saccade <= 0.0 {
                // Glance aside for a moment now and then.
                rig.saccade = rig.range(SACCADE);
                if rig.rand() < 0.3 {
                    let (dy, dp) = (rig.range((-0.15, 0.15)), rig.range((-0.1, 0.1)));
                    rig.target += Vec2::new(dy, dp);
                }
            }
        } else if rig.saccade <= 0.0 {
            rig.saccade = rig.range(SACCADE);
            rig.target = if rig.rand() < 0.4 {
                Vec2::new(rig.range((-0.08, 0.08)), rig.range((-0.05, 0.05)))
            } else {
                Vec2::new(rig.range((-0.35, 0.35)), rig.range((-0.2, 0.18)))
            };
        }
        let target = Vec2::new(
            rig.target.x.clamp(-MAX_YAW, MAX_YAW),
            rig.target.y.clamp(-MAX_PITCH, MAX_PITCH),
        );
        let k = 1.0 - (-GAZE_RATE * dt).exp();
        let gaze = rig.gaze;
        rig.gaze = gaze + (target - gaze) * k;

        // Blinks.
        if rig.blink >= 0.0 {
            rig.blink += dt;
            if rig.blink > BLINK_SECS {
                rig.blink = -1.0;
                rig.blink_in = if rig.rand() < 0.15 {
                    0.25
                } else {
                    rig.range(BLINK_EVERY)
                };
            }
        } else {
            rig.blink_in -= dt;
            if rig.blink_in <= 0.0 {
                rig.blink = 0.0;
            }
        }
        let shut = if rig.blink >= 0.0 {
            blink_curve(rig.blink)
        } else {
            0.0
        };
        // The lids follow the gaze: the upper lifts looking up, the lower rises looking down.
        let pitch = rig.gaze.y;
        let open = [rig.rest[0] + pitch * 0.6, rig.rest[1] - pitch * 0.5];
        let kl = 1.0 - (-LID_RATE * dt).exp();
        for i in 0..2 {
            let want = open[i] + (LID_SHUT - open[i]) * shut;
            rig.lids[i] += (want - rig.lids[i]) * kl;
        }

        let (gaze, lids) = (rig.gaze, rig.lids);
        for eye in &rig.eyes {
            if let Ok(mut t) = transforms.get_mut(eye.ball) {
                *t = eye.frame
                    * Transform::from_rotation(Quat::from_euler(
                        EulerRot::YXZ,
                        gaze.x,
                        gaze.y,
                        0.0,
                    ));
            }
            if let Ok(mut t) = transforms.get_mut(eye.lids[0]) {
                *t = eye.frame * Transform::from_rotation(Quat::from_rotation_x(lids[0]));
            }
            if let Ok(mut t) = transforms.get_mut(eye.lids[1]) {
                *t = eye.frame
                    * Transform::from_rotation(
                        Quat::from_rotation_z(std::f32::consts::PI)
                            * Quat::from_rotation_x(lids[1]),
                    );
            }
        }
    }
}

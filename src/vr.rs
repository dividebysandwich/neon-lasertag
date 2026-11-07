//! Playing in VR, whichever renderer drives the headset (bevy_mod_openxr in src/xr.rs, or the
//! GL renderer in src/glvr): raw controller and head input in `XrInput` becomes the local
//! player's command, and the tracking space is placed in the arena.
//!
//! - Left stick walks (relative to where the head looks), right stick turns smoothly. Room
//!   scale walking moves the player too; walls stop it (the play space slides back).
//! - The blaster sits in a holster at the right hip (left: `--left-handed`); grip it there to
//!   draw it (one hand at a time: the first to grab keeps it), let go and it goes back. The
//!   trigger fires.
//! - Hold B (or Y) to take a grenade into that hand; let go to throw it with the hand's speed.
//! - The other hand's trigger (the left one, unless it holds the blaster) switches on a carried
//!   shield.
//! - The right controller's menu button (Steam Frame), or holding B / Y for a second without a
//!   grenade, opens the VR menu (src/vrui.rs).
//! - The left controller's menu button (Steam Frame: view; Touch: menu) recenters: an open panel
//!   comes in front of the eyes again, and out of a game the play space is turned and moved so
//!   the head is at the centre of the arena, looking along -Z.
//!
//! Haptics are patterns (`Rumble`) played as an amplitude a frame per hand (`XrInput::haptic`),
//! which each backend sends as short vibrations.

use std::collections::VecDeque;

use bevy::prelude::*;
use lt_sim::sim::{look_rot, yaw_of};
use lt_sim::{Hand, PickupKind, Pose, SimEvent, Team};

use crate::assets::Models;
use crate::game::{Game, GameEvent, InputSet, LocalCmd, Mover, PredictSet};
use crate::materials::{AccentMaterial, NeonUniform, Palette, team_color};
use crate::segtext::{self, Align};
use crate::sevenseg::{Digit, show, spawn_digits};

/// The holster, from the eyes (along the body's facing; x to the gun side). `LT_HOLSTER=x,y,z`
/// puts it elsewhere.
const HOLSTER: Vec3 = Vec3::new(0.23, -0.58, -0.07);

fn holster_offset() -> Vec3 {
    static AT: std::sync::OnceLock<Vec3> = std::sync::OnceLock::new();
    *AT.get_or_init(|| {
        let v: Vec<f32> = std::env::var("LT_HOLSTER")
            .unwrap_or_default()
            .split(',')
            .filter_map(|t| t.parse().ok())
            .collect();
        if v.len() == 3 {
            Vec3::new(v[0], v[1], v[2])
        } else {
            HOLSTER
        }
    })
}
/// How far the blaster is turned nose down from the controller's aim, so its handle sits more
/// upright in the hand (degrees in `LT_GUN_TILT`, default 15).
static GUN_TILT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();

/// A hand this close to the holster draws the blaster when it grips.
const HOLSTER_REACH: f32 = 0.2;
/// Holding B / Y this long (without taking a grenade) opens the menu.
const MENU_HOLD: f32 = 0.9;
/// How long the backends vibrate for each frame's amplitude: a little over a frame.
pub const HAPTIC_NANOS: i64 = 22_000_000;
/// The Steam Frame's controllers (SteamVR), whose own interaction profile has the right
/// controller's menu button (Touch emulation only has the left one, its view button).
pub const FRAME_EXT: &[u8] = b"XR_VALVE_frame_controller_interaction\0";
pub const FRAME_PROFILE: &str = "/interaction_profiles/valve/frame_controller";

/// One controller as the runtime reports it (poses in the tracking space).
#[derive(Clone, Copy, Debug, Default)]
pub struct HandInput {
    pub grip: Option<Transform>,
    pub aim: Option<Transform>,
    pub squeeze: f32,
    pub trigger: f32,
    /// A / X
    pub primary: bool,
    /// B / Y
    pub secondary: bool,
    /// The menu button.
    pub menu: bool,
    pub stick: Vec2,
}

/// Filled in by the XR backend before `InputSet`; `haptic` is read back after `Update`.
#[derive(Resource, Default)]
pub struct XrInput {
    pub hands: [HandInput; 2],
    /// The head (between the eyes) in the tracking space.
    pub head: Option<Transform>,
    /// Vibration to play per hand this frame (0..1; the backend vibrates for a little longer
    /// than a frame, so a pattern's amplitude is set every frame), cleared by the backend.
    pub haptic: [f32; 2],
}

/// The entity whose transform places the tracking space in the arena.
#[derive(Resource, Clone, Copy)]
pub struct XrRig(pub Entity);

/// The tracking space's placement: yaw (from turning) and position, kept so the head is above
/// the predicted feet.
#[derive(Resource, Default)]
pub struct VrState {
    pub yaw: f32,
    pub rig: Transform,
    last_head: Option<Vec3>,
    pub gun: Option<Hand>,
    grenade: Option<Hand>,
    /// The hand that threw the last grenade.
    threw: Option<Hand>,
    firing: bool,
    /// World positions of each hand over the last moments (time, position).
    trail: [VecDeque<(f32, Vec3)>; 2],
    /// The body's facing in the tracking space (the head's, following slowly).
    body_yaw: Option<f32>,
    /// The holster in the tracking space, and in the world.
    holster: Option<Vec3>,
    pub holster_world: Option<Transform>,
    /// Per hand: in reach of the holster, gripping (last frame), the menu button held, how
    /// long B / Y is held (and whether that opened the menu).
    near: [bool; 2],
    gripping: [bool; 2],
    menu_held: [bool; 2],
    hold: [(f32, bool); 2],
    /// Times the menu was asked for (src/vrui.rs takes them).
    pub menu_requests: u32,
    /// Recentering was asked for this frame (the left menu button).
    pub recenter: bool,
    /// Out of a game: the tracking space in the arena (recentering moves it).
    lobby_rig: Transform,
    /// A VR panel takes the triggers (the blaster doesn't fire).
    pub ui: bool,
}

/// Vibration patterns being played: (hand, wait, length, amplitude, fading, played).
#[derive(Resource, Default)]
pub struct Rumble {
    pulses: Vec<(usize, f32, f32, f32, bool, f32)>,
}

impl Rumble {
    /// A pulse on `hand` (0 left, 1 right) after `wait` seconds.
    pub fn play(&mut self, hand: usize, wait: f32, secs: f32, amplitude: f32) {
        self.pulses.push((hand, wait, secs, amplitude, false, 0.0));
    }

    /// A pulse fading out.
    pub fn fade(&mut self, hand: usize, wait: f32, secs: f32, amplitude: f32) {
        self.pulses.push((hand, wait, secs, amplitude, true, 0.0));
    }

    pub fn both(&mut self, wait: f32, secs: f32, amplitude: f32) {
        self.play(0, wait, secs, amplitude);
        self.play(1, wait, secs, amplitude);
    }
}

/// Which side a hand is on.
fn hand_of(i: usize) -> Hand {
    if i == 0 { Hand::Left } else { Hand::Right }
}

/// Moves angle `a` towards `b` by `k` (0..1) of the way.
fn approach_angle(a: f32, b: f32, k: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * k
}

/// Where the blaster sits holstered: on the gun side's hip.
#[derive(Resource)]
pub struct Handedness {
    pub left: bool,
}

/// Hand poses and the rig are set after the prediction, before anything is drawn.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoseSet;

pub struct VrPlugin;

impl Plugin for VrPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<XrInput>()
            .init_resource::<VrState>()
            .init_resource::<Rumble>()
            .configure_sets(
                Update,
                PoseSet.after(PredictSet).before(crate::game::DrawSet),
            )
            .add_systems(Startup, |mut commands: Commands, args: Res<crate::Args>| {
                commands.insert_resource(Handedness {
                    left: args.left_handed,
                });
            })
            .add_systems(
                Update,
                (fake_hands, read_controls)
                    .chain()
                    .in_set(InputSet)
                    .run_if(resource_exists::<Game>),
            )
            .add_systems(Update, menu_buttons.in_set(InputSet))
            .add_systems(
                Update,
                (place_rig, wrist, wrist_points, holster)
                    .chain()
                    .in_set(PoseSet)
                    .run_if(resource_exists::<Game>),
            )
            .add_systems(
                Update,
                out_of_game
                    .in_set(PoseSet)
                    .run_if(not(resource_exists::<Game>)),
            )
            .add_systems(
                Update,
                (haptics.run_if(resource_exists::<Game>), play_rumble)
                    .chain()
                    .after(PoseSet),
            )
            .add_systems(
                Update,
                spawn_wrist.run_if(
                    resource_exists::<Palette>
                        .and_then(resource_exists::<Models>)
                        .and_then(not(resource_exists::<Wrist>)),
                ),
            )
            .add_systems(
                Update,
                spawn_holster.run_if(
                    resource_exists::<Palette>
                        .and_then(resource_exists::<Models>)
                        .and_then(not(resource_exists::<Holster>)),
                ),
            )
            .add_plugins(crate::vrui::VrUiPlugin);
    }
}

/// The menu buttons, in and out of a game: the right one asks for the menu, the left one
/// recenters.
fn menu_buttons(input: Res<XrInput>, mut vr: ResMut<VrState>) {
    vr.recenter = false;
    for i in 0..2 {
        let down = input.hands[i].menu;
        if down && !vr.menu_held[i] {
            if i == 0 {
                vr.recenter = true;
            } else {
                vr.menu_requests += 1;
            }
        }
        vr.menu_held[i] = down;
    }
}

/// Out of a game (the VR menu's scene, src/lobby.rs): the tracking space where recentering put
/// it (at first, its origin at the centre of the arena, facing along -Z), and the holster and
/// the wrist display hidden.
fn out_of_game(
    mut vr: ResMut<VrState>,
    rig: Option<Res<XrRig>>,
    holster: Option<Res<Holster>>,
    wrist: Option<Res<Wrist>>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    input: Res<XrInput>,
) {
    if vr.recenter
        && let Some(h) = input.head
    {
        // The head to the centre, looking along -Z.
        let rot = Quat::from_rotation_y(-flat_yaw(h.rotation));
        vr.lobby_rig = Transform {
            translation: -(rot * Vec3::new(h.translation.x, 0.0, h.translation.z)),
            rotation: rot,
            ..default()
        };
    }
    vr.yaw = 0.0;
    vr.rig = vr.lobby_rig;
    vr.gun = None;
    vr.holster_world = None;
    if let Some(r) = rig
        && let Ok(mut t) = transforms.get_mut(r.0)
        && *t != vr.rig
    {
        *t = vr.rig;
    }
    let mut hidden = Vec::new();
    if let Some(h) = holster {
        hidden.extend([h.root, h.ring]);
    }
    if let Some(w) = wrist {
        hidden.extend([w.root, w.panel]);
    }
    for e in hidden {
        if let Ok(mut v) = vis.get_mut(e)
            && *v != Visibility::Hidden
        {
            *v = Visibility::Hidden;
        }
    }
}

/// Testing without controllers (`LT_VR_FAKE_HANDS=1`): the right hand points ahead and a little
/// down (at a panel's second button), the left one is at the holster (`LT_VR_FAKE_HANDS=wrist`:
/// held up before the eyes, its wrist display turned to them).
fn fake_hands(
    mut input: ResMut<XrInput>,
    vr: Res<VrState>,
    mut mode: Local<Option<Option<String>>>,
) {
    let mode = mode.get_or_insert_with(|| {
        std::env::var("LT_VR_FAKE_HANDS")
            .ok()
            .filter(|v| v == "1" || v == "wrist")
    });
    let Some(head) = input.head.filter(|_| mode.is_some()) else {
        return;
    };
    if mode.as_deref() == Some("wrist") && input.hands[0].grip.is_none() {
        let rot = Quat::from_rotation_y(flat_yaw(head.rotation));
        // (The wrist display is turned by -1.2 rad about the hand's x: undone.)
        let t = Transform::from_translation(head.translation + rot * Vec3::new(-0.02, -0.12, -0.3))
            .with_rotation(rot * Quat::from_rotation_x(1.2 - 0.5));
        input.hands[0].grip = Some(t);
        input.hands[0].aim = Some(t);
    }
    let rot = Quat::from_rotation_y(flat_yaw(head.rotation));
    if input.hands[1].grip.is_none() {
        let at = head.translation + rot * Vec3::new(0.2, -0.25, -0.35);
        let target = head.translation + rot * Vec3::new(0.0, -0.17, -1.4);
        let t = Transform::from_translation(at).looking_at(target, Vec3::Y);
        input.hands[1].grip = Some(t);
        input.hands[1].aim = Some(t);
    }
    if input.hands[0].grip.is_none()
        && let Some(h) = vr.holster
    {
        input.hands[0].grip =
            Some(Transform::from_translation(h + Vec3::Y * 0.05).with_rotation(rot));
    }
}

fn flat_yaw(rot: Quat) -> f32 {
    let f = rot * Vec3::NEG_Z;
    yaw_of(Vec3::new(f.x, 0.0, f.z))
}

#[allow(clippy::too_many_arguments)]
fn read_controls(
    input: Res<XrInput>,
    mut vr: ResMut<VrState>,
    mut local: ResMut<LocalCmd>,
    game: Res<Game>,
    time: Res<Time>,
    mut events: MessageReader<GameEvent>,
    mut faced: Local<Option<u64>>,
    mut rumble: ResMut<Rumble>,
    handedness: Option<Res<Handedness>>,
) {
    let dt = time.delta_secs();
    // Face the centre of the arena on entering it and after respawning.
    let respawned = events
        .read()
        .any(|e| matches!(e.0, SimEvent::Respawned { player, .. } if Some(player) == game.me));
    if (respawned || *faced != Some(game.level.seed))
        && let Some(feet) = game
            .me
            .and_then(|me| game.snap.players.iter().find(|p| p.id == me))
            .map(|p| p.feet)
    {
        let want = crate::desktop::spawn_yaw(&game, feet);
        vr.yaw = want - input.head.map_or(0.0, |h| flat_yaw(h.rotation));
        *faced = Some(game.level.seed);
    }
    let rig_rot = Quat::from_rotation_y(vr.yaw);
    // Walking in the room.
    if let Some(h) = input.head {
        if let Some(last) = vr.last_head {
            let d = rig_rot * (h.translation - last);
            let d = Vec2::new(d.x, d.z);
            // A jump (tracking lost and found again) isn't a step.
            if d.length() < 0.5 {
                local.walk_frame += d;
            }
        }
        vr.last_head = Some(h.translation);
    }
    // Left stick: walk where the head looks.
    let head_yaw = vr.yaw + input.head.map_or(0.0, |h| flat_yaw(h.rotation));
    let s = input.hands[0].stick;
    let s = if s.length() < 0.15 { Vec2::ZERO } else { s };
    let fwd = Vec2::new(-head_yaw.sin(), -head_yaw.cos());
    let right = Vec2::new(head_yaw.cos(), -head_yaw.sin());
    local.cmd.stick = (fwd * s.y + right * s.x).clamp_length_max(1.0);
    local.cmd.walk = Vec2::ZERO;
    // Right stick: turn (about the head: the rig is placed under it again in `place_rig`).
    let t = input.hands[1].stick.x;
    if t.abs() > 0.2 {
        vr.yaw -= t.signum() * (t.abs() - 0.2) / 0.8 * 2.4 * dt;
    }
    local.cmd.yaw = vr.yaw + input.head.map_or(0.0, |h| flat_yaw(h.rotation));

    // The holster: at the hip on the gun side, turned with the body (which follows the head's
    // facing slowly).
    let left_handed = handedness.is_some_and(|h| h.left);
    if let Some(h) = input.head {
        let head_yaw = flat_yaw(h.rotation);
        let body = vr.body_yaw.map_or(head_yaw, |b| {
            approach_angle(b, head_yaw, 1.0 - (-dt * 3.0).exp())
        });
        vr.body_yaw = Some(body);
        let side = if left_handed { -1.0 } else { 1.0 };
        let at = holster_offset();
        let offset = Quat::from_rotation_y(body) * Vec3::new(at.x * side, 0.0, at.z);
        vr.holster = Some(h.translation + offset + Vec3::Y * at.y);
    }
    // The blaster: drawn from the holster by the first hand to grip it there, held until that
    // hand lets go, and back in the holster then.
    let alive = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me))
        .is_some_and(|p| p.respawn.is_none());
    if let Some(h) = vr.gun
        && input.hands[h.index()].squeeze < 0.4
    {
        vr.gun = None;
        rumble.play(h.index(), 0.0, 0.035, 0.85);
    }
    let order = if left_handed {
        [Hand::Left, Hand::Right]
    } else {
        [Hand::Right, Hand::Left]
    };
    for h in order {
        let i = h.index();
        let hand = &input.hands[i];
        let near = match (hand.grip, vr.holster) {
            (Some(g), Some(at)) => g.translation.distance(at) < HOLSTER_REACH,
            _ => false,
        };
        // A light tick on reaching the holster with an empty hand.
        if near && !vr.near[i] && vr.gun.is_none() && vr.grenade != Some(h) {
            rumble.play(i, 0.0, 0.015, 0.25);
        }
        let gripping = hand.squeeze > 0.6;
        if vr.gun.is_none() && near && gripping && !vr.gripping[i] && vr.grenade != Some(h) {
            vr.gun = Some(h);
            rumble.play(i, 0.0, 0.035, 0.85);
        }
        vr.near[i] = near;
        if hand.squeeze < 0.4 || gripping {
            vr.gripping[i] = gripping;
        }
    }
    local.cmd.gun_hand = vr.gun;
    local.cmd.vr = true;
    let trigger = vr.gun.map_or(0.0, |h| input.hands[h.index()].trigger);
    let was = vr.firing;
    vr.firing = if was { trigger > 0.5 } else { trigger > 0.75 };
    // (A panel open: the trigger points at it; a pull then doesn't fire when it closes.)
    let blocked = vr.ui;
    local.cmd.fire = vr.firing && alive && !blocked;
    if vr.firing && !was && alive && !blocked {
        local.fire_latch = true;
    }

    // The shield: the trigger of the hand without the blaster (the left one, unless it holds
    // the blaster).
    let free = if vr.gun == Some(Hand::Left) {
        Hand::Right
    } else {
        Hand::Left
    };
    local.cmd.shield = alive && !blocked && input.hands[free.index()].trigger > 0.75;
    // The barrier: gripping with that hand away from the holster.
    local.cmd.barrier =
        alive && !blocked && !vr.near[free.index()] && input.hands[free.index()].squeeze > 0.75;

    // The menu (its button: `menu_buttons`), or B / Y held without taking a grenade.
    for i in 0..2 {
        let hand = &input.hands[i];
        let (held, opened) = vr.hold[i];
        if hand.secondary && vr.grenade != Some(hand_of(i)) {
            let held = held + dt;
            let open = held >= MENU_HOLD && !opened;
            if open {
                vr.menu_requests += 1;
                rumble.play(i, 0.0, 0.03, 0.5);
            }
            vr.hold[i] = (held, opened || open);
        } else {
            vr.hold[i] = (0.0, false);
        }
    }

    // The grenade: B or Y, on the hand not holding the blaster.
    match vr.grenade {
        Some(h) if !input.hands[h.index()].secondary => {
            vr.grenade = None;
            vr.threw = Some(h);
            // Thrown with the hand's speed over the last 80 ms.
            let trail = &vr.trail[h.index()];
            if let (Some(&(t1, p1)), true) = (trail.back(), alive) {
                let (t0, p0) = trail
                    .iter()
                    .find(|(t, _)| t1 - *t <= 0.08)
                    .copied()
                    .unwrap_or((t1, p1));
                let v = if t1 - t0 > 0.005 {
                    (p1 - p0) / (t1 - t0)
                } else {
                    Vec3::ZERO
                };
                let feet = local.cmd.feet.unwrap_or_default();
                // (The sim throws it `Rules::vr_throw_factor` times as hard: `PlayerCmd::vr`.)
                local.cmd.throw = Some((p1 - feet, v * 1.3));
            }
        }
        None => {
            let has = game
                .me
                .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
                .is_some_and(|p| p.grenades > 0);
            for h in [Hand::Right, Hand::Left] {
                // (Not while B / Y is held for the menu, nor with a panel up.)
                if has
                    && input.hands[h.index()].secondary
                    && vr.gun != Some(h)
                    && vr.hold[h.index()].0 < 0.25
                    && !blocked
                {
                    vr.grenade = Some(h);
                    break;
                }
            }
        }
        _ => {}
    }
    local.cmd.grenade_hand = vr.grenade;
}

#[allow(clippy::too_many_arguments)]
fn place_rig(
    input: Res<XrInput>,
    mut vr: ResMut<VrState>,
    mut local: ResMut<LocalCmd>,
    mover: Res<Mover>,
    game: Res<Game>,
    models: Res<Models>,
    rig: Option<Res<XrRig>>,
    mut transforms: Query<&mut Transform>,
    time: Res<Time>,
) {
    // Feet: the prediction, or where the simulation has us (waiting to respawn).
    let me = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
    let feet = if mover.valid && me.is_some_and(|p| p.respawn.is_none()) {
        mover.feet
    } else {
        me.map_or(Vec3::ZERO, |p| p.feet)
    };
    let rot = Quat::from_rotation_y(vr.yaw);
    let head = input
        .head
        .unwrap_or(Transform::from_xyz(0.0, lt_sim::sim::EYE, 0.0));
    let h = head.translation;
    vr.rig = Transform {
        translation: feet - rot * Vec3::new(h.x, 0.0, h.z),
        rotation: rot,
        ..default()
    };
    if let Some(r) = rig
        && let Ok(mut t) = transforms.get_mut(r.0)
    {
        *t = vr.rig;
    }
    // Poses relative to the feet, in world orientation.
    let rig_t = vr.rig;
    let world = |t: &Transform| rig_t * *t;
    let head_w = world(&head);
    local.cmd.head = Pose {
        pos: head_w.translation - feet,
        rot: head_w.rotation,
    };
    let now = time.elapsed_secs();
    for (i, hand) in input.hands.iter().enumerate() {
        let Some(grip) = hand.grip else { continue };
        let g = world(&grip);
        // The blaster sits in the palm, pointing where the controller points.
        let rot = hand.aim.map_or(g.rotation, |a| world(&a).rotation);
        local.cmd.hands[i] = Pose {
            pos: g.translation - feet,
            rot,
        };
        let trail = &mut vr.trail[i];
        trail.push_back((now, g.translation));
        while trail.front().is_some_and(|(t, _)| now - t > 0.2) {
            trail.pop_front();
        }
    }
    if let Some(gh) = vr.gun {
        // The blaster held with its handle more upright in the hand: turned nose down about the
        // grip from the controller's aim (it fires along its barrel).
        let tilt = *GUN_TILT.get_or_init(|| {
            std::env::var("LT_GUN_TILT")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(30.0)
                .to_radians()
        });
        let hand = &mut local.cmd.hands[gh.index()];
        hand.rot *= Quat::from_rotation_x(-tilt);
        let p = *hand;
        let shotgun = game
            .me
            .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
            .is_some_and(|p| p.shotgun);
        let muzzle = p.pos
            + p.rot
                * if shotgun {
                    models.shotgun_muzzle
                } else {
                    models.muzzle
                };
        local.cmd.muzzle = Pose {
            pos: muzzle,
            rot: look_rot(p.rot * Vec3::NEG_Z),
        };
    }
    // The holster in the world: muzzle down, tilted a little forward.
    vr.holster_world = vr.holster.zip(vr.body_yaw).map(|(at, body)| {
        let rot =
            Quat::from_rotation_y(body) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2 + 0.3);
        rig_t * Transform::from_translation(at).with_rotation(rot)
    });
}

/// The blaster in its holster (with a faint ring) while no hand holds it.
#[derive(Resource)]
struct Holster {
    root: Entity,
    gun: Entity,
    ring: Entity,
    /// What the holster shows: the team's colours, and the shotgun instead of the blaster.
    team: Option<Team>,
    shotgun: bool,
    dim: Handle<AccentMaterial>,
    bright: Handle<AccentMaterial>,
}

fn spawn_holster(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    let root = commands
        .spawn((
            Name::new("holster"),
            Transform::IDENTITY,
            Visibility::Hidden,
        ))
        .id();
    let gun = commands
        .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
        .id();
    let dim = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(team_color(None), 0.5),
    });
    let bright = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(team_color(None), 2.5),
    });
    // A thin ring around the blaster's grip, level with the floor.
    let ring = commands
        .spawn((
            Mesh3d(meshes.add(Torus::new(0.062, 0.07))),
            MeshMaterial3d(dim.clone()),
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    commands.insert_resource(Holster {
        root,
        gun,
        ring,
        team: None,
        shotgun: false,
        dim,
        bright,
    });
}

#[allow(clippy::too_many_arguments)]
fn holster(
    mut commands: Commands,
    holster: Option<ResMut<Holster>>,
    vr: Res<VrState>,
    game: Res<Game>,
    local: Res<LocalCmd>,
    models: Res<Models>,
    palette: Res<Palette>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    mut mats: Query<&mut MeshMaterial3d<AccentMaterial>>,
) {
    let Some(mut h) = holster else { return };
    let me = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
    let team = me.map(|p| p.team);
    let shotgun = me.is_some_and(|p| p.shotgun);
    // The blaster (or the shotgun) in the team's colours.
    if team != h.team || shotgun != h.shotgun {
        h.team = team;
        h.shotgun = shotgun;
        commands.entity(h.gun).despawn_children();
        let model = if shotgun {
            &models.shotgun
        } else {
            &models.blaster
        };
        crate::actors::spawn_model(&mut commands, h.gun, model, &palette, team);
        for (m, k) in [(&h.dim, 0.5), (&h.bright, 2.5)] {
            if let Some(mut m) = accents.get_mut(m) {
                m.neon = NeonUniform::fixed(team_color(team), k);
            }
        }
    }
    let alive = me.is_some_and(|p| p.respawn.is_none());
    let show =
        alive && local.cmd.gun_hand.is_none() && vr.holster_world.is_some() && !local.autopilot;
    for e in [h.root, h.ring] {
        if let Ok(mut v) = vis.get_mut(e) {
            let want = if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
        }
    }
    let Some(at) = vr.holster_world else { return };
    if let Ok(mut t) = transforms.get_mut(h.root) {
        // The grip (the model's origin) a little above the holster's middle.
        *t = at;
        t.translation += Vec3::Y * 0.06;
    }
    if let Ok(mut t) = transforms.get_mut(h.ring) {
        *t = Transform::from_translation(at.translation + Vec3::Y * 0.02);
    }
    let near = vr.near.iter().any(|n| *n);
    if let Ok(mut m) = mats.get_mut(h.ring) {
        let want = if near {
            h.bright.clone()
        } else {
            h.dim.clone()
        };
        if m.0 != want {
            m.0 = want;
        }
    }
}

/// The display on the wrist of the free hand: hits left, a grenade carried, shots left (the
/// respawn countdown while tagged out), the powerups one has; Megatarget mode: above them a dot per secondary target
/// hit, or the seconds left to get into the megatarget's room.
#[derive(Resource)]
struct Wrist {
    root: Entity,
    digits: Vec<Digit>,
    bars: Vec<Entity>,
    grenade: Entity,
    /// A big countdown in front of the eyes while tagged out.
    panel: Entity,
    panel_digits: Vec<Digit>,
    mega_digits: Vec<Digit>,
    mega_pips: [Entity; 2],
    /// Under the hit bars: the match's time left (blinking in its last ten seconds), "2X"
    /// while points count double, and a blackout's seconds left.
    clock: Vec<Digit>,
    colon: Entity,
    double: Entity,
    blackout: Entity,
    blackout_digits: Vec<Digit>,
    /// Small copies of the powerups one has, in a row along the top, and under each a bar in
    /// its colour draining with its time left (shown while it runs).
    powerups: Vec<(PickupKind, Entity, Entity)>,
}

/// Hits-left bars on the wrist at most.
const MAX_BARS: usize = 6;

/// The wrist's powerup icons' size (of the pickups').
const ICON_SCALE: f32 = 0.08;
/// The width of a full powerup timer bar under its icon (m).
const TIMER_WIDTH: f32 = 0.024;

/// How full a running powerup's timer is (shield, cloak, speed boost; the charge gun's charge),
/// `None` while it isn't running.
fn powerup_timer(
    p: &lt_sim::sim::PlayerSnap,
    kind: PickupKind,
    rules: &lt_sim::Rules,
) -> Option<f32> {
    let k = match kind {
        PickupKind::Shield if p.shield > 0.0 => p.shield / rules.shield_time,
        PickupKind::Cloak if p.cloak > 0.0 => p.cloak / rules.cloak_time,
        PickupKind::Speed if p.speed > 0.0 => p.speed / rules.speed_time,
        PickupKind::Charge if p.charging > 0.0 => p.charging / rules.charge_time,
        _ => return None,
    };
    Some(k.clamp(0.0, 1.0))
}

fn spawn_wrist(
    mut commands: Commands,
    palette: Res<Palette>,
    models: Res<Models>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let root = commands
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    let white = palette.accent[2].clone();
    let digits = spawn_digits(
        &mut commands,
        &mut meshes,
        root,
        Vec3::new(-0.04, 0.03, 0.0),
        2,
        0.025,
        &white,
    );
    let bar = meshes.add(Cuboid::new(0.018, 0.006, 0.004));
    // (Laid out by `wrist` for the hits that tag a player out.)
    let bars = (0..MAX_BARS)
        .map(|_| {
            commands
                .spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(white.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id()
        })
        .collect();
    let grenade = commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(0.006))),
            MeshMaterial3d(white.clone()),
            Transform::from_xyz(0.05, 0.0, 0.0),
            ChildOf(root),
        ))
        .id();
    let mega_digits = spawn_digits(
        &mut commands,
        &mut meshes,
        root,
        Vec3::new(-0.03, 0.075, 0.0),
        2,
        0.016,
        &white,
    );
    let pip = meshes.add(Sphere::new(0.007));
    let mega_pips = [0, 1].map(|k| {
        commands
            .spawn((
                Mesh3d(pip.clone()),
                MeshMaterial3d(white.clone()),
                Transform::from_xyz(-0.012 + k as f32 * 0.024, 0.06, 0.0),
                Visibility::Hidden,
                ChildOf(root),
            ))
            .id()
    });
    let timer = meshes.add(Cuboid::new(TIMER_WIDTH, 0.003, 0.002));
    let powerups = PickupKind::POWERUPS
        .iter()
        .filter_map(|&k| {
            let model = models.powerups.get(&k)?;
            let e = commands
                .spawn((
                    Transform::from_scale(Vec3::splat(ICON_SCALE)),
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id();
            crate::actors::spawn_powerup(&mut commands, e, model, &palette, k);
            let accent = palette
                .pickup
                .get(&k)
                .map_or(white.clone(), |m| m.0.clone());
            let bar = commands
                .spawn((
                    Mesh3d(timer.clone()),
                    MeshMaterial3d(accent),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id();
            Some((k, e, bar))
        })
        .collect();
    // The clock: M SS with a colon of two dots between.
    let clock = [(-0.035, 1), (-0.012, 2)]
        .into_iter()
        .flat_map(|(x, n)| {
            spawn_digits(
                &mut commands,
                &mut meshes,
                root,
                Vec3::new(x, -0.03, 0.0),
                n,
                0.016,
                &white,
            )
        })
        .collect();
    let colon = commands
        .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
        .id();
    let dot = meshes.add(Cuboid::new(0.002, 0.002, 0.001));
    for y in [-0.026, -0.034] {
        commands.spawn((
            Mesh3d(dot.clone()),
            MeshMaterial3d(white.clone()),
            Transform::from_xyz(-0.0245, y, 0.0),
            ChildOf(colon),
        ));
    }
    let gold = palette
        .pickup
        .get(&PickupKind::Rapid)
        .map_or(white.clone(), |m| m.0.clone());
    let label = |commands: &mut Commands, meshes: &mut Assets<Mesh>, text: &str, at: Vec3, h| {
        let e = commands
            .spawn((Transform::IDENTITY, Visibility::Hidden, ChildOf(root)))
            .id();
        segtext::spawn(commands, meshes, e, text, h, at, Align::Left, &gold, None);
        e
    };
    let double = label(
        &mut commands,
        &mut meshes,
        "2X",
        Vec3::new(0.022, -0.03, 0.0),
        0.012,
    );
    let blackout = label(
        &mut commands,
        &mut meshes,
        "DARK",
        Vec3::new(-0.04, -0.055, 0.0),
        0.008,
    );
    let blackout_digits = spawn_digits(
        &mut commands,
        &mut meshes,
        blackout,
        Vec3::new(0.002, -0.055, 0.0),
        2,
        0.01,
        &white,
    );
    let panel = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden))
        .id();
    let panel_digits = spawn_digits(
        &mut commands,
        &mut meshes,
        panel,
        Vec3::new(-0.12, 0.0, 0.0),
        2,
        0.3,
        &white,
    );
    commands.insert_resource(Wrist {
        root,
        digits,
        bars,
        grenade,
        panel,
        panel_digits,
        mega_digits,
        mega_pips,
        clock,
        colon,
        double,
        blackout,
        blackout_digits,
        powerups,
    });
}

#[allow(clippy::too_many_arguments)]
fn wrist(
    wrist: Option<Res<Wrist>>,
    vr: Res<VrState>,
    local: Res<LocalCmd>,
    game: Res<Game>,
    mover: Res<Mover>,
    time: Res<Time>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
) {
    let Some(w) = wrist else { return };
    let me = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
    let Some(p) = me else { return };
    if let Ok(mut v) = vis.get_mut(w.root)
        && *v == Visibility::Hidden
    {
        *v = Visibility::Inherited;
    }
    let feet = if mover.valid { mover.feet } else { p.feet };
    // On the hand without the blaster (the left one by default).
    let hand = if vr.gun == Some(Hand::Left) {
        Hand::Right
    } else {
        Hand::Left
    };
    let pose = local.cmd.hands[hand.index()];
    if let Ok(mut t) = transforms.get_mut(w.root) {
        // On the back of the wrist, tilted towards the eyes, floating clear of the glove (it
        // rises up to 21 mm through the display's plane at the cuff).
        let tilt = Quat::from_rotation_x(-1.2);
        t.translation =
            feet + pose.pos + pose.rot * (Vec3::new(0.0, 0.035, 0.06) + tilt * Vec3::Z * 0.03);
        t.rotation = pose.rot * tilt;
    }
    let blink =
        (p.cooldown <= 0.0 && p.protect <= 0.0) || (time.elapsed_secs() * 8.0).fract() < 0.5;
    let n = game.rules.hits_to_tag.min(MAX_BARS as u8);
    let left = n.saturating_sub(p.hits);
    // (Three bars 24 mm apart; more, closer and narrower, over the same width.)
    let step = 0.048 / (n.max(2) - 1) as f32;
    for (k, e) in w.bars.iter().enumerate() {
        if let Ok(mut t) = transforms.get_mut(*e) {
            t.translation = Vec3::new(-0.03 + k as f32 * step, 0.0, 0.0);
            t.scale = Vec3::new((step / 0.024).min(1.0), 1.0, 1.0);
        }
        if let Ok(mut v) = vis.get_mut(*e) {
            *v = if (k as u8) < left && blink {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    // The powerups, side by side along the top (a running shield flickering in its last two
    // seconds).
    let mut x = -0.03;
    for &(kind, e, bar) in &w.powerups {
        let has = p.respawn.is_none() && p.has(kind);
        // (A running shield, cloak or speed boost flickers in its last two seconds.)
        let ending = match kind {
            PickupKind::Shield => p.shield > 0.0 && p.shield <= 2.0,
            PickupKind::Speed => p.speed <= 2.0,
            PickupKind::Cloak => p.cloak <= 2.0,
            _ => false,
        };
        let timer = powerup_timer(p, kind, &game.rules).filter(|_| has);
        if let Ok(mut v) = vis.get_mut(bar) {
            *v = if timer.is_some() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        if let Some(k) = timer
            && let Ok(mut t) = transforms.get_mut(bar)
        {
            // (Draining towards its left end.)
            t.translation = Vec3::new(x - TIMER_WIDTH / 2.0 * (1.0 - k), 0.088, 0.0);
            t.scale = Vec3::new(k.max(0.01), 1.0, 1.0);
        }
        let on = has && (!ending || (time.elapsed_secs() * 10.0).fract() < 0.6);
        if let Ok(mut v) = vis.get_mut(e) {
            *v = if on {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        if has && let Ok(mut t) = transforms.get_mut(e) {
            t.translation = Vec3::new(x, 0.11, 0.0);
            t.rotation = Quat::from_rotation_y(time.elapsed_secs() * 1.5);
            x += 0.03;
        }
    }
    // The clock, blinking in the last ten seconds.
    let clock_left = game.snap.time_left.max(0.0);
    let ticking = clock_left > 10.0 || game.snap.over || clock_left.fract() > 0.3;
    let s = clock_left.ceil() as u32;
    show(
        &if ticking {
            format!("{}{:02}", (s / 60).min(9), s % 60)
        } else {
            String::new()
        },
        &w.clock,
        &mut vis,
    );
    let toggle = |vis: &mut Query<&mut Visibility>, e, on: bool| {
        if let Ok(mut v) = vis.get_mut(e) {
            let want = if on {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
        }
    };
    toggle(&mut vis, w.colon, ticking);
    toggle(&mut vis, w.double, game.snap.double);
    let dark = match game.snap.event {
        Some((lt_sim::ArenaEvent::Blackout, t)) => Some(t),
        _ => None,
    };
    toggle(&mut vis, w.blackout, dark.is_some());
    show(
        &dark.map_or(String::new(), |t| {
            format!("{:>2}", (t.ceil() as u32).min(99))
        }),
        &w.blackout_digits,
        &mut vis,
    );
    if let Ok(mut v) = vis.get_mut(w.grenade) {
        *v = if p.grenades > 0 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    let secs = p.respawn.map(|r| r.ceil() as u32);
    // Shots left (blinking when empty), or the respawn countdown.
    // (Blinking fast while low, slowly when empty.)
    let low = p.ammo as f32 <= game.rules.ammo as f32 * crate::hud::LOW_AMMO && !p.reloading;
    let ammo = if p.ammo > 0 && (!low || (time.elapsed_secs() * 3.0).fract() < 0.7)
        || p.ammo == 0 && blink_slow(&time)
    {
        format!("{:>2}", p.ammo.min(99))
    } else {
        String::new()
    };
    show(
        &secs.map_or(ammo, |s| format!("{s:>2}")),
        &w.digits,
        &mut vis,
    );
    show(
        &secs.map_or(String::new(), |s| format!("{s:>2}")),
        &w.panel_digits,
        &mut vis,
    );
    let mega = game.level.mega.is_some() && p.respawn.is_none();
    let power = p.power.filter(|_| mega);
    show(
        &power.map_or(String::new(), |t| {
            format!("{:>2}", (t.ceil() as u32).min(99))
        }),
        &w.mega_digits,
        &mut vis,
    );
    for (k, e) in w.mega_pips.iter().enumerate() {
        if let Ok(mut v) = vis.get_mut(*e) {
            *v = if mega && power.is_none() && p.mega_hits & (1 << k) != 0 {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    if let Ok(mut v) = vis.get_mut(w.panel) {
        *v = if secs.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if let Ok(mut t) = transforms.get_mut(w.panel) {
        let head = feet + local.cmd.head.pos;
        let yaw = flat_yaw(local.cmd.head.rot);
        let r = Quat::from_rotation_y(yaw);
        t.translation = head + r * Vec3::new(0.0, -0.1, -1.6);
        t.rotation = r;
    }
}

fn blink_slow(time: &Time) -> bool {
    (time.elapsed_secs() * 2.0).fract() < 0.5
}

/// Seconds the points just scored float over the wrist.
const POINTS_LIFE: f32 = 1.6;

/// The points the local player scores ("+100 TAG"), rising over the wrist display and gone.
#[allow(clippy::too_many_arguments)]
fn wrist_points(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    wrist: Option<Res<Wrist>>,
    palette: Res<Palette>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<&mut Transform>,
    mut shown: Local<Vec<(Entity, f32)>>,
) {
    let Some(w) = wrist else { return };
    for GameEvent(e) in events.read() {
        let Some((what, points)) = crate::hud::my_points(e, &game) else {
            continue;
        };
        let gold = palette
            .pickup
            .get(&PickupKind::Rapid)
            .map_or(palette.accent[2].clone(), |m| m.0.clone());
        let root = commands
            .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(w.root)))
            .id();
        let x2 = if game.snap.double { " X2" } else { "" };
        segtext::spawn(
            &mut commands,
            &mut meshes,
            root,
            &format!("+{points} {what}{x2}"),
            0.01,
            Vec3::ZERO,
            Align::Center,
            &gold,
            None,
        );
        shown.push((root, 0.0));
    }
    let dt = time.delta_secs();
    let n = shown.len();
    for (i, (e, age)) in shown.iter_mut().enumerate() {
        *age += dt;
        if let Ok(mut t) = transforms.get_mut(*e) {
            // (Above the powerups, the newest lowest, rising; shrinking away at the end.)
            let y = 0.14 + 0.016 * (n - 1 - i) as f32 + 0.02 * *age;
            t.translation = Vec3::new(0.0, y, 0.0);
            t.scale = Vec3::splat(((POINTS_LIFE - *age) / 0.3).clamp(0.0, 1.0));
        }
    }
    shown.retain(|(e, age)| {
        let keep = *age < POINTS_LIFE;
        if !keep && let Ok(mut c) = commands.get_entity(*e) {
            c.despawn();
        }
        keep
    });
}

/// The game's events as vibration patterns.
fn haptics(
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    vr: Res<VrState>,
    local: Res<LocalCmd>,
    mover: Res<Mover>,
    mut rumble: ResMut<Rumble>,
    mut was_low: Local<bool>,
) {
    let Some(me) = game.me else { return };
    let gun = vr.gun.map_or(1, |h| h.index());
    for GameEvent(e) in events.read() {
        match *e {
            // A short sharp kick.
            SimEvent::Fired { player, .. } if player == me => {
                rumble.play(gun, 0.0, 0.025, 0.8);
                rumble.fade(gun, 0.025, 0.04, 0.3);
            }
            // Click click.
            SimEvent::Empty { player } if player == me => {
                rumble.play(gun, 0.0, 0.01, 0.3);
                rumble.play(gun, 0.08, 0.01, 0.3);
            }
            SimEvent::Denied { player } if player == me => rumble.play(gun, 0.0, 0.015, 0.2),
            SimEvent::PickedUp { player, .. } if player == me => {
                rumble.both(0.0, 0.05, 0.3);
                rumble.both(0.12, 0.05, 0.3);
            }
            SimEvent::GrenadeThrown { player, .. } if player == me => {
                let h = vr.threw.map_or(0, |h| h.index());
                rumble.fade(h, 0.0, 0.12, 0.45);
            }
            // Strong on both hands, stronger on the side it came from.
            SimEvent::Hit { player, by, .. } if player == me => {
                let eye = mover.feet + local.cmd.head.pos;
                let side = crate::hud::shooter_pos(&game, by).map_or(0.0, |from| {
                    let d = from - eye;
                    let right = local.cmd.head.rot * Vec3::X;
                    Vec3::new(d.x, 0.0, d.z)
                        .normalize_or_zero()
                        .dot(Vec3::new(right.x, 0.0, right.z).normalize_or_zero())
                });
                for (i, s) in [(0, -side), (1, side)] {
                    let amp = (0.7 + 0.3 * s).clamp(0.4, 1.0);
                    rumble.play(i, 0.0, 0.1, amp);
                    rumble.fade(i, 0.1, 0.2, amp * 0.5);
                }
            }
            // A long rumble.
            SimEvent::TaggedOut { player, .. } if player == me => {
                rumble.both(0.0, 0.15, 1.0);
                rumble.fade(0, 0.15, 0.7, 0.8);
                rumble.fade(1, 0.15, 0.7, 0.8);
            }
            SimEvent::TargetHit { by, .. } if by == me => {
                rumble.play(gun, 0.0, 0.05, 0.8);
                rumble.play(gun, 0.1, 0.05, 0.8);
            }
            // The charge gun: ready (a double tick), and the beam's heavy kick.
            SimEvent::ChargeFull { player } if player == me => {
                rumble.play(gun, 0.0, 0.02, 0.6);
                rumble.play(gun, 0.06, 0.02, 0.6);
            }
            SimEvent::BeamFired { player, .. } if player == me => {
                rumble.play(gun, 0.0, 0.06, 1.0);
                rumble.fade(gun, 0.06, 0.2, 0.6);
            }
            SimEvent::TaggedOut { by, player, .. } if by == me && player != me => {
                rumble.play(gun, 0.0, 0.06, 0.9);
                rumble.play(gun, 0.12, 0.04, 0.6);
            }
            // A running powerup over: a double tick on the wrist's hand.
            SimEvent::ShieldDown { player }
            | SimEvent::Uncloaked { player }
            | SimEvent::SpeedDown { player }
                if player == me =>
            {
                rumble.play(1 - gun, 0.0, 0.03, 0.45);
                rumble.play(1 - gun, 0.1, 0.03, 0.45);
            }
            // Healed: two soft pulses in both hands.
            SimEvent::Healed { player } if player == me => {
                rumble.both(0.0, 0.08, 0.25);
                rumble.both(0.15, 0.08, 0.25);
            }
            // A light tick for each hit on an enemy.
            SimEvent::Hit { .. } if crate::hud::hit_confirm(e, &game) == Some(false) => {
                rumble.play(gun, 0.0, 0.02, 0.5);
            }
            _ => {}
        }
    }
    let p = game.snap.players.iter().find(|p| p.id == me);
    // Running low on shots: a double tick in the gun hand, once.
    let low = p.is_some_and(|p| {
        p.ammo > 0 && p.ammo as f32 <= game.rules.ammo as f32 * crate::hud::LOW_AMMO
    });
    if low && !*was_low {
        rumble.play(gun, 0.0, 0.03, 0.4);
        rumble.play(gun, 0.1, 0.03, 0.4);
    }
    *was_low = low;
    // Charging: a hum in the gun hand growing as it charges.
    let charging = p.map_or(0.0, |p| p.charging);
    if charging > 0.0 {
        let k = (charging / game.rules.charge_time).min(1.0);
        rumble.play(gun, 0.0, 0.02, 0.08 + 0.25 * k);
    }
}

/// This frame's amplitude per hand: the strongest pulse playing.
fn play_rumble(mut rumble: ResMut<Rumble>, mut input: ResMut<XrInput>, time: Res<Time>) {
    let dt = time.delta_secs();
    let mut amp = [0.0f32; 2];
    for (hand, wait, secs, a, fade, played) in rumble.pulses.iter_mut() {
        if *wait > 0.0 {
            *wait -= dt;
            continue;
        }
        let k = if *fade {
            (1.0 - *played / *secs).max(0.0)
        } else {
            1.0
        };
        amp[*hand & 1] = amp[*hand & 1].max(*a * k);
        *played += dt;
    }
    rumble.pulses.retain(|p| p.1 > 0.0 || p.5 < p.2);
    for (h, a) in input.haptic.iter_mut().zip(amp) {
        *h = h.max(a.min(1.0));
    }
}

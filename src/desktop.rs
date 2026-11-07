//! Playing on a flat screen: mouse look, WASD, Ctrl crouches, left button fires, right button (or G) holds a
//! grenade and throws it when let go. Esc opens the in-game menu (src/pause.rs); while it or
//! the end of match screen (src/endscreen.rs) is up, the mouse is free and the game ignores
//! the input.

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use lt_sim::sim::{EYE, eye_on_neck, look_rot};
use lt_sim::{Hand, Pose, SimEvent};

use crate::assets::Models;
use crate::game::{Game, GameEvent, InputSet, LocalCmd, Mover, PredictSet, alpha};
use crate::settings::{Action, Inputs, Settings};

#[derive(Component)]
pub struct MainCamera;

/// The local player's view direction (desktop).
#[derive(Resource, Default)]
pub struct Look {
    pub yaw: f32,
    pub pitch: f32,
    /// The camera follows the simulation's yaw (an autopilot bot plays).
    pub follow_sim: bool,
    /// Crouching: 0 standing, 1 all the way down (eased).
    pub crouch: f32,
    /// Crouching by toggle (`Settings::toggle_crouch`): down until pressed again.
    pub crouch_on: bool,
}

/// How far crouching lowers the eyes (m), how long it takes (time constant, s), and how much
/// it slows walking.
const CROUCH_DROP: f32 = 0.6;
const CROUCH_TIME: f32 = 0.07;

impl Look {
    /// The eyes' height above the feet.
    pub fn eye_height(&self) -> f32 {
        EYE - CROUCH_DROP * self.crouch
    }
}

/// Head bob and weapon sway (flat screen): worked out from the movement and the mouse each
/// frame, applied to the camera and to the blaster in the hand.
#[derive(Resource, Default)]
pub struct Sway {
    /// Walking: 0 standing still, 1 at full speed (eased).
    amount: f32,
    /// Advances by half a turn a step, times `BOB_RATE`.
    phase: f32,
    /// Turning speed (rad/s: yaw, pitch), eased.
    turn: Vec2,
    /// The camera's offset in view space.
    pub camera: Vec3,
}

/// While a grenade is held: where it would be thrown from (world) and with what velocity (and
/// whether to show it anyway, for testing).
#[derive(Resource, Default)]
pub struct ThrowArc(Option<(Vec3, Vec3)>, pub bool);

/// The arc's drawing: dots along the path, and a ring where the grenade will go off.
#[derive(Resource)]
struct ArcView {
    dots: Vec<Entity>,
    ring: Entity,
}

/// Dots on the arc: one every this many ticks of flight, at most this many.
const ARC_STRIDE: usize = 3;
const ARC_DOTS: usize = 60;

/// Metres a step (as the footsteps in src/audio.rs).
const STRIDE: f32 = 0.8;
/// The head bob and the blaster's walking sway run slower than the steps: this many swings a
/// step (a full step's rate felt jittery).
const BOB_RATE: f32 = 0.3;

/// Flat-screen overlays that take the mouse (the game ignores the input while one is up).
#[derive(Resource, Default)]
pub struct Overlays {
    /// The in-game menu (Esc).
    pub menu: bool,
    /// The end of match screen.
    pub end: bool,
    /// The options screen (src/options.rs; also over the start menu).
    pub options: bool,
    /// The left button went down on an overlay: it doesn't fire until let go.
    pub hold_fire: bool,
}

impl Overlays {
    pub fn any(&self) -> bool {
        self.menu || self.end || self.options
    }
}

pub struct DesktopPlugin {
    pub autopilot: bool,
}

impl Plugin for DesktopPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Look {
            follow_sim: self.autopilot,
            ..default()
        })
        .init_resource::<Overlays>()
        .init_resource::<Sway>()
        .init_resource::<ThrowArc>()
        .add_systems(Startup, (spawn_camera, spawn_arc))
        .add_systems(Update, (grab_cursor, read_input.in_set(InputSet)).chain())
        .add_systems(
            Update,
            (on_events, place_camera, draw_arc)
                .chain()
                .after(PredictSet),
        )
        .add_systems(Update, apply_fov);
    }
}

pub fn camera_bundle() -> impl Bundle {
    (
        Camera3d::default(),
        Hdr,
        Tonemapping::AcesFitted,
        Bloom {
            intensity: 0.22,
            ..Bloom::NATURAL
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 80f32.to_radians(),
            near: 0.03,
            ..default()
        }),
        Msaa::Sample4,
    )
}

/// The field of view from the settings (`Settings::fov`).
fn apply_fov(settings: Res<Settings>, mut cams: Query<&mut Projection, With<MainCamera>>) {
    let want = settings.fov.to_radians();
    for mut p in &mut cams {
        let off = matches!(&*p, Projection::Perspective(pp) if (pp.fov - want).abs() > 1e-4);
        if off && let Projection::Perspective(pp) = &mut *p {
            pp.fov = want;
        }
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        MainCamera,
        camera_bundle(),
        Transform::from_xyz(0.0, 8.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y),
        crate::audio::listener(),
    ));
}

/// Clicking into the game takes the mouse; an overlay (or no game) gives it back.
fn grab_cursor(
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut overlays: ResMut<Overlays>,
    game: Option<Res<Game>>,
) {
    let Ok(mut c) = cursor.single_mut() else {
        return;
    };
    if overlays.any() || game.is_none() {
        if c.grab_mode != CursorGrabMode::None {
            set_grab(&mut c, false);
        }
        // The click on an overlay's button doesn't also fire.
        overlays.hold_fire = mouse.pressed(MouseButton::Left);
        return;
    }
    if !mouse.pressed(MouseButton::Left) {
        overlays.hold_fire = false;
    }
    if mouse.just_pressed(MouseButton::Left) && c.grab_mode == CursorGrabMode::None {
        set_grab(&mut c, true);
    }
}

pub fn set_grab(c: &mut CursorOptions, grab: bool) {
    c.grab_mode = if grab {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    c.visible = !grab;
}

/// The local player's feet, interpolated between ticks.
pub fn my_feet(game: &Game, a: f32) -> Option<Vec3> {
    let me = game.me?;
    let now = game.snap.players.iter().find(|p| p.id == me)?;
    let before = game
        .prev
        .players
        .iter()
        .find(|p| p.id == me)
        .map_or(now.feet, |p| p.feet);
    Some(before.lerp(now.feet, a))
}

/// Where the blaster is held, relative to the eye, in view space; and the shotgun.
const GRIP: Vec3 = Vec3::new(0.19, -0.2, -0.32);
const SHOTGUN_GRIP: Vec3 = Vec3::new(0.16, -0.24, -0.6);

#[allow(clippy::too_many_arguments)]
fn read_input(
    game: Option<Res<Game>>,
    mut local: ResMut<LocalCmd>,
    mut look: ResMut<Look>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    models: Res<Models>,
    time: Res<Time<Fixed>>,
    mover: Res<Mover>,
    overlays: Res<Overlays>,
    settings: Res<Settings>,
    mut sway: ResMut<Sway>,
    real: Res<Time>,
    mut arc: ResMut<ThrowArc>,
) {
    let Some(game) = game else { return };
    let grabbed = !overlays.any()
        && cursor
            .single()
            .is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    let input = Inputs {
        keys: &keys,
        mouse: &mouse,
    };
    let on = |a: Action| settings.pressed(a, &input);
    let dt = real.delta_secs().clamp(1e-4, 0.05);
    let mut turned = Vec2::ZERO;
    if grabbed {
        let s = 0.0022 * settings.sensitivity;
        let dy = if settings.invert_y {
            -motion.delta.y
        } else {
            motion.delta.y
        };
        let pitch = (look.pitch - dy * s).clamp(-1.45, 1.45);
        turned = Vec2::new(-motion.delta.x * s, pitch - look.pitch);
        look.yaw -= motion.delta.x * s;
        look.pitch = pitch;
    }
    let (mut gun_offset, mut gun_rot) = sway.update(dt, turned, &mover, game.rules.move_speed);
    if !settings.head_bob {
        sway.camera = Vec3::ZERO;
        (gun_offset, gun_rot) = (Vec3::ZERO, Quat::IDENTITY);
    }
    let fwd = Vec2::new(-look.yaw.sin(), -look.yaw.cos());
    let right = Vec2::new(look.yaw.cos(), -look.yaw.sin());
    let mut stick = Vec2::ZERO;
    if on(Action::Forward) {
        stick += fwd;
    }
    if on(Action::Back) {
        stick -= fwd;
    }
    if on(Action::Right) {
        stick += right;
    }
    if on(Action::Left) {
        stick -= right;
    }
    let alive = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
        .is_some_and(|p| p.respawn.is_none());
    if settings.toggle_crouch {
        if grabbed && settings.just_pressed(Action::Crouch, &input) {
            look.crouch_on = !look.crouch_on;
        }
        // (Up again after being tagged out.)
        if !alive {
            look.crouch_on = false;
        }
    } else {
        look.crouch_on = false;
    }
    let crouching = if grabbed && (on(Action::Crouch) && !settings.toggle_crouch || look.crouch_on)
    {
        1.0
    } else {
        0.0
    };
    look.crouch += (crouching - look.crouch) * (1.0 - (-dt / CROUCH_TIME).exp());
    // (Crouching slows the sim's movement: `PlayerCmd::crouch`.)
    let mut stick = stick.normalize_or_zero();
    if on(Action::Walk) {
        stick *= 0.5;
    }
    let view = Quat::from_rotation_y(look.yaw) * Quat::from_rotation_x(look.pitch);
    // The camera stays over the feet; the head others see turns on its neck in front of it.
    let cam = Vec3::Y * look.eye_height();
    let head = eye_on_neck(cam, view);
    let shotgun = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
        .is_some_and(|p| p.shotgun);
    // (The shotgun further out, its stock clear of the eye.)
    let hold = if shotgun { SHOTGUN_GRIP } else { GRIP };
    let grip = cam + view * (hold + gun_offset);
    let grip_rot = view * gun_rot;
    let muzzle = grip
        + grip_rot
            * if shotgun {
                models.shotgun_muzzle
            } else {
                models.muzzle
            };
    // Aim the bolts at whatever is under the crosshair.
    let feet = if mover.valid {
        mover.feet
    } else {
        my_feet(&game, alpha(&time)).unwrap_or_default()
    };
    let eye = feet + cam;
    let ahead = eye + view * Vec3::NEG_Z * 80.0;
    let aim = game.world.ray(eye, ahead).map_or(ahead, |h| h.1);
    let muzzle_rot = look_rot(aim - (feet + muzzle));

    let cmd = &mut local.cmd;
    cmd.stick = if grabbed { stick } else { Vec2::ZERO };
    cmd.walk = Vec2::ZERO;
    cmd.yaw = look.yaw;
    cmd.crouch = crouching > 0.0;
    cmd.head = Pose {
        pos: head,
        rot: view,
    };
    cmd.gun_hand = Some(Hand::Right);
    cmd.hands[1] = Pose {
        pos: grip,
        rot: grip_rot,
    };
    cmd.muzzle = Pose {
        pos: muzzle,
        rot: muzzle_rot,
    };
    cmd.fire = grabbed && !overlays.hold_fire && on(Action::Fire);
    if grabbed && settings.just_pressed(Action::Jump, &input) {
        cmd.jumps = cmd.jumps.wrapping_add(1);
    }
    if grabbed && settings.just_pressed(Action::Dash, &input) {
        cmd.dashes = cmd.dashes.wrapping_add(1);
    }
    cmd.shield = grabbed && on(Action::Shield);
    cmd.barrier = grabbed && on(Action::Barrier);
    if grabbed && !overlays.hold_fire && settings.just_pressed(Action::Fire, &input) {
        local.fire_latch = true;
    }
    let cmd = &mut local.cmd;
    let has = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
        .is_some_and(|p| p.grenades > 0);
    // (`.1`: shown for testing, `LT_DEV` "arc".)
    let holding = grabbed && has && on(Action::Grenade) || arc.1;
    let released = settings.just_released(Action::Grenade, &input);
    let off_hand = cam + view * Vec3::new(-0.22, -0.18, -0.35);
    cmd.hands[0] = Pose {
        pos: off_hand,
        rot: view,
    };
    let throw_vel = view * Vec3::NEG_Z * 11.0 + Vec3::Y * 2.5;
    arc.0 = None;
    if holding {
        cmd.grenade_hand = Some(Hand::Left);
        arc.0 = Some((feet + off_hand, throw_vel));
    } else if cmd.grenade_hand.is_some() && released {
        cmd.grenade_hand = None;
        cmd.throw = Some((off_hand, throw_vel));
    } else {
        cmd.grenade_hand = None;
    }
}

impl Sway {
    /// Eases the walking and turning, and gives the blaster's offset and turn in view space
    /// (`turned`: the view's yaw and pitch change this frame).
    fn update(&mut self, dt: f32, turned: Vec2, mover: &Mover, max_speed: f32) -> (Vec3, Quat) {
        let speed = if mover.valid && mover.vy.abs() < 0.5 {
            mover.vel.length()
        } else {
            0.0
        };
        let k = |tau: f32| 1.0 - (-dt / tau).exp();
        self.amount += ((speed / max_speed.max(0.1)).min(1.0) - self.amount) * k(0.15);
        self.phase = (self.phase + speed * dt / STRIDE * BOB_RATE * std::f32::consts::PI)
            % std::f32::consts::TAU;
        self.turn = self.turn.lerp(
            (turned / dt).clamp(Vec2::splat(-12.0), Vec2::splat(12.0)),
            k(0.07),
        );
        let (a, p) = (self.amount, self.phase);
        // The head dips at each step and sways a little sideways every other.
        self.camera = Vec3::new(p.sin() * 0.012, (2.0 * p).cos() * 0.016, 0.0) * a;
        // The blaster: a figure of eight while walking, lagging behind when the view turns.
        let walk = Vec3::new(p.sin() * 0.008, (2.0 * p).cos() * 0.006 - 0.004, 0.0) * a;
        let t = self.turn;
        let lag =
            Vec3::new(t.x * 0.006, -t.y * 0.006, 0.0).clamp(Vec3::splat(-0.03), Vec3::splat(0.03));
        let rot = Quat::from_euler(
            EulerRot::YXZ,
            (-t.x * 0.025).clamp(-0.1, 0.1),
            (-t.y * 0.025).clamp(-0.08, 0.08),
            (-t.x * 0.02).clamp(-0.08, 0.08) + p.sin() * 0.015 * a,
        );
        (walk + lag, rot)
    }
}

/// The way the spawn pad at `pos` faces (towards the middle of the arena).
pub fn spawn_yaw(game: &Game, pos: Vec3) -> f32 {
    game.level
        .spawns
        .iter()
        .flatten()
        .min_by(|a, b| a.pos.distance(pos).total_cmp(&b.pos.distance(pos)))
        .map_or(0.0, |s| s.yaw)
}

fn spawn_arc(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    palette: Res<crate::materials::Palette>,
) {
    let dot = meshes.add(Sphere::new(0.03).mesh().ico(1).unwrap());
    let dots = (0..ARC_DOTS)
        .map(|_| {
            commands
                .spawn((
                    Mesh3d(dot.clone()),
                    MeshMaterial3d(palette.bolt_for(None)),
                    Transform::default(),
                    Visibility::Hidden,
                ))
                .id()
        })
        .collect();
    let ring = commands
        .spawn((
            Mesh3d(meshes.add(Torus::new(0.42, 0.5))),
            MeshMaterial3d(palette.accent_for(None)),
            Transform::default(),
            Visibility::Hidden,
        ))
        .id();
    commands.insert_resource(ArcView { dots, ring });
}

/// Shows the held grenade's flight: dots along it (scrolling outwards), the bounces, and a
/// ring where it will stop and go off.
fn draw_arc(
    arc: Res<ThrowArc>,
    view: Res<ArcView>,
    game: Option<Res<Game>>,
    time: Res<Time>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    let path = match (&arc.0, &game) {
        (Some((from, vel)), Some(game)) => Some(lt_sim::sim::grenade_path(
            &game.world,
            &game.rules,
            *from,
            *vel,
        )),
        _ => None,
    };
    let (points, end) = match &path {
        Some((p, _)) => (p.as_slice(), p.last().copied()),
        None => (&[][..], None),
    };
    // The dots move along the path (a third of the stride a tick): it reads as a throw.
    let shift = ((time.elapsed_secs() * 20.0) as usize) % ARC_STRIDE;
    for (k, &e) in view.dots.iter().enumerate() {
        let Ok((mut t, mut v)) = q.get_mut(e) else {
            continue;
        };
        // Not the first metre or so (right at the eye).
        let i = 6 + k * ARC_STRIDE + shift;
        match points.get(i) {
            Some(p) if i + 1 < points.len() => {
                t.translation = *p;
                // Smaller further along.
                t.scale = Vec3::splat(1.0 - 0.5 * i as f32 / points.len() as f32);
                *v = Visibility::Inherited;
            }
            _ => *v = Visibility::Hidden,
        }
    }
    if let Ok((mut t, mut v)) = q.get_mut(view.ring) {
        match end {
            Some(p) => {
                let pulse = 1.0 + 0.12 * (time.elapsed_secs() * 6.0).sin();
                *t = Transform::from_translation(p + Vec3::Y * 0.03)
                    .with_scale(Vec3::new(pulse, 0.4, pulse));
                *v = Visibility::Inherited;
            }
            None => *v = Visibility::Hidden,
        }
    }
}

fn on_events(
    mut events: MessageReader<GameEvent>,
    game: Option<Res<Game>>,
    mut look: ResMut<Look>,
    mut faced: Local<Option<u64>>,
    recap: Res<crate::game::Recap>,
    time: Res<Time>,
) {
    let Some(game) = game else { return };
    let me = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
    // Tagged out: the view turns to whoever did it, and follows them.
    if let Some(p) = me
        && p.respawn.is_some()
        && recap.out
        && !look.follow_sim
        && let Some(k) = recap
            .by
            .filter(|&by| by != p.id)
            .and_then(|by| game.snap.players.iter().find(|q| q.id == by))
        && k.respawn.is_none()
    {
        let d = k.feet + k.head.pos - (p.feet + Vec3::Y * look.eye_height());
        if d.length() > 0.5 {
            let yaw = lt_sim::sim::yaw_of(d);
            let pitch = (d.y / d.length()).asin().clamp(-1.2, 1.2);
            let k = 1.0 - (-time.delta_secs() / FACE_KILLER).exp();
            let dy = (yaw - look.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            look.yaw += dy * k;
            look.pitch += (pitch - look.pitch) * k;
        }
    }
    // Facing the middle of the arena on entering it and after respawning.
    if *faced != Some(game.level.seed)
        && let Some(p) = me
    {
        *faced = Some(game.level.seed);
        look.yaw = spawn_yaw(&game, p.feet);
        look.pitch = 0.0;
    }
    for GameEvent(e) in events.read() {
        if let SimEvent::Respawned { player, pos } = e
            && Some(*player) == game.me
        {
            // (Moved off the spawn pads for testing: the warp turned the view itself.)
            if game.level.spawn_at(*pos).is_some() {
                look.yaw = spawn_yaw(&game, *pos);
                look.pitch = 0.0;
            }
        }
    }
}

/// How quickly the view turns to the one who tagged the player out (time constant, s).
const FACE_KILLER: f32 = 0.25;

/// The start menu's view: its turning speed (rad/s) and upward tilt (rad).
const LOBBY_TURN: f32 = 0.05;
const LOBBY_PITCH: f32 = 0.3;

fn place_camera(
    game: Option<Res<Game>>,
    look: Res<Look>,
    sway: Res<Sway>,
    time: Res<Time<Fixed>>,
    mover: Res<Mover>,
    real: Res<Time<Real>>,
    mut cam: Query<&mut Transform, With<MainCamera>>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    let Some(game) = game else {
        // The start menu's scene (src/lobby.rs): from the middle of the empty arena, turning
        // slowly, looking up at the fireworks.
        let yaw = real.elapsed_secs() * LOBBY_TURN;
        t.translation = Vec3::Y * EYE;
        t.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(LOBBY_PITCH);
        return;
    };
    let a = alpha(&time);
    let Some(snap_feet) = my_feet(&game, a) else {
        return;
    };
    let alive = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me))
        .is_some_and(|p| p.respawn.is_none());
    let feet = if mover.valid && alive && !look.follow_sim {
        mover.feet
    } else {
        snap_feet
    };
    let (yaw, pitch) = if look.follow_sim {
        let me = game
            .me
            .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
        me.map_or((look.yaw, look.pitch), |p| {
            let (_, x, _) = p.head.rot.to_euler(EulerRot::YXZ);
            (p.yaw, x)
        })
    } else {
        (look.yaw, look.pitch)
    };
    t.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let bob = if alive && !look.follow_sim {
        Quat::from_rotation_y(yaw) * sway.camera
    } else {
        Vec3::ZERO
    };
    let eye = if look.follow_sim {
        EYE
    } else {
        look.eye_height()
    };
    t.translation = feet + Vec3::Y * eye + bob;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_bobs_and_turning_sways() {
        let mut sway = Sway::default();
        let still = || {
            let mut m = Mover::default();
            m.valid = true;
            m
        };
        let mut mover = still();
        let (off, rot) = sway.update(1.0 / 60.0, Vec2::ZERO, &mover, 4.0);
        assert_eq!(
            (off.length(), sway.camera.length()),
            (0.0, 0.0),
            "still: nothing moves"
        );
        assert!(rot.angle_between(Quat::IDENTITY) < 1e-4);
        mover.vel = Vec2::new(4.0, 0.0);
        let mut ys = Vec::new();
        for _ in 0..120 {
            sway.update(1.0 / 60.0, Vec2::ZERO, &mover, 4.0);
            ys.push(sway.camera.y);
        }
        let (lo, hi) = ys
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
        assert!(hi - lo > 0.02 && hi - lo < 0.05, "a slight bob: {lo}..{hi}");
        // Turning left: the blaster lags to the right and points a little right.
        let (off, rot) = sway.update(1.0 / 60.0, Vec2::new(0.05, 0.0), &still(), 4.0);
        let (off2, rot2) = sway.update(1.0 / 60.0, Vec2::new(0.05, 0.0), &still(), 4.0);
        assert!(off2.x > off.x && (rot2 * Vec3::NEG_Z).x > (rot * Vec3::NEG_Z).x);
    }
}

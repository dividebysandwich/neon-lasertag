//! The flat screen's free camera, for looking around and taking pictures: F12 in a game lifts
//! the camera off the player and hides the HUD. The mouse looks, the movement keys fly it
//! (Jump up, Crouch down, Dash faster, Walk slower), the wheel narrows or widens the view, and
//! Shift with the wheel opens or closes the aperture: a depth of field focused on whatever is
//! in the middle of the view. It runs on real time, so it flies while the game is frozen
//! (src/freeze.rs). F12 again goes back to the player.

use std::collections::HashMap;

use bevy::camera::visibility::VisibilitySystems;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::desktop::{MainCamera, Overlays, set_grab};
use crate::game::{Game, PredictSet};
use crate::neonui;
use crate::settings::{Action, Inputs, Settings, WheelState};

#[derive(Resource)]
pub struct FreeCam {
    pub on: bool,
    /// A toggle asked for other than by F12 (testing).
    pub request: bool,
    yaw: f32,
    pitch: f32,
    vel: Vec3,
    /// The field of view (rad).
    fov: f32,
    /// The aperture: an index into `STOPS` (0: no depth of field).
    stop: usize,
    /// The distance in focus (m), following what's in the middle of the view.
    focus: f32,
    /// Part of a step of the wheel (touchpads roll in pixels).
    wheel: f32,
    /// How long the readout stays up (s), and whether it shows the keys too.
    note: f32,
    help: bool,
}

impl Default for FreeCam {
    fn default() -> FreeCam {
        FreeCam {
            on: false,
            request: false,
            yaw: 0.0,
            pitch: 0.0,
            vel: Vec3::ZERO,
            fov: 80f32.to_radians(),
            stop: 0,
            focus: 5.0,
            wheel: 0.0,
            note: 0.0,
            help: false,
        }
    }
}

impl FreeCam {
    /// Testing: the view's width (degrees) and the aperture (a step in `STOPS`).
    pub fn set_fov(&mut self, deg: f32) {
        self.fov = deg.clamp(FOV_MIN, FOV_MAX).to_radians();
    }

    pub fn set_stop(&mut self, stop: usize) {
        self.stop = stop.min(STOPS.len() - 1);
    }
}

/// The apertures Shift and the wheel step through (f-number; the first: none, all sharp).
const STOPS: [f32; 10] = [0.0, 8.0, 5.6, 4.0, 2.8, 2.0, 1.4, 1.0, 0.7, 0.5];
/// The narrowest and widest view (degrees), and how much a step of the wheel changes it.
const FOV_MIN: f32 = 5.0;
const FOV_MAX: f32 = 120.0;
const FOV_STEP: f32 = 0.92;
/// Flying speed (m/s), how quickly it gets there (time constant, s), and how quickly the
/// focus follows.
const SPEED: f32 = 5.0;
const EASE: f32 = 0.12;
const FOCUS_TIME: f32 = 0.15;
/// How long the readout shows after a change, and the keys after switching on (s).
const NOTE: f32 = 1.8;
const HELP: f32 = 5.0;

/// The readout in the corner (left up while the rest of the UI is hidden).
#[derive(Component)]
struct Readout;

/// The UI hidden while flying, and how it was.
#[derive(Resource, Default)]
struct HiddenUi(HashMap<Entity, Visibility>);

pub struct FreeCamPlugin;

impl Plugin for FreeCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FreeCam>()
            .init_resource::<HiddenUi>()
            .add_systems(
                Update,
                (
                    toggle.before(crate::game::InputSet),
                    (fly, readout).chain().after(PredictSet),
                ),
            )
            .add_systems(
                PostUpdate,
                hide_ui.before(VisibilitySystems::VisibilityPropagate),
            );
    }
}

fn toggle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    game: Option<Res<Game>>,
    overlays: Res<Overlays>,
    mut free: ResMut<FreeCam>,
    mut cams: Query<(Entity, &Transform, &Projection), With<MainCamera>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok((cam, t, proj)) = cams.single_mut() else {
        return;
    };
    let want = if game.is_none() {
        false
    } else if (keys.just_pressed(KeyCode::F12) || free.request) && !overlays.any() {
        !free.on
    } else {
        free.on
    };
    free.request = false;
    if want == free.on {
        return;
    }
    free.on = want;
    if want {
        // From where the player's eyes are, as they look.
        let (yaw, pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
        free.yaw = yaw;
        free.pitch = pitch;
        free.vel = Vec3::ZERO;
        if let Projection::Perspective(p) = proj {
            free.fov = p.fov;
        }
        free.note = HELP;
        free.help = true;
        if let Ok(mut c) = cursor.single_mut() {
            set_grab(&mut c, true);
        }
    } else {
        commands.entity(cam).remove::<DepthOfField>();
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    mut commands: Commands,
    mut free: ResMut<FreeCam>,
    game: Option<Res<Game>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    settings: Res<Settings>,
    overlays: Res<Overlays>,
    real: Res<Time<Real>>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut cams: Query<(Entity, &mut Transform, &mut Projection), With<MainCamera>>,
) {
    if !free.on {
        return;
    }
    let (Some(game), Ok((cam, mut t, mut proj))) = (game, cams.single_mut()) else {
        return;
    };
    let dt = real.delta_secs().min(0.1);
    let grabbed = !overlays.any()
        && cursor
            .single()
            .is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    free.note = (free.note - dt).max(0.0);
    if free.note == 0.0 {
        free.help = false;
    }
    let input = Inputs {
        keys: &keys,
        mouse: &mouse,
        wheel: WheelState::default(),
    };
    let on = |a: Action| grabbed && settings.pressed(a, &input);
    if grabbed {
        // (Slower narrowed in, as through a long lens.)
        let s = 0.0022 * settings.sensitivity * free.fov / 80f32.to_radians();
        let dy = if settings.invert_y {
            -motion.delta.y
        } else {
            motion.delta.y
        };
        free.yaw -= motion.delta.x * s;
        free.pitch = (free.pitch - dy * s).clamp(-1.55, 1.55);
        // The wheel: the view's width, or with Shift the aperture.
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        // (Shift turns some wheels sideways.)
        let raw = if shift && scroll.delta.y == 0.0 {
            -scroll.delta.x
        } else {
            scroll.delta.y
        };
        let steps = match scroll.unit {
            MouseScrollUnit::Line => raw,
            MouseScrollUnit::Pixel => raw / 40.0,
        };
        if steps != 0.0 {
            if shift {
                free.wheel += steps;
                while free.wheel.abs() >= 1.0 {
                    let up = free.wheel > 0.0;
                    free.wheel -= free.wheel.signum();
                    free.stop = if up {
                        (free.stop + 1).min(STOPS.len() - 1)
                    } else {
                        free.stop.saturating_sub(1)
                    };
                }
            } else {
                let deg = free.fov.to_degrees() * FOV_STEP.powf(steps);
                free.fov = deg.clamp(FOV_MIN, FOV_MAX).to_radians();
            }
            free.note = free.note.max(NOTE);
        }
    }
    let view = Quat::from_rotation_y(free.yaw) * Quat::from_rotation_x(free.pitch);
    let mut want = Vec3::ZERO;
    for (a, d) in [
        (Action::Forward, Vec3::NEG_Z),
        (Action::Back, Vec3::Z),
        (Action::Left, Vec3::NEG_X),
        (Action::Right, Vec3::X),
    ] {
        if on(a) {
            want += view * d;
        }
    }
    if on(Action::Jump) {
        want += Vec3::Y;
    }
    if on(Action::Crouch) {
        want -= Vec3::Y;
    }
    let mut speed = SPEED;
    if on(Action::Dash) {
        speed *= 3.0;
    }
    if on(Action::Walk) {
        speed *= 0.25;
    }
    let want = want.normalize_or_zero() * speed;
    let vel = free.vel;
    free.vel += (want - vel) * (1.0 - (-dt / EASE).exp());
    t.translation += free.vel * dt;
    t.rotation = view;
    if let Projection::Perspective(p) = &mut *proj
        && (p.fov - free.fov).abs() > 1e-5
    {
        p.fov = free.fov;
    }
    // Depth of field: focused on what's in the middle of the view.
    if free.stop == 0 {
        commands.entity(cam).remove::<DepthOfField>();
        return;
    }
    let ahead = t.translation + view * Vec3::NEG_Z * 200.0;
    let target = game
        .world
        .ray(t.translation, ahead)
        .map_or(200.0, |h| h.1.distance(t.translation))
        .max(0.1);
    free.focus += (target - free.focus) * (1.0 - (-dt / FOCUS_TIME).exp());
    commands.entity(cam).insert(DepthOfField {
        mode: DepthOfFieldMode::Bokeh,
        focal_distance: free.focus,
        aperture_f_stops: STOPS[free.stop],
        max_depth: 300.0,
        ..default()
    });
}

/// The view's width and the aperture in a corner for a moment after changing them, and the
/// keys after switching the free camera on.
fn readout(
    mut commands: Commands,
    free: Res<FreeCam>,
    shown: Query<Entity, With<Readout>>,
    mut texts: Query<(&mut Text, &mut TextColor), With<Readout>>,
) {
    let up = free.on && free.note > 0.0;
    let Ok(e) = shown.single() else {
        if up {
            commands.spawn((
                Readout,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(24.0),
                    bottom: Val::Px(20.0),
                    ..default()
                },
                neonui::text("", 15.0, neonui::TEXT),
                TextLayout::justify(Justify::Right),
                GlobalZIndex(30),
            ));
        }
        return;
    };
    if !up {
        commands.entity(e).despawn();
        return;
    }
    let Ok((mut text, mut color)) = texts.get_mut(e) else {
        return;
    };
    let aperture = match free.stop {
        0 => "all sharp".to_string(),
        s => format!("f/{}, focus {:.1} m", STOPS[s], free.focus),
    };
    let mut s = format!("FOV {:.0}°   {aperture}", free.fov.to_degrees());
    if free.help {
        s.push_str(
            "\nFREE CAMERA   mouse: look   WASD: fly   Space / Ctrl: up / down\n\
             Shift: faster   Alt: slower   wheel: FOV   Shift + wheel: aperture   F12: back",
        );
    }
    if text.0 != s {
        text.0 = s;
    }
    // Fading out in its last half second.
    color.0 = neonui::TEXT.with_alpha((free.note / 0.5).min(1.0));
}

/// The UI's roots, but the readout.
type UiRoots = (With<Node>, Without<ChildOf>, Without<Readout>);

/// Hides every UI root but the readout while flying (unless a menu is open over the game),
/// and puts them back as they were afterwards.
fn hide_ui(
    free: Res<FreeCam>,
    overlays: Res<Overlays>,
    mut hidden: ResMut<HiddenUi>,
    mut roots: Query<(Entity, &mut Visibility), UiRoots>,
) {
    if free.on && !overlays.any() {
        for (e, mut v) in &mut roots {
            hidden.0.entry(e).or_insert(*v);
            if *v != Visibility::Hidden {
                *v = Visibility::Hidden;
            }
        }
    } else if !hidden.0.is_empty() {
        for (e, was) in hidden.0.drain() {
            if let Ok((_, mut v)) = roots.get_mut(e) {
                *v = was;
            }
        }
    }
}

//! Seven-segment displays made of accent strips: they glow like the arena and draw in every
//! renderer (the GL headset renderer has no UI). At each end of the arena, on the rear wall
//! above the base, a board shows the scores (Red's left, Blue's right) and a smaller board
//! above it the time left; VR players also get one on the wrist (src/xr.rs).

use bevy::prelude::*;
use lt_sim::level::Level;

use crate::game::Game;
use crate::materials::{AccentMaterial, Palette};

/// Segments a..g lit for each digit.
const SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];

#[derive(Component)]
pub struct Scoreboard;

/// One digit: its seven segment entities.
#[derive(Clone, Copy)]
pub struct Digit(pub [Entity; 7]);

#[derive(Resource, Default)]
struct Boards {
    /// Per board: red score (5), minutes (2), seconds (2), blue score (5).
    digits: Vec<Vec<Digit>>,
}

/// Spawns `n` digits of height `h` from left to right, as children of `parent`.
pub fn spawn_digits(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
    start: Vec3,
    n: usize,
    h: f32,
    material: &Handle<AccentMaterial>,
) -> Vec<Digit> {
    let w = h * 0.55;
    let t = h * 0.09;
    let horiz = meshes.add(Cuboid::new(w - t, t, t * 0.5));
    let vert = meshes.add(Cuboid::new(t, h / 2.0 - t, t * 0.5));
    let mut out = Vec::new();
    for i in 0..n {
        let o = start + Vec3::X * i as f32 * (w + t * 2.5);
        // a b c d e f g
        let parts: [(Vec3, &Handle<Mesh>); 7] = [
            (Vec3::new(0.0, h / 2.0, 0.0), &horiz),
            (Vec3::new(w / 2.0, h / 4.0, 0.0), &vert),
            (Vec3::new(w / 2.0, -h / 4.0, 0.0), &vert),
            (Vec3::new(0.0, -h / 2.0, 0.0), &horiz),
            (Vec3::new(-w / 2.0, -h / 4.0, 0.0), &vert),
            (Vec3::new(-w / 2.0, h / 4.0, 0.0), &vert),
            (Vec3::new(0.0, 0.0, 0.0), &horiz),
        ];
        let segs = parts.map(|(p, m)| {
            commands
                .spawn((Mesh3d(m.clone()), MeshMaterial3d(material.clone()), Transform::from_translation(o + p), ChildOf(parent)))
                .id()
        });
        out.push(Digit(segs));
    }
    out
}

/// Shows `text` (digits, ' ' blank, '-' the middle bar) on `digits`.
pub fn show(text: &str, digits: &[Digit], vis: &mut Query<&mut Visibility>) {
    let chars: Vec<char> = text.chars().collect();
    for (i, d) in digits.iter().enumerate() {
        let mask = match chars.get(i) {
            Some(c) if c.is_ascii_digit() => SEGMENTS[c.to_digit(10).unwrap() as usize],
            Some('-') => 0x40,
            _ => 0,
        };
        for (k, e) in d.0.iter().enumerate() {
            if let Ok(mut v) = vis.get_mut(*e) {
                let on = mask & (1 << k) != 0;
                let want = if on { Visibility::Inherited } else { Visibility::Hidden };
                if *v != want {
                    *v = want;
                }
            }
        }
    }
}

/// The score board: the scores, 1 m digits, centred this high at the top of the rear walls (the
/// border is 6 m high); the clock
/// on a board of its own above it.
const SCORE_Y: f32 = 5.9;
const SCORE_H: f32 = 1.0;
const CLOCK_Y: f32 = 7.45;
const CLOCK_H: f32 = 0.65;
/// The boards stand this far out from the wall, their panels this thick behind the digits.
const OUT: f32 = 0.16;
const PANEL_T: f32 = 0.1;
/// Margin of a panel around its digits, and the width of its neon frame.
const MARGIN: f32 = 0.25;
const FRAME: f32 = 0.06;

/// The distance between digits' centres for digits of height `h` (as `spawn_digits`).
fn pitch(h: f32) -> f32 {
    h * 0.55 + h * 0.09 * 2.5
}

/// A board's panel, centred on `centre` (`size` its width and height), behind its digits: a dark
/// body and a neon frame, left half in `left`'s colour and right half in `right`'s.
#[allow(clippy::too_many_arguments)]
fn spawn_panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    root: Entity,
    centre: Vec2,
    size: Vec2,
    palette: &Palette,
    left: &Handle<AccentMaterial>,
    right: &Handle<AccentMaterial>,
) {
    let z = -PANEL_T / 2.0 - 0.02;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(size.x, size.y, PANEL_T))),
        MeshMaterial3d(palette.body.clone()),
        Transform::from_translation(centre.extend(z)),
        ChildOf(root),
    ));
    let fz = -0.02 + FRAME / 2.0;
    let half = meshes.add(Cuboid::new(size.x / 2.0, FRAME, FRAME));
    let side = meshes.add(Cuboid::new(FRAME, size.y, FRAME));
    for (sx, mat) in [(-1.0f32, left), (1.0, right)] {
        for sy in [-1.0f32, 1.0] {
            commands.spawn((
                Mesh3d(half.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_xyz(centre.x + sx * size.x / 4.0, centre.y + sy * (size.y - FRAME) / 2.0, fz),
                ChildOf(root),
            ));
        }
        commands.spawn((
            Mesh3d(side.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_xyz(centre.x + sx * (size.x - FRAME) / 2.0, centre.y, fz),
            ChildOf(root),
        ));
    }
}

pub fn spawn_scoreboards(commands: &mut Commands, level: &Level, palette: &Palette, meshes: &mut Assets<Mesh>) {
    commands.queue(|world: &mut World| {
        let mut boards = world.resource_mut::<Boards>();
        boards.digits.clear();
        let mut q = world.query_filtered::<Entity, With<Scoreboard>>();
        let old: Vec<Entity> = q.iter(world).collect();
        for e in old {
            world.entity_mut(e).despawn();
        }
    });
    let red = palette.accent[0].clone();
    let blue = palette.accent[1].clone();
    let white = palette.accent[2].clone();
    // The score board: Red's score on the left, Blue's on the right, a gap between.
    let sp = pitch(SCORE_H);
    let score_w = 4.0 * sp + SCORE_H * 0.55;
    let gap = 1.4;
    let score_size = Vec2::new(2.0 * score_w + gap + 2.0 * MARGIN, SCORE_H + 2.0 * MARGIN);
    // The clock: mm:ss, the colon in a gap of its own.
    let cp = pitch(CLOCK_H);
    let colon = CLOCK_H * 0.5;
    let clock_w = 3.0 * cp + CLOCK_H * 0.55 + colon;
    let clock_size = Vec2::new(clock_w + 2.0 * MARGIN, CLOCK_H + 2.0 * MARGIN);
    let dot = meshes.add(Cuboid::new(CLOCK_H * 0.1, CLOCK_H * 0.1, 0.05));
    let divider = meshes.add(Cuboid::new(FRAME, SCORE_H + MARGIN, FRAME));
    for side in [1.0f32, -1.0] {
        let rot = if side > 0.0 { Quat::from_rotation_y(std::f32::consts::PI) } else { Quat::IDENTITY };
        let root = commands
            .spawn((
                Scoreboard,
                crate::arena::ArenaPart,
                Transform::from_translation(Vec3::new(0.0, 0.0, side * (level.dims().half_length() - OUT)))
                    .with_rotation(rot),
                Visibility::default(),
            ))
            .id();
        spawn_panel(commands, meshes, root, Vec2::new(0.0, SCORE_Y), score_size, palette, &red, &blue);
        commands.spawn((
            Mesh3d(divider.clone()),
            MeshMaterial3d(white.clone()),
            Transform::from_xyz(0.0, SCORE_Y, -0.02 + FRAME / 2.0),
            ChildOf(root),
        ));
        spawn_panel(commands, meshes, root, Vec2::new(0.0, CLOCK_Y), clock_size, palette, &white, &white);

        let first = |centre: f32, n: usize, p: f32| centre - (n as f32 - 1.0) * p / 2.0;
        let red_x = -(gap + score_w) / 2.0;
        let blue_x = (gap + score_w) / 2.0;
        let min_x = -clock_w / 2.0 + CLOCK_H * 0.55 / 2.0;
        let sec_x = min_x + 2.0 * cp + colon;
        let mut digits = Vec::new();
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(first(red_x, 5, sp), SCORE_Y, 0.0), 5, SCORE_H, &red));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(min_x, CLOCK_Y, 0.0), 2, CLOCK_H, &white));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(sec_x, CLOCK_Y, 0.0), 2, CLOCK_H, &white));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(first(blue_x, 5, sp), SCORE_Y, 0.0), 5, SCORE_H, &blue));
        let colon_x = min_x + cp + (cp + colon) / 2.0;
        for y in [0.22, -0.22] {
            commands.spawn((
                Mesh3d(dot.clone()),
                MeshMaterial3d(white.clone()),
                Transform::from_xyz(colon_x, CLOCK_Y + y * CLOCK_H, 0.0),
                ChildOf(root),
            ));
        }
        commands.queue(move |world: &mut World| {
            world.resource_mut::<Boards>().digits.push(digits);
        });
    }
}

pub struct ScoreboardPlugin;

impl Plugin for ScoreboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Boards>().add_systems(Update, update.run_if(resource_exists::<Game>));
    }
}

fn update(game: Res<Game>, boards: Res<Boards>, mut vis: Query<&mut Visibility>) {
    let s = game.snap.time_left.max(0.0) as u32;
    let [r, b] = game.snap.scores;
    let text = format!("{:>5}{:>2}{:02}{:>5}", r.min(99999), (s / 60).min(99), s % 60, b.min(99999));
    for d in &boards.digits {
        show(&text, d, &mut vis);
    }
}

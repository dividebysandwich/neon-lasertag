//! Seven-segment displays made of accent strips: they glow like the arena and draw in every
//! renderer (the GL headset renderer has no UI). Scoreboards hang on the rear wall of each
//! base; VR players also get one on the wrist (src/xr.rs).

use bevy::prelude::*;
use lt_sim::level::{HALF_LENGTH, Level};

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

pub fn spawn_scoreboards(commands: &mut Commands, _level: &Level, palette: &Palette, meshes: &mut Assets<Mesh>) {
    commands.queue(|world: &mut World| {
        let mut boards = world.resource_mut::<Boards>();
        boards.digits.clear();
        let mut q = world.query_filtered::<Entity, With<Scoreboard>>();
        let old: Vec<Entity> = q.iter(world).collect();
        for e in old {
            world.entity_mut(e).despawn();
        }
    });
    let h = 1.0;
    let dot = meshes.add(Cuboid::new(0.1, 0.1, 0.05));
    for side in [1.0f32, -1.0] {
        let rot = if side > 0.0 { Quat::from_rotation_y(std::f32::consts::PI) } else { Quat::IDENTITY };
        let root = commands
            .spawn((
                Scoreboard,
                crate::arena::ArenaPart,
                Transform::from_translation(Vec3::new(0.0, 4.3, side * (HALF_LENGTH - 0.03))).with_rotation(rot),
                Visibility::default(),
            ))
            .id();
        let step = h * 0.55 + h * 0.09 * 2.5;
        let red = palette.accent[0].clone();
        let blue = palette.accent[1].clone();
        let white = palette.accent[2].clone();
        let mut digits = Vec::new();
        let x0 = -6.5 * step - 1.0;
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(x0, 0.0, 0.0), 5, h, &red));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(x0 + 5.0 * step + 1.0, 0.0, 0.0), 2, h, &white));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(x0 + 7.0 * step + 1.4, 0.0, 0.0), 2, h, &white));
        digits.extend(spawn_digits(commands, meshes, root, Vec3::new(x0 + 9.0 * step + 2.4, 0.0, 0.0), 5, h, &blue));
        for y in [0.22, -0.22] {
            commands.spawn((
                Mesh3d(dot.clone()),
                MeshMaterial3d(white.clone()),
                Transform::from_xyz(x0 + 6.5 * step + 1.2, y, 0.0),
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

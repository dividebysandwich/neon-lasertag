//! The start menu's scene (out of a game; VR's panel, src/vrui.rs, and the flat screen's
//! menu, src/menu.rs, show over it): an empty arena, just its floor and its rainbow border
//! walls, with the player in the middle, a circle of red and blue players standing around them
//! at ease (blasters pointed at the floor), and colourful fireworks going up all around it. The
//! arena's parts are `ArenaPart`s, so starting a game takes them down with everything else; the
//! players and the fireworks go with them.

use bevy::prelude::*;
use lt_sim::Team;
use lt_sim::hitbox::{CHEST_DROP, HEAD_CROWN, HEAD_SCALE};
use lt_sim::level::{ArenaSize, Level};
use lt_sim::sim::{EYE, eye_on_neck, yaw_of};

use crate::actors::spawn_model;
use crate::arena::{ArenaPart, Kit};
use crate::assets::Models;
use crate::game::Game;
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette};

/// Rockets go up from this far around the arena's centre (m).
const LAUNCH_RADIUS: (f32, f32) = (38.0, 75.0);
/// Seconds between rockets.
const LAUNCH_EVERY: (f32, f32) = (0.25, 0.9);
/// Sparks in a burst, their speed (m/s) and lifetime (s).
const SPARKS: usize = 70;
const SPARK_SPEED: (f32, f32) = (8.0, 15.0);
const SPARK_LIFE: (f32, f32) = (1.3, 2.3);
const GRAVITY: f32 = 9.8;
/// Sparks fall slower (air) and slow down.
const SPARK_GRAVITY: f32 = 3.5;
const DRAG: f32 = 1.1;
/// The fireworks' colours (linear, bright).
const COLORS: [Vec3; 8] = [
    Vec3::new(1.0, 0.1, 0.15),
    Vec3::new(1.0, 0.55, 0.05),
    Vec3::new(1.0, 0.95, 0.2),
    Vec3::new(0.2, 1.0, 0.3),
    Vec3::new(0.1, 0.9, 1.0),
    Vec3::new(0.2, 0.35, 1.0),
    Vec3::new(0.75, 0.2, 1.0),
    Vec3::new(1.0, 0.3, 0.8),
];

/// The players standing around the middle: how many, and how far from it (m).
const SQUAD: usize = 10;
const SQUAD_RADIUS: f32 = 3.2;

/// One of them, idly looking about: their head, the neck following it, and when they do it.
#[derive(Component)]
struct Idler {
    head: Entity,
    neck: Entity,
    phase: f32,
}

/// Where the chest's top is in a squad player's frame.
const CHEST: Vec3 = Vec3::new(0.0, EYE - CHEST_DROP, 0.0);

#[derive(Component)]
struct Rocket {
    vel: Vec3,
    fuse: f32,
    colors: [usize; 2],
}

#[derive(Component)]
struct Spark {
    vel: Vec3,
    age: f32,
    life: f32,
    size: f32,
}

/// A burst's flash: a halo growing and fading.
#[derive(Component)]
struct Flash {
    age: f32,
    mat: Handle<GlowMaterial>,
}

#[derive(Resource)]
struct Fireworks {
    sparks: Vec<Handle<AccentMaterial>>,
    rocket: Handle<AccentMaterial>,
    ball: Handle<Mesh>,
    halo: Handle<Mesh>,
    next: f32,
    rng: u64,
}

impl Fireworks {
    /// 0..1.
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

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                make.run_if(not(resource_exists::<Fireworks>)),
                (show, fireworks, idle)
                    .chain()
                    .run_if(resource_exists::<Fireworks>),
            ),
        );
    }
}

fn make(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    let sparks = COLORS
        .iter()
        .map(|&c| {
            accents.add(AccentMaterial {
                overlay: false,
                neon: NeonUniform::fixed(c, 9.0),
            })
        })
        .collect();
    let rocket = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(Vec3::new(1.0, 0.8, 0.5), 6.0),
    });
    // A disc of halo (uv.y 0 in the middle, 1 at the rim) for the flashes.
    let mut halo = crate::assets::MeshData::default();
    let n = 24;
    halo.pos.push([0.0, 0.0, 0.0]);
    halo.nrm.push([0.0, 0.0, 1.0]);
    halo.uv.push([0.0, 0.0]);
    for k in 0..n {
        let a = k as f32 / n as f32 * std::f32::consts::TAU;
        halo.pos.push([a.cos(), a.sin(), 0.0]);
        halo.nrm.push([0.0, 0.0, 1.0]);
        halo.uv.push([0.0, 1.0]);
        halo.idx.extend([0, 1 + k, 1 + (k + 1) % n]);
    }
    commands.insert_resource(Fireworks {
        sparks,
        rocket,
        ball: meshes.add(Sphere::new(1.0).mesh().ico(1).unwrap()),
        halo: meshes.add(halo.to_mesh()),
        next: 1.0,
        rng: 0x9E37_79B9_7F4A_7C15 ^ rand::random::<u64>(),
    });
}

/// Builds the empty arena when there is no game (and none starting), takes the fireworks down
/// when one starts.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn show(
    mut commands: Commands,
    game: Option<Res<Game>>,
    starting: Option<Res<crate::menu::Start>>,
    connecting: Option<Res<crate::Connecting>>,
    kit: Option<Res<Kit>>,
    palette: Option<Res<Palette>>,
    models: Option<Res<Models>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shown: Local<bool>,
    parts: Query<Entity, Or<(With<ArenaPart>, With<Rocket>, With<Spark>, With<Flash>)>>,
) {
    let lobby = game.is_none() && starting.is_none() && connecting.is_none();
    if lobby && !*shown {
        let (Some(kit), Some(palette)) = (kit, palette) else {
            return;
        };
        // Whatever is left of a game first.
        for e in &parts {
            commands.entity(e).try_despawn();
        }
        let level = Level::empty(rand::random::<u32>() as u64, ArenaSize::Small);
        crate::arena::spawn_arena(&mut commands, &level, &kit, &palette, &mut meshes);
        if let Some(models) = models {
            spawn_squad(&mut commands, &models, &palette);
        }
        *shown = true;
    } else if !lobby && *shown {
        // (The game's own arena is built once it exists: this goes first.)
        if game.is_none() {
            for e in &parts {
                commands.entity(e).try_despawn();
            }
        }
        *shown = false;
    }
}

/// Launches rockets, bursts them into sparks and moves everything (while the scene is up).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fireworks(
    mut commands: Commands,
    game: Option<Res<Game>>,
    time: Res<Time>,
    mut fw: ResMut<Fireworks>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    mut rockets: Query<(Entity, &mut Rocket, &mut Transform), (Without<Spark>, Without<Flash>)>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform), (Without<Rocket>, Without<Flash>)>,
    mut flashes: Query<(Entity, &mut Flash, &mut Transform), (Without<Rocket>, Without<Spark>)>,
) {
    if game.is_some() {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    fw.next -= dt;
    if fw.next <= 0.0 {
        fw.next = fw.range(LAUNCH_EVERY);
        let a = fw.rand() * std::f32::consts::TAU;
        let r = fw.range(LAUNCH_RADIUS);
        let at = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let vel = Vec3::new(
            fw.range((-3.0, 3.0)),
            fw.range((26.0, 36.0)),
            fw.range((-3.0, 3.0)),
        );
        let c = (fw.rand() * COLORS.len() as f32) as usize % COLORS.len();
        // Now and then a burst of two colours.
        let c2 = if fw.rand() < 0.3 {
            (c + 1 + (fw.rand() * 3.0) as usize) % COLORS.len()
        } else {
            c
        };
        let fuse = fw.range((1.6, 2.6));
        commands.spawn((
            Rocket {
                vel,
                fuse,
                colors: [c, c2],
            },
            Mesh3d(fw.ball.clone()),
            MeshMaterial3d(fw.rocket.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(0.22)),
            ArenaPart,
        ));
    }
    for (e, mut r, mut t) in &mut rockets {
        r.vel.y -= GRAVITY * dt;
        t.translation += r.vel * dt;
        r.fuse -= dt;
        if r.fuse > 0.0 {
            continue;
        }
        commands.entity(e).despawn();
        let at = t.translation;
        let size = fw.range((0.2, 0.32));
        for k in 0..SPARKS {
            // Evenly over a sphere (a spiral), each a little off.
            let y = 1.0 - 2.0 * (k as f32 + 0.5) / SPARKS as f32;
            let ring = (1.0 - y * y).sqrt();
            let phi = k as f32 * 2.399_963 + fw.rand() * 0.3;
            let dir = Vec3::new(ring * phi.cos(), y, ring * phi.sin());
            let vel = dir * fw.range(SPARK_SPEED);
            let color = r.colors[k % 2];
            commands.spawn((
                Spark {
                    vel,
                    age: 0.0,
                    life: fw.range(SPARK_LIFE),
                    size,
                },
                Mesh3d(fw.ball.clone()),
                MeshMaterial3d(fw.sparks[color].clone()),
                Transform::from_translation(at).with_scale(Vec3::splat(size)),
                ArenaPart,
            ));
        }
        let mat = glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::fixed(COLORS[r.colors[0]], 2.5),
        });
        commands.spawn((
            Flash {
                age: 0.0,
                mat: mat.clone(),
            },
            Mesh3d(fw.halo.clone()),
            MeshMaterial3d(mat),
            // Facing the centre, where the player stands.
            Transform::from_translation(at).looking_at(Vec3::new(0.0, at.y, 0.0), Vec3::Y),
            ArenaPart,
        ));
    }
    for (e, mut s, mut t) in &mut sparks {
        s.age += dt;
        if s.age >= s.life {
            commands.entity(e).despawn();
            continue;
        }
        s.vel *= (-DRAG * dt).exp();
        s.vel.y -= SPARK_GRAVITY * dt;
        t.translation += s.vel * dt;
        let k = 1.0 - s.age / s.life;
        // Shrinking, and twinkling towards the end.
        let twinkle = if k < 0.35 && (s.age * 23.0 + t.translation.x).sin() > 0.3 {
            0.3
        } else {
            1.0
        };
        t.scale = Vec3::splat(s.size * k.powf(0.6) * twinkle);
    }
    for (e, mut f, mut t) in &mut flashes {
        f.age += dt;
        let life = 0.6;
        if f.age >= life {
            commands.entity(e).despawn();
            glows.remove(&f.mat);
            continue;
        }
        let k = f.age / life;
        t.scale = Vec3::splat(4.0 + 10.0 * k);
        if let Some(mut m) = glows.get_mut(&f.mat) {
            m.neon.color.w = 2.5 * (1.0 - k) * (1.0 - k);
        }
    }
}

/// The animal (of `heads` in `Models::heads`) of the squad's `i`th player.
fn animal_of(i: usize, heads: usize) -> usize {
    (i * 7 + 3) % heads.max(1)
}

/// The players in a circle around the middle, facing it, red on the red half (+z), blue on
/// the blue one: at ease, the blaster hanging in the right hand pointed at the floor, the left
/// hand at the side.
fn spawn_squad(commands: &mut Commands, models: &Models, palette: &Palette) {
    for i in 0..SQUAD {
        let a = (i as f32 + 0.5) / SQUAD as f32 * std::f32::consts::TAU;
        let feet = Vec3::new(a.sin(), 0.0, a.cos()) * SQUAD_RADIUS;
        let team = Some(if feet.z > 0.0 { Team::Red } else { Team::Blue });
        let root = commands
            .spawn((
                Transform::from_translation(feet)
                    .with_rotation(Quat::from_rotation_y(yaw_of(-feet))),
                Visibility::default(),
                ArenaPart,
            ))
            .id();
        let part = |commands: &mut Commands, t: Transform| {
            commands
                .spawn((t, Visibility::default(), ChildOf(root)))
                .id()
        };
        let body = part(commands, Transform::from_translation(CHEST));
        spawn_model(commands, body, &models.torso, palette, team);
        let head = part(commands, Transform::from_xyz(0.0, EYE, 0.0));
        if !models.heads.is_empty() {
            // (Grown about the skull top, as in a game: src/actors.rs.)
            let grown = commands
                .spawn((
                    Transform::from_xyz(0.0, HEAD_CROWN * (1.0 - HEAD_SCALE), 0.0)
                        .with_scale(Vec3::splat(HEAD_SCALE)),
                    Visibility::default(),
                    ChildOf(head),
                ))
                .id();
            let animal = &models.heads[animal_of(i, models.heads.len())];
            spawn_model(commands, grown, &animal.model, palette, team);
            crate::eyes::spawn_eyes(
                commands, grown, animal, models, palette, team, i as u64, None,
            );
        }
        // The left hand hanging loose, fingers down.
        let gloves = [
            Transform::from_xyz(-0.25, 0.92, 0.03)
                .with_rotation(Quat::from_rotation_z(-0.15) * Quat::from_rotation_x(-1.35)),
            // The blaster held low before the hip, its muzzle to the floor a little ahead,
            // turned in a little across the body.
            Transform::from_xyz(0.2, 0.98, -0.16)
                .with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_x(-1.1)),
        ];
        let left = part(commands, gloves[0]);
        spawn_model(commands, left, &models.hands[0], palette, team);
        let right = part(commands, gloves[1]);
        spawn_model(commands, right, &models.hands[1], palette, team);
        spawn_model(commands, right, &models.blaster, palette, team);
        // The body between, posed as in a game (src/rig.rs): arms reaching the gloves, and legs
        // standing (the gait settled a moment on the spot).
        let b = &models.body;
        let segment = |commands: &mut Commands, model, t: Transform| {
            let e = part(commands, t);
            spawn_model(commands, e, model, palette, team);
            e
        };
        let neck = segment(commands, &b.neck, Transform::IDENTITY);
        let chest = Transform::from_translation(CHEST);
        for (glove, side) in gloves.iter().zip([-1.0, 1.0]) {
            let [upper, fore] = crate::rig::arm(&chest, side, glove);
            segment(commands, &b.upper_arm, upper);
            segment(commands, &b.forearm, fore);
        }
        let mut gait = crate::rig::Gait::default();
        let mut legs = crate::rig::Legs::default();
        for _ in 0..60 {
            legs = gait.update(1.0 / 60.0, Vec3::ZERO, CHEST, 0.0);
        }
        segment(commands, &b.pelvis, legs.pelvis);
        for i in 0..2 {
            segment(commands, &b.thigh, legs.thigh[i]);
            segment(commands, &b.shin, legs.shin[i]);
            segment(commands, &b.boot, legs.boot[i]);
        }
        // The tail at rest.
        if let Some(tail) = models
            .heads
            .get(animal_of(i, models.heads.len()))
            .and_then(|h| h.tail.as_ref())
        {
            for (piece, joint) in tail.pieces.iter().zip(tail.rest()) {
                segment(commands, piece, legs.pelvis * joint);
            }
        }
        commands.entity(root).insert(Idler {
            head,
            neck,
            phase: i as f32 * 2.3,
        });
    }
}

/// The players in the circle look about slowly, each in their own time.
fn idle(time: Res<Time>, idlers: Query<&Idler>, mut transforms: Query<&mut Transform>) {
    let t = time.elapsed_secs();
    for i in &idlers {
        if let Ok(mut h) = transforms.get_mut(i.head) {
            let yaw = 0.45 * (t * 0.31 + i.phase).sin() * (t * 0.13 + i.phase * 0.7).sin();
            let pitch = -0.1 + 0.08 * (t * 0.23 + i.phase * 1.9).sin();
            h.rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
            h.translation = eye_on_neck(Vec3::Y * EYE, h.rotation);
            let neck = crate::rig::neck(h.translation, h.rotation);
            if let Ok(mut n) = transforms.get_mut(i.neck) {
                *n = neck;
            }
        }
    }
}

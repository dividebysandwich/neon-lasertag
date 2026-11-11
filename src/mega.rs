//! The Megatarget mode's pieces (lt_sim::level::Mega), built from the neon materials so every
//! renderer draws them:
//!
//! - the megatarget: a big faceted core hanging from the room's ceiling in two turning rings,
//!   a shaft of light from it straight down to a glowing ring on the floor, and chevrons
//!   climbing the room's inside walls towards the ceiling;
//! - the secondary targets: a core in a double ring on the pocket side of their wall, orange
//!   until the local player has hit it (then green);
//! - the turrets: a housing on the wall, a gun turning to where the snapshot says it aims with
//!   a red eye (dark while switched off), and above it the deactivation target, a yellow
//!   bullseye (dark while off);
//! - the forcefields: translucent sheets (the neon shader's forcefield halo: hexagons, rolling
//!   bands, sparks) between glowing emitter posts, humming; cyan, green while the local player
//!   may pass, flashing white when someone walks into one.

use std::f32::consts::{PI, TAU};

use bevy::audio::{PlaybackMode, SpatialAudioSink, Volume};
use bevy::prelude::*;
use lt_sim::SimEvent;
use lt_sim::kit::{STOREY, WALL_T};
use lt_sim::level::{CORE_HALF, Level, Mega};
use lt_sim::sim::look_rot;

use crate::arena::ArenaPart;
use crate::assets::MeshData;
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette, ZONE_COLORS};
use crate::scenery::cone;
use crate::settings::Settings;

/// The megatarget's colour (magenta), the secondary targets' (orange, green once hit), the
/// forcefields' (cyan, green while open to the local player) and the switches' (yellow).
const CORE: Vec3 = Vec3::new(1.0, 0.1, 0.85);
const TO_HIT: Vec3 = ZONE_COLORS[4];
const DONE: Vec3 = ZONE_COLORS[2];
const FIELD: Vec3 = Vec3::new(0.1, 0.75, 1.0);
const SWITCH: Vec3 = ZONE_COLORS[1];
const DARK: Vec3 = Vec3::new(0.05, 0.05, 0.06);

pub struct MegaPlugin;

impl Plugin for MegaPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (on_events, animate, animate_relays, animate_lamps)
                .chain()
                .run_if(resource_exists::<Game>),
        );
    }
}

#[derive(Component)]
struct CoreView {
    core: Entity,
    rings: [Entity; 2],
    shaft: Handle<GlowMaterial>,
    floor: Handle<GlowMaterial>,
    since_hit: f32,
}

#[derive(Component)]
struct SecondaryView {
    side: u8,
    core: Entity,
    rings: [Entity; 2],
    mat: Handle<AccentMaterial>,
    since_hit: f32,
}

#[derive(Component)]
struct TurretView {
    k: usize,
    gun: Entity,
    eye: Handle<AccentMaterial>,
    sight: Handle<AccentMaterial>,
    switch: Handle<AccentMaterial>,
    /// Under the deactivation target: a bar draining with the time the turret stays off.
    timer: Entity,
    since_fired: f32,
}

/// The off timer's bar's full width (m).
const TIMER_W: f32 = 0.56;

#[derive(Component)]
struct ShieldView {
    k: usize,
    sheet: Handle<GlowMaterial>,
    posts: Handle<AccentMaterial>,
    flash: f32,
}

/// A chevron climbing a wall of the room: from, to, phase.
#[derive(Component)]
struct Climber(Vec3, Vec3, f32);

fn accent(accents: &mut Assets<AccentMaterial>, c: Vec3, i: f32) -> Handle<AccentMaterial> {
    accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(c, i),
    })
}

/// Spawns the Megatarget mode's pieces of `level` (as parts of the arena), if it has them.
pub fn spawn(
    commands: &mut Commands,
    level: &Level,
    secondary: [lt_sim::rules::SecondaryKind; 2],
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
    assets: &AssetServer,
) {
    let Some(m) = &level.mega else { return };
    spawn_core(commands, m, palette, meshes, accents, glows);
    spawn_secondaries(commands, m, secondary, palette, meshes, accents);
    spawn_relays(commands, m, secondary, palette, meshes, accents);
    spawn_lamps(commands, m, secondary, palette, meshes, accents);
    spawn_turrets(commands, m, palette, meshes, accents);
    spawn_shields(commands, m, meshes, accents, glows, assets);
}

fn spawn_core(
    commands: &mut Commands,
    m: &Mega,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
) {
    let c = m.core.centre();
    let ceiling = m.room.1.y;
    let bright = accent(accents, CORE, 16.0);
    let trim = accent(accents, CORE, 6.0);
    let root = commands
        .spawn((
            ArenaPart,
            Transform::from_translation(c),
            Visibility::default(),
            Name::new("megatarget"),
        ))
        .id();
    // The mount on the ceiling and a short stem.
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.55, 0.06))),
        MeshMaterial3d(palette.suit.clone()),
        Transform::from_xyz(0.0, ceiling - c.y - 0.03, 0.0),
        ChildOf(root),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Torus::new(0.5, 0.56))),
        MeshMaterial3d(trim.clone()),
        Transform::from_xyz(0.0, ceiling - c.y - 0.07, 0.0),
        ChildOf(root),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(0.08, 0.3))),
        MeshMaterial3d(palette.suit.clone()),
        Transform::from_xyz(0.0, CORE_HALF + 0.05, 0.0),
        ChildOf(root),
    ));
    // A dark faceted shell around the bright core (turning), two rings around them.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(CORE_HALF * 1.15)))),
        MeshMaterial3d(palette.body.clone()),
        Transform::from_rotation(Quat::from_rotation_y(0.785) * Quat::from_rotation_x(0.615)),
        ChildOf(root),
    ));
    let core = commands
        .spawn((
            Mesh3d(meshes.add(Sphere::new(CORE_HALF * 0.78).mesh().ico(2).unwrap())),
            MeshMaterial3d(bright.clone()),
            Transform::IDENTITY,
            ChildOf(root),
        ))
        .id();
    let ring = |r: f32| Torus::new(r - 0.035, r + 0.035);
    let rings = [CORE_HALF * 1.25, CORE_HALF * 1.6].map(|r| {
        commands
            .spawn((
                Mesh3d(meshes.add(ring(r))),
                MeshMaterial3d(trim.clone()),
                Transform::IDENTITY,
                ChildOf(root),
            ))
            .id()
    });

    // The shaft of light down to the floor, and a ring of light where it lands.
    let bottom = c.y - CORE_HALF;
    let mut shaft = MeshData::default();
    cone(
        &mut shaft,
        Vec3::new(c.x, bottom, c.z),
        Vec3::NEG_Y,
        bottom,
        0.4,
        1.1,
        24,
    );
    cone(
        &mut shaft,
        Vec3::new(c.x, bottom, c.z),
        Vec3::NEG_Y,
        bottom,
        0.15,
        0.45,
        16,
    );
    let shaft_mat = glows.add(GlowMaterial {
        overlay: false,
        neon: NeonUniform::fixed(CORE, 0.9).volume(),
    });
    commands.spawn((
        ArenaPart,
        Mesh3d(meshes.add(shaft.to_mesh())),
        MeshMaterial3d(shaft_mat.clone()),
        Transform::IDENTITY,
    ));
    let mut floor = MeshData::default();
    disc(&mut floor, Vec3::new(c.x, 0.03, c.z), 0.3, 1.6, 32);
    let floor_mat = glows.add(GlowMaterial {
        overlay: false,
        neon: NeonUniform::fixed(CORE, 1.6),
    });
    commands.spawn((
        ArenaPart,
        Mesh3d(meshes.add(floor.to_mesh())),
        MeshMaterial3d(floor_mat.clone()),
        Transform::IDENTITY,
    ));
    commands.spawn((
        ArenaPart,
        Mesh3d(meshes.add(Torus::new(1.08, 1.12))),
        MeshMaterial3d(trim.clone()),
        Transform::from_xyz(c.x, 0.02, c.z),
    ));
    commands.entity(root).insert(CoreView {
        core,
        rings,
        shaft: shaft_mat,
        floor: floor_mat,
        since_hit: 99.0,
    });

    // Chevrons climbing the inside walls: columns between the doorways, staggered.
    let chevron = meshes.add(crate::targets::chevron());
    let climb = accent(accents, CORE, 6.0);
    let inset = 0.06;
    // Columns every 2 m round the inside of the walls, but not in a doorway.
    let in_doorway = |p: Vec3| {
        m.shields.iter().any(|s| {
            let c = (s.min + s.max) / 2.0;
            Vec2::new(p.x - c.x, p.z - c.z).length() < (s.max.x - s.min.x) / 2.0 + 0.5
        })
    };
    let n = m.inner.len();
    let edge = |i: usize| (m.inner[i], m.inner[(i + 1) % n]);
    let perimeter: f32 = (0..n).map(|i| edge(i).0.distance(edge(i).1)).sum();
    let mut columns: Vec<Vec3> = Vec::new();
    let mut along = 1.0;
    let mut start = 0.0;
    for i in 0..n {
        let (a, b) = edge(i);
        let len = a.distance(b);
        while along < start + len && along < perimeter {
            let e = (b - a) / len;
            let q = a + e * (along - start) + Vec2::new(-e.y, e.x) * inset;
            let p = Vec3::new(q.x, 0.0, q.y);
            if !in_doorway(p) {
                columns.push(p);
            }
            along += 2.0;
        }
        start += len;
    }
    let top = ceiling - 0.4;
    for (i, p) in columns.into_iter().enumerate() {
        for k in 0..4 {
            commands.spawn((
                ArenaPart,
                Mesh3d(chevron.clone()),
                MeshMaterial3d(climb.clone()),
                Transform::from_translation(p),
                Climber(
                    p + Vec3::Y * 0.4,
                    Vec3::new(p.x, top, p.z),
                    k as f32 / 4.0 + i as f32 * 0.13,
                ),
            ));
        }
    }
}

/// A flat ring on the floor: radius r0 (halo v 0) to r1 (v 1), facing up.
fn disc(m: &mut MeshData, c: Vec3, r0: f32, r1: f32, seg: usize) {
    let base = m.pos.len() as u32;
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * TAU;
        let d = Vec3::new(a.cos(), 0.0, a.sin());
        for (r, v) in [(r0, 0.0), (r1, 1.0)] {
            m.pos.push((c + d * r).to_array());
            m.nrm.push([0.0, 1.0, 0.0]);
            m.uv.push([i as f32 / seg as f32, v]);
        }
    }
    for i in 0..seg as u32 {
        let (a, b, c, d) = (
            base + i * 2,
            base + i * 2 + 1,
            base + i * 2 + 3,
            base + i * 2 + 2,
        );
        m.idx.extend([a, c, b, a, d, c]);
    }
    // The middle, as bright as the inner edge.
    let mid = m.pos.len() as u32;
    m.pos.push(c.to_array());
    m.nrm.push([0.0, 1.0, 0.0]);
    m.uv.push([0.0, 0.0]);
    for i in 0..seg as u32 {
        m.idx.extend([mid, base + (i + 1) * 2, base + i * 2]);
    }
}

/// A rotation turning +Z to `n` (level).
/// The relay puzzles' beam (and its stub off a relay turned wrong).
const BEAM: Vec3 = Vec3::new(0.2, 1.0, 0.95);

/// A secondary target that's a relay puzzle: its relays (turned as the snapshot has them),
/// the beam's pieces from the emitter on (shown as far as it gets), and the receiver.
#[derive(Component)]
struct RelayView {
    side: usize,
    relays: [Entity; lt_sim::gadgets::RELAYS],
    /// Each relay's shown quarter turns off (eased), and its flash after a shot.
    shown: [f32; lt_sim::gadgets::RELAYS],
    flash: [f32; lt_sim::gadgets::RELAYS],
    frames: [Handle<AccentMaterial>; lt_sim::gadgets::RELAYS],
    beams: Vec<Entity>,
    stub: Entity,
    receiver: Handle<AccentMaterial>,
}

fn spawn_relays(
    commands: &mut Commands,
    m: &Mega,
    secondary: [lt_sim::rules::SecondaryKind; 2],
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    use lt_sim::gadgets::{PIECE_R, RELAYS};
    let plate = meshes.add(Cuboid::new(0.58, 0.58, 0.05));
    let cage = meshes.add(Torus::new(PIECE_R - 0.03, PIECE_R));
    let hub = meshes.add(Cylinder::new(0.035, 0.6));
    let beam = meshes.add(Cylinder::new(0.035, 1.0));
    let emitter = meshes.add(Cuboid::new(0.22, 0.22, 0.12));
    let ring = meshes.add(Torus::new(0.14, 0.2));
    let beam_mat = accent(accents, BEAM, 6.0);
    for side in 0..2 {
        if secondary[side] != lt_sim::rules::SecondaryKind::Relay {
            continue;
        }
        let route = m.relay_route[side];
        let root = commands
            .spawn((
                ArenaPart,
                Transform::IDENTITY,
                Visibility::default(),
                Name::new("relays"),
            ))
            .id();
        let part = |commands: &mut Commands, mesh: &Handle<Mesh>, mat, t: Transform| {
            commands
                .spawn((Mesh3d(mesh.clone()), mat, t, ChildOf(root)))
                .id()
        };
        // The emitter, on the end wall, facing along the beam.
        let out = (route[1] - route[0]).normalize();
        part(
            commands,
            &emitter,
            MeshMaterial3d(accent(accents, BEAM, 4.0)),
            Transform::from_translation(route[0])
                .with_rotation(Quat::from_rotation_arc(Vec3::Z, out)),
        );
        // The receiver over the target.
        let receiver = accent(accents, TO_HIT, 3.0);
        let into = (route[RELAYS + 1] - route[RELAYS]).normalize();
        part(
            commands,
            &ring,
            MeshMaterial3d(receiver.clone()),
            Transform::from_translation(route[RELAYS + 1])
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, into)),
        );
        let mut relays = [Entity::PLACEHOLDER; RELAYS];
        let frames: [Handle<AccentMaterial>; RELAYS] =
            std::array::from_fn(|_| accent(accents, TO_HIT, 4.0));
        for i in 0..RELAYS {
            let r = commands
                .spawn((
                    Transform::from_translation(route[i + 1]),
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            // A mirror plate (its face along local +z), in a ring, on a short hanger.
            commands.spawn((
                Mesh3d(plate.clone()),
                MeshMaterial3d(palette.suit.clone()),
                Transform::IDENTITY,
                ChildOf(r),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.5, 0.5, 0.02))),
                MeshMaterial3d(accent(accents, Vec3::new(0.8, 0.95, 1.0), 4.0)),
                Transform::from_xyz(0.0, 0.0, 0.03),
                ChildOf(r),
            ));
            commands.spawn((
                Mesh3d(cage.clone()),
                MeshMaterial3d(frames[i].clone()),
                Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ChildOf(r),
            ));
            commands.spawn((
                Mesh3d(hub.clone()),
                MeshMaterial3d(palette.suit.clone()),
                Transform::from_xyz(0.0, 0.45, 0.0),
                ChildOf(r),
            ));
            relays[i] = r;
        }
        let beams: Vec<Entity> = (0..=RELAYS)
            .map(|_| {
                part(
                    commands,
                    &beam,
                    MeshMaterial3d(beam_mat.clone()),
                    Transform::IDENTITY,
                )
            })
            .collect();
        let stub = part(
            commands,
            &beam,
            MeshMaterial3d(beam_mat.clone()),
            Transform::IDENTITY,
        );
        commands.entity(root).insert(RelayView {
            side,
            relays,
            shown: [0.0; RELAYS],
            flash: [0.0; RELAYS],
            frames,
            beams,
            stub,
            receiver,
        });
    }
}

/// The sequence puzzle's lamps' colours.
const LAMP_COLORS: [Vec3; lt_sim::gadgets::LAMPS] = [
    Vec3::new(0.1, 0.9, 1.0),
    Vec3::new(1.0, 0.2, 0.85),
    Vec3::new(1.0, 0.85, 0.1),
    Vec3::new(0.25, 1.0, 0.3),
];

/// A secondary target that's a sequence puzzle: its lamps (flashing the sequence while nobody
/// is into it) and the pips over the target counting the steps shot.
#[derive(Component)]
struct LampView {
    side: usize,
    lamps: [Handle<AccentMaterial>; lt_sim::gadgets::LAMPS],
    pips: [Handle<AccentMaterial>; lt_sim::gadgets::STEPS],
    /// The flash after a shot on each lamp (and whether it was the right one).
    flash: [(f32, bool); lt_sim::gadgets::LAMPS],
}

fn spawn_lamps(
    commands: &mut Commands,
    m: &Mega,
    secondary: [lt_sim::rules::SecondaryKind; 2],
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    use lt_sim::gadgets::{LAMPS, PIECE_R, STEPS};
    let disc = meshes.add(Cylinder::new(PIECE_R * 0.8, 0.05));
    let back = meshes.add(Cylinder::new(PIECE_R + 0.04, 0.03));
    let ring = meshes.add(Torus::new(PIECE_R * 0.85, PIECE_R + 0.02));
    let pip = meshes.add(Cuboid::new(0.1, 0.1, 0.03));
    for side in 0..2 {
        if secondary[side] != lt_sim::rules::SecondaryKind::Sequence {
            continue;
        }
        let root = commands
            .spawn((
                ArenaPart,
                Transform::IDENTITY,
                Visibility::default(),
                Name::new("lamps"),
            ))
            .id();
        let lamps: [Handle<AccentMaterial>; LAMPS] = std::array::from_fn(|i| {
            let (p, n) = m.lamps[side][i];
            let mat = accent(accents, LAMP_COLORS[i], 0.6);
            // (Cylinders stand along y: turned to face out of the wall.)
            let rot = Quat::from_rotation_arc(Vec3::Y, n);
            commands.spawn((
                Mesh3d(back.clone()),
                MeshMaterial3d(palette.suit.clone()),
                Transform::from_translation(p - n * 0.04).with_rotation(rot),
                ChildOf(root),
            ));
            commands.spawn((
                Mesh3d(disc.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(p).with_rotation(rot),
                ChildOf(root),
            ));
            let frame = accent(accents, LAMP_COLORS[i], 3.0);
            commands.spawn((
                Mesh3d(ring.clone()),
                MeshMaterial3d(frame),
                Transform::from_translation(p + n * 0.01).with_rotation(rot),
                ChildOf(root),
            ));
            mat
        });
        // The pips: a row over the target.
        let t = m.secondaries[side];
        let right = Vec3::Y.cross(t.normal);
        let face = t.centre() + t.normal * 0.12 + Vec3::Y * 0.6;
        let pips: [Handle<AccentMaterial>; STEPS] = std::array::from_fn(|k| {
            let mat = accent(accents, TO_HIT, 0.5);
            commands.spawn((
                Mesh3d(pip.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(face + right * ((k as f32 - 2.0) * 0.16))
                    .with_rotation(facing(t.normal)),
                ChildOf(root),
            ));
            mat
        });
        commands.entity(root).insert(LampView {
            side,
            lamps,
            pips,
            flash: [(0.0, true); LAMPS],
        });
    }
}

fn animate_lamps(
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut views: Query<&mut LampView>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    use lt_sim::gadgets::{LAMPS, STEPS, sequence, show};
    let shots: Vec<(usize, usize, bool)> = events
        .read()
        .filter_map(|GameEvent(e)| match *e {
            SimEvent::LampShot { side, lamp, ok, .. } => Some((side as usize, lamp as usize, ok)),
            _ => None,
        })
        .collect();
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    // (The show by the sim's clock, the same on every machine.)
    let secs = game.snap.tick as f32 * lt_sim::TICK;
    for mut v in &mut views {
        let side = v.side;
        let Some(g) = game.snap.gadgets.get(side) else {
            continue;
        };
        for &(s, l, ok) in &shots {
            if s == side && l < LAMPS {
                v.flash[l] = (1.0, ok);
            }
        }
        let seq = sequence(g.seed);
        let done = g.state.first().copied().unwrap_or(0) as usize;
        let lit = if g.hold <= 0.0 && done == 0 {
            show(&seq, secs)
        } else {
            None
        };
        for i in 0..LAMPS {
            let (f, ok) = v.flash[i];
            v.flash[i].0 = (f - dt * 2.5).max(0.0);
            let mut c = LAMP_COLORS[i];
            let mut k = if lit == Some(i as u8) { 9.0 } else { 0.6 };
            if g.hold > 0.0 {
                k = 5.0 + 4.0 * (t * 12.0 + i as f32).sin();
            }
            if f > 0.0 {
                c = c.lerp(
                    if ok {
                        Vec3::ONE
                    } else {
                        Vec3::new(1.0, 0.05, 0.05)
                    },
                    f,
                );
                k += 8.0 * f;
            }
            if let Some(mut m) = accents.get_mut(&v.lamps[i]) {
                m.neon.color = c.extend(k);
            }
        }
        for k in 0..STEPS {
            if let Some(mut m) = accents.get_mut(&v.pips[k]) {
                m.neon.color = if g.hold > 0.0 || k < done {
                    DONE.extend(5.0)
                } else {
                    TO_HIT.extend(0.5)
                };
            }
        }
    }
}

/// A beam piece (a unit cylinder along y) from `a` to `b`.
fn beam_between(a: Vec3, b: Vec3) -> Transform {
    let d = b - a;
    let len = d.length().max(0.001);
    Transform::from_translation((a + b) / 2.0)
        .with_rotation(Quat::from_rotation_arc(Vec3::Y, d / len))
        .with_scale(Vec3::new(1.0, len, 1.0))
}

fn animate_relays(
    time: Res<Time>,
    game: Res<Game>,
    mut events: MessageReader<GameEvent>,
    mut views: Query<&mut RelayView>,
    mut parts: Query<(&mut Transform, &mut Visibility), Without<RelayView>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    use lt_sim::gadgets::{RELAYS, beam_reach, relay_turn};
    let flashes: Vec<(usize, usize)> = events
        .read()
        .filter_map(|GameEvent(e)| match *e {
            SimEvent::RelayTurned { side, relay, .. } => Some((side as usize, relay as usize)),
            _ => None,
        })
        .collect();
    let Some(m) = game.level.mega.as_ref() else {
        return;
    };
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    for mut v in &mut views {
        let side = v.side;
        let Some(g) = game.snap.gadgets.get(side) else {
            continue;
        };
        let route = m.relay_route[side];
        for &(s, i) in &flashes {
            if s == side && i < RELAYS {
                v.flash[i] = 1.0;
            }
        }
        for i in 0..RELAYS {
            // Turning clockwise (seen from above) a quarter at a time, eased.
            let want = g.state.get(i).copied().unwrap_or(0) as f32;
            if want < v.shown[i] - 0.5 {
                v.shown[i] -= 4.0;
            }
            v.shown[i] = (v.shown[i] + dt * 6.0).min(want);
            v.flash[i] = (v.flash[i] - dt * 3.0).max(0.0);
            let (a, b) = relay_turn(&route, i);
            let right = Quat::from_rotation_arc(Vec3::Z, (b - a).normalize_or(Vec3::Z));
            let rot = Quat::from_rotation_y(-v.shown[i] * std::f32::consts::FRAC_PI_2) * right;
            if let Ok((mut tr, _)) = parts.get_mut(v.relays[i]) {
                tr.rotation = rot;
            }
            let ok = g.state.get(i) == Some(&0);
            if let Some(mut mat) = accents.get_mut(&v.frames[i]) {
                let c = if ok { BEAM } else { TO_HIT };
                mat.neon.color = c.lerp(Vec3::ONE, v.flash[i]).extend(4.0 + 6.0 * v.flash[i]);
            }
        }
        // The beam: as far as the relays are turned right, then off the first one turned
        // wrong (a stub its wrong way).
        let reach = beam_reach(&g.state);
        let solved = g.hold > 0.0 || reach == RELAYS;
        for (j, &e) in v.beams.iter().enumerate() {
            if let Ok((mut tr, mut vis)) = parts.get_mut(e) {
                let on = solved || j <= reach;
                *vis = if on {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if on {
                    *tr = beam_between(route[j], route[j + 1]);
                }
            }
        }
        if let Ok((mut tr, mut vis)) = parts.get_mut(v.stub) {
            if solved || reach >= RELAYS {
                *vis = Visibility::Hidden;
            } else {
                let (_, out) = relay_turn(&route, reach);
                let k = g.state.get(reach).copied().unwrap_or(0) as f32;
                let wrong = Quat::from_rotation_y(-k * std::f32::consts::FRAC_PI_2) * out;
                let p = route[reach + 1];
                *tr = beam_between(p, p + wrong * 0.9);
                *vis = Visibility::Inherited;
            }
        }
        if let Some(mut mat) = accents.get_mut(&v.receiver) {
            mat.neon.color = if solved {
                BEAM.extend(12.0 + 4.0 * (t * 20.0).sin())
            } else {
                TO_HIT.extend(3.0 + (t * 3.0).sin())
            };
        }
    }
}

fn facing(n: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Z, n.normalize_or(Vec3::Z))
}

fn spawn_secondaries(
    commands: &mut Commands,
    m: &Mega,
    secondary: [lt_sim::rules::SecondaryKind; 2],
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    let plate = meshes.add(Cuboid::new(1.0, 1.0, 0.05));
    let core = meshes.add(Sphere::new(0.2).mesh().ico(1).unwrap());
    let rings = [
        meshes.add(Torus::new(0.3, 0.34)),
        meshes.add(Torus::new(0.42, 0.45)),
    ];
    for (side, t) in m.secondaries.iter().enumerate() {
        // (A puzzle there instead: src/consoles.rs.)
        if secondary[side] != lt_sim::rules::SecondaryKind::Shoot {
            continue;
        }
        let mat = accent(accents, TO_HIT, 7.0);
        let c = t.centre();
        let root = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(c).with_rotation(facing(t.normal)),
                Visibility::default(),
                Name::new("secondary target"),
            ))
            .id();
        let back = -(t.max - t.min).abs().min_element() / 2.0;
        commands.spawn((
            Mesh3d(plate.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, 0.0, back),
            ChildOf(root),
        ));
        let core_e = commands
            .spawn((
                Mesh3d(core.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_xyz(0.0, 0.0, 0.02),
                ChildOf(root),
            ))
            .id();
        let ring_e = rings.clone().map(|r| {
            commands
                .spawn((
                    Mesh3d(r),
                    MeshMaterial3d(mat.clone()),
                    // Tori lie in XZ: stand them up facing +Z.
                    Transform::from_rotation(Quat::from_rotation_x(PI / 2.0)),
                    ChildOf(root),
                ))
                .id()
        });
        commands.entity(root).insert(SecondaryView {
            side: side as u8,
            core: core_e,
            rings: ring_e,
            mat,
            since_hit: 99.0,
        });
    }
}

fn spawn_turrets(
    commands: &mut Commands,
    m: &Mega,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    let barrel = meshes.add(Cylinder::new(0.075, 0.7));
    let ball = meshes.add(Sphere::new(0.24).mesh().ico(2).unwrap());
    let eye = meshes.add(Sphere::new(0.07));
    let tip = meshes.add(Torus::new(0.07, 0.1));
    let collar = meshes.add(Torus::new(0.24, 0.28));
    let sight = meshes.add(Cylinder::new(0.008, 3.0));
    let plate = meshes.add(Cuboid::new(0.6, 0.6, 0.05));
    let bull = [
        meshes.add(Torus::new(0.2, 0.24)),
        meshes.add(Torus::new(0.09, 0.12)),
    ];
    let dot = meshes.add(Sphere::new(0.04));
    let bar = meshes.add(Cuboid::new(TIMER_W, 0.05, 0.03));
    let timer_mat = accent(accents, SWITCH, 6.0);
    for (k, t) in m.turrets.iter().enumerate() {
        let eye_mat = accent(accents, Vec3::new(1.0, 0.05, 0.05), 8.0);
        let sight_mat = accent(accents, Vec3::new(1.0, 0.05, 0.05), 2.5);
        let switch_mat = accent(accents, SWITCH, 6.0);
        // The housing (the body's box) and its glowing trim.
        let (a, b) = t.body;
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(Cuboid::from_size(b - a))),
            MeshMaterial3d(palette.body.clone()),
            Transform::from_translation((a + b) / 2.0),
        ));
        let trim_at = (a + b) / 2.0 + Vec3::Y * ((b.y - a.y) / 2.0 + 0.01);
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(Cuboid::new(
                (b.x - a.x).max(0.05),
                0.02,
                (b.z - a.z).max(0.05),
            ))),
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_translation(trim_at).with_scale(Vec3::new(1.0, 1.0, 1.0)),
        ));
        // The gun: a ball with a barrel along its -Z, turned to where it aims.
        let gun = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(t.pos).with_rotation(look_rot(t.facing)),
                Visibility::default(),
                Name::new("turret"),
            ))
            .id();
        commands.spawn((
            Mesh3d(ball.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::IDENTITY,
            ChildOf(gun),
        ));
        commands.spawn((
            Mesh3d(barrel.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, 0.0, -0.45).with_rotation(Quat::from_rotation_x(PI / 2.0)),
            ChildOf(gun),
        ));
        commands.spawn((
            Mesh3d(tip.clone()),
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_xyz(0.0, 0.0, -0.8).with_rotation(Quat::from_rotation_x(PI / 2.0)),
            ChildOf(gun),
        ));
        // A ring round the ball, facing where it aims, and a short laser sight.
        commands.spawn((
            Mesh3d(collar.clone()),
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_rotation(Quat::from_rotation_x(PI / 2.0)),
            ChildOf(gun),
        ));
        commands.spawn((
            Mesh3d(sight.clone()),
            MeshMaterial3d(sight_mat.clone()),
            Transform::from_xyz(0.0, 0.0, -2.35).with_rotation(Quat::from_rotation_x(PI / 2.0)),
            ChildOf(gun),
        ));
        commands.spawn((
            Mesh3d(eye.clone()),
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_xyz(0.0, 0.13, -0.17),
            ChildOf(gun),
        ));
        // The deactivation target: a bullseye on a plate.
        let s = t.switch;
        let sw = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(s.centre()).with_rotation(facing(s.normal)),
                Visibility::default(),
            ))
            .id();
        let depth = (s.max - s.min).abs().min_element();
        commands.spawn((
            Mesh3d(plate.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, 0.0, -depth / 2.0 + 0.03),
            ChildOf(sw),
        ));
        for r in &bull {
            commands.spawn((
                Mesh3d(r.clone()),
                MeshMaterial3d(switch_mat.clone()),
                Transform::from_xyz(0.0, 0.0, -depth / 2.0 + 0.08)
                    .with_rotation(Quat::from_rotation_x(PI / 2.0)),
                ChildOf(sw),
            ));
        }
        commands.spawn((
            Mesh3d(dot.clone()),
            MeshMaterial3d(switch_mat.clone()),
            Transform::from_xyz(0.0, 0.0, -depth / 2.0 + 0.08),
            ChildOf(sw),
        ));
        let timer = commands
            .spawn((
                Mesh3d(bar.clone()),
                MeshMaterial3d(timer_mat.clone()),
                Transform::from_xyz(0.0, -0.37, -depth / 2.0 + 0.06),
                Visibility::Hidden,
                ChildOf(sw),
            ))
            .id();
        commands.entity(gun).insert(TurretView {
            k,
            gun,
            sight: sight_mat,
            eye: eye_mat,
            switch: switch_mat,
            timer,
            since_fired: 99.0,
        });
    }
}

fn spawn_shields(
    commands: &mut Commands,
    m: &Mega,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
    assets: &AssetServer,
) {
    let hum: Handle<AudioSource> = assets.load("sounds/shield_hum.ogg");
    let post = meshes.add(Cuboid::new(0.05, STOREY - 0.1, 0.05));
    let bar = meshes.add(Cuboid::new(1.0, 0.04, 0.05));
    for (k, s) in m.shields.iter().enumerate() {
        let z = (s.min.z + s.max.z) / 2.0;
        let (x0, x1) = (s.min.x, s.max.x);
        let (y0, y1) = (0.02, STOREY - 0.04);
        // The sheet: uv 0..1 across and up.
        let mut sheet = MeshData::default();
        for (x, y, u, v) in [
            (x0, y0, 0.0, 0.0),
            (x1, y0, 1.0, 0.0),
            (x1, y1, 1.0, 1.0),
            (x0, y1, 0.0, 1.0),
        ] {
            sheet.pos.push([x, y, z]);
            sheet.nrm.push([0.0, 0.0, 1.0]);
            sheet.uv.push([u, v]);
        }
        sheet.idx.extend([0, 1, 2, 0, 2, 3]);
        let sheet_mat = glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::fixed(FIELD, 2.4).shield(),
        });
        let posts_mat = accent(accents, FIELD, 6.0);
        let e = commands
            .spawn((
                ArenaPart,
                Mesh3d(meshes.add(sheet.to_mesh())),
                MeshMaterial3d(sheet_mat.clone()),
                Transform::IDENTITY,
                Name::new("forcefield"),
            ))
            .id();
        // Emitters: glowing strips up the doorway's sides and along its top and bottom.
        for x in [x0 - 0.02, x1 + 0.02] {
            for dz in [-WALL_T / 2.0 - 0.02, WALL_T / 2.0 + 0.02] {
                commands.spawn((
                    ArenaPart,
                    Mesh3d(post.clone()),
                    MeshMaterial3d(posts_mat.clone()),
                    Transform::from_xyz(x, STOREY / 2.0, z + dz),
                ));
            }
        }
        for y in [0.03, STOREY - 0.03] {
            commands.spawn((
                ArenaPart,
                Mesh3d(bar.clone()),
                MeshMaterial3d(posts_mat.clone()),
                Transform::from_xyz((x0 + x1) / 2.0, y, z).with_scale(Vec3::new(x1 - x0, 1.0, 1.0)),
            ));
        }
        commands.entity(e).insert(ShieldView {
            k,
            sheet: sheet_mat,
            posts: posts_mat,
            flash: 0.0,
        });
        // The hum, where it is.
        commands.spawn((
            ArenaPart,
            AudioPlayer(hum.clone()),
            PlaybackSettings {
                mode: PlaybackMode::Loop,
                spatial: true,
                // (Turned up to the sound setting's once playing: `animate`.)
                volume: Volume::Linear(0.0),
                ..default()
            },
            ShieldHum,
            Transform::from_xyz((x0 + x1) / 2.0, 1.5, z),
        ));
    }
}

/// A forcefield's hum (its volume follows the settings).
#[derive(Component)]
struct ShieldHum;

fn on_events(
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    mut cores: Query<&mut CoreView>,
    mut secondaries: Query<&mut SecondaryView>,
    mut turrets: Query<&mut TurretView>,
    mut shields: Query<&mut ShieldView>,
) {
    let Some(m) = &game.level.mega else { return };
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::MegaHit { .. } => {
                for mut c in &mut cores {
                    c.since_hit = 0.0;
                }
            }
            SimEvent::SecondaryHit { side, .. } => {
                for mut s in secondaries.iter_mut().filter(|s| s.side == side) {
                    s.since_hit = 0.0;
                }
            }
            SimEvent::TurretFired { turret, .. } => {
                for mut t in turrets.iter_mut().filter(|t| t.k == turret as usize) {
                    t.since_fired = 0.0;
                }
            }
            SimEvent::Shocked { pos, .. } => {
                let near = m
                    .shields
                    .iter()
                    .enumerate()
                    .min_by(|a, b| {
                        let d = |s: &lt_sim::level::Shield| ((s.min + s.max) / 2.0).distance(pos);
                        d(a.1).total_cmp(&d(b.1))
                    })
                    .map(|(k, _)| k);
                for mut s in shields.iter_mut().filter(|s| Some(s.k) == near) {
                    s.flash = 1.0;
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate(
    time: Res<Time>,
    game: Res<Game>,
    settings: Option<Res<Settings>>,
    mut cores: Query<&mut CoreView>,
    mut secondaries: Query<&mut SecondaryView>,
    mut turrets: Query<(&mut TurretView, &mut Transform), Without<Climber>>,
    mut shields: Query<&mut ShieldView>,
    mut climbers: Query<(&Climber, &mut Transform), Without<TurretView>>,
    mut visibility: Query<&mut Visibility>,
    mut parts: Query<&mut Transform, (Without<TurretView>, Without<Climber>, Without<CoreView>)>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    // (Spatial: their sinks are `SpatialAudioSink`s.)
    mut hums: Query<&mut SpatialAudioSink, With<ShieldHum>>,
) {
    let Some(mega) = &game.level.mega else { return };
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let mine = me.map_or(0, |p| p.mega_hits);
    let powered = me
        .and_then(|p| p.power)
        .filter(|_| me.is_some_and(|p| p.respawn.is_none()));

    for mut c in &mut cores {
        c.since_hit += dt;
        let flash = (1.0 - c.since_hit / 1.2).max(0.0);
        if let Ok(mut tr) = parts.get_mut(c.core) {
            tr.scale = Vec3::splat(1.0 + 0.1 * (t * 3.0).sin() + flash * 1.2);
            tr.rotation = Quat::from_rotation_y(t * 0.8);
        }
        for (k, r) in c.rings.iter().enumerate() {
            if let Ok(mut tr) = parts.get_mut(*r) {
                let s = if k == 0 { 1.0 } else { -0.7 };
                tr.rotation = Quat::from_rotation_y(t * 0.6 * s)
                    * Quat::from_rotation_x(1.1 * s + 0.25 * (t * 0.5 + k as f32).sin());
                tr.scale = Vec3::splat(1.0 + flash * 0.6);
            }
        }
        // The shaft breathes, brighter for whoever may come in to shoot, blazing after a hit.
        let want = if powered.is_some() { 1.5 } else { 0.9 };
        if let Some(mut mat) = glows.get_mut(&c.shaft) {
            mat.neon.color.w = want * (0.8 + 0.2 * (t * 2.0).sin()) + flash * 4.0;
        }
        if let Some(mut mat) = glows.get_mut(&c.floor) {
            mat.neon.color.w = 1.6 * (0.75 + 0.25 * (t * 2.0 + 1.0).sin()) + flash * 5.0;
        }
    }
    for (cl, mut tr) in &mut climbers {
        let k = (t * 0.45 + cl.2).fract();
        tr.translation = cl.0.lerp(cl.1, k);
        let face = facing_into(cl.0, mega);
        tr.rotation = Quat::from_mat3(&Mat3::from_cols(Vec3::Y.cross(face), Vec3::Y, face));
        tr.scale = Vec3::splat((k * 6.0).min(1.0) * (1.0 - k).powf(0.5) * 2.4);
    }
    for mut s in &mut secondaries {
        s.since_hit += dt;
        let flash = (1.0 - s.since_hit / 0.6).max(0.0);
        let done = mine & (1 << s.side) != 0 || powered.is_some();
        if let Some(mut mat) = accents.get_mut(&s.mat) {
            let c = if done { DONE } else { TO_HIT };
            let i = if done {
                3.5
            } else {
                7.0 * (0.8 + 0.2 * (t * 4.0).sin())
            };
            mat.neon.color = (c.lerp(Vec3::ONE, flash)).extend(i + flash * 10.0);
        }
        if let Ok(mut tr) = parts.get_mut(s.core) {
            tr.scale = Vec3::splat(1.0 + 0.1 * (t * 5.0).sin() + flash);
        }
        for (k, r) in s.rings.iter().enumerate() {
            if let Ok(mut tr) = parts.get_mut(*r) {
                let spin = if k == 0 { 1.0 } else { -1.3 };
                tr.rotation = Quat::from_rotation_z(t * spin) * Quat::from_rotation_x(PI / 2.0);
                tr.scale = Vec3::splat(1.0 + flash * 0.4);
            }
        }
    }
    for (mut v, mut tr) in &mut turrets {
        v.since_fired += dt;
        let Some(st) = game.snap.turrets.get(v.k) else {
            continue;
        };
        if st.aim.is_finite() && st.aim.length_squared() > 0.5 {
            let want = look_rot(st.aim);
            tr.rotation = tr.rotation.slerp(want, (dt * 15.0).min(1.0));
        }
        let on = st.off <= 0.0;
        let kick = (1.0 - v.since_fired / 0.15).max(0.0);
        if let Some(mut mat) = accents.get_mut(&v.eye) {
            mat.neon.color = if on {
                Vec3::new(1.0, 0.05, 0.05).extend(6.0 + 10.0 * kick)
            } else {
                DARK.extend(1.0)
            };
        }
        if let Some(mut mat) = accents.get_mut(&v.sight) {
            mat.neon.color = if on {
                Vec3::new(1.0, 0.05, 0.05).extend(2.0 + 0.8 * (t * 11.0 + v.k as f32).sin())
            } else {
                DARK.extend(0.0)
            };
        }
        if let Some(mut mat) = accents.get_mut(&v.switch) {
            mat.neon.color = if on {
                SWITCH.extend(5.0 + 2.0 * (t * 6.0).sin())
            } else {
                // Off: dark, a faint pulse as the time runs out.
                let left = (st.off / game.rules.turret_off.max(0.1)).clamp(0.0, 1.0);
                SWITCH.extend(0.3 + 1.2 * (1.0 - left) * (0.5 + 0.5 * (t * 8.0).sin()))
            };
        }
        // The off timer draining towards its left end.
        let left = (st.off / game.rules.turret_off.max(0.1)).clamp(0.0, 1.0);
        if let Ok(mut tr) = parts.get_mut(v.timer) {
            tr.scale.x = left.max(0.001);
            tr.translation.x = -TIMER_W / 2.0 * (1.0 - left);
        }
        if let Ok(mut vis) = visibility.get_mut(v.timer) {
            let want = if on {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
            if *vis != want {
                *vis = want;
            }
        }
        let _ = v.gun;
    }
    for mut s in &mut shields {
        s.flash = (s.flash - dt * 2.0).max(0.0);
        let base = if powered.is_some() { DONE } else { FIELD };
        let c = base.lerp(Vec3::ONE, s.flash);
        if let Some(mut mat) = glows.get_mut(&s.sheet) {
            mat.neon.color = c.extend(2.4 + s.flash * 6.0);
        }
        if let Some(mut mat) = accents.get_mut(&s.posts) {
            mat.neon.color = c.extend(5.0 + 1.5 * (t * 9.0 + s.k as f32).sin() + s.flash * 8.0);
        }
    }
    let gain = crate::settings::sound_gain(settings.as_deref());
    for mut sink in &mut hums {
        sink.set_volume(Volume::Linear(0.6 * gain));
    }
}

/// The way into the room from a point on its wall: away from the nearest side of its inside.
fn facing_into(p: Vec3, m: &Mega) -> Vec3 {
    let n = m.inner.len();
    let q = Vec2::new(p.x, p.z);
    (0..n)
        .map(|i| {
            let (a, b) = (m.inner[i], m.inner[(i + 1) % n]);
            let e = (b - a).normalize_or_zero();
            let t = (q - a).dot(b - a) / (b - a).length_squared().max(1e-6);
            let d = q.distance(a + (b - a) * t.clamp(0.0, 1.0));
            (d, Vec3::new(-e.y, 0.0, e.x))
        })
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map_or(Vec3::Z, |d| d.1)
}

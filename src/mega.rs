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

use bevy::audio::{PlaybackMode, Volume};
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
            (on_events, animate).chain().run_if(resource_exists::<Game>),
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
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
    assets: &AssetServer,
) {
    let Some(m) = &level.mega else { return };
    spawn_core(commands, m, palette, meshes, accents, glows);
    spawn_secondaries(commands, m, palette, meshes, accents);
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
    let (a, b) = m.room;
    let inset = 0.06;
    // Columns at x (or z) = -3, -1, 1, 3 on every wall, but not in a doorway.
    let in_doorway = |p: Vec3| {
        m.shields
            .iter()
            .any(|s| (p.z - s.min.z).abs() < 0.5 && p.x > s.min.x - 0.3 && p.x < s.max.x + 0.3)
    };
    let mut columns: Vec<Vec3> = Vec::new();
    for k in 0..4 {
        let u = -3.0 + 2.0 * k as f32;
        for z in [b.z - inset, a.z + inset] {
            let p = Vec3::new(u, 0.0, z);
            if !in_doorway(p) {
                columns.push(p);
            }
        }
        for x in [b.x - inset, a.x + inset] {
            columns.push(Vec3::new(x, 0.0, u));
        }
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
fn facing(n: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Z, n.normalize_or(Vec3::Z))
}

fn spawn_secondaries(
    commands: &mut Commands,
    m: &Mega,
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
    mut hums: Query<&mut AudioSink, With<ShieldHum>>,
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
    let gain = settings.map_or(1.0, |s| s.sound_volume);
    for mut sink in &mut hums {
        sink.set_volume(Volume::Linear(0.6 * gain));
    }
}

/// The way into the room from a point on its wall.
fn facing_into(p: Vec3, m: &Mega) -> Vec3 {
    let (a, b) = m.room;
    let d = [
        (p.x - a.x, Vec3::X),
        (b.x - p.x, Vec3::NEG_X),
        (p.z - a.z, Vec3::Z),
        (b.z - p.z, Vec3::NEG_Z),
    ];
    d.iter()
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map_or(Vec3::Z, |d| d.1)
}

//! The arena's special features (lt_sim::level::Level): mirror panels on walls that bolts
//! bounce off, jump pads and laser gates; and the barriers players put up. Built from the neon materials and lit StandardMaterial, so every renderer draws
//! them.
//!
//! - Mirrors: dark polished glass on the wall's face with a faint lavender tint, glinting
//!   diagonal streaks across it and a lavender frame; the glints flash in the bolt's colour
//!   when one bounces off.
//! - Jump pads: a mint ring on the floor around a brighter disc, a soft column of light over
//!   it, and chevrons climbing along the flight's path to where it comes down; the pad blazes
//!   when it throws someone.
//! - Laser gates: three beams in the colour of the zone they stand in between glowing emitter
//!   posts, humming while on, flickering just before they switch on, a faint trace while off;
//!   they flash when they zap someone.
//! - Barriers (`Snapshot::barriers`): a sheet of hard light in the team's colour (the neon
//!   shader's forcefield halo) between two glowing posts, flashing where a bolt stops on it,
//!   flickering out in its last second.

use std::f32::consts::FRAC_PI_2;

use bevy::audio::{PlaybackMode, Volume};
use bevy::prelude::*;
use lt_sim::SimEvent;
use lt_sim::level::{GATE_H, Level, PAD_RADIUS, Panel, pad_flight};

use crate::arena::ArenaPart;
use crate::assets::MeshData;
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette, team_color};
use crate::settings::Settings;

/// The mirrors' frame and glint colour (lavender), the tint over the glass and the glints'
/// brightness.
pub const MIRROR: Vec3 = Vec3::new(0.42, 0.34, 1.0);
const SHEEN: f32 = 0.06;
const STREAK: f32 = 1.4;
/// The jump pads' colour (mint).
pub const PAD: Vec3 = Vec3::new(0.15, 1.0, 0.6);
const CHEVRONS: usize = 5;

pub struct FeaturesPlugin;

impl Plugin for FeaturesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BarrierViews>().add_systems(
            Update,
            (on_events, animate, sync_barriers)
                .chain()
                .run_if(resource_exists::<Game>),
        );
    }
}

#[derive(Component)]
struct PadView {
    pos: Vec3,
    ring: Handle<AccentMaterial>,
    column: Handle<GlowMaterial>,
    since: f32,
}

/// A chevron climbing a jump pad's flight path: the path (feet positions), and its phase.
#[derive(Component)]
struct PadChevron(std::sync::Arc<Vec<Vec3>>, f32);

/// The barriers on screen, by id: the root, the sheet's and posts' materials, and the flash
/// of the last bolt stopped.
#[derive(Resource, Default)]
struct BarrierViews(
    std::collections::HashMap<u32, (Entity, Handle<GlowMaterial>, Handle<AccentMaterial>, f32)>,
);

#[derive(Component)]
struct GateView {
    k: usize,
    beams: Handle<AccentMaterial>,
    glow: Handle<GlowMaterial>,
    posts: Handle<AccentMaterial>,
    flash: f32,
}

/// A laser gate's hum (heard while it's on).
#[derive(Component)]
struct GateHum(usize);

/// The beams' heights.
const BEAMS: [f32; 3] = [0.45, 1.0, 1.55];

#[derive(Component)]
struct MirrorView {
    centre: Vec3,
    sheen: Handle<GlowMaterial>,
    streak: Handle<AccentMaterial>,
    flash: f32,
    color: Vec3,
}

/// A flat sheet over a panel's face (uv 0..1 across and up), `out` metres in front of it.
pub fn panel_sheet(p: &Panel, out: f32) -> MeshData {
    let n = p.normal;
    let along = Vec3::Y.cross(n).normalize_or(Vec3::X);
    let c = p.centre() + n * out;
    let half = (p.max - p.min) / 2.0;
    let w = half.dot(along.abs());
    let h = half.y;
    let mut sheet = MeshData::default();
    for (a, y, u, v) in [
        (-w, -h, 0.0, 0.0),
        (w, -h, 1.0, 0.0),
        (w, h, 1.0, 1.0),
        (-w, h, 0.0, 1.0),
    ] {
        sheet.pos.push((c + along * a + Vec3::Y * y).to_array());
        sheet.nrm.push(n.to_array());
        sheet.uv.push([u, v]);
    }
    // Facing out (counter-clockwise seen from in front).
    sheet.idx.extend([0, 1, 2, 0, 2, 3]);
    sheet
}

/// Spawns the features of `level` (as parts of the arena).
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    level: &Level,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    std: &mut Assets<StandardMaterial>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
    assets: &AssetServer,
) {
    let _ = palette;
    spawn_pads(commands, level, meshes, accents, glows);
    spawn_gates(commands, level, meshes, accents, glows, assets);
    if level.mirrors.is_empty() {
        return;
    }
    let glass = std.add(StandardMaterial {
        base_color: Color::linear_rgb(0.06, 0.06, 0.08),
        metallic: 0.9,
        perceptual_roughness: 0.1,
        reflectance: 1.0,
        ..default()
    });
    let frame = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(MIRROR, 1.8),
    });
    let bar = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    for m in &level.mirrors {
        let n = m.normal;
        let along = Vec3::Y.cross(n).normalize_or(Vec3::X);
        let half = (m.max - m.min) / 2.0;
        let (w, h) = (half.dot(along.abs()), half.y);
        let c = m.centre();
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(panel_sheet(m, 0.004).to_mesh())),
            MeshMaterial3d(glass.clone()),
            Transform::IDENTITY,
            Name::new("mirror"),
        ));
        // A faint tint over the glass.
        let sheen = glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::fixed(MIRROR, SHEEN),
        });
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(panel_sheet(m, 0.012).to_mesh())),
            MeshMaterial3d(sheen.clone()),
            Transform::IDENTITY,
        ));
        let rot = Quat::from_mat3(&Mat3::from_cols(along, Vec3::Y, n));
        // Glints: diagonal streaks across the glass.
        let streak = accents.add(AccentMaterial {
            overlay: false,
            neon: NeonUniform::fixed(MIRROR, STREAK),
        });
        let tilt = rot * Quat::from_rotation_z(0.6);
        for (x, y, len, t) in [
            (-0.3, 0.35, 1.3, 0.025),
            (0.05, 0.05, 1.8, 0.05),
            (0.38, -0.4, 0.7, 0.02),
        ] {
            commands.spawn((
                ArenaPart,
                Mesh3d(bar.clone()),
                MeshMaterial3d(streak.clone()),
                Transform::from_translation(c + along * x * w / 0.8 + Vec3::Y * y + n * 0.008)
                    .with_rotation(tilt)
                    .with_scale(Vec3::new(t, len, 0.004)),
            ));
        }
        commands.spawn((
            ArenaPart,
            Transform::default(),
            MirrorView {
                centre: c,
                sheen,
                streak,
                flash: 0.0,
                color: MIRROR,
            },
        ));
        // The frame: strips along the four edges.
        for (offset, size) in [
            (Vec3::Y * h, Vec3::new(2.0 * w + 0.06, 0.04, 0.03)),
            (-Vec3::Y * h, Vec3::new(2.0 * w + 0.06, 0.04, 0.03)),
            (along * w, Vec3::new(0.04, 2.0 * h, 0.03)),
            (-along * w, Vec3::new(0.04, 2.0 * h, 0.03)),
        ] {
            commands.spawn((
                ArenaPart,
                Mesh3d(bar.clone()),
                MeshMaterial3d(frame.clone()),
                Transform::from_translation(c + offset + n * 0.015)
                    .with_rotation(rot)
                    .with_scale(size),
            ));
        }
    }
}

fn spawn_pads(
    commands: &mut Commands,
    level: &Level,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
) {
    if level.pads.is_empty() {
        return;
    }
    let world = level.world();
    let ring_mesh = meshes.add(Torus::new(PAD_RADIUS - 0.05, PAD_RADIUS + 0.02));
    let inner_mesh = meshes.add(Torus::new(0.3, 0.34));
    let disc = meshes.add(Circle::new(PAD_RADIUS - 0.06));
    let disc_mat = glows.add(GlowMaterial {
        overlay: false,
        neon: NeonUniform::fixed(PAD, 0.5),
    });
    let inner_mat = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(PAD, 1.8),
    });
    // A chevron: two bars in a V, pointing along +Z.
    let bar = meshes.add(Cuboid::new(0.06, 0.03, 0.5));
    let chevron_mat = accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(PAD, 3.5),
    });
    for p in &level.pads {
        let ring = accents.add(AccentMaterial {
            overlay: false,
            neon: NeonUniform::fixed(PAD, 4.0),
        });
        let column = glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::fixed(PAD, 0.5).volume(),
        });
        let up = p.pos + Vec3::Y * 0.02;
        commands.spawn((
            ArenaPart,
            Mesh3d(ring_mesh.clone()),
            MeshMaterial3d(ring.clone()),
            Transform::from_translation(up),
            PadView {
                pos: p.pos,
                ring,
                column: column.clone(),
                since: 99.0,
            },
            Name::new("jump pad"),
        ));
        commands.spawn((
            ArenaPart,
            Mesh3d(inner_mesh.clone()),
            MeshMaterial3d(inner_mat.clone()),
            Transform::from_translation(up),
        ));
        commands.spawn((
            ArenaPart,
            Mesh3d(disc.clone()),
            MeshMaterial3d(disc_mat.clone()),
            Transform::from_translation(p.pos + Vec3::Y * 0.015)
                .with_rotation(Quat::from_rotation_x(-FRAC_PI_2)),
        ));
        let mut col = MeshData::default();
        crate::scenery::cone(&mut col, p.pos, Vec3::Y, 2.4, PAD_RADIUS - 0.1, 0.25, 20);
        commands.spawn((
            ArenaPart,
            Mesh3d(meshes.add(col.to_mesh())),
            MeshMaterial3d(column),
            Transform::IDENTITY,
        ));
        let Some(path) = pad_flight(&world, p.pos, p.launch) else {
            continue;
        };
        let path = std::sync::Arc::new(path);
        for k in 0..CHEVRONS {
            let e = commands
                .spawn((
                    ArenaPart,
                    Transform::from_translation(p.pos),
                    Visibility::default(),
                    PadChevron(path.clone(), k as f32 / CHEVRONS as f32),
                ))
                .id();
            for s in [-1.0f32, 1.0] {
                commands.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(chevron_mat.clone()),
                    Transform::from_xyz(s * 0.17, 0.0, -0.17)
                        .with_rotation(Quat::from_rotation_y(-s * 0.8)),
                    ChildOf(e),
                ));
            }
        }
    }
}

fn spawn_gates(
    commands: &mut Commands,
    level: &Level,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
    assets: &AssetServer,
) {
    if level.gates.is_empty() {
        return;
    }
    let hum: Handle<AudioSource> = assets.load("sounds/gate_hum.ogg");
    let unit = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    for (k, g) in level.gates.iter().enumerate() {
        let c = g.centre().with_y(0.0);
        let size = g.max - g.min;
        let across = if size.x > size.z { Vec3::X } else { Vec3::Z };
        let len = size.dot(across);
        let color = crate::arena::zone_color(level, c);
        let beams = accents.add(AccentMaterial {
            overlay: false,
            neon: NeonUniform::fixed(color, 6.0),
        });
        let glow = glows.add(GlowMaterial {
            overlay: false,
            neon: NeonUniform::fixed(color, 1.5),
        });
        let posts = accents.add(AccentMaterial {
            overlay: false,
            neon: NeonUniform::fixed(color, 3.0),
        });
        let beam = across * len + (Vec3::ONE - across - Vec3::Y) * 0.025 + Vec3::Y * 0.025;
        let halo = across * len + (Vec3::ONE - across - Vec3::Y) * 0.002 + Vec3::Y * 0.12;
        let e = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(c),
                Visibility::default(),
                GateView {
                    k,
                    beams: beams.clone(),
                    glow: glow.clone(),
                    posts: posts.clone(),
                    flash: 0.0,
                },
                Name::new("laser gate"),
            ))
            .id();
        for y in BEAMS {
            commands.spawn((
                Mesh3d(unit.clone()),
                MeshMaterial3d(beams.clone()),
                Transform::from_xyz(0.0, y, 0.0).with_scale(beam),
                ChildOf(e),
            ));
            commands.spawn((
                Mesh3d(unit.clone()),
                MeshMaterial3d(glow.clone()),
                Transform::from_xyz(0.0, y, 0.0).with_scale(halo),
                ChildOf(e),
            ));
        }
        // Emitter posts at both ends.
        for s in [-1.0, 1.0] {
            commands.spawn((
                Mesh3d(unit.clone()),
                MeshMaterial3d(posts.clone()),
                Transform::from_translation(
                    across * s * (len / 2.0 + 0.04) + Vec3::Y * GATE_H / 2.0,
                )
                .with_scale(Vec3::new(0.05, GATE_H, 0.05)),
                ChildOf(e),
            ));
        }
        commands.spawn((
            ArenaPart,
            AudioPlayer(hum.clone()),
            PlaybackSettings {
                mode: PlaybackMode::Loop,
                spatial: true,
                volume: Volume::Linear(0.0),
                ..default()
            },
            GateHum(k),
            Transform::from_translation(c + Vec3::Y * 1.0),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_barriers(
    mut commands: Commands,
    game: Res<Game>,
    time: Res<Time>,
    mut views: ResMut<BarrierViews>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    mut unit: Local<Option<Handle<Mesh>>>,
) {
    let unit = unit
        .get_or_insert_with(|| meshes.add(Cuboid::new(1.0, 1.0, 1.0)))
        .clone();
    let (w, h) = (game.rules.barrier_w, game.rules.barrier_h);
    views.0.retain(|id, v| {
        let keep = game.snap.barriers.iter().any(|b| b.id == *id);
        // (Gone with the arena already, when it was taken down.)
        if !keep && let Ok(mut e) = commands.get_entity(v.0) {
            e.despawn();
        }
        keep
    });
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for b in &game.snap.barriers {
        let v = views.0.entry(b.id).or_insert_with(|| {
            let color = team_color(Some(b.team));
            let sheet_mat = glows.add(GlowMaterial {
                overlay: false,
                neon: NeonUniform::fixed(color, 2.0).shield(),
            });
            let posts = accents.add(AccentMaterial {
                overlay: false,
                neon: NeonUniform::fixed(color, 4.0),
            });
            let rot = Quat::from_rotation_arc(Vec3::Z, b.normal);
            let root = commands
                .spawn((
                    ArenaPart,
                    Transform::from_translation(b.centre).with_rotation(rot),
                    Visibility::default(),
                    Name::new("barrier"),
                ))
                .id();
            // The sheet: uv 0..1 across and up, seen from both sides.
            let mut sheet = MeshData::default();
            for (x, y, u, v) in [
                (-w / 2.0, -h / 2.0, 0.0, 0.0),
                (w / 2.0, -h / 2.0, 1.0, 0.0),
                (w / 2.0, h / 2.0, 1.0, 1.0),
                (-w / 2.0, h / 2.0, 0.0, 1.0),
            ] {
                sheet.pos.push([x, y, 0.0]);
                sheet.nrm.push([0.0, 0.0, 1.0]);
                sheet.uv.push([u, v]);
            }
            sheet.idx.extend([0, 1, 2, 0, 2, 3]);
            commands.spawn((
                Mesh3d(meshes.add(sheet.to_mesh())),
                MeshMaterial3d(sheet_mat.clone()),
                Transform::IDENTITY,
                ChildOf(root),
            ));
            for x in [-w / 2.0, w / 2.0] {
                commands.spawn((
                    Mesh3d(unit.clone()),
                    MeshMaterial3d(posts.clone()),
                    Transform::from_xyz(x, 0.0, 0.0).with_scale(Vec3::new(0.06, h, 0.06)),
                    ChildOf(root),
                ));
            }
            for y in [-h / 2.0, h / 2.0] {
                commands.spawn((
                    Mesh3d(unit.clone()),
                    MeshMaterial3d(posts.clone()),
                    Transform::from_xyz(0.0, y, 0.0).with_scale(Vec3::new(w, 0.03, 0.03)),
                    ChildOf(root),
                ));
            }
            (root, sheet_mat, posts, 0.0)
        });
        v.3 = (v.3 - dt * 3.0).max(0.0);
        // Going up: swelling in; the last second: flickering out.
        let rise = ((game.rules.barrier_time - b.left) / 0.25).clamp(0.0, 1.0);
        let fade = if b.left < 1.0 {
            if (t * 12.0).fract() < 0.5 {
                b.left
            } else {
                b.left * 0.3
            }
        } else {
            1.0
        };
        let k = rise * fade;
        if let Some(mut m) = glows.get_mut(&v.1) {
            m.neon.color.w = (2.0 + 6.0 * v.3) * k;
        }
        if let Some(mut m) = accents.get_mut(&v.2) {
            m.neon.color.w = (4.0 + 6.0 * v.3) * k.max(0.2);
        }
    }
}

fn on_events(
    mut events: MessageReader<GameEvent>,
    mut mirrors: Query<&mut MirrorView>,
    mut pads: Query<&mut PadView>,
    mut gates: Query<&mut GateView>,
    mut barriers: ResMut<BarrierViews>,
) {
    for GameEvent(e) in events.read() {
        if let SimEvent::BarrierHit { id, .. } = *e
            && let Some(v) = barriers.0.get_mut(&id)
        {
            v.3 = 1.0;
        }
        if let SimEvent::GateZap { gate, .. } = *e {
            for mut g in gates.iter_mut().filter(|g| g.k == gate as usize) {
                g.flash = 1.0;
            }
        }
        if let SimEvent::Launched { pos, .. } = *e
            && let Some(mut p) = pads
                .iter_mut()
                .min_by(|a, b| a.pos.distance(pos).total_cmp(&b.pos.distance(pos)))
        {
            p.since = 0.0;
        }
        if let SimEvent::Reflected { pos, team, .. } = *e
            && let Some(mut m) = mirrors
                .iter_mut()
                .min_by(|a, b| a.centre.distance(pos).total_cmp(&b.centre.distance(pos)))
        {
            m.flash = 1.0;
            m.color = team_color(team);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn animate(
    time: Res<Time>,
    game: Res<Game>,
    settings: Option<Res<Settings>>,
    mut gates: Query<&mut GateView>,
    mut hums: Query<(&GateHum, &mut AudioSink)>,
    mut mirrors: Query<&mut MirrorView>,
    mut pads: Query<&mut PadView>,
    mut chevrons: Query<(&PadChevron, &mut Transform)>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    let tick = game.snap.tick;
    let gain = crate::settings::sound_gain(settings.as_deref());
    for mut v in &mut gates {
        let Some(g) = game.level.gates.get(v.k) else {
            continue;
        };
        v.flash = (v.flash - dt * 3.0).max(0.0);
        let on = g.active(tick);
        // On: bright, buzzing a little; about to come on: flickering; off: a faint trace.
        let k = if on {
            0.85 + 0.15 * (t * 37.0 + v.k as f32).sin()
        } else if g.warning(tick) {
            if (t * 14.0).fract() < 0.5 { 0.6 } else { 0.05 }
        } else {
            0.04
        };
        let k = k + v.flash * 2.0;
        if let Some(mut m) = accents.get_mut(&v.beams) {
            m.neon.color.w = 6.0 * k;
        }
        if let Some(mut m) = glows.get_mut(&v.glow) {
            m.neon.color.w = 1.5 * k;
        }
        if let Some(mut m) = accents.get_mut(&v.posts) {
            m.neon.color.w = if on { 2.0 } else { 0.7 };
        }
    }
    for (h, mut sink) in &mut hums {
        let on = game.level.gates.get(h.0).is_some_and(|g| g.active(tick));
        sink.set_volume(Volume::Linear(if on { 0.7 * gain } else { 0.0 }));
    }
    for mut p in &mut pads {
        p.since += dt;
        let blaze = (1.0 - p.since / 0.6).max(0.0);
        let beat = 0.5 + 0.5 * (t * 3.0 + p.pos.x).sin();
        if let Some(mut m) = accents.get_mut(&p.ring) {
            m.neon.color.w = 1.6 + 0.8 * beat + 8.0 * blaze;
        }
        if let Some(mut m) = glows.get_mut(&p.column) {
            m.neon.color.w = 0.35 + 0.25 * beat + 2.5 * blaze;
        }
    }
    // Chevrons climb the flight's path, growing in and fading out, about a second each way.
    for (c, mut tr) in &mut chevrons {
        let path = &c.0;
        let k = (t * 0.7 + c.1).fract();
        let f = k * (path.len() - 1) as f32;
        let i = (f as usize).min(path.len() - 2);
        let (a, b) = (path[i], path[i + 1]);
        let at = a.lerp(b, f - i as f32) + Vec3::Y * 0.9;
        let dir = (b - a).normalize_or(Vec3::Y);
        tr.translation = at;
        tr.rotation = Quat::from_rotation_arc(Vec3::Z, dir);
        tr.scale = Vec3::splat((k * 8.0).min(1.0) * (1.0 - k).powf(0.7));
    }
    for mut m in &mut mirrors {
        m.flash = (m.flash - dt * 2.5).max(0.0);
        let k = m.flash * m.flash;
        let c = MIRROR.lerp(m.color, k);
        // A slow glint, blazing in the bolt's colour when one bounces off.
        let glint = 0.75 + 0.25 * (t * 0.7 + m.centre.x * 0.3 + m.centre.z * 0.2).sin();
        if let Some(mut mat) = glows.get_mut(&m.sheen) {
            mat.neon.color = c.extend(SHEEN + 1.5 * k);
        }
        if let Some(mut mat) = accents.get_mut(&m.streak) {
            mat.neon.color = c.extend(STREAK * glint + 8.0 * k);
        }
        if m.flash <= 0.0 {
            m.color = MIRROR;
        }
    }
}

//! The arena's special features (lt_sim::level::Level): mirror panels on walls that bolts
//! bounce off, and jump pads. Built from the neon materials and lit StandardMaterial, so every renderer draws
//! them.
//!
//! - Mirrors: dark polished glass on the wall's face with a faint lavender tint, glinting
//!   diagonal streaks across it and a lavender frame; the glints flash in the bolt's colour
//!   when one bounces off.
//! - Jump pads: a mint ring on the floor around a brighter disc, a soft column of light over
//!   it, and chevrons climbing along the flight's path to where it comes down; the pad blazes
//!   when it throws someone.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use lt_sim::SimEvent;
use lt_sim::level::{Level, PAD_RADIUS, Panel, pad_flight};

use crate::arena::ArenaPart;
use crate::assets::MeshData;
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette, team_color};

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
        app.add_systems(
            Update,
            (on_events, animate).chain().run_if(resource_exists::<Game>),
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
pub fn spawn(
    commands: &mut Commands,
    level: &Level,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    std: &mut Assets<StandardMaterial>,
    accents: &mut Assets<AccentMaterial>,
    glows: &mut Assets<GlowMaterial>,
) {
    let _ = palette;
    spawn_pads(commands, level, meshes, accents, glows);
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
        neon: NeonUniform::fixed(PAD, 0.5),
    });
    let inner_mat = accents.add(AccentMaterial {
        neon: NeonUniform::fixed(PAD, 1.8),
    });
    // A chevron: two bars in a V, pointing along +Z.
    let bar = meshes.add(Cuboid::new(0.06, 0.03, 0.5));
    let chevron_mat = accents.add(AccentMaterial {
        neon: NeonUniform::fixed(PAD, 3.5),
    });
    for p in &level.pads {
        let ring = accents.add(AccentMaterial {
            neon: NeonUniform::fixed(PAD, 4.0),
        });
        let column = glows.add(GlowMaterial {
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
                        .with_rotation(Quat::from_rotation_y(s * 0.8)),
                    ChildOf(e),
                ));
            }
        }
    }
}

fn on_events(
    mut events: MessageReader<GameEvent>,
    mut mirrors: Query<&mut MirrorView>,
    mut pads: Query<&mut PadView>,
) {
    for GameEvent(e) in events.read() {
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

fn animate(
    time: Res<Time>,
    mut mirrors: Query<&mut MirrorView>,
    mut pads: Query<&mut PadView>,
    mut chevrons: Query<(&PadChevron, &mut Transform)>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
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

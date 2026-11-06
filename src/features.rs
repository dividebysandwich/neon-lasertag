//! The arena's special features (lt_sim::level::Level): mirror panels on walls that bolts
//! bounce off. Built from the neon materials and lit StandardMaterial, so every renderer draws
//! them.
//!
//! - Mirrors: dark polished glass on the wall's face with a faint lavender tint, glinting
//!   diagonal streaks across it and a lavender frame; the glints flash in the bolt's colour
//!   when one bounces off.

use bevy::prelude::*;
use lt_sim::SimEvent;
use lt_sim::level::{Level, Panel};

use crate::arena::ArenaPart;
use crate::assets::MeshData;
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette, team_color};

/// The mirrors' frame and glint colour (lavender), the tint over the glass and the glints'
/// brightness.
pub const MIRROR: Vec3 = Vec3::new(0.42, 0.34, 1.0);
const SHEEN: f32 = 0.06;
const STREAK: f32 = 1.4;

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

fn on_events(mut events: MessageReader<GameEvent>, mut mirrors: Query<&mut MirrorView>) {
    for GameEvent(e) in events.read() {
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
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
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

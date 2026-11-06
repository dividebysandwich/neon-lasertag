//! The team targets: a small glowing core hanging on a short stem from a roof over the upper
//! level (lt_sim::kit::target_box), in a cage ring, with chevrons around it and under it
//! that keep running in towards it. Built from the neon materials, so every renderer draws it.

use bevy::prelude::*;
use lt_sim::level::{Level, TargetDef};
use lt_sim::kit::{FLOOR_T, STOREY, TARGET_HALF};
use lt_sim::{SimEvent, Team};

use crate::arena::ArenaPart;
use crate::game::{Game, GameEvent};
use crate::materials::Palette;

/// A target's parts that move.
#[derive(Component)]
pub struct TargetView {
    pub id: u8,
    core: Entity,
    ring: Entity,
    /// Chevrons with their path: start, end (where they point), phase offset.
    arrows: Vec<(Entity, Vec3, Vec3, f32)>,
    /// Seconds since the last hit (flashes the core).
    pub since_hit: f32,
}

pub struct TargetsPlugin;

impl Plugin for TargetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (on_hits, animate).chain().run_if(resource_exists::<Game>));
    }
}

/// A chevron (">" pointing along +Y), from two bars.
fn chevron() -> Mesh {
    let bar = Cuboid::new(0.05, 0.24, 0.05).mesh().build();
    let place = |sx: f32| bar.clone().transformed_by(Transform::from_xyz(sx * 0.075, -0.075, 0.0).with_rotation(Quat::from_rotation_z(sx * 0.8)));
    let mut out = place(-1.0);
    let _ = out.merge(&place(1.0));
    out
}

/// Spawns the targets of `level` (as parts of the arena).
pub fn spawn(commands: &mut Commands, level: &Level, palette: &Palette, meshes: &mut Assets<Mesh>) {
    let core_mesh = meshes.add(Sphere::new(TARGET_HALF.x * 0.62).mesh().ico(1).unwrap());
    let shell_mesh = meshes.add(Cuboid::from_size(TARGET_HALF * 1.25));
    let ring_mesh = meshes.add(Torus::new(TARGET_HALF.x * 0.9, TARGET_HALF.x * 0.98));
    let stem_mesh = meshes.add(Cylinder::new(0.035, 1.0));
    let mount_mesh = meshes.add(Cylinder::new(0.16, 0.04));
    let arrow_mesh = meshes.add(chevron());
    for t in &level.targets {
        let team = Some(t.team);
        let c = t.centre();
        // The roof over the upper level.
        let ceiling = 2.0 * STOREY - FLOOR_T;
        let root = commands.spawn((ArenaPart, Transform::from_translation(c), Visibility::default(), Name::new("target"))).id();
        // The stem up to the ceiling, and its mount.
        let stem_len = ceiling - (c.y + TARGET_HALF.y * 0.6);
        commands.spawn((
            Mesh3d(stem_mesh.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, ceiling - c.y - stem_len / 2.0, 0.0).with_scale(Vec3::new(1.0, stem_len, 1.0)),
            ChildOf(root),
        ));
        commands.spawn((
            Mesh3d(mount_mesh.clone()),
            MeshMaterial3d(palette.accent_for(team)),
            Transform::from_xyz(0.0, ceiling - c.y - 0.02, 0.0),
            ChildOf(root),
        ));
        // A dark faceted shell with the bright core inside, and a ring around them.
        commands.spawn((
            Mesh3d(shell_mesh.clone()),
            MeshMaterial3d(palette.body.clone()),
            Transform::from_rotation(Quat::from_rotation_y(0.785) * Quat::from_rotation_x(0.615)).with_scale(Vec3::splat(0.55)),
            ChildOf(root),
        ));
        let core = commands
            .spawn((Mesh3d(core_mesh.clone()), MeshMaterial3d(palette.bolt_for(team)), Transform::IDENTITY, ChildOf(root)))
            .id();
        let ring = commands
            .spawn((Mesh3d(ring_mesh.clone()), MeshMaterial3d(palette.accent_for(team)), Transform::IDENTITY, ChildOf(root)))
            .id();
        // Chevrons: two at its sides in the plane facing its open side (the walls hem it in),
        // and a column of three under it pointing up.
        let n = t.normal;
        let side = Vec3::Y.cross(n).normalize_or(Vec3::X);
        let mut arrows = Vec::new();
        for (k, dir) in [side, -side].into_iter().enumerate() {
            let from = dir * 0.78 + n * 0.1;
            let to = dir * 0.4 + n * 0.1;
            let e = spawn_arrow(commands, root, &arrow_mesh, palette, team);
            arrows.push((e, from, to, k as f32 * 0.5));
        }
        for k in 0..3 {
            let from = Vec3::new(0.0, -1.7, 0.0) + n * 0.25;
            let to = Vec3::new(0.0, -0.45, 0.0) + n * 0.25;
            let e = spawn_arrow(commands, root, &arrow_mesh, palette, team);
            arrows.push((e, from, to, k as f32 / 3.0));
        }
        commands.entity(root).insert(TargetView { id: t.id, core, ring, arrows, since_hit: 99.0 });
    }
}

fn spawn_arrow(commands: &mut Commands, root: Entity, mesh: &Handle<Mesh>, palette: &Palette, team: Option<Team>) -> Entity {
    commands
        .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(palette.bolt_for(team)), Transform::IDENTITY, ChildOf(root)))
        .id()
}

fn on_hits(mut events: MessageReader<GameEvent>, mut views: Query<&mut TargetView>) {
    for GameEvent(e) in events.read() {
        if let SimEvent::TargetHit { target, .. } = *e {
            for mut v in &mut views {
                if v.id == target {
                    v.since_hit = 0.0;
                }
            }
        }
    }
}

fn animate(
    time: Res<Time>,
    game: Res<Game>,
    mut views: Query<(&mut TargetView, &Transform)>,
    mut transforms: Query<&mut Transform, Without<TargetView>>,
) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (mut v, root) in &mut views {
        v.since_hit += dt;
        let Some(def) = game.level.targets.iter().find(|d| d.id == v.id) else { continue };
        let flash = (1.0 - v.since_hit / 0.5).max(0.0);
        if let Ok(mut c) = transforms.get_mut(v.core) {
            let beat = 1.0 + 0.08 * (t * 5.0).sin();
            c.scale = Vec3::splat(beat + flash * 0.8);
            c.rotation = Quat::from_rotation_y(t * 1.3);
        }
        if let Ok(mut r) = transforms.get_mut(v.ring) {
            r.rotation = Quat::from_rotation_y(t * 0.9) * Quat::from_rotation_x(1.2 + 0.3 * (t * 0.7).sin());
            r.scale = Vec3::splat(1.0 + flash * 0.5);
        }
        let _ = root;
        for &(e, from, to, phase) in &v.arrows {
            let Ok(mut a) = transforms.get_mut(e) else { continue };
            arrow_pose(&mut a, def, from, to, (t * 0.8 + phase).fract());
        }
    }
}

/// A chevron `k` (0..1) of the way from `from` to `to`, pointing at `to`, growing in and
/// fading out (shrinking) as it arrives.
fn arrow_pose(a: &mut Transform, def: &TargetDef, from: Vec3, to: Vec3, k: f32) {
    let dir = (to - from).normalize_or(Vec3::Y);
    a.translation = from.lerp(to, k);
    // +Y of the chevron along the path; its face towards the open side.
    let face = def.normal.reject_from(dir).normalize_or(Vec3::Z);
    a.rotation = Quat::from_mat3(&Mat3::from_cols(dir.cross(face), dir, face));
    a.scale = Vec3::splat((k * 5.0).min(1.0) * (1.0 - k).powf(0.5) * 1.1);
}

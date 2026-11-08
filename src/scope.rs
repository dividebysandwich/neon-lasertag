//! VR: the prism lance's scope. A short tube on top of the blaster with a lens at its back end.
//! Held up to the eye (`ACTIVE_DIST`), the lens shows the arena magnified along the barrel. The
//! headset renderer draws that view from the muzzle with a narrow field of view and paints it
//! on the lens: `--gl-vr` in src/glvr/render.rs, `--vr` with a second camera here.
//! Away from the eye the lens is dark glass, and no extra view is drawn.
//!
//! The lens is no mesh of its own. [`ScopeLens`] on an entity tells the renderer where to draw
//! it and where the scope's camera is. The tube and its rings are ordinary meshes, its children.

use bevy::prelude::*;
use lt_sim::{Team, Weapon};

use crate::game::{Game, LocalCmd, Mover};
use crate::materials::Palette;
use crate::vr::VrState;

/// The lens's centre in the blaster's frame (above the top rib, at its back end, facing the
/// player), and its radius (m).
pub const LENS_AT: Vec3 = Vec3::new(0.0, 0.155, 0.031);
pub const LENS_RADIUS: f32 = 0.017;
/// The tube: its radius and length (ahead of the lens).
const TUBE_RADIUS: f32 = 0.02;
const TUBE_LENGTH: f32 = 0.1;
/// Magnification, about as the flat screen's zoom (`desktop::ZOOM`): the camera's half field of
/// view is the lens's apparent size at `EYE_RELIEF` divided by it.
const MAGNIFY: f32 = 2.5;
const EYE_RELIEF: f32 = 0.1;
/// The lens comes alive within this distance of an eye (m), facing it.
const ACTIVE_DIST: f32 = 0.3;

/// Where to draw the scope's lens (this entity's `GlobalTransform`: its centre, facing +Z
/// towards the player, `radius` big) and what it shows. With `on`, the view from `camera`
/// (world, looking along -Z) at half field of view `atan(tan)`; without, dark glass.
#[derive(Component, Clone, Copy, Debug)]
pub struct ScopeLens {
    pub on: bool,
    pub camera: Transform,
    pub tan: f32,
    pub radius: f32,
}

/// The scope's entities: the lens (the root) and its rings, which take the team's colour.
#[derive(Resource)]
pub struct Scope {
    pub lens: Entity,
    rings: [Entity; 2],
    team: Option<Team>,
}

pub fn spawn_scope(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    palette: Res<Palette>,
) {
    let tan = (LENS_RADIUS / EYE_RELIEF) / MAGNIFY;
    let lens = commands
        .spawn((
            Name::new("scope"),
            ScopeLens {
                on: false,
                camera: Transform::IDENTITY,
                tan,
                radius: LENS_RADIUS,
            },
            Transform::IDENTITY,
            Visibility::Hidden,
        ))
        .id();
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.03, 0.03, 0.035),
        perceptual_roughness: 0.4,
        ..default()
    });
    let along_z = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
    // The tube, just ahead of the lens, and the mount down to the rib.
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(TUBE_RADIUS, TUBE_LENGTH))),
        MeshMaterial3d(body.clone()),
        Transform::from_xyz(0.0, 0.0, -0.002 - TUBE_LENGTH / 2.0).with_rotation(along_z),
        ChildOf(lens),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.012, 0.014, 0.04))),
        MeshMaterial3d(body),
        Transform::from_xyz(0.0, -TUBE_RADIUS - 0.004, -0.05),
        ChildOf(lens),
    ));
    let ring = meshes.add(Torus::new(TUBE_RADIUS - 0.001, TUBE_RADIUS + 0.003));
    let rings = [0.0, -TUBE_LENGTH].map(|z| {
        commands
            .spawn((
                Mesh3d(ring.clone()),
                MeshMaterial3d(palette.bolt_for(None)),
                Transform::from_xyz(0.0, 0.0, z - 0.002).with_rotation(along_z),
                ChildOf(lens),
            ))
            .id()
    });
    commands.insert_resource(Scope {
        lens,
        rings,
        team: None,
    });
}

/// On the blaster while the prism lance is held, alive when held up to an eye.
#[allow(clippy::too_many_arguments)]
pub fn place_scope(
    mut commands: Commands,
    scope: Option<ResMut<Scope>>,
    vr: Res<VrState>,
    game: Res<Game>,
    local: Res<LocalCmd>,
    mover: Res<Mover>,
    models: Res<crate::assets::Models>,
    palette: Res<Palette>,
    mut q: Query<(&mut Transform, &mut Visibility, &mut ScopeLens)>,
) {
    let Some(mut scope) = scope else { return };
    let me = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me));
    let team = me.map(|p| p.team);
    if team != scope.team {
        scope.team = team;
        for e in scope.rings {
            commands
                .entity(e)
                .insert(MeshMaterial3d(palette.bolt_for(team)));
        }
    }
    let Ok((mut t, mut vis, mut lens)) = q.get_mut(scope.lens) else {
        return;
    };
    let gun = vr
        .gun
        .filter(|_| me.is_some_and(|p| p.weapon == Weapon::Lance && p.respawn.is_none()));
    let Some(hand) = gun else {
        *vis = Visibility::Hidden;
        lens.on = false;
        return;
    };
    *vis = Visibility::Inherited;
    // (The gun's pose as the avatar's: src/actors.rs.)
    let g = local.cmd.hands[hand.index()];
    let gun = Transform::from_translation(mover.feet + g.pos).with_rotation(g.rot);
    *t = Transform::from_translation(gun.transform_point(LENS_AT)).with_rotation(g.rot);
    // From the muzzle along the barrel: the middle of the view is where the shot goes.
    lens.camera =
        Transform::from_translation(gun.transform_point(models.muzzle)).with_rotation(g.rot);
    // Up to an eye, and looked into from behind.
    let eye = mover.feet + local.cmd.head.pos;
    let back = g.rot * Vec3::Z;
    let to_eye = eye - t.translation;
    lens.on = to_eye.length() < ACTIVE_DIST && to_eye.normalize_or_zero().dot(back) > 0.7;
}

/// Whether the scope is held up to an eye (the laser sight's beam would block its view).
pub fn scope_on(scope: Option<&Scope>, lenses: &Query<&ScopeLens>) -> bool {
    scope.is_some_and(|s| lenses.get(s.lens).is_ok_and(|l| l.on))
}

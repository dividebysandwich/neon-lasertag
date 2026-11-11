//! VR: the prism lance's scope. A short tube on top of the blaster with a lens at its back end.
//! Held up to the eye (`ACTIVE_DIST`), the lens shows the arena magnified along the barrel. The
//! headset renderer draws that view from the muzzle with a narrow field of view and paints it
//! on the lens: `--gl-vr` in src/glvr/render.rs, `--vr` with a second camera here.
//! Away from the eye the lens is dark glass, and no extra view is drawn.
//!
//! The lens is no mesh of its own. [`ScopeLens`] on an entity tells the renderer where to draw
//! it and where the scope's camera is. The tube and its rings are ordinary meshes, its children.
//! For `--vr`, [`BevyScopePlugin`] adds the camera (drawing into an image) and the lens's meshes:
//! the view, and a reticle over it.
//!
//! Magnified, a hand's tremor shakes the view, so the scope looks along a steadied aim
//! ([`Steady`]): small, quick wobbles smoothed away, deliberate swings followed at once. While
//! it is at an eye the lance fires along that aim too, where the reticle shows.

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

/// The steadied aim: a one-euro filter on the gun's rotation. Turning slowly, it follows at
/// `MIN_CUTOFF` Hz (smoothing a tremor away); faster, `BETA` Hz more per radian a second
/// (keeping up with a swing). Its speed is smoothed at `SPEED_CUTOFF` Hz.
const MIN_CUTOFF: f32 = 1.2;
const BETA: f32 = 3.0;
const SPEED_CUTOFF: f32 = 2.0;

/// The gun's steadied rotation (`MIN_CUTOFF`) and how fast it turns (rad/s, smoothed).
#[derive(Clone, Copy, Debug)]
pub struct Steady {
    pub rot: Quat,
    speed: f32,
}

impl Steady {
    pub fn new(rot: Quat) -> Self {
        Steady { rot, speed: 0.0 }
    }

    /// Follows `rot` over `dt` seconds.
    pub fn follow(&mut self, rot: Quat, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let k = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz * dt).exp();
        let speed = self.rot.angle_between(rot) / dt;
        self.speed += (speed - self.speed) * k(SPEED_CUTOFF);
        self.rot = self
            .rot
            .slerp(rot, k(MIN_CUTOFF + BETA * self.speed))
            .normalize();
    }
}

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
    pub team: Option<Team>,
    /// The gun's steadied aim (none while the lance isn't held).
    pub steady: Option<Steady>,
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
        steady: None,
    });
}

/// On the blaster while the prism lance is held, alive when held up to an eye; then the lance
/// fires along the steadied aim the scope looks along.
#[allow(clippy::too_many_arguments)]
pub fn place_scope(
    mut commands: Commands,
    scope: Option<ResMut<Scope>>,
    vr: Res<VrState>,
    game: Res<Game>,
    mut local: ResMut<LocalCmd>,
    time: Res<Time>,
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
        scope.steady = None;
        return;
    };
    *vis = Visibility::Inherited;
    // (The gun's pose as the avatar's: src/actors.rs.)
    let g = local.cmd.hands[hand.index()];
    let gun = Transform::from_translation(mover.feet + g.pos).with_rotation(g.rot);
    *t = Transform::from_translation(gun.transform_point(LENS_AT)).with_rotation(g.rot);
    // From the muzzle along the barrel, steadied: the middle of the view is where the shot
    // goes.
    let steady = scope.steady.get_or_insert(Steady::new(g.rot));
    steady.follow(g.rot, time.delta_secs());
    let aim = steady.rot;
    lens.camera =
        Transform::from_translation(gun.transform_point(models.muzzle)).with_rotation(aim);
    // Up to an eye, and looked into from behind.
    let eye = mover.feet + local.cmd.head.pos;
    let back = g.rot * Vec3::Z;
    let to_eye = eye - t.translation;
    lens.on = to_eye.length() < ACTIVE_DIST && to_eye.normalize_or_zero().dot(back) > 0.7;
    if lens.on {
        local.cmd.muzzle.rot = lt_sim::sim::look_rot(aim * Vec3::NEG_Z);
    }
}

/// `--vr`: the scope's view drawn by Bevy (into `VIEW_SIZE`² pixels).
pub struct BevyScopePlugin;

const VIEW_SIZE: u32 = 384;

impl Plugin for BevyScopePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_view.run_if(resource_exists::<Scope>.and_then(not(resource_exists::<View>))),
                update_view.after(crate::vr::PoseSet),
            ),
        );
    }
}

/// The scope's camera, its lens's mesh and materials (its view, dark glass), and the reticle's
/// material.
#[derive(Resource)]
struct View {
    camera: Entity,
    lens: Entity,
    on: Handle<StandardMaterial>,
    off: Handle<StandardMaterial>,
    reticle: Handle<StandardMaterial>,
    team: Option<Option<Team>>,
}

fn spawn_view(
    mut commands: Commands,
    scope: Res<Scope>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    lenses: Query<&ScopeLens>,
) {
    use bevy::camera::{Hdr, RenderTarget};
    use bevy::core_pipeline::tonemapping::Tonemapping;
    use bevy::render::render_resource::TextureFormat;
    let Ok(lens) = lenses.get(scope.lens) else {
        return;
    };
    // HDR and not tone mapped: the eyes' cameras do that (and the bloom) to the lens too.
    let image = images.add(Image::new_target_texture(
        VIEW_SIZE,
        VIEW_SIZE,
        TextureFormat::Rgba16Float,
        None,
    ));
    let camera = commands
        .spawn((
            Name::new("scope camera"),
            Camera3d::default(),
            Camera {
                order: -1,
                is_active: false,
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            Hdr,
            Tonemapping::None,
            Msaa::Sample4,
            Projection::Perspective(PerspectiveProjection {
                fov: 2.0 * lens.tan.atan(),
                aspect_ratio: 1.0,
                near: 0.03,
                ..default()
            }),
        ))
        .id();
    let on = materials.add(StandardMaterial {
        base_color_texture: Some(image),
        unlit: true,
        ..default()
    });
    let off = materials.add(StandardMaterial {
        base_color: Color::srgb(0.01, 0.012, 0.02),
        perceptual_roughness: 0.15,
        reflectance: 0.6,
        ..default()
    });
    let disc = meshes.add(disc(lens.radius));

    let lens_mesh = commands
        .spawn((
            Mesh3d(disc.clone()),
            MeshMaterial3d(off.clone()),
            Transform::from_xyz(0.0, 0.0, 0.0005),
            ChildOf(scope.lens),
        ))
        .id();
    let reticle = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(reticle_image())),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.spawn((
        Mesh3d(disc),
        MeshMaterial3d(reticle.clone()),
        Transform::from_xyz(0.0, 0.0, 0.001),
        Visibility::Hidden,
        ScopeReticle,
        ChildOf(scope.lens),
    ));
    commands.insert_resource(View {
        camera,
        lens: lens_mesh,
        on,
        off,
        reticle,
        team: None,
    });
}

/// The reticle over the lens: shown with the view.
#[derive(Component)]
struct ScopeReticle;

/// The camera follows the scope and draws only while it is held up to an eye; the lens shows
/// its view then, else dark glass.
#[allow(clippy::too_many_arguments)]
fn update_view(
    mut commands: Commands,
    view: Option<ResMut<View>>,
    scope: Option<Res<Scope>>,
    lenses: Query<&ScopeLens>,
    mut cams: Query<(&mut Camera, &mut Transform)>,
    mut reticles: Query<&mut Visibility, With<ScopeReticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(mut view), Some(scope)) = (view, scope) else {
        return;
    };

    let Ok(lens) = lenses.get(scope.lens) else {
        return;
    };
    if let Ok((mut cam, mut t)) = cams.get_mut(view.camera) {
        if cam.is_active != lens.on {
            cam.is_active = lens.on;
            let m = if lens.on { &view.on } else { &view.off };
            commands.entity(view.lens).insert(MeshMaterial3d(m.clone()));
        }
        *t = lens.camera;
    }
    for mut v in &mut reticles {
        let want = if lens.on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if view.team != Some(scope.team) {
        view.team = Some(scope.team);
        let c = match scope.team {
            Some(Team::Red) => crate::materials::RED,
            Some(Team::Blue) => crate::materials::BLUE,
            None => Vec3::ONE,
        };
        if let Some(mut m) = materials.get_mut(&view.reticle) {
            m.base_color = Color::linear_rgb(c.x, c.y, c.z);
        }
    }
}

/// A disc of `radius` facing +Z, its UVs across it as an image's (u right, v down).
/// (Bevy's own `Circle` mesh didn't show in the headset.)
fn disc(radius: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    const N: u32 = 40;
    let mut positions = vec![[0.0, 0.0, 0.0]];
    let mut uvs = vec![[0.5, 0.5]];
    for i in 0..N {
        let (y, x) = (i as f32 / N as f32 * std::f32::consts::TAU).sin_cos();
        positions.push([x * radius, y * radius, 0.0]);
        uvs.push([0.5 + 0.5 * x, 0.5 - 0.5 * y]);
    }
    let indices = (0..N).flat_map(|i| [0, 1 + i, 1 + (i + 1) % N]).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![[0.0, 0.0, 1.0]; N as usize + 1],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The reticle (as lens.glsl's): a fine white cross with a gap in the middle and a dot, and the
/// rim darkened as through a tube; clear elsewhere.
fn reticle_image() -> Image {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: usize = 256;
    let mut data = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) / N as f32 * 2.0 - Vec2::ONE;
            let r = p.length();
            let aa = 2.0 / N as f32;
            let w = 0.012;
            let line = (1.0 - ((p.x.abs() - w) / aa).clamp(0.0, 1.0))
                .max(1.0 - ((p.y.abs() - w) / aa).clamp(0.0, 1.0))
                * ((r - 0.1) / 0.02).clamp(0.0, 1.0)
                * (1.0 - ((r - 0.6) / 0.1).clamp(0.0, 1.0));
            let dot = 1.0 - ((r - 0.025) / aa).clamp(0.0, 1.0);
            let mark = line.max(dot) * 0.9;
            let rim = ((r - 0.75) / 0.25).clamp(0.0, 1.0) * 0.85;
            // Over: the mark white, the rim black.
            let alpha = mark + rim * (1.0 - mark);
            let white = if alpha > 0.0 { mark / alpha } else { 0.0 };
            let v = (white * 255.0) as u8;
            let i = (y * N + x) * 4;
            data[i..i + 4].copy_from_slice(&[v, v, v, (alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_smooths_a_tremor_and_follows_a_swing() {
        let dt = 1.0 / 90.0;
        // A 10 Hz tremor of half a degree about a fixed aim: the steadied aim wobbles far less.
        let mut s = Steady::new(Quat::IDENTITY);
        let mut worst = 0.0f32;
        for i in 0..270 {
            let t = i as f32 * dt;
            let wobble = 0.5f32.to_radians() * (std::f32::consts::TAU * 10.0 * t).sin();
            s.follow(Quat::from_rotation_y(wobble), dt);
            if i > 90 {
                worst = worst.max(s.rot.angle_between(Quat::IDENTITY));
            }
        }
        assert!(worst < 0.2f32.to_radians(), "{}", worst.to_degrees());
        // A quick 60 degree swing: within half a degree a tenth of a second after it stops.
        let to = Quat::from_rotation_y(60f32.to_radians());
        for i in 0..=18 {
            s.follow(Quat::IDENTITY.slerp(to, (i as f32 / 18.0).min(1.0)), dt);
        }
        for _ in 0..9 {
            s.follow(to, dt);
        }
        assert!(
            s.rot.angle_between(to) < 0.5f32.to_radians(),
            "lag {}",
            s.rot.angle_between(to).to_degrees()
        );
    }
}

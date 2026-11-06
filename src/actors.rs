//! Players, bolts, grenades and sparks on screen, following `Game::snap`.

use std::collections::HashMap;

use bevy::prelude::*;
use lt_sim::sim::{EYE, PlayerSnap, TICK};
use lt_sim::{Hand, PickupKind, PlayerId, Pose, SimEvent, Team};

use crate::assets::{Model, Models};
use crate::game::{DrawSet, Game, GameEvent, LocalCmd, Mover, alpha};
use crate::materials::{AccentMaterial, GlowMaterial, NeonGlobals, Palette, team_color};

/// Shared meshes.
#[derive(Resource)]
pub struct ActorMeshes {
    bolt: Handle<Mesh>,
    spark: Handle<Mesh>,
    ring: Handle<Mesh>,
    /// The pulsing shell around a flying grenade.
    halo: Handle<Mesh>,
}

#[derive(Component)]
pub struct Avatar(pub PlayerId);

/// Parts of an avatar that follow the player.
struct AvatarParts {
    root: Entity,
    /// The floating chest, below the head, turned with the body.
    body: Entity,
    head: Entity,
    /// Floating gloves, left and right.
    hands: [Entity; 2],
    /// Chest lights (hits left).
    bars: Vec<(Entity, u8)>,
    gun: Entity,
    gun_hits: Vec<(Entity, u8)>,
    grenade: Entity,
    /// VR: small copies of the powerups the local player has, over the back of the free hand.
    icons: Vec<(PickupKind, Entity)>,
    team: Team,
}

#[derive(Resource, Default)]
struct Views {
    players: HashMap<PlayerId, AvatarParts>,
    bolts: HashMap<u32, Entity>,
    /// Grenades in flight: the model and its pulsing shell.
    grenades: HashMap<u32, (Entity, Entity)>,
    pickups: HashMap<u32, Entity>,
    seed: u64,
}

#[derive(Component)]
struct Spark {
    vel: Vec3,
    life: f32,
    max: f32,
    size: f32,
    gravity: bool,
}

/// A hit's flash: a glowing ball swelling and fading (`size` m across at most), or, with
/// `light`, a point light of that intensity flickering out.
#[derive(Component)]
struct Flash {
    life: f32,
    max: f32,
    size: f32,
    light: Option<f32>,
}

#[derive(Component)]
struct Ring {
    life: f32,
    max: f32,
    size: f32,
}

pub struct ActorsPlugin;

impl Plugin for ActorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Views>()
            .add_systems(Startup, make_meshes)
            .add_systems(
                Update,
                (
                    sync_players,
                    sync_bolts,
                    sync_grenades,
                    sync_pickups,
                    effects,
                    animate_sparks,
                )
                    .in_set(DrawSet)
                    .run_if(resource_exists::<Game>),
            );
    }
}

fn make_meshes(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    commands.insert_resource(ActorMeshes {
        bolt: meshes.add(Capsule3d::new(0.022, 0.75)),
        spark: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Torus::new(0.95, 1.0)),
        halo: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
    });
}

/// Spawns a model's parts as children of `parent`, with the game's materials for each part;
/// returns the parts named `hit_N` with their N.
pub fn spawn_model(
    commands: &mut Commands,
    parent: Entity,
    model: &Model,
    palette: &Palette,
    team: Option<Team>,
) -> Vec<(Entity, u8)> {
    let mut hits = Vec::new();
    for (mat, mesh) in &model.parts {
        let mut e = commands.spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, ChildOf(parent)));
        match mat.as_str() {
            "body" => {
                e.insert(MeshMaterial3d(palette.suit.clone()));
            }
            "fur_light" => {
                e.insert(MeshMaterial3d(palette.fur.clone()));
            }
            "glow" | "eye_glow" => {
                e.insert(MeshMaterial3d::<GlowMaterial>(palette.glow_for(team)));
            }
            "halo_white" => {
                e.insert(MeshMaterial3d::<GlowMaterial>(palette.glow_for(None)));
            }
            "eyes" => {
                e.insert(MeshMaterial3d::<AccentMaterial>(palette.eyes_for(team)));
            }
            "flow" => {
                e.insert(MeshMaterial3d::<AccentMaterial>(palette.flow_for(team)));
            }
            "glow_white" | "muzzle" => {
                e.insert(MeshMaterial3d::<AccentMaterial>(palette.accent_for(None)));
            }
            m => {
                e.insert(MeshMaterial3d::<AccentMaterial>(palette.accent_for(team)));
                if let Some(n) = m.strip_prefix("hit_").and_then(|n| n.parse().ok()) {
                    hits.push((e.id(), n));
                }
            }
        }
    }
    hits
}

fn spawn_avatar(
    commands: &mut Commands,
    p: &PlayerSnap,
    models: &Models,
    palette: &Palette,
) -> AvatarParts {
    let team = Some(p.team);
    let root = commands
        .spawn((
            Avatar(p.id),
            Transform::from_translation(p.feet),
            Visibility::default(),
        ))
        .id();
    // Floating parts: chest, head (an animal, the same for a player id everywhere), gloves.
    let body = commands
        .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
        .id();
    let bars = spawn_model(commands, body, &models.torso, palette, team);
    let head = commands
        .spawn((
            Transform::from_xyz(0.0, EYE, 0.0),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    let animal = &models.heads[p.id as usize % models.heads.len()];
    spawn_model(commands, head, animal, palette, team);
    let hands = [0, 1].map(|i| {
        let h = commands
            .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
            .id();
        spawn_model(commands, h, &models.hands[i], palette, team);
        h
    });
    let gun = commands
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    let gun_hits = spawn_model(commands, gun, &models.blaster, palette, team);
    let grenade = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden))
        .id();
    spawn_model(commands, grenade, &models.grenade, palette, None);
    let icons = PickupKind::POWERUPS
        .iter()
        .filter_map(|&k| {
            let model = models.powerups.get(&k)?;
            let e = commands
                .spawn((Transform::IDENTITY, Visibility::Hidden))
                .id();
            let inner = commands
                .spawn((
                    Transform::from_scale(Vec3::splat(ICON_SCALE)),
                    Visibility::default(),
                    ChildOf(e),
                ))
                .id();
            spawn_powerup(commands, inner, model, palette, k);
            Some((k, e))
        })
        .collect();
    AvatarParts {
        root,
        body,
        head,
        hands,
        bars,
        gun,
        gun_hits,
        grenade,
        icons,
        team: p.team,
    }
}

/// The VR powerup icons' size (of the pickups').
const ICON_SCALE: f32 = 0.14;

/// A bolt's light (lumens).
const BOLT_LIGHT: f32 = 60_000.0;

/// How far below the eyes the floating chest hangs.
const CHEST_DROP: f32 = 0.3;

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * t
}

#[allow(clippy::too_many_arguments)]
fn sync_players(
    mut commands: Commands,
    game: Res<Game>,
    mut views: ResMut<Views>,
    models: Res<Models>,
    palette: Res<Palette>,
    local: Res<LocalCmd>,
    time: Res<Time<Fixed>>,
    real: Res<Time>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    mover: Res<Mover>,
    xr: Option<Res<crate::vr::XrInput>>,
) {
    let vr = xr.is_some();
    let a = alpha(&time);
    let views = &mut *views;
    // A new arena: start over.
    if views.seed != game.level.seed {
        views.seed = game.level.seed;
        for (_, v) in views.players.drain() {
            commands.entity(v.root).despawn();
            commands.entity(v.gun).despawn();
            commands.entity(v.grenade).despawn();
            for (_, e) in v.icons {
                commands.entity(e).despawn();
            }
        }
    }
    let gone: Vec<PlayerId> = views
        .players
        .iter()
        .filter(|(id, v)| {
            !game
                .snap
                .players
                .iter()
                .any(|p| p.id == **id && p.team == v.team)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in gone {
        if let Some(v) = views.players.remove(&id) {
            commands.entity(v.root).despawn();
            commands.entity(v.gun).despawn();
            commands.entity(v.grenade).despawn();
            for (_, e) in v.icons {
                commands.entity(e).despawn();
            }
        }
    }
    let blink = (real.elapsed_secs() * 8.0).fract() < 0.5;
    for p in &game.snap.players {
        let v = views
            .players
            .entry(p.id)
            .or_insert_with(|| spawn_avatar(&mut commands, p, &models, &palette));
        let prev = game.prev.players.iter().find(|q| q.id == p.id);
        let feet = prev.map_or(p.feet, |q| q.feet.lerp(p.feet, a));
        let yaw = prev.map_or(p.yaw, |q| lerp_angle(q.yaw, p.yaw, a));
        let me = Some(p.id) == game.me;
        let alive = p.respawn.is_none();
        let set_vis = |vis: &mut Query<&mut Visibility>, e: Entity, on: bool| {
            if let Ok(mut v) = vis.get_mut(e) {
                *v = if on {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        };
        let mine = me && !local.autopilot && mover.valid;
        set_vis(&mut vis, v.root, alive);
        set_vis(&mut vis, v.body, !me);
        set_vis(&mut vis, v.head, !me);
        if let Ok(mut t) = transforms.get_mut(v.root) {
            t.translation = if mine { mover.feet } else { feet };
        }
        let head = if mine { local.cmd.head } else { p.head };
        if let Ok(mut t) = transforms.get_mut(v.body) {
            // Below the head, turned with the body, leaning a little with the head.
            t.translation = head.pos - Vec3::Y * CHEST_DROP;
            t.rotation = Quat::from_rotation_y(yaw);
        }
        if let Ok(mut t) = transforms.get_mut(v.head) {
            *t = Transform {
                translation: head.pos,
                rotation: head.rot,
                ..default()
            };
        }
        let remaining = 3u8.saturating_sub(p.hits);
        let lit = |k: u8| k <= remaining && (p.cooldown <= 0.0 || blink);
        for &(e, k) in &v.bars {
            set_vis(&mut vis, e, lit(k));
        }
        for &(e, k) in &v.gun_hits {
            set_vis(&mut vis, e, lit(k));
        }
        // Hands: the local player's straight from this frame's input (or the headset).
        let (hands, gun_hand, grenade_hand, base) = if me && !local.autopilot && mover.valid {
            (
                local.cmd.hands,
                local.cmd.gun_hand,
                local.cmd.grenade_hand,
                mover.feet,
            )
        } else {
            (p.hands, p.gun_hand, p.grenade_hand, feet)
        };
        let place = |t: &mut Transform, pose: &Pose| {
            t.translation = base + pose.pos;
            t.rotation = pose.rot;
        };
        // Gloves: everyone's, and one's own in VR (on the flat screen only the blaster shows).
        for (i, &h) in v.hands.iter().enumerate() {
            set_vis(&mut vis, h, !me || vr);
            if let Ok(mut t) = transforms.get_mut(h) {
                t.translation = hands[i].pos;
                t.rotation = hands[i].rot;
            }
        }
        set_vis(&mut vis, v.gun, alive && gun_hand.is_some());
        if let Some(h) = gun_hand
            && let Ok(mut t) = transforms.get_mut(v.gun)
        {
            place(&mut t, &hands[h.index()]);
        }
        set_vis(
            &mut vis,
            v.grenade,
            alive && grenade_hand.is_some() && p.grenades > 0,
        );
        if let Some(h) = grenade_hand
            && let Ok(mut t) = transforms.get_mut(v.grenade)
        {
            place(&mut t, &hands[h.index()]);
            t.translation += hands[h.index()].rot * Vec3::new(0.0, 0.02, -0.04);
        }
        // VR: the powerups one has, in a row over the back of the hand without the blaster.
        let free = match gun_hand {
            Some(Hand::Left) => Hand::Right,
            _ => Hand::Left,
        };
        let pose = hands[free.index()];
        let mut k = 0.0;
        for &(kind, e) in &v.icons {
            let on = me && vr && alive && p.has(kind);
            set_vis(&mut vis, e, on);
            if on && let Ok(mut t) = transforms.get_mut(e) {
                let spin = real.elapsed_secs() * 1.5 + k;
                t.translation =
                    base + pose.pos + pose.rot * Vec3::new((k - 0.5) * 0.06, 0.075, 0.03);
                t.rotation = pose.rot * Quat::from_rotation_y(spin);
                k += 1.0;
            }
        }
    }
}

fn sync_bolts(
    mut commands: Commands,
    game: Res<Game>,
    mut views: ResMut<Views>,
    meshes: Res<ActorMeshes>,
    palette: Res<Palette>,
    time: Res<Time<Fixed>>,
    mut q: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
) {
    let a = alpha(&time);
    let eyes: Vec<Vec3> = cams.iter().map(|c| c.translation()).collect();
    let views = &mut *views;
    views.bolts.retain(|id, e| {
        let keep = game.snap.bolts.iter().any(|b| b.id == *id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    for b in &game.snap.bolts {
        let dir = b.vel.normalize_or(Vec3::Y);
        let head = b.pos + b.vel * a * TICK;
        // The capsule trails behind the bolt's head; hidden right at the eye.
        let pos = head - dir * 0.4;
        let rot = Quat::from_rotation_arc(Vec3::Y, dir);
        let near = eyes.iter().any(|e| (*e - pos).length() < 0.9);
        match views.bolts.get(&b.id) {
            Some(&e) => {
                if let Ok(mut t) = q.get_mut(e) {
                    t.translation = pos;
                    t.rotation = rot;
                }
                if let Ok(mut v) = vis.get_mut(e) {
                    *v = if near {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                }
            }
            None => {
                let e = commands
                    .spawn((
                        Mesh3d(meshes.bolt.clone()),
                        MeshMaterial3d(palette.bolt_for(b.team)),
                        Transform {
                            translation: pos,
                            rotation: rot,
                            ..default()
                        },
                        if near {
                            Visibility::Hidden
                        } else {
                            Visibility::Inherited
                        },
                    ))
                    .id();
                // Its light on the walls around it (shown even while the capsule is hidden at
                // the eye; the GL renderer takes the nearest few, see glvr/extract.rs).
                let c = team_color(b.team);
                commands.spawn((
                    PointLight {
                        color: Color::linear_rgb(c.x, c.y, c.z),
                        intensity: BOLT_LIGHT,
                        range: 4.5,
                        radius: 0.03,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, 0.4, 0.0),
                    Visibility::Visible,
                    ChildOf(e),
                ));
                views.bolts.insert(b.id, e);
            }
        }
    }
}

/// Grenades in flight: the model, a white shell around it slowly pulsing (bigger and faster
/// once it hovers and fires), and a trail of white sparks.
#[allow(clippy::too_many_arguments)]
fn sync_grenades(
    mut commands: Commands,
    game: Res<Game>,
    mut views: ResMut<Views>,
    models: Res<Models>,
    palette: Res<Palette>,
    meshes: Res<ActorMeshes>,
    time: Res<Time<Fixed>>,
    real: Res<Time>,
    mut q: Query<&mut Transform>,
    mut trail: Local<f32>,
) {
    let a = alpha(&time);
    let views = &mut *views;
    views.grenades.retain(|id, (e, _)| {
        let keep = game.snap.grenades.iter().any(|g| g.id == *id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let t = real.elapsed_secs();
    // Trail sparks: so many a second for each grenade.
    *trail += real.delta_secs() * 40.0;
    let sparks = trail.floor() as usize;
    *trail -= sparks as f32;
    for g in &game.snap.grenades {
        let prev = game.prev.grenades.iter().find(|p| p.id == g.id);
        let pos = prev.map_or(g.pos, |p| p.pos.lerp(g.pos, a));
        let spin = prev.map_or(g.spin, |p| p.spin + (g.spin - p.spin) * a);
        let rot = if g.active {
            Quat::from_rotation_y(spin * 2.0)
        } else {
            Quat::from_rotation_x(spin * 3.0) * Quat::from_rotation_z(spin)
        };
        let scale = Vec3::splat(if g.active { 1.6 } else { 1.0 });
        let (root, shell) = *views.grenades.entry(g.id).or_insert_with(|| {
            let e = commands
                .spawn((Transform::from_translation(pos), Visibility::default()))
                .id();
            spawn_model(&mut commands, e, &models.grenade, &palette, None);
            let shell = commands
                .spawn((
                    Mesh3d(meshes.halo.clone()),
                    MeshMaterial3d(palette.glow_for(None)),
                    Transform::from_scale(Vec3::splat(0.2)),
                    ChildOf(e),
                ))
                .id();
            (e, shell)
        });
        if let Ok(mut tr) = q.get_mut(root) {
            *tr = Transform {
                translation: pos,
                rotation: rot,
                scale,
            };
        }
        if let Ok(mut tr) = q.get_mut(shell) {
            let phase = g.id as f32 * 0.9;
            let (rate, size, swing) = if g.active {
                (4.0, 0.2, 0.06)
            } else {
                (2.2, 0.15, 0.06)
            };
            let k = 0.5 + 0.5 * (t * rate + phase).sin();
            tr.scale = Vec3::splat(size + swing * k);
        }
        let seed = (g.id.wrapping_mul(2654435761) ^ (t * 1000.0) as u32).max(1);
        for i in 0..sparks {
            let mut s = seed.wrapping_add(i as u32 * 7919).max(1);
            let mut rnd = move || {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s as f32 / u32::MAX as f32) * 2.0 - 1.0
            };
            let vel = Vec3::new(rnd(), rnd() * 0.5 + 0.6, rnd()) * if g.active { 1.2 } else { 0.4 };
            let life = 0.35 + 0.25 * rnd().abs();
            commands.spawn((
                Spark {
                    vel,
                    life,
                    max: life,
                    size: 0.022,
                    gravity: false,
                },
                Mesh3d(meshes.spark.clone()),
                MeshMaterial3d(palette.bolt_for(None)),
                Transform::from_translation(pos + Vec3::new(rnd(), rnd(), rnd()) * 0.05)
                    .with_scale(Vec3::splat(0.022)),
            ));
        }
    }
}

/// A hit's flash at `at`: a glowing ball of `size` and a light of `intensity` (lumens)
/// reaching `range`, lasting `life` s.
#[allow(clippy::too_many_arguments)]
fn flash(
    commands: &mut Commands,
    meshes: &ActorMeshes,
    palette: &Palette,
    team: Option<Team>,
    at: Vec3,
    size: f32,
    intensity: f32,
    range: f32,
    life: f32,
) {
    commands.spawn((
        Flash {
            life,
            max: life,
            size,
            light: None,
        },
        Mesh3d(meshes.halo.clone()),
        MeshMaterial3d(palette.glow_for(team)),
        Transform::from_translation(at).with_scale(Vec3::splat(0.01)),
    ));
    let c = team_color(team);
    commands.spawn((
        Flash {
            life,
            max: life,
            size,
            light: Some(intensity),
        },
        PointLight {
            color: Color::linear_rgb(c.x * 0.7 + 0.3, c.y * 0.7 + 0.3, c.z * 0.7 + 0.3),
            intensity,
            range,
            radius: 0.1,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_translation(at),
    ));
}

/// Sparks that fly out further and live longer, without gravity at first (a hit's spray).
#[allow(clippy::too_many_arguments)]
fn spray(
    commands: &mut Commands,
    meshes: &ActorMeshes,
    mat: Handle<AccentMaterial>,
    at: Vec3,
    n: usize,
    speed: f32,
    size: f32,
    seed: u32,
) {
    let mut s = seed.wrapping_mul(2246822519).max(1);
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    for _ in 0..n {
        let dir = Vec3::new(rnd(), rnd(), rnd()).normalize_or(Vec3::Y);
        let life = 0.5 + 0.4 * (rnd() * 0.5 + 0.5);
        commands.spawn((
            Spark {
                vel: dir * speed * (0.6 + 0.4 * rnd().abs()),
                life,
                max: life,
                size,
                gravity: rnd() > 0.0,
            },
            Mesh3d(meshes.spark.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(size)),
        ));
    }
}

fn burst(
    commands: &mut Commands,
    meshes: &ActorMeshes,
    mat: Handle<AccentMaterial>,
    at: Vec3,
    n: usize,
    speed: f32,
    size: f32,
    seed: u32,
) {
    let mut s = seed.wrapping_mul(2654435761).max(1);
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    for _ in 0..n {
        let dir = Vec3::new(rnd(), rnd(), rnd()).normalize_or(Vec3::Y);
        let life = 0.25 + 0.25 * (rnd() * 0.5 + 0.5);
        commands.spawn((
            Spark {
                vel: dir * speed * (0.5 + 0.5 * rnd().abs()),
                life,
                max: life,
                size,
                gravity: true,
            },
            Mesh3d(meshes.spark.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(at).with_scale(Vec3::splat(size)),
        ));
    }
}

/// Spawns a powerup's model as children of `parent`: its accents in the powerup's colour.
pub fn spawn_powerup(
    commands: &mut Commands,
    parent: Entity,
    model: &Model,
    palette: &Palette,
    kind: PickupKind,
) {
    let (accent, _) = palette.pickup_for(kind);
    for (mat, mesh) in &model.parts {
        let mut e = commands.spawn((Mesh3d(mesh.clone()), Transform::IDENTITY, ChildOf(parent)));
        if mat == "body" {
            e.insert(MeshMaterial3d(palette.suit.clone()));
        } else {
            e.insert(MeshMaterial3d(accent.clone()));
        }
    }
}

/// Grenades and powerups lying in the arena to be picked up: bigger, turning and bobbing over a
/// ring of their colour (white for grenades).
#[allow(clippy::too_many_arguments)]
fn sync_pickups(
    mut commands: Commands,
    game: Res<Game>,
    mut views: ResMut<Views>,
    models: Res<Models>,
    palette: Res<Palette>,
    meshes: Res<ActorMeshes>,
    time: Res<Time>,
    mut q: Query<&mut Transform>,
) {
    let views = &mut *views;
    views.pickups.retain(|id, e| {
        let keep = game.snap.pickups.iter().any(|p| p.id == *id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let t = time.elapsed_secs();
    for p in &game.snap.pickups {
        let e = *views.pickups.entry(p.id).or_insert_with(|| {
            let root = commands
                .spawn((Transform::from_translation(p.pos), Visibility::default()))
                .id();
            let grenade = p.kind == PickupKind::Grenade;
            let spin = commands
                .spawn((
                    Transform::IDENTITY.with_scale(Vec3::splat(if grenade { 1.8 } else { 1.3 })),
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            match models.powerups.get(&p.kind) {
                Some(m) if !grenade => {
                    spawn_powerup(&mut commands, spin, m, &palette, p.kind);
                }
                _ => {
                    spawn_model(&mut commands, spin, &models.grenade, &palette, None);
                }
            }
            let (accent, _) = palette.pickup_for(p.kind);
            commands.spawn((
                Mesh3d(meshes.ring.clone()),
                MeshMaterial3d(accent),
                Transform::from_xyz(0.0, -lt_sim::sim::PICKUP_HEIGHT + 0.03, 0.0)
                    .with_scale(Vec3::new(0.32, 0.4, 0.32)),
                ChildOf(root),
            ));
            root
        });
        if let Ok(mut tr) = q.get_mut(e) {
            let phase = p.id as f32 * 1.7;
            tr.translation = p.pos + Vec3::Y * ((t * 2.0 + phase).sin() * 0.06);
            tr.rotation = Quat::from_rotation_y(t * 1.8 + phase);
        }
    }
}

fn effects(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    meshes: Res<ActorMeshes>,
    palette: Res<Palette>,
    mut neon: ResMut<NeonGlobals>,
) {
    let team_of = |id: PlayerId| {
        game.snap
            .players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.team)
    };
    for (k, GameEvent(e)) in events.read().enumerate() {
        let seed = game.snap.tick as u32 ^ (k as u32 * 977);
        match *e {
            SimEvent::BoltImpact { pos, team, .. } => {
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(team),
                    pos,
                    6,
                    3.0,
                    0.025,
                    seed,
                );
            }
            SimEvent::PickedUp { pos, .. } | SimEvent::PickupSpawned { pos, .. } => {
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(None),
                    pos,
                    14,
                    2.5,
                    0.025,
                    seed,
                );
                commands.spawn((
                    Ring {
                        life: 0.5,
                        max: 0.5,
                        size: 0.8,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(None)),
                    Transform::from_translation(pos),
                ));
            }
            SimEvent::Hit { pos, player, .. } => {
                let team = team_of(player);
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(team),
                    pos,
                    24,
                    5.0,
                    0.035,
                    seed,
                );
                spray(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(None),
                    pos,
                    14,
                    3.5,
                    0.025,
                    seed ^ 0x5bd1,
                );
                flash(
                    &mut commands,
                    &meshes,
                    &palette,
                    team,
                    pos,
                    0.5,
                    120_000.0,
                    5.0,
                    0.35,
                );
            }
            SimEvent::TaggedOut { pos, player, .. } => {
                let team = team_of(player);
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(team),
                    pos,
                    50,
                    7.0,
                    0.05,
                    seed,
                );
                commands.spawn((
                    Ring {
                        life: 0.8,
                        max: 0.8,
                        size: 3.0,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(team)),
                    Transform::from_translation(pos),
                ));
                neon.pulse(pos, team_color(team));
            }
            SimEvent::TargetHit { pos, target, .. } => {
                // In the target's colour: its alcove lights up and flickers.
                let team = game
                    .level
                    .targets
                    .iter()
                    .find(|t| t.id == target)
                    .map(|t| t.team);
                let centre = game
                    .level
                    .targets
                    .iter()
                    .find(|t| t.id == target)
                    .map_or(pos, |t| t.centre());
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(team),
                    pos,
                    40,
                    6.0,
                    0.045,
                    seed,
                );
                spray(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(None),
                    pos,
                    30,
                    5.0,
                    0.03,
                    seed ^ 0x5bd1,
                );
                flash(
                    &mut commands,
                    &meshes,
                    &palette,
                    team,
                    centre,
                    1.4,
                    1_500_000.0,
                    12.0,
                    1.0,
                );
                commands.spawn((
                    Ring {
                        life: 0.7,
                        max: 0.7,
                        size: 2.5,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(team)),
                    Transform::from_translation(centre)
                        .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ));
                neon.pulse(pos, team_color(team) * 2.0);
            }
            SimEvent::GrenadeArmed { pos } => {
                commands.spawn((
                    Ring {
                        life: 0.5,
                        max: 0.5,
                        size: 2.0,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(None)),
                    Transform::from_translation(pos),
                ));
            }
            SimEvent::Respawned { pos, player } => {
                commands.spawn((
                    Ring {
                        life: 0.7,
                        max: 0.7,
                        size: 1.2,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(team_of(player))),
                    Transform::from_translation(pos + Vec3::Y * 0.05),
                ));
            }
            _ => {}
        }
    }
}

#[allow(clippy::type_complexity)]
fn animate_sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut sparks: Query<(Entity, &mut Spark, &mut Transform), (Without<Ring>, Without<Flash>)>,
    mut rings: Query<(Entity, &mut Ring, &mut Transform), (Without<Spark>, Without<Flash>)>,
    mut flashes: Query<
        (Entity, &mut Flash, &mut Transform, Option<&mut PointLight>),
        (Without<Spark>, Without<Ring>),
    >,
) {
    let dt = time.delta_secs();
    for (e, mut f, mut t, light) in &mut flashes {
        f.life -= dt;
        if f.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let k = 1.0 - f.life / f.max;
        match (f.light, light) {
            (Some(i), Some(mut l)) => {
                // Bright at once, flickering as it dies away.
                let flicker = if (k * 7.0).fract() < 0.5 { 1.0 } else { 0.45 };
                l.intensity = i * (1.0 - k).powi(2) * flicker;
            }
            _ => t.scale = Vec3::splat(f.size * 0.5 * (k * 4.0).min(1.0) * (1.0 - k).sqrt()),
        }
    }
    for (e, mut s, mut t) in &mut sparks {
        s.life -= dt;
        if s.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if s.gravity {
            s.vel.y -= 6.0 * dt;
        }
        s.vel *= 1.0 - 2.0 * dt;
        t.translation += s.vel * dt;
        t.scale = Vec3::splat(s.size * (s.life / s.max));
    }
    for (e, mut r, mut t) in &mut rings {
        r.life -= dt;
        if r.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let k = 1.0 - r.life / r.max;
        let s = 0.1 + r.size * k;
        t.scale = Vec3::new(s, 0.3 * (1.0 - k) + 0.05, s);
    }
}

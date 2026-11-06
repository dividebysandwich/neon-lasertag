//! Players, bolts, grenades and sparks on screen, following `Game::snap`.

use std::collections::HashMap;

use bevy::prelude::*;
use lt_sim::sim::{CLOAK_FAR, CLOAK_NEAR, EYE, PlayerSnap, TICK};
use lt_sim::{PickupKind, PlayerId, Pose, SimEvent, Team};

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
    /// A shielded player's beam of light from above, and its material.
    beam: Handle<Mesh>,
    beam_mat: Handle<GlowMaterial>,
    /// A ring of the charge gun's coil.
    coil: Handle<Mesh>,
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
    /// The weapon in hand: the blaster or the shotgun (children of `gun`), and their hit
    /// lights.
    gun: Entity,
    blaster: Entity,
    shotgun: Entity,
    /// The charge gun: coils round the blaster's barrel, and a glow at its muzzle growing as
    /// it charges.
    coil: Entity,
    charge_glow: Entity,
    gun_hits: Vec<(Entity, u8)>,
    grenade: Entity,
    /// The shield's beam of light (a child of `root`), with its light on the player.
    beam: Entity,
    /// Every part twice: as it is (`solid`), and as a cloaked player's shimmer (`ghost`, in
    /// `ghost_mat`, faded with the distance).
    solid: Vec<Entity>,
    ghost: Vec<Entity>,
    ghost_mat: Handle<GlowMaterial>,
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

/// A charge gun beam's streak: thins out over its life.
#[derive(Component)]
struct Tracer {
    life: f32,
    max: f32,
    width: f32,
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
                    speed_trails,
                    fade_tracers,
                    shot_lights,
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

fn make_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
) {
    // Two nested cones of light from above the head down to the floor, and a pool of light
    // where they land.
    let mut beam = crate::assets::MeshData::default();
    let top = Vec3::Y * BEAM_HEIGHT;
    crate::scenery::cone(&mut beam, top, Vec3::NEG_Y, BEAM_HEIGHT, 0.3, 0.62, 24);
    crate::scenery::cone(&mut beam, top, Vec3::NEG_Y, BEAM_HEIGHT, 0.12, 0.3, 16);
    let c = crate::materials::pickup_color(PickupKind::Shield);
    let beam_mat = glows.add(GlowMaterial {
        neon: crate::materials::NeonUniform::fixed(c, 1.4).volume(),
    });
    commands.insert_resource(ActorMeshes {
        beam: meshes.add(beam.to_mesh()),
        beam_mat,
        bolt: meshes.add(Capsule3d::new(0.022, 0.75)),
        spark: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        ring: meshes.add(Torus::new(0.95, 1.0)),
        halo: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
        coil: meshes.add(Torus::new(0.026, 0.034)),
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

/// A cloaked player's shimmer: its colour and brightest intensity (up close).
const CLOAK_TINT: Vec3 = Vec3::new(0.6, 0.75, 1.0);
const CLOAK_GLOW: f32 = 2.2;

/// Spawns a model under `parent` twice (see `AvatarParts::solid`); returns the parts named
/// `hit_N` with their N.
#[allow(clippy::too_many_arguments)]
fn spawn_dual(
    commands: &mut Commands,
    parent: Entity,
    model: &Model,
    palette: &Palette,
    team: Option<Team>,
    ghost_mat: &Handle<GlowMaterial>,
    solid: &mut Vec<Entity>,
    ghost: &mut Vec<Entity>,
) -> Vec<(Entity, u8)> {
    let s = commands
        .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(parent)))
        .id();
    let hits = spawn_model(commands, s, model, palette, team);
    let g = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden, ChildOf(parent)))
        .id();
    for (mat, mesh) in &model.parts {
        // (Not the halos.)
        if matches!(mat.as_str(), "glow" | "eye_glow" | "halo_white") {
            continue;
        }
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(ghost_mat.clone()),
            Transform::IDENTITY,
            ChildOf(g),
        ));
    }
    solid.push(s);
    ghost.push(g);
    hits
}

fn spawn_avatar(
    commands: &mut Commands,
    p: &PlayerSnap,
    models: &Models,
    palette: &Palette,
    meshes: &ActorMeshes,
    glows: &mut Assets<GlowMaterial>,
) -> AvatarParts {
    let team = Some(p.team);
    let ghost_mat = glows.add(GlowMaterial {
        neon: crate::materials::NeonUniform::fixed(CLOAK_TINT, CLOAK_GLOW).cloak(),
    });
    let (mut solid, mut ghost) = (Vec::new(), Vec::new());
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
    let bars = spawn_dual(
        commands,
        body,
        &models.torso,
        palette,
        team,
        &ghost_mat,
        &mut solid,
        &mut ghost,
    );
    let head = commands
        .spawn((
            Transform::from_xyz(0.0, EYE, 0.0),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    let animal = &models.heads[p.id as usize % models.heads.len()];
    spawn_dual(
        commands, head, animal, palette, team, &ghost_mat, &mut solid, &mut ghost,
    );
    let hands = [0, 1].map(|i| {
        let h = commands
            .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
            .id();
        spawn_dual(
            commands,
            h,
            &models.hands[i],
            palette,
            team,
            &ghost_mat,
            &mut solid,
            &mut ghost,
        );
        h
    });
    let gun = commands
        .spawn((Transform::IDENTITY, Visibility::default()))
        .id();
    let blaster = commands
        .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(gun)))
        .id();
    let mut gun_hits = spawn_dual(
        commands,
        blaster,
        &models.blaster,
        palette,
        team,
        &ghost_mat,
        &mut solid,
        &mut ghost,
    );
    let shotgun = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden, ChildOf(gun)))
        .id();
    gun_hits.extend(spawn_dual(
        commands,
        shotgun,
        &models.shotgun,
        palette,
        team,
        &ghost_mat,
        &mut solid,
        &mut ghost,
    ));
    let coil = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden, ChildOf(blaster)))
        .id();
    let (charge_accent, charge_halo) = palette.pickup_for(PickupKind::Charge);
    for k in 0..3 {
        let at = models.muzzle * Vec3::new(1.0, 1.0, 0.5 + 0.16 * k as f32);
        let ring = commands
            .spawn((
                Mesh3d(meshes.coil.clone()),
                MeshMaterial3d(charge_accent.clone()),
                Transform::from_translation(at)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ChildOf(coil),
            ))
            .id();
        solid.push(ring);
    }
    let charge_glow = commands
        .spawn((
            Mesh3d(meshes.halo.clone()),
            MeshMaterial3d(charge_halo),
            Transform::from_translation(models.muzzle).with_scale(Vec3::ZERO),
            Visibility::Hidden,
            ChildOf(blaster),
        ))
        .id();
    let grenade = commands
        .spawn((Transform::IDENTITY, Visibility::Hidden))
        .id();
    spawn_model(commands, grenade, &models.grenade, palette, None);
    let beam = commands
        .spawn((
            Mesh3d(meshes.beam.clone()),
            MeshMaterial3d(meshes.beam_mat.clone()),
            Transform::IDENTITY,
            Visibility::Hidden,
            ChildOf(root),
        ))
        .id();
    let c = crate::materials::pickup_color(PickupKind::Shield);
    commands.spawn((
        PointLight {
            color: Color::linear_rgb(c.x, c.y, c.z),
            intensity: BEAM_LIGHT,
            range: 2.6,
            radius: 0.1,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.0, 2.2, 0.0),
        Visibility::Inherited,
        ChildOf(beam),
    ));
    AvatarParts {
        root,
        body,
        head,
        hands,
        bars,
        gun,
        blaster,
        shotgun,
        coil,
        charge_glow,
        gun_hits,
        grenade,
        beam,
        solid,
        ghost,
        ghost_mat,
        team: p.team,
    }
}

/// The shield's beam: from this high above the feet, and its light (lumens).
const BEAM_HEIGHT: f32 = 3.2;
const BEAM_LIGHT: f32 = 40_000.0;

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
    meshes: Res<ActorMeshes>,
    mut glows: ResMut<Assets<GlowMaterial>>,
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
        }
    }
    let blink = (real.elapsed_secs() * 8.0).fract() < 0.5;
    // Where the local player sees from.
    let viewer = if mover.valid {
        Some(mover.feet + local.cmd.head.pos)
    } else {
        game.me
            .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
            .map(|p| p.feet + p.head.pos)
    };
    for p in &game.snap.players {
        let v = views.players.entry(p.id).or_insert_with(|| {
            spawn_avatar(&mut commands, p, &models, &palette, &meshes, &mut glows)
        });
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
        set_vis(&mut vis, v.blaster, !p.shotgun);
        set_vis(&mut vis, v.shotgun, p.shotgun);
        set_vis(&mut vis, v.coil, p.charge);
        // Charging: the glow swells, and throbs once full.
        let full = p.charging >= game.rules.charge_time;
        let k = (p.charging / game.rules.charge_time).min(1.0);
        set_vis(&mut vis, v.charge_glow, p.charging > 0.0 && !p.cloaked());
        if let Ok(mut t) = transforms.get_mut(v.charge_glow) {
            let throb = if full {
                1.0 + 0.25 * (real.elapsed_secs() * 18.0).sin()
            } else {
                1.0
            };
            t.scale = Vec3::splat((0.015 + 0.05 * k) * throb);
        }
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
        // Cloaked: unseen from afar, a shimmer fading in closer (one's own gun and gloves
        // always shimmer).
        let cloaked = alive && p.cloaked();
        let seen = if !cloaked || me {
            1.0
        } else {
            let d = viewer.map_or(0.0, |e| e.distance(feet + Vec3::Y * 1.2));
            ((CLOAK_FAR - d) / (CLOAK_FAR - CLOAK_NEAR)).clamp(0.0, 1.0)
        };
        for &e in &v.solid {
            set_vis(&mut vis, e, !cloaked);
        }
        for &e in &v.ghost {
            set_vis(&mut vis, e, cloaked && seen > 0.01);
        }
        if cloaked && let Some(mut m) = glows.get_mut(&v.ghost_mat) {
            m.neon.color.w = CLOAK_GLOW * seen;
        }
        // The shield's beam, flickering in its last two seconds.
        let flicker = p.shield > 2.0 || (real.elapsed_secs() * 10.0).fract() < 0.6;
        // (Not one's own: from inside, it would tint everything one looks at; the HUD and the
        // VR hand's powerup icons tell.)
        set_vis(&mut vis, v.beam, alive && !me && p.shield > 0.0 && flicker);
    }
}

/// The shots nearest the eye for `NeonGlobals::shots` (in a blackout they light up the arena's
/// lights around them): the bolts in flight, and for a moment where one was fired or struck.
fn shot_lights(
    game: Res<Game>,
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    mut events: MessageReader<GameEvent>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut neon: ResMut<NeonGlobals>,
    mut flashes: Local<Vec<(Vec3, f32)>>,
) {
    const FLASH: f32 = 0.3;
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::Fired { pos, .. } | SimEvent::BoltImpact { pos, .. } => {
                flashes.push((pos, FLASH))
            }
            SimEvent::Hit { pos, .. } | SimEvent::Reflected { pos, .. } => {
                flashes.push((pos, FLASH))
            }
            _ => {}
        }
    }
    let dt = time.delta_secs();
    for f in flashes.iter_mut() {
        f.1 -= dt;
    }
    flashes.retain(|f| f.1 > 0.0);
    let a = alpha(&fixed);
    let mut lights: Vec<Vec4> = game
        .snap
        .bolts
        .iter()
        .map(|b| (b.pos + b.vel * a * TICK).extend(if b.beam { 1.5 } else { 1.0 }))
        .chain(flashes.iter().map(|f| f.0.extend(1.2 * f.1 / FLASH)))
        .collect();
    let eye = cams.iter().next().map_or(Vec3::ZERO, |c| c.translation());
    lights.sort_by(|a, b| {
        a.truncate()
            .distance_squared(eye)
            .total_cmp(&b.truncate().distance_squared(eye))
    });
    neon.shots = [Vec4::ZERO; crate::materials::SHOT_LIGHTS];
    for (slot, l) in neon.shots.iter_mut().zip(lights) {
        *slot = l;
    }
}

fn fade_tracers(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Tracer, &mut Transform)>,
) {
    for (e, mut t, mut tr) in &mut q {
        t.life -= time.delta_secs();
        if t.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let w = t.width * t.life / t.max;
        tr.scale.x = w;
        tr.scale.y = w;
    }
}

/// Speed-boosted players leave a short trail of fading motes behind them (not while cloaked).
fn speed_trails(
    mut commands: Commands,
    game: Res<Game>,
    meshes: Res<ActorMeshes>,
    palette: Res<Palette>,
    time: Res<Time>,
    mut acc: Local<f32>,
) {
    *acc += time.delta_secs();
    if *acc < 0.04 {
        return;
    }
    *acc = 0.0;
    let mat = palette.pickup_for(PickupKind::Speed).0;
    for p in &game.snap.players {
        if p.speed <= 0.0 || p.respawn.is_some() || p.cloaked() || Some(p.id) == game.me {
            continue;
        }
        for h in [0.35, 0.9, 1.4] {
            commands.spawn((
                Spark {
                    vel: Vec3::ZERO,
                    life: 0.35,
                    max: 0.35,
                    size: 0.045,
                    gravity: false,
                },
                Mesh3d(meshes.spark.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(p.feet + Vec3::Y * h).with_scale(Vec3::splat(0.045)),
            ));
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
        // The capsule trails behind the bolt's head (a beam's long and thick); hidden right at
        // the eye.
        let scale = if b.beam {
            Vec3::new(2.5, 6.0, 2.5)
        } else {
            Vec3::ONE
        };
        let pos = head - dir * 0.4 * scale.y;
        let rot = Quat::from_rotation_arc(Vec3::Y, dir);
        let near = eyes.iter().any(|e| (*e - pos).length() < 0.9);
        match views.bolts.get(&b.id) {
            Some(&e) => {
                if let Ok(mut t) = q.get_mut(e) {
                    t.translation = pos;
                    t.rotation = rot;
                    t.scale = scale;
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
                            scale,
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
                    // (The shotgun turns about its middle, not its grip.)
                    let inner = commands
                        .spawn((
                            Transform::from_xyz(
                                0.0,
                                0.0,
                                if p.kind == PickupKind::Shotgun {
                                    0.13
                                } else {
                                    0.0
                                },
                            ),
                            Visibility::default(),
                            ChildOf(spin),
                        ))
                        .id();
                    spawn_powerup(&mut commands, inner, m, &palette, p.kind);
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
            // A charge gun's beam: a streak of light along its way, fading.
            SimEvent::BeamFired { player, pos, dir } => {
                let reach = game.rules.beam_range;
                let end = game
                    .world
                    .ray(pos, pos + dir * reach)
                    .map_or(pos + dir * reach, |h| h.1);
                let len = pos.distance(end);
                commands.spawn((
                    Tracer {
                        life: 0.3,
                        max: 0.3,
                        width: 0.05,
                    },
                    Mesh3d(meshes.spark.clone()),
                    MeshMaterial3d(palette.bolt_for(team_of(player))),
                    Transform::from_translation((pos + end) / 2.0)
                        .with_rotation(Quat::from_rotation_arc(Vec3::Z, dir))
                        .with_scale(Vec3::new(0.05, 0.05, len)),
                ));
                flash(
                    &mut commands,
                    &meshes,
                    &palette,
                    team_of(player),
                    pos,
                    0.35,
                    60_000.0,
                    4.0,
                    0.25,
                );
            }
            SimEvent::Launched { pos, .. } => {
                commands.spawn((
                    Ring {
                        life: 0.5,
                        max: 0.5,
                        size: 1.4,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(None)),
                    Transform::from_translation(pos + Vec3::Y * 0.1),
                ));
            }
            SimEvent::Reflected { pos, team, normal } => {
                burst(
                    &mut commands,
                    &meshes,
                    palette.bolt_for(team),
                    pos,
                    8,
                    2.5,
                    0.022,
                    seed,
                );
                commands.spawn((
                    Ring {
                        life: 0.3,
                        max: 0.3,
                        size: 0.5,
                    },
                    Mesh3d(meshes.ring.clone()),
                    MeshMaterial3d(palette.bolt_for(team)),
                    Transform::from_translation(pos + normal * 0.03)
                        .with_rotation(Quat::from_rotation_arc(Vec3::Y, normal)),
                ));
            }
            SimEvent::Blocked { pos, .. } => {
                burst(
                    &mut commands,
                    &meshes,
                    palette.pickup_for(PickupKind::Shield).0,
                    pos,
                    10,
                    3.5,
                    0.03,
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

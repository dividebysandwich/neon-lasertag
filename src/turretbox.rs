//! Turret boxes (lt_sim::sim::BoxState): a black box with neon edges, white while it works for
//! nobody, then in its team's colour; a lid of two leaves hinged at its sides; inside, a turret
//! on a column that rises out of it, aiming where the snapshot says (a dark eye while it's
//! switched off, flickering white while it can't be shot yet).
//!
//! On the box's +z and -z sides, the sliding puzzle (lt_sim::puzzle): the square framed in the
//! box's colour, its pieces glowing tiles, each its own colour, gliding to where the snapshot
//! has them. The same puzzle shows on both sides, the one seen from behind the other (columns
//! run along +x on both). A piece glows brighter under the pointer (`PuzzleAim::hover`) and
//! flashes red when it runs into another one.
//!
//! Clicking: on the flat screen, the crosshair on a piece within reach and the fire button
//! (the blaster doesn't fire while aiming at a puzzle: `aim_at`); in VR, the index finger of a
//! hand not holding a gun pointing at a piece (a thin ray from the fingertip shows where, or the
//! fingertip pressed onto it), and that hand's trigger (`vr_touch`). A hand pointing at a puzzle
//! shows the pointing glove (src/actors.rs: `PuzzleAim::point`).

use std::f32::consts::PI;

use bevy::prelude::*;
use lt_sim::level::{BOX_H, BOX_HALF, BOX_HEAD, Level, PUZZLE_CELL, TurretBox};
use lt_sim::puzzle::{DEPTH, N, Puzzle};
use lt_sim::sim::{BoxPhase, BoxSnap, POKE_REACH, box_motion, look_rot};
use lt_sim::{Hand, SimEvent};

use crate::arena::ArenaPart;
use crate::game::{Game, GameEvent, LocalCmd, Mover};
use crate::materials::{AccentMaterial, NeonUniform, Palette, team_color};

/// The pointing glove's index fingertip in the glove's frame (left hand; the right's is
/// mirrored): tools/models/avatars.py's `POINT_TIP`.
const FINGERTIP: Vec3 = Vec3::new(-0.003, 0.031, -0.13);
/// VR: a hand points at a puzzle this far from its fingertip (m) ...
const POINT_REACH: f32 = 2.6;
/// ... and at one its fingertip is pressed this far into (m).
const TOUCH_BEHIND: f32 = 0.06;
/// Flat screen: the crosshair clicks a puzzle this close to the eyes (m).
const CLICK_REACH: f32 = 2.6;
/// How fast the pieces glide (cells a second).
const GLIDE: f32 = 9.0;
/// The pieces' colours, by their number.
const PIECE_COLORS: [Vec3; 8] = [
    Vec3::new(0.1, 0.9, 1.0),
    Vec3::new(1.0, 0.2, 0.85),
    Vec3::new(1.0, 0.85, 0.1),
    Vec3::new(0.25, 1.0, 0.3),
    Vec3::new(1.0, 0.5, 0.05),
    Vec3::new(0.6, 0.35, 1.0),
    Vec3::new(0.75, 1.0, 0.1),
    Vec3::new(0.1, 0.55, 1.0),
];
const DARK: Vec3 = Vec3::new(0.05, 0.05, 0.06);
const STUCK: Vec3 = Vec3::new(1.0, 0.1, 0.05);

pub struct TurretBoxPlugin;

impl Plugin for TurretBoxPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PuzzleAim>()
            .init_resource::<Puzzles>()
            .add_systems(
                Update,
                (on_events, animate, finger_rays)
                    .chain()
                    .in_set(crate::game::DrawSet)
                    .run_if(resource_exists::<Game>),
            );
    }
}

/// What the local player points at: the pieces under the crosshair or the fingertips (box,
/// piece), and per hand (VR) whether it points.
#[derive(Resource, Default)]
pub struct PuzzleAim {
    pub hover: Vec<(usize, usize)>,
    pub point: [bool; 2],
    /// VR: each pointing hand's ray, from its fingertip to where it meets the puzzle.
    pub ray: [Option<(Vec3, Vec3)>; 2],
}

/// The puzzles of the seeds in play (made again from a seed only when it changes).
#[derive(Resource, Default)]
pub struct Puzzles(Vec<(u32, Puzzle)>);

impl Puzzles {
    pub fn get(&mut self, seed: u32) -> &Puzzle {
        let i = match self.0.iter().position(|p| p.0 == seed) {
            Some(i) => i,
            None => {
                if self.0.len() >= 16 {
                    self.0.remove(0);
                }
                self.0.push((seed, Puzzle::generate(seed)));
                self.0.len() - 1
            }
        };
        &self.0[i].1
    }
}

/// Whether the box's pieces can be clicked now (not while the turret comes out or folds away).
fn clickable(s: &BoxSnap) -> bool {
    matches!(s.phase, BoxPhase::Closed | BoxPhase::Up)
}

/// Whether feet at `feet` are close enough to the box to click (as the sim has it).
fn in_reach(b: &TurretBox, feet: Vec3) -> bool {
    let (min, max) = b.bounds();
    let gap = Vec2::new(
        (min.x - feet.x).max(feet.x - max.x).max(0.0),
        (min.z - feet.z).max(feet.z - max.z).max(0.0),
    );
    gap.length() <= POKE_REACH && (feet.y - b.pos.y).abs() <= 0.8
}

/// The puzzle face `face` of a box: the z of its plane, and the way it looks out.
fn face_z(b: &TurretBox, face: usize) -> (f32, f32) {
    let n = if face == 0 { 1.0 } else { -1.0 };
    (b.pos.z + n * BOX_HALF, n)
}

/// The cell (column, row) at a point on a box's face, if on the puzzle (its strips too).
fn cell_on(b: &TurretBox, face: usize, p: Vec3) -> Option<(i8, i8)> {
    let (c, r) = b.cell_at(face, p);
    let lim = -(DEPTH as f32)..(N + DEPTH) as f32;
    (lim.contains(&c) && lim.contains(&r)).then(|| (c.floor() as i8, r.floor() as i8))
}

/// Flat screen: the puzzle under the crosshair (from the eyes at `eye` looking along `dir`),
/// within reach of the player's feet: its box, and the piece there if any.
pub fn aim_at(
    game: &Game,
    puzzles: &mut Puzzles,
    feet: Vec3,
    eye: Vec3,
    dir: Vec3,
) -> Option<(usize, Option<usize>)> {
    ray_at(game, puzzles, feet, eye, dir, CLICK_REACH).map(|(_, k, p)| (k, p))
}

/// The puzzle a ray from `from` along `dir` meets within `reach` (m), of a box within reach of
/// the player's feet: the point it meets, its box, and the piece there if any.
fn ray_at(
    game: &Game,
    puzzles: &mut Puzzles,
    feet: Vec3,
    from: Vec3,
    dir: Vec3,
    reach: f32,
) -> Option<(Vec3, usize, Option<usize>)> {
    let dir = dir.normalize_or_zero();
    let mut best: Option<(f32, Vec3, usize, Option<usize>)> = None;
    for (k, (b, s)) in game.level.boxes.iter().zip(&game.snap.boxes).enumerate() {
        if !clickable(s) || !in_reach(b, feet) {
            continue;
        }
        for face in 0..2 {
            let (z, n) = face_z(b, face);
            // In front of that side, looking at it.
            if (from.z - z) * n <= 0.0 || dir.z * n >= 0.0 {
                continue;
            }
            let t = (z - from.z) / dir.z;
            if !(0.0..reach).contains(&t) || best.is_some_and(|x| x.0 < t) {
                continue;
            }
            let p = from + dir * t;
            let Some((c, r)) = cell_on(b, face, p) else {
                continue;
            };
            // (Nothing else in the way.)
            if game.world.ray(from, p).is_some_and(|h| h.0 * t < t - 0.05) {
                continue;
            }
            let piece = puzzles.get(s.seed).piece_at(&s.pieces, c, r);
            best = Some((t, p, k, piece));
        }
    }
    best.map(|(_, p, k, piece)| (p, k, piece))
}

/// VR: which hands point at a puzzle and at which piece: a ray from the index fingertip along
/// the hand (from a little behind the tip, so a finger pressed onto a piece points at it too);
/// a trigger pulled on a hand pointing at a piece clicks it (and doesn't put up a shield or a
/// gadget).
#[allow(clippy::too_many_arguments)]
pub fn vr_touch(
    game: Res<Game>,
    input: Res<crate::vr::XrInput>,
    vr: Res<crate::vr::VrState>,
    mover: Res<Mover>,
    mut local: ResMut<LocalCmd>,
    mut aim: ResMut<PuzzleAim>,
    mut puzzles: ResMut<Puzzles>,
    mut pulled: Local<[bool; 2]>,
) {
    aim.hover.clear();
    aim.point = [false; 2];
    aim.ray = [None; 2];
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let alive = me.is_some_and(|p| p.respawn.is_none());
    let feet = if mover.valid && alive {
        mover.feet
    } else {
        me.map_or(Vec3::ZERO, |p| p.feet)
    };
    for i in 0..2 {
        let hand = if i == 0 { Hand::Left } else { Hand::Right };
        let trigger = input.hands[i].trigger;
        let was = pulled[i];
        pulled[i] = if was { trigger > 0.5 } else { trigger > 0.75 };
        let busy = vr.gun == Some(hand)
            || vr.gun2 == Some(hand)
            || local.cmd.grenade_hand == Some(hand)
            || input.hands[i].grip.is_none();
        if !alive || busy || vr.ui {
            continue;
        }
        let pose = local.cmd.hands[i];
        let tip_local = if i == 0 {
            FINGERTIP
        } else {
            FINGERTIP * Vec3::new(-1.0, 1.0, 1.0)
        };
        let tip = feet + pose.pos + pose.rot * tip_local;
        let dir = pose.rot * Vec3::NEG_Z;
        let from = tip - dir * TOUCH_BEHIND;
        let Some((at, k, piece)) = ray_at(
            &game,
            &mut puzzles,
            feet,
            from,
            dir,
            POINT_REACH + TOUCH_BEHIND,
        ) else {
            continue;
        };
        aim.point[i] = true;
        aim.ray[i] = Some((tip, at));
        if let Some(piece) = piece {
            aim.hover.push((k, piece));
            if pulled[i] && !was {
                local.cmd.pokes = local.cmd.pokes.wrapping_add(1);
                local.cmd.poke = (k as u8, piece as u8);
            }
        }
        if pulled[i] {
            // (The trigger is the puzzle's here, not the shield's or the gadget's.)
            local.cmd.shield = false;
            local.cmd.gadget = false;
        }
    }
}

/// VR: the pointing hands' rays, a thin beam from the fingertip to the puzzle and a dot where
/// it meets it.
fn finger_rays(
    mut commands: Commands,
    aim: Res<PuzzleAim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut rays: Local<Option<[(Entity, Entity); 2]>>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    if aim.ray.iter().all(Option::is_none) && rays.is_none() {
        return;
    }
    let rays = rays.get_or_insert_with(|| {
        let beam = meshes.add(Cylinder::new(0.0018, 1.0));
        let dot = meshes.add(Sphere::new(0.006).mesh().ico(2).unwrap());
        let mat = accent(&mut accents, Vec3::new(0.85, 0.95, 1.0), 2.5);
        [0, 1].map(|_| {
            let mut part = |mesh: &Handle<Mesh>| {
                commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::default(),
                        Visibility::Hidden,
                    ))
                    .id()
            };
            (part(&beam), part(&dot))
        })
    });
    for (i, &(beam, dot)) in rays.iter().enumerate() {
        let ray = aim.ray[i];
        if let Ok((mut t, mut v)) = q.get_mut(beam) {
            match ray {
                Some((from, to)) if from.distance(to) > 0.02 => {
                    let len = from.distance(to);
                    *t = Transform::from_translation((from + to) / 2.0)
                        .with_rotation(Quat::from_rotation_arc(Vec3::Y, (to - from) / len))
                        .with_scale(Vec3::new(1.0, len, 1.0));
                    *v = Visibility::Inherited;
                }
                _ => *v = Visibility::Hidden,
            }
        }
        if let Ok((mut t, mut v)) = q.get_mut(dot) {
            match ray {
                Some((_, to)) => {
                    *t = Transform::from_translation(to);
                    *v = Visibility::Inherited;
                }
                None => *v = Visibility::Hidden,
            }
        }
    }
}

#[derive(Component)]
struct BoxView {
    k: usize,
    /// The edges' and the puzzles' frames' colour.
    trim: Handle<AccentMaterial>,
    grid: Handle<AccentMaterial>,
    leaves: [Entity; 2],
    column: Entity,
    head: Entity,
    eye: Handle<AccentMaterial>,
    collar: Handle<AccentMaterial>,
    sight: Handle<AccentMaterial>,
    /// The puzzle shown (its seed) and its pieces.
    seed: Option<u32>,
    pieces: Vec<PieceView>,
    /// The lid and the turret as shown (eased towards the snapshot's).
    lid: f32,
    rise: f32,
    since_fired: f32,
}

struct PieceView {
    /// On the +z side and on the -z side.
    roots: [Entity; 2],
    mat: Handle<AccentMaterial>,
    color: Vec3,
    /// How far out it's shown (cells), and the red flash after running into another piece.
    shown: f32,
    flash: f32,
}

/// Meshes the pieces are made of (spawned again for every new puzzle).
#[derive(Resource)]
struct BoxMeshes {
    tile: Handle<Mesh>,
    bridge_x: Handle<Mesh>,
    bridge_y: Handle<Mesh>,
}

/// The lid's thickness, and each leaf's width: half the box and a little more, so a leaf
/// hanging down the box's side is clear of its neon edges.
const LID_T: f32 = 0.05;
const LEAF_W: f32 = BOX_HALF + 0.025;

/// Size of a piece's tile and how proud of the box's face it stands.
const TILE: f32 = PUZZLE_CELL - 0.018;
const PROUD: f32 = 0.014;

fn accent(accents: &mut Assets<AccentMaterial>, c: Vec3, i: f32) -> Handle<AccentMaterial> {
    accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(c, i),
    })
}

/// Spawns `level`'s turret boxes (as parts of the arena).
pub fn spawn(
    commands: &mut Commands,
    level: &Level,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    commands.insert_resource(BoxMeshes {
        tile: meshes.add(Cuboid::new(TILE, TILE, 0.02)),
        bridge_x: meshes.add(Cuboid::new(PUZZLE_CELL - TILE + 0.004, TILE * 0.6, 0.016)),
        bridge_y: meshes.add(Cuboid::new(TILE * 0.6, PUZZLE_CELL - TILE + 0.004, 0.016)),
    });
    if level.boxes.is_empty() {
        return;
    }
    let w = 2.0 * BOX_HALF;
    let lid_t = LID_T;
    let body = meshes.add(Cuboid::new(w, BOX_H - lid_t, w));
    let leaf = meshes.add(Cuboid::new(LEAF_W, lid_t, w));
    let post = meshes.add(Cuboid::new(0.035, BOX_H, 0.035));
    let rim_x = meshes.add(Cuboid::new(w, 0.03, 0.035));
    let rim_z = meshes.add(Cuboid::new(0.035, 0.03, w));
    let leaf_edge = meshes.add(Cuboid::new(0.025, lid_t + 0.012, w));
    let leaf_line = meshes.add(Cuboid::new(0.02, 0.006, w - 0.1));
    let column = meshes.add(Cylinder::new(0.09, 1.1));
    let column_band = meshes.add(Torus::new(0.085, 0.115));
    let ball = meshes.add(Sphere::new(BOX_HEAD - 0.03).mesh().ico(2).unwrap());
    let barrel = meshes.add(Cylinder::new(0.06, 0.55));
    let tip = meshes.add(Torus::new(0.055, 0.085));
    let collar = meshes.add(Torus::new(BOX_HEAD - 0.04, BOX_HEAD));
    let eye = meshes.add(Sphere::new(0.06));
    let sight = meshes.add(Cylinder::new(0.007, 2.4));
    let span = (N + 2 * DEPTH) as f32 * PUZZLE_CELL;
    let plate = meshes.add(Cuboid::new(span + 0.04, span + 0.04, 0.01));
    let square = N as f32 * PUZZLE_CELL;
    let frame_h = meshes.add(Cuboid::new(square + 0.03, 0.015, 0.012));
    let frame_v = meshes.add(Cuboid::new(0.015, square + 0.03, 0.012));
    let grid_h = meshes.add(Cuboid::new(square, 0.004, 0.006));
    let grid_v = meshes.add(Cuboid::new(0.004, square, 0.006));
    for (k, b) in level.boxes.iter().enumerate() {
        let trim = accent(accents, Vec3::ONE, 4.0);
        let grid = accent(accents, Vec3::ONE, 0.5);
        let root = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(b.pos),
                Visibility::default(),
                Name::new("turret box"),
            ))
            .id();
        let child = |commands: &mut Commands, mesh: &Handle<Mesh>, t: Transform| {
            commands
                .spawn((Mesh3d(mesh.clone()), t, ChildOf(root)))
                .id()
        };
        let e = child(
            commands,
            &body,
            Transform::from_xyz(0.0, (BOX_H - lid_t) / 2.0, 0.0),
        );
        commands
            .entity(e)
            .insert(MeshMaterial3d(palette.body.clone()));
        // The neon edges: up the corners, round the top and the foot.
        let o = BOX_HALF + 0.005;
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let e = child(
                commands,
                &post,
                Transform::from_xyz(sx * o, BOX_H / 2.0, sz * o),
            );
            commands.entity(e).insert(MeshMaterial3d(trim.clone()));
        }
        for y in [0.03, BOX_H - lid_t - 0.02] {
            for s in [-1.0, 1.0] {
                let e = child(commands, &rim_x, Transform::from_xyz(0.0, y, s * o));
                commands.entity(e).insert(MeshMaterial3d(trim.clone()));
                let e = child(commands, &rim_z, Transform::from_xyz(s * o, y, 0.0));
                commands.entity(e).insert(MeshMaterial3d(trim.clone()));
            }
        }
        // The lid's leaves (placed by `animate`), each with a glowing free edge and two lines on
        // its underside (outwards once it hangs down the box's side).
        let leaves = [0, 1].map(|_| {
            let l = commands
                .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
                .id();
            commands.spawn((
                Mesh3d(leaf.clone()),
                MeshMaterial3d(palette.body.clone()),
                Transform::IDENTITY,
                ChildOf(l),
            ));
            l
        });
        for (i, &l) in leaves.iter().enumerate() {
            // (Its free edge towards the box's middle: -x for the +x leaf.)
            let s = if i == 0 { -1.0 } else { 1.0 };
            commands.spawn((
                Mesh3d(leaf_edge.clone()),
                MeshMaterial3d(trim.clone()),
                Transform::from_xyz(s * (LEAF_W / 2.0 - 0.0125), 0.0, 0.0),
                ChildOf(l),
            ));
            for x in [0.12, LEAF_W - 0.12] {
                commands.spawn((
                    Mesh3d(leaf_line.clone()),
                    MeshMaterial3d(trim.clone()),
                    Transform::from_xyz(s * (LEAF_W / 2.0 - x), -lid_t / 2.0 - 0.003, 0.0),
                    ChildOf(l),
                ));
            }
        }
        // The turret: a column, and the head on it.
        let column_e = commands
            .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
            .id();
        commands.spawn((
            Mesh3d(column.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, -0.55, 0.0),
            ChildOf(column_e),
        ));
        for y in [-0.25, -0.6] {
            commands.spawn((
                Mesh3d(column_band.clone()),
                MeshMaterial3d(trim.clone()),
                Transform::from_xyz(0.0, y, 0.0),
                ChildOf(column_e),
            ));
        }
        let eye_mat = accent(accents, Vec3::ONE, 6.0);
        let collar_mat = accent(accents, Vec3::ONE, 5.0);
        let sight_mat = accent(accents, Vec3::ONE, 2.0);
        let head = commands
            .spawn((
                Transform::from_translation(b.head(0.0) - b.pos),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        let part = |commands: &mut Commands,
                    mesh: &Handle<Mesh>,
                    mat: MeshMaterial3d<AccentMaterial>,
                    t: Transform| {
            commands.spawn((Mesh3d(mesh.clone()), mat, t, ChildOf(head)));
        };
        commands.spawn((
            Mesh3d(ball.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::IDENTITY,
            ChildOf(head),
        ));
        commands.spawn((
            Mesh3d(barrel.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(0.0, 0.0, -0.36).with_rotation(Quat::from_rotation_x(PI / 2.0)),
            ChildOf(head),
        ));
        part(
            commands,
            &tip,
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_xyz(0.0, 0.0, -0.62).with_rotation(Quat::from_rotation_x(PI / 2.0)),
        );
        part(
            commands,
            &collar,
            MeshMaterial3d(collar_mat.clone()),
            Transform::from_rotation(Quat::from_rotation_x(PI / 2.0)),
        );
        part(
            commands,
            &eye,
            MeshMaterial3d(eye_mat.clone()),
            Transform::from_xyz(0.0, 0.12, -0.15),
        );
        part(
            commands,
            &sight,
            MeshMaterial3d(sight_mat.clone()),
            Transform::from_xyz(0.0, 0.0, -1.85).with_rotation(Quat::from_rotation_x(PI / 2.0)),
        );
        // The puzzles' plates and frames, on both sides.
        for face in 0..2 {
            let (mid, n) = b.panel(face);
            let at = |d: Vec3| Transform::from_translation(mid - b.pos + d);
            let e = child(commands, &plate, at(n * 0.005));
            commands
                .entity(e)
                .insert(MeshMaterial3d(palette.suit.clone()));
            let centre = b.cell_pos(face, (N as f32 - 1.0) / 2.0, (N as f32 - 1.0) / 2.0) - mid;
            let half = square / 2.0 + 0.008;
            for s in [-1.0, 1.0] {
                let e = child(
                    commands,
                    &frame_h,
                    at(centre + Vec3::Y * s * half + n * 0.012),
                );
                commands.entity(e).insert(MeshMaterial3d(trim.clone()));
                let e = child(
                    commands,
                    &frame_v,
                    at(centre + Vec3::X * s * half + n * 0.012),
                );
                commands.entity(e).insert(MeshMaterial3d(trim.clone()));
            }
            for i in 1..N {
                let d = (i as f32 - N as f32 / 2.0) * PUZZLE_CELL;
                let e = child(commands, &grid_h, at(centre + Vec3::Y * d + n * 0.011));
                commands.entity(e).insert(MeshMaterial3d(grid.clone()));
                let e = child(commands, &grid_v, at(centre + Vec3::X * d + n * 0.011));
                commands.entity(e).insert(MeshMaterial3d(grid.clone()));
            }
        }
        commands.entity(root).insert(BoxView {
            k,
            trim,
            grid,
            leaves,
            column: column_e,
            head,
            eye: eye_mat,
            collar: collar_mat,
            sight: sight_mat,
            seed: None,
            pieces: Vec::new(),
            lid: 0.0,
            rise: 0.0,
            since_fired: 99.0,
        });
    }
}

/// A new puzzle's pieces on both sides of box `b` (children of `root`).
fn spawn_pieces(
    commands: &mut Commands,
    root: Entity,
    b: &TurretBox,
    puzzle: &Puzzle,
    meshes: &BoxMeshes,
    accents: &mut Assets<AccentMaterial>,
) -> Vec<PieceView> {
    puzzle
        .pieces
        .iter()
        .enumerate()
        .map(|(j, p)| {
            let color = PIECE_COLORS[j % PIECE_COLORS.len()];
            let mat = accent(accents, color, 1.2);
            let roots = [0, 1].map(|face| {
                let (_, n) = b.panel(face);
                let r = commands
                    .spawn((Transform::IDENTITY, Visibility::default(), ChildOf(root)))
                    .id();
                for &(c, row) in &p.cells {
                    let at = b.cell_pos(face, c as f32, row as f32) - b.pos + n * PROUD;
                    commands.spawn((
                        Mesh3d(meshes.tile.clone()),
                        MeshMaterial3d(mat.clone()),
                        Transform::from_translation(at),
                        ChildOf(r),
                    ));
                    // Joined to the next cell of the piece to the right and above, so it reads
                    // as one piece.
                    for (dc, dr, mesh) in [(1, 0, &meshes.bridge_x), (0, 1, &meshes.bridge_y)] {
                        if p.cells.contains(&(c + dc, row + dr)) {
                            let d = Vec3::new(dc as f32, dr as f32, 0.0) * PUZZLE_CELL / 2.0;
                            commands.spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(mat.clone()),
                                Transform::from_translation(at + d),
                                ChildOf(r),
                            ));
                        }
                    }
                }
                r
            });
            PieceView {
                roots,
                mat,
                color,
                shown: p.out as f32,
                flash: 0.0,
            }
        })
        .collect()
}

fn on_events(mut events: MessageReader<GameEvent>, mut views: Query<&mut BoxView>) {
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::PieceSlid {
                turret_box,
                piece,
                slide: lt_sim::PuzzleSlide::Stuck | lt_sim::PuzzleSlide::Blocked,
                ..
            } => {
                for mut v in views.iter_mut().filter(|v| v.k == turret_box as usize) {
                    if let Some(p) = v.pieces.get_mut(piece as usize) {
                        p.flash = 1.0;
                    }
                }
            }
            SimEvent::BoxFired { turret_box, .. } => {
                for mut v in views.iter_mut().filter(|v| v.k == turret_box as usize) {
                    v.since_fired = 0.0;
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn animate(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    aim: Res<PuzzleAim>,
    mut puzzles: ResMut<Puzzles>,
    box_meshes: Option<Res<BoxMeshes>>,
    mut views: Query<(Entity, &mut BoxView)>,
    mut transforms: Query<&mut Transform>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    let Some(box_meshes) = box_meshes else { return };
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (root, mut v) in &mut views {
        let k = v.k;
        let (Some(b), Some(s)) = (game.level.boxes.get(k), game.snap.boxes.get(k)) else {
            continue;
        };
        // A new puzzle: its pieces anew.
        if v.seed != Some(s.seed) {
            for p in v.pieces.drain(..) {
                for r in p.roots {
                    commands.entity(r).despawn();
                }
            }
            let puzzle = puzzles.get(s.seed);
            v.pieces = spawn_pieces(&mut commands, root, b, puzzle, &box_meshes, &mut accents);
            v.seed = Some(s.seed);
        }
        let color = team_color(s.owner);
        // The edges and the frames: the team's colour, breathing; brighter while it's out.
        let out = s.phase != BoxPhase::Closed;
        let breathe = 0.8 + 0.2 * (t * 2.5 + k as f32).sin();
        if let Some(mut m) = accents.get_mut(&v.trim) {
            m.neon.color = color.extend(if out { 5.0 } else { 3.5 } * breathe);
        }
        if let Some(mut m) = accents.get_mut(&v.grid) {
            m.neon.color = color.extend(0.5);
        }
        // The lid and the turret, eased towards the snapshot.
        let (lid, rise) = box_motion(s.phase, s.t);
        let ease = 1.0 - (-dt * 25.0).exp();
        v.lid += (lid - v.lid) * ease;
        v.rise += (rise - v.rise) * ease;
        for (i, &l) in v.leaves.iter().enumerate() {
            // Hinged just outside the box's top edges: up, over and all the way down, to hang
            // flat against the box's side (three quarters of a turn).
            let side = if i == 0 { 1.0 } else { -1.0 };
            let a = v.lid * 1.5 * PI;
            let rot = Quat::from_rotation_z(-side * a);
            let pivot = Vec3::new(side * LEAF_W, BOX_H, 0.0);
            let centre = Vec3::new(-side * LEAF_W / 2.0, -LID_T / 2.0, 0.0);
            if let Ok(mut tr) = transforms.get_mut(l) {
                tr.translation = pivot + rot * centre;
                tr.rotation = rot;
            }
        }
        let head = b.head(v.rise) - b.pos;
        if let Ok(mut tr) = transforms.get_mut(v.column) {
            tr.translation = head;
        }
        if let Ok(mut tr) = transforms.get_mut(v.head) {
            tr.translation = head;
            if s.aim.is_finite() && s.aim.length_squared() > 0.5 && v.rise > 0.01 {
                tr.rotation = tr.rotation.slerp(look_rot(s.aim), (dt * 15.0).min(1.0));
            }
        }
        v.since_fired += dt;
        let kick = (1.0 - v.since_fired / 0.15).max(0.0);
        let on = s.phase == BoxPhase::Up && s.off <= 0.0;
        // (White, flickering, while it can't be shot yet.)
        let guard = s.phase != BoxPhase::Up || s.guard > 0.0;
        let flicker = if guard && (t * 14.0).sin() > 0.0 {
            Vec3::ONE
        } else {
            color
        };
        if let Some(mut m) = accents.get_mut(&v.eye) {
            m.neon.color = if on {
                flicker.extend(6.0 + 10.0 * kick)
            } else {
                DARK.extend(1.0)
            };
        }
        if let Some(mut m) = accents.get_mut(&v.collar) {
            m.neon.color = if s.off > 0.0 {
                let left = (s.off / game.rules.box_off.max(0.1)).clamp(0.0, 1.0);
                color.extend(0.3 + 1.2 * (1.0 - left) * (0.5 + 0.5 * (t * 8.0).sin()))
            } else {
                flicker.extend(4.0)
            };
        }
        if let Some(mut m) = accents.get_mut(&v.sight) {
            m.neon.color = if on && s.phase == BoxPhase::Up {
                color.extend(1.6 + 0.6 * (t * 11.0 + k as f32).sin())
            } else {
                DARK.extend(0.0)
            };
        }
        // The pieces, gliding to where they are.
        let Some(puzzle) = v.seed.map(|seed| puzzles.get(seed).clone()) else {
            continue;
        };
        let clickable = clickable(s);
        for (j, (p, q)) in v.pieces.iter_mut().zip(&puzzle.pieces).enumerate() {
            let want = s.pieces.get(j).copied().unwrap_or(q.out) as f32;
            let d = want - p.shown;
            p.shown += d.signum() * d.abs().min(GLIDE * dt);
            p.flash = (p.flash - dt * 2.5).max(0.0);
            let (dc, dr) = q.outward();
            let off = Vec3::new(dc as f32, dr as f32, 0.0) * p.shown * PUZZLE_CELL;
            for &r in &p.roots {
                if let Ok(mut tr) = transforms.get_mut(r) {
                    tr.translation = off;
                }
            }
            let hover = aim.hover.contains(&(k, j));
            let home = want == 0.0;
            if let Some(mut m) = accents.get_mut(&p.mat) {
                let mut c = p.color;
                let mut i = if home { 1.6 } else { 1.0 };
                if !clickable {
                    i *= 0.5;
                }
                if hover {
                    c = c.lerp(Vec3::ONE, 0.35);
                    i = 3.5 + 0.8 * (t * 10.0).sin();
                }
                c = c.lerp(STUCK, p.flash);
                i += p.flash * 3.0;
                m.neon.color = c.extend(i);
            }
        }
    }
}

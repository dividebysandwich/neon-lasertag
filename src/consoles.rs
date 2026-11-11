//! Consoles (lt_sim::console): a black pedestal with neon edges (white while it works for
//! nobody, then in its team's colour), a hologram of what it does turning over it, and on its
//! faces a click puzzle (lt_sim::minigame) drawn from the snapshot's seed and state. The
//! puzzles glow on their own (fixed neon, not the arena's dimmable lighting), so they read in a
//! blackout too. While a console rests (no clicks), its puzzle and hologram are dim.
//!
//! Clicking works as on the turret boxes (src/turretbox.rs: `ray_at`, `vr_touch`): a console's
//! puzzle is `console::CONSOLE_POKE` plus its index in the clicks and in `PuzzleAim::hover`.

use std::f32::consts::TAU;

use bevy::prelude::*;
use lt_sim::SimEvent;
use lt_sim::console::{CONSOLE_H, CONSOLE_HALF, ConsoleDef, ConsoleKind, PanelFrame};
use lt_sim::minigame::Minigame;

use crate::arena::ArenaPart;
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, NeonUniform, Palette, team_color};
use crate::turretbox::{PIECE_COLORS, PuzzleAim, Puzzles};

pub struct ConsolePlugin;

impl Plugin for ConsolePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (on_events, animate)
                .chain()
                .in_set(crate::game::DrawSet)
                .run_if(resource_exists::<Game>),
        );
    }
}

/// How fast sliding pieces glide (cells a second).
const GLIDE: f32 = 9.0;
const STUCK: Vec3 = Vec3::new(1.0, 0.1, 0.05);
const DARK: Vec3 = Vec3::new(0.05, 0.05, 0.06);
/// Circuit wires with current through them, and without.
const WIRE_ON: Vec3 = Vec3::new(0.15, 1.0, 0.9);
const WIRE_OFF: Vec3 = Vec3::new(0.35, 0.4, 0.75);
/// Lights on, and off.
const LIGHT_ON: Vec3 = Vec3::new(1.0, 0.25, 0.8);
const LIGHT_OFF: Vec3 = Vec3::new(0.3, 0.32, 0.45);

/// The colour of what a console does (its hologram).
pub fn kind_color(kind: ConsoleKind) -> Vec3 {
    match kind {
        ConsoleKind::Vault => Vec3::new(1.0, 0.75, 0.1),
        ConsoleKind::Mast => Vec3::new(0.2, 1.0, 0.55),
        ConsoleKind::Beacon => Vec3::new(0.7, 0.45, 1.0),
        ConsoleKind::Breaker => Vec3::new(1.0, 0.35, 0.1),
        ConsoleKind::Bridge => Vec3::new(0.1, 0.85, 1.0),
        ConsoleKind::Secondary(_) => Vec3::new(1.0, 0.5, 0.05),
    }
}

/// The banner for solving a console of `kind`: its title and what it did (none for a secondary
/// target's: its charge has its own).
pub fn solved_banner(kind: ConsoleKind) -> Option<(&'static str, &'static str)> {
    Some(match kind {
        ConsoleKind::Secondary(_) => return None,
        ConsoleKind::Vault => (
            "SUPPLY VAULT CRACKED",
            "A SPECIAL WEAPON IS YOURS - IT RESETS FOR A MINUTE",
        ),
        ConsoleKind::Mast => (
            "SENSOR MAST ONLINE",
            "ENEMIES NEAR IT SHOW THROUGH WALLS FOR 20 S",
        ),
        ConsoleKind::Beacon => (
            "FORWARD BEACON TAKEN",
            "YOUR TEAM RESPAWNS HERE WHILE NO ENEMY IS NEAR",
        ),
        ConsoleKind::Breaker => (
            "BREAKER THROWN",
            "THIS QUARTER OF THE ARENA GOES DARK FOR 20 S",
        ),
        ConsoleKind::Bridge => (
            "LIGHT BRIDGE UP",
            "A WALKWAY OVER THE ARENA FOR 25 S - SHOOT DOWN FROM IT (ANYONE CAN USE IT)",
        ),
    })
}

#[derive(Component)]
struct ConsoleView {
    k: usize,
    trim: Handle<AccentMaterial>,
    icon: Entity,
    icon_mat: Handle<AccentMaterial>,
    /// Along the top's edges: bars filling up while it resets (hidden while it can be
    /// solved).
    charge: Vec<Entity>,
    charge_mat: Handle<AccentMaterial>,
    /// A forward beacon's column of light up from it (in its team's colour while it has one).
    beam: Option<(Entity, Handle<AccentMaterial>)>,
    /// A light bridge's bridge (scaled along its length as it comes and goes), its surface's
    /// and its edges' materials, and how far out it's shown (0..1).
    bridge: Option<BridgeView>,
    /// The puzzle shown (its seed), its root on each face and its elements.
    seed: Option<u32>,
    roots: Vec<Entity>,
    elems: Vec<ElemView>,
}

struct BridgeView {
    root: Entity,
    /// Along x or z (its root's local x is along it).
    surface: Handle<AccentMaterial>,
    edges: Handle<AccentMaterial>,
    shown: f32,
}

struct ElemView {
    /// Its root on each face.
    roots: Vec<Entity>,
    mat: Handle<AccentMaterial>,
    color: Vec3,
    /// How far it's shown along its way (sliding: cells out), and the red flash after a
    /// blocked click.
    shown: f32,
    flash: f32,
}

fn accent(accents: &mut Assets<AccentMaterial>, c: Vec3, i: f32) -> Handle<AccentMaterial> {
    accents.add(AccentMaterial {
        overlay: false,
        neon: NeonUniform::fixed(c, i),
    })
}

/// A console's puzzles (as the sim has them) for the level and rules in play.
pub fn defs(game: &Game) -> Vec<ConsoleDef> {
    lt_sim::console::console_defs(&game.level, &game.rules)
}

/// Spawns the consoles of the match in play (as parts of the arena).
pub fn spawn(
    commands: &mut Commands,
    game: &Game,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
) {
    let defs = defs(game);
    let w = 2.0 * CONSOLE_HALF;
    let body = meshes.add(Cuboid::new(w - 0.1, CONSOLE_H - 0.06, w - 0.1));
    let cap = meshes.add(Cuboid::new(w, 0.06, w));
    let post = meshes.add(Cuboid::new(0.035, CONSOLE_H, 0.035));
    let rim_x = meshes.add(Cuboid::new(w, 0.03, 0.035));
    let rim_z = meshes.add(Cuboid::new(0.035, 0.03, w));
    let vault = meshes.add(Cuboid::new(0.28, 0.2, 0.2));
    let vault_band = meshes.add(Cuboid::new(0.3, 0.035, 0.22));
    let charge_bar = meshes.add(Cuboid::new(1.0, 0.03, 0.03));
    let mast_pole = meshes.add(Cylinder::new(0.02, 0.5));
    let mast_dish = meshes.add(Torus::new(0.11, 0.14));
    let mast_tip = meshes.add(Sphere::new(0.04).mesh().ico(2).unwrap());
    let beacon_core = meshes.add(Cylinder::new(0.05, 0.42));
    let beacon_ring = meshes.add(Torus::new(0.1, 0.125));
    let beam_mesh = meshes.add(Cylinder::new(0.035, 1.0));
    let breaker_box = meshes.add(Cuboid::new(0.26, 0.3, 0.08));
    let bridge_icon = meshes.add(Cuboid::new(0.4, 0.04, 0.14));
    let bridge_pier = meshes.add(Cuboid::new(0.05, 0.16, 0.05));
    let breaker_lever = meshes.add(Cuboid::new(0.04, 0.22, 0.04));
    let breaker_knob = meshes.add(Cuboid::new(0.12, 0.05, 0.06));
    for (k, d) in defs.iter().enumerate() {
        let trim = accent(accents, Vec3::ONE, 4.0);
        let icon_mat = accent(accents, kind_color(d.kind), 4.0);
        let root = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(d.pos),
                Visibility::default(),
                Name::new("console"),
            ))
            .id();
        let part = |commands: &mut Commands, mesh: &Handle<Mesh>, mat, t: Transform| {
            commands.spawn((Mesh3d(mesh.clone()), mat, t, ChildOf(root)));
        };
        // (Wall panels, the secondary targets', have no pedestal.)
        let pedestal = d.nfaces == 2;
        if pedestal {
            part(
                commands,
                &body,
                MeshMaterial3d(palette.body.clone()),
                Transform::from_xyz(0.0, (CONSOLE_H - 0.06) / 2.0, 0.0),
            );
            commands.spawn((
                Mesh3d(cap.clone()),
                MeshMaterial3d(palette.suit.clone()),
                Transform::from_xyz(0.0, CONSOLE_H - 0.03, 0.0),
                ChildOf(root),
            ));
            let o = CONSOLE_HALF - 0.04;
            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                commands.spawn((
                    Mesh3d(post.clone()),
                    MeshMaterial3d(trim.clone()),
                    Transform::from_xyz(sx * o, CONSOLE_H / 2.0, sz * o),
                    ChildOf(root),
                ));
            }
            for y in [0.03, CONSOLE_H - 0.07] {
                for s in [-1.0, 1.0] {
                    commands.spawn((
                        Mesh3d(rim_x.clone()),
                        MeshMaterial3d(trim.clone()),
                        Transform::from_xyz(0.0, y, s * (o + 0.02)),
                        ChildOf(root),
                    ));
                    commands.spawn((
                        Mesh3d(rim_z.clone()),
                        MeshMaterial3d(trim.clone()),
                        Transform::from_xyz(s * (o + 0.02), y, 0.0),
                        ChildOf(root),
                    ));
                }
            }
        }
        // The hologram over it.
        let icon = commands
            .spawn((
                Transform::from_xyz(0.0, CONSOLE_H + 0.45, 0.0),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        let icon_part = |commands: &mut Commands, mesh: &Handle<Mesh>, t: Transform| {
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(icon_mat.clone()),
                t,
                ChildOf(icon),
            ));
        };
        match d.kind {
            ConsoleKind::Vault => {
                icon_part(commands, &vault, Transform::IDENTITY);
                for y in [-0.07, 0.07] {
                    icon_part(commands, &vault_band, Transform::from_xyz(0.0, y, 0.0));
                }
            }
            ConsoleKind::Mast => {
                icon_part(commands, &mast_pole, Transform::IDENTITY);
                for (y, s) in [(0.05, 1.0), (0.2, 0.7)] {
                    icon_part(
                        commands,
                        &mast_dish,
                        Transform::from_xyz(0.0, y, 0.0)
                            .with_rotation(Quat::from_rotation_x(0.5))
                            .with_scale(Vec3::splat(s)),
                    );
                }
                icon_part(commands, &mast_tip, Transform::from_xyz(0.0, 0.28, 0.0));
            }
            ConsoleKind::Secondary(_) => {}
            ConsoleKind::Bridge => {
                icon_part(commands, &bridge_icon, Transform::from_xyz(0.0, 0.08, 0.0));
                for x in [-0.17, 0.17] {
                    icon_part(commands, &bridge_pier, Transform::from_xyz(x, -0.04, 0.0));
                }
            }
            ConsoleKind::Breaker => {
                icon_part(commands, &breaker_box, Transform::IDENTITY);
                let tilt = Quat::from_rotation_x(-0.6);
                icon_part(
                    commands,
                    &breaker_lever,
                    Transform::from_translation(tilt * Vec3::new(0.0, 0.1, 0.0) + Vec3::Z * 0.06)
                        .with_rotation(tilt),
                );
                icon_part(
                    commands,
                    &breaker_knob,
                    Transform::from_translation(tilt * Vec3::new(0.0, 0.21, 0.0) + Vec3::Z * 0.06)
                        .with_rotation(tilt),
                );
            }
            ConsoleKind::Beacon => {
                icon_part(commands, &beacon_core, Transform::IDENTITY);
                for y in [-0.12, 0.0, 0.12] {
                    icon_part(commands, &beacon_ring, Transform::from_xyz(0.0, y, 0.0));
                }
            }
        }
        if !pedestal {
            commands.entity(icon).insert(Visibility::Hidden);
        }
        // A beacon's column of light, up to 8 m.
        let beam = (d.kind == ConsoleKind::Beacon).then(|| {
            // (Up to the ceiling, if there's one.)
            let from = d.pos + Vec3::Y * (CONSOLE_H + 0.7);
            let len = game
                .world
                .ray(from, from + Vec3::Y * 8.0)
                .map_or(8.0, |h| h.0 * 8.0);
            let m = accent(accents, kind_color(d.kind), 0.0);
            let e = commands
                .spawn((
                    Mesh3d(beam_mesh.clone()),
                    MeshMaterial3d(m.clone()),
                    Transform::from_xyz(0.0, CONSOLE_H + 0.7 + len / 2.0, 0.0)
                        .with_scale(Vec3::new(1.0, len, 1.0)),
                    Visibility::Hidden,
                    ChildOf(root),
                ))
                .id();
            (e, m)
        });
        // A light bridge's bridge: a slab of hard light with bright edges (hidden until up).
        let bridge = lt_sim::console::bridge_for(&game.level, d).map(|b| {
            let size = b.max - b.min;
            let along_x = size.x >= size.z;
            let (len, w) = if along_x {
                (size.x, size.z)
            } else {
                (size.z, size.x)
            };
            let surface = accent(accents, kind_color(d.kind), 0.0);
            let edges = accent(accents, kind_color(d.kind), 0.0);
            let rot = if along_x {
                Quat::IDENTITY
            } else {
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
            };
            let root = commands
                .spawn((
                    ArenaPart,
                    Transform::from_translation(b.mid()).with_rotation(rot),
                    Visibility::Hidden,
                    Name::new("light bridge"),
                ))
                .id();
            let slab = meshes.add(Cuboid::new(len, size.y, w));
            commands.spawn((
                Mesh3d(slab),
                MeshMaterial3d(surface.clone()),
                Transform::IDENTITY,
                ChildOf(root),
            ));
            // Its sides: an edge strip, and a low rail on posts (`BRIDGE_RAIL`, to shoot down
            // over).
            let rail = meshes.add(Cuboid::new(len, 0.05, 0.05));
            let post = meshes.add(Cuboid::new(0.04, lt_sim::console::BRIDGE_RAIL, 0.04));
            let posts = (len / 1.5).round().max(1.0) as usize;
            for s in [-1.0, 1.0] {
                let z = s * (w / 2.0 - 0.03);
                for y in [
                    size.y / 2.0 + 0.02,
                    size.y / 2.0 + lt_sim::console::BRIDGE_RAIL,
                ] {
                    commands.spawn((
                        Mesh3d(rail.clone()),
                        MeshMaterial3d(edges.clone()),
                        Transform::from_xyz(0.0, y, z),
                        ChildOf(root),
                    ));
                }
                for i in 0..=posts {
                    commands.spawn((
                        Mesh3d(post.clone()),
                        MeshMaterial3d(edges.clone()),
                        Transform::from_xyz(
                            (i as f32 / posts as f32 - 0.5) * (len - 0.05),
                            size.y / 2.0 + lt_sim::console::BRIDGE_RAIL / 2.0,
                            z,
                        ),
                        ChildOf(root),
                    ));
                }
            }
            // Crossbars every metre or so.
            let bar = meshes.add(Cuboid::new(0.04, 0.02, w - 0.1));
            let n = (len / 1.0).round().max(1.0) as usize;
            for i in 0..=n {
                commands.spawn((
                    Mesh3d(bar.clone()),
                    MeshMaterial3d(edges.clone()),
                    Transform::from_xyz(
                        (i as f32 / n as f32 - 0.5) * (len - 0.1),
                        size.y / 2.0 + 0.012,
                        0.0,
                    ),
                    ChildOf(root),
                ));
            }
            BridgeView {
                root,
                surface,
                edges,
                shown: 0.0,
            }
        });
        // The reset bars: along the top's +z and -z edges, over the puzzles.
        let charge_mat = accent(accents, kind_color(d.kind), 3.0);
        let charge: Vec<Entity> = if pedestal {
            [1.0, -1.0]
                .map(|s| {
                    commands
                        .spawn((
                            Mesh3d(charge_bar.clone()),
                            MeshMaterial3d(charge_mat.clone()),
                            Transform::from_xyz(0.0, CONSOLE_H + 0.02, s * (CONSOLE_HALF - 0.05)),
                            Visibility::Hidden,
                            ChildOf(root),
                        ))
                        .id()
                })
                .to_vec()
        } else {
            Vec::new()
        };
        commands.entity(root).insert(ConsoleView {
            k,
            trim,
            icon,
            icon_mat,
            charge,
            charge_mat,
            beam,
            bridge,
            seed: None,
            roots: Vec::new(),
            elems: Vec::new(),
        });
    }
}

/// A transform at `p` on `frame` (the face's own way round: +z out of it).
fn on_face(frame: &PanelFrame, p: Vec3) -> Transform {
    let rot = Quat::from_mat3(&Mat3::from_cols(frame.right, frame.up, frame.normal));
    Transform::from_translation(p).with_rotation(rot)
}

/// A new puzzle on every face of console `d` (children of `root`, which stands at `d.pos`):
/// each face's root, and the elements.
#[allow(clippy::too_many_arguments)]
fn spawn_puzzle(
    commands: &mut Commands,
    root: Entity,
    d: &ConsoleDef,
    game: &Minigame,
    palette: &Palette,
    meshes: &mut Assets<Mesh>,
    accents: &mut Assets<AccentMaterial>,
    trim: &Handle<AccentMaterial>,
) -> (Vec<Entity>, Vec<ElemView>) {
    let (n, cell) = (game.n(), game.cell());
    let ext = game.extent();
    let span = (ext.end - ext.start) as f32 * cell;
    let square = n as f32 * cell;
    let plate = meshes.add(Cuboid::new(span + 0.04, span + 0.04, 0.01));
    let frame_h = meshes.add(Cuboid::new(square + 0.03, 0.015, 0.012));
    let frame_v = meshes.add(Cuboid::new(0.015, square + 0.03, 0.012));
    let tile = meshes.add(Cuboid::new(cell - 0.018, cell - 0.018, 0.02));
    let bridge_x = meshes.add(Cuboid::new(0.022, (cell - 0.018) * 0.6, 0.016));
    let bridge_y = meshes.add(Cuboid::new((cell - 0.018) * 0.6, 0.022, 0.016));
    // A circuit tile's wire from its middle to one edge (up), its hub, and its backing.
    let wire = meshes.add(Cuboid::new(0.03, cell / 2.0 + 0.015, 0.012));
    let hub = meshes.add(Cuboid::new(0.05, 0.05, 0.014));
    let backing = meshes.add(Cuboid::new(cell - 0.012, cell - 0.012, 0.006));
    let mut roots = Vec::new();
    let mut elems: Vec<ElemView> = (0..game.elements())
        .map(|j| {
            let color = PIECE_COLORS[j % PIECE_COLORS.len()];
            ElemView {
                roots: Vec::new(),
                mat: accent(accents, color, 1.2),
                color,
                shown: 0.0,
                flash: 0.0,
            }
        })
        .collect();
    for f in d.faces() {
        // The face's root: on the face, its own axes (cells along x and y, out along z).
        let mid = f.cell_pos(n, cell, (n as f32 - 1.0) / 2.0, (n as f32 - 1.0) / 2.0);
        let face_root = commands
            .spawn((
                on_face(f, mid - d.pos),
                Visibility::default(),
                ChildOf(root),
            ))
            .id();
        roots.push(face_root);
        let span_mid = (ext.start + ext.end) as f32 / 2.0 - n as f32 / 2.0;
        commands.spawn((
            Mesh3d(plate.clone()),
            MeshMaterial3d(palette.suit.clone()),
            Transform::from_xyz(span_mid * cell, span_mid * cell, 0.005),
            ChildOf(face_root),
        ));
        let half = square / 2.0 + 0.008;
        for s in [-1.0, 1.0] {
            commands.spawn((
                Mesh3d(frame_h.clone()),
                MeshMaterial3d(trim.clone()),
                Transform::from_xyz(0.0, s * half, 0.012),
                ChildOf(face_root),
            ));
            commands.spawn((
                Mesh3d(frame_v.clone()),
                MeshMaterial3d(trim.clone()),
                Transform::from_xyz(s * half, 0.0, 0.012),
                ChildOf(face_root),
            ));
        }
        // A cell's middle on the face root, from the square's middle.
        let at = |c: f32, r: f32| {
            Vec3::new(
                (c + 0.5 - n as f32 / 2.0) * cell,
                (r + 0.5 - n as f32 / 2.0) * cell,
                0.0,
            )
        };
        match game {
            Minigame::Slide(p) => {
                for (e, q) in elems.iter_mut().zip(&p.pieces) {
                    let r = commands
                        .spawn((
                            Transform::IDENTITY,
                            Visibility::default(),
                            ChildOf(face_root),
                        ))
                        .id();
                    for &(c, row) in &q.cells {
                        let pos = at(c as f32, row as f32) + Vec3::Z * 0.014;
                        commands.spawn((
                            Mesh3d(tile.clone()),
                            MeshMaterial3d(e.mat.clone()),
                            Transform::from_translation(pos),
                            ChildOf(r),
                        ));
                        for (dc, dr, mesh) in [(1, 0, &bridge_x), (0, 1, &bridge_y)] {
                            if q.cells.contains(&(c + dc, row + dr)) {
                                let off = Vec3::new(dc as f32, dr as f32, 0.0) * cell / 2.0;
                                commands.spawn((
                                    Mesh3d(mesh.clone()),
                                    MeshMaterial3d(e.mat.clone()),
                                    Transform::from_translation(pos + off),
                                    ChildOf(r),
                                ));
                            }
                        }
                    }
                    e.roots.push(r);
                    e.shown = q.out as f32;
                }
            }
            Minigame::Circuit(cc) => {
                use lt_sim::minigame::circuit::{LEFT, RIGHT, UP, turned};
                for (i, e) in elems.iter_mut().enumerate() {
                    let (c, r) = (i % n, i / n);
                    // (Turned about its middle by `animate`.)
                    let tile_root = commands
                        .spawn((
                            Transform::from_translation(at(c as f32, r as f32)),
                            Visibility::default(),
                            ChildOf(face_root),
                        ))
                        .id();
                    commands.spawn((
                        Mesh3d(backing.clone()),
                        MeshMaterial3d(palette.body.clone()),
                        Transform::from_xyz(0.0, 0.0, 0.008),
                        ChildOf(tile_root),
                    ));
                    commands.spawn((
                        Mesh3d(hub.clone()),
                        MeshMaterial3d(e.mat.clone()),
                        Transform::from_xyz(0.0, 0.0, 0.016),
                        ChildOf(tile_root),
                    ));
                    for k in 0..4u8 {
                        if cc.shapes[i] & turned(UP, k) == 0 {
                            continue;
                        }
                        // (A quarter turn clockwise each: -90 degrees about the face's out.)
                        let rot = Quat::from_rotation_z(-(k as f32) * std::f32::consts::FRAC_PI_2);
                        commands.spawn((
                            Mesh3d(wire.clone()),
                            MeshMaterial3d(e.mat.clone()),
                            Transform::from_translation(rot * Vec3::new(0.0, cell / 4.0, 0.016))
                                .with_rotation(rot),
                            ChildOf(tile_root),
                        ));
                    }
                    e.roots.push(tile_root);
                    e.color = WIRE_OFF;
                }
                // The current's way in (left edge) and out (right edge): fixed stubs.
                // (Spawned once per face, below.)
                let stub = meshes.add(Cuboid::new(cell * 0.6, 0.04, 0.014));
                let ends = [
                    (at(-0.8, cc.source as f32), LEFT, WIRE_ON),
                    (
                        at(n as f32 - 0.2, cc.sink as f32),
                        RIGHT,
                        Vec3::new(1.0, 0.85, 0.1),
                    ),
                ];
                for (pos, _, color) in ends {
                    let m = accent(accents, color, 3.0);
                    commands.spawn((
                        Mesh3d(stub.clone()),
                        MeshMaterial3d(m),
                        Transform::from_translation(pos + Vec3::Z * 0.016),
                        ChildOf(face_root),
                    ));
                }
            }
            Minigame::Lights(_) => {
                for (i, e) in elems.iter_mut().enumerate() {
                    let (c, r) = (i % n, i / n);
                    let t = commands
                        .spawn((
                            Mesh3d(tile.clone()),
                            MeshMaterial3d(e.mat.clone()),
                            Transform::from_translation(at(c as f32, r as f32) + Vec3::Z * 0.014),
                            ChildOf(face_root),
                        ))
                        .id();
                    e.roots.push(t);
                    e.color = LIGHT_OFF;
                }
            }
        }
    }
    (roots, elems)
}

fn on_events(mut events: MessageReader<GameEvent>, mut views: Query<&mut ConsoleView>) {
    for GameEvent(e) in events.read() {
        if let SimEvent::ConsolePoked {
            console,
            element,
            slide: lt_sim::PuzzleSlide::Stuck | lt_sim::PuzzleSlide::Blocked,
            ..
        } = *e
        {
            for mut v in views.iter_mut().filter(|v| v.k == console as usize) {
                if let Some(p) = v.elems.get_mut(element as usize) {
                    p.flash = 1.0;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn animate(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    aim: Res<PuzzleAim>,
    palette: Res<Palette>,
    mut puzzles: ResMut<Puzzles>,
    mut views: Query<(Entity, &mut ConsoleView)>,
    mut transforms: Query<&mut Transform>,
    mut shown: Query<&mut Visibility>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut accents: ResMut<Assets<AccentMaterial>>,
) {
    if views.is_empty() {
        return;
    }
    let defs = defs(&game);
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (root, mut v) in &mut views {
        let k = v.k;
        let (Some(d), Some(s)) = (defs.get(k), game.snap.consoles.get(k)) else {
            continue;
        };
        if v.seed != Some(s.seed) {
            for r in v.roots.drain(..) {
                commands.entity(r).despawn();
            }
            let g = puzzles.game(s.seed).clone();
            let trim = v.trim.clone();
            let (roots, elems) = spawn_puzzle(
                &mut commands,
                root,
                d,
                &g,
                &palette,
                &mut meshes,
                &mut accents,
                &trim,
            );
            v.roots = roots;
            v.elems = elems;
            v.seed = Some(s.seed);
        }
        let open = s.open();
        // Resetting: the bars fill up from the middle.
        let rest = d.kind.rest().max(0.1);
        let full = (1.0 - s.cooldown / rest).clamp(0.0, 1.0);
        for &e in &v.charge {
            if let Ok(mut vis) = shown.get_mut(e) {
                vis.set_if_neq(if open {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                });
            }
            if let Ok(mut tr) = transforms.get_mut(e) {
                tr.scale.x = (2.0 * CONSOLE_HALF - 0.1) * full.max(0.01);
            }
        }
        if let Some(mut m) = accents.get_mut(&v.charge_mat) {
            m.neon.color = kind_color(d.kind).extend(2.0 + 1.5 * (t * 4.0).sin().abs());
        }
        let color = team_color(s.owner);
        let breathe = 0.8 + 0.2 * (t * 2.5 + k as f32).sin();
        if let Some(mut m) = accents.get_mut(&v.trim) {
            m.neon.color = color.extend(if open { 4.0 * breathe } else { 1.2 });
        }
        // The hologram: turning, bobbing, bright while it works or can be solved.
        if let Ok(mut tr) = transforms.get_mut(v.icon) {
            tr.translation.y = CONSOLE_H + 0.45 + 0.04 * (t * 1.7 + k as f32).sin();
            tr.rotation = Quat::from_rotation_y((t * 0.9 + k as f32) % TAU);
        }
        if let Some(mut m) = accents.get_mut(&v.icon_mat) {
            // (Its team's colour while it works for one: a beacon always does once taken.)
            let works = s.owner.is_some() && (s.active > 0.0 || d.kind == ConsoleKind::Beacon);
            let c = match s.owner {
                Some(team) if works => team_color(Some(team)),
                _ => kind_color(d.kind),
            };
            m.neon.color = if open || works {
                c.extend(4.0 + 1.5 * (t * 3.0).sin())
            } else {
                c.extend(0.6)
            };
        }
        if let Some(b) = &mut v.bridge {
            // Coming out from the middle in a moment, going the same way; flickering for the
            // last two seconds.
            let up = s.active > 0.0;
            b.shown = if up {
                (b.shown + dt * 3.0).min(1.0)
            } else {
                (b.shown - dt * 4.0).max(0.0)
            };
            let flicker = up && s.active < 2.0 && (t * 18.0).sin() < -0.2;
            if let Ok(mut vis) = shown.get_mut(b.root) {
                vis.set_if_neq(if b.shown > 0.001 && !flicker {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            if let Ok(mut tr) = transforms.get_mut(b.root) {
                tr.scale.x = b.shown.max(0.001);
            }
            let c = kind_color(d.kind);
            if let Some(mut m) = accents.get_mut(&b.surface) {
                m.neon.color = c.extend(0.35 + 0.1 * (t * 3.0).sin());
            }
            if let Some(mut m) = accents.get_mut(&b.edges) {
                m.neon.color = c.extend(4.0);
            }
        }
        if let Some((e, m)) = &v.beam {
            if let Ok(mut vis) = shown.get_mut(*e) {
                vis.set_if_neq(if s.owner.is_some() {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            if let Some(mut m) = accents.get_mut(m) {
                m.neon.color = team_color(s.owner).extend(2.5 + 0.8 * (t * 2.0).sin());
            }
        }
        let g = puzzles.game(s.seed).clone();
        let poke = (lt_sim::console::CONSOLE_POKE as usize + k, 0);
        let mut powered: Option<Vec<bool>> = None;
        for (j, e) in v.elems.iter_mut().enumerate() {
            let state = s.state.get(j).copied().unwrap_or(0);
            e.flash = (e.flash - dt * 2.5).max(0.0);
            let mut lit = 1.0;
            match &g {
                Minigame::Slide(p) => {
                    let q = &p.pieces[j.min(p.pieces.len() - 1)];
                    let want = state as f32;
                    let dd = want - e.shown;
                    e.shown += dd.signum() * dd.abs().min(GLIDE * dt);
                    let (dc, dr) = q.outward();
                    let off = Vec3::new(dc as f32, dr as f32, 0.0) * e.shown * g.cell();
                    for &r in &e.roots {
                        if let Ok(mut tr) = transforms.get_mut(r) {
                            tr.translation = off;
                        }
                    }
                    if want == 0.0 {
                        lit = 1.6;
                    }
                }
                Minigame::Circuit(cc) => {
                    // Turning (always clockwise: from 3 on to 0 is a quarter turn more).
                    let want = state as f32;
                    if want < e.shown - 0.5 {
                        e.shown -= 4.0;
                    }
                    e.shown = (e.shown + dt * 8.0).min(want);
                    let rot = Quat::from_rotation_z(-e.shown * std::f32::consts::FRAC_PI_2);
                    for &r in &e.roots {
                        if let Ok(mut tr) = transforms.get_mut(r) {
                            tr.rotation = rot;
                        }
                    }
                    let on = powered.get_or_insert_with(|| cc.powered(&s.state));
                    if on.get(j).copied().unwrap_or(false) {
                        e.color = WIRE_ON;
                        lit = 3.0;
                    } else {
                        e.color = WIRE_OFF;
                        lit = 0.9;
                    }
                }
                Minigame::Lights(_) => {
                    if state != 0 {
                        e.color = LIGHT_ON;
                        lit = 3.5;
                    } else {
                        e.color = LIGHT_OFF;
                        lit = 0.3;
                    }
                }
            }
            let hover = aim.hover.contains(&(poke.0, j));
            if let Some(mut m) = accents.get_mut(&e.mat) {
                let mut c = e.color;
                let mut i = lit;
                if !open {
                    c = c.lerp(DARK, 0.5);
                    i *= 0.35;
                } else if hover {
                    c = c.lerp(Vec3::ONE, 0.35);
                    i = 3.5 + 0.8 * (t * 10.0).sin();
                }
                c = c.lerp(STUCK, e.flash);
                i += e.flash * 3.0;
                m.neon.color = c.extend(i);
            }
        }
    }
}

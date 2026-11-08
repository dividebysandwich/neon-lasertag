//! Structures: bigger set pieces of the arena that leave the 2 m grid's right angles behind:
//! round and octagonal rooms, towers with spiral stairs, a colonnade, and the Megatarget
//! mode's rooms. Each is a `Design`: upright prisms over convex polygons (`Solid`, collided
//! as `Collider::Prism`), its own nav nodes and links, and its spotlights (`Lamp`).
//!
//! Designs come in sets by size (`Class`): small ones fit in 3 x 3 cells, medium ones in
//! 4 x 4, large ones in 5 x 5. The arena generator (level.rs) puts them on the grid either as a
//! mirrored pair (one in each half, turned half way round each other like the rest of the
//! arena) or, for the designs that look the same turned half way round (`symmetric`), one in
//! the arena's centre. The cells under a structure are `Ground::Struct` (no grid nodes); the
//! grid cells outside its doorways are linked to its own nodes (`StructPlace::ports`).
//!
//! The geometry is the single source for the models too: `tools/models/structures.json` (read
//! by the Blender builder, tools/models/structures.py) is generated from it by the
//! `structures_json_is_current` test (`LT_WRITE_STRUCTURES=1 cargo test -p lt-sim structures`
//! rewrites it). The builder decorates each solid by its `Tag` (neon bands on wall tops,
//! door jambs and slits, stair noses, deck edges), so the models always match the colliders.
//!
//! Local frame: metres, Y up, origin in the middle of the structure on the floor; angles in
//! degrees from +X towards +Z. Coordinates are snapped to 0.1 mm so every machine builds the
//! same arena whatever its trigonometry rounds to.

use std::sync::OnceLock;

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::collide::{Collider, World};
use crate::kit::{FLOOR_T, RAIL_H, STOREY};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StructKind {
    /// Small: a round bunker, roofed, two doorways and firing slits all round.
    Pillbox,
    /// Small: an octagonal hut, roofed, two doorways and windows.
    OctoHut,
    /// Small: a round tower, one doorway, a spiral stair up to a crenellated top with a
    /// spotlight.
    Watchtower,
    /// Medium: a round tower, two doorways, two stairs along its wall up to a crenellated top
    /// with two spotlights.
    RoundTower,
    /// Medium: an octagonal court open to the sky, four doorways, windows, a central pillar and
    /// low cover walls.
    OctoCourt,
    /// Large: a round tower with two stairs up to its top at the upper floor's height, and a
    /// spiral stair round its core on up to a vantage ring a storey higher, four spotlights.
    VantageTower,
    /// Large: a ring of columns under a round lintel, low curved walls inside around a dais,
    /// spotlights on it.
    Rotunda,
    /// The Megatarget mode's room: octagonal.
    MegaOctagon,
    /// The Megatarget mode's room: round, four columns inside.
    MegaRotunda,
    /// The Megatarget mode's room: hexagonal.
    MegaHexagon,
    /// The Megatarget mode's room: a long octagon with two low blocks inside.
    MegaLens,
}

/// A structure's size set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Class {
    Small,
    Medium,
    Large,
    /// The Megatarget mode's rooms (in the middle of its arena).
    Mega,
}

impl StructKind {
    pub const ALL: [StructKind; 11] = [
        StructKind::Pillbox,
        StructKind::OctoHut,
        StructKind::Watchtower,
        StructKind::RoundTower,
        StructKind::OctoCourt,
        StructKind::VantageTower,
        StructKind::Rotunda,
        StructKind::MegaOctagon,
        StructKind::MegaRotunda,
        StructKind::MegaHexagon,
        StructKind::MegaLens,
    ];

    pub const MEGA_ROOMS: [StructKind; 4] = [
        StructKind::MegaOctagon,
        StructKind::MegaRotunda,
        StructKind::MegaHexagon,
        StructKind::MegaLens,
    ];

    /// The mesh name in assets/models/structures.glb.
    pub fn name(self) -> &'static str {
        match self {
            StructKind::Pillbox => "pillbox",
            StructKind::OctoHut => "octo_hut",
            StructKind::Watchtower => "watchtower",
            StructKind::RoundTower => "round_tower",
            StructKind::OctoCourt => "octo_court",
            StructKind::VantageTower => "vantage_tower",
            StructKind::Rotunda => "rotunda",
            StructKind::MegaOctagon => "mega_octagon",
            StructKind::MegaRotunda => "mega_rotunda",
            StructKind::MegaHexagon => "mega_hexagon",
            StructKind::MegaLens => "mega_lens",
        }
    }

    pub fn class(self) -> Class {
        match self {
            StructKind::Pillbox | StructKind::OctoHut | StructKind::Watchtower => Class::Small,
            StructKind::RoundTower | StructKind::OctoCourt => Class::Medium,
            StructKind::VantageTower | StructKind::Rotunda => Class::Large,
            _ => Class::Mega,
        }
    }

    /// Whether it looks (about) the same turned half way round, so it may stand in the
    /// arena's centre, fair to both teams.
    pub fn symmetric(self) -> bool {
        !matches!(self, StructKind::Watchtower)
    }

    /// The structures of a size set.
    pub fn of(class: Class) -> impl Iterator<Item = StructKind> {
        StructKind::ALL
            .into_iter()
            .filter(move |k| k.class() == class)
    }

    pub fn design(self) -> &'static Design {
        static DESIGNS: OnceLock<Vec<Design>> = OnceLock::new();
        let all = DESIGNS.get_or_init(|| StructKind::ALL.iter().map(|k| build(*k)).collect());
        &all[StructKind::ALL.iter().position(|k| *k == self).unwrap()]
    }
}

/// What a solid is, for its looks (the Blender builder lights each kind its own way).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tag {
    /// A wall segment: bands along its long faces' top (and bottom, over an opening) edges, its
    /// exposed ends (door jambs, slit edges) lit.
    Wall,
    /// A stair tread: a nose line across its front edge, stringer lines along its sides.
    Step,
    /// A floor slab up in the air: lines along its exposed edges, top and underneath.
    Deck,
    /// A roof slab: a band round its rim.
    Roof,
    /// A column: lines up its corners, rings at its foot and head.
    Column,
    /// A thin railing: a line along its top.
    Rail,
}

/// An upright prism over a convex polygon (x, z; anticlockwise seen from above, i.e. positive
/// signed area). Side i runs from corner i to corner i + 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    pub poly: Vec<Vec2>,
    pub y0: f32,
    pub y1: f32,
    pub tag: Tag,
    /// The sides that are its faces (bit i: side i), as opposed to ends: a wall segment's
    /// outer and inner face, a tread's outer and inner edge.
    pub faces: u8,
    /// Steps: the side at its front (where one steps up onto it).
    pub front: u8,
}

/// A spotlight: its lens, and the level direction it looks out to (the client tilts the beam
/// down and sweeps it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lamp {
    pub pos: Vec3,
    pub dir: Vec3,
}

/// A Megatarget room's doorway (local, the +Z one is Red's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Doorway {
    /// The middle of the doorway's threshold, in the middle of the wall.
    pub centre: Vec3,
    /// Out of the room.
    pub out: Vec3,
    pub half_w: f32,
    pub height: f32,
    /// How far in and out from the middle the wall's faces are.
    pub inner: f32,
    pub outer: f32,
}

/// A structure's geometry, nav and lamps in its own frame.
pub struct Design {
    pub kind: StructKind,
    pub solids: Vec<Solid>,
    pub lamps: Vec<Lamp>,
    /// Where one can stand (feet).
    pub nodes: Vec<Vec3>,
    /// Pairs of nodes one walks between.
    pub links: Vec<(u16, u16)>,
    /// Its extent over the floor (x, z).
    pub min: Vec2,
    pub max: Vec2,
    /// Megatarget rooms: the forcefields' doorways, the inside of the walls (convex,
    /// anticlockwise) and the ceiling's height.
    pub doorways: Vec<Doorway>,
    pub inner: Vec<Vec2>,
    pub ceiling: f32,
    /// How many doorways a placement must link to the grid outside (the rooms: none).
    pub min_ports: usize,
    /// The structure alone on a floor (for checking ways in).
    pub world: World,
}

impl Design {
    pub fn colliders(&self) -> impl Iterator<Item = Collider> + '_ {
        self.solids
            .iter()
            .map(|s| Collider::prism(&s.poly, s.y0, s.y1))
    }

    /// Whether one can walk from `a` to `b` (feet, local) with nothing but the structure
    /// around.
    pub fn walk(&self, a: Vec3, b: Vec3) -> bool {
        walk(&self.world, a, b)
    }
}

/// Where a structure stands in an arena.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StructPlace {
    pub kind: StructKind,
    /// Its origin (on the floor).
    pub pos: Vec3,
    /// Quarter turns about +Y (`kit::turn`: 1 takes +Z to +X).
    pub rot: u8,
    /// Its cells (Struct on the grid).
    pub cells: Vec<(usize, usize)>,
    /// Ways in: one of its nodes (index into `Design::nodes`) and the grid cell outside it's
    /// linked to.
    pub ports: Vec<(u16, (usize, usize))>,
}

/// `v` (x, z) turned by quarter turns as `kit::turn` turns vectors (1: +Z to +X), exactly.
pub fn turn2(v: Vec2, rot: u8) -> Vec2 {
    match rot % 4 {
        0 => v,
        1 => Vec2::new(v.y, -v.x),
        2 => -v,
        _ => Vec2::new(-v.y, v.x),
    }
}

pub fn turn3(v: Vec3, rot: u8) -> Vec3 {
    let t = turn2(Vec2::new(v.x, v.z), rot);
    Vec3::new(t.x, v.y, t.y)
}

impl StructPlace {
    pub fn design(&self) -> &'static Design {
        self.kind.design()
    }

    /// A local point in the arena.
    pub fn to_world(&self, p: Vec3) -> Vec3 {
        self.pos + turn3(p, self.rot)
    }

    /// An arena point in the structure's frame.
    pub fn to_local(&self, p: Vec3) -> Vec3 {
        turn3(p - self.pos, (4 - self.rot % 4) % 4)
    }

    /// The structure's colliders in the arena.
    pub fn colliders(&self) -> impl Iterator<Item = Collider> + '_ {
        self.design().solids.iter().map(|s| {
            let poly: Vec<Vec2> = s
                .poly
                .iter()
                .map(|p| turn2(*p, self.rot) + Vec2::new(self.pos.x, self.pos.z))
                .collect();
            Collider::prism(&poly, s.y0 + self.pos.y, s.y1 + self.pos.y)
        })
    }

    /// Its lamps in the arena.
    pub fn lamps(&self) -> impl Iterator<Item = Lamp> + '_ {
        self.design().lamps.iter().map(|l| Lamp {
            pos: self.to_world(l.pos),
            dir: turn3(l.dir, self.rot),
        })
    }

    /// The same structure in the other half (the arena turned half way round its centre).
    pub fn mirrored(&self, dm: crate::level::Dims) -> StructPlace {
        StructPlace {
            kind: self.kind,
            pos: crate::level::mirror_point(self.pos),
            rot: (self.rot + 2) % 4,
            cells: self
                .cells
                .iter()
                .map(|&(x, z)| dm.mirror_cell(x, z))
                .collect(),
            ports: self
                .ports
                .iter()
                .map(|&(n, (x, z))| (n, dm.mirror_cell(x, z)))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Building the designs.

const T: f32 = 0.3;
const DOOR_HALF: f32 = 0.7;
const DOOR_H: f32 = 2.3;
/// A wall's top with a parapet over a floor at the upper level.
const PARAPET: f32 = STOREY + RAIL_H;
/// The Megatarget rooms' ceiling (as the old room's) and doorway width.
pub const MEGA_CEILING: f32 = 2.0 * STOREY - FLOOR_T;
const MEGA_DOOR_HALF: f32 = (crate::kit::CELL - crate::kit::POST) / 2.0;
/// Most degrees a curved wall's segment spans.
const SEG: f32 = 15.0;

fn snap(v: f32) -> f32 {
    (v * 1e4).round() / 1e4
}

fn pol(r: f32, deg: f32) -> Vec2 {
    let a = deg.to_radians();
    Vec2::new(r * a.cos(), r * a.sin())
}

/// An opening in a wall: its middle (degrees round a round wall, metres along a polygon
/// wall's side from its middle), half its width (m, on the wall's inside) and its sill and
/// head (a head at the wall's top or above makes it a notch: a doorway or a crenel).
#[derive(Clone, Copy)]
struct Opening {
    at: f32,
    half: f32,
    y0: f32,
    y1: f32,
}

fn door(at: f32) -> Opening {
    Opening {
        at,
        half: DOOR_HALF,
        y0: 0.0,
        y1: DOOR_H,
    }
}

#[derive(Default)]
struct B {
    solids: Vec<Solid>,
    lamps: Vec<Lamp>,
    /// Explicit nodes (stairs), and floors to fill with nodes: height, and the region (feet
    /// within this radius of the middle, or inside this polygon).
    stair_nodes: Vec<Vec3>,
    floors: Vec<(f32, f32)>,
    /// More rings of nodes: height and radius.
    rings: Vec<(f32, f32)>,
    region: Option<Vec<Vec2>>,
    doorways: Vec<Doorway>,
    inner: Vec<Vec2>,
    ceiling: f32,
    min_ports: usize,
}

impl B {
    fn add(&mut self, poly: Vec<Vec2>, y0: f32, y1: f32, tag: Tag, faces: u8, front: u8) {
        let mut poly: Vec<Vec2> = poly
            .into_iter()
            .map(|p| Vec2::new(snap(p.x), snap(p.y)))
            .collect();
        poly.dedup_by(|a, b| a.distance(*b) < 1e-3);
        let area: f32 = (0..poly.len())
            .map(|i| poly[i].perp_dot(poly[(i + 1) % poly.len()]))
            .sum();
        assert!(
            area > 0.0 && poly.len() >= 3,
            "{tag:?} solid not anticlockwise"
        );
        self.solids.push(Solid {
            poly,
            y0: snap(y0),
            y1: snap(y1),
            tag,
            faces,
            front,
        });
    }

    /// An annular sector r0..r1 from a0 to a1 degrees (r0 = 0: a wedge), in segments.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, r0: f32, r1: f32, a0: f32, a1: f32, y0: f32, y1: f32, tag: Tag) {
        let n = ((a1 - a0) / SEG).ceil().max(1.0) as usize;
        for i in 0..n {
            let b0 = a0 + (a1 - a0) * i as f32 / n as f32;
            let b1 = a0 + (a1 - a0) * (i + 1) as f32 / n as f32;
            if r0 < 1e-3 {
                self.add(
                    vec![Vec2::ZERO, pol(r1, b0), pol(r1, b1)],
                    y0,
                    y1,
                    tag,
                    0b010,
                    0,
                );
            } else {
                self.add(
                    vec![pol(r0, b0), pol(r1, b0), pol(r1, b1), pol(r0, b1)],
                    y0,
                    y1,
                    tag,
                    0b1010,
                    0,
                );
            }
        }
    }

    /// A round wall r0..r1, y0..y1, with openings (`Opening::at` in degrees).
    fn round_wall(&mut self, r0: f32, r1: f32, y0: f32, y1: f32, openings: &[Opening]) {
        let half_deg = |o: &Opening| (o.half / r0).min(1.0).asin().to_degrees();
        let norm = |a: f32| a.rem_euclid(360.0);
        let mut cuts: Vec<f32> = openings
            .iter()
            .flat_map(|o| [norm(o.at - half_deg(o)), norm(o.at + half_deg(o))])
            .collect();
        cuts.push(0.0);
        cuts.sort_by(f32::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        for i in 0..cuts.len() {
            let a0 = cuts[i];
            let a1 = if i + 1 < cuts.len() {
                cuts[i + 1]
            } else {
                cuts[0] + 360.0
            };
            let mid = (a0 + a1) / 2.0;
            let inside = |o: &&Opening| {
                let d = (norm(mid - o.at + 180.0) - 180.0).abs();
                d < half_deg(o)
            };
            let mut spans = vec![(y0, y1)];
            for o in openings.iter().filter(inside) {
                spans = spans
                    .into_iter()
                    .flat_map(|(s0, s1)| {
                        [(s0, s1.min(o.y0)), (s0.max(o.y1), s1)]
                            .into_iter()
                            .filter(|(a, b)| b - a > 0.01)
                    })
                    .collect();
            }
            for (s0, s1) in spans {
                self.arc(r0, r1, a0, a1, s0, s1, Tag::Wall);
            }
        }
    }

    /// A wall round the convex polygon `inner` (anticlockwise), `t` thick outwards, mitred at
    /// the corners, with openings: (side, opening with `at` in metres from the side's middle).
    fn poly_wall(
        &mut self,
        inner: &[Vec2],
        t: f32,
        y0: f32,
        y1: f32,
        openings: &[(usize, Opening)],
    ) {
        let n = inner.len();
        let outer = offset(inner, t);
        for i in 0..n {
            let (a, b) = (inner[i], inner[(i + 1) % n]);
            let (oa, ob) = (outer[i], outer[(i + 1) % n]);
            let len = a.distance(b);
            let e = (b - a) / len;
            let nrm = Vec2::new(e.y, -e.x);
            // Cuts along the side (metres from a), and what's open between them.
            let mut cuts = vec![0.0, len];
            for (_, o) in openings.iter().filter(|(s, _)| *s == i) {
                cuts.push(len / 2.0 + o.at - o.half);
                cuts.push(len / 2.0 + o.at + o.half);
            }
            cuts.sort_by(f32::total_cmp);
            for k in 0..cuts.len() - 1 {
                let (u0, u1) = (cuts[k], cuts[k + 1]);
                if u1 - u0 < 1e-3 {
                    continue;
                }
                let mid = (u0 + u1) / 2.0 - len / 2.0;
                let mut spans = vec![(y0, y1)];
                for (_, o) in openings
                    .iter()
                    .filter(|(s, o)| *s == i && (mid - o.at).abs() < o.half)
                {
                    spans = spans
                        .into_iter()
                        .flat_map(|(s0, s1)| {
                            [(s0, s1.min(o.y0)), (s0.max(o.y1), s1)]
                                .into_iter()
                                .filter(|(a, b)| b - a > 0.01)
                        })
                        .collect();
                }
                let p0 = a + e * u0;
                let p1 = a + e * u1;
                let q0 = if u0 < 1e-4 { oa } else { p0 + nrm * t };
                let q1 = if u1 > len - 1e-4 { ob } else { p1 + nrm * t };
                for (s0, s1) in spans {
                    self.add(vec![p0, q0, q1, p1], s0, s1, Tag::Wall, 0b1010, 0);
                }
            }
        }
    }

    /// A slab over the convex polygon `poly`, split into pieces of at most 8 corners.
    fn slab(&mut self, poly: &[Vec2], y0: f32, y1: f32, tag: Tag) {
        if poly.len() <= crate::collide::PRISM_MAX {
            self.add(poly.to_vec(), y0, y1, tag, 0xff, 0);
            return;
        }
        // A fan from the first corner, 6 corners a piece.
        let mut k = 1;
        while k + 1 < poly.len() {
            let end = (k + 5).min(poly.len() - 1);
            let mut piece = vec![poly[0]];
            piece.extend_from_slice(&poly[k..=end]);
            self.add(piece, y0, y1, tag, 0xff, 0);
            k = end;
        }
    }

    /// A round slab of radius r (r0 > 0: a ring).
    fn disc(&mut self, r0: f32, r: f32, y0: f32, y1: f32, tag: Tag) {
        self.arc(r0, r, 0.0, 360.0, y0, y1, tag);
    }

    /// A regular n-gon prism (a column), its first corner at `deg`.
    #[allow(clippy::too_many_arguments)]
    fn ngon(&mut self, c: Vec2, r: f32, n: usize, deg: f32, y0: f32, y1: f32, tag: Tag) {
        let poly: Vec<Vec2> = (0..n)
            .map(|i| c + pol(r, deg + 360.0 * i as f32 / n as f32))
            .collect();
        self.add(poly, y0, y1, tag, 0xff, 0);
    }

    /// A box (centre on the floor plan, half sizes, turned `deg`).
    fn block(&mut self, c: Vec2, half: Vec2, deg: f32, y0: f32, y1: f32, tag: Tag) {
        let (u, v) = (pol(1.0, deg), pol(1.0, deg + 90.0));
        let poly = vec![
            c - u * half.x - v * half.y,
            c + u * half.x - v * half.y,
            c + u * half.x + v * half.y,
            c - u * half.x + v * half.y,
        ];
        let faces = if half.x >= half.y { 0b0101 } else { 0b1010 };
        self.add(poly, y0, y1, tag, faces, 0);
    }

    /// A spiral stair between radii r0 and r1: `n` treads from `a0` degrees sweeping `sweep`
    /// (negative: clockwise), the first `rise` above `base`, each solid down to `base`. Puts a
    /// node on every tread.
    #[allow(clippy::too_many_arguments)]
    fn spiral(&mut self, r0: f32, r1: f32, a0: f32, sweep: f32, n: usize, base: f32, rise: f32) {
        let step = sweep / n as f32;
        for k in 0..n {
            let (b0, b1) = (a0 + step * k as f32, a0 + step * (k + 1) as f32);
            let top = base + rise * (k + 1) as f32;
            // Anticlockwise corners: the front is the side at b0.
            let (lo, hi, front) = if step > 0.0 { (b0, b1, 0) } else { (b1, b0, 2) };
            self.add(
                vec![pol(r0, lo), pol(r1, lo), pol(r1, hi), pol(r0, hi)],
                base,
                top,
                Tag::Step,
                0b1010,
                front,
            );
            let m = pol((r0 + r1) / 2.0, (b0 + b1) / 2.0);
            self.stair_nodes.push(Vec3::new(m.x, top, m.y));
        }
    }

    fn lamp(&mut self, r: f32, deg: f32, y: f32, inward: bool) {
        let p = pol(r, deg);
        let d = pol(if inward { -1.0 } else { 1.0 }, deg);
        self.lamps.push(Lamp {
            pos: Vec3::new(snap(p.x), y, snap(p.y)),
            dir: Vec3::new(snap(d.x), 0.0, snap(d.y)),
        });
    }

    /// A Megatarget room's forcefield doorways in its ±Z walls (inner face at `z_in`, outer at
    /// `z_out`, at x = 0).
    fn mega_doorways(&mut self, z_in: f32, z_out: f32) {
        for s in [1.0f32, -1.0] {
            let mid = (z_in + z_out) / 2.0;
            self.doorways.push(Doorway {
                centre: Vec3::new(0.0, 0.0, s * snap(mid)),
                out: Vec3::new(0.0, 0.0, s),
                half_w: MEGA_DOOR_HALF,
                height: STOREY,
                inner: snap(mid - z_in),
                outer: snap(z_out - mid),
            });
        }
        self.ceiling = MEGA_CEILING;
        self.region = Some(offset(&self.inner, -0.4));
    }
}

/// The convex polygon `poly` (anticlockwise) with every side moved out by `d` (in: negative).
fn offset(poly: &[Vec2], d: f32) -> Vec<Vec2> {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let prev = poly[(i + n - 1) % n];
            let (p, next) = (poly[i], poly[(i + 1) % n]);
            let n1 = {
                let e = (p - prev).normalize();
                Vec2::new(e.y, -e.x)
            };
            let n2 = {
                let e = (next - p).normalize();
                Vec2::new(e.y, -e.x)
            };
            let m = (n1 + n2) / (1.0 + n1.dot(n2));
            p + m * d
        })
        .collect()
}

/// A regular polygon's corners (anticlockwise) with apothem `a`, side i's outward normal at
/// `first + i * 360 / n` degrees.
fn regular(n: usize, a: f32, first: f32) -> Vec<Vec2> {
    let half = 180.0 / n as f32;
    let r = a / half.to_radians().cos();
    (0..n)
        .map(|i| pol(r, first - half + 360.0 * i as f32 / n as f32))
        .collect()
}

fn build(kind: StructKind) -> Design {
    let mut b = B {
        min_ports: 2,
        ..Default::default()
    };
    match kind {
        StructKind::Pillbox => {
            let (r0, r1, h) = (2.25, 2.55, 2.3);
            let slit = |at: f32, half: f32| Opening {
                at,
                half,
                y0: 1.15,
                y1: 1.5,
            };
            b.round_wall(
                r0,
                r1,
                0.0,
                h,
                &[
                    door(90.0),
                    door(270.0),
                    slit(0.0, 0.9),
                    slit(180.0, 0.9),
                    slit(45.0, 0.5),
                    slit(135.0, 0.5),
                    slit(225.0, 0.5),
                    slit(315.0, 0.5),
                ],
            );
            b.disc(0.0, 2.7, h, h + 0.25, Tag::Roof);
            b.floors.push((0.0, r0 - 0.3));
        }
        StructKind::OctoHut => {
            let inner = regular(8, 2.25, 45.0);
            let win = |half: f32, y0: f32, y1: f32| Opening {
                at: 0.0,
                half,
                y0,
                y1,
            };
            // Side i faces 45 (i + 1) degrees: doorways at +-Z, windows at +-X, slits between.
            let mut openings = vec![(1, door(0.0)), (5, door(0.0))];
            for s in [3, 7] {
                openings.push((s, win(0.55, 1.05, 1.75)));
            }
            for s in [0, 2, 4, 6] {
                openings.push((s, win(0.25, 1.2, 1.65)));
            }
            b.poly_wall(&inner, T, 0.0, 2.8, &openings);
            b.slab(&regular(8, 2.25 + T + 0.08, 45.0), 2.8, 3.0, Tag::Roof);
            b.floors.push((0.0, 2.0));
        }
        StructKind::Watchtower => {
            let (r0, r1) = (2.35, 2.65);
            let mut openings = vec![door(270.0)];
            for k in 0..8 {
                openings.push(Opening {
                    at: 22.5 + 45.0 * k as f32,
                    half: 0.35,
                    y0: PARAPET - 0.45,
                    y1: PARAPET + 1.0,
                });
            }
            b.round_wall(r0, r1, 0.0, PARAPET, &openings);
            b.ngon(Vec2::ZERO, 0.3, 8, 0.0, 0.0, PARAPET + 0.3, Tag::Column);
            // The stair from beside the doorway round to the top's landing over it.
            b.spiral(0.3, r0, 315.0, 270.0, 12, 0.0, STOREY / 12.0);
            b.arc(0.3, r0, 225.0, 382.5, STOREY - FLOOR_T, STOREY, Tag::Deck);
            // A railing along the landing's edge over the stairwell.
            let d = pol(1.0, 382.5 - 1.5);
            let p = Vec2::new(-d.y, d.x) * 0.04;
            b.add(
                vec![d * 0.3 - p, d * r0 - p, d * r0 + p, d * 0.3 + p],
                STOREY,
                PARAPET,
                Tag::Rail,
                0b1010,
                0,
            );
            b.lamp((r0 + r1) / 2.0, 90.0, PARAPET, false);
            b.floors.push((0.0, r0 - 0.3));
            b.floors.push((STOREY, r0 - 0.3));
            b.min_ports = 1;
        }
        StructKind::RoundTower => {
            let (r0, r1, ri) = (3.35, 3.65, 2.0);
            let mut openings = vec![door(90.0), door(270.0)];
            for k in 0..12 {
                openings.push(Opening {
                    at: 15.0 + 30.0 * k as f32,
                    half: 0.4,
                    y0: PARAPET - 0.45,
                    y1: PARAPET + 1.0,
                });
            }
            b.round_wall(r0, r1, 0.0, PARAPET, &openings);
            b.spiral(ri, r0, 120.0, 120.0, 12, 0.0, STOREY / 12.0);
            b.spiral(ri, r0, 300.0, 120.0, 12, 0.0, STOREY / 12.0);
            b.ngon(Vec2::ZERO, 0.4, 8, 0.0, 0.0, STOREY - FLOOR_T, Tag::Column);
            b.disc(0.0, ri, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri, r0, 60.0, 120.0, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri, r0, 240.0, 300.0, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri - 0.1, ri, 120.0, 240.0, STOREY, PARAPET, Tag::Rail);
            b.arc(ri - 0.1, ri, 300.0, 420.0, STOREY, PARAPET, Tag::Rail);
            b.lamp((r0 + r1) / 2.0, 0.0, PARAPET, false);
            b.lamp((r0 + r1) / 2.0, 180.0, PARAPET, false);
            b.floors.push((0.0, r0 - 0.3));
            b.floors.push((STOREY, r0 - 0.3));
            b.rings.push((0.0, (ri + r0) / 2.0));
            b.rings.push((STOREY, (ri + r0) / 2.0));
        }
        StructKind::OctoCourt => {
            let inner = regular(8, 3.4, 45.0);
            let mut openings = Vec::new();
            for s in [1, 3, 5, 7] {
                openings.push((s, door(0.0)));
            }
            for s in [0, 2, 4, 6] {
                openings.push((
                    s,
                    Opening {
                        at: 0.0,
                        half: 0.65,
                        y0: 1.05,
                        y1: 1.75,
                    },
                ));
            }
            b.poly_wall(&inner, T, 0.0, 2.8, &openings);
            b.ngon(Vec2::ZERO, 0.8, 8, 22.5, 0.0, 3.4, Tag::Column);
            for k in 0..4 {
                let deg = 45.0 + 90.0 * k as f32;
                b.block(
                    pol(2.0, deg),
                    Vec2::new(0.12, 0.8),
                    deg,
                    0.0,
                    crate::kit::HALF_H,
                    Tag::Wall,
                );
            }
            b.floors.push((0.0, 3.1));
        }
        StructKind::VantageTower => {
            let (r0, r1, ri) = (4.05, 4.35, 2.95);
            let mut openings = vec![
                Opening {
                    half: 0.75,
                    ..door(90.0)
                },
                Opening {
                    half: 0.75,
                    ..door(270.0)
                },
            ];
            for k in 0..12 {
                openings.push(Opening {
                    at: 15.0 + 30.0 * k as f32,
                    half: 0.45,
                    y0: PARAPET - 0.45,
                    y1: PARAPET + 1.0,
                });
            }
            b.round_wall(r0, r1, 0.0, PARAPET, &openings);
            b.spiral(ri, r0, 120.0, 120.0, 12, 0.0, STOREY / 12.0);
            b.spiral(ri, r0, 300.0, 120.0, 12, 0.0, STOREY / 12.0);
            let top = 2.0 * STOREY;
            b.ngon(
                Vec2::ZERO,
                0.45,
                8,
                0.0,
                0.0,
                top + RAIL_H + 0.5,
                Tag::Column,
            );
            b.disc(0.0, ri, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri, r0, 60.0, 120.0, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri, r0, 240.0, 300.0, STOREY - FLOOR_T, STOREY, Tag::Deck);
            b.arc(ri - 0.1, ri, 120.0, 240.0, STOREY, PARAPET, Tag::Rail);
            b.arc(ri - 0.1, ri, 300.0, 420.0, STOREY, PARAPET, Tag::Rail);
            // Round the core: the stair on up to the vantage ring, held up by four columns.
            let (c0, c1, v1) = (0.45, 1.7, 3.4);
            b.spiral(c0, c1, 0.0, 300.0, 12, STOREY, STOREY / 12.0);
            b.arc(c0, c1, 300.0, 360.0, top - FLOOR_T, top, Tag::Deck);
            b.disc(c1, v1, top - FLOOR_T, top, Tag::Deck);
            for k in 0..4 {
                b.ngon(
                    pol(ri - 0.1, 45.0 + 90.0 * k as f32),
                    0.15,
                    8,
                    0.0,
                    STOREY,
                    top - FLOOR_T,
                    Tag::Column,
                );
            }
            b.arc(c1, c1 + 0.1, 0.0, 275.0, top, top + RAIL_H, Tag::Rail);
            let crenels: Vec<Opening> = (0..8)
                .map(|k| Opening {
                    at: 22.5 + 45.0 * k as f32,
                    half: 0.35,
                    y0: top + RAIL_H - 0.4,
                    y1: top + RAIL_H + 1.0,
                })
                .collect();
            b.round_wall(v1 - 0.2, v1, top, top + RAIL_H, &crenels);
            for k in 0..4 {
                b.lamp(v1 - 0.1, 45.0 + 90.0 * k as f32, top + RAIL_H, false);
            }
            b.lamp((r0 + r1) / 2.0, 0.0, PARAPET, false);
            b.lamp((r0 + r1) / 2.0, 180.0, PARAPET, false);
            b.floors.push((0.0, r0 - 0.3));
            b.floors.push((STOREY, r0 - 0.3));
            b.rings.push((0.0, (ri + r0) / 2.0));
            b.rings.push((STOREY, (ri + r0) / 2.0));
            b.rings.push((STOREY, (c1 + ri) / 2.0));
            b.floors.push((top, v1 - 0.4));
        }
        StructKind::Rotunda => {
            for k in 0..12 {
                b.ngon(
                    pol(4.0, 15.0 + 30.0 * k as f32),
                    0.32,
                    8,
                    0.0,
                    0.0,
                    2.6,
                    Tag::Column,
                );
            }
            b.disc(3.55, 4.45, 2.6, 3.0, Tag::Roof);
            b.disc(0.0, 1.5, 0.0, 0.2, Tag::Deck);
            let gaps: Vec<Opening> = (0..4)
                .map(|k| Opening {
                    at: 90.0 * k as f32,
                    half: 0.75,
                    y0: 0.0,
                    y1: 5.0,
                })
                .collect();
            b.round_wall(2.3, 2.55, 0.0, crate::kit::HALF_H, &gaps);
            for k in 0..4 {
                b.lamp(3.9, 45.0 + 90.0 * k as f32, 3.0, true);
            }
            b.floors.push((0.0, 4.4));
            b.floors.push((0.2, 1.3));
        }
        StructKind::MegaOctagon => {
            b.inner = regular(8, 3.65, 45.0);
            let inner = b.inner.clone();
            let door = Opening {
                at: 0.0,
                half: MEGA_DOOR_HALF,
                y0: 0.0,
                y1: STOREY,
            };
            b.poly_wall(&inner, T, 0.0, MEGA_CEILING, &[(1, door), (5, door)]);
            b.slab(
                &regular(8, 3.65 + T, 45.0),
                MEGA_CEILING,
                2.0 * STOREY,
                Tag::Roof,
            );
            b.mega_doorways(3.65, 3.65 + T);
        }
        StructKind::MegaRotunda => {
            let (r0, r1) = (3.7, 4.0);
            b.inner = (0..32).map(|i| pol(r0, 360.0 * i as f32 / 32.0)).collect();
            let door = |at: f32| Opening {
                at,
                half: MEGA_DOOR_HALF,
                y0: 0.0,
                y1: STOREY,
            };
            b.round_wall(r0, r1, 0.0, MEGA_CEILING, &[door(90.0), door(270.0)]);
            b.disc(0.0, r1, MEGA_CEILING, 2.0 * STOREY, Tag::Roof);
            for k in 0..4 {
                b.ngon(
                    pol(2.6, 45.0 + 90.0 * k as f32),
                    0.25,
                    8,
                    0.0,
                    0.0,
                    MEGA_CEILING,
                    Tag::Column,
                );
            }
            let ha = (MEGA_DOOR_HALF / r0).asin();
            b.mega_doorways(r0 * ha.cos(), r1 * ha.cos());
        }
        StructKind::MegaHexagon => {
            b.inner = regular(6, 3.4, 30.0);
            let inner = b.inner.clone();
            let door = Opening {
                at: 0.0,
                half: MEGA_DOOR_HALF,
                y0: 0.0,
                y1: STOREY,
            };
            // Side i faces 30 + 60 i degrees: the doorways at +-Z.
            b.poly_wall(&inner, T, 0.0, MEGA_CEILING, &[(1, door), (4, door)]);
            b.slab(
                &regular(6, 3.4 + T, 30.0),
                MEGA_CEILING,
                2.0 * STOREY,
                Tag::Roof,
            );
            b.mega_doorways(3.4, 3.4 + T);
        }
        StructKind::MegaLens => {
            b.inner = vec![
                Vec2::new(4.4, -1.4),
                Vec2::new(4.4, 1.4),
                Vec2::new(2.4, 3.4),
                Vec2::new(-2.4, 3.4),
                Vec2::new(-4.4, 1.4),
                Vec2::new(-4.4, -1.4),
                Vec2::new(-2.4, -3.4),
                Vec2::new(2.4, -3.4),
            ];
            let inner = b.inner.clone();
            let door = Opening {
                at: 0.0,
                half: MEGA_DOOR_HALF,
                y0: 0.0,
                y1: STOREY,
            };
            b.poly_wall(&inner, T, 0.0, MEGA_CEILING, &[(2, door), (6, door)]);
            b.slab(&offset(&inner, T), MEGA_CEILING, 2.0 * STOREY, Tag::Roof);
            for s in [1.0, -1.0] {
                b.block(
                    Vec2::new(s * 2.7, 0.0),
                    Vec2::new(0.6, 0.6),
                    45.0,
                    0.0,
                    crate::kit::BLOCK,
                    Tag::Column,
                );
            }
            b.mega_doorways(3.4, 3.4 + T);
        }
    }
    if kind.class() == Class::Mega {
        b.floors.push((0.0, 10.0));
        b.min_ports = 0;
    }
    finish(kind, b)
}

/// A body standing at `p` (feet) fits there, with a little room to spare, on a floor at that
/// height.
fn stands(w: &World, p: Vec3) -> bool {
    let Some(h) = w.ground(p.x, p.z, p.y + 0.3) else {
        return false;
    };
    if (h - p.y).abs() > 0.06 {
        return false;
    }
    let p = Vec3::new(p.x, h, p.z);
    [
        Vec2::ZERO,
        Vec2::X * 0.1,
        Vec2::NEG_X * 0.1,
        Vec2::Y * 0.1,
        Vec2::NEG_Y * 0.1,
    ]
    .iter()
    .all(|o| {
        let q = p + Vec3::new(o.x, 0.0, o.y);
        let (r, _) = w.move_body(q, Vec2::ZERO, 0.0, 1.0 / 60.0);
        Vec2::new(r.x - q.x, r.z - q.z).length() < 1e-3 && (r.y - q.y).abs() < 0.05
    })
}

/// Whether a body walks from `a` to `b` (feet) in a straight line: always over floor within a
/// step of the line, never pushed aside.
pub fn walk(w: &World, a: Vec3, b: Vec3) -> bool {
    let d = Vec2::new(b.x - a.x, b.z - a.z).length();
    let n = (d / 0.2).ceil().max(1.0) as usize;
    (0..=n).all(|i| {
        let p = a.lerp(b, i as f32 / n as f32);
        let Some(h) = w.ground(p.x, p.z, p.y + 0.45) else {
            return false;
        };
        if h < p.y - 0.45 {
            return false;
        }
        let f = Vec3::new(p.x, h, p.z);
        let (r, _) = w.move_body(f, Vec2::ZERO, 0.0, 1.0 / 60.0);
        Vec2::new(r.x - f.x, r.z - f.z).length() < 1e-3
    })
}

fn finish(kind: StructKind, b: B) -> Design {
    let mut colliders: Vec<Collider> = b
        .solids
        .iter()
        .map(|s| Collider::prism(&s.poly, s.y0, s.y1))
        .collect();
    let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for c in &colliders {
        let (a, z) = c.bounds();
        min = min.min(Vec2::new(a.x, a.z));
        max = max.max(Vec2::new(z.x, z.z));
    }
    colliders.push(Collider::Box {
        min: Vec3::new(min.x - 4.0, -1.0, min.y - 4.0),
        max: Vec3::new(max.x + 4.0, 0.0, max.y + 4.0),
    });
    let world = World::new(colliders);

    // Nodes: on every tread, and a lattice over each floor.
    let mut nodes: Vec<Vec3> = b
        .stair_nodes
        .iter()
        .copied()
        .filter(|p| stands(&world, *p))
        .collect();
    // Rings round the middle (round rooms are what the structures mostly are), every 0.75 m
    // out, a node every 0.85 m round each.
    let reach = (max - min).max_element() / 2.0 + 0.5;
    for &(y, r) in &b.floors {
        let mut cands = vec![Vec2::ZERO];
        let mut k = 0;
        loop {
            let rr = 0.65 + 0.75 * k as f32;
            if rr > reach.min(if b.region.is_some() { reach } else { r }) {
                break;
            }
            let n = ((std::f32::consts::TAU * rr / 0.85).ceil() as usize).max(6);
            let phase = if k % 2 == 0 { 0.0 } else { 180.0 / n as f32 };
            cands.extend((0..n).map(|i| pol(rr, phase + 360.0 * i as f32 / n as f32)));
            k += 1;
        }
        for &(ry, rr) in &b.rings {
            if ry == y {
                let n = ((std::f32::consts::TAU * rr / 0.6).ceil() as usize).max(6);
                cands.extend((0..n).map(|i| pol(rr, 360.0 * i as f32 / n as f32)));
            }
        }
        for p in cands {
            let inside = match &b.region {
                Some(poly) => (0..poly.len()).all(|i| {
                    let (a, c) = (poly[i], poly[(i + 1) % poly.len()]);
                    (c - a).perp_dot(p - a) >= 0.0
                }),
                None => p.length() <= r,
            };
            let q = Vec3::new(snap(p.x), y, snap(p.y));
            if inside && stands(&world, q) && nodes.iter().all(|n| n.distance(q) > 0.5) {
                nodes.push(q);
            }
        }
    }
    let mut links = Vec::new();
    for i in 0..nodes.len() {
        for j in i + 1..nodes.len() {
            let (a, c) = (nodes[i], nodes[j]);
            let flat = Vec2::new(a.x - c.x, a.z - c.z).length();
            if flat <= 1.7 && (a.y - c.y).abs() <= 0.6 && walk(&world, a, c) {
                links.push((i as u16, j as u16));
            }
        }
    }
    // Only the nodes that hang together with the most others (a pocket between columns is
    // left to the grid outside).
    let mut comp = vec![usize::MAX; nodes.len()];
    let mut sizes = Vec::new();
    for s0 in 0..nodes.len() {
        if comp[s0] != usize::MAX {
            continue;
        }
        let c = sizes.len();
        comp[s0] = c;
        let mut stack = vec![s0];
        let mut size = 0;
        while let Some(n) = stack.pop() {
            size += 1;
            for &(a, z) in &links {
                let (a, z) = (a as usize, z as usize);
                for (x, y) in [(a, z), (z, a)] {
                    if x == n && comp[y] == usize::MAX {
                        comp[y] = c;
                        stack.push(y);
                    }
                }
            }
        }
        sizes.push(size);
    }
    if let Some(main) = (0..sizes.len()).max_by_key(|&c| sizes[c]) {
        let keep: Vec<usize> = (0..nodes.len()).filter(|&i| comp[i] == main).collect();
        let index = |i: usize| keep.iter().position(|&k| k == i).map(|p| p as u16);
        links = links
            .iter()
            .filter_map(|&(a, z)| Some((index(a as usize)?, index(z as usize)?)))
            .collect();
        nodes = keep.iter().map(|&i| nodes[i]).collect();
    }
    let lamp_max = b
        .lamps
        .iter()
        .fold(Vec2::ZERO, |m, l| m.max(Vec2::new(l.pos.x, l.pos.z).abs()));
    Design {
        kind,
        min: min.min(-lamp_max),
        max: max.max(lamp_max),
        solids: b.solids,
        lamps: b.lamps,
        nodes,
        links,
        doorways: b.doorways,
        inner: b.inner,
        ceiling: b.ceiling,
        min_ports: b.min_ports,
        world,
    }
}

/// The JSON handed to the Blender builder (tools/models/structures.py).
pub fn structures_json() -> String {
    let v2 = |p: Vec2| format!("[{}, {}]", p.x, p.y);
    let v3 = |p: Vec3| format!("[{}, {}, {}]", p.x, p.y, p.z);
    let tag = |t: Tag| match t {
        Tag::Wall => "wall",
        Tag::Step => "step",
        Tag::Deck => "deck",
        Tag::Roof => "roof",
        Tag::Column => "column",
        Tag::Rail => "rail",
    };
    let mut s = String::from("{\n");
    for (i, k) in StructKind::ALL.iter().enumerate() {
        let d = k.design();
        s += &format!(
            "  \"{}\": {{\n    \"bounds\": [{}, {}],\n    \"lamps\": [{}],\n    \"solids\": [\n",
            k.name(),
            v2(d.min),
            v2(d.max),
            d.lamps
                .iter()
                .map(|l| format!("{{\"pos\": {}, \"dir\": {}}}", v3(l.pos), v3(l.dir)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (j, so) in d.solids.iter().enumerate() {
            let poly: Vec<String> = so.poly.iter().map(|p| v2(*p)).collect();
            let comma = if j + 1 < d.solids.len() { "," } else { "" };
            s += &format!(
                "      {{\"tag\": \"{}\", \"y\": [{}, {}], \"faces\": {}, \"front\": {}, \"poly\": [{}]}}{comma}\n",
                tag(so.tag),
                so.y0,
                so.y1,
                so.faces,
                so.front,
                poly.join(", ")
            );
        }
        let comma = if i + 1 < StructKind::ALL.len() {
            ","
        } else {
            ""
        };
        s += &format!("    ]\n  }}{comma}\n");
    }
    s += "}\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structures_json_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/models/structures.json"
        );
        let json = structures_json();
        let _: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        if std::env::var_os("LT_WRITE_STRUCTURES").is_some() {
            std::fs::write(path, &json).unwrap();
        }
        let on_disk = std::fs::read_to_string(path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            on_disk, json,
            "structures.json is stale: LT_WRITE_STRUCTURES=1 cargo test -p lt-sim structures"
        );
    }

    /// The nodes reachable from `start` by the design's own links.
    fn reach(d: &Design, start: usize) -> Vec<bool> {
        let mut seen = vec![false; d.nodes.len()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(n) = stack.pop() {
            for &(a, b) in &d.links {
                let (a, b) = (a as usize, b as usize);
                for (x, y) in [(a, b), (b, a)] {
                    if x == n && !seen[y] {
                        seen[y] = true;
                        stack.push(y);
                    }
                }
            }
        }
        seen
    }

    #[test]
    fn designs_fit_their_class_and_hang_together() {
        for k in StructKind::ALL {
            let d = k.design();
            let half = d.min.abs().max(d.max.abs()).max_element();
            let limit = match k.class() {
                Class::Small => 3.0,
                Class::Medium => 4.0,
                Class::Large => 5.0,
                Class::Mega => 5.0,
            };
            assert!(half <= limit, "{k:?} is {half} m across half");
            assert!(!d.nodes.is_empty(), "{k:?} has no nodes");
            // Every node is reached from the ground nodes.
            let start = d.nodes.iter().position(|n| n.y < 0.1).expect("ground node");
            let seen = reach(d, start);
            let lost: Vec<Vec3> = (0..d.nodes.len())
                .filter(|&i| !seen[i])
                .map(|i| d.nodes[i])
                .collect();
            assert!(lost.is_empty(), "{k:?}: nodes not reached {lost:?}");
        }
    }

    #[test]
    fn towers_reach_their_tops() {
        for (k, top) in [
            (StructKind::Watchtower, STOREY),
            (StructKind::RoundTower, STOREY),
            (StructKind::VantageTower, 2.0 * STOREY),
        ] {
            let d = k.design();
            assert!(
                d.nodes.iter().any(|n| (n.y - top).abs() < 0.01),
                "{k:?} never gets to {top}"
            );
            assert!(!d.lamps.is_empty(), "{k:?} has no spotlights");
        }
    }

    #[test]
    fn walking_up_the_vantage_tower() {
        // A body walking node to node along a path from the ground to the top really gets
        // there (the stairs climb, nothing is in the way).
        let d = StructKind::VantageTower.design();
        let start = d.nodes.iter().position(|n| n.y < 0.1).unwrap();
        let goal = d
            .nodes
            .iter()
            .position(|n| (n.y - 2.0 * STOREY).abs() < 0.01)
            .unwrap();
        // Breadth-first path.
        let mut prev = vec![usize::MAX; d.nodes.len()];
        prev[start] = start;
        let mut queue = std::collections::VecDeque::from([start]);
        while let Some(n) = queue.pop_front() {
            for &(a, b) in &d.links {
                let (a, b) = (a as usize, b as usize);
                for (x, y) in [(a, b), (b, a)] {
                    if x == n && prev[y] == usize::MAX {
                        prev[y] = n;
                        queue.push_back(y);
                    }
                }
            }
        }
        assert!(prev[goal] != usize::MAX);
        let mut path = vec![goal];
        while *path.last().unwrap() != start {
            path.push(prev[*path.last().unwrap()]);
        }
        path.reverse();
        let mut p = d.nodes[start];
        let mut vy = 0.0;
        for &n in &path[1..] {
            let target = d.nodes[n];
            for _ in 0..240 {
                let to = Vec2::new(target.x - p.x, target.z - p.z);
                if to.length() < 0.05 {
                    break;
                }
                let step = to.normalize() * (4.5f32 / 60.0).min(to.length());
                (p, vy) = d.world.move_body(p, step, vy, 1.0 / 60.0);
            }
        }
        assert!((p.y - 2.0 * STOREY).abs() < 0.05, "ended at {p}");
    }

    #[test]
    fn mega_rooms_have_two_doorways() {
        for k in StructKind::MEGA_ROOMS {
            let d = k.design();
            assert_eq!(d.doorways.len(), 2, "{k:?}");
            assert!(d.inner.len() >= 3);
            for dw in &d.doorways {
                // Walk through the doorway from outside to inside.
                let out = dw.centre + dw.out * (dw.outer + 0.8);
                let inside = dw.centre - dw.out * (dw.inner + 0.8);
                assert!(d.walk(out, inside), "{k:?}: can't walk through its doorway");
            }
        }
    }
}

//! Procedural arenas: a 2 m grid, at most two floors, generated for Red's half (+z) and
//! turned half way round the centre for Blue's (cell (x, z) becomes (W-1-x, H-1-z)), so the two
//! bases are in opposite corners.
//!
//! Each team's half has: its base (a small corner of spawn pads, walled with a gate on each
//! open side), one or two team targets in the rear quarter (each hanging from a roof over the
//! upper level, in a cell walled on three sides on both floors and open to the rear, with no
//! upper floor in it or behind it: hit only from the rear quarter, from at least two places on
//! each floor), an upper floor of connected platforms with holes cut in (at least two ramps up
//! per half, and always a way across the arena on the upper level alone), and cover on the
//! ground: walls, walls with windows, half walls, pillars and blocks. Long open lanes are
//! broken up, and every attempt is checked for connectivity (and the targets for being
//! hittable) before it's accepted.
//!
//! Some pieces move (`Mover`), driven only by the sim tick so that every client works out
//! where they are from a snapshot's tick, with no extra network data:
//!
//! - **Timed doors**: a few walls (mirrored pairs) sink into the floor and rise again on a
//!   cycle, each pair at its own phase. They are only ever walls the arena already had (a
//!   full wall, a wall with windows or a half wall), so the arena with every door closed is
//!   the validated one; and none is in a base wall or next to a target alcove. Opening one
//!   shortens a route that otherwise takes a detour.
//! - **Sliding panels**: free-standing cover panels (mirrored pairs) gliding back and forth
//!   along a short straight track of open cells, the cells past both ends of the track open
//!   too, so a player in the way is always pushed somewhere free.
//!
//! Movers are not in `pieces` nor in the static `colliders`: the client draws them itself
//! (`Mover::offset` at the tick it shows) and the sim puts their colliders into
//! `World::dynamic` every tick.

use glam::{Vec3, Vec4};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::collide::{Collider, World};
use crate::kit::{self, CELL, PieceKind, STOREY};
use crate::types::Team;

/// Cells across (x).
pub const W: usize = 14;
/// Cells along (z), both halves.
pub const H: usize = 28;
pub const HALF: usize = H / 2;
/// Upper floors are allowed in Red's rows HALF..=UPPER_MAX (not over the bases).
const UPPER_MAX: usize = HALF + 9;
/// The first row of Red's base (its spawn pads: `BASE_W` cells in a corner, rows BASE_ROW..H).
const BASE_ROW: usize = H - 2;
const BASE_W: usize = 3;
/// The row of Red's targets: the first of its rear quarter.
const TARGET_ROW: usize = H * 3 / 4;

pub const HALF_WIDTH: f32 = W as f32 * CELL / 2.0;
pub const HALF_LENGTH: f32 = H as f32 * CELL / 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ground {
    #[default]
    Open,
    Spawn,
    Pillar,
    Block,
    /// `dir`: uphill; `seg` 0 (low end) ..= 2 (high end).
    Ramp { dir: u8, seg: u8 },
    /// Open towards `facing`.
    Alcove { facing: u8 },
    /// A reload station for blasters (walls with windows on its x sides).
    Reload,
}

impl Ground {
    /// The same cell turned half way round (Blue's half is Red's turned about the centre).
    fn mirrored(self) -> Ground {
        match self {
            Ground::Ramp { dir, seg } => Ground::Ramp { dir: (dir + 2) % 4, seg },
            Ground::Alcove { facing } => Ground::Alcove { facing: (facing + 2) % 4 },
            g => g,
        }
    }

    pub fn walkable(self) -> bool {
        matches!(self, Ground::Open | Ground::Spawn | Ground::Reload | Ground::Ramp { .. } | Ground::Alcove { .. })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wall {
    #[default]
    None,
    Full,
    Window,
    Half,
    Rail,
    /// A timed door (a `Mover`): a full wall while closed, nothing while open.
    Door,
}

impl Wall {
    fn piece(self) -> Option<PieceKind> {
        match self {
            Wall::None => None,
            Wall::Full => Some(PieceKind::WallFull),
            Wall::Window => Some(PieceKind::WallWindow),
            Wall::Half => Some(PieceKind::WallHalf),
            Wall::Rail => Some(PieceKind::Railing),
            // Drawn and collided as a mover.
            Wall::Door => None,
        }
    }

    fn tall(self) -> bool {
        matches!(self, Wall::Full | Wall::Window | Wall::Door)
    }
}

/// (dx, dz) of the four grid directions: 0 +Z, 1 +X, 2 -Z, 3 -X (as `kit::turn`).
pub const DIRS: [(i32, i32); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];

/// The arena's layout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Grid {
    pub ground: Vec<Ground>,
    pub upper: Vec<bool>,
    pub upper_block: Vec<bool>,
    /// Walls on the lines of constant z, per floor: index `l * W + ix`, line `l` in 0..=H.
    pub hwall: [Vec<Wall>; 2],
    /// Walls on the lines of constant x, per floor: index `iz * (W + 1) + l`, line `l` in 0..=W.
    pub vwall: [Vec<Wall>; 2],
}

pub fn idx(ix: usize, iz: usize) -> usize {
    iz * W + ix
}

/// The cell's counterpart in the other half (the arena turned half way round its centre).
pub fn mirror_cell(ix: usize, iz: usize) -> (usize, usize) {
    (W - 1 - ix, H - 1 - iz)
}

/// A point's counterpart in the other half.
pub fn mirror_point(p: Vec3) -> Vec3 {
    Vec3::new(-p.x, p.y, -p.z)
}

pub fn cell_centre(ix: usize, iz: usize) -> Vec3 {
    Vec3::new(
        (ix as f32 + 0.5) * CELL - HALF_WIDTH,
        0.0,
        (iz as f32 + 0.5) * CELL - HALF_LENGTH,
    )
}

/// The cell containing a point, if inside the arena.
pub fn cell_at(p: Vec3) -> Option<(usize, usize)> {
    let x = ((p.x + HALF_WIDTH) / CELL).floor();
    let z = ((p.z + HALF_LENGTH) / CELL).floor();
    if x < 0.0 || z < 0.0 || x >= W as f32 || z >= H as f32 {
        return None;
    }
    Some((x as usize, z as usize))
}

fn step(ix: usize, iz: usize, dir: u8, n: i32) -> Option<(usize, usize)> {
    let (dx, dz) = DIRS[dir as usize % 4];
    let x = ix as i32 + dx * n;
    let z = iz as i32 + dz * n;
    if x < 0 || z < 0 || x >= W as i32 || z >= H as i32 {
        return None;
    }
    Some((x as usize, z as usize))
}

/// An edge of the grid: horizontal line (constant z) or vertical line (constant x).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    H { l: usize, ix: usize },
    V { l: usize, iz: usize },
}

/// The edge between a cell and its neighbour in `dir`.
fn edge(ix: usize, iz: usize, dir: u8) -> Edge {
    match dir % 4 {
        0 => Edge::H { l: iz + 1, ix },
        1 => Edge::V { l: ix + 1, iz },
        2 => Edge::H { l: iz, ix },
        _ => Edge::V { l: ix, iz },
    }
}

impl Grid {
    fn new() -> Grid {
        Grid {
            ground: vec![Ground::Open; W * H],
            upper: vec![false; W * H],
            upper_block: vec![false; W * H],
            hwall: [vec![Wall::None; W * (H + 1)], vec![Wall::None; W * (H + 1)]],
            vwall: [vec![Wall::None; (W + 1) * H], vec![Wall::None; (W + 1) * H]],
        }
    }

    pub fn wall(&self, floor: usize, ix: usize, iz: usize, dir: u8) -> Wall {
        self.edge_wall(floor, edge(ix, iz, dir))
    }

    fn edge_wall(&self, floor: usize, e: Edge) -> Wall {
        match e {
            Edge::H { l, ix } => self.hwall[floor][l * W + ix],
            Edge::V { l, iz } => self.vwall[floor][iz * (W + 1) + l],
        }
    }

    // Setters writing Red's half and its mirror image in Blue's.

    fn set_ground(&mut self, ix: usize, iz: usize, g: Ground) {
        self.ground[idx(ix, iz)] = g;
        let (mx, mz) = mirror_cell(ix, iz);
        self.ground[idx(mx, mz)] = g.mirrored();
    }

    fn set_upper(&mut self, ix: usize, iz: usize, v: bool) {
        self.upper[idx(ix, iz)] = v;
        let (mx, mz) = mirror_cell(ix, iz);
        self.upper[idx(mx, mz)] = v;
    }

    fn set_upper_block(&mut self, ix: usize, iz: usize, v: bool) {
        self.upper_block[idx(ix, iz)] = v;
        let (mx, mz) = mirror_cell(ix, iz);
        self.upper_block[idx(mx, mz)] = v;
    }

    fn set_wall(&mut self, floor: usize, e: Edge, w: Wall) {
        match e {
            Edge::H { l, ix } => {
                self.hwall[floor][l * W + ix] = w;
                self.hwall[floor][(H - l) * W + W - 1 - ix] = w;
            }
            Edge::V { l, iz } => {
                self.vwall[floor][iz * (W + 1) + l] = w;
                self.vwall[floor][(H - 1 - iz) * (W + 1) + W - l] = w;
            }
        }
    }

    fn upper_walkable(&self, i: usize) -> bool {
        self.upper[i] && !self.upper_block[i]
    }
}

/// Bookkeeping while generating: cells and edges that must stay clear.
struct Reserve {
    cells: Vec<bool>,
    /// Edges that get no walls (either floor), keyed as in `Grid`.
    hedges: Vec<bool>,
    vedges: Vec<bool>,
}

impl Reserve {
    fn cell(&mut self, ix: usize, iz: usize) {
        self.cells[idx(ix, iz)] = true;
        let (mx, mz) = mirror_cell(ix, iz);
        self.cells[idx(mx, mz)] = true;
    }

    fn edge(&mut self, e: Edge) {
        match e {
            Edge::H { l, ix } => {
                self.hedges[l * W + ix] = true;
                self.hedges[(H - l) * W + W - 1 - ix] = true;
            }
            Edge::V { l, iz } => {
                self.vedges[iz * (W + 1) + l] = true;
                self.vedges[(H - 1 - iz) * (W + 1) + W - l] = true;
            }
        }
    }

    fn has_edge(&self, e: Edge) -> bool {
        match e {
            Edge::H { l, ix } => self.hedges[l * W + ix],
            Edge::V { l, iz } => self.vedges[iz * (W + 1) + l],
        }
    }

    fn has(&self, ix: usize, iz: usize) -> bool {
        self.cells[idx(ix, iz)]
    }
}

/// One piece of the kit placed in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PieceInstance {
    pub kind: PieceKind,
    pub pos: Vec3,
    /// Quarter turns about +Y (`kit::turn`).
    pub rot: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetDef {
    pub id: u8,
    pub team: Team,
    /// The panel's box.
    pub min: Vec3,
    pub max: Vec3,
    /// The way the panel faces (towards its own base).
    pub normal: Vec3,
    /// The cell in front of the alcove's opening.
    pub rear_cell: (usize, usize),
}

impl TargetDef {
    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) / 2.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spawn {
    pub pos: Vec3,
    pub yaw: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoverKind {
    /// Sinks into the floor (open) and rises again (closed).
    Door,
    /// Slides back and forth along its track.
    Panel,
}

/// A moving piece of the arena (see the module documentation). Where it is depends only on
/// the sim tick: `offset(tick)` from `pos`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mover {
    pub kind: MoverKind,
    /// The kit piece to draw (and whose shapes collide).
    pub piece: PieceKind,
    /// Quarter turns about +Y (`kit::turn`).
    pub rot: u8,
    /// The piece's position at rest: a door closed, a panel at the start of its track.
    pub pos: Vec3,
    /// The displacement at the far end of the movement: a door fully open (sunk into the
    /// floor), a panel at the other end of its track.
    pub travel: Vec3,
    /// Ticks held at rest and at the far end, and ticks to move between them (eased in and
    /// out). The cycle: rest, move out, far end, move back.
    pub hold: [u32; 2],
    pub move_ticks: u32,
    /// Ticks into the cycle at tick 0.
    pub phase: u32,
    /// Doors: the two nav nodes the door separates (linked while it's open).
    pub link: Option<(u32, u32)>,
}

impl Mover {
    /// Ticks in a whole cycle.
    pub fn period(&self) -> u32 {
        (self.hold[0] + self.hold[1] + 2 * self.move_ticks).max(1)
    }

    /// How far along its movement the piece is at `tick`: 0 at rest, 1 at the far end.
    pub fn fraction(&self, tick: u64) -> f32 {
        let t = ((tick + self.phase as u64) % self.period() as u64) as u32;
        let ease = |k: u32| {
            let x = k as f32 / self.move_ticks.max(1) as f32;
            x * x * (3.0 - 2.0 * x)
        };
        let [h0, h1] = self.hold;
        let m = self.move_ticks;
        if t < h0 {
            0.0
        } else if t < h0 + m {
            ease(t - h0)
        } else if t < h0 + m + h1 {
            1.0
        } else {
            1.0 - ease(t - h0 - m - h1)
        }
    }

    /// The displacement from `pos` at `tick` (clients: draw the piece at `pos + offset`,
    /// interpolating between ticks).
    pub fn offset(&self, tick: u64) -> Vec3 {
        self.travel * self.fraction(tick)
    }

    /// The colliders with the piece moved `fraction` of its way.
    pub fn colliders(&self, fraction: f32) -> impl Iterator<Item = Collider> + '_ {
        let pos = self.pos + self.travel * fraction;
        self.piece
            .shapes()
            .into_iter()
            .map(move |s| Collider::place(&s, pos, self.rot))
            // A door sunk all the way is under the floor.
            .filter(|c| c.bounds().1.y > 0.01)
    }

    /// Doors: open far enough to walk through.
    pub fn passable(&self, tick: u64) -> bool {
        self.kind == MoverKind::Panel || self.fraction(tick) > 0.92
    }

    /// Doors: ticks until passable (0 if it is now), and how many ticks it stays passable
    /// from then on.
    pub fn passable_window(&self, tick: u64) -> (u32, u32) {
        if self.kind == MoverKind::Panel {
            return (0, u32::MAX);
        }
        // Step through the cycle a few ticks at a time (a cycle is about 1300 ticks).
        let period = self.period();
        let step = 6u32;
        let mut wait = None;
        let mut k = 0u32;
        while k <= period * 2 {
            let open = self.passable(tick + k as u64);
            match wait {
                None if open => wait = Some(k),
                Some(w) if !open => return (w, k - w),
                _ => {}
            }
            k += step;
        }
        (wait.unwrap_or(u32::MAX), u32::MAX)
    }
}

/// Walkable places for bots: cell centres on the ground, on ramps and on the upper floor.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Nav {
    pub pos: Vec<Vec3>,
    pub valid: Vec<bool>,
    /// Links that are always there.
    pub links: Vec<Vec<u32>>,
    /// Links through doors: the two nodes and the door (index into `Level::movers`), usable
    /// only while the door is open.
    pub doors: Vec<(u32, u32, usize)>,
}

impl Nav {
    pub fn ground_node(ix: usize, iz: usize) -> usize {
        idx(ix, iz)
    }

    pub fn upper_node(ix: usize, iz: usize) -> usize {
        W * H + idx(ix, iz)
    }

    /// The nearest valid node to a point (by cell, then by height).
    pub fn node_at(&self, p: Vec3) -> Option<usize> {
        let (ix, iz) = cell_at(p)?;
        let g = Self::ground_node(ix, iz);
        let u = Self::upper_node(ix, iz);
        let pick = |a: usize, b: usize| {
            let da = (self.pos[a].y - p.y).abs();
            let db = (self.pos[b].y - p.y).abs();
            if da <= db { a } else { b }
        };
        match (self.valid[g], self.valid[u]) {
            (true, true) => Some(pick(g, u)),
            (true, false) => Some(g),
            (false, true) => Some(u),
            _ => None,
        }
    }

    /// Breadth-first reachability from `start` (every door closed).
    pub fn reach(&self, start: usize) -> Vec<bool> {
        self.reach_with(start, false)
    }

    /// Breadth-first reachability from `start`, with every door open or every door closed.
    pub fn reach_with(&self, start: usize, doors_open: bool) -> Vec<bool> {
        let mut extra: Vec<Vec<u32>> = vec![Vec::new(); self.pos.len()];
        if doors_open {
            for &(a, b, _) in &self.doors {
                extra[a as usize].push(b);
                extra[b as usize].push(a);
            }
        }
        let mut seen = vec![false; self.pos.len()];
        let mut queue = std::collections::VecDeque::new();
        seen[start] = true;
        queue.push_back(start);
        while let Some(n) = queue.pop_front() {
            for &m in self.links[n].iter().chain(&extra[n]) {
                let m = m as usize;
                if !seen[m] {
                    seen[m] = true;
                    queue.push_back(m);
                }
            }
        }
        seen
    }

    /// Shortest path (in links) from `a` to `b`, both included.
    pub fn path(&self, a: usize, b: usize) -> Option<Vec<usize>> {
        let mut prev = vec![u32::MAX; self.pos.len()];
        let mut queue = std::collections::VecDeque::new();
        prev[a] = a as u32;
        queue.push_back(a);
        while let Some(n) = queue.pop_front() {
            if n == b {
                let mut out = vec![b];
                let mut c = b;
                while c != a {
                    c = prev[c] as usize;
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            for &m in &self.links[n] {
                if prev[m as usize] == u32::MAX {
                    prev[m as usize] = n as u32;
                    queue.push_back(m as usize);
                }
            }
        }
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Level {
    pub seed: u64,
    pub grid: Grid,
    pub pieces: Vec<PieceInstance>,
    pub colliders: Vec<Collider>,
    pub targets: Vec<TargetDef>,
    /// Per team (`Team::index`).
    pub spawns: [Vec<Spawn>; 2],
    /// Reload stations (floor centres); the spawn pads refill blasters too.
    pub reloads: Vec<Vec3>,
    pub nav: Nav,
    /// Timed doors and sliding panels (not in `pieces` or `colliders`).
    pub movers: Vec<Mover>,
}

impl Level {
    /// The arena for `seed` (the same on every machine).
    pub fn generate(seed: u64) -> Level {
        for attempt in 0..500u64 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ attempt);
            if let Some(mut grid) = try_grid(&mut rng) {
                let nav = build_nav(&grid);
                if validate(&grid, &nav) {
                    // Movers only turn existing walls into doors and use open cells, so the
                    // validated arena is the one with every door closed.
                    let movers = place_movers(&mut grid, &nav, &mut rng);
                    let mut nav = build_nav(&grid);
                    for (i, m) in movers.iter().enumerate() {
                        if let Some((a, b)) = m.link {
                            nav.doors.push((a, b, i));
                        }
                    }
                    let level = build_level(seed, grid, nav, movers);
                    if level.targets.iter().all(|t| level.target_spots(t).iter().all(|n| *n >= 2)) {
                        return level;
                    }
                }
            }
        }
        panic!("no valid arena for seed {seed}");
    }

    /// The static arena (no movers).
    pub fn world(&self) -> World {
        World::new(self.colliders.clone())
    }

    /// The static arena with the movers where they are at `tick`.
    pub fn world_at(&self, tick: u64) -> World {
        let mut w = self.world();
        w.set_dynamic(self.mover_colliders(tick));
        w
    }

    /// The displacement of mover `i` from its `pos` at `tick` (zero if there's no such mover).
    pub fn mover_offset(&self, i: usize, tick: u64) -> Vec3 {
        self.movers.get(i).map_or(Vec3::ZERO, |m| m.offset(tick))
    }

    /// Every mover's colliders at `tick` (for `World::set_dynamic`).
    pub fn mover_colliders(&self, tick: u64) -> Vec<Collider> {
        self.movers.iter().flat_map(|m| m.colliders(m.fraction(tick))).collect()
    }

    /// Whether the cell is on a sliding panel's track (nothing should be put there).
    pub fn on_track(&self, ix: usize, iz: usize) -> bool {
        let c = cell_centre(ix, iz);
        self.movers.iter().filter(|m| m.kind == MoverKind::Panel).any(|m| {
            let (a, b) = (m.pos, m.pos + m.travel);
            let (lo, hi) = (a.min(b), a.max(b));
            c.x >= lo.x - 0.1 && c.x <= hi.x + 0.1 && c.z >= lo.z - 0.1 && c.z <= hi.z + 0.1
        })
    }

    /// How many places on the ground and on the upper floor (walkable cell centres, reached
    /// from the bases) a standing player can hit target `t` from.
    pub fn target_spots(&self, t: &TargetDef) -> [usize; 2] {
        let world = self.world();
        let Some(start) = self.spawns[0].first().and_then(|s| self.nav.node_at(s.pos)) else { return [0, 0] };
        let seen = self.nav.reach(start);
        let mut out = [0, 0];
        for (n, p) in self.nav.pos.iter().enumerate() {
            if !self.nav.valid[n] || !seen[n] || p.distance(t.centre()) > 30.0 {
                continue;
            }
            if target_seen(&world, *p + Vec3::Y * crate::sim::EYE, t, t.centre()) {
                out[(n >= W * H) as usize] += 1;
            }
        }
        out
    }

    /// The middle of a team's spawn pads.
    pub fn base_centre(&self, team: Team) -> Vec3 {
        let s = &self.spawns[team.index()];
        s.iter().map(|s| s.pos).sum::<Vec3>() / s.len().max(1) as f32
    }

    /// Whether a player standing at `feet` is refilling: on a reload station or a spawn pad.
    pub fn reloads_at(&self, feet: Vec3) -> bool {
        if feet.y > 0.5 {
            return false;
        }
        cell_at(feet).is_some_and(|(ix, iz)| matches!(self.grid.ground[idx(ix, iz)], Ground::Reload | Ground::Spawn))
    }
}

/// The cells around a target's opening: the target's own (under the upper floor), the opening
/// behind it and the upper floor around the opening (3 x 3 cells).
fn target_surround(ix: usize) -> Vec<(usize, usize)> {
    (TARGET_ROW..TARGET_ROW + 3).flat_map(|z| (ix.saturating_sub(1)..=(ix + 1).min(W - 1)).map(move |x| (x, z))).collect()
}

/// Whether upper-floor cells `a` and `b` are joined by upper floor (walls aside).
fn upper_joined(g: &Grid, a: (usize, usize), b: (usize, usize)) -> bool {
    if !g.upper[idx(a.0, a.1)] {
        return false;
    }
    let mut seen = vec![false; W * H];
    let mut stack = vec![a];
    seen[idx(a.0, a.1)] = true;
    while let Some((x, z)) = stack.pop() {
        if (x, z) == b {
            return true;
        }
        for d in 0..4 {
            if let Some((nx, nz)) = step(x, z, d, 1)
                && g.upper[idx(nx, nz)]
                && !seen[idx(nx, nz)]
            {
                seen[idx(nx, nz)] = true;
                stack.push((nx, nz));
            }
        }
    }
    false
}

/// Whether one can walk on the upper level alone (no ramps, through no walls) from around one
/// team's target to around the other's.
fn upper_crosses(g: &Grid, nav: &Nav) -> bool {
    let Some(ix) = (0..W).find(|&ix| matches!(g.ground[idx(ix, TARGET_ROW)], Ground::Alcove { .. })) else { return false };
    let from = Nav::upper_node(ix, TARGET_ROW + 2);
    let (mx, mz) = mirror_cell(ix, TARGET_ROW + 2);
    let to = Nav::upper_node(mx, mz);
    let mut seen = vec![false; nav.pos.len()];
    let mut stack = vec![from];
    seen[from] = true;
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        for &m in &nav.links[n] {
            let m = m as usize;
            if m >= W * H && !seen[m] {
                seen[m] = true;
                stack.push(m);
            }
        }
    }
    false
}

fn try_grid(rng: &mut ChaCha8Rng) -> Option<Grid> {
    let mut g = Grid::new();
    let mut res = Reserve {
        cells: vec![false; W * H],
        hedges: vec![false; W * (H + 1)],
        vedges: vec![false; (W + 1) * H],
    };

    // 1. The base: a small corner of spawn pads (Red's in a rear corner, Blue's in the
    //    opposite one), walled off with a gate on each open side.
    let left = rng.random_bool(0.5);
    let pad_x: Vec<usize> = if left { (0..BASE_W).collect() } else { (W - BASE_W..W).collect() };
    // The wall along x: on the line between the base's columns and the arena's.
    let side_line = if left { BASE_W } else { W - BASE_W };
    for iz in BASE_ROW - 1..H {
        for ix in pad_x[0].saturating_sub(1)..(pad_x[BASE_W - 1] + 2).min(W) {
            res.cell(ix, iz);
        }
    }
    for iz in BASE_ROW..H {
        for &ix in &pad_x {
            g.set_ground(ix, iz, Ground::Spawn);
        }
    }
    let wall = |rng: &mut ChaCha8Rng| if rng.random_bool(0.4) { Wall::Window } else { Wall::Full };
    let gate_x = pad_x[rng.random_range(0..BASE_W)];
    for &ix in &pad_x {
        let e = Edge::H { l: BASE_ROW, ix };
        if ix == gate_x {
            res.edge(e);
        } else {
            g.set_wall(0, e, wall(rng));
        }
    }
    let gate_z = rng.random_range(BASE_ROW..H);
    for iz in BASE_ROW..H {
        let e = Edge::V { l: side_line, iz };
        if iz == gate_z {
            res.edge(e);
        } else {
            g.set_wall(0, e, wall(rng));
        }
    }

    // 2. Team targets, in the rear quarter: each hangs from a roof over the upper level, in a
    //    cell walled on three sides on both floors and open towards the own rear. Neither that
    //    cell nor the one behind it has an upper floor, so the target is seen from the ground
    //    under and behind it, and from the upper floor around that opening (always there).
    let two = rng.random_bool(0.5);
    let cols: Vec<usize> = if two {
        let c = rng.random_range(2..=3usize);
        vec![c, W - 1 - c]
    } else {
        // Not in front of the own base.
        let c = rng.random_range(4..=5usize);
        vec![if left { W - 1 - c } else { c }]
    };
    for &ix in &cols {
        g.set_ground(ix, TARGET_ROW, Ground::Alcove { facing: 0 });
        for d in [1u8, 2, 3] {
            g.set_wall(0, edge(ix, TARGET_ROW, d), Wall::Full);
        }
        for (x, z) in target_surround(ix) {
            res.cell(x, z);
            if x != ix || z == TARGET_ROW + 2 {
                g.set_upper(x, z, true);
            }
        }
        res.cell(ix, TARGET_ROW + 3);
        res.edge(edge(ix, TARGET_ROW, 0));
        for d in 0..4 {
            res.edge(edge(ix, TARGET_ROW + 1, d));
        }
        // Nothing on the upper floor around the opening.
        for (x, z) in target_surround(ix) {
            for d in 0..4 {
                if let Some((nx, nz)) = step(x, z, d, 1)
                    && target_surround(ix).contains(&(nx, nz))
                {
                    res.edge(edge(x, z, d));
                }
            }
        }
    }

    // 2b. A reload station in the middle third, open along z, walled with windows along x.
    {
        let rz = HALF + rng.random_range(2..=5usize);
        let rx = rng.random_range(2..=W - 3);
        g.set_ground(rx, rz, Ground::Reload);
        res.cell(rx, rz);
        for d in [0u8, 2] {
            res.edge(edge(rx, rz, d));
            if let Some((x, z)) = step(rx, rz, d, 1)
                && z >= HALF
            {
                res.cell(x, z);
            }
        }
        for d in [1u8, 3] {
            g.set_wall(0, edge(rx, rz, d), Wall::Window);
            res.edge(edge(rx, rz, d));
        }
    }

    // 3. The upper floor: a bridge over the centre line, side galleries, a few more platforms.
    let rect = |g: &mut Grid, res: &Reserve, x0: usize, z0: usize, w: usize, l: usize| {
        for iz in z0..(z0 + l).min(UPPER_MAX + 1) {
            for ix in x0..(x0 + w).min(W) {
                if !res.has(ix, iz) && iz < BASE_ROW {
                    g.set_upper(ix, iz, true);
                }
            }
        }
    };
    let bw = rng.random_range(2..=4usize);
    let bx = rng.random_range(2..=W - 2 - bw);
    rect(&mut g, &res, bx, HALF, bw, rng.random_range(2..=4));
    // Its other end over the centre line (Blue's half is turned round: this joins the two).
    for ix in bx..bx + bw {
        if !res.has(ix, HALF - 1) {
            g.set_upper(ix, HALF - 1, true);
        }
    }
    for left in [true, false] {
        if rng.random_bool(0.7) {
            let x0 = if left { 0 } else { W - 2 };
            let z0 = HALF + rng.random_range(1..=4usize);
            rect(&mut g, &res, x0, z0, 2, rng.random_range(4..=7));
        }
    }
    for _ in 0..rng.random_range(1..=3) {
        let (w, l) = (rng.random_range(2..=4usize), rng.random_range(2..=4usize));
        let x0 = rng.random_range(0..=W - w);
        let z0 = rng.random_range(HALF + 1..=UPPER_MAX + 1 - l);
        rect(&mut g, &res, x0, z0, w, l);
    }
    let red_cells = (UPPER_MAX + 1 - HALF) * W;
    let coverage = |g: &Grid| {
        (HALF..=UPPER_MAX).flat_map(|iz| (0..W).map(move |ix| (ix, iz))).filter(|&(ix, iz)| g.upper[idx(ix, iz)]).count()
            as f32
            / red_cells as f32
    };
    let mut tries = 0;
    while coverage(&g) < 0.40 && tries < 8 {
        let (w, l) = (rng.random_range(2..=4usize), rng.random_range(2..=4usize));
        let x0 = rng.random_range(0..=W - w);
        let z0 = rng.random_range(HALF..=UPPER_MAX + 1 - l);
        rect(&mut g, &res, x0, z0, w, l);
        tries += 1;
    }
    if !(0.38..=0.58).contains(&coverage(&g)) {
        return None;
    }
    // A walkway on the upper level from each target's surroundings to the bridge, where the
    // upper level doesn't join them already: the upper level always crosses the arena.
    for &ix in &cols {
        if upper_joined(&g, (ix + 1, TARGET_ROW + 2), (bx, HALF)) {
            continue;
        }
        // Along z beside the target, on the side towards the bridge, then along x.
        let x = if bx + bw / 2 < ix { ix - 1 } else { ix + 1 };
        for z in (HALF..TARGET_ROW).rev() {
            if res.has(x, z) && !g.upper[idx(x, z)] {
                return None;
            }
            g.set_upper(x, z, true);
        }
        let (lo, hi) = if x < bx { (x, bx) } else { (bx + bw - 1, x) };
        for xx in lo..=hi {
            if res.has(xx, HALF) && !g.upper[idx(xx, HALF)] {
                return None;
            }
            g.set_upper(xx, HALF, true);
        }
    }

    // Holes to shoot through, inside the platforms.
    let mut interior: Vec<(usize, usize)> = (HALF + 1..=UPPER_MAX)
        .flat_map(|iz| (0..W).map(move |ix| (ix, iz)))
        .filter(|&(ix, iz)| {
            g.upper[idx(ix, iz)]
                && !res.has(ix, iz)
                && !matches!(g.ground[idx(ix, iz)], Ground::Alcove { .. })
                && (0..4).all(|d| step(ix, iz, d, 1).is_some_and(|(x, z)| g.upper[idx(x, z)]))
        })
        .collect();
    let holes = rng.random_range(2..=4usize);
    for _ in 0..holes {
        if interior.is_empty() {
            break;
        }
        let (ix, iz) = interior.swap_remove(rng.random_range(0..interior.len()));
        g.set_upper(ix, iz, false);
        interior.retain(|&(x, z)| x.abs_diff(ix) + z.abs_diff(iz) > 2);
    }
    // Also open a hole at the edge of the bridge sometimes, so the centre can be shot down into.
    if rng.random_bool(0.5) {
        let ix = rng.random_range(bx..bx + bw);
        if g.upper[idx(ix, HALF)] {
            g.set_upper(ix, HALF, false);
        }
    }

    // 4. Ramps: each platform gets one, the largest two; at least two per half.
    let comps = components(&g);
    let mut comps: Vec<Vec<(usize, usize)>> = comps
        .into_iter()
        .filter(|c| c.iter().any(|&(_, iz)| iz >= HALF))
        .collect();
    comps.sort_by_key(|c| std::cmp::Reverse(c.len()));
    let mut ramps = 0;
    for (n, comp) in comps.iter().enumerate() {
        let want = if n == 0 { 2 } else { 1 };
        let mut placed = 0;
        if comp.len() >= 3 {
            for _ in 0..want {
                if place_ramp(&mut g, &mut res, rng, comp) {
                    placed += 1;
                }
            }
        }
        if placed == 0 {
            // A platform no one can climb onto: remove it.
            for &(ix, iz) in comp {
                if iz >= HALF {
                    g.set_upper(ix, iz, false);
                }
            }
        }
        ramps += placed;
    }
    if ramps < 2 {
        return None;
    }

    // 5. Railings around the upper floor (except where ramps arrive).
    for iz in HALF..=UPPER_MAX {
        for ix in 0..W {
            if !g.upper[idx(ix, iz)] {
                continue;
            }
            for d in 0..4u8 {
                let Some((nx, nz)) = step(ix, iz, d, 1) else { continue };
                let e = edge(ix, iz, d);
                if g.upper[idx(nx, nz)] || res.has_edge(e) && is_ramp_top(&g, ix, iz, d) {
                    continue;
                }
                g.set_wall(1, e, Wall::Rail);
            }
        }
    }

    // The target's cell walled on the upper level too (where the railings would be).
    for ix in 0..W {
        if let Ground::Alcove { .. } = g.ground[idx(ix, TARGET_ROW)] {
            for d in [1u8, 2, 3] {
                g.set_wall(1, edge(ix, TARGET_ROW, d), Wall::Full);
            }
        }
    }

    // 6. Cover on the upper floor.
    let mut upper_edges = Vec::new();
    for iz in HALF..=UPPER_MAX {
        for ix in 0..W {
            for d in [0u8, 1] {
                if let Some((nx, nz)) = step(ix, iz, d, 1)
                    && g.upper[idx(ix, iz)]
                    && g.upper[idx(nx, nz)]
                    && nz <= UPPER_MAX
                    && !(d == 0 && iz == HALF - 1)
                {
                    upper_edges.push(edge(ix, iz, d));
                }
            }
        }
    }
    for _ in 0..rng.random_range(2..=4) {
        if upper_edges.is_empty() {
            break;
        }
        let e = upper_edges.swap_remove(rng.random_range(0..upper_edges.len()));
        if res.has_edge(e) {
            continue;
        }
        let w = match rng.random_range(0..10) {
            0..=4 => Wall::Window,
            5..=8 => Wall::Half,
            _ => Wall::Full,
        };
        g.set_wall(1, e, w);
    }
    for _ in 0..rng.random_range(0..=2) {
        let ix = rng.random_range(0..W);
        let iz = rng.random_range(HALF + 1..=UPPER_MAX);
        if g.upper[idx(ix, iz)] && !res.has(ix, iz) && (0..4).all(|d| !is_ramp_top(&g, ix, iz, d)) {
            g.set_upper_block(ix, iz, true);
        }
    }

    // 7. Cover on the ground: runs of walls, pillars and blocks.
    let free_edge = |g: &Grid, res: &Reserve, e: Edge| -> bool {
        if res.has_edge(e) {
            return false;
        }
        let cells: [(i32, i32); 2] = match e {
            Edge::H { l, ix } => {
                if l == 0 || l >= H {
                    return false;
                }
                [(ix as i32, l as i32 - 1), (ix as i32, l as i32)]
            }
            Edge::V { l, iz } => {
                if l == 0 || l >= W {
                    return false;
                }
                [(l as i32 - 1, iz as i32), (l as i32, iz as i32)]
            }
        };
        cells.iter().all(|&(x, z)| {
            let (x, z) = (x as usize, z as usize);
            !res.has(x, z) && g.ground[idx(x, z)] == Ground::Open
        }) && g.edge_wall(0, e) == Wall::None
    };
    for _ in 0..rng.random_range(10..=14) {
        let horizontal = rng.random_bool(0.5);
        let len = rng.random_range(1..=3usize);
        let wall = match rng.random_range(0..10) {
            0..=2 => Wall::Full,
            3..=6 => Wall::Window,
            _ => Wall::Half,
        };
        let ix = rng.random_range(0..W);
        let iz = rng.random_range(HALF..H - 1);
        for k in 0..len {
            let e = if horizontal {
                Edge::H { l: iz + 1, ix: ix + k }
            } else {
                Edge::V { l: ix + 1, iz: iz + k }
            };
            let in_bounds = match e {
                Edge::H { ix, .. } => ix < W,
                Edge::V { iz, .. } => iz < H,
            };
            if !in_bounds || !free_edge(&g, &res, e) {
                break;
            }
            g.set_wall(0, e, wall);
        }
    }
    // Half walls across the centre line.
    for _ in 0..rng.random_range(1..=2) {
        let ix = rng.random_range(1..W / 2);
        for x in [ix, W - 1 - ix] {
            let e = Edge::H { l: HALF, ix: x };
            if free_edge(&g, &res, e) {
                g.set_wall(0, e, Wall::Half);
            }
        }
    }
    let open_cells = |g: &Grid, res: &Reserve| -> Vec<(usize, usize)> {
        (HALF..H)
            .flat_map(|iz| (0..W).map(move |ix| (ix, iz)))
            .filter(|&(ix, iz)| g.ground[idx(ix, iz)] == Ground::Open && !res.has(ix, iz))
            .collect()
    };
    for (kind, n) in [(Ground::Pillar, rng.random_range(3..=5)), (Ground::Block, rng.random_range(4..=7))] {
        for _ in 0..n {
            let cells = open_cells(&g, &res);
            if cells.is_empty() {
                break;
            }
            let (ix, iz) = cells[rng.random_range(0..cells.len())];
            // Pillars under the upper floor look like supports.
            if kind == Ground::Pillar && !g.upper[idx(ix, iz)] && rng.random_bool(0.6) {
                continue;
            }
            g.set_ground(ix, iz, kind);
        }
    }

    // 8. Break up long open lanes along z (base to base) and along x.
    for _ in 0..40 {
        let Some((ix, iz)) = long_lane(&g, &res) else { break };
        let (ix, iz) = if iz < HALF { mirror_cell(ix, iz) } else { (ix, iz) };
        if g.ground[idx(ix, iz)] == Ground::Open && !res.has(ix, iz) {
            g.set_ground(ix, iz, if rng.random_bool(0.3) { Ground::Pillar } else { Ground::Block });
        } else {
            // Can't block here: reserve it so the next search picks another cell.
            res.cell(ix, iz);
        }
    }
    Some(g)
}

/// Is the edge from upper cell (ix, iz) towards `dir` the top of a ramp?
fn is_ramp_top(g: &Grid, ix: usize, iz: usize, dir: u8) -> bool {
    step(ix, iz, dir, 1).is_some_and(|(x, z)| {
        matches!(g.ground[idx(x, z)], Ground::Ramp { dir: up, seg: 2 } if up == (dir + 2) % 4)
    })
}

/// Places a ramp climbing onto platform `comp` from Red's half. True if one was placed.
fn place_ramp(g: &mut Grid, res: &mut Reserve, rng: &mut ChaCha8Rng, comp: &[(usize, usize)]) -> bool {
    let mut options = Vec::new();
    for &(ix, iz) in comp {
        if iz < HALF || !g.upper[idx(ix, iz)] || g.upper_block[idx(ix, iz)] {
            continue;
        }
        'dirs: for d in 0..4u8 {
            let mut cells = Vec::new();
            for k in 1..=4 {
                let Some((x, z)) = step(ix, iz, d, k) else { continue 'dirs };
                if !(HALF..H).contains(&z) || g.ground[idx(x, z)] != Ground::Open {
                    continue 'dirs;
                }
                // The ramp itself: clear, nothing above it, not reserved.
                if k <= 3 && (g.upper[idx(x, z)] || res.has(x, z)) {
                    continue 'dirs;
                }
                cells.push((x, z));
            }
            // Ramps along x in the centre row would collide with their own mirror image.
            if z_row_conflict(&cells) {
                continue;
            }
            options.push((ix, iz, d, cells));
        }
    }
    if options.is_empty() {
        return false;
    }
    let (ix, iz, d, cells) = options.swap_remove(rng.random_range(0..options.len()));
    let up = (d + 2) % 4;
    for (k, &(x, z)) in cells.iter().take(3).enumerate() {
        g.set_ground(x, z, Ground::Ramp { dir: up, seg: 2 - k as u8 });
        res.cell(x, z);
        for e in 0..4 {
            res.edge(edge(x, z, e));
        }
    }
    let (lx, lz) = cells[3];
    res.cell(lx, lz);
    res.edge(edge(ix, iz, d));
    true
}

fn z_row_conflict(cells: &[(usize, usize)]) -> bool {
    cells.iter().take(3).any(|&(_, z)| z == HALF - 1 || z < HALF)
}

/// Connected groups of upper-floor cells (over the whole arena).
fn components(g: &Grid) -> Vec<Vec<(usize, usize)>> {
    let mut seen = vec![false; W * H];
    let mut out = Vec::new();
    for iz in 0..H {
        for ix in 0..W {
            if !g.upper[idx(ix, iz)] || seen[idx(ix, iz)] {
                continue;
            }
            let mut comp = Vec::new();
            let mut stack = vec![(ix, iz)];
            seen[idx(ix, iz)] = true;
            while let Some((x, z)) = stack.pop() {
                comp.push((x, z));
                for d in 0..4 {
                    if let Some((nx, nz)) = step(x, z, d, 1)
                        && g.upper[idx(nx, nz)]
                        && !seen[idx(nx, nz)]
                    {
                        seen[idx(nx, nz)] = true;
                        stack.push((nx, nz));
                    }
                }
            }
            out.push(comp);
        }
    }
    out
}

/// A cell in the middle of an open ground lane longer than allowed, if any.
fn long_lane(g: &Grid, res: &Reserve) -> Option<(usize, usize)> {
    let open = |ix: usize, iz: usize| matches!(g.ground[idx(ix, iz)], Ground::Open | Ground::Spawn);
    // Along z, across the whole arena.
    for ix in 0..W {
        let mut start = 0;
        let mut run = 0;
        for iz in 0..H {
            let wall = iz > 0 && g.edge_wall(0, Edge::H { l: iz, ix }) != Wall::None;
            if !open(ix, iz) || wall {
                run = 0;
                start = iz + 1;
                if !open(ix, iz) {
                    continue;
                }
                start = iz;
            }
            run += 1;
            if run > 9 {
                // Try cells around the middle of the run, nearest first.
                let mid = start + run / 2;
                for k in 0..run as i32 {
                    let z = mid as i32 + if k % 2 == 0 { k / 2 } else { -(k / 2) - 1 };
                    if z < start as i32 || z > iz as i32 {
                        continue;
                    }
                    let z = z as usize;
                    let (rx, rz) = if z < HALF { mirror_cell(ix, z) } else { (ix, z) };
                    if g.ground[idx(rx, rz)] == Ground::Open && !res.has(rx, rz) {
                        return Some((rx, rz));
                    }
                }
                return Some((ix, start + run / 2));
            }
        }
    }
    // Along x, in Red's half.
    for iz in HALF..H {
        let mut run = 0;
        let mut start = 0;
        for ix in 0..W {
            let wall = ix > 0 && g.edge_wall(0, Edge::V { l: ix, iz }) != Wall::None;
            if !open(ix, iz) || wall {
                run = 0;
                if !open(ix, iz) {
                    start = ix + 1;
                    continue;
                }
                start = ix;
            }
            run += 1;
            if run > 9 {
                for x in start..start + run {
                    let x = (x + run / 3) % W;
                    if g.ground[idx(x, iz)] == Ground::Open && !res.has(x, iz) {
                        return Some((x, iz));
                    }
                }
                return Some((start + run / 2, iz));
            }
        }
    }
    None
}

fn build_nav(g: &Grid) -> Nav {
    let n = 2 * W * H;
    let mut nav = Nav { pos: vec![Vec3::ZERO; n], valid: vec![false; n], links: vec![Vec::new(); n], doors: Vec::new() };
    for iz in 0..H {
        for ix in 0..W {
            let i = idx(ix, iz);
            let c = cell_centre(ix, iz);
            let gn = Nav::ground_node(ix, iz);
            nav.pos[gn] = match g.ground[i] {
                Ground::Ramp { seg, .. } => c + Vec3::Y * (seg as f32 + 0.5) / 3.0 * STOREY,
                _ => c,
            };
            nav.valid[gn] = g.ground[i].walkable();
            let un = Nav::upper_node(ix, iz);
            nav.pos[un] = c + Vec3::Y * STOREY;
            nav.valid[un] = g.upper_walkable(i);
        }
    }
    for iz in 0..H {
        for ix in 0..W {
            let i = idx(ix, iz);
            let gn = Nav::ground_node(ix, iz);
            match g.ground[i] {
                Ground::Ramp { dir, seg } => {
                    // Along the ramp only: down from the low end, up from the high end.
                    if let Some((x, z)) = step(ix, iz, dir, 1) {
                        let next = if seg == 2 { Nav::upper_node(x, z) } else { Nav::ground_node(x, z) };
                        if nav.valid[next] {
                            nav.links[gn].push(next as u32);
                            nav.links[next].push(gn as u32);
                        }
                    }
                    if seg == 0
                        && let Some((x, z)) = step(ix, iz, (dir + 2) % 4, 1)
                    {
                        let prev = Nav::ground_node(x, z);
                        if nav.valid[prev] && !matches!(g.ground[idx(x, z)], Ground::Ramp { .. }) {
                            nav.links[gn].push(prev as u32);
                            nav.links[prev].push(gn as u32);
                        }
                    }
                }
                gr if gr.walkable() => {
                    for d in [0u8, 1] {
                        let Some((x, z)) = step(ix, iz, d, 1) else { continue };
                        let other = g.ground[idx(x, z)];
                        if other.walkable()
                            && !matches!(other, Ground::Ramp { .. })
                            && g.wall(0, ix, iz, d) == Wall::None
                        {
                            let m = Nav::ground_node(x, z);
                            nav.links[gn].push(m as u32);
                            nav.links[m].push(gn as u32);
                        }
                    }
                }
                _ => {}
            }
            let un = Nav::upper_node(ix, iz);
            if nav.valid[un] {
                for d in [0u8, 1] {
                    let Some((x, z)) = step(ix, iz, d, 1) else { continue };
                    let m = Nav::upper_node(x, z);
                    if nav.valid[m] && g.wall(1, ix, iz, d) == Wall::None {
                        nav.links[un].push(m as u32);
                        nav.links[m].push(un as u32);
                    }
                }
            }
        }
    }
    nav
}

/// A spawn pad's cell of `team`.
fn spawn_cell(g: &Grid, team: Team) -> Option<(usize, usize)> {
    (0..H)
        .flat_map(|iz| (0..W).map(move |ix| (ix, iz)))
        .find(|&(ix, iz)| g.ground[idx(ix, iz)] == Ground::Spawn && Team::of_z(cell_centre(ix, iz).z) == team)
}

fn validate(g: &Grid, nav: &Nav) -> bool {
    let (Some(red), Some(blue)) = (spawn_cell(g, Team::Red), spawn_cell(g, Team::Blue)) else { return false };
    let spawn = |team: Team| -> usize {
        let (ix, iz) = if team == Team::Red { red } else { blue };
        Nav::ground_node(ix, iz)
    };
    let seen = nav.reach(spawn(Team::Red));
    if !seen[spawn(Team::Blue)] {
        return false;
    }
    // Both target rears (Blue's targets are what Red has to reach).
    for iz in 0..H {
        for ix in 0..W {
            if let Ground::Alcove { facing } = g.ground[idx(ix, iz)] {
                let Some((x, z)) = step(ix, iz, facing, 1) else { return false };
                if !seen[Nav::ground_node(x, z)] {
                    return false;
                }
            }
            if matches!(g.ground[idx(ix, iz)], Ground::Ramp { .. }) && !seen[Nav::ground_node(ix, iz)] {
                return false;
            }
        }
    }
    if !upper_crosses(g, nav) {
        return false;
    }
    let total = nav.valid.iter().filter(|v| **v).count();
    let reached = nav.valid.iter().zip(&seen).filter(|(v, s)| **v && **s).count();
    let upper_total = (0..W * H).filter(|&i| nav.valid[W * H + i]).count();
    let upper_reached = (0..W * H).filter(|&i| nav.valid[W * H + i] && seen[W * H + i]).count();
    // The centre is reachable on the ground.
    let centre = (0..W).any(|ix| seen[Nav::ground_node(ix, HALF)]);
    centre
        && reached as f32 >= 0.9 * total as f32
        && upper_total > 0
        && upper_reached as f32 >= 0.9 * upper_total as f32
}

/// Door timing (ticks at 60 Hz): 12 s closed, 1 s down, 8 s open, 1 s up.
const DOOR_HOLD: [u32; 2] = [720, 480];
const DOOR_MOVE: u32 = 60;
/// Panels: a 6 s cycle, pausing half a second at each end.
const PANEL_HOLD: [u32; 2] = [30, 30];
const PANEL_MOVE: u32 = 150;

/// A cell's (ix, iz).
type CellXZ = (usize, usize);

/// Picks the doors and sliding panels for Red's half (and mirrors them): marks the doors in
/// the grid (`Wall::Door`) and returns the movers.
fn place_movers(g: &mut Grid, nav: &Nav, rng: &mut ChaCha8Rng) -> Vec<Mover> {
    let near_alcove = |ix: usize, iz: usize| -> bool {
        (iz.saturating_sub(1)..=(iz + 1).min(H - 1))
            .any(|z| (ix.saturating_sub(1)..=(ix + 1).min(W - 1)).any(|x| matches!(g.ground[idx(x, z)], Ground::Alcove { .. })))
    };
    let near_kind = |ix: usize, iz: usize, f: &dyn Fn(Ground) -> bool| -> bool {
        f(g.ground[idx(ix, iz)]) || (0..4).any(|d| step(ix, iz, d, 1).is_some_and(|(x, z)| f(g.ground[idx(x, z)])))
    };
    let open = |ix: usize, iz: usize| g.ground[idx(ix, iz)] == Ground::Open;

    // Doors: walls between two open cells of Red's half, outside the base, away from the
    // targets, where walking around takes a while (the door makes a real shortcut).
    let mut cands: Vec<(f32, Edge, CellXZ, CellXZ)> = Vec::new();
    for iz in HALF + 1..BASE_ROW - 1 {
        for ix in 0..W {
            for d in [0u8, 1] {
                let Some((nx, nz)) = step(ix, iz, d, 1) else { continue };
                let e = edge(ix, iz, d);
                if nz >= BASE_ROW - 1 || !matches!(g.edge_wall(0, e), Wall::Full | Wall::Window | Wall::Half) {
                    continue;
                }
                if !open(ix, iz) || !open(nx, nz) || near_alcove(ix, iz) || near_alcove(nx, nz) {
                    continue;
                }
                let (a, b) = (Nav::ground_node(ix, iz), Nav::ground_node(nx, nz));
                let Some(path) = nav.path(a, b) else { continue };
                let detour = path.len() as f32 - 1.0;
                if detour < 3.0 {
                    continue;
                }
                cands.push((detour.min(12.0) + rng.random::<f32>() * 3.0, e, (ix, iz), (nx, nz)));
            }
        }
    }
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let want = rng.random_range(2..=3usize);
    let mut chosen: Vec<(Edge, CellXZ, CellXZ)> = Vec::new();
    for &(_, e, a, b) in &cands {
        if chosen.len() >= want {
            break;
        }
        let far = chosen.iter().all(|&(_, ca, _)| ca.0.abs_diff(a.0) + ca.1.abs_diff(a.1) >= 3);
        if far {
            chosen.push((e, a, b));
        }
    }
    let mut movers = Vec::new();
    let period = DOOR_HOLD[0] + DOOR_HOLD[1] + 2 * DOOR_MOVE;
    let n_pairs = chosen.len().max(1) as u32;
    let start = rng.random_range(0..period);
    for (k, &(e, a, b)) in chosen.iter().enumerate() {
        let (pos, rot) = match e {
            Edge::H { l, ix } => (Vec3::new((ix as f32 + 0.5) * CELL - HALF_WIDTH, 0.0, l as f32 * CELL - HALF_LENGTH), 0),
            Edge::V { l, iz } => (Vec3::new(l as f32 * CELL - HALF_WIDTH, 0.0, (iz as f32 + 0.5) * CELL - HALF_LENGTH), 1),
        };
        let phase = (start + k as u32 * period / n_pairs) % period;
        let mirror = |(x, z): (usize, usize)| mirror_cell(x, z);
        for (pos, a, b) in [(pos, a, b), (mirror_point(pos), mirror(a), mirror(b))] {
            movers.push(Mover {
                kind: MoverKind::Door,
                piece: PieceKind::WallFull,
                rot,
                pos,
                travel: Vec3::NEG_Y * (STOREY + 0.05),
                hold: DOOR_HOLD,
                move_ticks: DOOR_MOVE,
                phase,
                link: Some((Nav::ground_node(a.0, a.1) as u32, Nav::ground_node(b.0, b.1) as u32)),
            });
        }
    }

    // Sliding panels: a straight track of 2-3 open cells with open cells past both ends and
    // nothing in the way, in the open (not by ramps, stations or targets).
    let mut tracks: Vec<(f32, Vec<CellXZ>, u8)> = Vec::new();
    for iz in HALF + 1..BASE_ROW - 1 {
        for ix in 0..W {
            for d in [0u8, 1] {
                for len in 2..=3i32 {
                    let cells: Option<Vec<(usize, usize)>> = (-1..=len).map(|k| step(ix, iz, d, k)).collect();
                    let Some(cells) = cells else { continue };
                    let ok = cells.iter().all(|&(x, z)| {
                        (HALF + 1..BASE_ROW - 1).contains(&z)
                            && open(x, z)
                            && !near_alcove(x, z)
                            && !near_kind(x, z, &|gr| matches!(gr, Ground::Ramp { .. } | Ground::Reload | Ground::Spawn))
                    }) && cells.windows(2).all(|w| g.wall(0, w[0].0, w[0].1, d) == Wall::None);
                    if !ok {
                        continue;
                    }
                    // Prefer open surroundings: count open neighbours of the track.
                    let side = (d + 1) % 4;
                    let roomy = cells[1..cells.len() - 1]
                        .iter()
                        .flat_map(|&(x, z)| [step(x, z, side, 1), step(x, z, side + 2, 1)])
                        .flatten()
                        .filter(|&(x, z)| g.ground[idx(x, z)].walkable())
                        .count();
                    tracks.push((roomy as f32 + rng.random::<f32>() * 2.0, cells[1..cells.len() - 1].to_vec(), d));
                }
            }
        }
    }
    tracks.sort_by(|a, b| b.0.total_cmp(&a.0));
    let want = rng.random_range(1..=2usize);
    let mut used: Vec<(usize, usize)> = chosen.iter().flat_map(|&(_, a, b)| [a, b]).collect();
    let mut placed = 0;
    for (_, cells, d) in tracks {
        if placed >= want {
            break;
        }
        if cells.iter().any(|&(x, z)| used.iter().any(|&(ux, uz)| ux.abs_diff(x) + uz.abs_diff(z) < 3)) {
            continue;
        }
        used.extend(&cells);
        placed += 1;
        let under_upper = cells.iter().any(|&(x, z)| g.upper[idx(x, z)]);
        let piece = if under_upper || rng.random_bool(0.5) { PieceKind::WallHalf } else { PieceKind::WallWindow };
        let first = cell_centre(cells[0].0, cells[0].1);
        let last = cell_centre(cells[cells.len() - 1].0, cells[cells.len() - 1].1);
        // Lengthwise along the track: a wall's length runs along X at rot 0.
        let rot = if d == 1 { 0 } else { 1 };
        let phase = rng.random_range(0..PANEL_HOLD[0] + PANEL_HOLD[1] + 2 * PANEL_MOVE);
        for (pos, travel) in [(first, last - first), (mirror_point(first), mirror_point(last - first))] {
            movers.push(Mover {
                kind: MoverKind::Panel,
                piece,
                rot,
                pos,
                travel,
                hold: PANEL_HOLD,
                move_ticks: PANEL_MOVE,
                phase,
                link: None,
            });
        }
    }
    // The doors' walls (the grid was only looked at until here).
    for &(e, _, _) in &chosen {
        g.set_wall(0, e, Wall::Door);
    }
    movers
}

fn build_level(seed: u64, g: Grid, nav: Nav, movers: Vec<Mover>) -> Level {
    let mut pieces = Vec::new();
    let mut targets = Vec::new();
    let mut spawns: [Vec<Spawn>; 2] = [Vec::new(), Vec::new()];
    let mut reloads = Vec::new();
    let up = Vec3::Y * STOREY;
    for iz in 0..H {
        for ix in 0..W {
            let i = idx(ix, iz);
            let c = cell_centre(ix, iz);
            pieces.push(PieceInstance { kind: PieceKind::FloorTile, pos: c, rot: 0 });
            match g.ground[i] {
                Ground::Open => {}
                Ground::Spawn => {
                    pieces.push(PieceInstance { kind: PieceKind::SpawnPad, pos: c, rot: 0 });
                    let team = Team::of_z(c.z);
                    // Facing the middle of the arena.
                    let yaw = c.x.atan2(c.z);
                    spawns[team.index()].push(Spawn { pos: c, yaw });
                }
                Ground::Reload => reloads.push(c),
                Ground::Pillar => pieces.push(PieceInstance { kind: PieceKind::Pillar, pos: c, rot: 0 }),
                Ground::Block => pieces.push(PieceInstance { kind: PieceKind::Block, pos: c, rot: 0 }),
                Ground::Ramp { dir, seg } => {
                    if seg == 1 {
                        pieces.push(PieceInstance { kind: PieceKind::Ramp, pos: c, rot: dir });
                    }
                }
                Ground::Alcove { facing } => {
                    // The roof the target hangs from, over the upper level.
                    pieces.push(PieceInstance { kind: PieceKind::UpperTile, pos: c + up * 2.0, rot: 0 });
                    let (pmin, pmax) = kit::target_box();
                    let (a, b) = kit::rotate_box(pmin, pmax, facing);
                    let (rx, rz) = step(ix, iz, facing, 1).unwrap_or((ix, iz));
                    targets.push(TargetDef {
                        id: 0,
                        team: Team::of_z(c.z),
                        min: a + c,
                        max: b + c,
                        normal: kit::turn(facing) * Vec3::Z,
                        rear_cell: (rx, rz),
                    });
                }
            }
            if g.upper[i] {
                pieces.push(PieceInstance { kind: PieceKind::UpperTile, pos: c + up, rot: 0 });
            }
            if g.upper_block[i] {
                pieces.push(PieceInstance { kind: PieceKind::Block, pos: c + up, rot: 0 });
            }
        }
    }
    // Walls on edges, and posts where they meet.
    for floor in 0..2 {
        let y = Vec3::Y * STOREY * floor as f32;
        for l in 1..H {
            for ix in 0..W {
                if let Some(kind) = g.hwall[floor][l * W + ix].piece() {
                    let pos = Vec3::new((ix as f32 + 0.5) * CELL - HALF_WIDTH, 0.0, l as f32 * CELL - HALF_LENGTH);
                    pieces.push(PieceInstance { kind, pos: pos + y, rot: 0 });
                }
            }
        }
        for iz in 0..H {
            for l in 1..W {
                if let Some(kind) = g.vwall[floor][iz * (W + 1) + l].piece() {
                    let pos = Vec3::new(l as f32 * CELL - HALF_WIDTH, 0.0, (iz as f32 + 0.5) * CELL - HALF_LENGTH);
                    pieces.push(PieceInstance { kind, pos: pos + y, rot: 1 });
                }
            }
        }
        for l in 1..H {
            for vx in 1..W {
                let touching = [
                    g.hwall[floor][l * W + vx - 1],
                    g.hwall[floor][l * W + vx],
                    g.vwall[floor][(l - 1) * (W + 1) + vx],
                    g.vwall[floor][l * (W + 1) + vx],
                ];
                let kind = if touching.iter().any(|w| w.tall()) {
                    PieceKind::PostFull
                } else if touching.iter().any(|w| *w != Wall::None) {
                    PieceKind::PostLow
                } else {
                    continue;
                };
                let pos = Vec3::new(vx as f32 * CELL - HALF_WIDTH, 0.0, l as f32 * CELL - HALF_LENGTH);
                pieces.push(PieceInstance { kind, pos: pos + y, rot: 0 });
            }
        }
    }
    // The border, its inner face on the arena's edge.
    let o = kit::BORDER_T / 2.0;
    for ix in 0..W {
        let x = (ix as f32 + 0.5) * CELL - HALF_WIDTH;
        pieces.push(PieceInstance { kind: PieceKind::BorderWall, pos: Vec3::new(x, 0.0, -HALF_LENGTH - o), rot: 0 });
        pieces.push(PieceInstance { kind: PieceKind::BorderWall, pos: Vec3::new(x, 0.0, HALF_LENGTH + o), rot: 2 });
    }
    for iz in 0..H {
        let z = (iz as f32 + 0.5) * CELL - HALF_LENGTH;
        pieces.push(PieceInstance { kind: PieceKind::BorderWall, pos: Vec3::new(-HALF_WIDTH - o, 0.0, z), rot: 1 });
        pieces.push(PieceInstance { kind: PieceKind::BorderWall, pos: Vec3::new(HALF_WIDTH + o, 0.0, z), rot: 3 });
    }
    for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        pieces.push(PieceInstance {
            kind: PieceKind::BorderPost,
            pos: Vec3::new(sx * (HALF_WIDTH + o), 0.0, sz * (HALF_LENGTH + o)),
            rot: 0,
        });
    }
    // Number the targets so Red's and Blue's mirror each other: Red's by x, then Blue's.
    targets.sort_by(|a, b| a.team.index().cmp(&b.team.index()).then(a.centre().x.total_cmp(&b.centre().x)));
    for (i, t) in targets.iter_mut().enumerate() {
        t.id = i as u8;
    }

    let mut colliders = vec![Collider::Box {
        min: Vec3::new(-HALF_WIDTH - 4.0, -1.0, -HALF_LENGTH - 4.0),
        max: Vec3::new(HALF_WIDTH + 4.0, 0.0, HALF_LENGTH + 4.0),
    }];
    for p in &pieces {
        for s in p.kind.shapes() {
            colliders.push(Collider::place(&s, p.pos, p.rot));
        }
    }
    for t in &targets {
        colliders.push(Collider::Box { min: t.min, max: t.max });
    }
    Level { seed, grid: g, pieces, colliders, targets, spawns, reloads, nav, movers }
}

/// Whether a bolt from `eye` to `aim` (a point of target `t`) reaches the target first.
pub fn target_seen(world: &World, eye: Vec3, t: &TargetDef, aim: Vec3) -> bool {
    let dir = (aim - eye).normalize_or_zero();
    world
        .ray(eye, aim + dir * 0.2)
        .is_some_and(|(_, q, _)| q.cmpge(t.min - 0.01).all() && q.cmple(t.max + 0.01).all())
}

/// A colour for debugging dumps: Red's half red, Blue's blue.
pub fn team_tint(z: f32) -> Vec4 {
    if z >= 0.0 { Vec4::new(1.0, 0.1, 0.1, 1.0) } else { Vec4::new(0.1, 0.3, 1.0, 1.0) }
}

/// An ASCII picture of the arena (ground floor / upper floor side by side) for tests and logs.
pub fn ascii(level: &Level) -> String {
    let g = &level.grid;
    let mut s = String::new();
    for iz in (0..H).rev() {
        for ix in 0..W {
            s.push(match g.ground[idx(ix, iz)] {
                Ground::Open => '.',
                Ground::Spawn => 'S',
                Ground::Pillar => 'o',
                Ground::Block => '#',
                Ground::Ramp { dir, .. } => ['^', '>', 'v', '<'][dir as usize],
                Ground::Alcove { .. } => 'T',
                Ground::Reload => 'R',
            });
        }
        s.push_str("   ");
        for ix in 0..W {
            let i = idx(ix, iz);
            s.push(if g.upper_block[i] {
                '#'
            } else if g.upper[i] {
                '='
            } else {
                ' '
            });
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> impl Iterator<Item = u64> {
        0..200u64
    }

    #[test]
    fn levels_are_symmetric_and_connected() {
        for seed in seeds() {
            let level = Level::generate(seed);
            let g = &level.grid;
            for iz in 0..H {
                for ix in 0..W {
                    let (mx, mz) = mirror_cell(ix, iz);
                    let a = g.ground[idx(ix, iz)];
                    let b = g.ground[idx(mx, mz)];
                    assert_eq!(a.mirrored(), b, "seed {seed} cell {ix},{iz}");
                    assert_eq!(g.upper[idx(ix, iz)], g.upper[idx(mx, mz)]);
                }
            }
            // Pieces mirror each other.
            for p in &level.pieces {
                let m = mirror_point(p.pos);
                assert!(
                    level.pieces.iter().any(|q| q.kind == p.kind && (q.pos - m).length() < 1e-3),
                    "seed {seed}: {p:?} has no mirror image"
                );
            }
            // Two floors, and the roofs over the targets.
            assert!(
                level.pieces.iter().all(|p| p.pos.y <= STOREY + 1e-3 || p.kind == PieceKind::UpperTile && p.pos.y <= 2.0 * STOREY + 1e-3),
                "at most two floors"
            );
        }
    }

    #[test]
    fn levels_have_ramps_targets_and_holes() {
        for seed in seeds() {
            let level = Level::generate(seed);
            let g = &level.grid;
            let ramps_red = level.pieces.iter().filter(|p| p.kind == PieceKind::Ramp && p.pos.z > 0.0).count();
            let ramps_blue = level.pieces.iter().filter(|p| p.kind == PieceKind::Ramp && p.pos.z < 0.0).count();
            assert!(ramps_red >= 2 && ramps_blue >= 2, "seed {seed}: ramps {ramps_red}/{ramps_blue}\n{}", ascii(&level));
            let red_targets = level.targets.iter().filter(|t| t.team == Team::Red).count();
            assert!((1..=2).contains(&red_targets), "seed {seed}");
            assert_eq!(level.targets.len(), red_targets * 2);
            // Open areas in the upper floor (cells without floor surrounded by floor on 2+ sides).
            let holes = (0..W * H)
                .filter(|&i| {
                    let (ix, iz) = (i % W, i / W);
                    !g.upper[i]
                        && (0..4)
                            .filter(|&d| step(ix, iz, d, 1).is_some_and(|(x, z)| g.upper[idx(x, z)]))
                            .count()
                            >= 2
                })
                .count();
            assert!(holes >= 2, "seed {seed}: no openings in the upper floor\n{}", ascii(&level));
            assert_eq!(level.spawns[0].len(), level.spawns[1].len());
            assert_eq!(level.reloads.len(), 2, "seed {seed}: a reload station per half");
            for r in &level.reloads {
                let n = level.nav.node_at(*r).unwrap();
                assert!(level.nav.reach(spawn_node(&level, Team::Blue))[n], "seed {seed}: station at {r} unreachable");
            }
            assert!(level.spawns[0].len() >= 6);
            // The bases: small, in opposite corners.
            for team in Team::ALL {
                let c = level.base_centre(team);
                assert!(c.x.abs() > HALF_WIDTH - 3.5 && c.z.abs() > HALF_LENGTH - 2.5, "seed {seed}: base at {c}");
                assert!(level.spawns[team.index()].iter().all(|s| s.pos.distance(c) < 3.0));
            }
            let (r, b) = (level.base_centre(Team::Red), level.base_centre(Team::Blue));
            assert!((r + b).length() < 1e-3 && r.x * b.x < 0.0, "seed {seed}: opposite corners {r} {b}");
        }
    }

    fn spawn_node(level: &Level, team: Team) -> usize {
        level.nav.node_at(level.spawns[team.index()][0].pos).unwrap()
    }

    /// The arena with every door closed (and panels at rest) or every door open (panels at
    /// the far end).
    fn world_doors(level: &Level, open: bool) -> World {
        let mut w = level.world();
        let f = if open { 1.0 } else { 0.0 };
        w.set_dynamic(level.movers.iter().flat_map(|m| m.colliders(f)).collect());
        w
    }

    #[test]
    fn movers_are_mirrored_and_placed_sensibly() {
        let (mut doors, mut panels) = (0, 0);
        for seed in seeds() {
            let level = Level::generate(seed);
            for m in &level.movers {
                let mirror = mirror_point(m.pos);
                let travel = mirror_point(m.travel);
                assert!(
                    level.movers.iter().any(|o| o.kind == m.kind
                        && o.piece == m.piece
                        && o.phase == m.phase
                        && (o.pos - mirror).length() < 1e-3
                        && (o.travel - travel).length() < 1e-3),
                    "seed {seed}: {m:?} has no mirror image"
                );
                // Not drawn as a static piece.
                assert!(!level.pieces.iter().any(|p| (p.pos - m.pos).length() < 1e-3 && p.kind == m.piece));
                // Out of the bases, away from the targets.
                assert!(m.pos.z.abs() < HALF_LENGTH - 3.0 * CELL - 0.1, "seed {seed}: {m:?} in a base");
                for t in &level.targets {
                    let c = t.centre();
                    assert!(
                        Vec3::new(c.x - m.pos.x, 0.0, c.z - m.pos.z).length() > 2.0,
                        "seed {seed}: {m:?} next to target {}",
                        t.id
                    );
                }
                match m.kind {
                    MoverKind::Door => {
                        doors += 1;
                        let (a, b) = m.link.unwrap();
                        assert!(level.nav.doors.iter().any(|d| (d.0, d.1) == (a, b)));
                        assert!(!level.nav.links[a as usize].contains(&b), "a door's link is not always there");
                        // Closed: a full wall; open: nothing above the floor.
                        assert!(m.colliders(0.0).count() > 0 && m.colliders(1.0).count() == 0);
                    }
                    MoverKind::Panel => panels += 1,
                }
                // The cycle: at rest, at the far end, and back.
                let p = m.period() as u64;
                assert_eq!(m.offset(p * 7), m.offset(0));
                let fr: Vec<f32> = (0..p).map(|t| m.fraction(t)).collect();
                assert!(fr.contains(&0.0) && fr.contains(&1.0));
                assert!(fr.windows(2).all(|w| (w[1] - w[0]).abs() < 0.05), "smooth");
            }
        }
        let n = seeds().count();
        println!("{} door pairs, {} panel pairs per arena", doors as f32 / 2.0 / n as f32, panels as f32 / 2.0 / n as f32);
        assert!(doors as f32 / 2.0 / n as f32 >= 2.0, "door pairs");
        assert!(panels as f32 / 2.0 / n as f32 >= 1.0, "panel pairs");
    }

    #[test]
    fn levels_connected_with_doors_open_and_closed() {
        for seed in seeds() {
            let level = Level::generate(seed);
            for open in [false, true] {
                let seen = level.nav.reach_with(spawn_node(&level, Team::Red), open);
                assert!(seen[spawn_node(&level, Team::Blue)], "seed {seed}, doors open {open}");
                for t in &level.targets {
                    assert!(seen[Nav::ground_node(t.rear_cell.0, t.rear_cell.1)], "seed {seed}, doors open {open}");
                }
            }
            // A door's two sides are connected the long way round too.
            for &(a, b, _) in &level.nav.doors {
                assert!(level.nav.path(a as usize, b as usize).is_some(), "seed {seed}: a door is the only way");
            }
        }
    }

    #[test]
    fn panels_never_overlap_the_arena() {
        for seed in seeds() {
            let level = Level::generate(seed);
            let world = level.world();
            for m in level.movers.iter().filter(|m| m.kind == MoverKind::Panel) {
                for k in 0..=40 {
                    for c in m.colliders(k as f32 / 40.0) {
                        let (a, b) = c.bounds();
                        let (a, b) = (a + 0.002, b - 0.002);
                        for (i, s) in world.colliders.iter().enumerate() {
                            if i == 0 {
                                continue; // the ground
                            }
                            let (sa, sb) = s.bounds();
                            let overlap = a.cmplt(sb).all() && b.cmpgt(sa).all();
                            assert!(!overlap, "seed {seed}: panel {m:?} at {k}/40 overlaps {s:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn targets_hit_only_from_the_rear_quarter() {
        for seed in 0..60u64 {
            let level = Level::generate(seed);
            for world in [world_doors(&level, false), world_doors(&level, true)] {
                for t in &level.targets {
                    let side = t.team.side();
                    for (n, p) in level.nav.pos.iter().enumerate() {
                        // Out of the rear quarter of the target's half.
                        if !level.nav.valid[n] || p.z * side >= HALF_LENGTH / 2.0 - 0.01 {
                            continue;
                        }
                        for h in [0.5, 1.0, 1.7] {
                            let eye = *p + Vec3::Y * h;
                            for k in 0..9 {
                                let f = Vec3::new((k % 3) as f32 / 2.0, (k / 3) as f32 / 2.0, ((k * 7) % 3) as f32 / 2.0);
                                let aim = t.min + (t.max - t.min) * (f * 0.9 + 0.05);
                                assert!(!target_seen(&world, eye, t, aim), "seed {seed}: target {} hit from {eye}", t.id);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn targets_hang_from_the_ceiling_and_are_seen_from_both_floors() {
        for seed in seeds() {
            let level = Level::generate(seed);
            for t in &level.targets {
                // Just under the roof over the upper level.
                let roof = 2.0 * STOREY - kit::FLOOR_T;
                assert!(t.max.y < roof && t.max.y > roof - 0.5, "seed {seed}");
                assert!(
                    level.pieces.iter().any(|p| p.kind == PieceKind::UpperTile
                        && (p.pos - Vec3::new(t.centre().x, 2.0 * STOREY, t.centre().z)).length() < 1.1),
                    "seed {seed}: no roof over target {}",
                    t.id
                );
                let [ground, upper] = level.target_spots(t);
                assert!(ground >= 2 && upper >= 2, "seed {seed}: target {} seen from {ground} / {upper}\n{}", t.id, ascii(&level));
                // In the rear quarter.
                assert!(t.centre().z * t.team.side() >= HALF_LENGTH / 2.0, "seed {seed}");
            }
        }
    }

    #[test]
    fn the_upper_level_crosses_the_arena() {
        for seed in seeds() {
            let level = Level::generate(seed);
            assert!(upper_crosses(&level.grid, &level.nav), "seed {seed}\n{}", ascii(&level));
        }
    }

    #[test]
    fn same_seed_same_level() {
        let a = Level::generate(42);
        let b = Level::generate(42);
        assert_eq!(a.pieces, b.pieces);
        assert_ne!(Level::generate(43).pieces, a.pieces);
    }

    #[test]
    fn print_one() {
        let level = Level::generate(1);
        println!("{}", ascii(&level));
        for t in &level.targets {
            println!("target {} {:?} at {} facing {}", t.id, t.team, t.centre(), t.normal);
        }
    }
}

//! Consoles: pedestals in the arena (`Level::consoles`) with a click puzzle
//! (`crate::minigame`) on their +z and -z sides, like the turret boxes'. Solving one does
//! something for the solver's team, by its kind (`ConsoleKind`; the sim's `sim::consoles`).
//!
//! The puzzles a match has (`console_defs`): the arena's pedestals, then in Megatarget mode
//! the secondary targets' panels where they are puzzles (`crate::rules::SecondaryKind`), and
//! the megatarget's own console in its room. Clients work them out the same way from the
//! level and the rules, so only the puzzles' seeds and states go over the network.

use glam::Vec3;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::level::{
    ArenaSize, BOX_H, BOX_HALF, GameMode, Grid, Ground, Level, Mover, Wall, mirror_point,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConsoleKind {
    /// A supply vault: hands the solver a special weapon, then resets for a minute.
    Vault,
    /// A sensor mast: for `MAST_TIME` it pulses for the solver's team, showing the enemies
    /// within `MAST_RANGE` through walls (as a pulse sensor, further); nothing switches it off
    /// before then, and nobody can take it over.
    Mast,
    /// A forward beacon: the solver's team respawns at it (`BEACON_SHARE` of the time, while
    /// no enemy is within `BEACON_CLEAR`) until the other team takes it over with its puzzle.
    /// Not in Infection.
    Beacon,
    /// A blackout breaker: for `BREAKER_TIME` the arena's lights go dark in the quarter it
    /// stands in (`quarter`; as a blackout there: only shots, players and the like glow).
    Breaker,
    /// A light bridge's switch: for `BRIDGE_TIME` a bridge of hard light (`Level::bridges`, the
    /// one nearest it) spans a gap between two parts of the upper floor, for anyone to cross.
    Bridge,
}

/// Seconds a light bridge stands.
pub const BRIDGE_TIME: f32 = 25.0;
/// How far from its bridge's middle (m) a light bridge's console stands at most.
const BRIDGE_NEAR: f32 = 9.0;

/// A light bridge over a gap in the upper floor (1 to 3 cells, open floor below): the slab it
/// is while it stands, and its ends (feet on the floor either side).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    pub min: Vec3,
    pub max: Vec3,
    pub ends: [Vec3; 2],
}

impl Bridge {
    pub fn mid(&self) -> Vec3 {
        (self.min + self.max) / 2.0
    }

    fn mirrored(&self) -> Bridge {
        let (a, b) = (mirror_point(self.min), mirror_point(self.max));
        Bridge {
            min: a.min(b),
            max: a.max(b),
            ends: self.ends.map(mirror_point),
        }
    }
}

/// The gaps a light bridge could span in Red's half: straight across 1 to 3 cells with no upper
/// floor (and open ground below), between upper floor walkable at both ends, with nothing on
/// the upper floor's edges in the way.
fn bridge_spans(g: &Grid) -> Vec<Bridge> {
    use crate::kit::{CELL, STOREY};
    let dm = g.dims;
    let walk = |ix: usize, iz: usize| {
        let i = dm.idx(ix, iz);
        g.upper[i] && !g.upper_block[i]
    };
    let gap = |ix: usize, iz: usize| {
        let i = dm.idx(ix, iz);
        !g.upper[i] && g.ground[i] == Ground::Open
    };
    let mut out = Vec::new();
    for iz in dm.half() + 1..dm.h - 2 {
        for ix in 0..dm.w {
            if !walk(ix, iz) {
                continue;
            }
            // Along +x (dir 1) and +z (dir 0).
            for dir in [1u8, 0] {
                for len in 1..=3 {
                    let Some(end) = dm.step(ix, iz, dir, len + 1) else {
                        continue;
                    };
                    let cells: Vec<(usize, usize)> =
                        (1..=len).filter_map(|k| dm.step(ix, iz, dir, k)).collect();
                    if cells.len() != len as usize
                        || !walk(end.0, end.1)
                        || !cells.iter().all(|&(x, z)| gap(x, z))
                        || end.1 <= dm.half()
                    {
                        continue;
                    }
                    // Nothing on the upper floor's edges along the way (but railings, which
                    // the bridge's ends open: `place_consoles`).
                    let path = std::iter::once((ix, iz)).chain(cells.iter().copied());
                    if path
                        .clone()
                        .any(|(x, z)| !matches!(g.wall(1, x, z, dir), Wall::None | Wall::Rail))
                    {
                        continue;
                    }
                    let a = dm.cell_centre(ix, iz);
                    let b = dm.cell_centre(end.0, end.1);
                    let (lo, hi) = (a.min(b), a.max(b));
                    let w = CELL / 2.0 - 0.2;
                    let side = if dir == 1 {
                        Vec3::new(0.0, 0.0, w)
                    } else {
                        Vec3::new(w, 0.0, 0.0)
                    };
                    // (From the near ends' edges, a little onto the floor either side.)
                    let along = if dir == 1 { Vec3::X } else { Vec3::Z } * (CELL / 2.0 - 0.1);
                    out.push(Bridge {
                        min: lo + along - side + Vec3::Y * (STOREY - 0.12),
                        max: hi - along + side + Vec3::Y * STOREY,
                        ends: [a + Vec3::Y * STOREY, b + Vec3::Y * STOREY],
                    });
                    break;
                }
            }
        }
    }
    out
}

/// Seconds a blackout breaker keeps its quarter dark.
pub const BREAKER_TIME: f32 = 20.0;

/// The arena's quarter a point is in (by the centre lines): 0 at +x +z, 1 at +x -z, 2 at -x
/// +z, 3 at -x -z.
pub fn quarter(p: Vec3) -> usize {
    (p.x < 0.0) as usize * 2 + (p.z < 0.0) as usize
}

/// How often a team respawns at its beacon rather than at its base, and how far enemies must
/// be from it (m).
pub const BEACON_SHARE: f32 = 0.6;
pub const BEACON_CLEAR: f32 = 8.0;

/// Seconds a sensor mast works for its team, and how far its pulses reach (m).
pub const MAST_TIME: f32 = 20.0;
pub const MAST_RANGE: f32 = 18.0;

impl ConsoleKind {
    /// The kinds standing in the arena (on pedestals).
    pub const PEDESTALS: [ConsoleKind; 5] = [
        ConsoleKind::Vault,
        ConsoleKind::Mast,
        ConsoleKind::Beacon,
        ConsoleKind::Breaker,
        ConsoleKind::Bridge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ConsoleKind::Vault => "supply vault",
            ConsoleKind::Mast => "sensor mast",
            ConsoleKind::Beacon => "forward beacon",
            ConsoleKind::Breaker => "blackout breaker",
            ConsoleKind::Bridge => "light bridge",
        }
    }

    /// Seconds it rests after being solved (no clicks: resetting).
    pub fn rest(self) -> f32 {
        match self {
            ConsoleKind::Vault => 60.0,
            // (Locked while it works, and a while after.)
            ConsoleKind::Mast => MAST_TIME + 20.0,
            // (A moment, so it doesn't flip back and forth.)
            ConsoleKind::Beacon => 5.0,
            ConsoleKind::Breaker => BREAKER_TIME + 30.0,
            ConsoleKind::Bridge => BRIDGE_TIME + 10.0,
        }
    }

    /// A short name (testing: `LT_DEV=warp-console=vault`).
    pub fn id(self) -> &'static str {
        match self {
            ConsoleKind::Vault => "vault",
            ConsoleKind::Mast => "mast",
            ConsoleKind::Beacon => "beacon",
            ConsoleKind::Breaker => "breaker",
            ConsoleKind::Bridge => "bridge",
        }
    }

    /// Whether the arena gets this kind in `mode`.
    pub fn in_mode(self, mode: GameMode) -> bool {
        !(self == ConsoleKind::Beacon && mode == GameMode::Infection)
    }
}

/// A console's pedestal in the arena: the middle of its foot, on the floor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Console {
    pub pos: Vec3,
    pub kind: ConsoleKind,
}

/// A pedestal is a turret box's size: a half wall, seen and shot over.
pub const CONSOLE_HALF: f32 = BOX_HALF;
pub const CONSOLE_H: f32 = BOX_H;

impl Console {
    pub fn bounds(&self) -> (Vec3, Vec3) {
        (
            self.pos - Vec3::new(CONSOLE_HALF, 0.0, CONSOLE_HALF),
            self.pos + Vec3::new(CONSOLE_HALF, CONSOLE_H, CONSOLE_HALF),
        )
    }

    /// The puzzle on side `face` (0: +z, 1: -z), columns along +x on both (the one seen from
    /// behind the other, as on the turret boxes).
    pub fn frame(&self, face: usize) -> PanelFrame {
        let n = if face == 0 { Vec3::Z } else { Vec3::NEG_Z };
        PanelFrame {
            centre: self.pos + n * CONSOLE_HALF + Vec3::Y * (CONSOLE_H - 0.54),
            right: Vec3::X,
            up: Vec3::Y,
            normal: n,
        }
    }
}

/// Where a puzzle lies: the middle of its square, the ways its columns and rows run, and the
/// way its face looks out.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelFrame {
    pub centre: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub normal: Vec3,
}

impl PanelFrame {
    /// The middle of cell (column, row) of a square `n` cells of `cell` m across (cells
    /// outside the square too).
    pub fn cell_pos(&self, n: usize, cell: f32, c: f32, r: f32) -> Vec3 {
        let h = n as f32 / 2.0;
        self.centre + self.right * ((c + 0.5 - h) * cell) + self.up * ((r + 0.5 - h) * cell)
    }

    /// The cell (column, row; fractional) at a point on the face.
    pub fn cell_at(&self, n: usize, cell: f32, p: Vec3) -> (f32, f32) {
        let h = n as f32 / 2.0;
        let d = p - self.centre;
        (d.dot(self.right) / cell + h, d.dot(self.up) / cell + h)
    }

    /// Where a ray from `from` along `dir` (normalised) meets the face from in front, within
    /// `reach`: how far along and the point.
    pub fn ray(&self, from: Vec3, dir: Vec3, reach: f32) -> Option<(f32, Vec3)> {
        let ahead = (from - self.centre).dot(self.normal);
        let along = dir.dot(self.normal);
        if ahead <= 0.0 || along >= 0.0 {
            return None;
        }
        let t = ahead / -along;
        (t < reach).then(|| (t, from + dir * t))
    }
}

/// A puzzle in play: what solving it does, where its puzzles are (one or two faces), and the
/// box a player's feet must be near to click it (`floor`: the floor they stand on).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConsoleDef {
    pub kind: ConsoleKind,
    pub pos: Vec3,
    pub bounds: (Vec3, Vec3),
    pub floor: f32,
    pub faces: [PanelFrame; 2],
    pub nfaces: usize,
}

impl ConsoleDef {
    pub fn faces(&self) -> &[PanelFrame] {
        &self.faces[..self.nfaces]
    }

    /// Whether feet at `feet` are close enough to click it.
    pub fn in_reach(&self, feet: Vec3) -> bool {
        let (min, max) = self.bounds;
        let gap = glam::Vec2::new(
            (min.x - feet.x).max(feet.x - max.x).max(0.0),
            (min.z - feet.z).max(feet.z - max.z).max(0.0),
        );
        gap.length() <= crate::sim::POKE_REACH && (feet.y - self.floor).abs() <= 0.8
    }

    /// A pedestal's.
    fn pedestal(c: &Console) -> ConsoleDef {
        ConsoleDef {
            kind: c.kind,
            pos: c.pos,
            bounds: c.bounds(),
            floor: c.pos.y,
            faces: [c.frame(0), c.frame(1)],
            nfaces: 2,
        }
    }
}

/// The puzzles of a match on `level` with `rules`, in the order the sim has them.
pub fn console_defs(level: &Level, _rules: &crate::rules::Rules) -> Vec<ConsoleDef> {
    level.consoles.iter().map(ConsoleDef::pedestal).collect()
}

/// The light bridge console `def` raises (the bridge nearest it).
pub fn bridge_for(level: &Level, def: &ConsoleDef) -> Option<Bridge> {
    if def.kind != ConsoleKind::Bridge {
        return None;
    }
    level
        .bridges
        .iter()
        .min_by(|a, b| {
            a.mid()
                .distance(def.pos)
                .total_cmp(&b.mid().distance(def.pos))
        })
        .copied()
}

/// The light bridges standing (`up`: whether console k has its bridge up), as colliders.
pub fn bridge_colliders(
    level: &Level,
    defs: &[ConsoleDef],
    up: impl Fn(usize) -> bool,
) -> Vec<crate::collide::Collider> {
    defs.iter()
        .enumerate()
        .filter(|(k, _)| up(*k))
        .filter_map(|(_, d)| bridge_for(level, d))
        .map(|b| crate::collide::Collider::Box {
            min: b.min,
            max: b.max,
        })
        .collect()
}

/// A click on a console's puzzle in `PlayerCmd::poke` (and `PuzzleAim`): its index plus this
/// (below it: a turret box's).
pub const CONSOLE_POKE: u8 = 128;

/// Consoles per half: two in a small arena, three in a medium one, five in a large one (one
/// of each kind there is, as far as they go).
pub fn consoles_per_half(size: ArenaSize) -> usize {
    match size {
        ArenaSize::Small => 2,
        ArenaSize::Medium => 3,
        ArenaSize::Large => 5,
    }
}

/// Turns `n` of the blocks in each half into consoles (mirrored pairs, each kind once), as
/// the turret boxes (`level::place_boxes`): open floor in front of their +z and -z sides,
/// spread out, away from the boxes too. A light bridge's console stands near its bridge (a
/// gap of `bridge_spans`, if the arena has one): the bridges it gets (`Level::bridges`).
pub(crate) fn place_consoles(
    g: &mut Grid,
    movers: &[Mover],
    n: usize,
    mode: GameMode,
    seed: u64,
) -> Vec<Bridge> {
    let dm = g.dims;
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ 0xC025_01E5_0000_0002);
    let spans = bridge_spans(g);
    let mut kinds: Vec<ConsoleKind> = ConsoleKind::PEDESTALS
        .into_iter()
        .filter(|k| k.in_mode(mode) && (*k != ConsoleKind::Bridge || !spans.is_empty()))
        .collect();
    // (Shuffled.)
    for i in (1..kinds.len()).rev() {
        kinds.swap(i, rng.random_range(0..=i));
    }
    kinds.truncate(n);
    // (The light bridge's first: it has the fewest places to go.)
    kinds.sort_by_key(|k| *k != ConsoleKind::Bridge);
    let span = (!spans.is_empty()).then(|| spans[rng.random_range(0..spans.len())]);
    let mut bridges = Vec::new();
    let open = |g: &Grid, ix: usize, iz: usize| {
        g.ground[dm.idx(ix, iz)] == Ground::Open && !crate::level::on_track(movers, dm, ix, iz)
    };
    let mut cells: Vec<(usize, usize)> = (dm.half() + 1..dm.base_row() - 1)
        .flat_map(|iz| (0..dm.w).map(move |ix| (ix, iz)))
        .filter(|&(ix, iz)| {
            g.ground[dm.idx(ix, iz)] == Ground::Block
                && open(g, ix, iz + 1)
                && open(g, ix, iz - 1)
                && g.wall(0, ix, iz, 0) == Wall::None
                && g.wall(0, ix, iz, 2) == Wall::None
        })
        .collect();
    // Apart from the turret boxes and the others (both halves), and from its own mirror.
    let mut placed: Vec<glam::Vec3> = (0..dm.h)
        .flat_map(|iz| (0..dm.w).map(move |ix| (ix, iz)))
        .filter(|&(ix, iz)| g.ground[dm.idx(ix, iz)] == Ground::Turret)
        .map(|(ix, iz)| dm.cell_centre(ix, iz))
        .collect();
    for kind in kinds {
        let apart = |&(ix, iz): &(usize, usize)| {
            let c = dm.cell_centre(ix, iz);
            placed
                .iter()
                .flat_map(|&p| [p, mirror_point(p)])
                .chain([mirror_point(c)])
                .map(|p| p.distance(c))
                .fold(f32::MAX, f32::min)
        };
        let near = |&(ix, iz): &(usize, usize)| match (kind, span) {
            (ConsoleKind::Bridge, Some(s)) => {
                let d = dm.cell_centre(ix, iz) - s.mid();
                glam::Vec2::new(d.x, d.z).length() <= BRIDGE_NEAR
            }
            _ => true,
        };
        let far = cells
            .iter()
            .filter(|c| near(c))
            .map(apart)
            .fold(0.0, f32::max);
        let good: Vec<(usize, usize)> = cells
            .iter()
            .copied()
            .filter(|c| near(c) && apart(c) >= (far * 0.8).max(4.0))
            .collect();
        if good.is_empty() {
            continue;
        }
        let (ix, iz) = good[rng.random_range(0..good.len())];
        g.set_ground(ix, iz, Ground::Console { kind });
        placed.push(dm.cell_centre(ix, iz));
        cells.retain(|&c| c != (ix, iz));
        if let (ConsoleKind::Bridge, Some(s)) = (kind, span) {
            // The railings at its ends open (and their mirror images).
            for (end, other) in [(s.ends[0], s.ends[1]), (s.ends[1], s.ends[0])] {
                let (Some((x, z)), d) = (dm.cell_at(end), other - end) else {
                    continue;
                };
                let dir = if d.x.abs() > d.z.abs() {
                    if dm
                        .step(x, z, 1, 1)
                        .is_some_and(|n| (dm.cell_centre(n.0, n.1) - end).x * d.x > 0.0)
                    {
                        1
                    } else {
                        3
                    }
                } else if dm
                    .step(x, z, 0, 1)
                    .is_some_and(|n| (dm.cell_centre(n.0, n.1) - end).z * d.z > 0.0)
                {
                    0
                } else {
                    2
                };
                if g.wall(1, x, z, dir) == Wall::Rail {
                    g.set_wall(1, crate::level::edge(x, z, dir), Wall::None);
                }
            }
            bridges = vec![s, s.mirrored()];
        }
    }
    bridges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Ground;

    #[test]
    fn arenas_get_mirrored_consoles_open_in_front() {
        let (mut full, mut arenas) = (0, 0);
        for seed in 0..24u64 {
            for (size, mode) in [
                (ArenaSize::Small, GameMode::Classic),
                (ArenaSize::Medium, GameMode::Megatarget),
                (ArenaSize::Large, GameMode::Prism),
                (ArenaSize::Medium, GameMode::Infection),
            ] {
                let level = Level::generate_for(seed, size, mode);
                let dm = level.dims();
                // (A light bridge only where there's a gap for one.)
                let spans = !bridge_spans(&level.grid).is_empty();
                let want = consoles_per_half(size).min(
                    ConsoleKind::PEDESTALS
                        .iter()
                        .filter(|k| k.in_mode(mode) && (**k != ConsoleKind::Bridge || spans))
                        .count(),
                );
                // (Fewer where the arena has no room for them all, far enough apart.)
                let n = level.consoles.len();
                assert!(
                    n >= 2 && n <= 2 * want && n % 2 == 0,
                    "seed {seed} {size:?}: {n}"
                );
                full += (n == 2 * want) as usize;
                arenas += 1;
                for c in &level.consoles {
                    assert!(c.kind.in_mode(mode));
                    assert!(
                        level
                            .consoles
                            .iter()
                            .any(|o| o.kind == c.kind && o.pos.distance(mirror_point(c.pos)) < 0.01),
                        "seed {seed}: mirrored"
                    );
                    for b in &level.boxes {
                        assert!(b.pos.distance(c.pos) > 3.0, "seed {seed}: by a box");
                    }
                    for dz in [-1.3, 1.3] {
                        let (ix, iz) = dm.cell_at(c.pos + Vec3::Z * dz).unwrap();
                        assert_eq!(level.grid.ground[dm.idx(ix, iz)], Ground::Open);
                    }
                }
            }
        }
        assert!(full * 10 >= arenas * 8, "all of them in {full} of {arenas}");
        let bridged = (0..24u64)
            .filter(|&seed| {
                let level = Level::generate_for(seed, ArenaSize::Large, GameMode::Classic);
                let n = level
                    .consoles
                    .iter()
                    .filter(|c| c.kind == ConsoleKind::Bridge)
                    .count();
                assert!(n == 0 && level.bridges.is_empty() || n == 2 && level.bridges.len() == 2);
                for b in &level.bridges {
                    assert!(level.consoles.iter().any(|c| c.kind == ConsoleKind::Bridge
                        && (c.pos - b.mid()).truncate().length() < 12.0));
                }
                n > 0
            })
            .count();
        println!("light bridges in {bridged} of 24 large arenas");
        assert!(
            bridged >= 8,
            "light bridges in {bridged} of 24 large arenas"
        );
    }
}

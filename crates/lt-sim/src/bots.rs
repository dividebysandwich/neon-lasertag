//! Bots: anything that turns the game state into a `PlayerCmd` each tick, like a local player
//! or a remote one. `Bot` plays the game with a little tactics; other controllers implement
//! `Controller` without touching the rules.
//!
//! # How a bot thinks
//!
//! **Map knowledge.** When the bots first see an arena (keyed on `Level::seed`) they build one
//! `BotGraph` and share it (`Arc`): the level's nav nodes (cell centres on the ground, ramps and
//! the upper floor) with extra diagonal links where both orthogonal neighbours are linked and
//! the diagonal is walkable, edge costs in metres, A* paths, and string-pulling (a waypoint is
//! skipped when the straight line to a later one is clear for the body at knee and chest height
//! and has floor under it all the way). Per node and per team it also precomputes tactics:
//! cover from the enemy base (fraction of chest-height lines to the enemy spawn pads that are
//! blocked), which enemy targets can be hit from it (from behind, through the alcove opening),
//! which of the team's own targets' approaches it overlooks, and walking distances to both
//! bases.
//!
//! **Roles and skill.** Every bot is offensive or defensive (`Bots` alternates them per team).
//! Its skill (0..1, from the game's `Difficulty`) sets how well it plays, with some spread
//! between bots: its fixed aiming error (an angle applied to every shot, about 6.5° standard
//! deviation at Easy, 3.5° at Normal and 2° at Hard, so even Hard bots miss at range), reaction
//! time, turn speed, how much it leads moving enemies, how readily it throws grenades and how
//! much it cares about cover when picking spots.
//!
//! **Plans.** Every 10–20 s a bot picks a plan: hold a covered spot near where it is (peeking
//! out now and then), engage the nearest enemy it knows about (closing in to fighting range,
//! then strafing), or its role's job: an attacker goes to a covered spot from which it can hit
//! an enemy target and shoots the target again and again; a defender takes a covered spot
//! overlooking its own targets' approaches. Spots are scored (cover, line of sight, distance,
//! a bit of randomness) and claimed, so team mates spread out.
//!
//! **Overrides.** Low on shots → refill at the nearest reload station or own spawn pad, then
//! carry on. A pickup in sight that it would take (a grenade while carrying none, a powerup it
//! hasn't got) → fetch it. Visible enemies in range
//! are always shot at (strafing while doing so); a hit makes the bot look for the shooter.
//! A carried shield goes on as soon as it has an enemy to shoot at; shielded enemies aren't
//! shot at, and with a shotgun nothing out of its reach. Cloaked enemies are only seen up close
//! (`sim::CLOAK_NEAR`), though still heard. Carrying a grenade, it lobs it at two or more enemies close together that it knows about
//! (seen recently, heard nearby or heard shooting), picking a low or high arc whose simulated
//! flight first lands next to the group, and never near team mates or itself.
//!
//! **Team play.** A team's bots share a blackboard (`TeamBoard`). Enemies one of them sees are
//! known to all of them for a few seconds. Attackers don't trickle into the enemy half one by
//! one: on their way over they meet at a staging point (a covered spot on their own side of the
//! centre line) and push together once two or more have gathered and the rest is not far
//! behind, or after a timeout, or alone when nobody else is attacking; latecomers shortly after
//! a push follow it straight away. When one of the team's targets is hit, or an enemy is seen
//! close to its opening, the defenders drop what they are doing and take covered spots
//! overlooking that target's rear approach. Claimed spots keep them from all picking the same
//! one.
//!
//! **Moving pieces.** Door links are in the graph separately: A* (`BotGraph::path_at`) takes
//! one if the door will be open when the bot gets there, or opens soon (costing the wait), and
//! never one that stays shut. A bot whose route goes through a door that turns out shut waits
//! in front of it if it opens soon and plans again otherwise. Sliding panels' tracks are
//! walkable; a bot a panel gets in the way of is unstuck like any other.
//!
//! **Robustness.** Routes are rebuilt when the bot stops making progress (with a random nudge),
//! and after a few failures the goal is given up and another chosen; every goal has a fallback
//! (no spot of a kind → hold or engage; nothing known → patrol). Dying clears all state.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::f32::consts::{PI, TAU};
use std::sync::{Arc, Mutex, MutexGuard};

use glam::{Quat, Vec2, Vec3};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::collide::{BODY_RADIUS, GRAVITY, World};
use crate::level::{self, Dims, Level, MoverKind};
use crate::sim::{EYE, PICKUP_HEIGHT, Player, Sim, TICK, eye_on_neck, look_rot, yaw_of};
use crate::types::*;

pub trait Controller: Send + Sync {
    fn command(&mut self, sim: &Sim, me: PlayerId) -> PlayerCmd;
}

/// How well the bots play (menus and `lt-server --difficulty`). Callers hand the bots
/// `difficulty.skill()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    /// The bots' skill, 0..1.
    pub fn skill(self) -> f32 {
        match self {
            Difficulty::Easy => 0.15,
            Difficulty::Normal => 0.55,
            Difficulty::Hard => 0.9,
        }
    }

    /// "easy", "normal" or "hard" (any case; also "e", "n", "h", "medium").
    pub fn parse(s: &str) -> Option<Difficulty> {
        match s.trim().to_ascii_lowercase().as_str() {
            "easy" | "e" => Some(Difficulty::Easy),
            "normal" | "n" | "medium" => Some(Difficulty::Normal),
            "hard" | "h" => Some(Difficulty::Hard),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        }
    }
}

/// Heights above the feet for walkability tests: just over a step, and the chest.
const KNEE: f32 = 0.45;
const CHEST: f32 = 1.2;
/// Height of the lines used for cover tests.
const COVER_H: f32 = 1.3;
/// Farthest a bot shoots at players.
const SIGHT: f32 = 36.0;
/// Field of view (cosine of the half angle) for noticing enemies; closer than `SENSE` always.
const FOV_COS: f32 = 0.26;
const SENSE: f32 = 6.0;
/// Enemies are heard (position known, not shootable) this close, or this close when firing.
const HEAR: f32 = 10.0;
const SHOT_HEAR: f32 = 25.0;
/// How long a seen or heard enemy is remembered (seconds).
const MEMORY: f64 = 5.0;
/// Below this many shots the bot goes to refill.
const LOW_AMMO: u16 = 8;
/// Pickups in sight this close are fetched.
const PICKUP_SIGHT: f32 = 18.0;
/// Grenades: enemies this close together are a group, thrown at from this range, landing this
/// close to the group; never with team mates or the thrower this close to the landing point.
const GROUP_RADIUS: f32 = 3.5;
const THROW_MIN: f32 = 6.0;
const THROW_MAX: f32 = 20.0;
const THROW_LAND: f32 = 2.5;
const MATE_CLEAR: f32 = 5.0;
const SELF_CLEAR: f32 = 6.0;
/// Engaging: stop closing in on a visible enemy this close.
const KEEP_DIST: f32 = 11.0;
/// Enemy targets are shot at from at most this far.
const TARGET_RANGE: f32 = 22.0;
/// Team play: shared sightings are trusted this long (seconds; one's own memory lasts longer).
const SHARED_MEMORY: f64 = 3.0;
/// An enemy this close to the opening of one of the team's targets raises the alarm.
const ALARM_NEAR: f32 = 8.0;
/// Defenders answer an alarm at most this old, from at most this far away.
const ALARM_FRESH: f64 = 6.0;
const ALARM_REACH: f32 = 45.0;
/// Attackers at the staging point push once the others gathered or this long after the first
/// arrived with company (`GROUP_SETTLE`); anyone waiting goes after `GROUP_WAIT` regardless.
/// Arrivals up to `PUSH_JOIN` after a push follow it; a bot that can't get to the staging point
/// for `GATE_GIVE_UP` goes on alone.
const GROUP_SETTLE: f64 = 4.0;
const GROUP_WAIT: f64 = 10.0;
const PUSH_JOIN: f64 = 4.0;
const GATE_GIVE_UP: f64 = 25.0;
/// Within this of the staging point counts as there.
const STAGING_REACH: f32 = 2.5;

// ---------------------------------------------------------------------------------------------
// The bots' map of an arena.

/// Per node and per team: what a bot standing there can do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tactics {
    /// Fraction (0..1) of chest-height lines from here to the enemy base that are blocked.
    pub cover: f32,
    /// Bit `id`: the enemy target `id` can be hit from here.
    pub shoots: u16,
    /// Bit `id`: the approach of the team's own target `id` is in sight from here.
    pub watches: u16,
    /// Walking distance to the own base and to the enemy base (infinite if unreachable).
    pub home: f32,
    pub enemy_base: f32,
}

/// The navigation graph and tactical map of one arena, shared by all bots.
pub struct BotGraph {
    pub seed: u64,
    pub dims: Dims,
    /// Node positions (floor height) and whether a node is walkable; indices as `level::Nav`.
    pub pos: Vec<Vec3>,
    pub valid: Vec<bool>,
    /// Links with their lengths (the level's links plus walkable diagonals).
    pub links: Vec<Vec<(u32, f32)>>,
    /// Links through doors, usable while the door is open: node, length, door (mover index).
    pub door_links: Vec<Vec<(u32, f32, usize)>>,
    /// Per team (`Team::index`), per node.
    pub tactics: [Vec<Tactics>; 2],
    /// Connected component of each valid node (`u32::MAX` for invalid ones), with every door
    /// closed and with every door open.
    comp: Vec<u32>,
    comp_open: Vec<u32>,
    /// Megatarget mode: per secondary target, the nodes it can be hit from.
    pub mega_spots: [Vec<usize>; 2],
    /// Per node, the team whose spawn pad it is on (the other team's routes keep off: it tags
    /// them out).
    pub spawn: Vec<Option<Team>>,
}

/// Walking speed assumed for door timing (metres a second).
const DOOR_ETA_SPEED: f32 = 3.5;
/// Longest wait for a door to open that a route accepts (ticks), and the least time it has to
/// stay open to be worth it.
const DOOR_WAIT_MAX: u32 = 240;
const DOOR_OPEN_MIN: u32 = 40;
/// Ticks a bot needs to get through a laser gate (it waits if the gate is on or switches on
/// sooner).
const GATE_CLEAR_TICKS: u32 = 24;
/// How far bots see in a blackout (m).
const BLACKOUT_SIGHT: f32 = 12.0;
/// With the charge gun, enemies at least this far off (m) get a beam, closer ones bolts.
const BEAM_MIN_RANGE: f32 = 8.0;
/// What going through a laser gate costs a route (metres), for the wait.
const GATE_DETOUR: f32 = 3.0;

/// A* open-list entry, ordered so that `BinaryHeap` pops the lowest estimate first.
#[derive(Clone, Copy, PartialEq)]
struct Open(f32, u32);

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `World::clear` for long lines, quicker: `World::ray` looks at every collider in the line's
/// bounding box, so a line across the arena is tested in short pieces (stopping at the first
/// blocked one) that each only see the colliders nearby.
pub fn clear(world: &World, a: Vec3, b: Vec3) -> bool {
    const PIECE: f32 = 3.0;
    let len = a.distance(b);
    let n = (len / PIECE).ceil().max(1.0) as usize;
    let mut from = a;
    for k in 1..=n {
        let to = a.lerp(b, k as f32 / n as f32);
        if world.ray(from, to).is_some() {
            return false;
        }
        from = to;
    }
    true
}

/// Can a player walk straight from `a` to `b` (both feet positions on the same floor level)?
/// Clear at knee height for the body's width and at chest height, with floor all the way.
pub fn walkable(world: &World, a: Vec3, b: Vec3) -> bool {
    if (a.y - b.y).abs() > 0.25 {
        return false;
    }
    let d = Vec2::new(b.x - a.x, b.z - a.z);
    let len = d.length();
    if len < 1e-3 {
        return true;
    }
    let side = Vec3::new(-d.y, 0.0, d.x) / len * (BODY_RADIUS + 0.04);
    for off in [Vec3::ZERO, side, -side] {
        if !world.clear(a + off + Vec3::Y * KNEE, b + off + Vec3::Y * KNEE) {
            return false;
        }
    }
    if !world.clear(a + Vec3::Y * CHEST, b + Vec3::Y * CHEST) {
        return false;
    }
    let steps = (len / 0.6).ceil() as usize;
    let top = a.y.max(b.y) + 0.3;
    for s in 1..steps {
        let p = a.lerp(b, s as f32 / steps as f32);
        match world.ground(p.x, p.z, top) {
            Some(y) if (y - p.y).abs() < 0.35 => {}
            _ => return false,
        }
    }
    true
}

/// Does a bolt from `eye` towards the middle of `t` hit the panel (and nothing before it)?
fn hits_panel(world: &World, eye: Vec3, t: &level::TargetDef) -> bool {
    let c = t.centre();
    if (eye - c).dot(t.normal) < 0.3 {
        return false;
    }
    let dir = (c - eye).normalize_or_zero();
    match world.ray(eye, c + dir * 0.1) {
        Some((_, q, _)) => q.cmpge(t.min - 0.03).all() && q.cmple(t.max + 0.03).all(),
        None => false,
    }
}

/// Does a bolt from `eye` towards the middle of the box hit it (and nothing before it)?
fn hits_box(world: &World, eye: Vec3, min: Vec3, max: Vec3) -> bool {
    level::box_seen(world, eye, min, max, (min + max) / 2.0)
}

impl BotGraph {
    /// Builds the graph and the tactical map for `level` (a few milliseconds).
    pub fn build(level: &Level, world: &World) -> BotGraph {
        let nav = &level.nav;
        let n = nav.pos.len();
        let pos = nav.pos.clone();
        let valid = nav.valid.clone();
        let mut links: Vec<Vec<(u32, f32)>> = (0..n)
            .map(|i| {
                nav.links[i]
                    .iter()
                    .map(|&m| (m, pos[i].distance(pos[m as usize])))
                    .collect()
            })
            .collect();

        // Diagonals: a 2x2 block of linked cells on one flat floor, walkable corner to corner.
        let linked = |a: usize, b: usize| nav.links[a].contains(&(b as u32));
        let dm = level.dims();
        let wh = dm.cells();
        for floor in 0..2 {
            let node = |ix: usize, iz: usize| floor * wh + dm.idx(ix, iz);
            for iz in 0..dm.h - 1 {
                for ix in 0..dm.w {
                    for jx in [ix.wrapping_sub(1), ix + 1] {
                        if jx >= dm.w {
                            continue;
                        }
                        let (a, b, o1, o2) = (
                            node(ix, iz),
                            node(jx, iz + 1),
                            node(jx, iz),
                            node(ix, iz + 1),
                        );
                        if ![a, b, o1, o2].iter().all(|&m| valid[m]) {
                            continue;
                        }
                        if !(linked(a, o1) && linked(a, o2) && linked(o1, b) && linked(o2, b)) {
                            continue;
                        }
                        let y = pos[a].y;
                        if [b, o1, o2].iter().any(|&m| (pos[m].y - y).abs() > 0.05) {
                            continue;
                        }
                        if walkable(world, pos[a], pos[b]) {
                            let d = pos[a].distance(pos[b]);
                            links[a].push((b as u32, d));
                            links[b].push((a as u32, d));
                        }
                    }
                }
            }
        }

        // Laser gates: the average wait for one to go off, as a detour.
        for gt in &level.gates {
            let (a, b) = (gt.link.0 as usize, gt.link.1 as usize);
            for (x, y) in [(a, b), (b, a)] {
                if let Some(l) = links[x].iter_mut().find(|l| l.0 as usize == y) {
                    l.1 += GATE_DETOUR;
                }
            }
        }

        // Jump pads: one way, up (the flight is about as quick as walking its length).
        for &(a, b, _) in &nav.jumps {
            let d = pos[a as usize]
                .with_y(0.0)
                .distance(pos[b as usize].with_y(0.0))
                + 1.0;
            links[a as usize].push((b, d));
        }

        let mut door_links: Vec<Vec<(u32, f32, usize)>> = vec![Vec::new(); n];
        for &(a, b, m) in &nav.doors {
            let d = pos[a as usize].distance(pos[b as usize]);
            door_links[a as usize].push((b, d, m));
            door_links[b as usize].push((a, d, m));
        }

        // Connected components (for quick "is it reachable" checks).
        let components = |doors: bool| {
            let mut comp = vec![u32::MAX; n];
            let mut next = 0;
            for s in 0..n {
                if !valid[s] || comp[s] != u32::MAX {
                    continue;
                }
                comp[s] = next;
                let mut queue = VecDeque::from([s]);
                while let Some(i) = queue.pop_front() {
                    let through_doors = door_links[i].iter().filter(|_| doors).map(|l| l.0);
                    for m in links[i].iter().map(|l| l.0).chain(through_doors) {
                        if comp[m as usize] == u32::MAX {
                            comp[m as usize] = next;
                            queue.push_back(m as usize);
                        }
                    }
                }
                next += 1;
            }
            comp
        };
        let (comp, comp_open) = (components(false), components(true));

        let mut g = BotGraph {
            dims: level.dims(),
            seed: level.seed,
            pos,
            valid,
            links,
            door_links,
            tactics: [Vec::new(), Vec::new()],
            comp,
            comp_open,
            mega_spots: [Vec::new(), Vec::new()],
            spawn: (0..n).map(|i| level.spawn_at(nav.pos[i])).collect(),
        };
        g.tactics = Team::ALL.map(|team| g.tactics_for(level, world, team));
        if let Some(m) = &level.mega {
            for (i, t) in m.secondaries.iter().enumerate() {
                g.mega_spots[i] = (0..g.pos.len())
                    .filter(|&n| g.valid[n] && g.pos[n].distance(t.centre()) < 10.0)
                    .filter(|&n| {
                        let eye = g.pos[n] + Vec3::Y * EYE;
                        hits_box(world, eye, t.min, t.max)
                            && hits_box(world, eye - Vec3::Y * 0.3, t.min, t.max)
                    })
                    .collect();
            }
        }
        g
    }

    fn tactics_for(&self, level: &Level, world: &World, team: Team) -> Vec<Tactics> {
        let enemy = team.other();
        // Threats: every other enemy spawn pad, and the space in front of the enemy base.
        let mut threats: Vec<Vec3> = level.spawns[enemy.index()]
            .iter()
            .step_by(2)
            .map(|s| s.pos + Vec3::Y * COVER_H)
            .collect();
        // In front of the enemy base: towards the middle of the arena.
        let base = level.base_centre(enemy);
        threats.push(
            base - Vec3::new(base.x, 0.0, base.z).normalize_or_zero() * 6.0 + Vec3::Y * COVER_H,
        );
        let spawn_nodes = |t: Team| -> Vec<usize> {
            level.spawns[t.index()]
                .iter()
                .filter_map(|s| level.nav.node_at(s.pos))
                .collect()
        };
        let home = self.costs_from(&spawn_nodes(team));
        let away = self.costs_from(&spawn_nodes(enemy));
        // Where an attacker on one of our targets would stand: the cell in front of the alcove's
        // opening, and two cells further out.
        let approaches: Vec<(u8, [Vec3; 2])> = level
            .targets
            .iter()
            .filter(|t| t.team == team)
            .map(|t| {
                let rc = level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1) + Vec3::Y * CHEST;
                (t.id, [rc, rc + t.normal * 4.0])
            })
            .collect();
        let enemy_targets: Vec<&level::TargetDef> =
            level.targets.iter().filter(|t| t.team == enemy).collect();

        (0..self.pos.len())
            .map(|n| {
                if !self.valid[n] {
                    return Tactics {
                        home: f32::INFINITY,
                        enemy_base: f32::INFINITY,
                        ..Default::default()
                    };
                }
                let p = self.pos[n];
                let chest = p + Vec3::Y * COVER_H;
                let blocked = threats.iter().filter(|&&t| !clear(world, chest, t)).count();
                let eye = p + Vec3::Y * EYE;
                let mut shoots = 0u16;
                for t in &enemy_targets {
                    if eye.distance(t.centre()) < TARGET_RANGE
                        && hits_panel(world, eye, t)
                        && hits_panel(world, eye - Vec3::Y * 0.3, t)
                    {
                        shoots |= 1 << t.id;
                    }
                }
                let mut watches = 0u16;
                for (id, pts) in &approaches {
                    if pts
                        .iter()
                        .any(|&q| (2.0..20.0).contains(&eye.distance(q)) && clear(world, eye, q))
                    {
                        watches |= 1 << id;
                    }
                }
                Tactics {
                    cover: blocked as f32 / threats.len().max(1) as f32,
                    shoots,
                    watches,
                    home: home[n],
                    enemy_base: away[n],
                }
            })
            .collect()
    }

    /// Walking distances from the nearest of `sources` (Dijkstra).
    pub fn costs_from(&self, sources: &[usize]) -> Vec<f32> {
        let mut cost = vec![f32::INFINITY; self.pos.len()];
        let mut heap = BinaryHeap::new();
        for &s in sources {
            if self.valid[s] {
                cost[s] = 0.0;
                heap.push(Open(0.0, s as u32));
            }
        }
        while let Some(Open(c, i)) = heap.pop() {
            let i = i as usize;
            if c > cost[i] {
                continue;
            }
            for &(m, l) in &self.links[i] {
                let nc = c + l;
                if nc < cost[m as usize] {
                    cost[m as usize] = nc;
                    heap.push(Open(nc, m));
                }
            }
        }
        cost
    }

    /// Whether `b` can be walked to from `a`.
    pub fn connected(&self, a: usize, b: usize) -> bool {
        self.valid[a] && self.valid[b] && self.comp[a] == self.comp[b]
    }

    /// Shortest path from `a` to `b` (both included) with every door closed, A* with
    /// straight-line distance.
    pub fn path(&self, a: usize, b: usize) -> Option<Vec<usize>> {
        if !self.connected(a, b) {
            return None;
        }
        self.astar(a, b, None, None).map(|(p, _)| p)
    }

    /// Shortest path from `a` to `b` setting out at `tick`: doors are taken if they will be
    /// open on arrival (long enough to get through), or open soon, costing the wait. Not over
    /// the spawn pads of `avoid` (the walker's enemy). With the doors on the way (mover
    /// indices).
    pub fn path_at(
        &self,
        a: usize,
        b: usize,
        level: &Level,
        tick: u64,
        avoid: Option<Team>,
    ) -> Option<(Vec<usize>, Vec<usize>)> {
        if !(self.valid[a] && self.valid[b] && self.comp_open[a] == self.comp_open[b]) {
            return None;
        }
        self.astar(a, b, Some((level, tick)), avoid)
    }

    fn astar(
        &self,
        a: usize,
        b: usize,
        doors: Option<(&Level, u64)>,
        avoid: Option<Team>,
    ) -> Option<(Vec<usize>, Vec<usize>)> {
        let n = self.pos.len();
        let h = |i: usize| self.pos[i].distance(self.pos[b]);
        let mut g = vec![f32::INFINITY; n];
        let mut prev = vec![u32::MAX; n];
        let mut heap = BinaryHeap::new();
        g[a] = 0.0;
        heap.push(Open(h(a), a as u32));
        while let Some(Open(f, i)) = heap.pop() {
            let i = i as usize;
            if i == b {
                let mut out = vec![b];
                let mut c = b;
                while c != a {
                    c = prev[c] as usize;
                    out.push(c);
                }
                out.reverse();
                let used = out
                    .windows(2)
                    .filter_map(|w| {
                        let static_link = self.links[w[0]].iter().any(|l| l.0 as usize == w[1]);
                        let door = self.door_links[w[0]].iter().find(|l| l.0 as usize == w[1]);
                        door.filter(|_| !static_link).map(|l| l.2)
                    })
                    .collect();
                return Some((out, used));
            }
            if f > g[i] + h(i) + 1e-3 {
                continue; // a stale entry
            }
            let gi = g[i];
            let through_doors: Vec<(u32, f32)> = doors
                .into_iter()
                .flat_map(|(level, tick)| {
                    self.door_links[i].iter().filter_map(move |&(m, l, door)| {
                        let mover = level.movers.get(door)?;
                        let eta = (gi / DOOR_ETA_SPEED / TICK) as u64;
                        let (wait, open_for) = mover.passable_window(tick + eta);
                        if wait == 0 && open_for >= DOOR_OPEN_MIN {
                            Some((m, l))
                        } else if wait > 0 && wait <= DOOR_WAIT_MAX && open_for >= DOOR_OPEN_MIN {
                            Some((m, l + 2.0 + wait as f32 * TICK * DOOR_ETA_SPEED))
                        } else {
                            None
                        }
                    })
                })
                .collect();
            for (m, l) in self.links[i].iter().copied().chain(through_doors) {
                let m = m as usize;
                if avoid.is_some() && self.spawn[m] == avoid && m != b {
                    continue;
                }
                let ng = g[i] + l;
                if ng < g[m] {
                    g[m] = ng;
                    prev[m] = i as u32;
                    heap.push(Open(ng + h(m), m as u32));
                }
            }
        }
        None
    }

    /// What a node path costs (its links' lengths, the gates' waits included).
    pub fn path_cost(&self, path: &[usize]) -> f32 {
        path.windows(2)
            .map(|w| {
                self.links[w[0]]
                    .iter()
                    .find(|l| l.0 as usize == w[1])
                    .map_or(f32::INFINITY, |l| l.1)
            })
            .sum()
    }

    /// Length of a node path.
    pub fn path_len(&self, path: &[usize]) -> f32 {
        path.windows(2)
            .map(|w| self.pos[w[0]].distance(self.pos[w[1]]))
            .sum()
    }

    /// Waypoints for walking `path` from `start`: each waypoint is the farthest node (looking a
    /// few ahead) that can be walked to in a straight line from the previous one.
    pub fn smooth(&self, world: &World, start: Vec3, path: &[usize]) -> Vec<Vec3> {
        let pts: Vec<Vec3> = path.iter().map(|&i| self.pos[i]).collect();
        let mut out = Vec::new();
        let mut from = start;
        let mut i = 0;
        while i < pts.len() {
            let mut j = i;
            for (k, &p) in pts
                .iter()
                .enumerate()
                .take((i + 8).min(pts.len()))
                .skip(i + 1)
            {
                if walkable(world, from, p) {
                    j = k;
                } else {
                    break;
                }
            }
            out.push(pts[j]);
            from = pts[j];
            i = j + 1;
        }
        out
    }

    /// The node to start walking from at `feet`: of the valid nodes in this and the
    /// neighbouring cells on about the same level, the nearest one that can be walked to
    /// (falling back to the nearest one at all).
    pub fn nearest(&self, world: &World, feet: Vec3) -> Option<usize> {
        let dm = self.dims;
        let (cx, cz) = dm.cell_at(feet)?;
        let mut cands: Vec<(f32, usize)> = Vec::new();
        for iz in cz.saturating_sub(1)..=(cz + 1).min(dm.h - 1) {
            for ix in cx.saturating_sub(1)..=(cx + 1).min(dm.w - 1) {
                for floor in 0..2 {
                    let n = floor * dm.cells() + dm.idx(ix, iz);
                    let p = self.pos[n];
                    let dy = (p.y - feet.y).abs();
                    if self.valid[n] && dy < 1.6 {
                        cands.push((Vec2::new(p.x - feet.x, p.z - feet.z).length() + dy * 2.0, n));
                    }
                }
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        cands
            .iter()
            .find(|&&(_, n)| {
                let p = self.pos[n];
                (p.y - feet.y).abs() > 0.25
                    || world.clear(feet + Vec3::Y * KNEE, p + Vec3::Y * KNEE)
            })
            .or(cands.first())
            .map(|&(_, n)| n)
    }
}

// ---------------------------------------------------------------------------------------------
// State shared by a game's bots.

#[derive(Default)]
struct SharedState {
    graph: Option<Arc<BotGraph>>,
    /// Spots claimed by bots (so team mates spread out).
    claims: HashMap<PlayerId, (Team, Vec3)>,
    /// Per team (`Team::index`).
    boards: [TeamBoard; 2],
    /// Team play switched off (each bot on its own; for comparisons in tests).
    solo: bool,
}

/// What a team's bots tell each other (see "Team play" in the module documentation).
#[derive(Default)]
struct TeamBoard {
    /// Enemies seen by any bot of the team: id, feet, when.
    sightings: Vec<(PlayerId, Vec3, f64)>,
    /// The attackers' meeting point (a node), picked when first needed.
    staging: Option<usize>,
    /// The team's attackers as they last reported in.
    attackers: HashMap<PlayerId, Attacker>,
    /// When the attackers last pushed.
    push_at: Option<f64>,
    /// One of the team's targets is in trouble.
    alarm: Option<Alarm>,
    /// Per target: the tick of the last hit already turned into an alarm.
    hits_seen: Vec<u64>,
}

#[derive(Clone, Copy, Debug)]
struct Attacker {
    /// When it last reported in (alive and thinking).
    seen: f64,
    /// Held back until the push (heading for the staging point or waiting there).
    gated: bool,
    /// Waiting at the staging point since.
    waiting: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct Alarm {
    target: u8,
    /// Where the threat is (the shooter, the enemy seen, or the target's opening).
    threat: Vec3,
    at: f64,
}

impl TeamBoard {
    /// The staging point for `team`'s attackers: a well covered, reachable ground node on the
    /// own side of the centre line, near the middle of the arena's width.
    fn staging(&mut self, level: &Level, g: &BotGraph, team: Team) -> Option<usize> {
        if self.staging.is_none() {
            let tac = &g.tactics[team.index()];
            let wh = level.dims().cells();
            self.staging = (0..wh)
                .filter(|&n| {
                    let p = g.pos[n];
                    let z = p.z * team.side();
                    g.valid[n]
                        && (3.0..9.0).contains(&z)
                        && tac[n].home.is_finite()
                        && tac[n].enemy_base.is_finite()
                        && !level.reloads_at(p)
                })
                .map(|n| {
                    (
                        2.0 * tac[n].cover - 0.08 * g.pos[n].x.abs() - 0.02 * tac[n].enemy_base,
                        n,
                    )
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|b| b.1);
        }
        self.staging
    }

    /// Whether the attackers waiting at the staging point should go now (`me` is one of them).
    fn push_now(&self, now: f64) -> bool {
        if self.push_at.is_some_and(|t| now - t < PUSH_JOIN) {
            return true;
        }
        let fresh: Vec<&Attacker> = self
            .attackers
            .values()
            .filter(|a| now - a.seen < 0.5)
            .collect();
        let gated = fresh.iter().filter(|a| a.gated).count();
        let waiting: Vec<f64> = fresh
            .iter()
            .filter(|a| a.gated)
            .filter_map(|a| a.waiting)
            .collect();
        let Some(first) = waiting.iter().copied().reduce(f64::min) else {
            return false;
        };
        let ready = waiting.len();
        let waited = now - first;
        // Alone only after a moment: the others may not have reported yet.
        ready >= 2 && (ready >= gated || waited >= GROUP_SETTLE)
            || ready == 1 && gated == 1 && fresh.len() == 1 && waited >= 1.0
            || waited >= GROUP_WAIT
    }
}

type Shared = Arc<Mutex<SharedState>>;

fn lock(s: &Shared) -> MutexGuard<'_, SharedState> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------------------------------------
// The bot.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Goes for the enemy's targets.
    Offense,
    /// Guards the own targets.
    Defense,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// Hold a covered spot nearby.
    Hold,
    /// Go after the nearest known enemy.
    Engage,
    /// Shoot an enemy target from a covered spot.
    Attack,
    /// Watch the own targets from a covered spot.
    Defend,
    /// Megatarget mode: hit both secondary targets, then get into the room and hit the
    /// megatarget.
    Mega,
}

/// Something to do before going on with the plan.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Errand {
    Reload { node: usize, pos: Vec3 },
    Pickup { id: u32, node: usize, pos: Vec3 },
}

/// An enemy seen or heard.
#[derive(Clone, Copy, Debug)]
struct Seen {
    id: PlayerId,
    feet: Vec3,
    vel: Vec3,
    at: f64,
}

/// What steering towards a goal came to.
enum Steer {
    Move(Vec2),
    Arrived,
    Unreachable,
}

/// A bot that plays with roles, cover, target attacks, reloads and grenades (see the module
/// documentation).
pub struct Bot {
    rng: ChaCha8Rng,
    role: Role,
    skill: f32,
    /// Standard deviation of each shot's angular error (radians).
    aim_error: f32,
    /// Seconds an enemy has to be in sight before the first shot.
    reaction: f32,
    /// Fastest turn (radians a second).
    turn_rate: f32,
    shared: Shared,
    graph: Option<Arc<BotGraph>>,
    me: Option<PlayerId>,

    plan: Plan,
    plan_left: f32,
    /// The next decision's plan, if something forced it (badly hurt: take cover).
    next_plan_hint: Option<Plan>,
    spot: Option<usize>,
    /// Engaging: the node chased and the enemy position it was picked for; patrolling: a node.
    chase: Option<(usize, Vec3)>,
    patrol: Option<usize>,
    errand: Option<Errand>,
    errand_time: f32,
    errand_retry: f32,

    route: Vec<Vec3>,
    route_goal: Option<usize>,
    /// Doors (mover indices) the route goes through.
    route_doors: Vec<usize>,
    /// Laser gates (indices into `Level::gates`) the route goes through.
    route_gates: Vec<usize>,
    progress_pos: Vec3,
    progress_t: f32,
    stuck: u32,
    shortcut_t: f32,
    nudge: Option<(Vec2, f32)>,

    memory: Vec<Seen>,
    /// Enemies in sight this tick: id, distance, aim point.
    visible: Vec<(PlayerId, f32, Vec3)>,
    enemy: Option<PlayerId>,
    seen_for: f32,
    /// Look here for a while (hit from there).
    alert: Option<(Vec3, f32)>,
    prev_hits: u8,

    aim: Vec3,
    /// This shot's error: angle and direction around the aim line.
    shot_err: (f32, f32),
    fire_prev: bool,
    strafe: f32,
    peek: Option<(Vec2, f32)>,
    peek_t: f32,
    grenade_t: f32,
    scan_phase: f32,

    /// Attackers: going into the enemy half (the team pushed, or it's already there).
    pushing: bool,
    /// Attackers: held back for the push since.
    gate_since: Option<f64>,
    /// Defenders: the last alarm answered.
    alarm_seen: f64,
    /// Megatarget plan: the secondary target going for and the node to shoot it from.
    mega_spot: Option<(usize, usize)>,
}

/// The original name of the bot (the client's autopilot uses it).
pub type WanderBot = Bot;

impl Bot {
    /// A bot of its own (not sharing a map with others); its role follows from the seed.
    pub fn new(seed: u64, skill: f32) -> Bot {
        let role = if seed.is_multiple_of(2) {
            Role::Offense
        } else {
            Role::Defense
        };
        Bot::with_role(seed, skill, role, Shared::default())
    }

    fn with_role(seed: u64, skill: f32, role: Role, shared: Shared) -> Bot {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let skill = skill.clamp(0.0, 1.0);
        // Each bot's own accuracy: set by the skill, with a spread of about ±25 % so bots of one
        // difficulty don't all play alike. The floor keeps the best bots human: about 2° at
        // Hard still misses a chest at 20 m now and then, more so on the move.
        let spread = |rng: &mut ChaCha8Rng| 0.75 + 0.5 * rng.random::<f32>();
        let aim_error = ((7.5 - 6.0 * skill) * spread(&mut rng))
            .max(1.6)
            .to_radians();
        // Seconds before the first shot at someone new: about 0.75 s at Easy, 0.25 s at Hard.
        let reaction = (0.8 - 0.6 * skill) * spread(&mut rng);
        let turn_rate = (2.5 + 6.5 * skill) * spread(&mut rng);
        let scan_phase = rng.random_range(0.0..TAU);
        Bot {
            rng,
            role,
            skill,
            aim_error,
            reaction,
            turn_rate,
            shared,
            graph: None,
            me: None,
            plan: Plan::Hold,
            plan_left: 0.0,
            next_plan_hint: None,
            spot: None,
            chase: None,
            patrol: None,
            errand: None,
            errand_time: 0.0,
            errand_retry: 0.0,
            route: Vec::new(),
            route_goal: None,
            route_doors: Vec::new(),
            route_gates: Vec::new(),
            progress_pos: Vec3::ZERO,
            progress_t: 0.0,
            stuck: 0,
            shortcut_t: 0.0,
            nudge: None,
            memory: Vec::new(),
            visible: Vec::new(),
            enemy: None,
            seen_for: 0.0,
            alert: None,
            prev_hits: 0,
            aim: Vec3::NEG_Z,
            shot_err: (0.0, 0.0),
            fire_prev: false,
            strafe: 1.0,
            peek: None,
            peek_t: 3.0,
            grenade_t: 0.0,
            scan_phase,
            pushing: false,
            gate_since: None,
            alarm_seen: -1.0,
            mega_spot: None,
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// The shared map for the sim's arena, built by the first bot to need it.
    fn graph_for(&mut self, sim: &Sim) -> Arc<BotGraph> {
        let g = {
            let mut s = lock(&self.shared);
            match &s.graph {
                Some(g) if g.seed == sim.level.seed && g.pos.len() == sim.level.nav.pos.len() => {
                    g.clone()
                }
                _ => {
                    // The arena with every door shut (the validated one) and no panels: where
                    // the movers are now is only for now.
                    let mut world = sim.level.world();
                    world.set_dynamic(
                        sim.level
                            .movers
                            .iter()
                            .filter(|m| m.kind == MoverKind::Door)
                            .flat_map(|m| m.colliders(0.0))
                            .collect(),
                    );
                    let g = Arc::new(BotGraph::build(&sim.level, &world));
                    s.graph = Some(g.clone());
                    s.claims.clear();
                    s.boards = Default::default();
                    g
                }
            }
        };
        if !self.graph.as_ref().is_some_and(|old| Arc::ptr_eq(old, &g)) {
            // A new arena: forget everything about the old one.
            self.reset();
            self.graph = Some(g.clone());
        }
        g
    }

    /// Forgets the current plan, route and enemies (after dying, in a new arena).
    fn reset(&mut self) {
        self.release_claim();
        if let Some(me) = self.me {
            for b in &mut lock(&self.shared).boards {
                b.attackers.remove(&me);
            }
        }
        self.pushing = false;
        self.gate_since = None;
        self.plan_left = 0.0;
        self.spot = None;
        self.chase = None;
        self.patrol = None;
        self.errand = None;
        self.errand_retry = 0.0;
        self.route.clear();
        self.route_goal = None;
        self.stuck = 0;
        self.nudge = None;
        self.memory.clear();
        self.visible.clear();
        self.enemy = None;
        self.seen_for = 0.0;
        self.alert = None;
        self.prev_hits = 0;
        self.fire_prev = false;
        self.peek = None;
        self.mega_spot = None;
    }

    fn claim(&mut self, team: Team, pos: Vec3) {
        if let Some(me) = self.me {
            lock(&self.shared).claims.insert(me, (team, pos));
        }
    }

    fn release_claim(&mut self) {
        if let Some(me) = self.me {
            lock(&self.shared).claims.remove(&me);
        }
    }

    /// Team play, once a tick: shares what this bot sees, learns what the others saw, raises
    /// alarms for the team's targets and (defenders) answers them.
    fn share(&mut self, sim: &Sim, g: &BotGraph, p: &Player, now: f64) {
        if self.me.is_none() {
            return;
        }
        let respond = {
            let mut s = lock(&self.shared);
            if s.solo {
                return;
            }
            let b = &mut s.boards[p.team.index()];
            for &(id, _, _) in &self.visible {
                let Some(o) = sim.player(id) else { continue };
                match b.sightings.iter_mut().find(|s| s.0 == id) {
                    Some(s) => (s.1, s.2) = (o.feet, now),
                    None => b.sightings.push((id, o.feet, now)),
                }
            }
            // A hit on one of the team's targets: everyone hears it, and roughly where from.
            b.hits_seen.resize(sim.last_target_hit.len(), 0);
            for t in sim.level.targets.iter().filter(|t| t.team == p.team) {
                let Some((tick, by)) = sim.last_target_hit.get(t.id as usize).copied().flatten()
                else {
                    continue;
                };
                if tick <= b.hits_seen[t.id as usize] {
                    continue;
                }
                b.hits_seen[t.id as usize] = tick;
                let opening = sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1);
                let threat = match sim.player(by).filter(|o| o.alive() && o.team != p.team) {
                    Some(o) => {
                        if o.feet.distance(opening) < 25.0 {
                            match b.sightings.iter_mut().find(|s| s.0 == by) {
                                Some(s) => (s.1, s.2) = (o.feet, now),
                                None => b.sightings.push((by, o.feet, now)),
                            }
                        }
                        o.feet
                    }
                    None => opening,
                };
                b.alarm = Some(Alarm {
                    target: t.id,
                    threat,
                    at: now,
                });
            }
            b.sightings.retain(|s| {
                now - s.2 < SHARED_MEMORY && sim.player(s.0).is_some_and(|o| o.alive())
            });
            // An enemy seen close to a target's opening.
            for &(_, feet, at) in &b.sightings {
                if now - at > 0.25 {
                    continue;
                }
                for t in sim.level.targets.iter().filter(|t| t.team == p.team) {
                    let opening = sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1);
                    if feet.distance(opening) < ALARM_NEAR
                        && b.alarm.is_none_or(|a| now - a.at > 2.0)
                    {
                        b.alarm = Some(Alarm {
                            target: t.id,
                            threat: feet,
                            at: now,
                        });
                    }
                }
            }
            let shared: Vec<(PlayerId, Vec3, f64)> = b.sightings.clone();
            let respond = match (self.role, b.alarm) {
                (Role::Defense, Some(a))
                    if a.at > self.alarm_seen
                        && now - a.at < ALARM_FRESH
                        && !matches!(self.errand, Some(Errand::Reload { .. })) =>
                {
                    self.alarm_seen = a.at;
                    Some(a)
                }
                _ => None,
            };
            drop(s);
            // What the team saw, unless this bot knows better.
            for (id, feet, at) in shared {
                match self.memory.iter_mut().find(|m| m.id == id) {
                    Some(m) if m.at >= at => {}
                    Some(m) => {
                        m.feet = feet;
                        m.at = at;
                    }
                    None => self.memory.push(Seen {
                        id,
                        feet,
                        vel: Vec3::ZERO,
                        at,
                    }),
                }
            }
            respond
        };
        if let Some(a) = respond
            && let Some(t) = sim.level.targets.iter().find(|t| t.id == a.target)
            && t.centre().distance(p.feet) < ALARM_REACH
            && let Some(spot) = self.pick_guard_spot(sim, g, p, t)
        {
            self.release_claim();
            self.plan = Plan::Defend;
            self.plan_left = self.rng.random_range(10.0..16.0);
            self.spot = Some(spot);
            self.chase = None;
            self.patrol = None;
            self.route_goal = None;
            self.peek = None;
            self.claim(p.team, g.pos[spot]);
            // Look that way on the way there.
            self.alert = Some((a.threat + Vec3::Y * CHEST, 1.5));
        }
    }

    /// Answering an alarm: a covered spot overlooking target `t`'s rear approach, a few metres
    /// from its opening, away from team mates' spots.
    fn pick_guard_spot(
        &mut self,
        sim: &Sim,
        g: &BotGraph,
        p: &Player,
        t: &level::TargetDef,
    ) -> Option<usize> {
        let here = g.nearest(&sim.world, p.feet)?;
        let tac = &g.tactics[p.team.index()];
        let approach = sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1) + t.normal * 2.0;
        let mates: Vec<Vec3> = lock(&self.shared)
            .claims
            .iter()
            .filter(|(id, (team, _))| Some(**id) != self.me && *team == p.team)
            .map(|(_, (_, pos))| *pos)
            .collect();
        let mut best: Option<(f32, usize)> = None;
        for (n, tc) in tac.iter().enumerate() {
            let pos = g.pos[n];
            let d = pos.distance(approach);
            if tc.watches & (1 << t.id) == 0
                || d > 14.0
                || !g.connected(here, n)
                || sim.level.reloads_at(pos)
            {
                continue;
            }
            let mut s = 1.5 * tc.cover - 0.15 * (d - 6.0).abs() - 0.01 * pos.distance(p.feet);
            s += self.rng.random::<f32>() * (0.3 + 0.6 * (1.0 - self.skill));
            if mates.iter().any(|m| m.distance(pos) < 3.5) {
                s -= 3.0;
            }
            if best.is_none_or(|b| s > b.0) {
                best = Some((s, n));
            }
        }
        best.map(|b| b.1)
    }

    /// Attackers: until the team pushes, a destination in the enemy half becomes the staging
    /// point (see "Team play"). Reports this bot's state to the board.
    fn gate(
        &mut self,
        sim: &Sim,
        g: &BotGraph,
        p: &Player,
        dest: Option<(usize, Vec3)>,
        now: f64,
    ) -> Option<(usize, Vec3)> {
        let Some(me) = self.me else { return dest };
        // (Megatarget: the objectives are on both sides, nothing to push into.)
        if self.role != Role::Offense || sim.level.mega.is_some() {
            return dest;
        }
        let side = p.team.side();
        let z = p.feet.z * side;
        if z < -1.0 {
            // Over there already.
            self.pushing = true;
        } else if z > 12.0 {
            // Back home (refilling, respawned): the next push is a new one.
            self.pushing = false;
        }
        let mut s = lock(&self.shared);
        if s.solo {
            return dest;
        }
        let b = &mut s.boards[p.team.index()];
        let over_there = dest.is_some_and(|(n, _)| g.pos[n].z * side < -0.5);
        let mut state = Attacker {
            seen: now,
            gated: false,
            waiting: None,
        };
        let mut out = dest;
        // Refilling goes straight; fetching a grenade from the enemy half waits for the push.
        let reloading = matches!(self.errand, Some(Errand::Reload { .. }));
        if over_there && !self.pushing && !reloading {
            let since = *self.gate_since.get_or_insert(now);
            match b.staging(&sim.level, g, p.team) {
                Some(st) if now - since < GATE_GIVE_UP => {
                    let at = g.pos[st];
                    state.gated = true;
                    let there = Vec2::new(at.x - p.feet.x, at.z - p.feet.z).length()
                        < STAGING_REACH
                        && (at.y - p.feet.y).abs() < 1.0;
                    if there {
                        state.waiting =
                            Some(b.attackers.get(&me).and_then(|a| a.waiting).unwrap_or(now));
                        b.attackers.insert(me, state);
                        if b.push_now(now) {
                            if b.push_at.is_none_or(|t| now - t >= PUSH_JOIN) {
                                b.push_at = Some(now);
                            }
                            self.pushing = true;
                            state = Attacker {
                                seen: now,
                                gated: false,
                                waiting: None,
                            };
                        }
                    }
                    if !self.pushing {
                        out = Some((st, at));
                    }
                }
                _ => self.pushing = true,
            }
        }
        if !state.gated {
            self.gate_since = None;
        }
        b.attackers.insert(me, state);
        out
    }

    fn remember(&mut self, id: PlayerId, feet: Vec3, now: f64) {
        match self.memory.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                let dt = (now - s.at) as f32;
                if dt > 0.0 && dt < 0.5 {
                    s.vel = s.vel.lerp((feet - s.feet) / dt, 0.3);
                } else if dt >= 0.5 {
                    s.vel = Vec3::ZERO;
                }
                s.feet = feet;
                s.at = now;
            }
            None => self.memory.push(Seen {
                id,
                feet,
                vel: Vec3::ZERO,
                at: now,
            }),
        }
    }

    /// Who is in sight (in the field of view, or very close) and who is heard.
    fn perceive(&mut self, sim: &Sim, p: &Player, now: f64) {
        self.visible.clear();
        let eye = p.eye();
        let fwd = Vec2::new(self.aim.x, self.aim.z).normalize_or_zero();
        // (Less far in a blackout.)
        let sight = if matches!(sim.event, Some((ArenaEvent::Blackout, _))) {
            BLACKOUT_SIGHT
        } else {
            SIGHT
        };
        for o in &sim.players {
            if o.team == p.team || !o.alive() {
                continue;
            }
            let chest = o.chest();
            let to = chest - eye;
            let d = to.length();
            let in_view = d < SENSE || fwd.dot(Vec2::new(to.x, to.z).normalize_or_zero()) > FOV_COS;
            let mut known = false;
            if d < sight && in_view && o.seen_from(d) {
                // The chest if it shows, otherwise the head (over a half wall).
                let point = if clear(&sim.world, eye, chest) {
                    Some(chest)
                } else if clear(&sim.world, eye, o.eye()) {
                    Some(o.eye())
                } else {
                    None
                };
                if let Some(pt) = point {
                    self.visible.push((o.id, d, pt));
                    known = true;
                }
            }
            if d < HEAR || d < SHOT_HEAR && o.fire_cd > o.fire_interval(&sim.rules) - 0.05 {
                known = true;
            }
            if known {
                self.remember(o.id, o.feet, now);
            }
        }
        self.memory
            .retain(|s| now - s.at < MEMORY && sim.player(s.id).is_some_and(|o| o.alive()));
    }

    /// After a hit: look for the shooter (the nearest enemy with a line of sight), and maybe
    /// head for cover.
    fn react_to_hits(&mut self, sim: &Sim, p: &Player, now: f64) {
        if p.hits > self.prev_hits {
            let eye = p.eye();
            let enemies = sim.players.iter().filter(|o| o.team != p.team && o.alive());
            let suspect = enemies
                .clone()
                .filter(|o| clear(&sim.world, eye, o.eye()))
                .min_by(|a, b| a.feet.distance(p.feet).total_cmp(&b.feet.distance(p.feet)))
                .or_else(|| {
                    enemies
                        .filter(|o| o.feet.distance(p.feet) < 30.0)
                        .min_by(|a, b| a.feet.distance(p.feet).total_cmp(&b.feet.distance(p.feet)))
                });
            if let Some(o) = suspect {
                self.alert = Some((o.chest(), 1.5));
                self.remember(o.id, o.feet, now);
            }
            if p.hits + 1 >= sim.rules.hits_to_tag
                && self.plan == Plan::Engage
                && self.rng.random::<f32>() < 0.6
            {
                self.plan_left = 0.0;
                self.next_plan_hint = Some(Plan::Hold);
            }
        }
        self.prev_hits = p.hits;
    }
}

/// `dir` turned away by `angle` towards the side `phi` (around the line).
fn apply_error(dir: Vec3, (angle, phi): (f32, f32)) -> Vec3 {
    let right = dir.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(dir).normalize_or(Vec3::Y);
    let axis = right * phi.cos() + up * phi.sin();
    (Quat::from_axis_angle(axis, angle) * dir).normalize_or(dir)
}

/// Where a grenade thrown from `from` with `vel` first hits the arena (as the sim flies it).
fn first_impact(world: &World, from: Vec3, vel: Vec3) -> Option<Vec3> {
    let mut pos = from;
    let mut vel = vel;
    for _ in 0..(3.5 / TICK) as usize {
        vel.y -= GRAVITY * TICK;
        let to = pos + vel * TICK;
        if let Some((_, q, _)) = world.ray(pos, to) {
            return Some(q);
        }
        pos = to;
        if pos.y < -5.0 {
            return None;
        }
    }
    None
}

/// A throw from `from` whose first impact is within `THROW_LAND` of `aim`: low and high arcs
/// are tried, each simulated against the world and its speed corrected a couple of times.
/// Returns the velocity and the impact point.
pub fn solve_throw(world: &World, from: Vec3, aim: Vec3, max_speed: f32) -> Option<(Vec3, Vec3)> {
    let d3 = aim - from;
    let flat = Vec2::new(d3.x, d3.z);
    let dist = flat.length();
    if dist < 1.0 {
        return None;
    }
    let dir = flat / dist;
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    for ang in [0.2f32, 0.4, 0.6, 0.8, 1.0, 1.2] {
        let (s, c) = ang.sin_cos();
        let denom = 2.0 * c * c * (dist * s / c - d3.y);
        if denom <= 0.0 {
            continue;
        }
        let mut v = (GRAVITY * dist * dist / denom).sqrt();
        for _ in 0..3 {
            if v > max_speed {
                break;
            }
            let vel = Vec3::new(dir.x * v * c, v * s, dir.y * v * c);
            let Some(q) = first_impact(world, from, vel) else {
                break;
            };
            let err = Vec2::new(q.x - aim.x, q.z - aim.z).length();
            if err < THROW_LAND && (q.y - aim.y).abs() < 1.2 {
                if best.is_none_or(|b| err < b.0) {
                    best = Some((err, vel, q));
                }
                break;
            }
            // Short or long along the line: scale the speed (unless a wall stopped it early).
            let along = Vec2::new(q.x - from.x, q.z - from.z).dot(dir);
            if along < 0.5 * dist {
                break;
            }
            v *= (dist / along).sqrt();
        }
    }
    best.map(|(_, v, q)| (v, q))
}

impl Bot {
    /// Starts, finishes or drops errands: refilling when low on shots, fetching a grenade.
    fn update_errand(&mut self, sim: &Sim, g: &BotGraph, p: &Player) {
        self.errand_time += TICK;
        self.errand_retry -= TICK;
        let done = match self.errand {
            Some(Errand::Reload { .. }) => p.ammo >= sim.rules.ammo || self.errand_time > 30.0,
            Some(Errand::Pickup { id, .. }) => {
                !sim.pickups()
                    .any(|k| k.id == id && p.wants(k.kind, &sim.rules))
                    || self.errand_time > 15.0
            }
            None => false,
        };
        if done {
            self.errand = None;
            self.route_goal = None;
        }
        if self.errand_retry > 0.0 {
            return;
        }
        let Some(here) = g.nearest(&sim.world, p.feet) else {
            return;
        };
        if p.ammo < LOW_AMMO && !matches!(self.errand, Some(Errand::Reload { .. })) {
            // The nearest (by walking) reload station or own spawn pad.
            let cost = g.costs_from(&[here]);
            let best = sim
                .level
                .reloads
                .iter()
                .copied()
                .chain(sim.level.spawns[p.team.index()].iter().map(|s| s.pos))
                .filter_map(|pos| sim.level.nav.node_at(pos).map(|n| (cost[n], n, pos)))
                .filter(|c| c.0.is_finite())
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match best {
                Some((_, node, pos)) => self.set_errand(Errand::Reload { node, pos }),
                None => self.errand_retry = 3.0,
            }
        } else if self.errand.is_none() {
            let eye = p.eye();
            let best = sim
                .pickups()
                .filter(|k| {
                    p.wants(k.kind, &sim.rules)
                        && eye.distance(k.pos) < PICKUP_SIGHT
                        && sim.world.clear(eye, k.pos)
                })
                .filter_map(|k| {
                    let node = sim.level.nav.node_at(k.pos - Vec3::Y * PICKUP_HEIGHT)?;
                    g.connected(here, node).then_some((
                        eye.distance(k.pos),
                        k.id,
                        node,
                        k.pos - Vec3::Y * PICKUP_HEIGHT,
                    ))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, id, node, pos)) = best {
                self.set_errand(Errand::Pickup { id, node, pos });
            }
        }
    }

    fn set_errand(&mut self, e: Errand) {
        self.errand = Some(e);
        self.errand_time = 0.0;
        self.route_goal = None;
        self.peek = None;
    }

    /// Picks the next plan (and its spot); see the module documentation.
    fn decide(&mut self, sim: &Sim, g: &BotGraph, p: &Player) {
        self.plan_left = self.rng.random_range(10.0..20.0);
        self.release_claim();
        self.spot = None;
        self.chase = None;
        self.patrol = None;
        self.route_goal = None;
        self.peek = None;
        self.mega_spot = None;
        let role_plan = match (self.role, sim.level.mega.is_some()) {
            (Role::Offense, false) => Plan::Attack,
            (Role::Defense, false) => Plan::Defend,
            (Role::Offense, true) => Plan::Mega,
            // Defenders go for it too now and then, and otherwise hunt.
            (Role::Defense, true) => {
                if self.rng.random_bool(0.4) {
                    Plan::Mega
                } else {
                    Plan::Engage
                }
            }
        };
        let plan = self.next_plan_hint.take().unwrap_or_else(|| {
            let r: f32 = self.rng.random();
            let (role_w, engage_w) = match self.role {
                Role::Offense => (0.55, 0.3),
                Role::Defense => (0.55, 0.2),
            };
            if r < role_w {
                role_plan
            } else if r < role_w + engage_w {
                Plan::Engage
            } else {
                Plan::Hold
            }
        });
        // Fallbacks for arenas (or positions) without a spot of the kind.
        for plan in [plan, Plan::Hold, Plan::Engage] {
            if plan == Plan::Engage {
                self.plan = Plan::Engage;
                return;
            }
            if plan == Plan::Mega {
                self.plan = Plan::Mega;
                self.plan_left = self.rng.random_range(35.0..50.0);
                return;
            }
            if let Some(s) = self.pick_spot(sim, g, p, plan) {
                self.plan = plan;
                self.spot = Some(s);
                self.claim(p.team, g.pos[s]);
                return;
            }
        }
    }

    /// The best spot for `plan`, by cover, purpose, distance and a little chance, away from
    /// team mates' spots.
    fn pick_spot(&mut self, sim: &Sim, g: &BotGraph, p: &Player, plan: Plan) -> Option<usize> {
        let here = g.nearest(&sim.world, p.feet)?;
        let tac = &g.tactics[p.team.index()];
        let bits = |own: bool| -> u16 {
            sim.level
                .targets
                .iter()
                .filter(|t| (t.team == p.team) == own)
                .fold(0, |b, t| b | 1 << t.id)
        };
        let (enemy_bits, own_bits) = (bits(false), bits(true));
        let mates: Vec<Vec3> = lock(&self.shared)
            .claims
            .iter()
            .filter(|(id, (team, _))| Some(**id) != self.me && *team == p.team)
            .map(|(_, (_, pos))| *pos)
            .collect();
        let mut best: Option<(f32, usize)> = None;
        for (n, &t) in tac.iter().enumerate() {
            if !g.connected(here, n) {
                continue;
            }
            let pos = g.pos[n];
            // Not on a pad or station: stepping on and off while peeking would keep refilling.
            if sim.level.reloads_at(pos) {
                continue;
            }
            let dist = pos.distance(p.feet);
            let mut s = match plan {
                Plan::Attack => {
                    if t.shoots & enemy_bits == 0 {
                        continue;
                    }
                    // Not right in front of the panel, not too far either.
                    let d = sim
                        .level
                        .targets
                        .iter()
                        .filter(|tg| t.shoots & (1 << tg.id) != 0)
                        .map(|tg| tg.centre().distance(pos))
                        .fold(f32::MAX, f32::min);
                    2.0 * t.cover
                        - 0.01 * dist
                        - 0.1 * (4.0 - d).max(0.0)
                        - 0.04 * (d - 12.0).max(0.0)
                }
                Plan::Defend => {
                    if t.watches & own_bits == 0 || t.home > t.enemy_base {
                        continue;
                    }
                    1.5 * t.cover + 0.3 * (t.watches & own_bits).count_ones() as f32 - 0.01 * dist
                }
                Plan::Hold => {
                    if dist > 12.0 {
                        continue;
                    }
                    2.0 * t.cover - 0.06 * dist
                }
                Plan::Engage | Plan::Mega => continue,
            };
            // Worse bots care less about cover: their choice is noisier and cover counts less.
            s *= 0.5 + 0.5 * self.skill;
            s += self.rng.random::<f32>() * (0.5 + 1.2 * (1.0 - self.skill));
            if mates.iter().any(|m| m.distance(pos) < 3.5) {
                s -= 3.0;
            }
            if best.is_none_or(|b| s > b.0) {
                best = Some((s, n));
            }
        }
        best.map(|b| b.1)
    }

    /// Where to go now: the goal node and the exact end point (`None`: stay and fight).
    fn destination(&mut self, sim: &Sim, g: &BotGraph, p: &Player) -> Option<(usize, Vec3)> {
        if let Some(e) = self.errand {
            return Some(match e {
                Errand::Reload { node, pos } | Errand::Pickup { node, pos, .. } => (node, pos),
            });
        }
        match self.plan {
            Plan::Hold | Plan::Attack | Plan::Defend => self.spot.map(|s| (s, g.pos[s])),
            Plan::Engage => self.engage_dest(sim, g, p),
            Plan::Mega => self.mega_dest(sim, g, p),
        }
    }

    /// The Megatarget plan's next stop: a spot in a pocket whose secondary target this bot
    /// still has to hit, or with both hit the inside of the nearer doorway into the room
    /// (`None` once in the room: shoot from there).
    fn mega_dest(&mut self, sim: &Sim, g: &BotGraph, p: &Player) -> Option<(usize, Vec3)> {
        let m = sim.level.mega.as_ref()?;
        if p.power.is_some() {
            if m.in_room(p.feet) {
                return None;
            }
            let s = m.shields.iter().min_by(|a, b| {
                a.outside
                    .distance(p.feet)
                    .total_cmp(&b.outside.distance(p.feet))
            })?;
            let node = sim.level.nav.node_at(s.outside)?;
            return Some((node, s.inside));
        }
        let here = g.nearest(&sim.world, p.feet)?;
        if self
            .mega_spot
            .is_none_or(|(side, _)| p.mega_hits & (1 << side) != 0)
        {
            // The nearer secondary target still to hit, a random spot of those it's hit from.
            let mut sides: Vec<usize> = (0..2).filter(|i| p.mega_hits & (1 << i) == 0).collect();
            sides.sort_by(|a, b| {
                let d = |i: usize| m.secondaries[i].centre().distance(p.feet);
                d(*a).total_cmp(&d(*b))
            });
            self.mega_spot = sides.into_iter().find_map(|side| {
                let spots: Vec<usize> = g.mega_spots[side]
                    .iter()
                    .copied()
                    .filter(|&n| g.connected(here, n))
                    .collect();
                (!spots.is_empty()).then(|| (side, spots[self.rng.random_range(0..spots.len())]))
            });
        }
        let (_, n) = self.mega_spot?;
        Some((n, g.pos[n]))
    }

    fn engage_dest(&mut self, sim: &Sim, g: &BotGraph, p: &Player) -> Option<(usize, Vec3)> {
        // In sight and close enough: stand and fight.
        if self.visible.iter().any(|v| v.1 < KEEP_DIST) {
            return None;
        }
        let known = self
            .memory
            .iter()
            .min_by(|a, b| a.feet.distance(p.feet).total_cmp(&b.feet.distance(p.feet)))
            .copied();
        if let Some(t) = known {
            if self.chase.is_none_or(|(_, at)| at.distance(t.feet) > 3.0) {
                self.chase = sim
                    .level
                    .nav
                    .node_at(t.feet)
                    .or_else(|| g.nearest(&sim.world, t.feet))
                    .map(|n| (n, t.feet));
            }
            if let Some((n, _)) = self.chase {
                self.patrol = None;
                return Some((n, g.pos[n]));
            }
        }
        // Nobody known: patrol (attackers into the enemy half, defenders around the middle).
        let here = g.nearest(&sim.world, p.feet)?;
        if self.patrol.is_none_or(|n| g.pos[n].distance(p.feet) < 1.5) {
            let enemy = p.team.other();
            let mut pick = None;
            for k in 0..60 {
                let n = self.rng.random_range(0..g.pos.len());
                if !g.connected(here, n) {
                    continue;
                }
                let z = g.pos[n].z * enemy.side();
                let ok = match self.role {
                    Role::Offense => (0.0..22.0).contains(&z),
                    Role::Defense => z.abs() < 10.0,
                };
                if ok || k > 40 {
                    pick = Some(n);
                    break;
                }
            }
            self.patrol = pick;
        }
        self.patrol.map(|n| (n, g.pos[n]))
    }

    /// Gives up the current goal after it turned out unreachable.
    fn give_up(&mut self) {
        self.route.clear();
        self.route_goal = None;
        self.stuck = 0;
        if self.errand.is_some() {
            self.errand = None;
            self.errand_retry = 2.0;
            return;
        }
        match self.plan {
            Plan::Engage => {
                self.chase = None;
                self.patrol = None;
                self.plan_left = self.plan_left.min(1.0);
            }
            _ => {
                self.release_claim();
                self.spot = None;
                self.mega_spot = None;
                self.plan_left = 0.0;
            }
        }
    }

    /// Follows (building when needed) a route to `goal`, ending at `end`.
    fn steer(
        &mut self,
        sim: &Sim,
        g: &BotGraph,
        feet: Vec3,
        goal: usize,
        end: Vec3,
        arrive: f32,
        team: Option<Team>,
    ) -> Steer {
        let world = &sim.world;
        let to_end = Vec2::new(end.x - feet.x, end.z - feet.z);
        if to_end.length() < arrive && (end.y - feet.y).abs() < 1.0 {
            self.route.clear();
            self.stuck = 0;
            self.nudge = None;
            return Steer::Arrived;
        }
        if self.route_goal != Some(goal) || self.route.is_empty() {
            self.route_goal = Some(goal);
            let Some(start) = g.nearest(world, feet) else {
                return Steer::Unreachable;
            };
            let avoid = team.map(Team::other);
            let Some((path, doors)) = g.path_at(start, goal, &sim.level, sim.tick, avoid) else {
                return Steer::Unreachable;
            };
            self.route_gates = sim
                .level
                .gates
                .iter()
                .enumerate()
                .filter(|(_, gt)| {
                    path.windows(2).any(|w| {
                        let (a, b) = (w[0] as u32, w[1] as u32);
                        gt.link == (a, b) || gt.link == (b, a)
                    })
                })
                .map(|(k, _)| k)
                .collect();
            self.route = g.smooth(world, feet, &path);
            self.route_doors = doors;
            if end.distance(g.pos[goal]) > 0.05 {
                self.route.push(end);
            }
        }
        // A door on the route that is shut: wait in front of it if it opens soon, otherwise
        // plan again.
        let mut wait_at_door = false;
        for &d in &self.route_doors {
            let Some(m) = sim.level.movers.get(d) else {
                continue;
            };
            if m.passable(sim.tick) {
                continue;
            }
            let (wait, _) = m.passable_window(sim.tick);
            let near = Vec2::new(m.pos.x - feet.x, m.pos.z - feet.z).length() < 2.2;
            if wait > DOOR_WAIT_MAX + 60 {
                self.route.clear();
                self.route_goal = None;
                self.route_doors.clear();
                return Steer::Move(Vec2::ZERO);
            } else if near {
                wait_at_door = true;
            }
        }
        // A laser gate just ahead that is on, or switches on before the bot is through: wait.
        let next = self.route.first().copied().unwrap_or(end);
        let heading = Vec2::new(next.x - feet.x, next.z - feet.z);
        for &k in &self.route_gates {
            let Some(gt) = sim.level.gates.get(k) else {
                continue;
            };
            let c = gt.centre();
            let to = Vec2::new(c.x - feet.x, c.z - feet.z);
            if to.length() < 1.7
                && to.dot(heading) > 0.0
                && !gt.touches(feet)
                && gt.until_on(sim.tick) < GATE_CLEAR_TICKS
            {
                wait_at_door = true;
            }
        }
        if wait_at_door {
            self.progress_t = 0.0;
            self.progress_pos = feet;
            return Steer::Move(Vec2::ZERO);
        }
        // Cut corners left over after strafing or a nudge.
        self.shortcut_t -= TICK;
        if self.shortcut_t <= 0.0 {
            self.shortcut_t = 0.25;
            if self.route.len() >= 2 && walkable(world, feet, self.route[1]) {
                self.route.remove(0);
            }
        }
        while let Some(&w) = self.route.first() {
            let d = Vec2::new(w.x - feet.x, w.z - feet.z).length();
            let r = if self.route.len() == 1 { 0.3 } else { 0.55 };
            if d < r && (w.y - feet.y).abs() < 1.3 {
                self.route.remove(0);
            } else {
                break;
            }
        }
        // Not getting anywhere: nudge aside and plan again; give up after a few tries.
        self.progress_t += TICK;
        if self.progress_t >= 1.0 {
            let moved = feet.distance(self.progress_pos);
            if moved < 0.4 {
                self.stuck += 1;
                self.route.clear();
                let a = self.rng.random_range(0.0..TAU);
                self.nudge = Some((Vec2::new(a.cos(), a.sin()), 0.4));
                if self.stuck >= 4 {
                    return Steer::Unreachable;
                }
            } else if moved > 1.5 {
                self.stuck = 0;
            }
            self.progress_t = 0.0;
            self.progress_pos = feet;
        }
        if let Some((d, t)) = self.nudge {
            self.nudge = (t > TICK).then_some((d, t - TICK));
            return Steer::Move(d);
        }
        match self.route.first() {
            Some(&w) => Steer::Move(Vec2::new(w.x - feet.x, w.z - feet.z).normalize_or_zero()),
            None => Steer::Move(to_end.normalize_or_zero()),
        }
    }

    /// The enemy to shoot: the current one while in sight, else the nearest; one that can't be
    /// hit right now (recovering from a hit) only if there's nobody else. With the aim point and
    /// whether a shot would count.
    fn pick_target(&self, sim: &Sim, p: &Player) -> Option<(PlayerId, f32, Vec3, bool)> {
        // (Not behind an enemy barrier either: it would stop the bolt.)
        let eye = p.eye();
        let hittable = |id: PlayerId| {
            sim.player(id).is_some_and(|o| {
                o.cooldown < 0.5
                    && o.shield <= 0.0
                    && !sim.barrier_blocks(eye, o.chest(), Some(p.team))
            })
        };
        let mut cands: Vec<(PlayerId, f32, Vec3)> = self.visible.clone();
        cands.sort_by(|a, b| {
            let key = |c: &(PlayerId, f32, Vec3)| (!hittable(c.0), Some(c.0) != self.enemy, c.1);
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0)
                .then(ka.1.cmp(&kb.1))
                .then(ka.2.total_cmp(&kb.2))
        });
        cands.first().map(|&(id, d, pt)| (id, d, pt, hittable(id)))
    }

    /// The middle of an enemy target this bot can hit from where it stands (Megatarget mode:
    /// one of the targets that counts for it, or a turret's switch on its way).
    fn panel_in_reach(&self, sim: &Sim, p: &Player, eye: Vec3, muzzle: Vec3) -> Option<Vec3> {
        if let Some(m) = &sim.level.mega {
            let seen = |min: Vec3, max: Vec3| {
                hits_box(&sim.world, eye, min, max) && hits_box(&sim.world, muzzle, min, max)
            };
            if p.power.is_some() && m.in_room(p.feet) {
                return seen(m.core.min, m.core.max).then(|| m.core.centre());
            }
            if let Some(t) = (0..2)
                .filter(|_| p.power.is_none())
                .filter(|i| p.mega_hits & (1 << i) == 0)
                .map(|i| m.secondaries[i])
                .find(|t| t.centre().distance(eye) < 12.0 && seen(t.min, t.max))
            {
                return Some(t.centre());
            }
            if self.plan != Plan::Mega {
                return None;
            }
            return m
                .turrets
                .iter()
                .zip(&sim.turrets)
                .filter(|(t, st)| st.off <= 0.0 && t.switch.centre().distance(eye) < 22.0)
                .find(|(t, _)| seen(t.switch.min, t.switch.max))
                .map(|(t, _)| t.switch.centre());
        }
        sim.level
            .targets
            .iter()
            .filter(|t| t.team != p.team && t.centre().distance(eye) < TARGET_RANGE)
            .find(|t| hits_panel(&sim.world, eye, t) && hits_panel(&sim.world, muzzle, t))
            .map(|t| t.centre())
    }

    /// Where to look with nothing to shoot.
    fn look_dir(&self, sim: &Sim, p: &Player, move_dir: Vec2, now: f64) -> Vec3 {
        let eye = p.eye();
        if let Some((pt, _)) = self.alert {
            return (pt - eye).normalize_or(self.aim);
        }
        // Pre-aim where an enemy was seen a moment ago.
        if let Some(s) = self
            .memory
            .iter()
            .filter(|s| now - s.at < 3.0 && s.feet.distance(p.feet) < 30.0)
            .min_by(|a, b| a.feet.distance(p.feet).total_cmp(&b.feet.distance(p.feet)))
        {
            return (s.feet + Vec3::Y * CHEST - eye).normalize_or(self.aim);
        }
        if move_dir != Vec2::ZERO {
            return Vec3::new(move_dir.x, 0.0, move_dir.y);
        }
        // Holding: watch the own targets' approach (defending) or the enemy's side, scanning.
        let watch = if self.plan == Plan::Defend {
            sim.level
                .targets
                .iter()
                .filter(|t| t.team == p.team)
                .map(|t| {
                    sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1) + t.normal * 3.0
                })
                .min_by(|a, b| a.distance(p.feet).total_cmp(&b.distance(p.feet)))
        } else {
            None
        };
        let at = watch.unwrap_or_else(|| sim.level.base_centre(p.team.other()));
        let flat = Vec3::new(at.x - eye.x, 0.0, at.z - eye.z).normalize_or(Vec3::NEG_Z);
        let scan = 0.6 * (now as f32 * 0.5 + self.scan_phase).sin();
        Quat::from_rotation_y(scan) * flat
    }

    fn roll_error(&mut self) {
        // About normally distributed (sum of three uniforms), standard deviation `aim_error`.
        let n: f32 = (0..3).map(|_| self.rng.random::<f32>()).sum::<f32>() - 1.5;
        self.shot_err = (self.aim_error * n * 2.0, self.rng.random_range(0.0..TAU));
    }

    /// A grenade throw at a group of known enemies, if there is a good one.
    fn try_grenade(&mut self, sim: &Sim, p: &Player, now: f64) -> Option<(Vec3, Vec3)> {
        if p.grenades == 0 {
            return None;
        }
        self.grenade_t -= TICK;
        if self.grenade_t > 0.0 {
            return None;
        }
        self.grenade_t = 0.4;
        let known: Vec<Vec3> = self
            .memory
            .iter()
            .filter(|s| now - s.at < 3.0)
            .map(|s| s.feet)
            .collect();
        let mates: Vec<Vec3> = sim
            .players
            .iter()
            .filter(|o| o.team == p.team && o.id != p.id && o.alive())
            .map(|o| o.feet)
            .collect();
        let flat = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length();
        let mut best: Option<(usize, f32, Vec3)> = None;
        for a in &known {
            let group: Vec<Vec3> = known
                .iter()
                .filter(|b| b.distance(*a) < GROUP_RADIUS)
                .copied()
                .collect();
            if group.len() < 2 {
                continue;
            }
            let c = group.iter().sum::<Vec3>() / group.len() as f32;
            let d = flat(c, p.feet);
            if !(THROW_MIN..=THROW_MAX).contains(&d)
                || mates.iter().any(|m| m.distance(c) < MATE_CLEAR)
            {
                continue;
            }
            if best.is_none_or(|b| group.len() > b.0 || group.len() == b.0 && d < b.1) {
                best = Some((group.len(), d, c));
            }
        }
        let (n, _, aim) = best?;
        // Readiness grows with skill: Easy bots often hold on to their grenade, and only throw
        // at bigger groups with confidence.
        let ready = (0.15 + 0.85 * self.skill) * if n >= 3 { 1.5 } else { 1.0 };
        if self.rng.random::<f32>() > ready {
            self.grenade_t = 1.5;
            return None;
        }
        let dir = Vec2::new(aim.x - p.feet.x, aim.z - p.feet.z).normalize_or_zero();
        let mut rel = Vec3::new(dir.x * 0.3, 1.5, dir.y * 0.3);
        if !sim.world.clear(p.eye(), p.feet + rel) {
            rel = p.eye() - p.feet;
        }
        let (vel, land) = solve_throw(
            &sim.world,
            p.feet + rel,
            aim + Vec3::Y * 0.05,
            sim.rules.grenade_max_speed,
        )?;
        if land.distance(p.feet) < SELF_CLEAR || mates.iter().any(|m| m.distance(land) < MATE_CLEAR)
        {
            return None;
        }
        self.grenade_t = 2.0;
        Some((rel, vel))
    }
}

impl Drop for Bot {
    fn drop(&mut self) {
        self.release_claim();
    }
}

impl Controller for Bot {
    fn command(&mut self, sim: &Sim, me: PlayerId) -> PlayerCmd {
        if self.me != Some(me) {
            self.release_claim();
            self.me = Some(me);
        }
        let g = self.graph_for(sim);
        let Some(p) = sim.player(me) else {
            return PlayerCmd::default();
        };
        if !self.aim.is_finite() || self.aim.length_squared() < 0.5 {
            self.aim = Vec3::NEG_Z;
        }
        if !p.alive() {
            self.reset();
            return self.pose(PlayerCmd::default());
        }
        let now = sim.tick as f64 * TICK as f64;
        let world = &sim.world;
        let feet = p.feet;

        // Senses and decisions.
        self.perceive(sim, p, now);
        self.share(sim, &g, p, now);
        self.react_to_hits(sim, p, now);
        if let Some((pt, t)) = self.alert {
            self.alert = (t > TICK).then_some((pt, t - TICK));
        }
        self.update_errand(sim, &g, p);
        self.plan_left -= TICK;
        if p.power.is_some() {
            // The room is open to it: nothing else matters (not even refilling).
            if self.plan != Plan::Mega {
                self.release_claim();
                self.spot = None;
                self.chase = None;
                self.route_goal = None;
                self.plan = Plan::Mega;
            }
            self.errand = None;
            self.plan_left = self.plan_left.max(1.0);
        }
        if self.plan_left <= 0.0 {
            self.decide(sim, &g, p);
        }

        // Moving.
        let mut move_dir = Vec2::ZERO;
        let mut anchor = None;
        let dest = self.destination(sim, &g, p);
        let dest = self.gate(sim, &g, p, dest, now);
        // With the power, at a doorway of the room: straight through (inside, the nearest nodes
        // aren't linked to the rest).
        let doorway = sim.level.mega.as_ref().and_then(|m| {
            if p.power.is_none() || m.in_room(feet) {
                return None;
            }
            m.shields.iter().find_map(|s| {
                let mid = (s.outside + s.inside) / 2.0;
                (Vec2::new(mid.x - feet.x, mid.z - feet.z).length() < 1.3).then(|| {
                    Vec2::new(s.inside.x - feet.x, s.inside.z - feet.z).normalize_or_zero()
                })
            })
        });
        let dest = if doorway.is_some() { None } else { dest };
        if let Some(d) = doorway {
            move_dir = d;
            self.route.clear();
            self.route_goal = None;
        }
        match dest {
            None if doorway.is_some() => {}
            Some((node, end)) => {
                let arrive = if self.errand.is_none() && self.plan != Plan::Engage {
                    1.0
                } else {
                    0.45
                };
                let team = sim.player(me).map(|p| p.team);
                match self.steer(sim, &g, feet, node, end, arrive, team) {
                    Steer::Move(d) => move_dir = d,
                    Steer::Arrived => anchor = Some(end),
                    Steer::Unreachable => self.give_up(),
                }
            }
            None => {
                self.route.clear();
                self.route_goal = None;
                anchor = Some(feet);
            }
        }
        if move_dir == Vec2::ZERO {
            self.progress_t = 0.0;
            self.progress_pos = feet;
        }

        // Fighting: strafe while moving, or around the spot held.
        let target = self.pick_target(sim, p);
        if target.map(|t| t.0) != self.enemy {
            self.enemy = target.map(|t| t.0);
            self.seen_for = 0.0;
            self.roll_error();
        }
        if target.is_some() {
            self.seen_for += TICK;
            self.peek = None;
            if self.rng.random::<f32>() < TICK * 0.8 {
                self.strafe = -self.strafe;
            }
            let fwd = if move_dir != Vec2::ZERO {
                move_dir
            } else {
                Vec2::new(self.aim.x, self.aim.z).normalize_or_zero()
            };
            let side = Vec2::new(-fwd.y, fwd.x) * self.strafe;
            if doorway.is_some() {
                // Straight through.
            } else if move_dir != Vec2::ZERO {
                move_dir = (move_dir * 0.55 + side * 0.65).normalize_or_zero();
            } else if let Some(a) = anchor
                && !matches!(self.errand, Some(Errand::Reload { .. }))
            {
                let back = Vec2::new(a.x - feet.x, a.z - feet.z);
                if back.length() > 1.2 {
                    move_dir = back.normalize_or_zero();
                } else if walkable(world, feet, feet + Vec3::new(side.x, 0.0, side.y)) {
                    move_dir = side * 0.7;
                } else {
                    self.strafe = -self.strafe;
                }
            }
        } else if anchor.is_some() && self.errand.is_none() && self.plan != Plan::Engage {
            // Holding a spot: step out to have a look now and then.
            self.peek_t -= TICK;
            if self.peek.is_none() && self.peek_t <= 0.0 {
                self.peek_t = self.rng.random_range(3.0..7.0);
                let a = self.rng.random_range(0.0..8.0f32).floor() * PI / 4.0;
                let d = Vec2::new(a.cos(), a.sin());
                if walkable(world, feet, feet + Vec3::new(d.x, 0.0, d.y) * 1.5) {
                    self.peek = Some((d, 0.5));
                }
            }
        }
        if let Some((d, t)) = self.peek {
            // Out for half a second; the route brings it back to the spot afterwards.
            if target.is_none() {
                move_dir = d * 0.8;
            }
            self.peek = (t > TICK).then_some((d, t - TICK));
        }

        // Aiming and shooting.
        let eye = p.eye();
        let body = Quat::from_rotation_y(yaw_of(Vec3::new(self.aim.x, 0.0, self.aim.z)));
        let muzzle = feet + body * Vec3::new(0.18, EYE - 0.25, -0.35);
        let shoot_at = match target {
            Some((id, d, point, ok)) => {
                let vel = self
                    .memory
                    .iter()
                    .find(|s| s.id == id)
                    .map_or(Vec3::ZERO, |s| s.vel);
                let lead = vel * (d / sim.rules.blaster_bolt_speed) * (0.4 + 0.6 * self.skill);
                Some((point + lead, ok && self.seen_for >= self.reaction))
            }
            None => self.panel_in_reach(sim, p, eye, muzzle).map(|c| (c, true)),
        };
        let want = match shoot_at {
            Some((pt, _)) => apply_error((pt - muzzle).normalize_or(self.aim), self.shot_err),
            None => self.look_dir(sim, p, move_dir, now),
        };
        let want = want.normalize_or(self.aim);
        let angle = self.aim.angle_between(want);
        if angle > 1e-4 {
            let step = (self.turn_rate * TICK).min(angle);
            let (axis, _) = Quat::from_rotation_arc(self.aim, want).to_axis_angle();
            self.aim = (Quat::from_axis_angle(axis, step) * self.aim).normalize_or(want);
        }
        let mut fire = false;
        let aimed = self.aim.angle_between(want) < 0.025;
        if let Some(held) = p.charging {
            // The charge gun, charging: hold on for a beam at an enemy further off (until it's
            // full and on target), otherwise (and at target panels: bolts score as much, and
            // quicker) let go at once for a plain bolt.
            let far = target.is_some()
                && shoot_at.is_some_and(|(pt, ok)| ok && pt.distance(muzzle) > BEAM_MIN_RANGE)
                && p.ammo >= sim.rules.beam_cost;
            let full = held >= sim.rules.charge_time;
            fire = far && !(full && aimed) && held < sim.rules.charge_time * 2.5;
            if !fire {
                self.roll_error();
            }
        } else if let Some((pt, true)) = shoot_at
            && (!p.shotgun || pt.distance(muzzle) < sim.rules.shotgun_range * 0.9)
            && !self.fire_prev
            && p.fire_cd <= 0.0
            && p.cooldown <= 0.0
            && p.ammo > 0
            && aimed
        {
            fire = true;
            self.roll_error();
        }
        self.fire_prev = fire;

        // Never into a forcefield without the power (strafing, nudges).
        if let Some(m) = &sim.level.mega
            && p.power.is_none()
            && move_dir != Vec2::ZERO
            && m.touching_shield(feet + Vec3::new(move_dir.x, 0.0, move_dir.y) * 0.35)
                .is_some()
        {
            move_dir = Vec2::ZERO;
            self.strafe = -self.strafe;
        }

        let throw = self.try_grenade(sim, p, now);
        // A carried shield goes on in a fight.
        let shield = p.shields > 0 && p.shield <= 0.0 && target.is_some();
        // A carried barrier goes up when hit in a fight, towards the enemy (the bot faces its
        // aim).
        let barrier = p.barriers > 0 && p.cooldown > 0.0 && target.is_some();
        self.pose(PlayerCmd {
            stick: move_dir.clamp_length_max(1.0),
            fire,
            throw,
            shield,
            barrier,
            ..Default::default()
        })
    }
}

impl Bot {
    /// Fills in the body, head, hands and blaster from the current aim.
    fn pose(&self, mut cmd: PlayerCmd) -> PlayerCmd {
        let yaw = yaw_of(Vec3::new(self.aim.x, 0.0, self.aim.z));
        let body = Quat::from_rotation_y(yaw);
        let look = look_rot(self.aim);
        cmd.yaw = yaw;
        cmd.head = Pose {
            pos: eye_on_neck(Vec3::Y * EYE, look),
            rot: look,
        };
        cmd.gun_hand = Some(Hand::Right);
        cmd.muzzle = Pose {
            pos: body * Vec3::new(0.18, EYE - 0.25, -0.35),
            rot: look,
        };
        cmd.hands[1] = Pose {
            pos: body * Vec3::new(0.2, 1.3, -0.25),
            rot: look,
        };
        cmd.hands[0] = Pose {
            pos: body * Vec3::new(-0.2, 1.1, -0.15),
            rot: body,
        };
        cmd
    }
}

/// The bots of a game, by player id.
#[derive(Default)]
pub struct Bots {
    pub bots: HashMap<PlayerId, Box<dyn Controller>>,
    /// The arena map and spot claims, shared by this game's bots.
    shared: Shared,
    /// Roles handed out, to keep a mix in each team.
    roles: HashMap<PlayerId, (Team, Role)>,
    /// The skill last handed to `fill` / `fill_team` (for bots that come back later).
    skill: Option<f32>,
}

impl Bots {
    /// Team play on (the default) or off: off, every bot plays on its own (no shared sightings,
    /// no grouping up, no alarms).
    pub fn set_teamwork(&mut self, on: bool) {
        lock(&self.shared).solo = !on;
    }

    /// The skill of this game's bots: the one last given to `fill` or `fill_team`, Normal's
    /// before that.
    pub fn skill(&self) -> f32 {
        self.skill.unwrap_or(Difficulty::Normal.skill())
    }

    fn add(&mut self, sim: &Sim, id: PlayerId, seed: u64, skill: f32) {
        self.skill = Some(skill);
        let Some(team) = sim.player(id).map(|p| p.team) else {
            return;
        };
        let count = |r: Role| {
            self.roles
                .values()
                .filter(|&&(t, rr)| t == team && rr == r)
                .count()
        };
        let role = if count(Role::Offense) <= count(Role::Defense) {
            Role::Offense
        } else {
            Role::Defense
        };
        self.roles.insert(id, (team, role));
        self.bots.insert(
            id,
            Box::new(Bot::with_role(seed, skill, role, self.shared.clone())),
        );
    }

    /// Adds `n` bots, balancing the teams.
    pub fn fill(&mut self, sim: &mut Sim, n: usize, skill: f32) {
        for _ in 0..n {
            let name = bot_name(sim);
            if let Some(id) = sim.add_player(&name, None, true) {
                let seed = sim.level.seed ^ (id as u64 * 7919);
                self.add(sim, id, seed, skill);
            }
        }
    }

    /// Adds one bot to `team`.
    pub fn fill_team(&mut self, sim: &mut Sim, team: Team, skill: f32) {
        let name = bot_name(sim);
        if let Some(id) = sim.add_player(&name, Some(team), true) {
            let seed = sim.level.seed ^ (id as u64 * 7919) ^ sim.tick;
            self.add(sim, id, seed, skill);
        }
    }

    /// Removes one bot of `team`; false if it has none.
    pub fn remove_one(&mut self, sim: &mut Sim, team: Team) -> bool {
        let Some(id) = sim
            .players
            .iter()
            .find(|p| p.bot && p.team == team && self.bots.contains_key(&p.id))
            .map(|p| p.id)
        else {
            return false;
        };
        self.bots.remove(&id);
        self.roles.remove(&id);
        sim.remove_player(id);
        true
    }

    /// Sets every bot's command for the next step.
    pub fn think(&mut self, sim: &mut Sim) {
        // In id order, so that games replay the same (bots share spot claims).
        let mut ids: Vec<PlayerId> = self.bots.keys().copied().collect();
        ids.sort_unstable();
        let mut cmds = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(bot) = self.bots.get_mut(&id) {
                cmds.push((id, bot.command(sim, id)));
            }
        }
        for (id, cmd) in cmds {
            sim.set_cmd(id, cmd);
        }
    }
}

/// A bot name nobody in the game has yet (numbered once the list runs out).
fn bot_name(sim: &Sim) -> String {
    const NAMES: [&str; 18] = [
        "Volt", "Pixel", "Quasar", "Neon", "Flux", "Glitch", "Nova", "Ion", "Laser", "Byte",
        "Photon", "Zap", "Echo", "Spark", "Prism", "Vector", "Pulse", "Arc",
    ];
    let taken = |n: &str| sim.players.iter().any(|p| p.name == n);
    (0..)
        .map(|k| {
            let base = NAMES[k % NAMES.len()];
            if k < NAMES.len() {
                format!("{base} (bot)")
            } else {
                format!("{base} {} (bot)", k / NAMES.len() + 1)
            }
        })
        .find(|n| !taken(n))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;

    #[derive(Debug, Default)]
    struct Stats {
        scores: [u32; 2],
        fired: u32,
        /// Shots aimed at a target panel (roughly: towards a panel, from its front).
        panel_shots: u32,
        hits: u32,
        target_hits: u32,
        /// Target hits by offensive bots.
        offense_target_hits: u32,
        reloads: u32,
        pickups: u32,
        grenades: u32,
        launches: u32,
        zaps: u32,
        moved: f32,
    }

    /// Plays a match of `n` bots for `secs` seconds.
    fn play(seed: u64, n: usize, secs: f32, rules: Rules) -> Stats {
        let mut sim = Sim::new(
            seed,
            Rules {
                match_time: secs,
                ..rules
            },
        );
        let mut bots = Bots::default();
        bots.fill(&mut sim, n, 0.6);
        let mut s = Stats::default();
        let mut last: Vec<Vec3> = sim.players.iter().map(|p| p.feet).collect();
        for _ in 0..((secs + 1.0) / TICK) as usize {
            bots.think(&mut sim);
            sim.step();
            for (p, l) in sim.players.iter().zip(last.iter_mut()) {
                if (p.feet - *l).length() < 2.0 {
                    s.moved += (p.feet - *l).length();
                }
                *l = p.feet;
            }
            for e in sim.drain_events() {
                match e {
                    SimEvent::Fired { player, pos, dir } => {
                        s.fired += 1;
                        let team = sim.player(player).map(|p| p.team);
                        if sim.level.targets.iter().any(|t| {
                            Some(t.team) != team
                                && (pos - t.centre()).dot(t.normal) > 0.0
                                && dir.angle_between(t.centre() - pos) < 0.25
                        }) {
                            s.panel_shots += 1;
                        }
                    }
                    SimEvent::Hit { .. } => s.hits += 1,
                    SimEvent::TargetHit { by, .. } => {
                        s.target_hits += 1;
                        if bots.roles.get(&by).is_some_and(|r| r.1 == Role::Offense) {
                            s.offense_target_hits += 1;
                        }
                    }
                    SimEvent::Reloading { .. } => s.reloads += 1,
                    SimEvent::PickedUp { .. } => s.pickups += 1,
                    SimEvent::GrenadeThrown { .. } => s.grenades += 1,
                    SimEvent::Launched { .. } => s.launches += 1,
                    SimEvent::GateZap { .. } => s.zaps += 1,
                    _ => {}
                }
            }
        }
        assert!(sim.over);
        s.scores = sim.scores;
        println!(
            "seed {seed}, {n} bots, {secs} s: scores {:?}, shots {} ({} at panels), hits {}, target hits {} \
             ({} by attackers), reloads {}, pickups {}, grenades thrown {}, jump pads {}, gate zaps {}, walked {:.0} m",
            s.scores,
            s.fired,
            s.panel_shots,
            s.hits,
            s.target_hits,
            s.offense_target_hits,
            s.reloads,
            s.pickups,
            s.grenades,
            s.launches,
            s.zaps,
            s.moved
        );
        s
    }

    #[test]
    fn bots_play_a_match() {
        let s = play(9, 8, 120.0, Rules::default());
        assert!(s.fired > 20, "bots fire: {s:?}");
        assert!(s.hits > 3, "bots hit: {s:?}");
        assert!(s.moved > 200.0, "bots move: {s:?}");
    }

    #[test]
    fn bot_graph_is_quick_and_finds_paths() {
        let mut total = std::time::Duration::ZERO;
        for seed in 0..20u64 {
            let level = Level::generate(seed, level::ArenaSize::Small);
            let world = level.world();
            let t0 = std::time::Instant::now();
            let g = BotGraph::build(&level, &world);
            total += t0.elapsed();
            assert!(g.tactics.iter().all(|t| t.len() == g.pos.len()));
            // Some spots exist for both jobs.
            for team in Team::ALL {
                let tac = &g.tactics[team.index()];
                assert!(
                    tac.iter().any(|t| t.shoots != 0),
                    "seed {seed}: no spot to hit {team:?}'s enemy targets from"
                );
                assert!(
                    tac.iter().any(|t| t.watches != 0),
                    "seed {seed}: no spot to guard {team:?}'s targets from"
                );
            }
            let diagonals: usize = (0..g.pos.len())
                .map(|i| g.links[i].len() - level.nav.links[i].len())
                .sum::<usize>()
                / 2;
            assert!(diagonals > 50, "seed {seed}: {diagonals} diagonals");
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let valid: Vec<usize> = (0..g.pos.len()).filter(|&i| g.valid[i]).collect();
            for _ in 0..300 {
                let a = valid[rng.random_range(0..valid.len())];
                let b = valid[rng.random_range(0..valid.len())];
                let bfs = level.nav.path(a, b);
                let astar = g.path(a, b);
                assert_eq!(bfs.is_some(), astar.is_some(), "seed {seed}: {a} -> {b}");
                if let (Some(bfs), Some(path)) = (bfs, astar) {
                    assert_eq!((path[0], *path.last().unwrap()), (a, b));
                    assert!(
                        path.windows(2)
                            .all(|w| g.links[w[0]].iter().any(|l| l.0 as usize == w[1]))
                    );
                    assert!(g.path_cost(&path) <= g.path_cost(&bfs) + 1e-3);
                    // Smoothed waypoints are walkable one to the next on flat stretches.
                    let pts = g.smooth(&world, g.pos[a], &path);
                    assert!(pts.len() <= path.len() && pts.last() == Some(&g.pos[b]));
                }
            }
        }
        let avg = total.as_secs_f32() * 1000.0 / 20.0;
        println!("bot graph: {avg:.1} ms per arena");
        let limit = if cfg!(debug_assertions) { 400.0 } else { 40.0 };
        assert!(avg < limit, "{avg} ms");
    }

    #[test]
    fn bots_attack_targets_reload_and_use_grenades() {
        // Two arenas, three and a half minutes each.
        let mut total = Stats::default();
        for seed in [3, 12] {
            let s = play(seed, 8, 210.0, Rules::default());
            total.target_hits += s.target_hits;
            total.offense_target_hits += s.offense_target_hits;
            total.reloads += s.reloads;
            total.pickups += s.pickups;
            total.grenades += s.grenades;
            total.launches += s.launches;
        }
        assert!(
            total.offense_target_hits > 0,
            "offensive bots hit enemy targets: {total:?}"
        );
        assert!(total.reloads > 0, "bots reload: {total:?}");
        assert!(total.pickups > 0, "bots pick up grenades: {total:?}");
        assert!(total.grenades > 0, "bots throw grenades: {total:?}");
        assert!(total.launches > 0, "bots take jump pads: {total:?}");
    }

    #[test]
    fn bots_miss_a_fair_share() {
        // No grenades (nor powerups), so every hit is a blaster's; shots at panels don't count.
        let s = play(
            21,
            8,
            150.0,
            Rules {
                pickup_pairs: 0,
                powerup_pairs: 0,
                ..Rules::default()
            },
        );
        let at_players = s.fired.saturating_sub(s.panel_shots).max(1);
        let rate = s.hits as f32 / at_players as f32;
        println!("hit rate at players: {rate:.2}");
        assert!(s.hits > 3 && rate < 0.6, "{rate}: {s:?}");
    }

    /// When each of a lone team's attackers first got into the enemy half (seconds), with team
    /// play on or off.
    fn crossings(seed: u64, teamwork: bool) -> Vec<f32> {
        // (No powerups to fetch on the way.)
        let rules = Rules {
            powerup_pairs: 0,
            ..Rules::default()
        };
        let mut sim = Sim::new(seed, rules);
        let mut bots = Bots::default();
        bots.set_teamwork(teamwork);
        for _ in 0..6 {
            bots.fill_team(&mut sim, Team::Red, 0.6);
        }
        let mut attackers: Vec<PlayerId> = bots
            .roles
            .iter()
            .filter(|(_, r)| r.1 == Role::Offense)
            .map(|(id, _)| *id)
            .collect();
        attackers.sort_unstable();
        // One attacker starts out ahead, near the centre line: on its own it would be over
        // long before the others.
        let nav = &sim.level.nav;
        let ahead = (0..sim.level.dims().cells())
            .filter(|&n| {
                nav.valid[n]
                    && (4.0..6.0).contains(&nav.pos[n].z)
                    && !sim.level.reloads_at(nav.pos[n])
            })
            .min_by(|&a, &b| nav.pos[a].x.abs().total_cmp(&nav.pos[b].x.abs()))
            .map(|n| nav.pos[n])
            .unwrap();
        let pi = sim
            .players
            .iter()
            .position(|p| p.id == attackers[0])
            .unwrap();
        sim.players[pi].feet = ahead;
        let mut first: HashMap<PlayerId, f32> = HashMap::new();
        for k in 0..(60.0 / TICK) as usize {
            bots.think(&mut sim);
            sim.step();
            for &id in &attackers {
                if sim.player(id).is_some_and(|p| p.feet.z < -1.0) {
                    first.entry(id).or_insert(k as f32 * TICK);
                }
            }
        }
        let mut t: Vec<f32> = attackers
            .iter()
            .map(|id| first.get(id).copied().unwrap_or(60.0))
            .collect();
        t.sort_by(f32::total_cmp);
        t
    }

    #[test]
    fn attackers_group_up_before_pushing() {
        // When the attackers get into the enemy half, team play on and off.
        let (mut with, mut without) = (Vec::new(), Vec::new());
        for seed in 1..=6 {
            let a = crossings(seed, true);
            let b = crossings(seed, false);
            println!("seed {seed}: together {a:?}, alone {b:?}");
            with.push(a);
            without.push(b);
        }
        // Arenas where the first two went over within a second and a half of each other (a
        // lone push after the timeout can happen when the others are busy elsewhere).
        let close = |runs: &[Vec<f32>]| runs.iter().filter(|t| t[1] - t[0] < 1.5).count();
        let (c_with, c_without) = (close(&with), close(&without));
        println!("first two together in {c_with} of 6 arenas with team play, {c_without} without");
        assert!(
            c_with >= 4 && c_with >= c_without + 3,
            "together {c_with}, alone {c_without}"
        );
    }

    /// A lone team's defenders' mean distance to the opening of one of their targets, some
    /// seconds after it was hit (with team play on or off).
    fn guard_distance(seed: u64, teamwork: bool) -> (f32, f32) {
        let mut sim = Sim::new(seed, Rules::default());
        let mut bots = Bots::default();
        bots.set_teamwork(teamwork);
        for _ in 0..6 {
            bots.fill_team(&mut sim, Team::Red, 0.6);
        }
        let defenders: Vec<PlayerId> = bots
            .roles
            .iter()
            .filter(|(_, r)| r.1 == Role::Defense)
            .map(|(id, _)| *id)
            .collect();
        // The red target furthest from the middle of the defenders after they settled.
        for _ in 0..(8.0 / TICK) as usize {
            bots.think(&mut sim);
            sim.step();
        }
        let mean_to = |sim: &Sim, at: Vec3| {
            defenders
                .iter()
                .filter_map(|id| sim.player(*id))
                .map(|p| p.feet.distance(at))
                .sum::<f32>()
                / defenders.len() as f32
        };
        let t = *sim
            .level
            .targets
            .iter()
            .filter(|t| t.team == Team::Red)
            .max_by(|a, b| {
                let at = |t: &level::TargetDef| {
                    mean_to(
                        &sim,
                        sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1),
                    )
                };
                at(a).total_cmp(&at(b))
            })
            .unwrap();
        let opening = sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1);
        let before = mean_to(&sim, opening);
        // Hit by an unknown shooter (nobody plays Blue).
        sim.last_target_hit[t.id as usize] = Some((sim.tick, 200));
        for _ in 0..(10.0 / TICK) as usize {
            bots.think(&mut sim);
            sim.step();
        }
        (before, mean_to(&sim, opening))
    }

    #[test]
    fn defenders_converge_after_target_hits() {
        let (mut with, mut without) = (0.0, 0.0);
        for seed in 1..=6 {
            let a = guard_distance(seed, true);
            let b = guard_distance(seed, false);
            println!(
                "seed {seed}: defenders' distance before -> after: together {a:?}, alone {b:?}"
            );
            assert!(a.1 < 15.0, "seed {seed}: defenders didn't come: {a:?}");
            with += a.1;
            without += b.1;
        }
        println!(
            "mean distance after the hit: together {:.1} m, alone {:.1} m",
            with / 6.0,
            without / 6.0
        );
        assert!(with < 0.75 * without, "together {with}, alone {without}");
    }

    #[test]
    fn routes_use_doors_only_while_open() {
        let mut checked = 0;
        for seed in 0..10u64 {
            let level = Level::generate(seed, level::ArenaSize::Small);
            let world = level.world();
            let g = BotGraph::build(&level, &world);
            let Some(&(a, b, door)) = level.nav.doors.first() else {
                continue;
            };
            let m = level.movers[door];
            let tick_where = |want: f32| {
                (0..m.period() as u64 * 2).find(|&t| (0..400).all(|k| m.fraction(t + k) == want))
            };
            // Open (and staying open): straight through.
            let t = tick_where(1.0).unwrap();
            let (path, doors) = g.path_at(a as usize, b as usize, &level, t, None).unwrap();
            assert_eq!((path.len(), doors), (2, vec![door]), "seed {seed}");
            // Shut for a long while: the long way round (perhaps through another door, open then).
            let t = (0..m.period() as u64)
                .find(|&t| m.passable_window(t).0 > DOOR_WAIT_MAX + 10)
                .unwrap();
            let (path, doors) = g.path_at(a as usize, b as usize, &level, t, None).unwrap();
            assert!(
                path.len() > 2 && !doors.contains(&door),
                "seed {seed}: {} nodes, doors {doors:?} (shut {door})",
                path.len()
            );
            checked += 1;
        }
        assert!(checked >= 8);
    }

    #[test]
    fn difficulty_sets_skill() {
        for d in Difficulty::ALL {
            assert_eq!(Difficulty::parse(d.name()), Some(d));
        }
        assert_eq!(Difficulty::default(), Difficulty::Normal);
        assert!(Difficulty::parse("impossible").is_none());
        // On average over many bots: Easy aims worse, reacts and turns slower than Hard.
        let avg = |d: Difficulty| {
            let bots: Vec<Bot> = (0..50).map(|k| Bot::new(k, d.skill())).collect();
            let n = bots.len() as f32;
            (
                bots.iter().map(|b| b.aim_error).sum::<f32>() / n,
                bots.iter().map(|b| b.reaction).sum::<f32>() / n,
                bots.iter().map(|b| b.turn_rate).sum::<f32>() / n,
            )
        };
        let (easy, normal, hard) = (
            avg(Difficulty::Easy),
            avg(Difficulty::Normal),
            avg(Difficulty::Hard),
        );
        assert!(
            easy.0 > normal.0 && normal.0 > hard.0 && hard.0 > 1.5f32.to_radians(),
            "{easy:?} {normal:?} {hard:?}"
        );
        assert!(easy.1 > normal.1 && normal.1 > hard.1);
        assert!(easy.2 < normal.2 && normal.2 < hard.2);
    }

    #[test]
    fn throws_land_near_the_aim() {
        let level = Level::generate(5, level::ArenaSize::Small);
        let world = level.world();
        let from = level.base_centre(Team::Red) + Vec3::Y * 1.5;
        let mut found = 0;
        for k in 0..20 {
            let aim = from + Vec3::new(k as f32 - 10.0, -1.5, -8.0 - k as f32 * 0.5);
            if let Some((v, q)) = solve_throw(&world, from, aim, 16.0) {
                found += 1;
                assert!(v.length() <= 16.0);
                assert!(Vec2::new(q.x - aim.x, q.z - aim.z).length() < THROW_LAND);
            }
        }
        assert!(found > 3, "{found}");
    }

    #[test]
    fn bots_go_for_the_megatarget() {
        let mut totals = [0usize; 6];
        for seed in [3u64, 8] {
            let mut sim = Sim::new(
                seed,
                Rules {
                    mode: level::GameMode::Megatarget,
                    match_time: 300.0,
                    ..Rules::default()
                },
            );
            let mut bots = Bots::default();
            bots.fill(&mut sim, 8, 0.6);
            let mut n = [0usize; 6];
            for _ in 0..(300.0 / TICK) as usize {
                bots.think(&mut sim);
                sim.step();
                for e in sim.drain_events() {
                    match e {
                        SimEvent::SecondaryHit { charged, .. } => {
                            n[0] += 1;
                            n[1] += charged as usize;
                        }
                        SimEvent::MegaHit { .. } => n[2] += 1,
                        SimEvent::Shocked { .. } => n[3] += 1,
                        SimEvent::TurretOff { .. } => n[4] += 1,
                        SimEvent::TaggedOut { by: HAZARD, .. } => n[5] += 1,
                        _ => {}
                    }
                }
            }
            println!(
                "seed {seed}: secondary hits {}, powered {}, megatarget {}, shocked {}, \
                 turrets off {}, zapped {}, scores {:?}",
                n[0], n[1], n[2], n[3], n[4], n[5], sim.scores
            );
            for k in 0..6 {
                totals[k] += n[k];
            }
        }
        assert!(totals[0] >= 6, "secondaries hit: {totals:?}");
        assert!(totals[2] >= 1, "the megatarget hit: {totals:?}");
        assert_eq!(totals[3], 0, "bots keep out of forcefields: {totals:?}");
    }
}

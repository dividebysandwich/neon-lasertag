//! Turret boxes (`Level::boxes`): a turret folded up in a box, working for nobody until a
//! player solves the sliding puzzle on the box's side (`crate::puzzle`). Then the lid opens, the
//! turret rises out and shoots the solver team's enemies within reach, two shots a second (after
//! `TURRET_WARMUP` on finding one, as the Megatarget turrets). Shot once it has been out for
//! `Rules::box_guard` seconds, it is off for `Rules::box_off`. Solving the puzzle again, the other
//! team takes it over: the turret folds away, the lid closes, and it all comes out again in the
//! new team's colour. Every solve sets a new puzzle (from a new seed, in the snapshot with the
//! pieces' places, so team mates can carry on with one another's work).
//!
//! Clicks on the pieces come with the players' commands (`PlayerCmd::pokes`), from up to
//! `POKE_REACH` away from the box; none count while the turret is coming out or folding away.

use glam::{Quat, Vec3};
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::{Bolt, BoltExtra, HAZARD, Sim, TICK, TURRET_WARMUP};
use crate::level::BOX_HEAD;
use crate::puzzle::{Puzzle, Slide};
use crate::types::{PuzzleSlide, SimEvent, Team};

/// Seconds the lid takes to open (or close), and the turret to rise out (or fold away).
pub const LID_TIME: f32 = 0.35;
pub const RISE_TIME: f32 = 0.6;
/// How far from a box's sides (m, along the floor) a player's feet may be to click its pieces.
pub const POKE_REACH: f32 = 1.8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BoxPhase {
    /// Shut, the turret inside, working for nobody.
    #[default]
    Closed,
    /// The lid opening, then the turret rising out.
    Opening,
    /// Out and working.
    Up,
    /// The turret folding away, then the lid closing (it comes out again for the other team).
    Closing,
}

/// How far the lid is open and the turret out (0..1 each), `t` seconds into `phase`.
pub fn box_motion(phase: BoxPhase, t: f32) -> (f32, f32) {
    let k = |t: f32, len: f32| (t / len).clamp(0.0, 1.0);
    match phase {
        BoxPhase::Closed => (0.0, 0.0),
        BoxPhase::Up => (1.0, 1.0),
        BoxPhase::Opening => (k(t, LID_TIME), k(t - LID_TIME, RISE_TIME)),
        BoxPhase::Closing => (1.0 - k(t - RISE_TIME, LID_TIME), 1.0 - k(t, RISE_TIME)),
    }
}

#[derive(Clone, Debug)]
pub struct BoxState {
    /// The team the turret works for (its colour), and the one taking it over (while it
    /// folds away).
    pub owner: Option<Team>,
    pub next: Option<Team>,
    pub phase: BoxPhase,
    /// Seconds into `phase`.
    pub t: f32,
    /// The puzzle (from its seed), and how far out each piece is.
    pub seed: u32,
    pub puzzle: Puzzle,
    pub pieces: Vec<u8>,
    /// Where the gun points.
    pub aim: Vec3,
    /// Seconds left switched off (shot), and of its protection after coming out.
    pub off: f32,
    pub guard: f32,
    fire_t: f32,
    engaged: bool,
}

impl BoxState {
    fn new(seed: u32) -> BoxState {
        let puzzle = Puzzle::generate(seed);
        BoxState {
            owner: None,
            next: None,
            phase: BoxPhase::Closed,
            t: 0.0,
            seed,
            pieces: puzzle.start(),
            puzzle,
            aim: Vec3::NEG_Z,
            off: 0.0,
            guard: 0.0,
            fire_t: TURRET_WARMUP,
            engaged: false,
        }
    }

    pub fn snap(&self) -> BoxSnap {
        BoxSnap {
            owner: self.owner,
            phase: self.phase,
            t: self.t,
            seed: self.seed,
            pieces: self.pieces.clone(),
            aim: self.aim,
            off: self.off,
            guard: self.guard,
        }
    }

    /// How far the turret is out (0..1).
    pub fn rise(&self) -> f32 {
        box_motion(self.phase, self.t).1
    }
}

/// A turret box as the clients see it (`BoxState`, the puzzle by its seed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoxSnap {
    pub owner: Option<Team>,
    pub phase: BoxPhase,
    pub t: f32,
    pub seed: u32,
    pub pieces: Vec<u8>,
    pub aim: Vec3,
    pub off: f32,
    pub guard: f32,
}

impl Sim {
    /// A box per `Level::boxes`, each with a puzzle of its own.
    pub(super) fn init_boxes(&mut self) {
        self.boxes = (0..self.level.boxes.len())
            .map(|_| BoxState::new(self.box_rng.random()))
            .collect();
    }

    /// Once a tick: the players' clicks, then the boxes' lids and turrets.
    pub(super) fn step_boxes(&mut self) {
        if self.boxes.is_empty() {
            return;
        }
        for pi in 0..self.players.len() {
            let p = &mut self.players[pi];
            let n = p.cmd.pokes;
            let before = p.pokes.replace(n);
            if before.is_none_or(|b| b == n) || !p.alive() {
                continue;
            }
            let (k, piece) = p.cmd.poke;
            self.poke(pi, k as usize, piece as usize);
        }
        for k in 0..self.boxes.len() {
            self.step_box(k);
        }
    }

    /// Player `pi` clicked a piece of box `k`'s puzzle.
    fn poke(&mut self, pi: usize, k: usize, piece: usize) {
        let (Some(def), Some(st)) = (self.level.boxes.get(k).copied(), self.boxes.get_mut(k))
        else {
            return;
        };
        if !matches!(st.phase, BoxPhase::Closed | BoxPhase::Up) || piece >= st.pieces.len() {
            return;
        }
        // Close enough: the feet near the box, on its floor.
        let p = &self.players[pi];
        let (min, max) = def.bounds();
        let gap = glam::Vec2::new(
            (min.x - p.feet.x).max(p.feet.x - max.x).max(0.0),
            (min.z - p.feet.z).max(p.feet.z - max.z).max(0.0),
        );
        if gap.length() > POKE_REACH || (p.feet.y - def.pos.y).abs() > 0.8 {
            return;
        }
        let by = p.id;
        let slide = match st.puzzle.poke(&mut st.pieces, piece) {
            Slide::In => PuzzleSlide::In,
            Slide::Stuck => PuzzleSlide::Stuck,
            Slide::Out => PuzzleSlide::Out,
            Slide::Blocked => PuzzleSlide::Blocked,
        };
        let q = &st.puzzle.pieces[piece];
        let n = q.cells.len().max(1) as f32;
        let (c, r) = q
            .cells_at(st.pieces[piece])
            .fold((0.0, 0.0), |(c, r), (x, y)| (c + x as f32, r + y as f32));
        let pos = def.cell_pos(0, c / n, r / n);
        self.events.push(SimEvent::PieceSlid {
            turret_box: k as u8,
            piece: piece as u8,
            by,
            slide,
            pos,
        });
        if st.puzzle.solved(&st.pieces) {
            self.solve(k, pi);
        }
    }

    /// Box `k`'s puzzle solved by player `pi`: a new one, and the turret for `pi`'s team.
    fn solve(&mut self, k: usize, pi: usize) {
        let (team, by) = (self.players[pi].team, self.players[pi].id);
        let seed = self.box_rng.random();
        let pos = self.level.boxes[k].pos;
        let st = &mut self.boxes[k];
        st.seed = seed;
        st.puzzle = Puzzle::generate(seed);
        st.pieces = st.puzzle.start();
        self.events.push(SimEvent::PuzzleSolved {
            turret_box: k as u8,
            by,
            team,
            pos: pos + Vec3::Y * crate::level::BOX_H,
        });
        if st.owner == Some(team) {
            return;
        }
        match st.phase {
            BoxPhase::Closed => {
                st.owner = Some(team);
                st.phase = BoxPhase::Opening;
                st.t = 0.0;
                self.events.push(SimEvent::BoxLid {
                    turret_box: k as u8,
                    open: true,
                    pos: pos + Vec3::Y * crate::level::BOX_H,
                });
            }
            BoxPhase::Up => {
                st.next = Some(team);
                st.phase = BoxPhase::Closing;
                st.t = 0.0;
                st.off = 0.0;
                st.engaged = false;
                self.events.push(SimEvent::BoxTurret {
                    turret_box: k as u8,
                    up: false,
                    pos: self.level.boxes[k].head(1.0),
                });
            }
            BoxPhase::Opening | BoxPhase::Closing => {}
        }
    }

    fn step_box(&mut self, k: usize) {
        let def = self.level.boxes[k];
        let top = def.pos + Vec3::Y * crate::level::BOX_H;
        let st = &mut self.boxes[k];
        let was = st.t;
        st.t += TICK;
        let crossed = |at: f32| was < at && st.t >= at;
        match st.phase {
            BoxPhase::Closed => {}
            BoxPhase::Opening => {
                if crossed(LID_TIME) {
                    self.events.push(SimEvent::BoxTurret {
                        turret_box: k as u8,
                        up: true,
                        pos: def.head(0.0),
                    });
                }
                if st.t >= LID_TIME + RISE_TIME {
                    st.phase = BoxPhase::Up;
                    st.t = 0.0;
                    st.guard = self.rules.box_guard;
                    st.off = 0.0;
                    st.fire_t = TURRET_WARMUP;
                    st.engaged = false;
                }
            }
            BoxPhase::Closing => {
                if crossed(RISE_TIME) {
                    self.events.push(SimEvent::BoxLid {
                        turret_box: k as u8,
                        open: false,
                        pos: top,
                    });
                }
                if st.t >= RISE_TIME + LID_TIME {
                    // Shut: the new team's colour, and out again.
                    st.owner = st.next.take();
                    st.t = 0.0;
                    st.phase = if st.owner.is_some() {
                        self.events.push(SimEvent::BoxLid {
                            turret_box: k as u8,
                            open: true,
                            pos: top,
                        });
                        BoxPhase::Opening
                    } else {
                        BoxPhase::Closed
                    };
                }
            }
            BoxPhase::Up => self.step_box_turret(k),
        }
    }

    /// A turret that's out: off for a while after being shot, else looking for the nearest
    /// enemy of its team it sees in reach, turning to them and firing.
    fn step_box_turret(&mut self, k: usize) {
        let def = self.level.boxes[k];
        let rules = &self.rules;
        let st = &mut self.boxes[k];
        let Some(team) = st.owner else { return };
        st.guard = (st.guard - TICK).max(0.0);
        let head = def.head(1.0);
        if st.off > 0.0 {
            st.off -= TICK;
            if st.off <= 0.0 {
                st.off = 0.0;
                st.fire_t = TURRET_WARMUP;
                self.events.push(SimEvent::BoxOn {
                    turret_box: k as u8,
                });
            }
            // Drooping while off.
            let flat = Vec3::new(st.aim.x, 0.0, st.aim.z).normalize_or(Vec3::NEG_Z);
            let rest = (flat - Vec3::Y * 0.9).normalize();
            st.aim = st.aim.lerp(rest, 0.05).normalize_or(rest);
            st.engaged = false;
            return;
        }
        st.fire_t -= TICK;
        let muzzle = head + st.aim * 0.3;
        let target = self
            .players
            .iter()
            .filter(|p| {
                p.alive()
                    && p.team != team
                    && p.cooldown <= 0.0
                    && p.shield <= 0.0
                    && p.seen_from(p.chest().distance(muzzle), self.cloak_near(p))
            })
            .map(|p| p.chest())
            .filter(|c| c.distance(muzzle) < rules.turret_range && self.world.clear(muzzle, *c))
            .min_by(|a, b| a.distance(muzzle).total_cmp(&b.distance(muzzle)));
        let want = match target {
            Some(c) => (c - head).normalize_or(Vec3::NEG_Z),
            // Nobody: turning slowly round, looking a little down.
            None => {
                let t = self.tick as f32 * TICK;
                Quat::from_rotation_y(t * 0.5 + k as f32 * 1.3)
                    * Vec3::new(0.0, -0.2, -1.0).normalize()
            }
        };
        let st = &mut self.boxes[k];
        if target.is_some() && !st.engaged {
            st.fire_t = st.fire_t.max(TURRET_WARMUP);
            self.events.push(SimEvent::BoxEngaged {
                turret_box: k as u8,
            });
        }
        st.engaged = target.is_some();
        let angle = st.aim.angle_between(want);
        if angle > 1e-4 {
            let step = (rules.turret_turn * TICK).min(angle);
            let axis = st.aim.cross(want).normalize_or(Vec3::Y);
            st.aim = (Quat::from_axis_angle(axis, step) * st.aim).normalize_or(want);
        }
        if target.is_none() || st.fire_t > 0.0 || st.aim.angle_between(want) >= 0.06 {
            return;
        }
        st.fire_t = rules.box_every;
        let aim = st.aim;
        // Off by a random angle in a random direction.
        let side = aim.any_orthonormal_vector();
        let err: f32 = ((0..3).map(|_| self.rng.random::<f32>()).sum::<f32>() - 1.5)
            * 2.0
            * rules.turret_spread;
        let roll = self.rng.random_range(0.0..std::f32::consts::TAU);
        let dir = Quat::from_axis_angle(Quat::from_axis_angle(aim, roll) * side, err) * aim;
        let from = head + dir * 0.5;
        let (speed, life) = (rules.bolt_speed, rules.bolt_life);
        let id = self.id();
        self.bolts.push(Bolt {
            id,
            owner: HAZARD,
            owner_team: team,
            team: Some(team),
            pos: from,
            vel: dir * speed,
            life,
            extra: BoltExtra::default(),
        });
        self.events.push(SimEvent::BoxFired {
            turret_box: k as u8,
            pos: from,
            dir,
        });
    }

    /// The first turret head out of its box (by more than half) on the segment `a`..`b`: its
    /// box and where the segment meets it.
    pub(super) fn box_head_hit(&self, a: Vec3, b: Vec3) -> Option<(usize, Vec3)> {
        let d = b - a;
        let dd = d.length_squared();
        if dd < 1e-12 {
            return None;
        }
        let mut best: Option<(f32, usize)> = None;
        for (k, st) in self.boxes.iter().enumerate() {
            let rise = st.rise();
            if rise < 0.5 {
                continue;
            }
            let c = self.level.boxes[k].head(rise);
            let f = a - c;
            let bh = f.dot(d);
            let cc = f.length_squared() - BOX_HEAD * BOX_HEAD;
            let disc = bh * bh - dd * cc;
            if disc < 0.0 {
                continue;
            }
            // (From outside it, ahead.)
            let t = (-bh - disc.sqrt()) / dd;
            if cc > 0.0 && (0.0..=1.0).contains(&t) && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, k));
            }
        }
        best.map(|(t, k)| (k, a + d * t))
    }

    /// Bolt `b` hit box `k`'s turret at `q`: switched off, unless it's its own team's, another
    /// turret's, or the turret has only just come out.
    pub(super) fn box_shot(&mut self, k: usize, b: &Bolt, q: Vec3) {
        let st = &mut self.boxes[k];
        let counts = st.phase == BoxPhase::Up
            && st.guard <= 0.0
            && b.owner != HAZARD
            && st.owner != Some(b.owner_team);
        if counts {
            st.off = self.rules.box_off;
            st.engaged = false;
            self.events.push(SimEvent::BoxOff {
                turret_box: k as u8,
                by: b.owner,
                pos: q,
            });
        } else {
            let c = self.level.boxes[k].head(st.rise());
            self.events.push(SimEvent::BoltImpact {
                pos: q,
                normal: (q - c).normalize_or(Vec3::Y),
                team: b.team,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{ArenaSize, BOX_HALF, GameMode, Ground, Level, mirror_point};
    use crate::rules::Rules;
    use crate::types::{PlayerCmd, PlayerId};

    #[test]
    fn arenas_get_mirrored_turret_boxes_open_in_front() {
        let mut with = 0;
        for seed in 0..24u64 {
            for (size, mode) in [
                (ArenaSize::Small, GameMode::Classic),
                (ArenaSize::Medium, GameMode::Megatarget),
                (ArenaSize::Large, GameMode::Classic),
            ] {
                let level = Level::generate_for(seed, size, mode);
                let dm = level.dims();
                assert!(level.boxes.len() % 2 == 0, "seed {seed}: in pairs");
                with += (!level.boxes.is_empty()) as usize;
                for b in &level.boxes {
                    assert!(
                        level
                            .boxes
                            .iter()
                            .any(|o| o.pos.distance(mirror_point(b.pos)) < 0.01),
                        "seed {seed}: mirrored"
                    );
                    // Open floor in front of both puzzles.
                    for dz in [-1.3, 1.3] {
                        let (ix, iz) = dm.cell_at(b.pos + Vec3::Z * dz).unwrap();
                        assert_eq!(level.grid.ground[dm.idx(ix, iz)], Ground::Open);
                    }
                }
            }
        }
        assert!(with >= 60, "{with} of 72 arenas have turret boxes");
    }

    /// A sim on an arena with turret boxes, its first box, and a player of each team.
    fn box_sim() -> (Sim, usize, PlayerId, PlayerId) {
        let rules = Rules {
            match_time: 1000.0,
            spawn_protect: 0.0,
            // (Shot on and on, for counting the turrets' shots.)
            hit_cooldown: 0.0,
            hits_to_tag: 200,
            pickup_pairs: 0,
            powerup_pairs: 0,
            event_first: (5000.0, 5000.0),
            ..Rules::default()
        };
        let seed = (0..).find(|&s| !Level::generate(s, ArenaSize::Small).boxes.is_empty());
        let mut sim = Sim::new(seed.unwrap(), rules);
        let red = sim.add_player("red", Some(Team::Red), false).unwrap();
        let blue = sim.add_player("blue", Some(Team::Blue), false).unwrap();
        // (Both well away until wanted.)
        let far = |t: Team| sim.level.base_centre(t);
        let (r, b) = (far(Team::Red), far(Team::Blue));
        sim.teleport(red, r, 0.0);
        sim.teleport(blue, b, 0.0);
        sim.step();
        (sim, 0, red, blue)
    }

    /// Player `id` in front of box `k`'s +z side.
    fn to_box(sim: &mut Sim, k: usize, id: PlayerId) {
        let at = sim.level.boxes[k].pos + Vec3::Z * (BOX_HALF + 0.5);
        sim.teleport(id, at, 0.0);
        sim.step();
    }

    fn click(sim: &mut Sim, id: PlayerId, k: usize, piece: usize) {
        let mut cmd = sim.player(id).unwrap().cmd;
        cmd.pokes = cmd.pokes.wrapping_add(1);
        cmd.poke = (k as u8, piece as u8);
        sim.set_cmd(id, cmd);
        sim.step();
    }

    fn solve(sim: &mut Sim, id: PlayerId, k: usize) {
        let seed = sim.boxes[k].seed;
        for _ in 0..80 {
            if sim.boxes[k].seed != seed {
                return;
            }
            let i = sim.boxes[k].puzzle.hint(&sim.boxes[k].pieces).unwrap();
            click(sim, id, k, i);
        }
        panic!("not solved");
    }

    fn run(sim: &mut Sim, secs: f32) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        for _ in 0..(secs / TICK) as usize {
            sim.step();
            ev.extend(sim.drain_events());
        }
        ev
    }

    #[test]
    fn clicks_count_only_near_the_box() {
        let (mut sim, k, red, _) = box_sim();
        let before = sim.boxes[k].pieces.clone();
        click(&mut sim, red, k, 0);
        assert_eq!(sim.boxes[k].pieces, before, "from the base: nothing");
        to_box(&mut sim, k, red);
        click(&mut sim, red, k, 0);
        assert_ne!(sim.boxes[k].pieces, before);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::PieceSlid { by, .. } if *by == red))
        );
    }

    #[test]
    fn a_solved_box_opens_and_its_turret_shoots_the_other_team() {
        let (mut sim, k, red, blue) = box_sim();
        to_box(&mut sim, k, red);
        click(&mut sim, red, k, 0);
        let seed = sim.boxes[k].seed;
        solve(&mut sim, red, k);
        assert_ne!(sim.boxes[k].seed, seed, "a new puzzle");
        assert_eq!(sim.boxes[k].owner, Some(Team::Red));
        assert_eq!(sim.boxes[k].phase, BoxPhase::Opening);
        let ev = run(&mut sim, 1.2);
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::BoxLid { open: true, .. }))
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::BoxTurret { up: true, .. }))
        );
        assert_eq!(sim.boxes[k].phase, BoxPhase::Up);
        // Red right beside it: not shot. Blue in the open on the other side: shot at, two a
        // second.
        assert!(!ev.iter().any(|e| matches!(e, SimEvent::BoxFired { .. })));
        let spot = sim.level.boxes[k].pos - Vec3::Z * (BOX_HALF + 0.6);
        sim.teleport(blue, spot, 0.0);
        // (Turning round to them and the start-up wait first.)
        let first = run(&mut sim, 2.0);
        let ev = run(&mut sim, 2.0);
        let shots = ev
            .iter()
            .filter(|e| matches!(e, SimEvent::BoxFired { .. }))
            .count();
        assert!((3..=5).contains(&shots), "{shots} shots");
        let ev = [first, ev].concat();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::Hit { player, .. } if *player == blue))
        );
    }

    #[test]
    fn shot_turrets_switch_off_and_the_other_team_takes_them_over() {
        let (mut sim, k, red, blue) = box_sim();
        to_box(&mut sim, k, red);
        click(&mut sim, red, k, 0);
        solve(&mut sim, red, k);
        run(&mut sim, 1.2);
        let head = sim.level.boxes[k].head(1.0);
        // Shot straight after coming out: nothing.
        let st = &mut sim.boxes[k];
        let bolt = |team: Team, owner: PlayerId| Bolt {
            id: 0,
            owner,
            owner_team: team,
            team: Some(team),
            pos: head + Vec3::new(3.0, 0.0, 0.0),
            vel: Vec3::new(-60.0, 0.0, 0.0),
            life: 1.0,
            extra: BoltExtra::default(),
        };
        assert!(st.guard > 0.0);
        let b = bolt(Team::Blue, blue);
        let hit = sim.box_head_hit(b.pos, b.pos + b.vel * 0.1).unwrap();
        assert_eq!(hit.0, k);
        sim.box_shot(k, &b, hit.1);
        assert_eq!(sim.boxes[k].off, 0.0, "protected for a moment");
        run(&mut sim, 3.0);
        // Its own team's bolt: nothing; the other's: off for ten seconds.
        sim.box_shot(k, &bolt(Team::Red, red), head);
        assert_eq!(sim.boxes[k].off, 0.0);
        sim.box_shot(k, &b, head);
        assert!(sim.boxes[k].off > 9.9);
        // Blue solves it: it folds away, closes, changes sides and comes out again.
        to_box(&mut sim, k, blue);
        sim.teleport(red, sim.level.base_centre(Team::Red), 0.0);
        click(&mut sim, blue, k, 0);
        solve(&mut sim, blue, k);
        assert_eq!(sim.boxes[k].phase, BoxPhase::Closing);
        assert_eq!(sim.boxes[k].owner, Some(Team::Red), "red until shut");
        let ev = run(&mut sim, 1.0);
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::BoxTurret { up: false, .. }))
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::BoxLid { open: false, .. }))
        );
        assert_eq!(sim.boxes[k].owner, Some(Team::Blue));
        run(&mut sim, 1.0);
        assert_eq!(sim.boxes[k].phase, BoxPhase::Up);
        assert_eq!(sim.boxes[k].off, 0.0);
    }
}

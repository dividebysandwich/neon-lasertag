//! Megatarget mode: the secondary targets' shooting puzzles (`crate::gadgets`), each side's
//! pieces' state.

use glam::Vec3;
use rand::Rng;
use serde::{Deserialize, Serialize};

use super::{Bolt, HAZARD, Sim, TICK};
use crate::gadgets::{HOLD, LAMPS, LOCK, RELAYS, STEPS, beam_reach, relay_start, sequence};
use crate::rules::SecondaryKind;
use crate::types::SimEvent;

#[derive(Clone, Debug, Default)]
pub struct GadgetState {
    pub seed: u32,
    /// Each piece's state (relays: quarter turns off), or (lamps) the steps of the sequence
    /// shot so far.
    pub state: Vec<u8>,
    /// Seconds left of the solved beam (no shots count; then a new puzzle).
    pub hold: f32,
    /// Each piece's seconds left of taking no shots.
    lock: Vec<f32>,
}

/// A side's shooting puzzle as the clients see it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GadgetSnap {
    pub seed: u32,
    pub state: Vec<u8>,
    pub hold: f32,
}

impl GadgetState {
    fn new(kind: SecondaryKind, seed: u32) -> GadgetState {
        let (state, pieces) = match kind {
            SecondaryKind::Relay => (relay_start(seed), RELAYS),
            SecondaryKind::Sequence => (vec![0], LAMPS),
            _ => (Vec::new(), 0),
        };
        GadgetState {
            seed,
            lock: vec![0.0; pieces],
            state,
            hold: 0.0,
        }
    }

    pub fn snap(&self) -> GadgetSnap {
        GadgetSnap {
            seed: self.seed,
            state: self.state.clone(),
            hold: self.hold,
        }
    }
}

impl Sim {
    pub(super) fn init_gadgets(&mut self) {
        if self.level.mega.is_none() {
            return;
        }
        self.gadgets = (0..2)
            .map(|side| GadgetState::new(self.secondary[side], self.console_rng.random()))
            .collect();
    }

    /// Once a tick: the pieces' locks, and a solved beam's hold (then a new puzzle).
    pub(super) fn step_gadgets(&mut self) {
        for side in 0..self.gadgets.len() {
            let g = &mut self.gadgets[side];
            for l in &mut g.lock {
                *l = (*l - TICK).max(0.0);
            }
            if g.hold > 0.0 {
                g.hold -= TICK;
                if g.hold <= 0.0 {
                    let seed = self.console_rng.random();
                    self.gadgets[side] = GadgetState::new(self.secondary[side], seed);
                }
            }
        }
    }

    /// Piece `i` of side `side`'s puzzle in the way of bolt `b` (at `q`).
    pub(super) fn gadget_shot(&mut self, side: usize, i: usize, b: &Bolt, q: Vec3) {
        let impact = SimEvent::BoltImpact {
            pos: q,
            normal: -(b.vel.normalize_or_zero()),
            team: b.team,
        };
        let Some(g) = self.gadgets.get_mut(side) else {
            return;
        };
        if b.owner == HAZARD || g.hold > 0.0 || g.lock.get(i).is_none_or(|l| *l > 0.0) {
            self.events.push(impact);
            return;
        }
        if self.secondary[side] == SecondaryKind::Sequence {
            let seq = sequence(g.seed);
            let step = g.state[0] as usize;
            let ok = seq.get(step) == Some(&(i as u8));
            g.lock[i] = LOCK;
            g.state[0] = if ok { step as u8 + 1 } else { 0 };
            self.events.push(SimEvent::LampShot {
                side: side as u8,
                lamp: i as u8,
                by: b.owner,
                pos: q,
                ok,
            });
            if ok && step + 1 == STEPS {
                g.hold = HOLD;
                self.events.push(SimEvent::SequenceDone {
                    side: side as u8,
                    by: b.owner,
                    pos: q,
                });
                self.gadget_solved(side, b.owner, q);
            }
        } else if self.secondary[side] == SecondaryKind::Relay {
            g.state[i] = (g.state[i] + 1) % 4;
            g.lock[i] = LOCK;
            self.events.push(SimEvent::RelayTurned {
                side: side as u8,
                relay: i as u8,
                by: b.owner,
                pos: q,
            });
            if beam_reach(&g.state) == RELAYS {
                g.hold = HOLD;
                let at = self.level.mega.as_ref().unwrap().relay_route[side][RELAYS + 1];
                self.events.push(SimEvent::RelayAligned {
                    side: side as u8,
                    by: b.owner,
                    pos: at,
                });
                self.gadget_solved(side, b.owner, at);
            }
        }
    }

    /// Side `side`'s shooting puzzle solved by `by`'s shot: they and their team mates in the
    /// pocket charge there.
    fn gadget_solved(&mut self, side: usize, by: crate::types::PlayerId, at: Vec3) {
        let Some(pi) = self.players.iter().position(|p| p.id == by) else {
            return;
        };
        let team = self.players[pi].team;
        let Some(m) = self.level.mega.clone() else {
            return;
        };
        let mut who = vec![pi];
        who.extend((0..self.players.len()).filter(|&j| {
            let p = &self.players[j];
            j != pi && p.alive() && p.team == team && m.in_pocket(side, p.feet)
        }));
        for j in who {
            self.charge_secondary(side, j, at);
        }
    }

    /// The first piece of a shooting puzzle on the segment `a`..`b`.
    pub(super) fn gadget_hit(&self, a: Vec3, b: Vec3) -> Option<(usize, usize, Vec3)> {
        let m = self.level.mega.as_ref()?;
        crate::gadgets::hit(m, self.secondary, a, b)
    }

    /// Whether side `side`'s piece `i` takes a shot now (for the bots).
    pub fn gadget_ready(&self, side: usize, i: usize) -> bool {
        self.gadgets
            .get(side)
            .is_some_and(|g| g.hold <= 0.0 && g.lock.get(i).is_some_and(|l| *l <= 0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::super::BoltExtra;
    use super::*;
    use crate::level::GameMode;
    use crate::rules::Rules;
    use crate::types::{PlayerId, Team};

    fn relay_sim() -> (Sim, PlayerId, PlayerId, PlayerId) {
        let rules = Rules {
            match_time: 1000.0,
            spawn_protect: 0.0,
            pickup_pairs: 0,
            powerup_pairs: 0,
            turret_range: 0.0,
            event_first: (5000.0, 5000.0),
            mode: GameMode::Megatarget,
            secondary: SecondaryKind::Relay,
            ..Rules::default()
        };
        let mut sim = Sim::new(9, rules);
        let red = sim.add_player("red", Some(Team::Red), false).unwrap();
        let mate = sim.add_player("mate", Some(Team::Red), false).unwrap();
        let blue = sim.add_player("blue", Some(Team::Blue), false).unwrap();
        let (a, b) = sim.level.mega.as_ref().unwrap().pockets[0];
        let mid = (a + b) / 2.0;
        sim.teleport(red, mid, 0.0);
        sim.teleport(mate, mid + Vec3::Z * 1.0, 0.0);
        sim.teleport(blue, mid - Vec3::Z * 1.0, 0.0);
        sim.step();
        (sim, red, mate, blue)
    }

    /// Player `id` shoots relay `i` of side 0 (a bolt from just below it).
    fn shoot(sim: &mut Sim, id: PlayerId, i: usize) {
        let team = sim.player(id).unwrap().team;
        let r = sim.level.mega.as_ref().unwrap().relay_route[0][i + 1];
        let b = Bolt {
            id: 0,
            owner: id,
            owner_team: team,
            team: Some(team),
            pos: r - Vec3::Y * 1.0,
            vel: Vec3::Y * 40.0,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        let (side, j, q) = sim.gadget_hit(b.pos, b.pos + b.vel * 0.1).expect("hits it");
        assert_eq!((side, j), (0, i));
        sim.gadget_shot(side, j, &b, q);
        for _ in 0..(0.4 / TICK) as usize {
            sim.step();
        }
    }

    #[test]
    fn turning_the_relays_right_charges_the_shooter_and_team_mates_in_the_pocket() {
        let (mut sim, red, mate, blue) = relay_sim();
        assert_eq!(sim.secondary, [SecondaryKind::Relay; 2]);
        // Blue puts one out of line first: it turns for anyone.
        let before = sim.gadgets[0].state.clone();
        shoot(&mut sim, blue, 0);
        assert_eq!(sim.gadgets[0].state[0], (before[0] + 1) % 4);
        sim.drain_events();
        for i in 0..RELAYS {
            while sim.gadgets[0].state[i] != 0 {
                shoot(&mut sim, red, i);
            }
        }
        let ev = sim.drain_events();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::RelayAligned { side: 0, .. }))
        );
        let hit = |id| {
            ev.iter()
                .any(|e| matches!(e, SimEvent::SecondaryHit { by, side: 0, .. } if *by == id))
        };
        assert!(hit(red) && hit(mate) && !hit(blue));
        // Held a moment, then scrambled anew.
        assert!(sim.gadgets[0].hold > 0.0);
        for _ in 0..((HOLD + 0.1) / TICK) as usize {
            sim.step();
        }
        assert!(beam_reach(&sim.gadgets[0].state) < RELAYS);
    }

    #[test]
    fn shooting_the_lamps_in_order_charges_and_a_wrong_one_starts_over() {
        let rules = Rules {
            match_time: 1000.0,
            spawn_protect: 0.0,
            pickup_pairs: 0,
            powerup_pairs: 0,
            turret_range: 0.0,
            event_first: (5000.0, 5000.0),
            mode: GameMode::Megatarget,
            secondary: SecondaryKind::Sequence,
            ..Rules::default()
        };
        let mut sim = Sim::new(9, rules);
        let red = sim.add_player("red", Some(Team::Red), false).unwrap();
        let blue = sim.add_player("blue", Some(Team::Blue), false).unwrap();
        let (a, b) = sim.level.mega.as_ref().unwrap().pockets[0];
        sim.teleport(red, (a + b) / 2.0, 0.0);
        sim.step();
        let lamp = |sim: &mut Sim, id: PlayerId, i: usize| {
            let team = sim.player(id).unwrap().team;
            let (p, n) = sim.level.mega.as_ref().unwrap().lamps[0][i];
            let bolt = Bolt {
                id: 0,
                owner: id,
                owner_team: team,
                team: Some(team),
                pos: p + n * 1.5,
                vel: -n * 40.0,
                life: 1.0,
                extra: BoltExtra::default(),
            };
            let (side, j, q) = sim.gadget_hit(bolt.pos, p).expect("hits it");
            assert_eq!((side, j), (0, i));
            sim.gadget_shot(side, j, &bolt, q);
            for _ in 0..(0.4 / TICK) as usize {
                sim.step();
            }
        };
        let seq = sequence(sim.gadgets[0].seed);
        lamp(&mut sim, red, seq[0] as usize);
        lamp(&mut sim, red, seq[1] as usize);
        assert_eq!(sim.gadgets[0].state[0], 2);
        // Blue shoots one out of turn: over again.
        let wrong = (0..LAMPS).find(|&l| l as u8 != seq[2]).unwrap();
        lamp(&mut sim, blue, wrong);
        assert_eq!(sim.gadgets[0].state[0], 0);
        sim.drain_events();
        for &l in &seq {
            lamp(&mut sim, red, l as usize);
        }
        let ev = sim.drain_events();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::SequenceDone { side: 0, .. }))
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::SecondaryHit { by, side: 0, .. } if *by == red))
        );
    }
}

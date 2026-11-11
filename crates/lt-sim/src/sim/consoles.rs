//! Consoles (`crate::console`): a click puzzle each (`crate::minigame`), solved like a turret
//! box's (clicks in `PlayerCmd::pokes`, from within `POKE_REACH`; team mates carry on with one
//! another's work). Solving one sets a new puzzle (of a kind the new seed picks) and does what
//! its kind does for the solver's team; most then rest for a while (`cooldown`: no clicks).

use rand::Rng;
use serde::{Deserialize, Serialize};

use super::Sim;
use glam::Vec3;

use crate::console::{ConsoleDef, ConsoleKind, MAST_RANGE, MAST_TIME, console_defs};
use crate::minigame::Minigame;
use crate::types::{PickupKind, PlayerId, SimEvent, Team};

/// What a supply vault hands out: one of the special weapons (at random, one the solver isn't
/// holding).
pub const VAULT_KINDS: [PickupKind; 5] = [
    PickupKind::Shotgun,
    PickupKind::Charge,
    PickupKind::Ricochet,
    PickupKind::Lance,
    PickupKind::Dual,
];

#[derive(Clone, Debug)]
pub struct ConsoleState {
    pub seed: u32,
    pub game: Minigame,
    /// Each element's state (`Minigame::start`, `Minigame::poke`).
    pub state: Vec<u8>,
    /// The team it works for (those of its kinds that work for one).
    pub owner: Option<Team>,
    /// Seconds left of what it does, and of its rest (no clicks until 0).
    pub active: f32,
    pub cooldown: f32,
    /// A sensor mast's seconds to its next pulse.
    next_pulse: f32,
}

impl ConsoleState {
    fn new(seed: u32) -> ConsoleState {
        let game = Minigame::generate(seed);
        ConsoleState {
            seed,
            state: game.start(),
            game,
            owner: None,
            active: 0.0,
            cooldown: 0.0,
            next_pulse: 0.0,
        }
    }

    pub fn snap(&self) -> ConsoleSnap {
        ConsoleSnap {
            seed: self.seed,
            state: self.state.clone(),
            owner: self.owner,
            active: self.active,
            cooldown: self.cooldown,
        }
    }

    /// Whether its puzzle can be clicked now.
    pub fn open(&self) -> bool {
        self.cooldown <= 0.0
    }
}

/// A console as the clients see it (`ConsoleState`, the puzzle by its seed).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleSnap {
    pub seed: u32,
    pub state: Vec<u8>,
    pub owner: Option<Team>,
    pub active: f32,
    pub cooldown: f32,
}

impl ConsoleSnap {
    pub fn open(&self) -> bool {
        self.cooldown <= 0.0
    }
}

impl Sim {
    /// The match's consoles (`console_defs`), each with a puzzle of its own.
    pub(super) fn init_consoles(&mut self) {
        self.console_defs = std::sync::Arc::new(console_defs(&self.level, &self.rules));
        self.consoles = (0..self.console_defs.len())
            .map(|_| ConsoleState::new(0))
            .collect();
        for k in 0..self.consoles.len() {
            let seed = self.console_seed(k);
            self.consoles[k] = ConsoleState::new(seed);
        }
    }

    /// A new seed for console `k`'s puzzle (one of its kind, if it always has one kind).
    fn console_seed(&mut self, k: usize) -> u32 {
        let want = self.console_defs[k].game;
        loop {
            let seed: u32 = self.console_rng.random();
            if want.is_none_or(|g| crate::minigame::MinigameKind::of(seed) == g) {
                return seed;
            }
        }
    }

    /// Once a tick: the consoles' timers, and what they do while they work.
    pub(super) fn step_consoles(&mut self) {
        for k in 0..self.consoles.len() {
            let def = self.console_defs[k];
            let st = &mut self.consoles[k];
            if st.active > 0.0
                && let Some(team) = st.owner
                && def.kind == ConsoleKind::Mast
            {
                st.next_pulse -= super::TICK;
                if st.next_pulse <= 0.0 {
                    st.next_pulse = self.rules.sensor_every;
                    let top = def.pos + Vec3::Y * (crate::console::CONSOLE_H + 1.0);
                    self.pulse(u32::MAX - k as u32, team, top, MAST_RANGE, MAST_RANGE);
                }
            }
            let st = &mut self.consoles[k];
            st.active = (st.active - super::TICK).max(0.0);
            if st.cooldown > 0.0 {
                st.cooldown -= super::TICK;
                if st.cooldown <= 0.0 {
                    st.cooldown = 0.0;
                    let pos = self.console_defs[k].faces[0].centre;
                    self.events.push(SimEvent::ConsoleReady {
                        console: k as u8,
                        pos,
                    });
                }
            }
        }
    }

    /// Whether player `pi` may click console `k` now (close enough, and it's open).
    fn may_poke(&self, pi: usize, k: usize) -> bool {
        let (Some(def), Some(st)) = (self.console_defs.get(k), self.consoles.get(k)) else {
            return false;
        };
        st.open() && def.in_reach(self.players[pi].feet)
    }

    /// Player `pi` clicked element `i` of console `k`'s puzzle.
    pub(super) fn console_poke(&mut self, pi: usize, k: usize, i: usize) {
        if !self.may_poke(pi, k) {
            return;
        }
        let def = self.console_defs[k];
        let by = self.players[pi].id;
        let st = &mut self.consoles[k];
        if i >= st.game.elements() {
            return;
        }
        let slide = st.game.poke(&mut st.state, i);
        let pos = def.faces[0].centre;
        self.events.push(SimEvent::ConsolePoked {
            console: k as u8,
            element: i as u8,
            by,
            slide,
            pos,
        });
        if st.game.solved(&st.state) {
            self.console_solved(k, pi);
        }
    }

    /// Testing: player `id` clicks element `i` of console `k` (if close enough, as any click),
    /// or (`i` None) the one the puzzle's hint says next; or solves it at once (`solve`).
    pub fn dev_console_poke(&mut self, id: PlayerId, k: usize, i: Option<usize>, solve: bool) {
        let Some(pi) = self.players.iter().position(|p| p.id == id) else {
            return;
        };
        let Some(st) = self.consoles.get(k) else {
            return;
        };
        if solve {
            if self.may_poke(pi, k) {
                self.console_solved(k, pi);
            }
            return;
        }
        if let Some(j) = i.or_else(|| st.game.hint(&st.state)) {
            self.console_poke(pi, k, j);
        }
    }

    /// Testing: a new puzzle of `kind` on console `k` (and done resting).
    pub fn dev_console_game(&mut self, k: usize, kind: crate::minigame::MinigameKind) {
        let Some(st) = self.consoles.get_mut(k) else {
            return;
        };
        let seed = (st.seed.wrapping_add(1)..)
            .find(|&s| crate::minigame::MinigameKind::of(s) == kind)
            .unwrap_or(st.seed);
        *st = ConsoleState {
            owner: st.owner,
            active: st.active,
            next_pulse: st.next_pulse,
            ..ConsoleState::new(seed)
        };
    }

    /// Console `k`'s puzzle solved by player `pi`: a new one, and what the console does.
    fn console_solved(&mut self, k: usize, pi: usize) {
        let (team, by, feet) = (
            self.players[pi].team,
            self.players[pi].id,
            self.players[pi].feet,
        );
        let def = self.console_defs[k];
        let seed = self.console_seed(k);
        self.consoles[k] = ConsoleState {
            owner: self.consoles[k].owner,
            active: self.consoles[k].active,
            cooldown: self.consoles[k].cooldown,
            next_pulse: self.consoles[k].next_pulse,
            ..ConsoleState::new(seed)
        };
        self.consoles[k].cooldown = def.kind.rest();
        self.events.push(SimEvent::ConsoleSolved {
            console: k as u8,
            kind: def.kind,
            by,
            team,
            pos: def.faces[0].centre,
        });
        match def.kind {
            ConsoleKind::Vault => {
                // A special weapon straight into the solver's hands.
                let held = self.players[pi].weapon;
                let kinds: Vec<PickupKind> = VAULT_KINDS
                    .into_iter()
                    .filter(|k| k.weapon() != Some(held))
                    .collect();
                let kind = kinds[self.console_rng.random_range(0..kinds.len())];
                let rules = self.rules.clone();
                self.players[pi].take(kind, &rules);
                let face = def
                    .faces()
                    .iter()
                    .min_by(|a, b| a.centre.distance(feet).total_cmp(&b.centre.distance(feet)))
                    .copied()
                    .unwrap_or(def.faces[0]);
                self.events.push(SimEvent::PickedUp {
                    id: 0,
                    player: by,
                    pos: face.centre + face.normal * 0.3,
                    kind,
                });
            }
            ConsoleKind::Mast => {
                let st = &mut self.consoles[k];
                st.owner = Some(team);
                st.active = MAST_TIME;
                st.next_pulse = 0.0;
            }
            ConsoleKind::Beacon => self.consoles[k].owner = Some(team),
            ConsoleKind::Breaker => {
                let st = &mut self.consoles[k];
                st.owner = Some(team);
                st.active = crate::console::BREAKER_TIME;
            }
            ConsoleKind::Bridge => {
                let st = &mut self.consoles[k];
                st.owner = Some(team);
                st.active = crate::console::BRIDGE_TIME;
            }
            ConsoleKind::Secondary(side) => {
                // The solver, and their team mates in the pocket.
                let side = side as usize;
                let pos = def.faces[0].centre;
                let Some(m) = self.level.mega.clone() else {
                    return;
                };
                let mut who: Vec<usize> = vec![pi];
                who.extend((0..self.players.len()).filter(|&j| {
                    let p = &self.players[j];
                    j != pi && p.alive() && p.team == team && m.in_pocket(side, p.feet)
                }));
                for j in who {
                    self.charge_secondary(side, j, pos);
                }
            }
        }
    }

    /// Where a player of `team` respawns at one of its beacons, if it does this time: in front of
    /// one of its sides (facing away), `BEACON_SHARE` of the time, while no enemy is near.
    pub(super) fn beacon_spawn(&mut self, team: Team) -> Option<crate::level::Spawn> {
        let beacons: Vec<ConsoleDef> = self
            .console_defs
            .iter()
            .zip(&self.consoles)
            .filter(|(d, s)| d.kind == ConsoleKind::Beacon && s.owner == Some(team))
            .map(|(d, _)| *d)
            .filter(|d| {
                !self.players.iter().any(|p| {
                    p.alive()
                        && p.team != team
                        && p.feet.distance(d.pos) < crate::console::BEACON_CLEAR
                })
            })
            .collect();
        if beacons.is_empty() || self.rng.random::<f32>() >= crate::console::BEACON_SHARE {
            return None;
        }
        let d = beacons[self.rng.random_range(0..beacons.len())];
        let f = d.faces()[self.rng.random_range(0..d.nfaces)];
        let mut pos = f.centre + f.normal * 1.0;
        pos.y = d.floor;
        Some(crate::level::Spawn {
            pos,
            yaw: super::yaw_of(f.normal),
        })
    }

    /// The light bridges standing now, as colliders.
    pub(super) fn bridge_colliders(&self) -> Vec<crate::collide::Collider> {
        crate::console::bridge_colliders(&self.level, &self.console_defs, |k| {
            self.consoles[k].active > 0.0
        })
    }

    /// Whether `p` is in a quarter of the arena a blackout breaker has dark now.
    pub fn dark_at(&self, p: Vec3) -> bool {
        let q = crate::console::quarter(p);
        self.console_defs.iter().zip(&self.consoles).any(|(d, s)| {
            d.kind == ConsoleKind::Breaker && s.active > 0.0 && crate::console::quarter(d.pos) == q
        })
    }

    /// The consoles' puzzles in play (`console_defs`).
    pub fn console_defs(&self) -> &[ConsoleDef] {
        &self.console_defs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{ArenaSize, GameMode, Level};
    use crate::rules::Rules;
    use crate::sim::TICK;
    use crate::types::PlayerCmd;

    /// A sim on an arena with a console of `kind`, that console, and a player of each team.
    pub(crate) fn console_sim(
        kind: ConsoleKind,
        mode: GameMode,
    ) -> (Sim, usize, PlayerId, PlayerId) {
        let rules = Rules {
            match_time: 1000.0,
            spawn_protect: 0.0,
            pickup_pairs: 0,
            powerup_pairs: 0,
            event_first: (5000.0, 5000.0),
            mode,
            arena: ArenaSize::Large,
            ..Rules::default()
        };
        let seed = (0..)
            .find(|&s| {
                Level::generate_for(s, ArenaSize::Large, mode)
                    .consoles
                    .iter()
                    .any(|c| c.kind == kind)
            })
            .unwrap();
        let mut sim = Sim::new(seed, rules);
        let red = sim.add_player("red", Some(Team::Red), false).unwrap();
        let blue = sim.add_player("blue", Some(Team::Blue), false).unwrap();
        let far = |t: Team| sim.level.base_centre(t);
        let (r, b) = (far(Team::Red), far(Team::Blue));
        sim.teleport(red, r, 0.0);
        sim.teleport(blue, b, 0.0);
        sim.step();
        let k = sim
            .console_defs()
            .iter()
            .position(|d| d.kind == kind)
            .unwrap();
        (sim, k, red, blue)
    }

    /// Player `id` in front of console `k`'s first face.
    pub(crate) fn to_console(sim: &mut Sim, k: usize, id: PlayerId) {
        let f = sim.console_defs()[k].faces[0];
        let mut at = f.centre + f.normal * 0.6;
        at.y = sim.console_defs()[k].floor;
        sim.teleport(id, at, 0.0);
        sim.step();
    }

    pub(crate) fn click(sim: &mut Sim, id: PlayerId, k: usize, i: usize) {
        let mut cmd: PlayerCmd = sim.player(id).unwrap().cmd;
        cmd.pokes = cmd.pokes.wrapping_add(1);
        cmd.poke = (crate::console::CONSOLE_POKE + k as u8, i as u8);
        sim.set_cmd(id, cmd);
        sim.step();
    }

    /// Clicks the hints until console `k`'s puzzle is solved (a new seed).
    pub(crate) fn solve(sim: &mut Sim, id: PlayerId, k: usize) {
        let seed = sim.consoles[k].seed;
        for _ in 0..300 {
            if sim.consoles[k].seed != seed {
                return;
            }
            let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
            click(sim, id, k, i);
        }
        panic!("not solved");
    }

    pub(crate) fn run(sim: &mut Sim, secs: f32) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        for _ in 0..(secs / TICK) as usize {
            sim.step();
            ev.extend(sim.drain_events());
        }
        ev
    }

    #[test]
    fn console_clicks_count_only_near_it() {
        let (mut sim, k, red, _) = console_sim(ConsoleKind::Vault, GameMode::Classic);
        let before = sim.consoles[k].state.clone();
        let i = sim.consoles[k].game.hint(&before).unwrap();
        click(&mut sim, red, k, i);
        assert_eq!(sim.consoles[k].state, before, "from the base: nothing");
        to_console(&mut sim, k, red);
        click(&mut sim, red, k, i);
        assert_ne!(sim.consoles[k].state, before);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::ConsolePoked { by, .. } if *by == red))
        );
    }

    #[test]
    fn a_solved_vault_lays_out_a_weapon_and_rests() {
        let (mut sim, k, red, _) = console_sim(ConsoleKind::Vault, GameMode::Classic);
        to_console(&mut sim, k, red);
        sim.drain_events();
        let seed = sim.consoles[k].seed;
        let mut ev = Vec::new();
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, red, k, i);
        solve(&mut sim, red, k);
        ev.extend(sim.drain_events());
        assert_ne!(sim.consoles[k].seed, seed, "a new puzzle");
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::ConsoleSolved { by, .. } if *by == red))
        );
        // A special weapon in hand.
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::PickedUp { player, kind, .. }
            if *player == red && VAULT_KINDS.contains(kind)))
        );
        assert_ne!(
            sim.player(red).unwrap().weapon,
            crate::types::Weapon::Blaster
        );
        // Resting: clicks do nothing until it's ready again.
        assert!(!sim.consoles[k].open());
        let before = sim.consoles[k].state.clone();
        let i = sim.consoles[k].game.hint(&before).unwrap();
        click(&mut sim, red, k, i);
        assert_eq!(sim.consoles[k].state, before);
        let ev = run(&mut sim, ConsoleKind::Vault.rest() + 0.5);
        assert!(ev.iter().any(
            |e| matches!(e, SimEvent::ConsoleReady { console, .. } if *console as usize == k)
        ));
        assert!(sim.consoles[k].open());
    }

    #[test]
    fn a_solved_mast_shows_enemies_and_cannot_be_taken_or_switched_off() {
        let (mut sim, k, red, blue) = console_sim(ConsoleKind::Mast, GameMode::Classic);
        to_console(&mut sim, k, red);
        // Blue a little way off, behind cover or not: shown.
        let def = sim.console_defs()[k];
        let near = def.pos - def.faces[0].normal * 5.0;
        sim.teleport(blue, near, 0.0);
        sim.drain_events();
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, red, k, i);
        solve(&mut sim, red, k);
        assert_eq!(sim.consoles[k].owner, Some(Team::Red));
        let ev = run(&mut sim, 3.0);
        assert!(ev.iter().any(
            |e| matches!(e, SimEvent::Pulse { team: Team::Red, seen, range, .. }
            if *seen != 0 && *range == MAST_RANGE)
        ));
        assert!(sim.player(blue).unwrap().revealed > 0.0);
        // Blue can't take it over while it works: locked.
        to_console(&mut sim, k, blue);
        let before = sim.consoles[k].state.clone();
        let i = sim.consoles[k].game.hint(&before).unwrap();
        click(&mut sim, blue, k, i);
        assert_eq!(sim.consoles[k].state, before);
        // Shot all over: still pulsing until its time is up.
        let ev = run(&mut sim, MAST_TIME - 5.0);
        assert!(
            ev.iter()
                .filter(|e| matches!(
                    e,
                    SimEvent::Pulse {
                        team: Team::Red,
                        ..
                    }
                ))
                .count()
                >= 6
        );
        run(&mut sim, 3.0);
        let ev = run(&mut sim, 3.0);
        assert!(!ev.iter().any(|e| matches!(
            e,
            SimEvent::Pulse {
                team: Team::Red,
                ..
            }
        )));
    }

    #[test]
    fn teams_respawn_at_their_beacon_until_the_other_takes_it() {
        let (mut sim, k, red, blue) = console_sim(ConsoleKind::Beacon, GameMode::Classic);
        let at = sim.console_defs()[k].pos;
        to_console(&mut sim, k, red);
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, red, k, i);
        solve(&mut sim, red, k);
        assert_eq!(sim.consoles[k].owner, Some(Team::Red));
        // Red tagged out over and over (Blue far away): mostly back at the beacon.
        sim.teleport(red, sim.level.base_centre(Team::Blue), 0.0);
        let mut there = 0;
        for _ in 0..20 {
            let pi = sim.players.iter().position(|p| p.id == red).unwrap();
            sim.zap(pi, crate::types::TagHow::Field);
            run(&mut sim, 16.0);
            let p = sim.player(red).unwrap();
            assert!(p.alive());
            there += (p.feet.distance(at) < 2.0) as usize;
            sim.teleport(red, sim.level.base_centre(Team::Blue), 0.0);
        }
        assert!((6..=18).contains(&there), "at the beacon {there} of 20");
        // Blue takes it over (after its rest).
        run(&mut sim, 5.0);
        to_console(&mut sim, k, blue);
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, blue, k, i);
        solve(&mut sim, blue, k);
        assert_eq!(sim.consoles[k].owner, Some(Team::Blue));
    }

    #[test]
    fn no_beacons_in_infection() {
        for seed in 0..12 {
            let level = Level::generate_for(seed, ArenaSize::Large, GameMode::Infection);
            assert!(!level.consoles.iter().any(|c| c.kind == ConsoleKind::Beacon));
        }
    }

    #[test]
    fn a_solved_breaker_darkens_its_quarter_for_a_while() {
        let (mut sim, k, red, _) = console_sim(ConsoleKind::Breaker, GameMode::Classic);
        let at = sim.console_defs()[k].pos;
        let other = Vec3::new(-at.x, 1.0, at.z);
        assert!(!sim.dark_at(at));
        to_console(&mut sim, k, red);
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, red, k, i);
        solve(&mut sim, red, k);
        assert!(sim.dark_at(at + Vec3::new(0.0, 1.0, 0.0)));
        assert!(!sim.dark_at(other), "only its quarter");
        run(&mut sim, crate::console::BREAKER_TIME + 0.5);
        assert!(!sim.dark_at(at));
        assert!(!sim.consoles[k].open(), "still resetting");
    }

    #[test]
    fn a_light_bridge_holds_players_while_it_stands() {
        let (mut sim, k, red, _) = console_sim(ConsoleKind::Bridge, GameMode::Classic);
        let def = sim.console_defs()[k];
        let b = crate::console::bridge_for(&sim.level, &def).expect("its bridge");
        to_console(&mut sim, k, red);
        let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
        click(&mut sim, red, k, i);
        solve(&mut sim, red, k);
        run(&mut sim, 0.1);
        // On it, over the gap: stays up.
        let top = Vec3::new(b.mid().x, b.max.y + 0.02, b.mid().z);
        sim.teleport(red, top, 0.0);
        run(&mut sim, 1.0);
        let y = sim.player(red).unwrap().feet.y;
        assert!(
            (y - b.max.y).abs() < 0.1,
            "stood at {y}, bridge at {}",
            b.max.y
        );
        // Gone: down to the floor below.
        run(&mut sim, crate::console::BRIDGE_TIME);
        let y = sim.player(red).unwrap().feet.y;
        assert!(y < 0.5, "still up at {y}");
    }

    #[test]
    fn solved_secondary_puzzles_charge_the_solver_and_team_mates_in_the_pocket() {
        use crate::rules::SecondaryKind;
        for kind in [SecondaryKind::Circuit, SecondaryKind::Lights] {
            let rules = Rules {
                match_time: 1000.0,
                spawn_protect: 0.0,
                pickup_pairs: 0,
                powerup_pairs: 0,
                turret_range: 0.0,
                event_first: (5000.0, 5000.0),
                mode: GameMode::Megatarget,
                secondary: kind,
                ..Rules::default()
            };
            let mut sim = Sim::new(9, rules);
            assert_eq!(sim.secondary, [kind, kind]);
            let red = sim.add_player("red", Some(Team::Red), false).unwrap();
            let mate = sim.add_player("mate", Some(Team::Red), false).unwrap();
            let blue = sim.add_player("blue", Some(Team::Blue), false).unwrap();
            let k = sim
                .console_defs()
                .iter()
                .position(|d| d.kind == ConsoleKind::Secondary(0))
                .unwrap();
            assert_eq!(sim.consoles[k].game.kind(), kind.minigame().unwrap());
            let m = sim.level.mega.clone().unwrap();
            let (a, b) = m.pockets[0];
            let mid = (a + b) / 2.0;
            to_console(&mut sim, k, red);
            sim.teleport(mate, mid + Vec3::Z * 1.0, 0.0);
            sim.teleport(blue, mid - Vec3::Z * 1.0, 0.0);
            sim.step();
            // Shooting it does nothing.
            sim.drain_events();
            let i = sim.consoles[k].game.hint(&sim.consoles[k].state).unwrap();
            click(&mut sim, red, k, i);
            solve(&mut sim, red, k);
            let ev = sim.drain_events();
            let hit = |id| {
                ev.iter()
                    .any(|e| matches!(e, SimEvent::SecondaryHit { by, side: 0, .. } if *by == id))
            };
            assert!(hit(red) && hit(mate), "{kind:?}");
            assert!(!hit(blue), "{kind:?}: not the enemy");
            assert_eq!(sim.player(red).unwrap().mega_hits, 1);
            assert_eq!(sim.player(blue).unwrap().mega_hits, 0);
            assert_eq!(sim.scores[0], 2 * sim.rules.secondary_points);
        }
    }

    #[test]
    fn random_secondaries_come_in_every_kind() {
        use crate::rules::SecondaryKind;
        let mut seen = std::collections::HashSet::new();
        for seed in 0..40 {
            let level = Level::generate_for(seed, ArenaSize::Small, GameMode::Megatarget);
            let kinds = crate::console::secondary_kinds(&level, &Rules::default());
            assert_eq!(
                kinds,
                crate::console::secondary_kinds(&level, &Rules::default())
            );
            seen.extend(kinds);
        }
        for k in SecondaryKind::PICKS {
            assert!(seen.contains(&k), "{k:?} never");
        }
    }
}

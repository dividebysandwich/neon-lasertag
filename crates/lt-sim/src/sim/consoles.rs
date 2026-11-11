//! Consoles (`crate::console`): a click puzzle each (`crate::minigame`), solved like a turret
//! box's (clicks in `PlayerCmd::pokes`, from within `POKE_REACH`; team mates carry on with one
//! another's work). Solving one sets a new puzzle (of a kind the new seed picks) and does what
//! its kind does for the solver's team; most then rest for a while (`cooldown`: no clicks).

use rand::Rng;
use serde::{Deserialize, Serialize};

use super::Sim;
use crate::console::{ConsoleDef, ConsoleKind, console_defs};
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
            .map(|_| ConsoleState::new(self.console_rng.random()))
            .collect();
    }

    /// Once a tick: the consoles' timers.
    pub(super) fn step_consoles(&mut self) {
        for k in 0..self.consoles.len() {
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

    /// Console `k`'s puzzle solved by player `pi`: a new one, and what the console does.
    fn console_solved(&mut self, k: usize, pi: usize) {
        let (team, by, feet) = (
            self.players[pi].team,
            self.players[pi].id,
            self.players[pi].feet,
        );
        let def = self.console_defs[k];
        let seed = self.console_rng.random();
        self.consoles[k] = ConsoleState {
            owner: self.consoles[k].owner,
            active: self.consoles[k].active,
            cooldown: self.consoles[k].cooldown,
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
        }
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
}

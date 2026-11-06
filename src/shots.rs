//! A remote game's own shots, at once: the trigger, the bolts and what they hit are worked out
//! here as the server will (`lt_sim::shot`), instead of waiting a round trip and the
//! interpolation delay for the server to tell. The bolts fly against the players as drawn, and
//! what they hit goes to the server as `HitClaim`s, which it checks and applies (favouring the
//! shooter, within `lt_sim::sim::MAX_LAG`).
//!
//! The effects (sound, haptics, sparks) come from the events made here; the server's echo of
//! them is dropped (`Shots::echo`). Tag-outs, scores and target hits still come from the server.

use std::collections::VecDeque;

use bevy::math::Vec3;
use lt_sim::collide::World;
use lt_sim::level::Level;
use lt_sim::shot::{self, BoltTarget, Contact, Gun, HitClaim, Shot, Trigger};
use lt_sim::sim::{Bolt, BoltSnap, PlayerSnap, Snapshot};
use lt_sim::{PlayerCmd, PlayerId, Rules, SimEvent, TICK};

/// Bolts drawn from here get ids from this up (the server's are far below).
const ID_BASE: u32 = 0xC000_0000;
/// Ticks a predicted hit or impact waits for the server's echo of it.
const ECHO_WAIT: u64 = 120;
/// How near (m) the server's impact has to be to one made here to be its echo.
const ECHO_NEAR: f32 = 0.5;

struct Flying {
    bolt: Bolt,
    shot: u32,
    pellet: u8,
}

#[derive(Default)]
pub struct Shots {
    gun: Gun,
    /// Shots fired (`PlayerCmd::shot`), and the ammo each cost, until the server's state
    /// includes them.
    seq: u32,
    costs: VecDeque<(u32, u16)>,
    bolts: Vec<Flying>,
    next_id: u32,
    claims: Vec<HitClaim>,
    /// Ticks counted here; the victims hit and spots struck lately (for `echo`).
    clock: u64,
    hit: VecDeque<(u64, PlayerId)>,
    struck: VecDeque<(u64, Vec3)>,
}

/// What `first_hit` needs of a player as drawn (the blaster's box isn't in the snapshot).
fn snap_cmd(p: &PlayerSnap) -> PlayerCmd {
    PlayerCmd {
        yaw: p.yaw,
        head: p.head,
        hands: p.hands,
        ..Default::default()
    }
}

impl Shots {
    /// The trigger this tick, for command `cmd` (which gets the shot count). `me`: the local
    /// player as the server last told (`None` before that); `feet`: where it is now.
    #[allow(clippy::too_many_arguments)]
    pub fn trigger(
        &mut self,
        cmd: &mut PlayerCmd,
        me: Option<&PlayerSnap>,
        feet: Vec3,
        world: &World,
        rules: &Rules,
        id: PlayerId,
        out: &mut Vec<SimEvent>,
    ) {
        self.clock += 1;
        cmd.shot = self.seq;
        let Some(me) = me else {
            self.gun.prev_fire = cmd.fire;
            return;
        };
        // The server's word on the gun, with the shots it hasn't seen yet taken off.
        while self.costs.front().is_some_and(|c| c.0 <= me.shot) {
            self.costs.pop_front();
        }
        let owed: u16 = self.costs.iter().map(|c| c.1).sum();
        self.gun.ammo = me.ammo.saturating_sub(owed);
        self.gun.rapid = me.rapid;
        self.gun.shotgun = me.shotgun;
        self.gun.charge = me.charge;
        self.gun.cooldown = me.cooldown;
        self.gun.tick();
        if me.respawn.is_some() {
            self.gun.prev_fire = cmd.fire;
            self.gun.charging = None;
            return;
        }
        let player = id;
        let shot = match self.gun.trigger(cmd.fire, cmd.gun_hand.is_some(), rules) {
            None => return,
            Some(Trigger::Denied) => return out.push(SimEvent::Denied { player }),
            Some(Trigger::Empty) => return out.push(SimEvent::Empty { player }),
            Some(Trigger::ChargeFull) => return out.push(SimEvent::ChargeFull { player }),
            Some(Trigger::Fire(s)) => s,
        };
        self.gun.fired(shot, rules);
        self.seq += 1;
        cmd.shot = self.seq;
        let cost = if shot == Shot::Beam {
            rules.beam_cost
        } else {
            1
        };
        self.costs.push_back((self.seq, cost));
        let eye = feet + shot::head_offset(cmd.head.pos);
        let (from, dir) = shot::muzzle(feet, eye, cmd);
        out.push(SimEvent::Fired {
            player,
            pos: from,
            dir,
        });
        if shot == Shot::Beam {
            out.push(SimEvent::BeamFired {
                player,
                pos: from,
                dir,
            });
        }
        let team = Some(me.team);
        if let Some((_, q, n)) = world.ray(eye, from) {
            self.struck.push_back((self.clock, q));
            out.push(SimEvent::BoltImpact {
                pos: q,
                normal: n,
                team,
            });
            return;
        }
        let seed = shot::shot_seed(id, self.seq);
        for (k, (vel, life, extra)) in shot::shot_bolts(shot, dir, rules, seed)
            .into_iter()
            .enumerate()
        {
            self.next_id = self.next_id.wrapping_add(1);
            self.bolts.push(Flying {
                bolt: Bolt {
                    id: ID_BASE | (self.next_id & !ID_BASE),
                    owner: id,
                    owner_team: me.team,
                    team,
                    pos: from,
                    vel,
                    life,
                    extra,
                },
                shot: self.seq,
                pellet: k as u8,
            });
        }
    }

    /// A tick of the bolts' flight against `snap` (the players as drawn, at tick `view`), the
    /// arena and its barriers. Hits become claims (`take_claims`).
    pub fn fly(
        &mut self,
        snap: &Snapshot,
        view: f64,
        world: &World,
        level: &Level,
        rules: &Rules,
        out: &mut Vec<SimEvent>,
    ) {
        let cmds: Vec<PlayerCmd> = snap.players.iter().map(snap_cmd).collect();
        let targets = || {
            snap.players.iter().zip(&cmds).map(|(p, cmd)| BoltTarget {
                id: p.id,
                team: p.team,
                alive: p.respawn.is_none(),
                feet: p.feet,
                eye: p.feet + p.head.pos,
                cmd,
            })
        };
        let Shots {
            bolts,
            claims,
            clock,
            hit,
            struck,
            ..
        } = self;
        let clock = *clock;
        let mut i = 0;
        while i < bolts.len() {
            let f = &mut bolts[i];
            let mut gone = false;
            // (A beam goes on through whoever it hits: maybe several in a tick.)
            for _ in 0..8 {
                match shot::bolt_contact(
                    world,
                    &snap.barriers,
                    rules,
                    &f.bolt,
                    targets().enumerate(),
                ) {
                    Contact::Clear => {}
                    Contact::Barrier { index, at } => {
                        struck.push_back((clock, at));
                        out.push(SimEvent::BarrierHit {
                            id: snap.barriers[index].id,
                            pos: at,
                        });
                        gone = true;
                    }
                    Contact::Player { index, at } => {
                        let p = &snap.players[index];
                        claims.push(HitClaim {
                            shot: f.shot,
                            pellet: f.pellet,
                            victim: p.id,
                            view: view as f32,
                            at,
                        });
                        hit.push_back((clock, p.id));
                        struck.push_back((clock, at));
                        // As the server will likely judge it.
                        out.push(if p.shield > 0.0 {
                            SimEvent::Blocked {
                                player: p.id,
                                pos: at,
                            }
                        } else if p.cooldown > 0.0 {
                            SimEvent::BoltImpact {
                                pos: at,
                                normal: -f.bolt.vel.normalize_or_zero(),
                                team: f.bolt.team,
                            }
                        } else {
                            SimEvent::Hit {
                                player: p.id,
                                by: f.bolt.owner,
                                pos: at,
                                hits: (p.hits + f.bolt.extra.damage.max(1)).min(rules.hits_to_tag),
                            }
                        });
                        if f.bolt.extra.pierce {
                            f.bolt.extra.struck |= 1u64 << (p.id % 64);
                            continue;
                        }
                        gone = true;
                    }
                    Contact::Arena { q, n } => {
                        struck.push_back((clock, q));
                        if let Some(n) = shot::bounce(level, rules, &mut f.bolt, q) {
                            out.push(SimEvent::Reflected {
                                pos: q,
                                normal: n,
                                team: f.bolt.team,
                            });
                        } else {
                            out.push(SimEvent::BoltImpact {
                                pos: q,
                                normal: n,
                                team: f.bolt.team,
                            });
                            gone = true;
                        }
                    }
                }
                break;
            }
            f.bolt.pos += f.bolt.vel * TICK;
            f.bolt.life -= TICK;
            if gone || f.bolt.life <= 0.0 {
                bolts.swap_remove(i);
            } else {
                i += 1;
            }
        }
        let old = self.clock.saturating_sub(ECHO_WAIT);
        self.hit.retain(|h| h.0 > old);
        self.struck.retain(|s| s.0 > old);
    }

    /// The claims to send.
    pub fn take_claims(&mut self) -> Vec<HitClaim> {
        std::mem::take(&mut self.claims)
    }

    /// Shows the bolts flying from here instead of the server's copies of them, and the gun's
    /// state as predicted (the HUD's ammo).
    pub fn show(&self, snap: &mut Snapshot, me: PlayerId) {
        snap.bolts.retain(|b| b.owner != me || b.team.is_none());
        snap.bolts.extend(self.bolts.iter().map(|f| BoltSnap {
            id: f.bolt.id,
            pos: f.bolt.pos,
            vel: f.bolt.vel,
            team: f.bolt.team,
            beam: f.bolt.extra.pierce,
            owner: me,
        }));
        if let Some(p) = snap.players.iter_mut().find(|p| p.id == me) {
            p.ammo = self.gun.ammo;
            p.charging = self.gun.charging.unwrap_or(0.0);
        }
    }

    /// Whether the server's event `e` is one made here already (and so dropped).
    pub fn echo(&mut self, e: &SimEvent, me: PlayerId) -> bool {
        match *e {
            SimEvent::Fired { player, .. }
            | SimEvent::BeamFired { player, .. }
            | SimEvent::Empty { player }
            | SimEvent::Denied { player }
            | SimEvent::ChargeFull { player } => player == me,
            SimEvent::Hit { player, by, .. } if by == me => self.seen_hit(player),
            SimEvent::Blocked { player, .. } => self.seen_hit(player),
            SimEvent::BoltImpact { pos, .. }
            | SimEvent::Reflected { pos, .. }
            | SimEvent::BarrierHit { pos, .. } => {
                match self
                    .struck
                    .iter()
                    .position(|s| s.1.distance(pos) < ECHO_NEAR)
                {
                    Some(k) => {
                        self.struck.remove(k);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn seen_hit(&mut self, victim: PlayerId) -> bool {
        match self.hit.iter().position(|h| h.1 == victim) {
            Some(k) => {
                self.hit.remove(k);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lt_sim::sim::{EYE, Sim, look_rot};
    use lt_sim::{Hand, Pose, Team};

    /// A remote shooter's predicted bolts against the server's sim (no delay): what they hit is
    /// claimed and the claims count; the sim's own copies hit nobody.
    #[test]
    fn predicted_hits_are_accepted() {
        let rules = Rules {
            pickup_pairs: 0,
            powerup_pairs: 0,
            ..Rules::default()
        };
        let mut sim = Sim::new(7, rules.clone());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let b = sim.add_player("b", Some(Team::Blue), false).unwrap();
        sim.players[0].client_hits = true;
        let world = sim.level.world();
        let nav = &sim.level.nav;
        let open: Vec<Vec3> = (0..nav.pos.len())
            .filter(|&i| nav.valid[i])
            .map(|i| nav.pos[i])
            .collect();
        let (p, q) = open
            .iter()
            .flat_map(|p| open.iter().map(move |q| (*p, *q)))
            .find(|(p, q)| {
                let d = p.distance(*q);
                (3.5..5.0).contains(&d)
                    && (p.y - q.y).abs() < 0.01
                    && world.clear(*p + Vec3::Y * EYE, *q + Vec3::Y * 1.2)
                    && world.clear(*p + Vec3::Y * 1.4, *q + Vec3::Y * 1.4)
            })
            .expect("an open line");
        sim.teleport(a, p, lt_sim::sim::yaw_of(q - p));
        sim.teleport(b, q, 0.0);
        let mut shots = Shots::default();
        let mut events = Vec::new();
        let mut server = Vec::new();
        for t in 0..60 {
            let snap = sim.snapshot();
            let me = snap.players.iter().find(|s| s.id == a).cloned();
            let feet = sim.player(a).unwrap().feet;
            let muzzle = feet + Vec3::Y * (EYE - 0.2);
            let chest = sim.player(b).unwrap().chest();
            let mut cmd = PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: muzzle - feet,
                    rot: look_rot(chest - muzzle),
                },
                fire: t == 10,
                view_tick: snap.tick,
                feet: Some(feet),
                ..Default::default()
            };
            shots.trigger(&mut cmd, me.as_ref(), feet, &world, &rules, a, &mut events);
            sim.set_cmd(a, cmd);
            sim.step();
            let snap = sim.snapshot();
            shots.fly(
                &snap,
                snap.tick as f64,
                &world,
                &sim.level.clone(),
                &rules,
                &mut events,
            );
            server.extend(sim.drain_events());
            for c in shots.take_claims() {
                assert_eq!(sim.claim_hit(a, c), lt_sim::sim::Claim::Accepted, "{c:?}");
            }
        }
        assert!(events.iter().any(|e| matches!(e, SimEvent::Fired { .. })));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SimEvent::Hit { player, .. } if *player == b)),
            "{events:?}"
        );
        // (On a spawn pad the victim is healed again at once.)
        let hits: Vec<_> = server
            .iter()
            .filter(|e| matches!(e, SimEvent::Hit { player, by, .. } if *player == b && *by == a))
            .collect();
        assert_eq!(hits.len(), 1, "the claim counted, once: {server:?}");
        assert_eq!(sim.player(a).unwrap().shots, 1);
    }
}

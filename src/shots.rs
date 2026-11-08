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
use lt_sim::hitbox::Part;
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
    /// Infection under way (`Snapshot::infected`): a phantom's bolts are short and deadly.
    pub infected: bool,
    gun: Gun,
    /// Dual pistols' second.
    gun2: Gun,
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
            self.gun.queued = false;
            return;
        };
        // The server's word on the gun, with the shots it hasn't seen yet taken off.
        while self.costs.front().is_some_and(|c| c.0 <= me.shot) {
            self.costs.pop_front();
        }
        let owed: u16 = self.costs.iter().map(|c| c.1).sum();
        self.gun.ammo = me.ammo.saturating_sub(owed);
        self.gun.rapid = me.rapid;
        self.gun.weapon = me.weapon;
        self.gun.cooldown = me.cooldown;
        self.gun.tick();
        // (Dual pistols: the second's trigger, sharing the shots.)
        self.gun2.ammo = self.gun.ammo;
        self.gun2.rapid = me.rapid;
        self.gun2.weapon = me.weapon;
        self.gun2.cooldown = me.cooldown;
        self.gun2.tick();
        if me.respawn.is_some() {
            self.gun.prev_fire = cmd.fire;
            self.gun.charging = None;
            self.gun.queued = false;
            self.gun2.prev_fire = cmd.fire2;
            self.gun2.queued = false;
            return;
        }
        let player = id;
        // Both triggers first (the second's with the first's shot paid), then the shots, in
        // order (as the sim numbers them).
        let mut fired = Vec::new();
        for second in [false, true] {
            let t = if second {
                if me.weapon != lt_sim::Weapon::Dual {
                    self.gun2.prev_fire = cmd.fire2;
                    self.gun2.queued = false;
                    continue;
                }
                self.gun2.ammo = self.gun.ammo;
                self.gun2.trigger(cmd.fire2, cmd.gun2_hand.is_some(), rules)
            } else {
                self.gun.trigger(cmd.fire, cmd.gun_hand.is_some(), rules)
            };
            match t {
                None => {}
                Some(Trigger::Denied) => out.push(SimEvent::Denied { player }),
                Some(Trigger::Empty) => out.push(SimEvent::Empty { player }),
                Some(Trigger::ChargeFull) => out.push(SimEvent::ChargeFull { player }),
                Some(Trigger::Fire(shot)) => {
                    if second {
                        self.gun2.fired(shot, rules);
                        self.gun.ammo = self.gun2.ammo;
                    } else {
                        self.gun.fired(shot, rules);
                    }
                    fired.push((shot, second));
                }
            }
        }
        for (shot, second) in fired {
            self.fire(cmd, me, feet, world, rules, id, shot, second, out);
        }
    }

    /// One shot fired (`trigger`): counted, and its bolts flying.
    #[allow(clippy::too_many_arguments)]
    fn fire(
        &mut self,
        cmd: &mut PlayerCmd,
        me: &PlayerSnap,
        feet: Vec3,
        world: &World,
        rules: &Rules,
        id: PlayerId,
        shot: Shot,
        second: bool,
        out: &mut Vec<SimEvent>,
    ) {
        let player = id;
        self.seq += 1;
        cmd.shot = self.seq;
        self.costs.push_back((self.seq, shot.cost(rules)));
        let eye = feet + shot::head_offset(cmd.head.pos);
        let (from, dir) = shot::muzzle_of(feet, eye, cmd, second);
        out.push(SimEvent::Fired {
            player,
            pos: from,
            dir,
            second,
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
        let bolts = shot::shot_bolts(shot, dir, rules, seed);
        let bolts = shot::as_phantom(bolts, rules, me.team, self.infected);
        for (k, (vel, life, extra)) in bolts.into_iter().enumerate() {
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
        let decoy_cmds: Vec<PlayerCmd> = snap
            .decoys
            .iter()
            .map(|d| PlayerCmd {
                yaw: d.yaw,
                head: d.head,
                hands: d.hands,
                gun_hand: d.gun_hand,
                ..Default::default()
            })
            .collect();
        // (The decoys after the players: a bolt stops at one, the server pops it.)
        let targets = || {
            snap.players
                .iter()
                .zip(&cmds)
                .map(|(p, cmd)| BoltTarget {
                    id: p.id,
                    team: p.team,
                    alive: p.respawn.is_none(),
                    feet: p.feet,
                    eye: p.feet + p.head.pos,
                    cmd,
                })
                .chain(
                    snap.decoys
                        .iter()
                        .zip(&decoy_cmds)
                        .map(|(d, cmd)| BoltTarget {
                            id: d.owner,
                            team: d.team,
                            alive: true,
                            feet: d.feet,
                            eye: d.feet + shot::head_offset(d.head.pos),
                            cmd,
                        }),
                )
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
                let contact = shot::bolt_contact(
                    world,
                    &snap.barriers,
                    rules,
                    &f.bolt,
                    targets().enumerate(),
                );
                if f.bolt.extra.lance {
                    out.push(SimEvent::Streak {
                        player: f.bolt.owner,
                        from: f.bolt.pos,
                        to: match contact {
                            Contact::Clear => f.bolt.pos + f.bolt.vel * TICK,
                            Contact::Player { at, .. }
                            | Contact::Barrier { at, .. }
                            | Contact::Mirror { at, .. } => at,
                            Contact::Arena { q, .. } => q,
                        },
                    });
                }
                match contact {
                    Contact::Clear => gone = f.bolt.extra.lance,
                    Contact::Mirror { index, at, n } => {
                        struck.push_back((clock, at));
                        match shot::bounce_placed(&mut f.bolt, at, n) {
                            Some(n) => out.push(SimEvent::Reflected {
                                pos: at,
                                normal: n,
                                team: f.bolt.team,
                            }),
                            None => {
                                out.push(SimEvent::BarrierHit {
                                    id: snap.barriers[index].id,
                                    pos: at,
                                });
                                gone = true;
                            }
                        }
                    }
                    Contact::Barrier { index, at } => {
                        struck.push_back((clock, at));
                        out.push(SimEvent::BarrierHit {
                            id: snap.barriers[index].id,
                            pos: at,
                        });
                        gone = true;
                    }
                    Contact::Player { index, .. } if index >= snap.players.len() => {
                        gone = true;
                    }
                    Contact::Player { index, at, part } => {
                        let p = &snap.players[index];
                        let head = f.bolt.extra.lance && part == Part::Head;
                        claims.push(HitClaim {
                            shot: f.shot,
                            pellet: f.pellet,
                            victim: p.id,
                            view: view as f32,
                            at,
                            head,
                        });
                        let damage = if head {
                            rules.lance_head_damage
                        } else {
                            f.bolt.extra.damage.max(1)
                        };
                        let counts = p.shield <= 0.0 && p.cooldown <= 0.0 && p.protect <= 0.0;
                        if head && counts {
                            out.push(SimEvent::Headshot {
                                player: p.id,
                                by: f.bolt.owner,
                                pos: at,
                            });
                        }
                        hit.push_back((clock, p.id));
                        struck.push_back((clock, at));
                        // As the server will likely judge it.
                        if p.shield <= 0.0 && (p.cooldown > 0.0 || p.protect > 0.0) {
                            out.push(SimEvent::Immune {
                                player: p.id,
                                by: f.bolt.owner,
                                pos: at,
                            });
                        }
                        out.push(if p.shield > 0.0 {
                            SimEvent::Blocked {
                                player: p.id,
                                by: f.bolt.owner,
                                pos: at,
                            }
                        } else if p.cooldown > 0.0 || p.protect > 0.0 {
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
                                hits: (p.hits + damage).min(rules.hits_to_tag),
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
                        if let Some(n) = shot::bounce(
                            level,
                            rules,
                            &mut f.bolt,
                            q,
                            n,
                            matches!(snap.event, Some((lt_sim::ArenaEvent::MirrorStorm, _))),
                        ) {
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
        snap.bolts.extend(
            self.bolts
                .iter()
                .filter(|f| !f.bolt.extra.lance)
                .map(|f| BoltSnap {
                    id: f.bolt.id,
                    pos: f.bolt.pos,
                    vel: f.bolt.vel,
                    team: f.bolt.team,
                    beam: f.bolt.extra.pierce,
                    owner: me,
                    bounces: f.bolt.extra.bounces,
                }),
        );
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
            | SimEvent::ChargeFull { player }
            | SimEvent::Streak { player, .. } => player == me,
            SimEvent::Headshot { by, .. } => by == me,
            SimEvent::Hit { player, by, .. } if by == me => self.seen_hit(player),
            SimEvent::Blocked { player, .. } => self.seen_hit(player),
            SimEvent::Immune { player, by, .. } if by == me => self.seen_hit(player),
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

    /// A remote shooter (with the charge gun if `charge`, holding the trigger down for `hold`
    /// ticks) fires once at an enemy 3.5-5 m away; its predicted bolts fly against the server's
    /// sim (no delay) and their claims are applied. The events made here and the sim's.
    fn remote_shot(charge: bool, hold: u64) -> (Vec<SimEvent>, Vec<SimEvent>, PlayerId, PlayerId) {
        remote_shot_lagged(charge.then_some(lt_sim::PickupKind::Charge), hold, 0, false)
    }

    /// `remote_shot` with `lag` ticks each way between the client and the sim (commands,
    /// snapshots and claims all late by that).
    fn remote_shot_lagged(
        gun: Option<lt_sim::PickupKind>,
        hold: u64,
        lag: usize,
        walk: bool,
    ) -> (Vec<SimEvent>, Vec<SimEvent>, PlayerId, PlayerId) {
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
            // (Off the spawn pads: the enemy's tag out.)
            .filter(|&i| nav.valid[i] && sim.level.spawn_at(nav.pos[i]).is_none())
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
        if let Some(k) = gun {
            sim.give(a, k);
        }
        // The victim strafes across the line of fire.
        if walk {
            let side = (q - p).normalize().cross(Vec3::Y);
            sim.set_cmd(
                b,
                PlayerCmd {
                    stick: bevy::math::Vec2::new(side.x, side.z) * 0.5,
                    ..Default::default()
                },
            );
        }
        let mut shots = Shots::default();
        let mut events = Vec::new();
        let mut server = Vec::new();
        let mut snaps: VecDeque<Snapshot> = VecDeque::new();
        let mut cmds: VecDeque<PlayerCmd> = VecDeque::new();
        let mut claims: VecDeque<Vec<HitClaim>> = VecDeque::new();
        let mut early: Vec<HitClaim> = Vec::new();
        for t in 0..(hold + 60 + 3 * lag as u64) {
            snaps.push_back(sim.snapshot());
            if snaps.len() > lag + 1 {
                snaps.pop_front();
            }
            let snap = snaps.front().unwrap().clone();
            let me = snap.players.iter().find(|s| s.id == a).cloned();
            let feet = sim.player(a).unwrap().feet;
            let muzzle = feet + Vec3::Y * (EYE - 0.2);
            // (Aimed at the middle of the victim's chest as shown: it hangs from the neck,
            // behind the eyes, as `lt_sim::hitbox` has it.)
            let chest = snap
                .players
                .iter()
                .find(|s| s.id == b)
                .map(|s| {
                    lt_sim::sim::chest_top(s.feet + s.head.pos, s.head.rot)
                        - Vec3::Y * lt_sim::hitbox::CHEST_HALF.y
                })
                .unwrap();
            let mut cmd = PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: muzzle - feet,
                    rot: look_rot(chest - muzzle),
                },
                fire: (10..10 + hold).contains(&t),
                view_tick: snap.tick,
                feet: Some(feet),
                ..Default::default()
            };
            shots.trigger(&mut cmd, me.as_ref(), feet, &world, &rules, a, &mut events);
            cmds.push_back(cmd);
            if cmds.len() > lag {
                sim.set_cmd(a, cmds.pop_front().unwrap());
            }
            // (As the server tries them: before the step, again while too early.)
            // (Claims sent with a command arrive with it.)
            let due = if claims.len() >= lag.max(1) {
                claims.pop_front().unwrap()
            } else {
                Vec::new()
            };
            for c in std::mem::take(&mut early).into_iter().chain(due) {
                match sim.claim_hit(a, c) {
                    lt_sim::sim::Claim::Accepted => {}
                    lt_sim::sim::Claim::Early => early.push(c),
                    r => panic!("{r:?}: {c:?} at {}", sim.tick),
                }
            }
            sim.step();
            let view = snap.tick + 1;
            let snap = snaps.get(1).cloned().unwrap_or_else(|| sim.snapshot());
            shots.fly(
                &snap,
                view as f64,
                &world,
                &sim.level.clone(),
                &rules,
                &mut events,
            );
            server.extend(sim.drain_events());
            claims.push_back(shots.take_claims());
        }
        (events, server, a, b)
    }

    /// What the sim made of a shot: hits on `b` by `a`, and their damage.
    fn hits_on(server: &[SimEvent], a: PlayerId, b: PlayerId) -> Vec<u8> {
        server
            .iter()
            .filter_map(|e| match *e {
                SimEvent::Hit {
                    player, by, hits, ..
                } if player == b && by == a => Some(hits),
                _ => None,
            })
            .collect()
    }

    /// The predicted bolts' claims count; the sim's own copies hit nobody.
    #[test]
    fn predicted_hits_are_accepted() {
        let (events, server, a, b) = remote_shot(false, 1);
        assert!(events.iter().any(|e| matches!(e, SimEvent::Fired { .. })));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SimEvent::Hit { player, .. } if *player == b)),
            "{events:?}"
        );
        // (On a spawn pad the victim is healed again at once.)
        assert_eq!(
            hits_on(&server, a, b),
            [1],
            "the claim counted, once: {server:?}"
        );
    }

    /// Shots over a lagging connection: the claims still count.
    #[test]
    fn lagged_claims_count() {
        for walk in [false, true] {
            for lag in [2, 6, 12] {
                let (_, server, a, b) = remote_shot_lagged(None, 1, lag, walk);
                assert_eq!(hits_on(&server, a, b), [1], "bolt, {lag} ticks: {server:?}");
                let (_, server, a, b) =
                    remote_shot_lagged(Some(lt_sim::PickupKind::Charge), 70, lag, walk);
                assert_eq!(hits_on(&server, a, b), [2], "beam, {lag} ticks: {server:?}");
                let (events, server, a, b) =
                    remote_shot_lagged(Some(lt_sim::PickupKind::Lance), 1, lag, walk);
                assert_eq!(
                    hits_on(&server, a, b),
                    [1],
                    "lance, {lag} ticks: {server:?}"
                );
                assert!(events.iter().any(|e| matches!(e, SimEvent::Streak { .. })));
            }
        }
    }

    /// A fully charged beam's claim counts, with the beam's damage.
    #[test]
    fn predicted_beams_hit() {
        let (events, server, a, b) = remote_shot(true, 70);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SimEvent::BeamFired { .. })),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SimEvent::Hit { player, .. } if *player == b)),
            "predicted: {events:?}"
        );
        assert_eq!(hits_on(&server, a, b), [2], "{server:?}");
    }
}

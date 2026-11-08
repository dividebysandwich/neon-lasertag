//! Firing and a bolt's flight without the sim's side effects, shared by the sim and a remote
//! client's prediction of its own shots (src/shots.rs): the trigger (rate, ammo, the charge gun),
//! what a shot fires (the shotgun's spread from a seed both sides know), and what a bolt meets
//! in a tick.
//!
//! **Client-side hits.** A remote player's client flies its own bolts at once against the
//! players as it draws them and tells the server what they hit (`HitClaim`); the server's
//! copies of those bolts never hit players themselves. The server checks each claim against
//! the shot it fired and where the victim was at that moment (`Sim::claim_hit`).

use glam::{Quat, Vec3};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::collide::World;
use crate::hitbox::{Part, first_hit};
use crate::level::Level;
use crate::rules::Rules;
use crate::sim::{Barrier, Bolt, BoltExtra, EYE, TICK};
use crate::types::*;

/// What a trigger pull fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shot {
    Bolt,
    Spread,
    Beam,
    /// A bolt that bounces off every wall.
    Ricochet,
    /// The prism lance's (all but) hitscan shot.
    Lance,
}

/// The parts of a player the trigger works with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gun {
    pub prev_fire: bool,
    pub fire_cd: f32,
    pub charging: Option<f32>,
    pub ammo: u16,
    pub rapid: bool,
    pub weapon: Weapon,
    /// The lockout after a hit.
    pub cooldown: f32,
    /// Pulled a moment too early (within `FIRE_BUFFER` of being ready): fires once ready.
    pub queued: bool,
}

/// How early (s) a trigger pull may come before the gun is ready and still fire once it is.
pub const FIRE_BUFFER: f32 = 0.1;

/// What the trigger did this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Pulled during the hit lockout.
    Denied,
    /// Pulled with no shots left.
    Empty,
    /// The charge gun has been held long enough.
    ChargeFull,
    Fire(Shot),
}

impl Gun {
    /// Least time between shots.
    pub fn interval(&self, rules: &Rules) -> f32 {
        let base = match self.weapon {
            Weapon::Ricochet => rules.ricochet_cooldown,
            Weapon::Lance => rules.lance_cooldown,
            Weapon::Dual => rules.dual_cooldown,
            _ => rules.fire_cooldown,
        };
        if self.rapid { base * 0.5 } else { base }
    }

    /// The tick's timers (before `trigger`).
    pub fn tick(&mut self) {
        // (Snapped to 0 within a rounding error of the last tick, so an interval of whole ticks
        // is just that.)
        self.fire_cd = if self.fire_cd > TICK + 1e-5 {
            self.fire_cd - TICK
        } else {
            0.0
        };
    }

    /// One shot per trigger pull; the charge gun fires on letting go (a beam once held for
    /// `Rules::charge_time`), and a hit or holstering stops its charging. A pull just before
    /// the gun is ready fires once it is (`FIRE_BUFFER`). `armed`: a gun is in hand. A `Fire`
    /// is paid for with `fired`.
    pub fn trigger(&mut self, fire: bool, armed: bool, rules: &Rules) -> Option<Trigger> {
        let mut pull = fire && !self.prev_fire;
        self.prev_fire = fire;
        if !armed || self.cooldown > 0.0 {
            self.charging = None;
            self.queued = false;
        }
        if self.queued && self.fire_cd <= 0.0 {
            self.queued = false;
            pull = true;
        }
        let mut out = None;
        if pull && armed {
            if self.cooldown > 0.0 {
                return Some(Trigger::Denied);
            } else if self.fire_cd > 0.0 {
                // Faster than the gun's rate: kept for when it's ready if that's soon, else
                // ignored.
                self.queued = self.fire_cd <= FIRE_BUFFER;
            } else if self.ammo == 0 {
                self.fire_cd = self.interval(rules);
                return Some(Trigger::Empty);
            } else {
                match self.weapon {
                    // (A queued pull let go of already: a tap.)
                    Weapon::Charge if !fire => return Some(Trigger::Fire(Shot::Bolt)),
                    Weapon::Charge => self.charging = Some(0.0),
                    Weapon::Shotgun => return Some(Trigger::Fire(Shot::Spread)),
                    Weapon::Blaster => return Some(Trigger::Fire(Shot::Bolt)),
                    Weapon::Ricochet => return Some(Trigger::Fire(Shot::Ricochet)),
                    Weapon::Lance => return Some(Trigger::Fire(Shot::Lance)),
                    Weapon::Dual => return Some(Trigger::Fire(Shot::Bolt)),
                }
            }
        }
        if let Some(t) = self.charging {
            if fire {
                let held = t + TICK;
                self.charging = Some(held);
                if t < rules.charge_time && held >= rules.charge_time {
                    out = Some(Trigger::ChargeFull);
                }
            } else {
                self.charging = None;
                out = Some(Trigger::Fire(
                    if t >= rules.charge_time && self.ammo >= rules.beam_cost {
                        Shot::Beam
                    } else {
                        Shot::Bolt
                    },
                ));
            }
        }
        out
    }

    /// Pays for a shot.
    pub fn fired(&mut self, shot: Shot, rules: &Rules) {
        self.fire_cd = self.interval(rules);
        self.ammo = self.ammo.saturating_sub(shot.cost(rules));
    }
}

impl Shot {
    /// The shots it takes from the gun.
    pub fn cost(self, rules: &Rules) -> u16 {
        match self {
            Shot::Beam => rules.beam_cost,
            Shot::Lance => rules.lance_cost,
            _ => 1,
        }
    }

    /// How fast its bolts fly (m/s).
    pub fn speed(self, rules: &Rules) -> f32 {
        match self {
            Shot::Bolt | Shot::Spread => rules.blaster_bolt_speed,
            Shot::Ricochet => rules.blaster_bolt_speed * 0.75,
            Shot::Beam => rules.beam_speed,
            // (Across its whole range in a tick.)
            Shot::Lance => rules.lance_range / TICK,
        }
    }
}

/// The eyes' offset from the feet for a commanded head position (none: standing height).
pub fn head_offset(h: Vec3) -> Vec3 {
    if h == Vec3::ZERO {
        Vec3::Y * EYE
    } else {
        h.clamp(Vec3::new(-0.6, 0.4, -0.6), Vec3::new(0.6, 2.2, 0.6))
    }
}

/// Where a shot leaves from and its direction (unit), for a player at `feet` with eyes at
/// `eye`: the muzzle, or the eye when the muzzle is far from it.
pub fn muzzle(feet: Vec3, eye: Vec3, cmd: &PlayerCmd) -> (Vec3, Vec3) {
    let mut from = feet + cmd.muzzle.pos;
    if !from.is_finite() || (from - eye).length() > 1.5 {
        from = eye;
    }
    let mut dir = cmd.muzzle.forward();
    if !dir.is_finite() || dir.length_squared() < 0.5 {
        dir = Quat::from_rotation_y(cmd.yaw) * Vec3::NEG_Z;
    }
    (from, dir.normalize())
}

/// `muzzle`, for the second of dual pistols (`PlayerCmd::muzzle2`) if `second`.
pub fn muzzle_of(feet: Vec3, eye: Vec3, cmd: &PlayerCmd, second: bool) -> (Vec3, Vec3) {
    if second {
        let c = PlayerCmd {
            muzzle: cmd.muzzle2,
            ..*cmd
        };
        muzzle(feet, eye, &c)
    } else {
        muzzle(feet, eye, cmd)
    }
}

/// The seed of a remote player's `seq`-th shot (the shotgun's spread), known to both sides.
pub fn shot_seed(player: PlayerId, seq: u32) -> u64 {
    let x = ((player as u64) << 32 | seq as u64) ^ 0x9E37_79B9_7F4A_7C15;
    // (splitmix64's finaliser.)
    let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// The bolts a shot along `dir` fires: velocity, life and extras each.
pub fn shot_bolts(shot: Shot, dir: Vec3, rules: &Rules, seed: u64) -> Vec<(Vec3, f32, BoltExtra)> {
    match shot {
        Shot::Bolt => vec![(
            dir * shot.speed(rules),
            rules.bolt_life,
            BoltExtra::default(),
        )],
        Shot::Spread => {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let side = dir.any_orthonormal_vector();
            let up = dir.cross(side);
            let speed = shot.speed(rules);
            let life = rules.shotgun_range / speed;
            (0..rules.shotgun_bolts)
                .map(|_| {
                    // Evenly over the cone's disc.
                    let r = rng.random::<f32>().sqrt() * rules.shotgun_spread.tan();
                    let a = rng.random_range(0.0..std::f32::consts::TAU);
                    let d = (dir + (side * a.cos() + up * a.sin()) * r).normalize();
                    (d * speed, life, BoltExtra::default())
                })
                .collect()
        }
        // (Across its whole range in a tick, and a tick more after a mirror.)
        Shot::Lance => vec![(
            dir * shot.speed(rules),
            TICK * (1 + rules.mirror_bounces as u32) as f32 - TICK * 0.5,
            BoltExtra {
                lance: true,
                ..BoltExtra::default()
            },
        )],
        Shot::Ricochet => vec![(
            dir * shot.speed(rules),
            rules.bolt_life,
            BoltExtra {
                ricochet: true,
                ..BoltExtra::default()
            },
        )],
        Shot::Beam => vec![(
            dir * shot.speed(rules),
            rules.beam_range / shot.speed(rules),
            BoltExtra {
                damage: rules.beam_damage,
                pierce: true,
                ..BoltExtra::default()
            },
        )],
    }
}

/// Infection (`infected`: under way): a phantom's (Red's) bolts fly only
/// `Rules::phantom_range` and tag out at once.
pub fn as_phantom(
    bolts: Vec<(Vec3, f32, BoltExtra)>,
    rules: &Rules,
    team: Team,
    infected: bool,
) -> Vec<(Vec3, f32, BoltExtra)> {
    if !infected || team != Team::Red {
        return bolts;
    }
    bolts
        .into_iter()
        .map(|(vel, life, extra)| {
            let speed = vel.length().max(1.0);
            (
                vel,
                life.min(rules.phantom_range / speed),
                BoltExtra {
                    damage: rules.hits_to_tag,
                    ..extra
                },
            )
        })
        .collect()
}

/// A player as a bolt can meet them.
#[derive(Clone, Copy, Debug)]
pub struct BoltTarget<'a> {
    pub id: PlayerId,
    pub team: Team,
    pub alive: bool,
    pub feet: Vec3,
    pub eye: Vec3,
    /// What `hitbox::first_hit` uses (head, hands, blaster, yaw).
    pub cmd: &'a PlayerCmd,
}

/// The first thing a bolt meets in a tick's flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Contact {
    Clear,
    /// A player (`index` as given with the targets), at `at`, on `part`.
    Player {
        index: usize,
        at: Vec3,
        part: Part,
    },
    /// A barrier (index into the barriers given).
    Barrier {
        index: usize,
        at: Vec3,
    },
    /// A mirror put up (index into the barriers given), at `at` with normal `n`.
    Mirror {
        index: usize,
        at: Vec3,
        n: Vec3,
    },
    /// The arena (a wall, a mirror, a target panel...), at `q` with normal `n`.
    Arena {
        q: Vec3,
        n: Vec3,
    },
}

/// What bolt `b` meets flying a tick from `b.pos`: the arena, a barrier (its own team's bolts
/// pass) or a player of `targets` (team bolts pass through team mates and their shooter,
/// grenade bolts hit everyone, and a beam passes those it already struck).
pub fn bolt_contact<'a>(
    world: &World,
    barriers: &[Barrier],
    rules: &Rules,
    b: &Bolt,
    targets: impl IntoIterator<Item = (usize, BoltTarget<'a>)>,
) -> Contact {
    let to = b.pos + b.vel * TICK;
    let mut t_best = 1.0f32;
    let mut hit: Option<(Vec3, Vec3)> = None;
    if let Some((t, q, n)) = world.ray(b.pos, to) {
        t_best = t;
        hit = Some((q, n));
    }
    let mut barrier: Option<usize> = None;
    for (k, br) in barriers.iter().enumerate() {
        if b.team == Some(br.team) && !br.mirror {
            continue;
        }
        if let Some(t) = br.crossed(b.pos, to, rules.barrier_w, rules.barrier_h)
            && t < t_best
        {
            t_best = t;
            barrier = Some(k);
        }
    }
    let mut victim: Option<(usize, Part)> = None;
    for (i, p) in targets {
        let passes = !p.alive
            || b.team.is_some() && (p.id == b.owner || b.team == Some(p.team))
            || b.extra.struck & (1u64 << (p.id % 64)) != 0;
        if passes {
            continue;
        }
        if let Some((s, part)) = first_hit(p.feet, p.eye, p.cmd, b.pos, to)
            && s <= t_best
        {
            t_best = s;
            victim = Some((i, part));
        }
    }
    let at = b.pos + (to - b.pos) * t_best;
    if let Some((index, part)) = victim {
        Contact::Player { index, at, part }
    } else if let Some(index) = barrier {
        let br = &barriers[index];
        if br.mirror {
            Contact::Mirror {
                index,
                at,
                n: br.normal,
            }
        } else {
            Contact::Barrier { index, at }
        }
    } else if let Some((q, n)) = hit {
        Contact::Arena { q, n }
    } else {
        Contact::Clear
    }
}

/// Whether `q` (where a bolt struck) is on the box `min`..`max`.
pub fn on_box(q: Vec3, min: Vec3, max: Vec3) -> bool {
    q.cmpge(min - 0.01).all() && q.cmple(max + 0.01).all()
}

/// A bolt that struck the arena at `q` (normal `n`): off a mirror again if it struck one and may
/// still bounce, or off any wall but a scoring panel for a ricochet bolt or in a mirror storm
/// (`storm`; the normal it bounced off), from where it struck.
pub fn bounce(
    level: &Level,
    rules: &Rules,
    b: &mut Bolt,
    q: Vec3,
    n: Vec3,
    storm: bool,
) -> Option<Vec3> {
    let n = if b.extra.ricochet {
        if b.extra.bounces >= rules.ricochet_bounces || level.scores_at(q) {
            return None;
        }
        n
    } else {
        if b.extra.bounces >= rules.mirror_bounces {
            return None;
        }
        match level.mirrors.iter().find(|m| on_box(q, m.min, m.max)) {
            Some(m) => m.normal,
            None if storm && !level.scores_at(q) => n,
            None => return None,
        }
    };
    b.vel -= 2.0 * b.vel.dot(n) * n;
    // (The flight moves it on by a tick's worth after this.)
    b.pos = q + n * 0.02 - b.vel * TICK;
    b.extra.bounces += 1;
    Some(n)
}

/// Bolts a mirror put up turns back: up to this many bounces in all.
pub const MIRROR_MAX_BOUNCES: u8 = 4;

/// A bolt that met a mirror put up at `at` (normal `n`): off it again unless it has bounced
/// too often (the normal facing it).
pub fn bounce_placed(b: &mut Bolt, at: Vec3, n: Vec3) -> Option<Vec3> {
    if b.extra.bounces >= MIRROR_MAX_BOUNCES {
        return None;
    }
    let n = if b.vel.dot(n) > 0.0 { -n } else { n };
    b.vel -= 2.0 * b.vel.dot(n) * n;
    b.pos = at + n * 0.02 - b.vel * TICK;
    b.extra.bounces += 1;
    Some(n)
}

/// The first mirror put up that the segment `a`..`b` crosses (of `barriers`): where along it
/// (0..1), and its index.
pub fn placed_mirror(
    barriers: &[Barrier],
    rules: &Rules,
    a: Vec3,
    b: Vec3,
) -> Option<(f32, usize)> {
    barriers
        .iter()
        .enumerate()
        .filter(|(_, br)| br.mirror)
        .filter_map(|(k, br)| Some((br.crossed(a, b, rules.barrier_w, rules.barrier_h)?, k)))
        .min_by(|x, y| x.0.total_cmp(&y.0))
}

/// A remote client's word that one of its bolts hit a player, as it showed them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HitClaim {
    /// The shot (`PlayerCmd::shot`) and which of its bolts (the shotgun's pellets).
    pub shot: u32,
    pub pellet: u8,
    pub victim: PlayerId,
    /// The tick the client showed when the bolt met them (fractional: between snapshots).
    pub view: f32,
    /// Where it met them.
    pub at: Vec3,
    /// It met their head.
    #[serde(default)]
    pub head: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(rules: &Rules) -> Gun {
        Gun {
            ammo: rules.ammo,
            ..Default::default()
        }
    }

    /// Ticks (fire held, then let go) until a shot, at most `n`: the tick it fired on.
    fn ticks_to_fire(g: &mut Gun, rules: &Rules, n: usize) -> Option<usize> {
        for k in 1..=n {
            g.tick();
            if let Some(Trigger::Fire(s)) = g.trigger(k == 1, true, rules) {
                g.fired(s, rules);
                return Some(k);
            }
        }
        None
    }

    #[test]
    fn a_pull_just_too_early_fires_once_ready() {
        let rules = Rules::default();
        let mut g = ready(&rules);
        assert_eq!(
            g.trigger(true, true, &rules),
            Some(Trigger::Fire(Shot::Bolt))
        );
        g.fired(Shot::Bolt, &rules);
        g.trigger(false, true, &rules);
        // Up to 3 ticks short of ready: fires on the tick it's ready.
        let left = (rules.fire_cooldown / TICK).round() as usize;
        for _ in 0..left - 3 - 1 {
            g.tick();
            g.trigger(false, true, &rules);
        }
        assert_eq!(ticks_to_fire(&mut g, &rules, 10), Some(4));
    }

    #[test]
    fn a_pull_well_too_early_is_ignored() {
        let rules = Rules::default();
        let mut g = ready(&rules);
        g.trigger(true, true, &rules);
        g.fired(Shot::Bolt, &rules);
        g.trigger(false, true, &rules);
        assert_eq!(ticks_to_fire(&mut g, &rules, 60), None);
    }

    #[test]
    fn a_hit_drops_a_waiting_pull() {
        let rules = Rules::default();
        let mut g = ready(&rules);
        g.trigger(true, true, &rules);
        g.fired(Shot::Bolt, &rules);
        g.trigger(false, true, &rules);
        g.fire_cd = 0.05;
        g.trigger(true, true, &rules);
        assert!(g.queued);
        g.cooldown = 3.0;
        g.tick();
        assert_eq!(g.trigger(true, true, &rules), None);
        assert!(!g.queued);
    }
}

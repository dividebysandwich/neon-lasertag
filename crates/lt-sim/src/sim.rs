//! The authoritative game state, stepped at a fixed `TICK`.
//!
//! **Lag compensation.** A remote player aims at the world as their client showed it, a
//! little in the past (network delay plus interpolation). The sim keeps every player's
//! hit-relevant state for the last `HISTORY` ticks; a bolt fired with a `PlayerCmd::view_tick`
//! is flown at once through the ticks since then (at most `MAX_LAG`), hitting players where
//! they were at the ticks the shooter saw and the arena as it is now, and then flies on
//! normally. Grenade bolts are not compensated.

use std::sync::Arc;

use glam::{Quat, Vec2, Vec3};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::collide::{BODY_HEIGHT, BODY_RADIUS, GRAVITY, World};
use crate::hitbox::first_hit;
use crate::level::{Ground, Level, MoverKind, Panel};
use crate::rules::Rules;
use crate::types::*;

pub const TICK: f32 = 1.0 / 60.0;
pub const MAX_PLAYERS: usize = 32;
/// How close (horizontally) a player has to come to take a grenade pickup.
pub const PICKUP_REACH: f32 = 0.9;
/// Pickups hover this high above the floor.
pub const PICKUP_HEIGHT: f32 = 0.6;
/// Default standing eye height.
pub const EYE: f32 = 1.65;
/// Ticks of player history kept for lag compensation, and the most a bolt is compensated
/// (250 ms).
/// Seconds a turret waits before its first shot at a player it has just found, or after it
/// comes back on.
pub const TURRET_WARMUP: f32 = 0.5;
pub const HISTORY: usize = 20;
/// A cloaked player can't be seen from further than `CLOAK_FAR` (m), and is see-through, fading
/// in to `CLOAK_NEAR` (the bots and turrets see one closer than `CLOAK_NEAR`).
pub const CLOAK_NEAR: f32 = 3.0;
pub const CLOAK_FAR: f32 = 7.0;
pub const MAX_LAG: u64 = 15;

/// A player as a bolt could hit them at a past tick.
#[derive(Clone, Copy, Debug)]
struct Ghost {
    id: PlayerId,
    alive: bool,
    feet: Vec3,
    eye: Vec3,
    /// The parts of the command `hitbox::first_hit` uses (head, hands, blaster, yaw).
    cmd: PlayerCmd,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub team: Team,
    pub bot: bool,
    pub feet: Vec3,
    pub vy: f32,
    /// Flying from a jump pad: the horizontal velocity it gave (zero on the ground).
    pub air: Vec2,
    pub cmd: PlayerCmd,
    prev_fire: bool,
    /// Hits taken since the last respawn.
    pub hits: u8,
    /// Time left of the blaster's lockout after a hit (no hits taken meanwhile either).
    pub cooldown: f32,
    pub fire_cd: f32,
    /// Waiting to respawn, seconds left.
    pub respawn: Option<f32>,
    pub grenades: u8,
    /// Shots left in the blaster.
    pub ammo: u16,
    /// Refilling at a reload station or spawn pad (fractional shots so far).
    pub reloading: Option<f32>,
    pub score: u32,
    /// Enemy players this player tagged out.
    pub tags: u32,
    /// Times this player was tagged out.
    pub tagged: u32,
    /// Bolts fired (match statistics).
    pub shots: u32,
    /// Bolts that hit an enemy player and counted (not during the victim's cooldown).
    pub landed: u32,
    /// Hits on the enemy team's targets.
    pub target_hits: u32,
    /// Assists (an enemy this player hit tagged out by a team mate soon after), defends (an
    /// enemy tagged out near one's own targets), and tag-outs with a grenade and while
    /// cloaked.
    pub assists: u32,
    pub defends: u32,
    pub grenade_tags: u32,
    pub cloak_tags: u32,
    /// The enemies who hit this player since they were last unhurt, and the tick of each one's
    /// last hit.
    pub hit_by: Vec<(PlayerId, u64)>,
    /// Ticks to ignore predicted positions (just respawned: the client hasn't seen it yet).
    pub claim_block: u32,
    /// Megatarget mode: the secondary targets hit since starting over (bit `side`), and once
    /// both are, the seconds left to get into the megatarget's room.
    pub mega_hits: u8,
    pub power: Option<f32>,
    /// Powerups: rapid fire (until tagged out), shields carried, and the seconds left of the
    /// shield in use (0: none).
    pub rapid: bool,
    pub shields: u8,
    pub shield: f32,
    /// Holding a shotgun instead of the blaster (until tagged out).
    pub shotgun: bool,
    /// Seconds left of the cloak (0: none), and of showing after firing or a hit.
    pub cloak: f32,
    pub decloak: f32,
    /// Seconds left of the speed boost (0: none).
    pub speed: f32,
    /// Holding the charge gun instead of the blaster (until tagged out), and how long its
    /// trigger has been held (while it charges).
    pub charge: bool,
    pub charging: Option<f32>,
    /// Barriers carried.
    pub barriers: u8,
    prev_barrier: bool,
}

impl Player {
    /// Cloaked and not showing right now.
    pub fn cloaked(&self) -> bool {
        self.cloak > 0.0 && self.decloak <= 0.0
    }

    /// Whether the bots and the turrets see this player from `dist` (m) away.
    pub fn seen_from(&self, dist: f32) -> bool {
        !self.cloaked() || dist < CLOAK_NEAR
    }

    pub fn alive(&self) -> bool {
        self.respawn.is_none()
    }

    /// Least time between this player's shots.
    pub fn fire_interval(&self, rules: &Rules) -> f32 {
        if self.rapid {
            rules.fire_cooldown * 0.5
        } else {
            rules.fire_cooldown
        }
    }

    /// How fast the stick moves this player (m/s).
    pub fn move_speed(&self, rules: &Rules) -> f32 {
        rules.move_speed
            * if self.speed > 0.0 {
                rules.speed_factor
            } else {
                1.0
            }
    }

    /// Whether walking over a pickup of this kind would take it.
    pub fn wants(&self, kind: PickupKind, rules: &Rules) -> bool {
        match kind {
            PickupKind::Grenade => self.grenades < rules.grenade_max,
            PickupKind::Rapid => !self.rapid,
            PickupKind::Shield => self.shields == 0,
            // (One special gun at a time: the other stays where it is until this one is lost
            // with a tag-out; one's own refills it.)
            PickupKind::Shotgun => !self.charge && (!self.shotgun || self.ammo < rules.ammo),
            PickupKind::Charge => !self.shotgun && (!self.charge || self.ammo < rules.ammo),
            PickupKind::Cloak => self.cloak <= 0.0,
            PickupKind::Speed => self.speed <= 0.0,
            PickupKind::Barrier => self.barriers == 0,
        }
    }

    /// Gives this player what a pickup of this kind holds.
    fn take(&mut self, kind: PickupKind, rules: &Rules) {
        match kind {
            PickupKind::Grenade => self.grenades += 1,
            PickupKind::Rapid => self.rapid = true,
            PickupKind::Shield => self.shields += 1,
            PickupKind::Shotgun | PickupKind::Charge => {
                self.shotgun = kind == PickupKind::Shotgun;
                self.charge = kind == PickupKind::Charge;
                self.charging = None;
                self.ammo = rules.ammo;
                self.reloading = None;
            }
            PickupKind::Cloak => self.cloak = rules.cloak_time,
            PickupKind::Speed => self.speed = rules.speed_time,
            PickupKind::Barrier => self.barriers += 1,
        }
    }

    /// Tagged out: the powerups are gone.
    fn lose_powerups(&mut self) {
        self.rapid = false;
        self.shields = 0;
        self.shield = 0.0;
        self.shotgun = false;
        self.cloak = 0.0;
        self.decloak = 0.0;
        self.speed = 0.0;
        self.charge = false;
        self.charging = None;
        self.barriers = 0;
    }

    pub fn eye(&self) -> Vec3 {
        self.feet + self.head_offset()
    }

    fn head_offset(&self) -> Vec3 {
        let h = self.cmd.head.pos;
        if h == Vec3::ZERO {
            Vec3::Y * EYE
        } else {
            h.clamp(Vec3::new(-0.6, 0.4, -0.6), Vec3::new(0.6, 2.2, 0.6))
        }
    }

    /// The middle of the floating chest (bots aim here).
    pub fn chest(&self) -> Vec3 {
        self.eye() - Vec3::Y * (crate::hitbox::CHEST_DROP + crate::hitbox::CHEST_HALF.y)
    }

    /// Where along the segment `a`..`b` (0..1) it first hits this player, and what it hits.
    pub fn hit_by(&self, a: Vec3, b: Vec3) -> Option<(f32, crate::hitbox::Part)> {
        first_hit(self.feet, self.eye(), &self.cmd, a, b)
    }
}

/// A Megatarget turret's state.
#[derive(Clone, Copy, Debug)]
pub struct TurretState {
    /// Where its gun points.
    pub aim: Vec3,
    /// Seconds left switched off.
    pub off: f32,
    fire_t: f32,
    /// Has a player in its sights.
    engaged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurretSnap {
    pub aim: Vec3,
    pub off: f32,
}

/// A grenade or a powerup lying in the arena for anyone to take.
#[derive(Clone, Copy, Debug)]
pub struct Pickup {
    pub id: u32,
    pub pos: Vec3,
    pub kind: PickupKind,
}

/// A place for pickups of a kind: there's a pickup there, or one will appear somewhere in
/// `half` after `wait` seconds.
#[derive(Clone, Copy, Debug)]
struct PickupSlot {
    half: Team,
    kind: PickupKind,
    pickup: Option<Pickup>,
    wait: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Bolt {
    pub id: u32,
    pub owner: PlayerId,
    pub owner_team: Team,
    /// The bolt's colour: the shooter's team, or `None` (white) from a grenade.
    pub team: Option<Team>,
    pub pos: Vec3,
    pub vel: Vec3,
    pub life: f32,
    pub extra: BoltExtra,
}

/// What some bolts do besides one hit: bounce off mirrors, hit harder, fly through players.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoltExtra {
    /// Mirrors bounced off so far.
    pub bounces: u8,
    /// Hits it deals at once (the charge gun's beam: `Rules::beam_damage`).
    pub damage: u8,
    /// Flies on through the players it hits (each hit once: `struck`, by player id bit).
    pub pierce: bool,
    pub struck: u64,
}

impl Default for BoltExtra {
    fn default() -> Self {
        BoltExtra {
            bounces: 0,
            damage: 1,
            pierce: false,
            struck: 0,
        }
    }
}

/// A barrier: a wall of hard light standing on the floor (`centre` at half its height),
/// facing `normal` (the way its player looked from, horizontal).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Barrier {
    pub id: u32,
    pub team: Team,
    pub centre: Vec3,
    pub normal: Vec3,
    /// Seconds left standing.
    pub left: f32,
}

impl Barrier {
    /// Where along `a`..`b` (0..1) it crosses this barrier of `w` x `h` metres, if it does.
    pub fn crossed(&self, a: Vec3, b: Vec3, w: f32, h: f32) -> Option<f32> {
        let d = b - a;
        let denom = d.dot(self.normal);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (self.centre - a).dot(self.normal) / denom;
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        let x = a + d * t - self.centre;
        let along = Vec3::Y.cross(self.normal);
        (x.dot(along).abs() <= w / 2.0 && x.y.abs() <= h / 2.0).then_some(t)
    }
}

/// What a trigger pull fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shot {
    Bolt,
    Spread,
    Beam,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum GrenadePhase {
    Flying,
    /// Hovering between `from` and `to` (`hop` above where it landed).
    Active {
        from: Vec3,
        to: Vec3,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Grenade {
    pub id: u32,
    pub owner: PlayerId,
    pub team: Team,
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub spin: f32,
    pub phase: GrenadePhase,
    next_burst: f32,
}

/// Everything a client needs to draw a moment of the game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub time_left: f32,
    pub scores: [u32; 2],
    pub players: Vec<PlayerSnap>,
    pub bolts: Vec<BoltSnap>,
    pub grenades: Vec<GrenadeSnap>,
    pub pickups: Vec<PickupSnap>,
    pub over: bool,
    /// Megatarget mode's turrets (as `Mega::turrets`).
    pub turrets: Vec<TurretSnap>,
    /// Barriers standing.
    pub barriers: Vec<Barrier>,
    /// The arena event going on (seconds left), and whether points count double.
    pub event: Option<(ArenaEvent, f32)>,
    pub double: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickupSnap {
    pub id: u32,
    pub pos: Vec3,
    pub kind: PickupKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnap {
    pub id: PlayerId,
    pub team: Team,
    pub feet: Vec3,
    pub yaw: f32,
    pub head: Pose,
    pub hands: [Pose; 2],
    pub gun_hand: Option<Hand>,
    pub grenade_hand: Option<Hand>,
    pub hits: u8,
    pub cooldown: f32,
    pub respawn: Option<f32>,
    pub grenades: u8,
    pub ammo: u16,
    pub reloading: bool,
    pub score: u32,
    /// Match statistics: enemies tagged out, times tagged out, bolts fired, bolts that hit an
    /// enemy and counted, enemy target hits.
    pub tags: u32,
    pub tagged: u32,
    pub shots: u32,
    pub landed: u32,
    pub target_hits: u32,
    /// `Player::assists`, `defends`, `grenade_tags`, `cloak_tags`.
    pub assists: u32,
    pub defends: u32,
    pub grenade_tags: u32,
    pub cloak_tags: u32,
    /// Megatarget mode (`Player::mega_hits`, `Player::power`).
    pub mega_hits: u8,
    pub power: Option<f32>,
    /// Powerups (`Player::rapid`, `shields`, `shield`, `shotgun`).
    pub rapid: bool,
    pub shields: u8,
    pub shield: f32,
    pub shotgun: bool,
    /// `Player::cloak`, `decloak`.
    pub cloak: f32,
    pub decloak: f32,
    /// `Player::speed`.
    pub speed: f32,
    /// `Player::charge`, and `Player::charging` (0: not charging).
    pub charge: bool,
    pub charging: f32,
    /// `Player::barriers`.
    pub barriers: u8,
}

impl PlayerSnap {
    /// Whether the player has this powerup now (carried or in use).
    pub fn has(&self, kind: PickupKind) -> bool {
        match kind {
            PickupKind::Grenade => self.grenades > 0,
            PickupKind::Rapid => self.rapid,
            PickupKind::Shield => self.shields > 0 || self.shield > 0.0,
            PickupKind::Shotgun => self.shotgun,
            PickupKind::Cloak => self.cloak > 0.0,
            PickupKind::Speed => self.speed > 0.0,
            PickupKind::Charge => self.charge,
            PickupKind::Barrier => self.barriers > 0,
        }
    }

    /// Cloaked and not showing right now (`Player::cloaked`).
    pub fn cloaked(&self) -> bool {
        self.cloak > 0.0 && self.decloak <= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoltSnap {
    pub id: u32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub team: Option<Team>,
    /// A charge gun's beam.
    pub beam: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrenadeSnap {
    pub id: u32,
    pub pos: Vec3,
    pub spin: f32,
    pub active: bool,
}

pub struct Sim {
    pub rules: Rules,
    pub level: Arc<Level>,
    pub world: World,
    pub players: Vec<Player>,
    pub bolts: Vec<Bolt>,
    pub grenades: Vec<Grenade>,
    slots: Vec<PickupSlot>,
    pub scores: [u32; 2],
    pub time_left: f32,
    pub tick: u64,
    pub over: bool,
    /// Per target (by id): the tick and shooter of its last hit (the bots' alarm; events are
    /// drained by the caller, so bots can't see them).
    pub last_target_hit: Vec<Option<(u64, PlayerId)>>,
    pub turrets: Vec<TurretState>,
    /// Barriers standing.
    pub barriers: Vec<Barrier>,
    /// The arena event going on, with its seconds left; when the next one comes (seconds into
    /// the match); and whether points count double now.
    pub event: Option<(ArenaEvent, f32)>,
    pub next_event: f32,
    pub double: bool,
    /// Every player's hit-relevant state at the end of each of the last `HISTORY` ticks.
    history: std::collections::VecDeque<(u64, Vec<Ghost>)>,
    events: Vec<SimEvent>,
    rng: ChaCha8Rng,
    next_id: u32,
    /// Distance from a base to the far end, for the respawn time.
    max_dist: f32,
}

impl Sim {
    pub fn new(seed: u64, rules: Rules) -> Sim {
        Self::with_level(
            Arc::new(Level::generate_for(seed, rules.arena, rules.mode)),
            rules,
        )
    }

    pub fn with_level(level: Arc<Level>, rules: Rules) -> Sim {
        let world = level.world_at(0);
        let max_dist = (level.base_centre(Team::Red) - level.base_centre(Team::Blue))
            .length()
            .max(1.0);
        let grenades = (0..rules.pickup_pairs as usize * 2).map(|i| PickupSlot {
            half: if i % 2 == 0 { Team::Red } else { Team::Blue },
            kind: PickupKind::Grenade,
            pickup: None,
            wait: 1.0 + (i / 2) as f32 * 2.0,
        });
        // Powerups come a little later, one kind after the other.
        let powerups = PickupKind::POWERUPS
            .iter()
            .enumerate()
            .flat_map(|(k, &kind)| {
                (0..rules.powerup_pairs as usize * 2).map(move |i| PickupSlot {
                    half: if i % 2 == 0 { Team::Red } else { Team::Blue },
                    kind,
                    pickup: None,
                    wait: 6.0 + k as f32 * 3.0 + (i / 2) as f32 * 10.0,
                })
            });
        let slots = grenades.chain(powerups).collect();
        let mut sim = Sim {
            time_left: rules.match_time,
            rules,
            world,
            players: Vec::new(),
            bolts: Vec::new(),
            grenades: Vec::new(),
            slots,
            scores: [0; 2],
            tick: 0,
            over: false,
            last_target_hit: vec![None; level.targets.len()],
            turrets: level.mega.as_ref().map_or(Vec::new(), |m| {
                m.turrets
                    .iter()
                    .map(|t| TurretState {
                        aim: t.facing,
                        off: 0.0,
                        fire_t: 1.0,
                        engaged: false,
                    })
                    .collect()
            }),
            barriers: Vec::new(),
            event: None,
            next_event: 0.0,
            double: false,
            history: std::collections::VecDeque::with_capacity(HISTORY + 1),
            events: Vec::new(),
            rng: ChaCha8Rng::seed_from_u64(level.seed ^ 0x5EED),
            next_id: 1,
            max_dist,
            level,
        };
        let (lo, hi) = sim.rules.event_first;
        sim.next_event = sim.rng.random_range(lo..=hi.max(lo));
        sim
    }

    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    fn player_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Joins a player (on the smaller team unless `team` is given); `None` if full.
    pub fn add_player(&mut self, name: &str, team: Option<Team>, bot: bool) -> Option<PlayerId> {
        if self.players.len() >= MAX_PLAYERS {
            return None;
        }
        let id = (0..=255u8).find(|i| self.player(*i).is_none())?;
        let team = team.unwrap_or_else(|| {
            let red = self.players.iter().filter(|p| p.team == Team::Red).count();
            let blue = self.players.len() - red;
            if red <= blue { Team::Red } else { Team::Blue }
        });
        let mut p = Player {
            id,
            name: name.chars().take(24).collect(),
            team,
            bot,
            feet: Vec3::ZERO,
            vy: 0.0,
            air: Vec2::ZERO,
            cmd: PlayerCmd::default(),
            prev_fire: false,
            hits: 0,
            cooldown: 0.0,
            fire_cd: 0.0,
            respawn: None,
            grenades: 0,
            ammo: self.rules.ammo,
            reloading: None,
            score: 0,
            tags: 0,
            tagged: 0,
            shots: 0,
            landed: 0,
            target_hits: 0,
            assists: 0,
            defends: 0,
            grenade_tags: 0,
            cloak_tags: 0,
            hit_by: Vec::new(),
            claim_block: 0,
            mega_hits: 0,
            power: None,
            rapid: false,
            shields: 0,
            shield: 0.0,
            shotgun: false,
            cloak: 0.0,
            decloak: 0.0,
            speed: 0.0,
            charge: false,
            charging: None,
            barriers: 0,
            prev_barrier: false,
        };
        let spawn = self.pick_spawn(team);
        p.feet = spawn.pos;
        p.cmd.yaw = spawn.yaw;
        self.players.push(p);
        Some(id)
    }

    pub fn remove_player(&mut self, id: PlayerId) {
        self.players.retain(|p| p.id != id);
    }

    /// The latest command of a player, applied by every `step` until replaced.
    pub fn set_cmd(&mut self, id: PlayerId, cmd: PlayerCmd) {
        if let Some(p) = self.player_mut(id) {
            // A throw stays until a step uses it.
            let throw = cmd.throw.or(p.cmd.throw);
            p.cmd = cmd;
            p.cmd.throw = throw;
        }
    }

    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    fn pick_spawn(&mut self, team: Team) -> crate::level::Spawn {
        let spawns = &self.level.spawns[team.index()];
        // The pad furthest from everyone else.
        let mut best = spawns[0];
        let mut best_d = -1.0;
        for s in spawns {
            let d = self
                .players
                .iter()
                .filter(|p| p.alive())
                .map(|p| (p.feet - s.pos).length_squared())
                .fold(f32::MAX, f32::min);
            let d = d + self.rng.random::<f32>() * 0.5;
            if d > best_d {
                best_d = d;
                best = *s;
            }
        }
        best
    }

    pub fn respawn_time(&self, team: Team, at: Vec3) -> f32 {
        let d = (at - self.level.base_centre(team)).length();
        let t = (d / self.max_dist).clamp(0.0, 1.0);
        self.rules.respawn_min + (self.rules.respawn_max - self.rules.respawn_min) * t
    }

    fn id(&mut self) -> u32 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }

    pub fn step(&mut self) {
        self.tick += 1;
        if self.over {
            return;
        }
        self.time_left -= TICK;
        if self.time_left <= 0.0 {
            self.time_left = 0.0;
            self.over = true;
            self.events.push(SimEvent::MatchOver {
                scores: self.scores,
            });
            return;
        }
        self.step_events();
        self.step_movers();
        self.step_players();
        self.step_gates();
        self.step_mega();
        self.step_bolts();
        self.step_grenades();
        self.step_barriers();
        self.step_pickups();
        self.record();
    }

    /// Remembers where everyone is at the end of this tick (for lag compensation).
    fn record(&mut self) {
        let ghosts = self
            .players
            .iter()
            .map(|p| Ghost {
                id: p.id,
                alive: p.alive(),
                feet: p.feet,
                eye: p.eye(),
                cmd: p.cmd,
            })
            .collect();
        if self.history.len() >= HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((self.tick, ghosts));
    }

    /// The recorded players at the end of `tick`, if still kept.
    fn past(&self, tick: u64) -> Option<usize> {
        let first = self.history.front()?.0;
        let i = tick.checked_sub(first)? as usize;
        (i < self.history.len() && self.history[i].0 == tick).then_some(i)
    }

    /// The pickups (grenades and powerups) lying in the arena.
    pub fn pickups(&self) -> impl Iterator<Item = &Pickup> {
        self.slots.iter().filter_map(|s| s.pickup.as_ref())
    }

    /// A random spot for a pickup in `half`: a walkable cell (either floor) between the base and
    /// the centre, not on a ramp or a station, away from other pickups.
    fn pickup_spot(&mut self, half: Team) -> Option<Vec3> {
        let nav = &self.level.nav;
        let grid = &self.level.grid;
        let dm = self.level.dims();
        for _ in 0..60 {
            let ix = self.rng.random_range(0..dm.w);
            let iz = self.rng.random_range(dm.half()..dm.h - 3);
            let iz = if half == Team::Red { iz } else { dm.h - 1 - iz };
            let upper = self.rng.random_bool(0.35);
            let n = if upper {
                nav.upper_node(ix, iz)
            } else {
                nav.ground_node(ix, iz)
            };
            if !nav.valid[n]
                || self
                    .level
                    .mega
                    .as_ref()
                    .is_some_and(|m| m.in_room(nav.pos[n]))
                || !upper
                    && (grid.ground[dm.idx(ix, iz)] != Ground::Open || self.level.on_track(ix, iz))
            {
                continue;
            }
            let pos = nav.pos[n] + Vec3::Y * PICKUP_HEIGHT;
            if self.level.pad_at(nav.pos[n]).is_some() {
                continue;
            }
            if self
                .slots
                .iter()
                .filter_map(|s| s.pickup)
                .any(|p| (p.pos - pos).length() < 6.0)
            {
                continue;
            }
            return Some(pos);
        }
        None
    }

    /// Moves the doors and panels to where they are this tick. A player in a sliding panel's
    /// way is pushed ahead of it (the walls still stopping them; whatever overlap is left, the
    /// player's own move resolves sideways). Doors rising under a player push them out of the
    /// doorway in the player's move.
    fn step_movers(&mut self) {
        if self.level.movers.is_empty() {
            return;
        }
        let level = self.level.clone();
        let tick = self.tick;
        let all = level.mover_colliders(tick);
        for (i, m) in level.movers.iter().enumerate() {
            if m.kind != MoverKind::Panel {
                continue;
            }
            let d = m.offset(tick) - m.offset(tick.saturating_sub(1));
            let dir = Vec2::new(d.x, d.z);
            if dir.length_squared() < 1e-10 {
                continue;
            }
            let dir = dir.normalize();
            let mine: Vec<_> = m.colliders(m.fraction(tick)).collect();
            // Pushed through a world without this panel (with it, the push would be undone).
            let others: Vec<_> = level
                .movers
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .flat_map(|(_, o)| o.colliders(o.fraction(tick)))
                .collect();
            self.world.set_dynamic(others);
            for p in self.players.iter_mut().filter(|p| p.alive()) {
                for c in &mine {
                    let (min, max) = c.bounds();
                    if p.feet.y >= max.y || p.feet.y + BODY_HEIGHT <= min.y {
                        continue;
                    }
                    let (nx, nz) = (p.feet.x.clamp(min.x, max.x), p.feet.z.clamp(min.z, max.z));
                    if Vec2::new(p.feet.x - nx, p.feet.z - nz).length_squared()
                        >= BODY_RADIUS * BODY_RADIUS
                    {
                        continue;
                    }
                    // How far along the motion until the body clears the box.
                    let need = if dir.x.abs() > dir.y.abs() {
                        if dir.x > 0.0 {
                            max.x + BODY_RADIUS - p.feet.x
                        } else {
                            p.feet.x - (min.x - BODY_RADIUS)
                        }
                    } else if dir.y > 0.0 {
                        max.z + BODY_RADIUS - p.feet.z
                    } else {
                        p.feet.z - (min.z - BODY_RADIUS)
                    };
                    if need > 0.0 {
                        let (feet, vy) =
                            self.world.move_body(p.feet, dir * (need + 0.01), p.vy, 0.0);
                        p.feet = feet;
                        p.vy = vy;
                    }
                }
            }
        }
        self.world.set_dynamic(all);
    }

    fn step_pickups(&mut self) {
        for i in 0..self.slots.len() {
            match self.slots[i].pickup {
                Some(pk) => {
                    let rules = &self.rules;
                    let taker = self.players.iter_mut().find(|p| {
                        p.alive()
                            && p.wants(pk.kind, rules)
                            && Vec2::new(p.feet.x - pk.pos.x, p.feet.z - pk.pos.z).length()
                                < PICKUP_REACH
                            && (pk.pos.y - p.feet.y).abs() < 1.6
                    });
                    if let Some(p) = taker {
                        p.take(pk.kind, rules);
                        let player = p.id;
                        self.events.push(SimEvent::PickedUp {
                            id: pk.id,
                            player,
                            pos: pk.pos,
                            kind: pk.kind,
                        });
                        let (lo, hi) = if pk.kind == PickupKind::Grenade {
                            self.rules.pickup_respawn
                        } else {
                            self.rules.powerup_respawn
                        };
                        self.slots[i].pickup = None;
                        self.slots[i].wait = self.rng.random_range(lo..=hi.max(lo));
                    }
                }
                None => {
                    self.slots[i].wait -= TICK;
                    if self.slots[i].wait <= 0.0 {
                        let (half, kind) = (self.slots[i].half, self.slots[i].kind);
                        match self.pickup_spot(half) {
                            Some(pos) => {
                                let id = self.id();
                                self.slots[i].pickup = Some(Pickup { id, pos, kind });
                                self.events.push(SimEvent::PickupSpawned { id, pos, kind });
                            }
                            None => self.slots[i].wait = 1.0,
                        }
                    }
                }
            }
        }
    }

    fn step_players(&mut self) {
        let rules = self.rules.clone();
        for i in 0..self.players.len() {
            // Timers.
            let respawn_now = {
                let p = &mut self.players[i];
                p.cooldown = (p.cooldown - TICK).max(0.0);
                // (Snapped to 0 within a rounding error of the last tick, so an interval of
                // whole ticks is just that.)
                p.fire_cd = if p.fire_cd > TICK + 1e-5 {
                    p.fire_cd - TICK
                } else {
                    0.0
                };
                match p.respawn.as_mut() {
                    Some(t) => {
                        *t -= TICK;
                        *t <= 0.0
                    }
                    None => false,
                }
            };
            if respawn_now {
                let team = self.players[i].team;
                let s = self.pick_spawn(team);
                let p = &mut self.players[i];
                p.respawn = None;
                p.hits = 0;
                p.cooldown = 0.0;
                p.ammo = rules.ammo;
                p.grenades = 0;
                p.lose_powerups();
                p.reloading = None;
                p.feet = s.pos;
                p.vy = 0.0;
                p.air = Vec2::ZERO;
                p.cmd.yaw = s.yaw;
                p.claim_block = 30;
                self.events.push(SimEvent::Respawned {
                    player: p.id,
                    pos: s.pos,
                });
            }
            if !self.players[i].alive() {
                let p = &mut self.players[i];
                p.cmd.throw = None;
                p.prev_fire = p.cmd.fire;
                continue;
            }

            // Movement.
            self.players[i].claim_block = self.players[i].claim_block.saturating_sub(1);
            let (feet, vy) = {
                let p = &self.players[i];
                let d = match p.cmd.feet {
                    Some(c) if c.is_finite() && p.claim_block == 0 => {
                        // A predicted position: as far as the speed limit allows (with slack for
                        // ticks arriving unevenly).
                        let max = (p.move_speed(&rules) + rules.walk_speed_max) * TICK * 2.5;
                        Vec2::new(c.x - p.feet.x, c.z - p.feet.z).clamp_length_max(max)
                    }
                    _ => {
                        let stick = p.cmd.stick.clamp_length_max(1.0)
                            * p.move_speed(&rules)
                            * TICK
                            * air_control(p.air);
                        let walk = p.cmd.walk.clamp_length_max(rules.walk_speed_max * TICK);
                        stick + walk + p.air * TICK
                    }
                };
                let d = if d.is_finite() { d } else { Vec2::ZERO };
                self.world.move_body(p.feet, d, p.vy, TICK)
            };
            let reload_here = self.level.reloads_at(feet);
            let home = self.level.spawn_at(feet);
            let p = &mut self.players[i];
            p.feet = feet;
            p.vy = vy;
            // Jump pads: down again, and thrown up.
            if let Some((vy, air)) = pad_step(&self.level, feet, vy, &mut p.air) {
                p.vy = vy;
                p.air = air;
                self.events.push(SimEvent::Launched {
                    player: p.id,
                    pos: feet,
                });
            }

            // Healed on the team's own spawn pads.
            if p.hits > 0 && home == Some(p.team) {
                p.hits = 0;
                p.hit_by.clear();
                self.events.push(SimEvent::Healed { player: p.id });
            }

            // Refilling the blaster at a reload station or a spawn pad.
            if reload_here && p.ammo < rules.ammo {
                let acc = match p.reloading {
                    Some(a) => a,
                    None => {
                        self.events.push(SimEvent::Reloading { player: p.id });
                        0.0
                    }
                } + rules.reload_rate * TICK;
                let whole = acc.floor();
                p.ammo = (p.ammo as u32 + whole as u32).min(rules.ammo as u32) as u16;
                p.reloading = Some(acc - whole);
                if p.ammo >= rules.ammo {
                    p.reloading = None;
                    self.events.push(SimEvent::Reloaded { player: p.id });
                }
            } else {
                p.reloading = None;
            }

            // The shield: switched on, running out.
            if p.cmd.shield && p.shields > 0 && p.shield <= 0.0 {
                p.shields -= 1;
                p.shield = rules.shield_time;
                self.events.push(SimEvent::ShieldUp { player: p.id });
            } else if p.shield > 0.0 {
                p.shield -= TICK;
                if p.shield <= 0.0 {
                    p.shield = 0.0;
                    self.events.push(SimEvent::ShieldDown { player: p.id });
                }
            }

            // Putting up a carried barrier (on pressing).
            let deploy = p.cmd.barrier && !p.prev_barrier && p.barriers > 0;
            p.prev_barrier = p.cmd.barrier;
            if deploy {
                p.barriers -= 1;
                let id = self.id();
                let b = Barrier {
                    id,
                    ..self.barrier_for(i)
                };
                self.barriers.push(b);
                self.events.push(SimEvent::BarrierUp {
                    id,
                    player: self.players[i].id,
                    pos: b.centre,
                });
            }
            let p = &mut self.players[i];

            // The speed boost running out.
            if p.speed > 0.0 {
                p.speed -= TICK;
                if p.speed <= 0.0 {
                    p.speed = 0.0;
                    self.events.push(SimEvent::SpeedDown { player: p.id });
                }
            }

            // The cloak running out.
            p.decloak = (p.decloak - TICK).max(0.0);
            if p.cloak > 0.0 {
                p.cloak -= TICK;
                if p.cloak <= 0.0 {
                    p.cloak = 0.0;
                    self.events.push(SimEvent::Uncloaked { player: p.id });
                }
            }

            // Firing: one shot per trigger pull; the charge gun fires on letting go (a beam once
            // held for `Rules::charge_time`), and a hit or holstering stops its charging.
            let pull = p.cmd.fire && !p.prev_fire;
            p.prev_fire = p.cmd.fire;
            if p.cmd.gun_hand.is_none() || p.cooldown > 0.0 {
                p.charging = None;
            }
            let mut shot = None;
            if pull && p.cmd.gun_hand.is_some() {
                if p.cooldown > 0.0 {
                    self.events.push(SimEvent::Denied { player: p.id });
                } else if p.fire_cd > 0.0 {
                    // Faster than the blaster's rate: ignored.
                } else if p.ammo == 0 {
                    p.fire_cd = p.fire_interval(&rules);
                    self.events.push(SimEvent::Empty { player: p.id });
                } else if p.charge {
                    p.charging = Some(0.0);
                } else if p.shotgun {
                    shot = Some(Shot::Spread);
                } else {
                    shot = Some(Shot::Bolt);
                }
            }
            if let Some(t) = p.charging {
                if p.cmd.fire {
                    let held = t + TICK;
                    p.charging = Some(held);
                    if t < rules.charge_time && held >= rules.charge_time {
                        self.events.push(SimEvent::ChargeFull { player: p.id });
                    }
                } else {
                    p.charging = None;
                    shot = Some(if t >= rules.charge_time && p.ammo >= rules.beam_cost {
                        Shot::Beam
                    } else {
                        Shot::Bolt
                    });
                }
            }
            if let Some(shot) = shot {
                self.fire(i, shot, &rules);
            }

            // Throwing.
            let p = &mut self.players[i];
            if let Some((rel, vel)) = p.cmd.throw.take()
                && p.grenades > 0
            {
                p.grenades -= 1;
                let eye = p.eye();
                let mut pos = p.feet + rel.clamp_length_max(2.0);
                let (id, team) = (p.id, p.team);
                if !pos.is_finite() || !self.world.clear(eye, pos) {
                    pos = eye;
                }
                let vel = if vel.is_finite() {
                    vel.clamp_length_max(rules.grenade_max_speed)
                } else {
                    Vec3::ZERO
                };
                let gid = self.id();
                self.grenades.push(Grenade {
                    id: gid,
                    owner: id,
                    team,
                    pos,
                    vel,
                    age: 0.0,
                    spin: 0.0,
                    phase: GrenadePhase::Flying,
                    next_burst: 0.0,
                });
                self.events
                    .push(SimEvent::GrenadeThrown { player: id, pos });
            }
        }
    }

    /// Player `pi` fires (it may: see `step_players`): a blaster bolt, the shotgun's spread of
    /// short-lived bolts or the charge gun's beam.
    fn fire(&mut self, pi: usize, shot: Shot, rules: &Rules) {
        let p = &mut self.players[pi];
        p.fire_cd = p.fire_interval(rules);
        p.decloak = rules.cloak_flash;
        let cost = if shot == Shot::Beam {
            rules.beam_cost
        } else {
            1
        };
        p.ammo = p.ammo.saturating_sub(cost);
        p.shots += 1;
        let eye = p.eye();
        let mut from = p.feet + p.cmd.muzzle.pos;
        // A muzzle far from its player, or through a wall, fires from the eye.
        if !from.is_finite() || (from - eye).length() > 1.5 {
            from = eye;
        }
        let mut dir = p.cmd.muzzle.forward();
        if !dir.is_finite() || dir.length_squared() < 0.5 {
            dir = Quat::from_rotation_y(p.cmd.yaw) * Vec3::NEG_Z;
        }
        let dir = dir.normalize();
        let (id, team, view) = (p.id, p.team, p.cmd.view_tick);
        let blocked = self.world.ray(eye, from);
        // Fired first, so its event comes before any hit it makes straight away.
        self.events.push(SimEvent::Fired {
            player: id,
            pos: from,
            dir,
        });
        if shot == Shot::Beam {
            self.events.push(SimEvent::BeamFired {
                player: id,
                pos: from,
                dir,
            });
        }
        if let Some((_, q, n)) = blocked {
            self.events.push(SimEvent::BoltImpact {
                pos: q,
                normal: n,
                team: Some(team),
            });
            return;
        }
        let (dirs, speed, life, extra) = match shot {
            Shot::Bolt => (
                vec![dir],
                rules.blaster_bolt_speed,
                rules.bolt_life,
                BoltExtra::default(),
            ),
            Shot::Spread => {
                let side = dir.any_orthonormal_vector();
                let up = dir.cross(side);
                let dirs: Vec<Vec3> = (0..rules.shotgun_bolts)
                    .map(|_| {
                        // Evenly over the cone's disc.
                        let r = self.rng.random::<f32>().sqrt() * rules.shotgun_spread.tan();
                        let a = self.rng.random_range(0.0..std::f32::consts::TAU);
                        (dir + (side * a.cos() + up * a.sin()) * r).normalize()
                    })
                    .collect();
                (
                    dirs,
                    rules.blaster_bolt_speed,
                    rules.shotgun_range / rules.blaster_bolt_speed,
                    BoltExtra::default(),
                )
            }
            Shot::Beam => (
                vec![dir],
                rules.beam_speed,
                rules.beam_range / rules.beam_speed,
                BoltExtra {
                    damage: rules.beam_damage,
                    pierce: true,
                    ..BoltExtra::default()
                },
            ),
        };
        for d in dirs {
            let mut bolt = Bolt {
                id: self.id(),
                owner: id,
                owner_team: team,
                team: Some(team),
                pos: from,
                vel: d * speed,
                life,
                extra,
            };
            if self.compensate(&mut bolt, view) {
                self.bolts.push(bolt);
            }
        }
    }

    /// Lag compensation for a bolt just fired from a command made looking at `view_tick`:
    /// flies it through the ticks since then against the players as they were (see the module
    /// documentation). False if it hit something meanwhile.
    fn compensate(&mut self, bolt: &mut Bolt, view_tick: u64) -> bool {
        if view_tick == 0 || view_tick >= self.tick {
            return true;
        }
        let lag = (self.tick - view_tick).min(MAX_LAG);
        for k in 0..lag {
            // The bolt's k-th tick meets the world the shooter saw k ticks after `view_tick`.
            let Some(h) = self.past(self.tick - lag + k) else {
                continue;
            };
            if self.bolt_tick(bolt, Some(h)) {
                return false;
            }
            bolt.pos += bolt.vel * TICK;
            bolt.life -= TICK;
        }
        bolt.life > 0.0
    }

    /// One tick of a bolt's flight from `b.pos`: the first of the arena (including the target
    /// panels) and the players it meets, the players as they are now or as they were at the
    /// end of the recorded tick `past`. Applies the hit; true if the bolt is used up.
    fn bolt_tick(&mut self, b: &mut Bolt, past: Option<usize>) -> bool {
        let to = b.pos + b.vel * TICK;
        let mut t_best = 1.0f32;
        let mut hit: Option<(Vec3, Vec3)> = None;
        if let Some((t, q, n)) = self.world.ray(b.pos, to) {
            t_best = t;
            hit = Some((q, n));
        }
        // A barrier in the way (but its own team's bolts pass).
        let mut barrier: Option<usize> = None;
        let (bw, bh) = (self.rules.barrier_w, self.rules.barrier_h);
        for (k, br) in self.barriers.iter().enumerate() {
            if b.team == Some(br.team) {
                continue;
            }
            if let Some(t) = br.crossed(b.pos, to, bw, bh)
                && t < t_best
            {
                t_best = t;
                barrier = Some(k);
            }
        }
        // Team bolts pass through team mates (and their shooter); grenade bolts hit everyone.
        let struck = b.extra.struck;
        let passes = |p: &Player| {
            !p.alive()
                || b.team.is_some() && (p.id == b.owner || b.team == Some(p.team))
                || struck & (1u64 << (p.id % 64)) != 0
        };
        let mut victim: Option<usize> = None;
        match past {
            None => {
                for (pi, p) in self.players.iter().enumerate() {
                    if passes(p) {
                        continue;
                    }
                    if let Some((s, _)) = p.hit_by(b.pos, to)
                        && s <= t_best
                    {
                        t_best = s;
                        victim = Some(pi);
                    }
                }
            }
            Some(h) => {
                for g in &self.history[h].1 {
                    // Where they were, if they were there then and are still in the game now.
                    let Some(pi) = self.players.iter().position(|p| p.id == g.id) else {
                        continue;
                    };
                    if !g.alive || passes(&self.players[pi]) {
                        continue;
                    }
                    if let Some((s, _)) = first_hit(g.feet, g.eye, &g.cmd, b.pos, to)
                        && s <= t_best
                    {
                        t_best = s;
                        victim = Some(pi);
                    }
                }
            }
        }
        let mut remove = false;
        if victim.is_none()
            && let Some(k) = barrier
        {
            let at = b.pos + (to - b.pos) * t_best;
            self.events.push(SimEvent::BarrierHit {
                id: self.barriers[k].id,
                pos: at,
            });
            return true;
        }
        if let Some(pi) = victim {
            let at = b.pos + (to - b.pos) * t_best;
            self.hit_player(pi, b, at);
            if !b.extra.pierce {
                return true;
            }
            // A beam: on through them (and whoever else is in its way this tick).
            b.extra.struck |= 1u64 << (self.players[pi].id % 64);
            return self.bolt_tick(b, past);
        } else if let Some((q, n)) = hit {
            // Where the bolt struck, on the box (however oblique the shot).
            let struck =
                |min: Vec3, max: Vec3| q.cmpge(min - 0.01).all() && q.cmple(max + 0.01).all();
            // A mirror: off it again (as many times as the rules allow), from where it struck.
            if b.extra.bounces < self.rules.mirror_bounces
                && let Some(m) = self.level.mirrors.iter().find(|m| struck(m.min, m.max))
            {
                let n = m.normal;
                b.vel -= 2.0 * b.vel.dot(n) * n;
                // (The flight moves it on by a tick's worth after this.)
                b.pos = q + n * 0.02 - b.vel * TICK;
                b.extra.bounces += 1;
                self.events.push(SimEvent::Reflected {
                    pos: q,
                    normal: n,
                    team: b.team,
                });
                return false;
            }
            if self.mega_bolt(b, q, &struck) {
                return true;
            }
            // A target panel?
            let target = self
                .level
                .targets
                .iter()
                .find(|t| struck(t.min, t.max))
                .copied();
            match target {
                Some(t) if t.team != b.owner_team => {
                    self.award(b.owner_team, Some(b.owner), self.rules.target_points);
                    if let Some(p) = self.player_mut(b.owner) {
                        p.target_hits += 1;
                    }
                    if let Some(h) = self.last_target_hit.get_mut(t.id as usize) {
                        *h = Some((self.tick, b.owner));
                    }
                    self.events.push(SimEvent::TargetHit {
                        target: t.id,
                        by: b.owner,
                        pos: q,
                    });
                }
                _ => self.events.push(SimEvent::BoltImpact {
                    pos: q,
                    normal: n,
                    team: b.team,
                }),
            }
            remove = true;
        }
        remove
    }

    fn step_bolts(&mut self) {
        let mut i = 0;
        while i < self.bolts.len() {
            let mut bolt = self.bolts[i];
            let remove = self.bolt_tick(&mut bolt, None);
            let b = &mut self.bolts[i];
            *b = bolt;
            b.pos += b.vel * TICK;
            b.life -= TICK;
            if remove || b.life <= 0.0 {
                self.bolts.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn hit_player(&mut self, pi: usize, b: &Bolt, at: Vec3) {
        let (hit_cooldown, hits_to_tag) = (self.rules.hit_cooldown, self.rules.hits_to_tag);
        let p = &mut self.players[pi];
        if p.shield > 0.0 {
            self.events.push(SimEvent::Blocked {
                player: p.id,
                pos: at,
            });
            return;
        }
        if p.cooldown > 0.0 {
            // Still recovering from the last hit: the bolt just fizzles.
            self.events.push(SimEvent::BoltImpact {
                pos: at,
                normal: (b.pos - at).normalize_or_zero(),
                team: b.team,
            });
            return;
        }
        p.hits = (p.hits + b.extra.damage.max(1)).min(hits_to_tag);
        p.cooldown = hit_cooldown;
        p.decloak = self.rules.cloak_flash;
        let (id, team, feet, hits) = (p.id, p.team, p.feet, p.hits);
        // Who hit them, for assists.
        if b.owner != HAZARD && b.owner_team != team {
            let tick = self.tick;
            p.hit_by.retain(|h| h.0 != b.owner);
            p.hit_by.push((b.owner, tick));
        }
        // Only team bolts at an enemy count as landed (grenade bolts hit anyone).
        if b.team.is_some_and(|t| t != team)
            && let Some(s) = self.player_mut(b.owner)
        {
            s.landed += 1;
        }
        self.events.push(SimEvent::Hit {
            player: id,
            by: b.owner,
            pos: at,
            hits,
        });
        if hits >= hits_to_tag {
            let wait = self.respawn_time(team, feet);
            let p = &mut self.players[pi];
            p.respawn = Some(wait);
            p.tagged += 1;
            p.cmd.fire = false;
            p.mega_hits = 0;
            p.power = None;
            p.lose_powerups();
            self.events.push(SimEvent::TaggedOut {
                player: id,
                by: b.owner,
                pos: at,
                respawn_in: wait,
            });
            let hit_by = std::mem::take(&mut self.players[pi].hit_by);
            if b.owner_team != team && b.owner != HAZARD {
                self.award(b.owner_team, Some(b.owner), self.rules.tag_points);
                let grenade = b.team.is_none();
                if let Some(s) = self.player_mut(b.owner) {
                    s.tags += 1;
                    if grenade {
                        s.grenade_tags += 1;
                    }
                    if s.cloak > 0.0 {
                        s.cloak_tags += 1;
                    }
                }
                // Defending: tagged near the tagger's own targets (Megatarget mode: their
                // team's doorway into the megatarget's room).
                let range = self.rules.defend_range;
                let near = |c: Vec3| c.distance(feet) < range;
                let defended = match &self.level.mega {
                    None => self
                        .level
                        .targets
                        .iter()
                        .any(|t| t.team == b.owner_team && near(t.centre())),
                    Some(m) => m
                        .shields
                        .iter()
                        .any(|s| Team::of_z(s.outside.z) == b.owner_team && near(s.outside)),
                };
                if defended {
                    self.award(b.owner_team, Some(b.owner), self.rules.defend_points);
                    if let Some(s) = self.player_mut(b.owner) {
                        s.defends += 1;
                    }
                    self.events.push(SimEvent::Defended {
                        player: b.owner,
                        victim: id,
                    });
                }
            }
            // Assists: everyone else of the other team who hit them a moment ago.
            let since = self
                .tick
                .saturating_sub((self.rules.assist_window / TICK) as u64);
            for (helper, tick) in hit_by {
                let Some(h) = self.player(helper) else {
                    continue;
                };
                if helper == b.owner || h.team == team || tick < since {
                    continue;
                }
                let helper_team = h.team;
                self.award(helper_team, Some(helper), self.rules.assist_points);
                if let Some(h) = self.player_mut(helper) {
                    h.assists += 1;
                }
                self.events.push(SimEvent::Assist {
                    player: helper,
                    victim: id,
                });
            }
        }
    }

    fn step_grenades(&mut self) {
        let rules = self.rules.clone();
        let mut i = 0;
        while i < self.grenades.len() {
            let mut g = self.grenades[i];
            g.age += TICK;
            let mut done = false;
            match g.phase {
                GrenadePhase::Flying => {
                    let (pos, vel, bounce) =
                        grenade_step(&self.world, g.pos, g.vel, rules.grenade_restitution);
                    (g.pos, g.vel) = (pos, vel);
                    if let Some((q, speed)) = bounce
                        && speed > 1.2
                    {
                        self.events.push(SimEvent::GrenadeBounce { pos: q, speed });
                    }
                    g.spin += g.vel.length() * TICK * 2.0;
                    if g.age >= rules.grenade_fuse {
                        let up = g.pos + Vec3::Y * rules.grenade_hop;
                        let to = match self.world.ray(g.pos, up + Vec3::Y * 0.2) {
                            Some((_, q, _)) => q - Vec3::Y * 0.2,
                            None => up,
                        };
                        g.phase = GrenadePhase::Active {
                            from: g.pos,
                            to: to.max(g.pos),
                        };
                        g.next_burst = 0.3;
                        self.events.push(SimEvent::GrenadeArmed { pos: g.pos });
                    }
                }
                GrenadePhase::Active { from, to } => {
                    let t = g.age - rules.grenade_fuse;
                    let k = (t / 0.3).clamp(0.0, 1.0);
                    let k = k * k * (3.0 - 2.0 * k);
                    g.pos = from.lerp(to, k) + Vec3::Y * (t * 3.0).sin() * 0.04 * k;
                    g.spin += TICK * (4.0 + 8.0 * k);
                    g.next_burst -= TICK;
                    if g.next_burst <= 0.0 {
                        g.next_burst += rules.grenade_burst_every;
                        let (lo, hi) = rules.grenade_burst_bolts;
                        let n = self.rng.random_range(lo..=hi.max(lo));
                        // Players the grenade sees (anyone: team mates and the thrower too).
                        let seen: Vec<Vec3> = self
                            .players
                            .iter()
                            .filter(|p| p.alive())
                            .map(|p| p.eye() - Vec3::Y * 0.35)
                            .filter(|c| {
                                c.distance(g.pos) < GRENADE_AIM_RANGE && self.world.clear(g.pos, *c)
                            })
                            .collect();
                        for _ in 0..n {
                            let z: f32 = self.rng.random_range(-1.0..1.0);
                            let phi: f32 = self.rng.random_range(0.0..std::f32::consts::TAU);
                            let r = (1.0 - z * z).sqrt();
                            let mut dir = Vec3::new(r * phi.cos(), z, r * phi.sin());
                            if !seen.is_empty() && self.rng.random::<f32>() < rules.grenade_aimed {
                                // At a player, a little off (the random direction nudges it).
                                let at = seen[self.rng.random_range(0..seen.len())];
                                dir =
                                    ((at - g.pos).normalize_or(dir) + dir * 0.06).normalize_or(dir);
                            }
                            let id = self.id();
                            self.bolts.push(Bolt {
                                id,
                                owner: g.owner,
                                owner_team: g.team,
                                team: None,
                                pos: g.pos + dir * 0.1,
                                vel: dir * rules.bolt_speed,
                                life: rules.bolt_life,
                                extra: BoltExtra::default(),
                            });
                        }
                        self.events.push(SimEvent::GrenadeBurst { pos: g.pos });
                    }
                    if t >= rules.grenade_active {
                        done = true;
                        self.events.push(SimEvent::GrenadeDone { pos: g.pos });
                    }
                }
            }
            if done {
                self.grenades.swap_remove(i);
            } else {
                self.grenades[i] = g;
                i += 1;
            }
        }
    }

    /// Arena events coming and going, and double points at the end.
    fn step_events(&mut self) {
        if let Some((kind, left)) = self.event.as_mut() {
            *left -= TICK;
            if *left <= 0.0 {
                let kind = *kind;
                self.event = None;
                self.events.push(SimEvent::EventEnd { kind });
            }
        }
        let rules = &self.rules;
        let elapsed = rules.match_time - self.time_left;
        if rules.events
            && self.event.is_none()
            && elapsed >= self.next_event
            && self.time_left > rules.event_quiet
        {
            let kind = if self.rng.random_bool(0.5) {
                ArenaEvent::Blackout
            } else {
                ArenaEvent::Overdrive
            };
            self.start_event(kind);
            let (lo, hi) = self.rules.event_every;
            self.next_event = elapsed + self.rng.random_range(lo..=hi.max(lo));
        }
        if !self.double && self.rules.double_last > 0.0 && self.time_left <= self.rules.double_last
        {
            self.double = true;
            self.events.push(SimEvent::DoublePoints);
        }
    }

    /// Starts an arena event now (testing too).
    pub fn start_event(&mut self, kind: ArenaEvent) {
        if let Some((old, _)) = self.event.take() {
            self.events.push(SimEvent::EventEnd { kind: old });
        }
        let length = match kind {
            ArenaEvent::Blackout => self.rules.blackout_time,
            // (Over at once; the time is the announcement's.)
            ArenaEvent::Overdrive => {
                for slot in &mut self.slots {
                    if slot.pickup.is_none() && slot.wait < f32::MAX {
                        slot.wait = 0.0;
                    }
                }
                4.0
            }
        };
        self.event = Some((kind, length));
        self.events.push(SimEvent::EventStart { kind });
    }

    /// Points for a team, and for one of its players (twice as many at the end).
    fn award(&mut self, team: Team, player: Option<PlayerId>, points: u32) {
        let points = if self.double { points * 2 } else { points };
        self.scores[team.index()] += points;
        if let Some(p) = player.and_then(|id| self.player_mut(id)) {
            p.score += points;
        }
    }

    /// Barriers running out.
    fn step_barriers(&mut self) {
        for b in &mut self.barriers {
            b.left -= TICK;
        }
        let events = &mut self.events;
        self.barriers.retain(|b| {
            let keep = b.left > 0.0;
            if !keep {
                events.push(SimEvent::BarrierDown {
                    id: b.id,
                    pos: b.centre,
                });
            }
            keep
        });
    }

    /// Where a barrier for player `pi` goes up: `Rules::barrier_dist` in front of them (nearer
    /// with a wall in the way), standing on the floor there, facing them.
    fn barrier_for(&self, pi: usize) -> Barrier {
        let p = &self.players[pi];
        let ahead = Quat::from_rotation_y(p.cmd.yaw) * Vec3::NEG_Z;
        let chest = p.feet + Vec3::Y * 1.0;
        let reach = self.rules.barrier_dist;
        let dist = match self.world.ray(chest, chest + ahead * (reach + 0.3)) {
            Some((t, _, _)) => (t * (reach + 0.3) - 0.3).max(0.4),
            None => reach,
        };
        let spot = p.feet + ahead * dist;
        let floor = self
            .world
            .ground(spot.x, spot.z, p.feet.y + 0.5)
            .unwrap_or(p.feet.y);
        Barrier {
            id: 0,
            team: p.team,
            centre: Vec3::new(spot.x, floor + self.rules.barrier_h / 2.0, spot.z),
            normal: -ahead,
            left: self.rules.barrier_time,
        }
    }

    /// Whether a barrier stops a bolt of `team` (`None`: anyone's) from `a` to `b`.
    pub fn barrier_blocks(&self, a: Vec3, b: Vec3, team: Option<Team>) -> bool {
        let (w, h) = (self.rules.barrier_w, self.rules.barrier_h);
        self.barriers
            .iter()
            .any(|br| Some(br.team) != team && br.crossed(a, b, w, h).is_some())
    }

    /// Laser gates: whoever touches one while it's on takes a hit (unless shielded or still
    /// recovering from the last).
    fn step_gates(&mut self) {
        if self.level.gates.is_empty() {
            return;
        }
        let level = self.level.clone();
        for (k, gate) in level.gates.iter().enumerate() {
            if !gate.active(self.tick) {
                continue;
            }
            for pi in 0..self.players.len() {
                let p = &self.players[pi];
                if !p.alive() || p.cooldown > 0.0 || p.shield > 0.0 || !gate.touches(p.feet) {
                    continue;
                }
                let at = p.chest();
                self.events.push(SimEvent::GateZap {
                    gate: k as u8,
                    player: p.id,
                    pos: at,
                });
                let bolt = Bolt {
                    id: 0,
                    owner: HAZARD,
                    owner_team: Team::Red,
                    team: None,
                    pos: gate.centre(),
                    vel: Vec3::ZERO,
                    life: 0.0,
                    extra: BoltExtra::default(),
                };
                self.hit_player(pi, &bolt, at);
            }
        }
    }

    /// Megatarget mode, once a tick: the players' time to get into the room, the forcefields
    /// and the turrets.
    fn step_mega(&mut self) {
        let Some(mega) = self.level.mega.clone() else {
            return;
        };
        let rules = self.rules.clone();
        for i in 0..self.players.len() {
            let p = &mut self.players[i];
            if !p.alive() {
                continue;
            }
            let (id, feet) = (p.id, p.feet);
            let mut zap = false;
            if let Some(t) = p.power.as_mut() {
                *t -= TICK;
                if *t <= 0.0 {
                    p.power = None;
                    p.mega_hits = 0;
                    self.events.push(SimEvent::PowerLost { player: id });
                    // Still in the room (or its doorway) without having hit the megatarget.
                    zap = mega.in_room(feet) || mega.touching_shield(feet).is_some();
                }
            } else if mega.touching_shield(feet).is_some() {
                self.events.push(SimEvent::Shocked {
                    player: id,
                    pos: feet + Vec3::Y * 1.2,
                });
                zap = true;
            }
            if zap {
                self.zap(i);
            }
        }

        for k in 0..self.turrets.len() {
            let def = mega.turrets[k];
            let st = &mut self.turrets[k];
            if st.off > 0.0 {
                st.off -= TICK;
                if st.off <= 0.0 {
                    st.off = 0.0;
                    st.fire_t = TURRET_WARMUP;
                    self.events.push(SimEvent::TurretOn { turret: k as u8 });
                }
                // Drooping while off.
                let rest = (def.facing - Vec3::Y * 0.6).normalize();
                st.aim = st.aim.lerp(rest, 0.05).normalize_or(rest);
                st.engaged = false;
                continue;
            }
            st.fire_t -= TICK;
            // The nearest player it sees in front of it who can be hit now.
            let muzzle = def.pos + def.facing * 0.3;
            let target = self
                .players
                .iter()
                .filter(|p| {
                    p.alive()
                        && p.cooldown <= 0.0
                        && p.shield <= 0.0
                        && p.seen_from(p.chest().distance(muzzle))
                })
                .map(|p| p.chest())
                .filter(|c| {
                    let d = *c - muzzle;
                    d.length() < rules.turret_range
                        && d.normalize_or_zero().dot(def.facing) > 0.1
                        && self.world.clear(muzzle, *c)
                })
                .min_by(|a, b| a.distance(muzzle).total_cmp(&b.distance(muzzle)));
            let want = match target {
                Some(c) => (c - def.pos).normalize_or(def.facing),
                // Nobody: sweep slowly from side to side.
                None => {
                    let t = self.tick as f32 * TICK;
                    Quat::from_rotation_y((t * 0.5 + k as f32 * 1.7).sin() * 0.7)
                        * (def.facing - Vec3::Y * 0.25).normalize()
                }
            };
            let st = &mut self.turrets[k];
            if target.is_some() && !st.engaged {
                st.fire_t = st.fire_t.max(TURRET_WARMUP);
                self.events
                    .push(SimEvent::TurretEngaged { turret: k as u8 });
            }
            st.engaged = target.is_some();
            let angle = st.aim.angle_between(want);
            if angle > 1e-4 {
                let step = (rules.turret_turn * TICK).min(angle);
                let axis = st.aim.cross(want).normalize_or(Vec3::Y);
                st.aim = (Quat::from_axis_angle(axis, step) * st.aim).normalize_or(want);
            }
            if target.is_some() && st.fire_t <= 0.0 && st.aim.angle_between(want) < 0.06 {
                st.fire_t = rules.turret_every;
                let aim = st.aim;
                // Off by a random angle in a random direction.
                let side = aim.any_orthonormal_vector();
                let err: f32 = ((0..3).map(|_| self.rng.random::<f32>()).sum::<f32>() - 1.5)
                    * 2.0
                    * rules.turret_spread;
                let roll = self.rng.random_range(0.0..std::f32::consts::TAU);
                let dir = Quat::from_axis_angle(Quat::from_axis_angle(aim, roll) * side, err) * aim;
                let from = def.pos + dir * 0.5;
                let id = self.id();
                self.bolts.push(Bolt {
                    id,
                    owner: HAZARD,
                    owner_team: Team::Red,
                    team: None,
                    pos: from,
                    vel: dir * rules.bolt_speed,
                    life: rules.bolt_life,
                    extra: BoltExtra::default(),
                });
                self.events.push(SimEvent::TurretFired {
                    turret: k as u8,
                    pos: from,
                    dir,
                });
            }
        }
    }

    /// Gives a player a pickup's contents at once (testing).
    pub fn give(&mut self, id: PlayerId, kind: PickupKind) {
        let rules = self.rules.clone();
        if let Some(p) = self.player_mut(id)
            && p.alive()
            && p.wants(kind, &rules)
        {
            p.take(kind, &rules);
        }
    }

    /// Lays a pickup at `pos` (testing; it doesn't come back once taken).
    pub fn drop_pickup(&mut self, kind: PickupKind, pos: Vec3) {
        let id = self.id();
        self.slots.push(PickupSlot {
            half: Team::of_z(pos.z),
            kind,
            pickup: Some(Pickup { id, pos, kind }),
            wait: f32::MAX,
        });
        self.events.push(SimEvent::PickupSpawned { id, pos, kind });
    }

    /// Puts a player somewhere else at once, facing `yaw` (testing; its client is told by a
    /// `Respawned` event, as after a respawn).
    pub fn teleport(&mut self, id: PlayerId, feet: Vec3, yaw: f32) {
        let Some(p) = self.player_mut(id) else { return };
        p.feet = feet;
        p.vy = 0.0;
        p.air = Vec2::ZERO;
        p.cmd.yaw = yaw;
        p.claim_block = 30;
        self.events.push(SimEvent::Respawned {
            player: id,
            pos: feet,
        });
    }

    /// Tags player `pi` out without a shooter (a forcefield, the room's timer).
    fn zap(&mut self, pi: usize) {
        let (team, feet) = (self.players[pi].team, self.players[pi].feet);
        let wait = self.respawn_time(team, feet);
        let p = &mut self.players[pi];
        p.hits = self.rules.hits_to_tag;
        p.hit_by.clear();
        p.respawn = Some(wait);
        p.tagged += 1;
        p.cmd.fire = false;
        p.mega_hits = 0;
        p.power = None;
        p.lose_powerups();
        self.events.push(SimEvent::TaggedOut {
            player: p.id,
            by: HAZARD,
            pos: p.eye() - Vec3::Y * 0.4,
            respawn_in: wait,
        });
    }

    /// Sends a player back to one of its team's spawn pads at once, healed and refilled.
    fn send_home(&mut self, pi: usize) {
        let team = self.players[pi].team;
        let s = self.pick_spawn(team);
        let ammo = self.rules.ammo;
        let p = &mut self.players[pi];
        p.hits = 0;
        p.cooldown = 0.0;
        p.ammo = ammo;
        p.reloading = None;
        p.mega_hits = 0;
        p.power = None;
        p.feet = s.pos;
        p.vy = 0.0;
        p.air = Vec2::ZERO;
        p.cmd.yaw = s.yaw;
        p.claim_block = 30;
        self.events.push(SimEvent::Respawned {
            player: p.id,
            pos: s.pos,
        });
    }

    /// A bolt that struck the arena at `q`: whether it was one of the Megatarget mode's
    /// targets or switches (`struck` tells whether the strike was on a box), handled here.
    fn mega_bolt(&mut self, b: &Bolt, q: Vec3, struck: &dyn Fn(Vec3, Vec3) -> bool) -> bool {
        let Some(mega) = self.level.mega.as_ref() else {
            return false;
        };
        let on = |p: &Panel| struck(p.min, p.max);
        if let Some(k) = mega.turrets.iter().position(|t| on(&t.switch)) {
            if b.owner != HAZARD {
                self.turrets[k].off = self.rules.turret_off;
                self.events.push(SimEvent::TurretOff {
                    turret: k as u8,
                    by: b.owner,
                    pos: q,
                });
            }
            return true;
        }
        let side = mega.secondaries.iter().position(on);
        let core = on(&mega.core);
        if side.is_none() && !core {
            return false;
        }
        let in_room = |p: &Player| mega.in_room(p.feet);
        // Only a player's own blaster bolts count.
        let shooter = b.team.and_then(|_| {
            self.players
                .iter()
                .position(|p| p.id == b.owner && p.alive())
        });
        let counts = shooter.is_some_and(|pi| {
            let p = &self.players[pi];
            match side {
                Some(i) => p.power.is_none() && p.mega_hits & (1 << i) == 0,
                None => p.power.is_some() && in_room(p),
            }
        });
        if !counts {
            self.events.push(SimEvent::BoltImpact {
                pos: q,
                normal: -(b.vel.normalize_or_zero()),
                team: b.team,
            });
            return true;
        }
        let pi = shooter.unwrap();
        let points = if side.is_some() {
            self.rules.secondary_points
        } else {
            self.rules.megatarget_points
        };
        self.award(b.owner_team, Some(b.owner), points);
        let power_time = self.rules.power_time;
        let p = &mut self.players[pi];
        p.target_hits += 1;
        match side {
            Some(i) => {
                p.mega_hits |= 1 << i;
                let charged = p.mega_hits == 0b11;
                if charged {
                    p.power = Some(power_time);
                }
                self.events.push(SimEvent::SecondaryHit {
                    side: i as u8,
                    by: b.owner,
                    pos: q,
                    charged,
                });
            }
            None => {
                self.events.push(SimEvent::MegaHit {
                    by: b.owner,
                    pos: q,
                });
                self.send_home(pi);
            }
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            time_left: self.time_left,
            scores: self.scores,
            over: self.over,
            players: self
                .players
                .iter()
                .map(|p| PlayerSnap {
                    id: p.id,
                    team: p.team,
                    feet: p.feet,
                    yaw: p.cmd.yaw,
                    head: Pose {
                        pos: p.head_offset(),
                        rot: p.cmd.head.rot,
                    },
                    hands: p.cmd.hands,
                    gun_hand: p.cmd.gun_hand,
                    grenade_hand: p.cmd.grenade_hand,
                    hits: p.hits,
                    cooldown: p.cooldown,
                    respawn: p.respawn,
                    grenades: p.grenades,
                    ammo: p.ammo,
                    reloading: p.reloading.is_some(),
                    score: p.score,
                    tags: p.tags,
                    tagged: p.tagged,
                    shots: p.shots,
                    landed: p.landed,
                    target_hits: p.target_hits,
                    assists: p.assists,
                    defends: p.defends,
                    grenade_tags: p.grenade_tags,
                    cloak_tags: p.cloak_tags,
                    mega_hits: p.mega_hits,
                    power: p.power,
                    rapid: p.rapid,
                    shields: p.shields,
                    shield: p.shield,
                    shotgun: p.shotgun,
                    cloak: p.cloak,
                    decloak: p.decloak,
                    speed: p.speed,
                    charge: p.charge,
                    charging: p.charging.unwrap_or(0.0),
                    barriers: p.barriers,
                })
                .collect(),
            bolts: self
                .bolts
                .iter()
                .map(|b| BoltSnap {
                    id: b.id,
                    pos: b.pos,
                    vel: b.vel,
                    team: b.team,
                    beam: b.extra.pierce,
                })
                .collect(),
            grenades: self
                .grenades
                .iter()
                .map(|g| GrenadeSnap {
                    id: g.id,
                    pos: g.pos,
                    spin: g.spin,
                    active: matches!(g.phase, GrenadePhase::Active { .. }),
                })
                .collect(),
            pickups: self
                .pickups()
                .map(|p| PickupSnap {
                    id: p.id,
                    pos: p.pos,
                    kind: p.kind,
                })
                .collect(),
            turrets: self
                .turrets
                .iter()
                .map(|t| TurretSnap {
                    aim: t.aim,
                    off: t.off,
                })
                .collect(),
            barriers: self.barriers.clone(),
            event: self.event,
            double: self.double,
        }
    }
}

/// How much of the stick's movement counts while flying from a jump pad (all of it on the
/// ground).
pub fn air_control(air: Vec2) -> f32 {
    if air == Vec2::ZERO { 1.0 } else { 0.25 }
}

/// Jump pads, after a body's move (the sim's and the client's prediction alike): on landing
/// the flight's horizontal velocity `air` is gone; standing on a pad, the vertical speed and
/// horizontal velocity it throws the body with.
pub fn pad_step(level: &Level, feet: Vec3, vy: f32, air: &mut Vec2) -> Option<(f32, Vec2)> {
    if vy != 0.0 {
        return None;
    }
    *air = Vec2::ZERO;
    level
        .pad_at(feet)
        .map(|p| (p.launch.y, Vec2::new(p.launch.x, p.launch.z)))
}

/// Yaw (0 = facing -Z) of a horizontal direction.
pub fn yaw_of(dir: Vec3) -> f32 {
    (-dir.x).atan2(-dir.z)
}

/// The rotation looking along `dir`.
pub fn look_rot(dir: Vec3) -> Quat {
    let d = dir.normalize_or(Vec3::NEG_Z);
    let yaw = yaw_of(d);
    let pitch = d.y.clamp(-1.0, 1.0).asin();
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)
}

/// A hovering grenade aims some bolts (`Rules::grenade_aimed`) at players it sees this close.
pub const GRENADE_AIM_RANGE: f32 = 20.0;

/// One tick of a flying grenade: falling, bouncing off the arena (losing speed), coming to rest
/// on a floor. Returns the new position and velocity, and where it bounced and how hard.
pub fn grenade_step(
    world: &World,
    pos: Vec3,
    vel: Vec3,
    restitution: f32,
) -> (Vec3, Vec3, Option<(Vec3, f32)>) {
    let mut vel = vel;
    vel.y -= GRAVITY * TICK;
    let to = pos + vel * TICK;
    match world.ray(pos, to) {
        Some((_, q, n)) => {
            let vn = vel.dot(n);
            let tangent = vel - n * vn;
            vel = tangent * 0.8 - n * vn * restitution;
            if n.y > 0.7 && vel.length() < 0.4 {
                vel = Vec3::ZERO;
            }
            (q + n * 0.06, vel, Some((q, vn.abs())))
        }
        None => (to, vel, None),
    }
}

/// Where a grenade thrown from `from` (world) with `vel` flies until its fuse runs out (as the
/// sim will fly it, the arena's moving pieces aside): a point a tick, and the bounces.
pub fn grenade_path(world: &World, rules: &Rules, from: Vec3, vel: Vec3) -> (Vec<Vec3>, Vec<Vec3>) {
    let vel = if vel.is_finite() {
        vel.clamp_length_max(rules.grenade_max_speed)
    } else {
        Vec3::ZERO
    };
    let (mut pos, mut vel) = (from, vel);
    let mut path = vec![pos];
    let mut bounces = Vec::new();
    for _ in 0..(rules.grenade_fuse / TICK) as usize {
        let (p, v, b) = grenade_step(world, pos, vel, rules.grenade_restitution);
        (pos, vel) = (p, v);
        path.push(pos);
        if let Some((q, speed)) = b
            && speed > 1.2
        {
            bounces.push(q);
        }
        if vel == Vec3::ZERO {
            break;
        }
    }
    (path, bounces)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty-ish arena spot: two players facing each other in the open.
    fn duel() -> (Sim, PlayerId, PlayerId) {
        let mut sim = Sim::new(7, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let b = sim.add_player("b", Some(Team::Blue), false).unwrap();
        // Put them in their spawn areas' open rows, 3 m apart on the same line.
        let base = sim.level.base_centre(Team::Red);
        sim.players[0].feet = base;
        sim.players[1].feet = base + Vec3::new(0.0, 0.0, -3.0);
        (sim, a, b)
    }

    fn shoot(sim: &mut Sim, from: PlayerId, at: PlayerId) {
        let target = sim.player(at).unwrap().eye() - Vec3::Y * 0.3;
        let me = sim.player(from).unwrap();
        let muzzle_pos = me.eye() - Vec3::Y * 0.2 - me.feet;
        let dir = (target - (me.feet + muzzle_pos)).normalize();
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: muzzle_pos,
                rot: look_rot(dir),
            },
            fire: true,
            ..Default::default()
        };
        sim.set_cmd(from, cmd);
        sim.step();
        sim.set_cmd(from, PlayerCmd { fire: false, ..cmd });
        for _ in 0..30 {
            sim.step();
        }
    }

    /// A ground-floor mirror of the arena for `seed` and the directions along and out of it.
    fn a_mirror(sim: &Sim) -> (crate::level::Panel, Vec3, Vec3) {
        let m = *sim
            .level
            .mirrors
            .iter()
            .find(|m| m.max.y < 3.0)
            .expect("a mirror on the ground");
        let n = m.normal;
        (m, n, Vec3::Y.cross(n))
    }

    #[test]
    fn jump_pads_throw_players_onto_the_upper_floor() {
        let seed = (1..40)
            .find(|&s| {
                !Level::generate(s, crate::level::ArenaSize::Small)
                    .pads
                    .is_empty()
            })
            .expect("an arena with jump pads");
        let mut sim = Sim::new(seed, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let pad = sim.level.pads[0];
        sim.teleport(a, pad.pos, 0.0);
        sim.players[0].claim_block = 0;
        sim.drain_events();
        sim.step();
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Launched { player, .. } if *player == a))
        );
        for _ in 0..180 {
            sim.step();
        }
        let p = sim.player(a).unwrap();
        assert!(
            (p.feet - pad.land).length() < 0.6,
            "landed at {}, not {}",
            p.feet,
            pad.land
        );
        assert_eq!(p.air, Vec2::ZERO);
        assert_eq!(p.vy, 0.0);
    }

    #[test]
    fn laser_gates_hit_only_while_on() {
        let seed = (1..60)
            .find(|&s| {
                !Level::generate(s, crate::level::ArenaSize::Small)
                    .gates
                    .is_empty()
            })
            .expect("an arena with laser gates");
        let mut sim = Sim::new(seed, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let g = sim.level.gates[0];
        let mid = g.centre().with_y(0.0);
        let on = |sim: &Sim| g.active(sim.tick + 1);
        // Off: nothing.
        while on(&sim) {
            sim.tick += 1;
        }
        sim.teleport(a, mid, 0.0);
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // On: one hit, then the hit's cooldown.
        while !on(&sim) {
            sim.tick += 1;
        }
        sim.drain_events();
        sim.step();
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 1);
        assert_eq!(
            sim.drain_events()
                .iter()
                .filter(|e| matches!(e, SimEvent::GateZap { .. }))
                .count(),
            1
        );
        // Shielded: nothing.
        sim.players[0].cooldown = 0.0;
        sim.players[0].shield = 5.0;
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 1);
    }

    #[test]
    fn a_speed_boost_speeds_up_until_it_runs_out() {
        let (mut sim, a, b) = duel();
        let walk = |sim: &mut Sim| {
            sim.set_cmd(
                a,
                PlayerCmd {
                    stick: Vec2::new(0.0, -1.0),
                    ..Default::default()
                },
            );
            let from = sim.player(a).unwrap().feet;
            for _ in 0..10 {
                sim.step();
            }
            let d = sim.player(a).unwrap().feet - from;
            sim.players[0].feet = from;
            sim.set_cmd(a, PlayerCmd::default());
            d.length()
        };
        let plain = walk(&mut sim);
        sim.give(a, PickupKind::Speed);
        assert!(!sim.player(a).unwrap().wants(PickupKind::Speed, &sim.rules));
        let fast = walk(&mut sim);
        assert!(
            (fast / plain - 1.5).abs() < 0.05,
            "{fast} m against {plain} m"
        );
        for _ in 0..(15.0 / TICK) as usize {
            sim.step();
        }
        assert_eq!(sim.player(a).unwrap().speed, 0.0);
        assert!((walk(&mut sim) / plain - 1.0).abs() < 0.05);
        // Gone when tagged out.
        sim.give(a, PickupKind::Speed);
        let bolt = Bolt {
            id: 1,
            owner: b,
            owner_team: Team::Blue,
            team: Some(Team::Blue),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        sim.players[0].hits = 2;
        sim.hit_player(0, &bolt, Vec3::ZERO);
        assert_eq!(sim.player(a).unwrap().speed, 0.0);
    }

    #[test]
    fn the_charge_gun_taps_bolts_and_charges_beams_through_players() {
        let (mut sim, a, b) = duel();
        // (Standing on the spawn pads, which would refill it.)
        sim.rules.reload_rate = 0.0;
        let c = sim.add_player("c", Some(Team::Blue), false).unwrap();
        // Lined up: b 3 m in front of a, c 2 m behind b.
        let bf = sim.player(b).unwrap().feet;
        sim.players[2].feet = bf + Vec3::new(0.0, 0.0, -2.0);
        sim.give(a, PickupKind::Charge);
        assert!(sim.player(a).unwrap().charge);
        let me = sim.player(a).unwrap();
        let target = sim.player(b).unwrap().chest();
        let muzzle_pos = me.eye() - Vec3::Y * 0.2 - me.feet;
        let dir = (target - (me.feet + muzzle_pos)).normalize();
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: muzzle_pos,
                rot: look_rot(dir),
            },
            fire: true,
            ..Default::default()
        };
        let beams = |evs: &[SimEvent]| {
            evs.iter()
                .filter(|e| matches!(e, SimEvent::BeamFired { .. }))
                .count()
        };
        // A tap: a plain bolt, one hit on b only.
        sim.set_cmd(a, cmd);
        sim.step();
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..20 {
            sim.step();
        }
        let evs = sim.drain_events();
        assert_eq!(beams(&evs), 0);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        assert_eq!(sim.player(c).unwrap().hits, 0);
        assert_eq!(sim.player(a).unwrap().ammo, sim.rules.ammo - 1);
        // Held a second and let go: a beam through both, two hits each.
        for p in &mut sim.players {
            p.hits = 0;
            p.cooldown = 0.0;
        }
        sim.set_cmd(a, cmd);
        for _ in 0..(sim.rules.charge_time / TICK) as usize + 2 {
            sim.step();
        }
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::ChargeFull { .. }))
        );
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..10 {
            sim.step();
        }
        let evs = sim.drain_events();
        assert_eq!(beams(&evs), 1, "{evs:?}");
        assert_eq!(sim.player(b).unwrap().hits, 2);
        assert_eq!(sim.player(c).unwrap().hits, 2);
        assert_eq!(
            sim.player(a).unwrap().ammo,
            sim.rules.ammo - 1 - sim.rules.beam_cost
        );
        // A hit while charging stops it.
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..30 {
            sim.step();
        }
        sim.set_cmd(a, cmd);
        for _ in 0..30 {
            sim.step();
        }
        assert!(sim.player(a).unwrap().charging.is_some());
        let bolt = Bolt {
            id: 1,
            owner: b,
            owner_team: Team::Blue,
            team: Some(Team::Blue),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        sim.hit_player(0, &bolt, Vec3::ZERO);
        sim.step();
        assert!(sim.player(a).unwrap().charging.is_none());
        // One special gun at a time.
        sim.players[0].hits = 0;
        assert!(
            !sim.player(a)
                .unwrap()
                .wants(PickupKind::Shotgun, &sim.rules)
        );
        sim.give(a, PickupKind::Shotgun);
        let p = sim.player(a).unwrap();
        assert!(!p.shotgun && p.charge);
    }

    #[test]
    fn barriers_stop_enemy_bolts_only() {
        let (mut sim, a, b) = duel();
        sim.give(a, PickupKind::Barrier);
        assert_eq!(sim.player(a).unwrap().barriers, 1);
        // Facing b (3 m away along -z): up between them.
        sim.set_cmd(
            a,
            PlayerCmd {
                yaw: 0.0,
                barrier: true,
                ..Default::default()
            },
        );
        sim.step();
        sim.step();
        assert_eq!(sim.barriers.len(), 1, "one barrier for one press");
        assert_eq!(sim.player(a).unwrap().barriers, 0);
        let br = sim.barriers[0];
        let fa = sim.player(a).unwrap().feet;
        assert!(((br.centre - fa).with_y(0.0).length() - sim.rules.barrier_dist).abs() < 0.05);
        assert_eq!(br.team, Team::Red);
        // b's bolt at a stops on it.
        let bolt = |owner, team, from: Vec3, to: Vec3| Bolt {
            id: 7,
            owner,
            owner_team: team,
            team: Some(team),
            pos: from,
            vel: (to - from).normalize() * 40.0,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        let (ca, cb) = (
            sim.player(a).unwrap().chest(),
            sim.player(b).unwrap().chest(),
        );
        sim.bolts.push(bolt(b, Team::Blue, cb, ca));
        for _ in 0..10 {
            sim.step();
        }
        let evs = sim.drain_events();
        assert!(evs.iter().any(|e| matches!(e, SimEvent::BarrierHit { .. })));
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // a's goes through it at b.
        sim.bolts.push(bolt(a, Team::Red, ca, cb));
        for _ in 0..10 {
            sim.step();
        }
        assert_eq!(sim.player(b).unwrap().hits, 1);
        for _ in 0..(sim.rules.barrier_time / TICK) as usize {
            sim.step();
        }
        assert!(sim.barriers.is_empty());
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::BarrierDown { .. }))
        );
        // Facing a wall a metre off: nearer.
        let (m, n, _) = a_mirror(&sim);
        let floor = m.min.y - 0.2;
        let c = m.centre();
        sim.players[0].feet = Vec3::new(c.x, floor, c.z) + n * 1.0;
        sim.players[0].cmd.yaw = yaw_of(-n);
        let near = sim.barrier_for(0);
        assert!((near.centre - sim.players[0].feet).with_y(0.0).length() < 0.8);
    }

    #[test]
    fn assists_and_defends_score() {
        let (mut sim, a, b) = duel();
        let a2 = sim.add_player("a2", Some(Team::Red), false).unwrap();
        let bolt = |owner| Bolt {
            id: 1,
            owner,
            owner_team: Team::Red,
            team: Some(Team::Red),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        let bi = pi_of(&sim, b);
        // Far from every target.
        sim.players[bi].feet = Vec3::ZERO;
        sim.hit_player(bi, &bolt(a), Vec3::ZERO);
        for by in [a2, a2] {
            sim.players[bi].cooldown = 0.0;
            sim.hit_player(bi, &bolt(by), Vec3::ZERO);
        }
        assert!(!sim.player(b).unwrap().alive());
        let evs = sim.drain_events();
        assert!(evs.iter().any(
            |e| matches!(e, SimEvent::Assist { player, victim } if *player == a && *victim == b)
        ));
        assert!(!evs.iter().any(|e| matches!(e, SimEvent::Defended { .. })));
        let (pa, pa2) = (sim.player(a).unwrap(), sim.player(a2).unwrap());
        assert_eq!((pa.assists, pa.score), (1, sim.rules.assist_points));
        assert_eq!(
            (pa2.tags, pa2.assists, pa2.score),
            (1, 0, sim.rules.tag_points)
        );
        assert_eq!(
            sim.scores[0],
            sim.rules.tag_points + sim.rules.assist_points
        );

        // Tagged out right by Red's own target: a defend for a2.
        let t = *sim
            .level
            .targets
            .iter()
            .find(|t| t.team == Team::Red)
            .unwrap();
        let bi = pi_of(&sim, b);
        sim.players[bi].respawn = None;
        sim.players[bi].hits = 0;
        sim.players[bi].cooldown = 0.0;
        sim.players[bi].feet = t.centre().with_y(0.0);
        for _ in 0..3 {
            sim.players[bi].cooldown = 0.0;
            sim.hit_player(bi, &bolt(a2), Vec3::ZERO);
        }
        let evs = sim.drain_events();
        assert!(
            evs.iter()
                .any(|e| matches!(e, SimEvent::Defended { player, .. } if *player == a2))
        );
        let pa2 = sim.player(a2).unwrap();
        assert_eq!(pa2.defends, 1);
        assert_eq!(
            pa2.score,
            2 * sim.rules.tag_points + sim.rules.defend_points
        );
        // (No assist for a: their hit was on b's previous life.)
        assert_eq!(sim.player(a).unwrap().assists, 1);
    }

    #[test]
    fn arena_events_come_on_schedule_and_points_double_at_the_end() {
        let run = |seed| {
            let mut sim = Sim::new(seed, Rules::default());
            let mut starts = Vec::new();
            let mut doubles = Vec::new();
            while !sim.over {
                sim.step();
                for e in sim.drain_events() {
                    match e {
                        SimEvent::EventStart { kind } => starts.push((kind, sim.time_left)),
                        SimEvent::DoublePoints => doubles.push(sim.time_left),
                        _ => {}
                    }
                }
            }
            (starts, doubles)
        };
        let (starts, doubles) = run(5);
        assert_eq!(
            run(5),
            (starts.clone(), doubles.clone()),
            "the same for a seed"
        );
        let rules = Rules::default();
        assert!(!starts.is_empty() && starts.len() <= 4, "{starts:?}");
        for &(_, left) in &starts {
            let elapsed = rules.match_time - left;
            assert!(elapsed >= rules.event_first.0 - 0.1, "{starts:?}");
            assert!(left > rules.event_quiet, "{starts:?}");
        }
        assert_eq!(doubles.len(), 1);
        assert!((doubles[0] - rules.double_last).abs() < 0.05);
    }

    #[test]
    fn overdrive_fills_every_pickup_and_double_points_double() {
        let (mut sim, a, b) = duel();
        sim.step();
        let slots = sim.slots.len();
        assert!(sim.pickups().count() < slots);
        sim.start_event(ArenaEvent::Overdrive);
        sim.step();
        assert_eq!(sim.pickups().count(), slots);
        assert!(matches!(sim.event, Some((ArenaEvent::Overdrive, _))));
        // The last minute: a tag-out is worth twice as much.
        sim.time_left = sim.rules.double_last - 1.0;
        sim.step();
        assert!(sim.double);
        let bolt = Bolt {
            id: 1,
            owner: a,
            owner_team: Team::Red,
            team: Some(Team::Red),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        let bi = pi_of(&sim, b);
        sim.players[bi].feet = Vec3::ZERO;
        sim.players[bi].hits = 2;
        sim.hit_player(bi, &bolt, Vec3::ZERO);
        assert_eq!(sim.player(a).unwrap().score, 2 * sim.rules.tag_points);
        assert_eq!(sim.scores[0], 2 * sim.rules.tag_points);
    }

    #[test]
    fn mirrors_bounce_bolts_once() {
        let mut sim = Sim::new(7, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let b = sim.add_player("b", Some(Team::Blue), false).unwrap();
        let (m, n, along) = a_mirror(&sim);
        let c = m.centre();
        // The victim off to one side in front of the mirror, the bolt coming in from the
        // other side at 45 degrees, at the victim's chest height.
        let floor = sim.world.ground(c.x, c.z, c.y).unwrap_or(0.0);
        let spot = c + n * 1.2 - along * 0.6;
        sim.players[1].feet = Vec3::new(spot.x, floor, spot.z);
        sim.players[0].feet = sim.level.base_centre(Team::Red);
        let chest = sim.players[1].chest();
        let from = Vec3::new(c.x, chest.y, c.z) + n * 1.2 + along * 0.6;
        let at = Vec3::new(c.x, chest.y, c.z);
        sim.bolts.push(Bolt {
            id: 1,
            owner: a,
            owner_team: Team::Red,
            team: Some(Team::Red),
            pos: from,
            vel: (at - from).normalize() * 40.0,
            life: 2.0,
            extra: BoltExtra::default(),
        });
        for _ in 0..10 {
            sim.step();
        }
        let evs = sim.drain_events();
        assert!(
            evs.iter().any(|e| matches!(e, SimEvent::Reflected { .. })),
            "bounced: {evs:?}"
        );
        assert_eq!(sim.player(b).unwrap().hits, 1, "and hit: {evs:?}");

        // A bolt that has bounced already stops at the mirror.
        sim.bolts.push(Bolt {
            id: 2,
            owner: a,
            owner_team: Team::Red,
            team: Some(Team::Red),
            pos: c + n * 1.5,
            vel: -n * 40.0,
            life: 2.0,
            extra: BoltExtra {
                bounces: 1,
                ..BoltExtra::default()
            },
        });
        for _ in 0..10 {
            sim.step();
        }
        let evs = sim.drain_events();
        assert!(!evs.iter().any(|e| matches!(e, SimEvent::Reflected { .. })));
        assert!(evs.iter().any(|e| matches!(e, SimEvent::BoltImpact { .. })));
        assert!(sim.bolts.is_empty());
    }

    #[test]
    fn three_hits_tag_out_with_cooldowns() {
        let (mut sim, a, b) = duel();
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        let ev = sim.drain_events();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::Hit { player, hits: 1, .. } if *player == b))
        );
        // During the cooldown further hits don't count.
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        // The victim can't fire meanwhile.
        sim.drain_events();
        shoot(&mut sim, b, a);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Denied { player } if *player == b))
        );
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // Wait out the cooldown, hit twice more.
        for _ in 0..(3.0 / TICK) as usize {
            sim.step();
        }
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 2);
        for _ in 0..(3.0 / TICK) as usize {
            sim.step();
        }
        sim.drain_events();
        shoot(&mut sim, a, b);
        let pb = sim.player(b).unwrap();
        assert!(!pb.alive());
        let wait = pb.respawn.unwrap();
        assert!((5.0..=15.0).contains(&(wait + 0.6)), "{wait}");
        assert_eq!(sim.scores, [100, 0]);
        // Respawns at its own base, healed.
        for _ in 0..(16.0 / TICK) as usize {
            sim.step();
        }
        let pb = sim.player(b).unwrap();
        assert!(pb.alive() && pb.hits == 0);
        assert!(pb.feet.z < 0.0, "blue respawns on blue's side: {}", pb.feet);
    }

    #[test]
    fn predicted_feet_are_limited_and_collide() {
        let (mut sim, a, _) = duel();
        let start = sim.player(a).unwrap().feet;
        // A claim 10 m away moves only a speed-limited step.
        sim.set_cmd(
            a,
            PlayerCmd {
                feet: Some(start + Vec3::new(10.0, 0.0, 0.0)),
                ..Default::default()
            },
        );
        sim.step();
        let moved = (sim.player(a).unwrap().feet - start).length();
        assert!(moved > 0.05 && moved < 0.4, "{moved}");
        // Claims into the border wall stop at the wall.
        for _ in 0..600 {
            let f = sim.player(a).unwrap().feet;
            sim.set_cmd(
                a,
                PlayerCmd {
                    feet: Some(f + Vec3::new(0.0, 0.0, 0.1)),
                    ..Default::default()
                },
            );
            sim.step();
        }
        let f = sim.player(a).unwrap().feet;
        assert!(f.z < lt_level_half() - 0.3, "{f}");
    }

    fn lt_level_half() -> f32 {
        crate::level::ArenaSize::Small.dims().half_length()
    }

    #[test]
    fn respawn_time_grows_with_distance() {
        let sim = Sim::new(3, Rules::default());
        let base = sim.level.base_centre(Team::Red);
        let near = sim.respawn_time(Team::Red, base);
        let mid = sim.respawn_time(Team::Red, Vec3::ZERO);
        let far = sim.respawn_time(Team::Red, sim.level.base_centre(Team::Blue));
        assert!((near - 5.0).abs() < 0.01 && (far - 15.0).abs() < 0.01 && near < mid && mid < far);
    }

    #[test]
    fn team_target_scores_for_the_enemy_only() {
        let mut sim = Sim::new(11, Rules::default());
        let blue = sim.add_player("b", Some(Team::Blue), false).unwrap();
        let red = sim.add_player("r", Some(Team::Red), false).unwrap();
        let t = *sim
            .level
            .targets
            .iter()
            .find(|t| t.team == Team::Red)
            .unwrap();
        let spot = sim.level.dims().cell_centre(t.rear_cell.0, t.rear_cell.1) + t.normal * 0.6;
        for (who, expect) in [(red, [0, 0]), (blue, [0, 100])] {
            let pi = sim.players.iter().position(|p| p.id == who).unwrap();
            sim.players[pi].feet = spot;
            let eye = sim.players[pi].eye();
            let dir = (t.centre() - eye).normalize();
            let cmd = PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: eye - spot,
                    rot: look_rot(dir),
                },
                fire: true,
                ..Default::default()
            };
            sim.set_cmd(who, cmd);
            sim.step();
            sim.set_cmd(who, PlayerCmd { fire: false, ..cmd });
            for _ in 0..30 {
                sim.step();
            }
            assert_eq!(sim.scores, expect);
            sim.players[pi].feet = Vec3::new(0.0, 0.0, 100.0);
        }
    }

    #[test]
    fn grenade_timeline_and_white_bolts() {
        let (mut sim, a, _b) = duel();
        let base = sim.level.base_centre(Team::Red);
        sim.players[0].grenades = 1;
        sim.set_cmd(
            a,
            PlayerCmd {
                throw: Some((Vec3::new(0.0, 1.2, -0.3), Vec3::new(0.0, 2.0, -3.0))),
                ..Default::default()
            },
        );
        sim.step();
        assert_eq!(sim.grenades.len(), 1);
        assert_eq!(sim.player(a).unwrap().grenades, 0);
        let mut armed_at = None;
        let mut bursts = 0;
        let mut max_y_active = 0.0f32;
        let mut ground_y = 0.0;
        for k in 0..(8.0 / TICK) as usize {
            sim.step();
            for e in sim.drain_events() {
                match e {
                    SimEvent::GrenadeArmed { pos } => {
                        armed_at = Some(k as f32 * TICK);
                        ground_y = pos.y;
                    }
                    SimEvent::GrenadeBurst { .. } => bursts += 1,
                    _ => {}
                }
            }
            if let Some(g) = sim.grenades.first()
                && matches!(g.phase, GrenadePhase::Active { .. })
            {
                max_y_active = max_y_active.max(g.pos.y);
            }
            if sim.bolts.iter().any(|b| b.team.is_none()) {
                assert!(
                    sim.bolts
                        .iter()
                        .filter(|b| b.team.is_none())
                        .all(|b| b.owner == a)
                );
            }
        }
        let armed = armed_at.unwrap();
        assert!((armed - 3.0).abs() < 0.05, "{armed}");
        assert!(sim.grenades.is_empty(), "gone after 3 + 4 s");
        assert!(
            (max_y_active - ground_y - 1.0).abs() < 0.1,
            "hops about a metre: {max_y_active} {ground_y}"
        );
        assert!(bursts >= 25, "{bursts}");
        let _ = base;
    }

    #[test]
    fn grenade_bolts_hit_team_mates() {
        let mut sim = Sim::new(5, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let mate = sim.add_player("m", Some(Team::Red), false).unwrap();
        let base = sim.level.base_centre(Team::Red);
        sim.players[0].feet = base + Vec3::new(0.0, 0.0, 30.0); // out of the way
        // Off the spawn pads (which would heal the hit).
        sim.players[1].feet = Vec3::new(0.0, 0.0, 6.0);
        let mi = 1;
        // A bolt from a grenade straight at the team mate.
        let eye = sim.players[mi].eye();
        sim.bolts.push(Bolt {
            id: 99,
            owner: a,
            owner_team: Team::Red,
            team: None,
            pos: eye + Vec3::X * 2.0,
            vel: Vec3::NEG_X * 40.0,
            life: 1.0,
            extra: BoltExtra::default(),
        });
        for _ in 0..10 {
            sim.step();
        }
        assert_eq!(sim.player(mate).unwrap().hits, 1);
        assert_eq!(sim.scores, [0, 0], "no points for hitting one's own team");
    }

    #[test]
    fn ammo_runs_out_and_refills_at_the_base() {
        let (mut sim, a, _) = duel();
        // Away from the spawn pads, holding the trigger every other tick: three shots a second.
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        let fire = |sim: &mut Sim, on: bool| {
            sim.set_cmd(
                a,
                PlayerCmd {
                    gun_hand: Some(Hand::Right),
                    muzzle: Pose {
                        pos: Vec3::Y * 1.4,
                        rot: Quat::IDENTITY,
                    },
                    fire: on,
                    ..Default::default()
                },
            );
            sim.step();
        };
        let mut fired = 0;
        let mut empty = 0;
        for k in 0..(10.0 / TICK) as usize {
            fire(&mut sim, k % 2 == 0);
            for e in sim.drain_events() {
                match e {
                    SimEvent::Fired { .. } => fired += 1,
                    SimEvent::Empty { .. } => empty += 1,
                    _ => {}
                }
            }
        }
        assert!((29..=31).contains(&fired), "three shots a second: {fired}");
        for _ in 0..(10.0 / TICK) as usize {
            fire(&mut sim, false);
            fire(&mut sim, true);
        }
        let shots: usize = sim
            .drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Fired { .. }))
            .count();
        assert_eq!(fired + shots, 50, "50 shots in a blaster");
        assert!(empty > 0 || sim.player(a).unwrap().ammo == 0);
        // Standing on a spawn pad refills it.
        let pad = sim.level.spawns[0][0].pos;
        sim.players[0].feet = pad;
        for _ in 0..(3.0 / TICK) as usize {
            fire(&mut sim, false);
        }
        assert_eq!(sim.player(a).unwrap().ammo, 50);
        let ev = sim.drain_events();
        assert!(ev.iter().any(|e| matches!(e, SimEvent::Reloading { .. })));
        assert!(ev.iter().any(|e| matches!(e, SimEvent::Reloaded { .. })));
        // And so does a reload station.
        assert!(sim.level.reloads.iter().all(|r| sim.level.reloads_at(*r)));
    }

    #[test]
    fn own_spawn_pads_heal() {
        let (mut sim, a, b) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[0].hits = 2;
        sim.players[1].hits = 1;
        // Blue on red's pads: still hurt.
        sim.players[1].feet = sim.level.spawns[Team::Red.index()][0].pos;
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 2);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        sim.players[0].feet = sim.level.spawns[Team::Red.index()][1].pos;
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 0);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Healed { player } if *player == a))
        );
    }

    /// Whether a body at `feet` is inside anything solid (beyond a little slack).
    fn embedded(world: &World, feet: Vec3) -> Option<crate::collide::Collider> {
        world
            .colliders
            .iter()
            .skip(1)
            .chain(&world.dynamic)
            .copied()
            .find(|c| {
                if matches!(c, crate::collide::Collider::Ramp { .. }) {
                    return false;
                }
                let (min, max) = c.bounds();
                if max.y <= feet.y + crate::collide::STEP || min.y >= feet.y + BODY_HEIGHT {
                    return false;
                }
                let n = Vec2::new(feet.x.clamp(min.x, max.x), feet.z.clamp(min.z, max.z));
                Vec2::new(feet.x, feet.z).distance(n) < BODY_RADIUS - 0.03
            })
    }

    #[test]
    fn sliding_panels_push_players_out_of_the_way() {
        let mut checked = 0;
        for seed in 0..10u64 {
            let mut sim = Sim::new(
                seed,
                Rules {
                    match_time: 1000.0,
                    ..Rules::default()
                },
            );
            let Some(mi) = sim
                .level
                .movers
                .iter()
                .position(|m| m.kind == MoverKind::Panel)
            else {
                continue;
            };
            let m = sim.level.movers[mi];
            let a = sim.add_player("a", Some(Team::Red), false).unwrap();
            // Wait for the panel to rest at the start of its track, then stand where it's going.
            while m.fraction(sim.tick) > 0.0 || m.fraction(sim.tick + 1) > 0.0 {
                sim.tick += 1;
            }
            let spot = m.pos + m.travel;
            sim.players[0].feet = spot;
            let mut moved: f32 = 0.0;
            for _ in 0..m.period() {
                sim.step();
                let f = sim.player(a).unwrap().feet;
                moved = moved.max(Vec2::new(f.x - spot.x, f.z - spot.z).length());
                assert!(
                    embedded(&sim.world, f).is_none(),
                    "seed {seed}: player at {f} inside {:?}",
                    embedded(&sim.world, f)
                );
                assert!(f.y.abs() < 0.05, "seed {seed}: still on the floor: {f}");
            }
            assert!(moved > 0.5, "seed {seed}: pushed only {moved} m");
            checked += 1;
        }
        assert!(checked >= 5, "{checked}");
    }

    #[test]
    fn doors_block_bolts_while_closed() {
        let mut checked = 0;
        for seed in 0..10u64 {
            let mut sim = Sim::new(
                seed,
                Rules {
                    match_time: 1000.0,
                    ..Rules::default()
                },
            );
            let Some(m) = sim
                .level
                .movers
                .iter()
                .find(|m| m.kind == MoverKind::Door)
                .copied()
            else {
                continue;
            };
            let across = if m.rot % 2 == 0 { Vec3::Z } else { Vec3::X };
            for open in [false, true] {
                let want = if open { 1.0 } else { 0.0 };
                while (0..90).any(|k| m.fraction(sim.tick + k) != want) {
                    sim.tick += 1;
                }
                sim.step();
                sim.drain_events();
                let from = m.pos + Vec3::Y * 1.5 - across * 1.5;
                sim.bolts.push(Bolt {
                    id: 1,
                    owner: 0,
                    owner_team: Team::Red,
                    team: Some(Team::Red),
                    pos: from,
                    vel: across * 40.0,
                    life: 2.0,
                    extra: BoltExtra::default(),
                });
                for _ in 0..4 {
                    sim.step();
                }
                let impact = sim.drain_events().into_iter().find_map(|e| match e {
                    SimEvent::BoltImpact { pos, .. } => Some(pos),
                    _ => None,
                });
                let at_door = impact.is_some_and(|q| (q - from).dot(across) < 1.6);
                assert_eq!(
                    at_door, !open,
                    "seed {seed}, door open {open}: impact {impact:?}, door at {}",
                    m.pos
                );
                sim.bolts.clear();
            }
            checked += 1;
        }
        assert!(checked >= 5, "{checked}");
    }

    /// A Blue player strafing across Red's base, a Red shooter 3.5 m away aiming where the
    /// target was `lag` ticks ago (leading it as it then moved); the command made at
    /// `view_tick` = now - `lag` if `compensated`, else 0. Returns the target's hits.
    fn lagged_shot(lag: u64, compensated: bool) -> u8 {
        let mut sim = Sim::new(
            7,
            Rules {
                pickup_pairs: 0,
                ..Rules::default()
            },
        );
        let a = sim.add_player("shooter", Some(Team::Red), false).unwrap();
        let b = sim.add_player("target", Some(Team::Blue), false).unwrap();
        // Somewhere open: the target walks 3 m along x in plain sight of the shooter.
        let start = Vec3::new(-2.5, 0.0, -3.0);
        let world = sim.level.world();
        let shooter = (0..sim.level.dims().cells())
            .map(|n| sim.level.nav.pos[n])
            .find(|&p| {
                let eye = p + Vec3::Y * EYE;
                (0..=14).all(|k| {
                    let q = p + start + Vec3::X * (k as f32 * 0.25);
                    sim.level
                        .dims()
                        .cell_at(q)
                        .is_some_and(|(x, z)| sim.level.nav.valid[sim.level.dims().idx(x, z)])
                        && world.clear(eye, q + Vec3::Y * 1.2)
                        && world.clear(q + Vec3::Y * 0.5, q + Vec3::new(0.6, 0.5, 0.0))
                        && world.clear(q + Vec3::Y * 0.5, q + Vec3::new(0.0, 0.5, 0.6))
                        && world.clear(q + Vec3::Y * 0.5, q + Vec3::new(0.0, 0.5, -0.6))
                })
            })
            .expect("an open spot");
        sim.players[0].feet = shooter;
        sim.players[1].feet = shooter + start;
        let walk = PlayerCmd {
            stick: Vec2::X,
            ..Default::default()
        };
        sim.set_cmd(b, walk);
        for _ in 0..30 {
            sim.step();
        }
        // The next step fires; the shooter's client showed the world `lag` ticks before it.
        let view = sim.tick + 1 - lag;
        let me = sim.player(a).unwrap();
        let muzzle = me.eye() - Vec3::Y * 0.2;
        let chest_at = |sim: &Sim, t: u64| {
            let h = sim.past(t).unwrap();
            let g = sim.history[h].1.iter().find(|g| g.id == b).unwrap();
            g.eye - Vec3::Y * (crate::hitbox::CHEST_DROP + crate::hitbox::CHEST_HALF.y)
        };
        // Lead the target as the shooter saw it move: the k-th tick of the bolt's flight meets
        // the target as it was at view + k.
        let step = sim.rules.blaster_bolt_speed * TICK;
        let aim = (0..lag)
            .map(|k| chest_at(&sim, view + k))
            .enumerate()
            .min_by(|(k1, c1), (k2, c2)| {
                let e = |k: usize, c: &Vec3| ((c.distance(muzzle) / step) - (k as f32 + 0.5)).abs();
                e(*k1, c1).total_cmp(&e(*k2, c2))
            })
            .unwrap()
            .1;
        let dir = (aim - muzzle).normalize();
        let me = sim.player(a).unwrap();
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: muzzle - me.feet,
                rot: look_rot(dir),
            },
            fire: true,
            view_tick: if compensated { view } else { 0 },
            ..Default::default()
        };
        sim.set_cmd(a, cmd);
        sim.step();
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..30 {
            sim.step();
        }
        sim.player(b).unwrap().hits
    }

    #[test]
    fn lag_compensation_hits_what_the_shooter_saw() {
        for lag in [6, 10, 14] {
            assert_eq!(
                lagged_shot(lag, true),
                1,
                "{lag} ticks behind, compensated: a hit"
            );
            assert_eq!(
                lagged_shot(lag, false),
                0,
                "{lag} ticks behind, uncompensated: a miss"
            );
        }
        // Beyond the history only `MAX_LAG` is made up: still nothing breaks.
        let _ = lagged_shot(19, true);
    }

    #[test]
    fn grenade_pickups_appear_and_are_taken() {
        let rules = Rules {
            powerup_pairs: 0,
            ..Rules::default()
        };
        let mut sim = Sim::new(4, rules);
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        assert_eq!(
            sim.player(a).unwrap().grenades,
            0,
            "players start without grenades"
        );
        for _ in 0..(10.0 / TICK) as usize {
            sim.step();
        }
        let pickups: Vec<Pickup> = sim.pickups().copied().collect();
        assert_eq!(pickups.len(), 4);
        assert_eq!(
            pickups.iter().filter(|p| p.pos.z > 0.0).count(),
            2,
            "two in each half"
        );
        let p = pickups[0];
        sim.players[0].feet = p.pos - Vec3::Y * PICKUP_HEIGHT;
        sim.step();
        assert_eq!(sim.player(a).unwrap().grenades, 1);
        assert_eq!(sim.pickups().count(), 3);
        // Only one carried: the next one stays.
        let q = sim.pickups().next().copied().unwrap();
        sim.players[0].feet = q.pos - Vec3::Y * PICKUP_HEIGHT;
        sim.step();
        assert_eq!(sim.pickups().count(), 3);
        for _ in 0..(17.0 / TICK) as usize {
            sim.step();
        }
        assert_eq!(sim.pickups().count(), 4, "the taken one comes back");
    }

    #[test]
    fn powerups_appear_in_both_halves_and_are_taken_once() {
        let mut sim = Sim::new(4, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        for _ in 0..(30.0 / TICK) as usize {
            sim.step();
        }
        for kind in PickupKind::POWERUPS {
            let of_kind: Vec<Pickup> = sim.pickups().filter(|p| p.kind == kind).copied().collect();
            assert_eq!(of_kind.len(), 2, "{kind:?}: one in each half");
            assert_eq!(of_kind.iter().filter(|p| p.pos.z > 0.0).count(), 1);
        }
        let rapid: Vec<Pickup> = sim
            .pickups()
            .filter(|p| p.kind == PickupKind::Rapid)
            .copied()
            .collect();
        sim.players[0].feet = rapid[0].pos - Vec3::Y * PICKUP_HEIGHT;
        sim.step();
        assert!(sim.player(a).unwrap().rapid);
        assert!(sim.drain_events().iter().any(|e| matches!(
            e,
            SimEvent::PickedUp {
                kind: PickupKind::Rapid,
                ..
            }
        )));
        // Already has it: the other stays.
        sim.players[0].feet = rapid[1].pos - Vec3::Y * PICKUP_HEIGHT;
        sim.step();
        assert!(sim.pickups().any(|p| p.id == rapid[1].id));
    }

    #[test]
    fn rapid_fire_doubles_the_rate_until_tagged_out() {
        let (mut sim, a, b) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.give(a, PickupKind::Rapid);
        let mut fired = 0;
        for k in 0..(5.0 / TICK) as usize {
            sim.set_cmd(
                a,
                PlayerCmd {
                    gun_hand: Some(Hand::Right),
                    muzzle: Pose {
                        pos: Vec3::Y * 1.4,
                        rot: Quat::IDENTITY,
                    },
                    fire: k % 2 == 0,
                    ..Default::default()
                },
            );
            sim.step();
            fired += sim
                .drain_events()
                .iter()
                .filter(|e| matches!(e, SimEvent::Fired { .. }))
                .count();
        }
        assert!((29..=31).contains(&fired), "six shots a second: {fired}");
        // Tagged out: gone.
        let bolt = Bolt {
            id: 1,
            owner: b,
            owner_team: Team::Blue,
            team: Some(Team::Blue),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        sim.players[0].hits = 2;
        sim.hit_player(0, &bolt, Vec3::ZERO);
        assert!(!sim.player(a).unwrap().alive());
        assert!(!sim.player(a).unwrap().rapid);
    }

    #[test]
    fn a_shield_stops_hits_for_its_time() {
        let (mut sim, a, b) = duel();
        sim.give(b, PickupKind::Shield);
        assert_eq!(sim.player(b).unwrap().shields, 1);
        sim.set_cmd(
            b,
            PlayerCmd {
                shield: true,
                ..Default::default()
            },
        );
        sim.step();
        sim.set_cmd(b, PlayerCmd::default());
        let pb = sim.player(b).unwrap();
        assert!(pb.shields == 0 && pb.shield > 9.9);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::ShieldUp { player } if *player == b))
        );
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 0, "shielded");
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Blocked { player, .. } if *player == b))
        );
        for _ in 0..(10.0 / TICK) as usize {
            sim.step();
        }
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::ShieldDown { player } if *player == b))
        );
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 1, "the shield ran out");
    }

    #[test]
    fn the_shotgun_sprays_short_lived_bolts_and_refills() {
        let (mut sim, a, _) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.players[0].ammo = 3;
        sim.give(a, PickupKind::Shotgun);
        let pa = sim.player(a).unwrap();
        assert!(pa.shotgun && pa.ammo == 50, "taking it fills it up");
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: Vec3::Y * 1.4,
                    rot: Quat::IDENTITY,
                },
                fire: true,
                ..Default::default()
            },
        );
        sim.step();
        assert_eq!(sim.player(a).unwrap().ammo, 49, "one shot");
        let mine: Vec<Bolt> = sim.bolts.iter().filter(|b| b.owner == a).copied().collect();
        assert_eq!(mine.len(), 10);
        let rules = &sim.rules;
        for b in &mine {
            let off = b.vel.normalize().angle_between(Vec3::NEG_Z);
            assert!(off <= rules.shotgun_spread + 1e-4, "{off}");
            assert!(b.life * rules.blaster_bolt_speed <= rules.shotgun_range + 0.01);
        }
        assert!(
            mine.iter()
                .any(|b| b.vel.normalize().angle_between(Vec3::NEG_Z) > 0.02),
            "spread out"
        );
        // Gone with the next tag-out.
        sim.players[0].lose_powerups();
        assert!(!sim.player(a).unwrap().shotgun);
    }

    #[test]
    fn a_cloak_shows_after_firing_and_hits_and_runs_out() {
        let (mut sim, a, b) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.give(a, PickupKind::Cloak);
        let pa = sim.player(a).unwrap();
        assert!(pa.cloaked());
        assert!(!pa.seen_from(CLOAK_FAR) && pa.seen_from(CLOAK_NEAR - 0.5));
        let fire = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: Vec3::Y * 1.4,
                rot: Quat::IDENTITY,
            },
            fire: true,
            ..Default::default()
        };
        sim.set_cmd(a, fire);
        sim.step();
        sim.set_cmd(
            a,
            PlayerCmd {
                fire: false,
                ..fire
            },
        );
        assert!(!sim.player(a).unwrap().cloaked(), "showing after firing");
        for _ in 0..(0.5 / TICK) as usize + 1 {
            sim.step();
        }
        assert!(sim.player(a).unwrap().cloaked(), "and hidden again");
        // A hit shows it too.
        let bolt = Bolt {
            id: 1,
            owner: b,
            owner_team: Team::Blue,
            team: Some(Team::Blue),
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        sim.hit_player(0, &bolt, Vec3::ZERO);
        assert!(!sim.player(a).unwrap().cloaked());
        sim.drain_events();
        for _ in 0..(100.0 / TICK) as usize {
            sim.step();
        }
        assert_eq!(sim.player(a).unwrap().cloak, 0.0);
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Uncloaked { player } if *player == a))
        );
    }

    fn mega_sim(turrets: bool) -> Sim {
        Sim::new(
            9,
            Rules {
                mode: crate::level::GameMode::Megatarget,
                pickup_pairs: 0,
                turret_range: if turrets { 24.0 } else { 0.0 },
                match_time: 1000.0,
                ..Rules::default()
            },
        )
    }

    /// Player `id` (standing where it is) fires one bolt at `point`, then waits half a second.
    fn fire_at(sim: &mut Sim, id: PlayerId, point: Vec3) {
        let me = sim.player(id).unwrap();
        let eye = me.eye();
        let dir = (point - eye).normalize();
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: eye - me.feet,
                rot: look_rot(dir),
            },
            fire: true,
            ..Default::default()
        };
        sim.set_cmd(id, cmd);
        sim.step();
        sim.set_cmd(id, PlayerCmd { fire: false, ..cmd });
        for _ in 0..30 {
            sim.step();
        }
    }

    fn walk(sim: &mut Sim, id: PlayerId, to: Vec3, secs: f32) {
        for _ in 0..(secs / TICK) as usize {
            let f = sim.player(id).unwrap().feet;
            let d = Vec2::new(to.x - f.x, to.z - f.z);
            let stick = if d.length() < 0.1 {
                Vec2::ZERO
            } else {
                d.normalize()
            };
            sim.set_cmd(
                id,
                PlayerCmd {
                    stick,
                    ..Default::default()
                },
            );
            sim.step();
        }
    }

    fn pi_of(sim: &Sim, id: PlayerId) -> usize {
        sim.players.iter().position(|p| p.id == id).unwrap()
    }

    #[test]
    fn megatarget_needs_both_secondaries_and_sends_home() {
        let mut sim = mega_sim(false);
        let m = sim.level.mega.clone().unwrap();
        assert!(sim.level.targets.is_empty(), "no team targets");
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let i = pi_of(&sim, a);
        // Into the left pocket, then the right one, shooting each secondary twice.
        for side in 0..2 {
            let t = m.secondaries[side];
            sim.players[i].feet = Vec3::new(t.centre().x + t.normal.x * 2.0, 0.0, 2.2);
            fire_at(&mut sim, a, t.centre());
            assert_eq!(sim.player(a).unwrap().mega_hits, (1u8 << (side + 1)) - 1);
            let score = sim.scores;
            fire_at(&mut sim, a, t.centre());
            assert_eq!(sim.scores, score, "a second hit doesn't count");
        }
        assert_eq!(sim.scores, [200, 0]);
        let p = sim.player(a).unwrap();
        assert!(p.power.is_some_and(|t| t > 55.0), "{:?}", p.power);
        // Through Red's forcefield into the room, then up at the megatarget.
        sim.players[i].feet = m.shields[0].outside;
        walk(&mut sim, a, m.shields[0].inside, 1.5);
        let p = sim.player(a).unwrap();
        assert!(p.alive() && m.in_room(p.feet), "{}", p.feet);
        sim.drain_events();
        fire_at(&mut sim, a, m.core.centre());
        let ev = sim.drain_events();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::MegaHit { by, .. } if *by == a))
        );
        let p = sim.player(a).unwrap();
        assert_eq!(sim.scores, [1200, 0]);
        assert!(p.power.is_none() && p.mega_hits == 0 && p.ammo == sim.rules.ammo);
        assert!(
            sim.level.reloads_at(p.feet),
            "back on a spawn pad: {}",
            p.feet
        );
        // Back to the doorway at once: the forcefield shocks again.
        sim.players[i].feet = m.shields[0].outside;
        walk(&mut sim, a, m.shields[0].inside, 1.5);
        assert!(
            !sim.player(a).unwrap().alive(),
            "both secondaries again first"
        );
    }

    #[test]
    fn forcefields_kill_and_block() {
        let mut sim = mega_sim(false);
        let m = sim.level.mega.clone().unwrap();
        let a = sim.add_player("a", Some(Team::Blue), false).unwrap();
        let i = pi_of(&sim, a);
        // Bolts and sight stop at the forcefield.
        let s = m.shields[1];
        let from = s.outside + Vec3::Y * 1.5;
        assert!(!sim.world.clear(from, s.inside + Vec3::Y * 1.5));
        // Walking in without the power: tagged out.
        sim.players[i].feet = s.outside;
        walk(&mut sim, a, s.inside, 1.5);
        assert!(!sim.player(a).unwrap().alive());
        let ev = sim.drain_events();
        assert!(ev.iter().any(|e| matches!(e, SimEvent::Shocked { .. })));
        assert!(
            ev.iter().any(
                |e| matches!(e, SimEvent::TaggedOut { by: HAZARD, player, .. } if *player == a)
            )
        );
        assert_eq!(sim.scores, [0, 0]);
        // In the room when the time runs out: tagged out too.
        let mut sim = mega_sim(false);
        let b = sim.add_player("b", Some(Team::Red), false).unwrap();
        let j = pi_of(&sim, b);
        sim.players[j].feet = m.shields[0].inside;
        sim.players[j].power = Some(0.2);
        sim.players[j].mega_hits = 3;
        for _ in 0..30 {
            sim.step();
        }
        let p = sim.player(b).unwrap();
        assert!(!p.alive() && p.power.is_none() && p.mega_hits == 0);
        // Outside it, only the power goes.
        let c = sim.add_player("c", Some(Team::Red), false).unwrap();
        let k = pi_of(&sim, c);
        sim.players[k].feet = m.shields[0].outside + Vec3::Z * 2.0;
        sim.players[k].power = Some(0.2);
        for _ in 0..30 {
            sim.step();
        }
        let p = sim.player(c).unwrap();
        assert!(p.alive() && p.power.is_none());
    }

    #[test]
    fn turrets_shoot_everyone_until_switched_off() {
        let mut sim = mega_sim(true);
        let m = sim.level.mega.clone().unwrap();
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let i = pi_of(&sim, a);
        // A place in front of the doorway's turrets with both switches in sight (none of the
        // upper floor in between).
        let t = m.turrets[0];
        let world = sim.level.world();
        let spot = (5..12)
            .map(|k| Vec3::new(t.pos.x + k as f32, 0.0, t.pos.z))
            .chain(sim.level.nav.pos.iter().copied())
            .filter(|p| p.distance(t.pos) < 14.0 && sim.level.nav.node_at(*p).is_some())
            .find(|p| {
                m.turrets[..2].iter().all(|u| {
                    let s = u.switch;
                    crate::level::box_seen(&world, *p + Vec3::Y * EYE, s.min, s.max, s.centre())
                })
            })
            .expect("a spot in sight of both switches");
        sim.players[i].feet = spot;
        sim.drain_events();
        for _ in 0..(2.5 / TICK) as usize {
            sim.step();
        }
        let ev = sim.drain_events();
        assert!(ev.iter().any(|e| matches!(e, SimEvent::TurretFired { .. })));
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::Hit { player, by: HAZARD, .. } if *player == a)),
            "{ev:?}"
        );
        // Both turrets over this doorway switched off: no more shots for a while.
        for _ in 0..(3.2 / TICK) as usize {
            sim.step();
        }
        sim.players[i].hits = 0;
        sim.players[i].cooldown = 0.0;
        for k in 0..2 {
            fire_at(&mut sim, a, m.turrets[k].switch.centre());
            sim.players[i].cooldown = 0.0;
        }
        assert!(sim.turrets[0].off > 10.0 && sim.turrets[1].off > 10.0);
        sim.drain_events();
        for _ in 0..(5.0 / TICK) as usize {
            sim.step();
        }
        assert!(
            !sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::TurretFired { turret: 0 | 1, .. }))
        );
        for _ in 0..(11.0 / TICK) as usize {
            sim.step();
        }
        assert_eq!(sim.turrets[0].off, 0.0);
    }

    #[test]
    fn turrets_wait_before_their_first_shot() {
        let mut sim = mega_sim(true);
        let m = sim.level.mega.clone().unwrap();
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let i = pi_of(&sim, a);
        let t = m.turrets[0];
        // Behind it until long past the turrets' start-up wait, then in front of it.
        sim.players[i].feet = Vec3::new(t.pos.x, 0.0, t.pos.z) - t.facing * 3.0;
        for _ in 0..(2.0 / TICK) as usize {
            sim.step();
        }
        assert!(
            !sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::TurretEngaged { turret: 0 }))
        );
        sim.players[i].feet = Vec3::new(t.pos.x + 7.0, 0.0, t.pos.z);
        let (mut engaged, mut fired) = (None, None);
        for n in 0..(2.0 / TICK) as usize {
            sim.step();
            for e in sim.drain_events() {
                match e {
                    SimEvent::TurretEngaged { turret: 0 } => engaged = engaged.or(Some(n)),
                    SimEvent::TurretFired { turret: 0, .. } => fired = fired.or(Some(n)),
                    _ => {}
                }
            }
        }
        let (engaged, fired) = (engaged.unwrap(), fired.unwrap());
        assert!(
            (fired - engaged) as f32 * TICK >= TURRET_WARMUP - TICK,
            "engaged at {engaged}, fired at {fired}"
        );
    }
}

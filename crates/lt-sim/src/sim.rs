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
use crate::level::{GameMode, Ground, Level, MoverKind, PAD_RADIUS, Panel};
use crate::rules::Rules;
use crate::shot::{BoltTarget, Contact, Gun, HitClaim, Shot, Trigger, bolt_contact, on_box};
use crate::types::*;

mod boxes;
mod consoles;
mod gadgets;
pub use boxes::{
    BoxPhase, BoxSnap, BoxState, LID_TIME, POKE_REACH, RISE_TIME, box_motion, head_hit,
};
pub use consoles::{ConsoleSnap, ConsoleState, VAULT_KINDS};
pub use gadgets::{GadgetSnap, GadgetState};

pub const TICK: f32 = 1.0 / 60.0;
pub const MAX_PLAYERS: usize = 32;
/// How close (horizontally) a player has to come to take a grenade pickup.
pub const PICKUP_REACH: f32 = 0.9;
/// Pickups hover this high above the floor.
pub const PICKUP_HEIGHT: f32 = 0.6;
/// The least speed (m/s) coming down that counts as landing (`SimEvent::Landed`): about a
/// jump's.
pub const LAND_SPEED: f32 = 3.0;
/// Default standing eye height.
pub const EYE: f32 = 1.65;
/// Seconds a turret waits before its first shot at a player it has just found, or after it
/// comes back on.
pub const TURRET_WARMUP: f32 = 0.5;
/// Ticks of player history kept for lag compensation (1 s), and the most a bolt is
/// compensated.
pub const HISTORY: usize = 60;
/// A cloaked player can't be seen from further than `CLOAK_FAR` (m), and is see-through, fading
/// in to `CLOAK_NEAR` (the bots and turrets see one closer than `CLOAK_NEAR`).
pub const CLOAK_NEAR: f32 = 3.0;
pub const CLOAK_FAR: f32 = 9.0;

/// How a cloaked player shows (m): fully from the first distance (where the bots see them), a
/// shimmer fading in from the second. Infection's phantoms (`phantom`) are shrouded more
/// thinly than a cloak pickup's wearer (`Rules::phantom_sight`).
pub fn cloak_fade(rules: &Rules, phantom: bool) -> (f32, f32) {
    if phantom {
        (rules.phantom_sight, rules.phantom_sight * 2.0)
    } else {
        (CLOAK_NEAR, CLOAK_FAR)
    }
}
/// (600 ms: also the oldest moment a client's hit claim may be about.)
pub const MAX_LAG: u64 = 36;

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
    /// Jumping (`jump_step`).
    pub moves: Moves,
    pub cmd: PlayerCmd,
    prev_fire: bool,
    /// A trigger pull waiting for the gun to be ready (`Gun::queued`).
    queued: bool,
    /// Dual pistols: the second's trigger, time to its next shot and pull waiting.
    prev_fire2: bool,
    fire_cd2: f32,
    queued2: bool,
    /// Hits taken since the last respawn.
    pub hits: u8,
    /// Time left of the blaster's lockout after a hit (no hits taken meanwhile either).
    pub cooldown: f32,
    /// Seconds left of the protection after respawning (`Rules::spawn_protect`).
    pub protect: f32,
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
    /// Megatarget mode: the megatarget energized for this player (its console solved; until
    /// they hit it or lose the power).
    pub core_live: bool,
    /// Powerups: rapid fire (until tagged out), shields carried, and the seconds left of the
    /// shield in use (0: none).
    pub rapid: bool,
    pub shields: u8,
    pub shield: f32,
    /// The gun in hand: the blaster, or a special one (until tagged out).
    pub weapon: Weapon,
    /// Seconds left of the cloak (0: none), and of showing after firing or a hit.
    pub cloak: f32,
    pub decloak: f32,
    /// Seconds left of the speed boost (0: none).
    pub speed: f32,
    /// Seconds left that an enemy pulse sensor shows this player to the other team.
    pub revealed: f32,
    /// How long the charge gun's trigger has been held (while it charges).
    pub charging: Option<f32>,
    /// The gadget carried, and how many.
    pub gadget: Option<(Gadget, u8)>,
    prev_gadget: bool,
    /// The med beam: seconds of beam left, the team mate it's on now, and how far along the
    /// next hit healed and shot refilled are.
    pub medbeam_left: f32,
    pub mending: Option<PlayerId>,
    mend: f32,
    mend_ammo: f32,
    /// A remote client judges this player's hits on others (`shot::HitClaim`), and the shots
    /// it fired lately to check them against.
    pub client_hits: bool,
    shot_log: std::collections::VecDeque<ShotRecord>,
    /// The command's `PlayerCmd::pokes` last seen (none yet: the first one only counts from).
    pokes: Option<u32>,
}

/// A shot whose hits a client judges, as the sim fired it.
#[derive(Clone, Debug)]
struct ShotRecord {
    seq: u32,
    /// The command's `view_tick`.
    view_tick: u64,
    team: Team,
    from: Vec3,
    /// Each bolt: velocity, life, extras, the sim's bolt id, and the players it struck.
    pellets: Vec<(Vec3, f32, BoltExtra, u32, u64)>,
}

/// Shots kept per player for checking claims.
const SHOT_LOG: usize = 24;

/// What became of a hit claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    Accepted,
    /// The shot hasn't been fired here yet (its command still on the way): ask again.
    Early,
    Rejected(&'static str),
}

impl Player {
    /// Cloaked and not showing right now.
    pub fn cloaked(&self) -> bool {
        self.cloak > 0.0 && self.decloak <= 0.0
    }

    /// Whether the bots and the turrets see this player from `dist` (m) away, `near` being
    /// how close a cloaked one has to come (`cloak_fade`).
    pub fn seen_from(&self, dist: f32, near: f32) -> bool {
        !self.cloaked() || dist < near
    }

    pub fn alive(&self) -> bool {
        self.respawn.is_none()
    }

    /// Least time between this player's shots.
    pub fn fire_interval(&self, rules: &Rules) -> f32 {
        self.gun().interval(rules)
    }

    /// The trigger's state.
    pub fn gun(&self) -> Gun {
        Gun {
            prev_fire: self.prev_fire,
            fire_cd: self.fire_cd,
            charging: self.charging,
            ammo: self.ammo,
            rapid: self.rapid,
            weapon: self.weapon,
            cooldown: self.cooldown,
            queued: self.queued,
        }
    }

    /// The second of dual pistols' trigger (the shots are shared).
    pub fn gun2(&self) -> Gun {
        Gun {
            prev_fire: self.prev_fire2,
            fire_cd: self.fire_cd2,
            queued: self.queued2,
            charging: None,
            ..self.gun()
        }
    }

    fn set_gun2(&mut self, g: Gun) {
        self.prev_fire2 = g.prev_fire;
        self.fire_cd2 = g.fire_cd;
        self.queued2 = g.queued;
        self.ammo = g.ammo;
    }

    fn set_gun(&mut self, g: Gun) {
        self.prev_fire = g.prev_fire;
        self.fire_cd = g.fire_cd;
        self.queued = g.queued;
        self.charging = g.charging;
        self.ammo = g.ammo;
    }

    /// As a bolt can meet them now.
    pub fn target(&self) -> BoltTarget<'_> {
        BoltTarget {
            id: self.id,
            team: self.team,
            alive: self.alive(),
            feet: self.feet,
            eye: self.eye(),
            cmd: &self.cmd,
        }
    }

    /// How fast the stick moves this player (m/s).
    pub fn move_speed(&self, rules: &Rules) -> f32 {
        move_speed(rules, self.speed > 0.0, self.cmd.crouch)
    }

    /// Whether walking over a pickup of this kind would take it (Infection's phantoms take
    /// none).
    pub fn wants(&self, kind: PickupKind, rules: &Rules) -> bool {
        if rules.mode == GameMode::Infection && self.team == Team::Red {
            return false;
        }
        match kind {
            PickupKind::Grenade => self.grenades < rules.grenade_max,
            PickupKind::Rapid => !self.rapid,
            PickupKind::Shield => self.shields == 0,
            PickupKind::Cloak => self.cloak <= 0.0,
            PickupKind::Speed => self.speed <= 0.0,
            // (A special gun replaces the one in hand; one's own refills it.)
            _ if let Some(w) = kind.weapon() => self.weapon != w || self.ammo < rules.ammo,
            // (A gadget replaces the one carried.)
            _ if let Some(g) = kind.gadget() => self.gadget.is_none_or(|(c, _)| c != g),
            _ => false,
        }
    }

    /// Gives this player what a pickup of this kind holds.
    fn take(&mut self, kind: PickupKind, rules: &Rules) {
        match kind {
            PickupKind::Grenade => self.grenades += 1,
            PickupKind::Rapid => self.rapid = true,
            PickupKind::Shield => self.shields += 1,
            PickupKind::Cloak => self.cloak = rules.cloak_time,
            PickupKind::Speed => self.speed = rules.speed_time,
            _ if let Some(w) = kind.weapon() => {
                self.weapon = w;
                self.charging = None;
                self.ammo = rules.ammo;
                self.reloading = None;
            }
            _ if let Some(g) = kind.gadget() => {
                self.gadget = Some((g, 1));
                if g == Gadget::Medbeam {
                    self.medbeam_left = rules.medbeam_time;
                }
            }
            _ => {}
        }
    }

    /// The gadget carried, if any.
    pub fn gadget_kind(&self) -> Option<Gadget> {
        self.gadget.map(|(g, _)| g)
    }

    /// One of the carried gadgets used up.
    fn use_gadget(&mut self) {
        if let Some((g, n)) = self.gadget {
            self.gadget = (n > 1).then(|| (g, n - 1));
        }
    }

    /// Tagged out: the powerups are gone.
    fn lose_powerups(&mut self) {
        self.rapid = false;
        self.shields = 0;
        self.shield = 0.0;
        self.weapon = Weapon::Blaster;
        self.cloak = 0.0;
        self.decloak = 0.0;
        self.speed = 0.0;
        self.charging = None;
        self.gadget = None;
        self.medbeam_left = 0.0;
        self.mending = None;
        self.mend = 0.0;
    }

    pub fn eye(&self) -> Vec3 {
        self.feet + self.head_offset()
    }

    fn head_offset(&self) -> Vec3 {
        crate::shot::head_offset(self.cmd.head.pos)
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
/// `half` after `wait` seconds (a powerup's kind rolled from `pool` each time).
#[derive(Clone, Copy, Debug)]
struct PickupSlot {
    half: Team,
    kind: PickupKind,
    pool: Option<Pool>,
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
    /// Bounces off every wall (the ricochet rifle).
    pub ricochet: bool,
    /// The prism lance's shot: gone once its tick's way is clear, a head hit counts extra.
    pub lance: bool,
    /// A remote client judges this bolt's hits on players (`shot::HitClaim`); the sim's copy
    /// stops at them without hitting. Its shot (`PlayerCmd::shot`) and pellet.
    pub client: bool,
    pub shot: u32,
    pub pellet: u8,
}

impl Default for BoltExtra {
    fn default() -> Self {
        BoltExtra {
            bounces: 0,
            damage: 1,
            pierce: false,
            struck: 0,
            ricochet: false,
            lance: false,
            client: false,
            shot: 0,
            pellet: 0,
        }
    }
}

/// A pulse sensor: flying (`stuck` false) or stuck to the arena at `pos`, pulsing every
/// `Rules::sensor_every` seconds for `left` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sensor {
    pub id: u32,
    pub team: Team,
    pub owner: PlayerId,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Stuck, and the normal of what it stuck to.
    pub stuck: bool,
    pub normal: Vec3,
    pub left: f32,
    pub next_pulse: f32,
}

/// A decoy: a copy of player `owner` as it was when sent off (its `cmd`: head, hands, gun),
/// running along `yaw` until popped or its time's up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoy {
    pub id: u32,
    pub team: Team,
    pub owner: PlayerId,
    pub feet: Vec3,
    pub vy: f32,
    pub cmd: PlayerCmd,
    pub left: f32,
}

impl Decoy {
    pub fn eye(&self) -> Vec3 {
        self.feet + crate::shot::head_offset(self.cmd.head.pos)
    }

    pub fn chest(&self) -> Vec3 {
        self.eye() - Vec3::Y * (crate::hitbox::CHEST_DROP + crate::hitbox::CHEST_HALF.y)
    }

    /// As a bolt can meet it (as its owner: its own team's bolts pass).
    pub fn target(&self) -> BoltTarget<'_> {
        BoltTarget {
            id: self.owner,
            team: self.team,
            alive: true,
            feet: self.feet,
            eye: self.eye(),
            cmd: &self.cmd,
        }
    }

    pub fn snap(&self) -> DecoySnap {
        DecoySnap {
            id: self.id,
            team: self.team,
            owner: self.owner,
            feet: self.feet,
            yaw: self.cmd.yaw,
            head: self.cmd.head,
            hands: self.cmd.hands,
            gun_hand: self.cmd.gun_hand,
        }
    }
}

/// A decoy as a client draws it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecoySnap {
    pub id: u32,
    pub team: Team,
    pub owner: PlayerId,
    pub feet: Vec3,
    pub yaw: f32,
    pub head: Pose,
    pub hands: [Pose; 2],
    pub gun_hand: Option<Hand>,
}

/// Prism mode: a team's prism, at home, carried, flying (thrown or dropped) or lying on the
/// floor (`dropped`: seconds until it goes home).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prism {
    pub team: Team,
    pub home: Vec3,
    pub pos: Vec3,
    pub vel: Vec3,
    pub carrier: Option<PlayerId>,
    pub dropped: Option<f32>,
    /// Thrown by: none of that team may catch it back for a moment.
    #[serde(default)]
    pub thrower: Option<(PlayerId, f32)>,
}

impl Prism {
    pub fn at_home(&self) -> bool {
        self.carrier.is_none() && self.dropped.is_none() && self.vel == Vec3::ZERO
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
    /// A mirror (`Gadget::Mirror`): turns back every bolt instead of stopping the enemy's.
    #[serde(default)]
    pub mirror: bool,
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
    /// The turret boxes (as `Level::boxes`).
    #[serde(default)]
    pub boxes: Vec<BoxSnap>,
    /// The consoles (as `console::console_defs`).
    #[serde(default)]
    pub consoles: Vec<ConsoleSnap>,
    /// Megatarget mode: the secondary targets' shooting puzzles (`crate::gadgets`).
    #[serde(default)]
    pub gadgets: Vec<GadgetSnap>,
    /// Barriers standing.
    pub barriers: Vec<Barrier>,
    /// Pulse sensors thrown.
    #[serde(default)]
    pub sensors: Vec<Sensor>,
    /// Decoys running.
    #[serde(default)]
    pub decoys: Vec<DecoySnap>,
    /// Prism mode: the prisms (Red's, Blue's).
    #[serde(default)]
    pub prisms: Vec<Prism>,
    /// The arena event going on (seconds left), and whether points count double.
    pub event: Option<(ArenaEvent, f32)>,
    pub double: bool,
    #[serde(default)]
    pub phase: MatchPhase,
    /// `Sim::bounty`.
    #[serde(default)]
    pub bounty: Option<PlayerId>,
    /// `Sim::infected`.
    #[serde(default)]
    pub infected: bool,
}

impl Snapshot {
    /// The seconds the arena's clocks show: the warmup's left while it runs (the match's clock
    /// stands meanwhile), else the match's.
    pub fn clock(&self) -> f32 {
        match self.phase {
            MatchPhase::Warmup(left) => left,
            _ => self.time_left,
        }
        .max(0.0)
    }
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
    /// `Player::protect`.
    #[serde(default)]
    pub protect: f32,
    /// Seconds until the player may dash again (`Moves::dash_ready`).
    #[serde(default)]
    pub dash_ready: f32,
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
    /// Megatarget mode (`Player::mega_hits`, `Player::power`, `Player::core_live`).
    pub mega_hits: u8,
    pub power: Option<f32>,
    #[serde(default)]
    pub core_live: bool,
    /// Powerups (`Player::rapid`, `shields`, `shield`, `weapon`).
    pub rapid: bool,
    pub shields: u8,
    pub shield: f32,
    pub weapon: Weapon,
    /// `Player::cloak`, `decloak`.
    pub cloak: f32,
    pub decloak: f32,
    /// `Player::speed`.
    pub speed: f32,
    /// `Player::revealed`.
    #[serde(default)]
    pub revealed: f32,
    /// `Player::charging` (0: not charging).
    pub charging: f32,
    /// `Player::gadget`.
    pub gadget: Option<(Gadget, u8)>,
    /// `PlayerCmd::gun2_hand`.
    #[serde(default)]
    pub gun2_hand: Option<Hand>,
    /// `Player::medbeam_left`, `mending`.
    #[serde(default)]
    pub medbeam_left: f32,
    #[serde(default)]
    pub mending: Option<PlayerId>,
    /// The command's `PlayerCmd::shot` (the client's shots this state includes).
    #[serde(default)]
    pub shot: u32,
}

impl PlayerSnap {
    /// Whether the player has this powerup now (carried or in use).
    pub fn has(&self, kind: PickupKind) -> bool {
        match kind {
            PickupKind::Grenade => self.grenades > 0,
            PickupKind::Rapid => self.rapid,
            PickupKind::Shield => self.shields > 0 || self.shield > 0.0,
            PickupKind::Cloak => self.cloak > 0.0,
            PickupKind::Speed => self.speed > 0.0,
            _ if let Some(w) = kind.weapon() => self.weapon == w,
            _ if let Some(g) = kind.gadget() => self.gadget.is_some_and(|(c, _)| c == g),
            _ => false,
        }
    }

    /// The gadget carried, if any.
    pub fn gadget_kind(&self) -> Option<Gadget> {
        self.gadget.map(|(g, _)| g)
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
    pub owner: PlayerId,
    /// Walls bounced off so far.
    #[serde(default)]
    pub bounces: u8,
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
    /// Warming up, counting down or under way (`begin`).
    pub phase: MatchPhase,
    /// Per target (by id): the tick and shooter of its last hit (the bots' alarm; events are
    /// drained by the caller, so bots can't see them).
    pub last_target_hit: Vec<Option<(u64, PlayerId)>>,
    pub turrets: Vec<TurretState>,
    /// The turret boxes (as `Level::boxes`).
    pub boxes: Vec<BoxState>,
    /// The consoles' puzzles (as `console_defs`).
    pub consoles: Vec<ConsoleState>,
    /// Megatarget mode: how each secondary target charges players (`console::secondary_kinds`).
    pub secondary: [crate::rules::SecondaryKind; 2],
    /// Megatarget mode: the secondary targets' shooting puzzles, per side.
    pub gadgets: Vec<GadgetState>,
    console_defs: Arc<Vec<crate::console::ConsoleDef>>,
    /// Barriers standing.
    pub barriers: Vec<Barrier>,
    /// Pulse sensors thrown.
    pub sensors: Vec<Sensor>,
    /// Decoys running.
    pub decoys: Vec<Decoy>,
    /// Prism mode: the prisms (Red's, Blue's).
    pub prisms: Vec<Prism>,
    /// The arena event going on, with its seconds left; when the next one comes (seconds into
    /// the match); and whether points count double now.
    pub event: Option<(ArenaEvent, f32)>,
    pub next_event: f32,
    pub double: bool,
    /// The last arena event (the next is another).
    last_event: Option<ArenaEvent>,
    /// The player with a bounty on them (`ArenaEvent::Bounty`).
    pub bounty: Option<PlayerId>,
    /// Infection: the phantoms have been picked, and seconds to the survivors' next points.
    pub infected: bool,
    survive_t: f32,
    /// Every player's hit-relevant state at the end of each of the last `HISTORY` ticks.
    history: std::collections::VecDeque<(u64, Vec<Ghost>)>,
    events: Vec<SimEvent>,
    rng: ChaCha8Rng,
    /// The turret boxes' puzzles' seeds (apart, so the rest plays out as without them).
    box_rng: ChaCha8Rng,
    /// The consoles' puzzles' seeds and what they lay out (apart, as `box_rng`).
    console_rng: ChaCha8Rng,
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
        let secondary = crate::console::secondary_kinds(&level, &rules);
        let world = level.world_at(0);
        let max_dist = (level.base_centre(Team::Red) - level.base_centre(Team::Blue))
            .length()
            .max(1.0);
        let grenades = (0..rules.pickup_pairs as usize * 2).map(|i| PickupSlot {
            half: if i % 2 == 0 { Team::Red } else { Team::Blue },
            kind: PickupKind::Grenade,
            pool: None,
            pickup: None,
            wait: 1.0 + (i / 2) as f32 * 2.0,
        });
        // Powerups come a little later, one slot after the other.
        let powerups = Pool::ALL
            .iter()
            .flat_map(|&pool| std::iter::repeat_n(pool, pool.slots()))
            .enumerate()
            .flat_map(|(k, pool)| {
                (0..rules.powerup_pairs as usize * 2).map(move |i| PickupSlot {
                    half: if i % 2 == 0 { Team::Red } else { Team::Blue },
                    kind: pool.kinds().next().unwrap_or(PickupKind::Grenade),
                    pool: Some(pool),
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
            phase: MatchPhase::Live,
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
            boxes: Vec::new(),
            consoles: Vec::new(),
            console_defs: Arc::new(Vec::new()),
            secondary,
            gadgets: Vec::new(),
            barriers: Vec::new(),
            sensors: Vec::new(),
            decoys: Vec::new(),
            prisms: Vec::new(),
            event: None,
            next_event: 0.0,
            double: false,
            last_event: None,
            bounty: None,
            infected: false,
            survive_t: 0.0,
            history: std::collections::VecDeque::with_capacity(HISTORY + 1),
            events: Vec::new(),
            rng: ChaCha8Rng::seed_from_u64(level.seed ^ 0x5EED),
            box_rng: ChaCha8Rng::seed_from_u64(level.seed ^ 0xB0C5),
            console_rng: ChaCha8Rng::seed_from_u64(level.seed ^ 0xC025),
            next_id: 1,
            max_dist,
            level,
        };
        let (lo, hi) = sim.rules.event_first;
        sim.next_event = sim.rng.random_range(lo..=hi.max(lo));
        sim.init_boxes();
        sim.init_consoles();
        sim.init_gadgets();
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
        // (Infection under way: late comers are phantoms.)
        let team = if self.infected { Team::Red } else { team };
        let mut p = Player {
            id,
            name: name.chars().take(24).collect(),
            team,
            bot,
            feet: Vec3::ZERO,
            vy: 0.0,
            air: Vec2::ZERO,
            moves: Moves::default(),
            cmd: PlayerCmd::default(),
            prev_fire: false,
            queued: false,
            prev_fire2: false,
            fire_cd2: 0.0,
            queued2: false,
            hits: 0,
            cooldown: 0.0,
            protect: 0.0,
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
            // (A client's prediction from before, maybe in the last arena, isn't taken.)
            claim_block: 30,
            mega_hits: 0,
            power: None,
            core_live: false,
            rapid: false,
            shields: 0,
            shield: 0.0,
            weapon: Weapon::Blaster,
            cloak: 0.0,
            decloak: 0.0,
            speed: 0.0,
            revealed: 0.0,
            charging: None,
            gadget: None,
            prev_gadget: false,
            medbeam_left: 0.0,
            mending: None,
            mend: 0.0,
            mend_ammo: 0.0,
            client_hits: false,
            shot_log: std::collections::VecDeque::new(),
            pokes: None,
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

    /// Moves player `id` over to `team`: they start over at its base (back from being tagged
    /// out if they were), their powerups gone and a carried prism dropped where they stood.
    /// Nothing happens (false) on the same team, or in Infection, where the game hands out the
    /// sides.
    pub fn switch_team(&mut self, id: PlayerId, team: Team) -> bool {
        if self.infection() {
            return false;
        }
        let Some(pi) = self.players.iter().position(|p| p.id == id) else {
            return false;
        };
        if self.players[pi].team == team {
            return false;
        }
        let feet = self.players[pi].feet;
        if let Some(k) = self.prisms.iter().position(|x| x.carrier == Some(id)) {
            self.drop_prism(k, feet + Vec3::Y * 0.3);
        }
        let p = &mut self.players[pi];
        p.team = team;
        p.respawn = None;
        p.grenades = 0;
        p.hit_by.clear();
        p.cmd.fire = false;
        p.cmd.fire2 = false;
        p.lose_powerups();
        self.send_home(pi);
        self.events
            .push(SimEvent::SwitchedTeam { player: id, team });
        true
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

    /// Starts the match off: with the warmup (`Rules::warmup`) or the countdown
    /// (`Rules::countdown`), if any (a new sim is under way at once).
    pub fn begin(&mut self) {
        if self.rules.warmup > 0.0 {
            self.phase = MatchPhase::Warmup(self.rules.warmup);
        } else {
            self.start_countdown();
        }
    }

    fn start_countdown(&mut self) {
        if self.rules.countdown > 0.0 {
            self.phase = MatchPhase::Countdown(self.rules.countdown);
            self.events.push(SimEvent::Countdown {
                n: self.rules.countdown.ceil() as u8,
            });
        } else {
            self.phase = MatchPhase::Live;
            self.events.push(SimEvent::Go);
        }
    }

    /// Ends the warmup (its time up, or everyone's here): whatever happened in it is forgotten,
    /// everyone starts over at their base, and the countdown begins.
    pub fn end_warmup(&mut self) {
        if !matches!(self.phase, MatchPhase::Warmup(_)) {
            return;
        }
        self.scores = [0; 2];
        self.bolts.clear();
        self.grenades.clear();
        self.barriers.clear();
        self.sensors.clear();
        self.decoys.clear();
        self.prisms.clear();
        self.event = None;
        self.double = false;
        self.time_left = self.rules.match_time;
        for i in 0..self.players.len() {
            let p = &mut self.players[i];
            p.score = 0;
            p.tags = 0;
            p.tagged = 0;
            p.shots = 0;
            p.landed = 0;
            p.target_hits = 0;
            p.assists = 0;
            p.defends = 0;
            p.grenade_tags = 0;
            p.cloak_tags = 0;
            p.hit_by.clear();
            p.respawn = None;
            p.grenades = 0;
            p.lose_powerups();
            self.send_home(i);
        }
        self.last_target_hit.fill(None);
        self.start_countdown();
    }

    pub fn step(&mut self) {
        self.tick += 1;
        if self.over {
            return;
        }
        match self.phase {
            MatchPhase::Warmup(t) => {
                if t - TICK <= 0.0 {
                    self.end_warmup();
                } else {
                    self.phase = MatchPhase::Warmup(t - TICK);
                }
            }
            MatchPhase::Countdown(t) => {
                let t = t - TICK;
                if t <= 0.0 {
                    self.phase = MatchPhase::Live;
                    self.events.push(SimEvent::Go);
                } else {
                    if t.ceil() < (t + TICK).ceil() {
                        self.events.push(SimEvent::Countdown { n: t.ceil() as u8 });
                    }
                    self.phase = MatchPhase::Countdown(t);
                    // Moving about, nothing else.
                    self.step_movers();
                    self.step_players();
                    self.record();
                    return;
                }
            }
            MatchPhase::Live => {}
        }
        if matches!(self.phase, MatchPhase::Warmup(_)) {
            // (The clock stands.)
        } else {
            self.time_left -= TICK;
        }
        self.step_infection();
        if self.over {
            return;
        }
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
        self.step_prisms();
        self.step_players();
        self.step_medbeams();
        self.step_gates();
        self.step_mega();
        self.step_pokes();
        self.step_boxes();
        self.step_consoles();
        self.step_gadgets();
        self.step_bolts();
        self.step_grenades();
        self.step_barriers();
        self.step_sensors();
        self.step_decoys();
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
        // (The light bridges standing are moving pieces too.)
        let bridges = self.bridge_colliders();
        if self.level.movers.is_empty() {
            if !bridges.is_empty() || !self.world.dynamic.is_empty() {
                self.world.set_dynamic(bridges);
            }
            return;
        }
        let level = self.level.clone();
        let tick = self.tick;
        let mut all = level.mover_colliders(tick);
        all.extend(bridges.iter().cloned());
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
                .chain(bridges.iter().cloned())
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

    /// A kind from `pool` for a slot in `half`: one not lying in that half yet if there is
    /// one.
    fn roll_kind(&mut self, pool: Pool, half: Team) -> PickupKind {
        let lying: Vec<PickupKind> = self
            .slots
            .iter()
            .filter(|s| s.half == half)
            .filter_map(|s| s.pickup.map(|p| p.kind))
            .collect();
        let all: Vec<PickupKind> = pool.kinds().collect();
        let fresh: Vec<PickupKind> = all.iter().copied().filter(|k| !lying.contains(k)).collect();
        let from = if fresh.is_empty() { &all } else { &fresh };
        from[self.rng.random_range(0..from.len())]
    }

    fn step_pickups(&mut self) {
        for i in 0..self.slots.len() {
            match self.slots[i].pickup {
                Some(pk) => {
                    let rules = &self.rules;
                    let phantoms = self.level.mode == GameMode::Infection;
                    let taker = self.players.iter_mut().find(|p| {
                        p.alive()
                            && !(phantoms && p.team == Team::Red)
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
                        if let Some(pool) = self.slots[i].pool {
                            self.slots[i].kind = self.roll_kind(pool, self.slots[i].half);
                        }
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
        let gravity = self.gravity();
        let hold = matches!(self.phase, MatchPhase::Countdown(_));
        for i in 0..self.players.len() {
            // (A prism's carrier has its hands full: no firing.)
            let carrying = self.carrying(self.players[i].id);
            if hold {
                let c = &mut self.players[i].cmd;
                c.fire = false;
                c.throw = None;
                c.shield = false;
                c.gadget = false;
            }
            // Timers.
            let respawn_now = {
                let p = &mut self.players[i];
                p.cooldown = (p.cooldown - TICK).max(0.0);
                // (Exactly 0 at the end, not a rounding error above it.)
                p.protect = if p.protect > TICK * 1.5 {
                    p.protect - TICK
                } else {
                    0.0
                };
                let mut gun = p.gun();
                gun.tick();
                p.set_gun(gun);
                let mut gun = p.gun2();
                gun.tick();
                p.set_gun2(gun);
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
                let s = match self.beacon_spawn(team) {
                    Some(s) => s,
                    None => self.pick_spawn(team),
                };
                let p = &mut self.players[i];
                p.respawn = None;
                p.hits = 0;
                p.cooldown = 0.0;
                p.protect = rules.spawn_protect;
                p.ammo = rules.ammo;
                p.grenades = 0;
                p.lose_powerups();
                p.reloading = None;
                p.feet = s.pos;
                p.vy = 0.0;
                p.air = Vec2::ZERO;
                // (Presses while tagged out don't count.)
                p.moves = Moves {
                    jumps: Some(p.cmd.jumps),
                    dashes: Some(p.cmd.dashes),
                    ..Moves::default()
                };
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
                p.prev_fire2 = p.cmd.fire2;
                p.queued = false;
                p.queued2 = false;
                continue;
            }

            // Movement.
            self.players[i].claim_block = self.players[i].claim_block.saturating_sub(1);
            {
                let p = &mut self.players[i];
                if let Some(up) = jump_step(&rules, &mut p.moves, p.cmd.jumps, p.vy, p.air, TICK) {
                    p.vy = up;
                    self.events.push(SimEvent::Jumped {
                        player: p.id,
                        pos: p.feet,
                    });
                }
            }
            let dash = {
                let p = &mut self.players[i];
                let (dash, started) = dash_step(
                    &rules,
                    &mut p.moves,
                    p.cmd.dashes,
                    p.cmd.stick,
                    p.cmd.yaw,
                    p.air,
                    TICK,
                );
                if started {
                    self.events.push(SimEvent::Dashed {
                        player: p.id,
                        pos: p.feet,
                    });
                }
                dash
            };
            let (feet, vy) = {
                let p = &self.players[i];
                let d = match p.cmd.feet {
                    Some(c) if c.is_finite() && p.claim_block == 0 => {
                        // A predicted position: as far as the speed limit allows (with slack for
                        // ticks arriving unevenly; a dash's while one is under way).
                        let dashing = if dash.is_some() {
                            rules.dash_speed
                        } else {
                            0.0
                        };
                        let max =
                            (p.move_speed(&rules) + rules.walk_speed_max + dashing) * TICK * 2.5;
                        Vec2::new(c.x - p.feet.x, c.z - p.feet.z).clamp_length_max(max)
                    }
                    _ => {
                        let stick = match dash {
                            Some(v) => v * TICK,
                            None => {
                                p.cmd.stick.clamp_length_max(1.0)
                                    * p.move_speed(&rules)
                                    * TICK
                                    * air_control(p.air)
                            }
                        };
                        let walk = p.cmd.walk.clamp_length_max(rules.walk_speed_max * TICK);
                        stick + walk + p.air * TICK
                    }
                };
                let d = if d.is_finite() { d } else { Vec2::ZERO };
                // (A jump pad's flight at full gravity: it's aimed for that.)
                let g = if p.air == Vec2::ZERO {
                    gravity
                } else {
                    GRAVITY
                };
                self.world.move_body_g(p.feet, d, p.vy, TICK, g)
            };
            let reload_here = self.level.reloads_at(feet);
            let home = self.level.spawn_at(feet);
            // The enemy's spawn pads tag a player out at once.
            if home.is_some_and(|t| t != self.players[i].team) {
                let p = &mut self.players[i];
                p.feet = feet;
                p.vy = vy;
                self.zap(i, TagHow::Pad);
                continue;
            }
            let p = &mut self.players[i];
            let fall = -p.vy;
            let from = p.feet;
            p.feet = feet;
            p.vy = vy;
            if vy == 0.0 && fall > LAND_SPEED {
                self.events.push(SimEvent::Landed {
                    player: p.id,
                    pos: feet,
                    speed: fall,
                });
            }
            // Jump pads: down again, and thrown up.
            if let Some((vy, air)) = pad_step(&self.level, from, feet, PAD_RADIUS, vy, &mut p.air) {
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
                p.protect = 0.0;
                self.events.push(SimEvent::ShieldUp { player: p.id });
            } else if p.shield > 0.0 {
                p.shield -= TICK;
                if p.shield <= 0.0 {
                    p.shield = 0.0;
                    self.events.push(SimEvent::ShieldDown { player: p.id });
                }
            }

            // Using the carried gadget (on pressing).
            let press = p.cmd.gadget && !p.prev_gadget;
            p.prev_gadget = p.cmd.gadget;
            if press && let Some((g @ (Gadget::Barrier | Gadget::Mirror), _)) = p.gadget {
                p.use_gadget();
                p.protect = 0.0;
                let id = self.id();
                let mirror = g == Gadget::Mirror;
                let b = Barrier {
                    id,
                    mirror,
                    left: if mirror {
                        rules.mirror_time
                    } else {
                        rules.barrier_time
                    },
                    ..self.barrier_for(i)
                };
                self.barriers.push(b);
                self.events.push(SimEvent::BarrierUp {
                    id,
                    player: self.players[i].id,
                    pos: b.centre,
                });
            }
            if press && let Some((Gadget::Sensor, _)) = self.players[i].gadget {
                let p = &mut self.players[i];
                p.use_gadget();
                p.protect = 0.0;
                let eye = p.eye();
                let (_, dir) = crate::shot::muzzle(p.feet, eye, &p.cmd);
                let from = eye + dir * 0.3;
                // (From the eye, unless that's inside a wall.)
                let from = if self.world.clear(eye, from) {
                    from
                } else {
                    eye
                };
                let (owner, team) = (p.id, p.team);
                let id = self.id();
                self.sensors.push(Sensor {
                    id,
                    team,
                    owner,
                    pos: from,
                    vel: dir * rules.sensor_speed + Vec3::Y * 1.5,
                    stuck: false,
                    normal: Vec3::Y,
                    left: rules.sensor_time,
                    next_pulse: 0.0,
                });
                self.events.push(SimEvent::SensorThrown {
                    id,
                    player: owner,
                    pos: from,
                });
            }
            if press && let Some((Gadget::Decoy, _)) = self.players[i].gadget {
                let p = &mut self.players[i];
                p.use_gadget();
                let (owner, team, feet) = (p.id, p.team, p.feet);
                let cmd = PlayerCmd {
                    yaw: p.cmd.yaw,
                    head: p.cmd.head,
                    hands: p.cmd.hands,
                    gun_hand: p.cmd.gun_hand,
                    muzzle: p.cmd.muzzle,
                    ..PlayerCmd::default()
                };
                let id = self.id();
                self.decoys.push(Decoy {
                    id,
                    team,
                    owner,
                    feet,
                    vy: 0.0,
                    cmd,
                    left: rules.decoy_time,
                });
                self.events.push(SimEvent::DecoyOut {
                    id,
                    player: owner,
                    pos: feet,
                });
            }
            let p = &mut self.players[i];

            // A pulse sensor's showing running out.
            p.revealed = (p.revealed - TICK).max(0.0);

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

            // Firing (`Gun::trigger`; dual pistols: each, the first's shot paid for before the
            // second's trigger, and numbered first when both fire at once).
            let mut gun = p.gun();
            let trigger = gun.trigger(p.cmd.fire, p.cmd.gun_hand.is_some() && !carrying, &rules);
            p.set_gun(gun);
            let mut triggers = vec![(trigger, false)];
            if p.weapon == Weapon::Dual {
                let mut gun2 = p.gun2();
                if let Some(Trigger::Fire(s)) = trigger {
                    gun2.ammo = gun2.ammo.saturating_sub(s.cost(&rules));
                }
                let t2 = gun2.trigger(p.cmd.fire2, p.cmd.gun2_hand.is_some() && !carrying, &rules);
                p.prev_fire2 = gun2.prev_fire;
                p.queued2 = gun2.queued;
                triggers.push((t2, true));
            } else {
                p.prev_fire2 = p.cmd.fire2;
                p.queued2 = false;
            }
            let player = p.id;
            let shots = triggers
                .iter()
                .filter(|t| matches!(t.0, Some(Trigger::Fire(_))))
                .count() as u32;
            let mut k = 0;
            for (trigger, second) in triggers {
                match trigger {
                    Some(Trigger::Denied) => self.events.push(SimEvent::Denied { player }),
                    Some(Trigger::Empty) => self.events.push(SimEvent::Empty { player }),
                    Some(Trigger::ChargeFull) => self.events.push(SimEvent::ChargeFull { player }),
                    Some(Trigger::Fire(shot)) => {
                        self.players[i].protect = 0.0;
                        k += 1;
                        let seq = self.players[i].cmd.shot.wrapping_sub(shots - k);
                        self.fire(i, shot, &rules, second, seq)
                    }
                    None => {}
                }
            }

            // Throwing.
            let p = &mut self.players[i];
            if let Some((rel, vel)) = p.cmd.throw.take()
                && p.grenades > 0
            {
                p.grenades -= 1;
                p.protect = 0.0;
                let eye = p.eye();
                let mut pos = p.feet + rel.clamp_length_max(2.0);
                let (id, team) = (p.id, p.team);
                if !pos.is_finite() || !self.world.clear(eye, pos) {
                    pos = eye;
                }
                // (VR players' throws harder.)
                let k = if p.cmd.vr { rules.vr_throw_factor } else { 1.0 };
                let vel = if vel.is_finite() {
                    (vel * k).clamp_length_max(rules.grenade_max_speed * k)
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
    fn fire(&mut self, pi: usize, shot: Shot, rules: &Rules, second: bool, seq: u32) {
        let seed = self.rng.random::<u64>();
        let p = &mut self.players[pi];
        if second {
            let mut gun = p.gun2();
            gun.fired(shot, rules);
            p.set_gun2(gun);
        } else {
            let mut gun = p.gun();
            gun.fired(shot, rules);
            p.set_gun(gun);
        }
        p.decloak = rules.cloak_flash;
        p.shots += 1;
        let eye = p.eye();
        let (from, dir) = crate::shot::muzzle_of(p.feet, eye, &p.cmd, second);
        let (id, team, view) = (p.id, p.team, p.cmd.view_tick);
        let client = p.client_hits;
        // A client judging its hits made the same spread.
        let seed = if client {
            crate::shot::shot_seed(id, seq)
        } else {
            seed
        };
        let blocked = self.world.ray(eye, from);
        // Fired first, so its event comes before any hit it makes straight away.
        self.events.push(SimEvent::Fired {
            player: id,
            pos: from,
            dir,
            second,
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
        let mut pellets = Vec::new();
        let bolts = crate::shot::shot_bolts(shot, dir, rules, seed);
        let bolts = crate::shot::as_phantom(bolts, rules, team, self.infected);
        for (k, (vel, life, extra)) in bolts.into_iter().enumerate() {
            let extra = BoltExtra {
                client,
                shot: seq,
                pellet: k as u8,
                ..extra
            };
            let mut bolt = Bolt {
                id: self.id(),
                owner: id,
                owner_team: team,
                team: Some(team),
                pos: from,
                vel,
                life,
                extra,
            };
            pellets.push((vel, life, extra, bolt.id, 0));
            if self.compensate(&mut bolt, view) {
                self.bolts.push(bolt);
            }
        }
        if client {
            let log = &mut self.players[pi].shot_log;
            if log.len() >= SHOT_LOG {
                log.pop_front();
            }
            log.push_back(ShotRecord {
                seq,
                view_tick: view,
                team,
                from,
                pellets,
            });
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
        // (The decoys after the players, as they are now.)
        let n = self.players.len();
        let decoys = self
            .decoys
            .iter()
            .enumerate()
            .map(|(k, d)| (n + k, d.target()));
        let contact = match past {
            None => bolt_contact(
                &self.world,
                &self.barriers,
                &self.rules,
                b,
                self.players
                    .iter()
                    .map(|p| p.target())
                    .enumerate()
                    .chain(decoys),
            ),
            Some(h) => {
                let players = &self.players;
                // Where they were, if they were there then and are still in the game now.
                let ghosts = self.history[h].1.iter().filter_map(|g| {
                    let pi = players.iter().position(|p| p.id == g.id)?;
                    let p = &players[pi];
                    Some((
                        pi,
                        BoltTarget {
                            id: g.id,
                            team: p.team,
                            alive: g.alive && p.alive(),
                            feet: g.feet,
                            eye: g.eye,
                            cmd: &g.cmd,
                        },
                    ))
                });
                bolt_contact(
                    &self.world,
                    &self.barriers,
                    &self.rules,
                    b,
                    ghosts.chain(decoys),
                )
            }
        };
        // A turret box's turret in the way first?
        let end = match contact {
            Contact::Clear => b.pos + b.vel * TICK,
            Contact::Player { at, .. }
            | Contact::Barrier { at, .. }
            | Contact::Mirror { at, .. } => at,
            Contact::Arena { q, .. } => q,
        };
        if let Some((side, i, q)) = self.gadget_hit(b.pos, end) {
            if b.extra.lance {
                self.events.push(SimEvent::Streak {
                    player: b.owner,
                    from: b.pos,
                    to: q,
                });
            }
            self.gadget_shot(side, i, b, q);
            return true;
        }
        if let Some((k, q)) = self.box_head_hit(b.pos, end) {
            if b.extra.lance {
                self.events.push(SimEvent::Streak {
                    player: b.owner,
                    from: b.pos,
                    to: q,
                });
            }
            self.box_shot(k, b, q);
            return true;
        }
        if b.extra.lance {
            let to = match contact {
                Contact::Clear => b.pos + b.vel * TICK,
                Contact::Player { at, .. }
                | Contact::Barrier { at, .. }
                | Contact::Mirror { at, .. } => at,
                Contact::Arena { q, .. } => q,
            };
            self.events.push(SimEvent::Streak {
                player: b.owner,
                from: b.pos,
                to,
            });
        }
        match contact {
            Contact::Clear => b.extra.lance,
            Contact::Mirror { index, at, n } => match crate::shot::bounce_placed(b, at, n) {
                Some(n) => {
                    self.events.push(SimEvent::Reflected {
                        pos: at,
                        normal: n,
                        team: b.team,
                    });
                    false
                }
                None => {
                    self.events.push(SimEvent::BarrierHit {
                        id: self.barriers[index].id,
                        pos: at,
                    });
                    true
                }
            },
            Contact::Barrier { index, at } => {
                self.events.push(SimEvent::BarrierHit {
                    id: self.barriers[index].id,
                    pos: at,
                });
                true
            }
            Contact::Player { index, at, .. } if index >= n => {
                let d = self.decoys.swap_remove(index - n);
                self.events.push(SimEvent::DecoyPopped {
                    id: d.id,
                    by: b.owner,
                    pos: at,
                });
                if b.extra.pierce {
                    b.extra.struck |= 1u64 << (d.owner % 64);
                    return self.bolt_tick(b, past);
                }
                true
            }
            Contact::Player { index, at, part } => {
                // A client judges this one's hits (`claim_hit`): it just stops (a beam flies
                // on).
                if !b.extra.client {
                    self.hit_part(index, b, at, part == crate::hitbox::Part::Head);
                }
                if !b.extra.pierce {
                    return true;
                }
                // A beam: on through them (and whoever else is in its way this tick).
                b.extra.struck |= 1u64 << (self.players[index].id % 64);
                self.bolt_tick(b, past)
            }
            Contact::Arena { q, n } => {
                // A mirror: off it again (as many times as the rules allow), from where it
                // struck.
                if let Some(n) =
                    crate::shot::bounce(&self.level, &self.rules, b, q, n, self.storm())
                {
                    self.events.push(SimEvent::Reflected {
                        pos: q,
                        normal: n,
                        team: b.team,
                    });
                    return false;
                }
                // Where the bolt struck, on the box (however oblique the shot).
                let struck = |min: Vec3, max: Vec3| on_box(q, min, max);
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
                true
            }
        }
    }

    /// A remote client (playing `shooter`, `Player::client_hits`) says one of its bolts hit a
    /// player. Checked against the shot as fired here (re-flown through the arena: the claimed
    /// spot must be on its way) and the victim as they were at the claimed moment; then the hit
    /// counts as any other (a shield blocks it, the hit cooldown lets it fizzle).
    pub fn claim_hit(&mut self, shooter: PlayerId, c: HitClaim) -> Claim {
        /// How far the claimed spot may be from the shot's way, and from the victim's body.
        const OFF_PATH: f32 = 0.6;
        const OFF_BODY: f32 = 1.0;
        let Some(si) = self.players.iter().position(|p| p.id == shooter) else {
            return Claim::Rejected("no shooter");
        };
        let s = &self.players[si];
        if !s.client_hits {
            return Claim::Rejected("not judged by the client");
        }
        let Some(ri) = s.shot_log.iter().rposition(|r| r.seq == c.shot) else {
            // (The command firing it may be set but not stepped yet: a beam hits within the tick
            // it's fired, so its claim comes with its command.)
            return if c.shot >= s.cmd.shot {
                Claim::Early
            } else {
                Claim::Rejected("no such shot")
            };
        };
        let rec = &s.shot_log[ri];
        let Some(&(vel, life, extra, bolt_id, struck)) = rec.pellets.get(c.pellet as usize) else {
            return Claim::Rejected("no such bolt");
        };
        let bit = 1u64 << (c.victim % 64);
        if struck & bit != 0 || (!extra.pierce && struck != 0) {
            return Claim::Rejected("already hit");
        }
        let Some(vi) = self.players.iter().position(|p| p.id == c.victim) else {
            return Claim::Rejected("no victim");
        };
        let v = &self.players[vi];
        if vi == si || v.team == rec.team || !v.alive() {
            return Claim::Rejected("not a living enemy");
        }
        if !c.view.is_finite() || !c.at.is_finite() || c.view < 0.0 {
            return Claim::Rejected("nonsense");
        }
        let view = c.view.floor() as u64;
        if self.tick.saturating_sub(view) > MAX_LAG || view > self.tick {
            return Claim::Rejected("too old");
        }
        // The victim there then (either tick around the view), near the spot.
        let near_body = [view, view + 1].iter().any(|&t| {
            self.past(t).is_some_and(|h| {
                self.history[h].1.iter().any(|g| {
                    g.id == c.victim && g.alive && {
                        let lo = g.feet + Vec3::Y * 0.3;
                        let hi = g.eye + Vec3::Y * 0.2;
                        dist_to_segment(c.at, lo, hi) <= OFF_BODY
                    }
                })
            })
        });
        if !near_body {
            return Claim::Rejected("not where the victim was");
        }
        // The shot's way through the arena (mirrors and all), tick by tick: the spot on it, at
        // about the tick of the bolt's flight the claimed moment comes to.
        let flown = (rec.view_tick != 0).then_some(c.view - rec.view_tick as f32);
        let mut b = Bolt {
            id: bolt_id,
            owner: shooter,
            owner_team: rec.team,
            team: Some(rec.team),
            pos: rec.from,
            vel,
            life,
            extra,
        };
        let mut on_path = false;
        let mut k = 0u32;
        while b.life > 0.0 && k < 600 {
            let to = b.pos + b.vel * TICK;
            let near = dist_to_segment(c.at, b.pos, to) <= OFF_PATH;
            let timely = flown.is_none_or(|f| (k as f32 - f).abs() <= 6.0 + 0.2 * k as f32);
            if near && timely {
                on_path = true;
                break;
            }
            // A wall ends the way (a mirror turns it, and one put up).
            let wall = self.world.ray(b.pos, to);
            let placed = crate::shot::placed_mirror(&self.barriers, &self.rules, b.pos, to);
            match (wall, placed) {
                (_, Some((t, k))) if wall.is_none_or(|w| t < w.0) => {
                    let at = b.pos + (to - b.pos) * t;
                    if crate::shot::bounce_placed(&mut b, at, self.barriers[k].normal).is_none() {
                        break;
                    }
                }
                (Some((_, q, n)), _)
                    if crate::shot::bounce(
                        &self.level,
                        &self.rules,
                        &mut b,
                        q,
                        n,
                        self.storm(),
                    )
                    .is_none() =>
                {
                    break;
                }
                _ => {}
            }
            b.pos += b.vel * TICK;
            b.life -= TICK;
            k += 1;
        }
        if !on_path {
            return Claim::Rejected("not on the shot's way");
        }
        // It counts.
        if let Some(r) = self.players[si].shot_log.get_mut(ri) {
            r.pellets[c.pellet as usize].4 |= bit;
        }
        if !extra.pierce {
            self.bolts.retain(|x| x.id != bolt_id);
        }
        b.pos = c.at - b.vel.normalize_or_zero() * 0.1;
        // A head hit: the spot near where the head was.
        let head = c.head
            && [view, view + 1].iter().any(|&t| {
                self.past(t).is_some_and(|h| {
                    self.history[h].1.iter().any(|g| {
                        g.id == c.victim && c.at.distance(g.eye) <= crate::hitbox::HEAD_RADIUS + 0.3
                    })
                })
            });
        self.hit_part(vi, &b, c.at, head);
        Claim::Accepted
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

    /// `hit_player`, with a prism lance's head hit dealing `Rules::lance_head_damage`.
    fn hit_part(&mut self, pi: usize, b: &Bolt, at: Vec3, head: bool) {
        if !(head && b.extra.lance) {
            return self.hit_player(pi, b, at);
        }
        let p = &self.players[pi];
        if p.shield <= 0.0 && p.cooldown <= 0.0 && p.protect <= 0.0 {
            self.events.push(SimEvent::Headshot {
                player: p.id,
                by: b.owner,
                pos: at,
            });
        }
        let mut b = *b;
        b.extra.damage = self.rules.lance_head_damage;
        self.hit_player(pi, &b, at)
    }

    fn hit_player(&mut self, pi: usize, b: &Bolt, at: Vec3) {
        let (hit_cooldown, hits_to_tag) = (self.rules.hit_cooldown, self.rules.hits_to_tag);
        let p = &mut self.players[pi];
        if p.shield > 0.0 {
            self.events.push(SimEvent::Blocked {
                player: p.id,
                by: b.owner,
                pos: at,
            });
            return;
        }
        if p.cooldown > 0.0 || p.protect > 0.0 {
            // Still recovering from the last hit (or just respawned): the bolt just fizzles.
            self.events.push(SimEvent::Immune {
                player: p.id,
                by: b.owner,
                pos: at,
            });
            self.events.push(SimEvent::BoltImpact {
                pos: at,
                normal: (b.pos - at).normalize_or_zero(),
                team: b.team,
            });
            return;
        }
        // (Infection: a phantom goes down in `Rules::phantom_hits` survivors' hits.)
        let damage = if self.infected && p.team == Team::Red && b.owner_team == Team::Blue {
            let per = hits_to_tag.div_ceil(self.rules.phantom_hits.clamp(1, hits_to_tag));
            b.extra.damage.max(1).saturating_mul(per)
        } else {
            b.extra.damage.max(1)
        };
        p.hits = (p.hits + damage).min(hits_to_tag);
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
            p.core_live = false;
            p.lose_powerups();
            let how = if b.owner == HAZARD {
                // (A gate's bolt stands still.)
                if b.life <= 0.0 && b.vel == Vec3::ZERO {
                    TagHow::Gate
                } else {
                    TagHow::Turret
                }
            } else if b.team.is_none() {
                TagHow::Grenade
            } else if b.extra.pierce {
                TagHow::Beam
            } else if self
                .player(b.owner)
                .is_some_and(|s| s.weapon == Weapon::Shotgun)
            {
                TagHow::Shotgun
            } else {
                TagHow::Blaster
            };
            self.events.push(SimEvent::TaggedOut {
                player: id,
                by: b.owner,
                pos: at,
                respawn_in: wait,
                how,
            });
            // Infection: a survivor tagged out comes back a phantom.
            if self.infected && team == Team::Blue {
                self.players[pi].team = Team::Red;
                self.events.push(SimEvent::Infected { player: id });
            }
            let hit_by = std::mem::take(&mut self.players[pi].hit_by);
            if b.owner_team != team && b.owner != HAZARD {
                // (A bounty's worth more.)
                let points = if self.bounty == Some(id) {
                    self.events.push(SimEvent::BountyClaimed {
                        player: id,
                        by: b.owner,
                    });
                    self.end_bounty();
                    self.rules.bounty_points.max(self.rules.tag_points)
                } else {
                    self.rules.tag_points
                };
                self.award(b.owner_team, Some(b.owner), points);
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
        let gravity = self.gravity();
        let mut i = 0;
        while i < self.grenades.len() {
            let mut g = self.grenades[i];
            g.age += TICK;
            let mut done = false;
            match g.phase {
                GrenadePhase::Flying => {
                    let (pos, vel, bounce) = grenade_step(
                        &self.world,
                        g.pos,
                        g.vel,
                        rules.grenade_restitution,
                        gravity,
                    );
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
                // Lasted out: the bounty is theirs.
                if kind == ArenaEvent::Bounty
                    && let Some(id) = self.bounty.take()
                    && let Some(team) = self.player(id).map(|p| p.team)
                {
                    self.award(team, Some(id), self.rules.bounty_points);
                    self.events.push(SimEvent::BountySurvived { player: id });
                }
            }
        }
        // (A bounty's player gone: so is the bounty.)
        if let Some(id) = self.bounty
            && self.player(id).is_none()
        {
            self.end_bounty();
        }
        let rules = &self.rules;
        let elapsed = rules.match_time - self.time_left;
        if rules.events
            && self.event.is_none()
            && elapsed >= self.next_event
            && self.time_left > rules.event_quiet
        {
            // (Any but the last one, and those on all match long.)
            let kinds: Vec<ArenaEvent> = ArenaEvent::ALL
                .into_iter()
                .filter(|k| Some(*k) != self.last_event && !rules.modifiers.contains(k))
                .collect();
            let kind = kinds[self.rng.random_range(0..kinds.len())];
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

    /// The gravity now (m/s²): less in low gravity.
    pub fn gravity(&self) -> f32 {
        gravity_for(self.event, &self.rules)
    }

    /// The bounty's over before its time (claimed, or its player gone).
    fn end_bounty(&mut self) {
        self.bounty = None;
        if let Some((ArenaEvent::Bounty, _)) = self.event {
            self.event = None;
            self.events.push(SimEvent::EventEnd {
                kind: ArenaEvent::Bounty,
            });
        }
    }

    /// A mirror storm going on.
    pub fn storm(&self) -> bool {
        self.rules.in_effect(self.event, ArenaEvent::MirrorStorm)
    }

    /// Starts an arena event now (testing too).
    pub fn start_event(&mut self, kind: ArenaEvent) {
        if let Some((old, _)) = self.event.take() {
            self.events.push(SimEvent::EventEnd { kind: old });
        }
        self.bounty = None;
        self.last_event = Some(kind);
        let length = match kind {
            ArenaEvent::Blackout => self.rules.blackout_time,
            ArenaEvent::MirrorStorm => self.rules.storm_time,
            ArenaEvent::LowGravity => self.rules.lowgrav_time,
            // On the top scorer (of those in the game; none scored yet: anyone).
            ArenaEvent::Bounty => {
                let top = self
                    .players
                    .iter()
                    .filter(|p| p.alive())
                    .map(|p| p.score)
                    .max();
                let tops: Vec<PlayerId> = self
                    .players
                    .iter()
                    .filter(|p| p.alive() && Some(p.score) == top)
                    .map(|p| p.id)
                    .collect();
                if tops.is_empty() {
                    return;
                }
                let id = tops[self.rng.random_range(0..tops.len())];
                self.bounty = Some(id);
                self.events.push(SimEvent::BountyOn { player: id });
                self.rules.bounty_time
            }
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
    /// Pulse sensors: flying until they meet the arena, then pulsing until their time is up
    /// (each pulse shows the enemies in range, `Player::revealed`, and strips their cloaks).
    fn step_sensors(&mut self) {
        let gravity = self.gravity();
        let rules = &self.rules;
        let mut pulses = Vec::new();
        let mut i = 0;
        while i < self.sensors.len() {
            let s = &mut self.sensors[i];
            if !s.stuck {
                s.vel.y -= gravity * TICK;
                let to = s.pos + s.vel * TICK;
                match self.world.ray(s.pos, to) {
                    Some((_, q, n)) => {
                        s.pos = q + n * 0.05;
                        s.normal = n;
                        s.vel = Vec3::ZERO;
                        s.stuck = true;
                        self.events.push(SimEvent::SensorStuck {
                            id: s.id,
                            pos: s.pos,
                        });
                    }
                    None => s.pos = to,
                }
                // (Lost out of the arena.)
                if s.pos.y < -20.0 {
                    s.left = 0.0;
                }
            } else {
                s.left -= TICK;
                s.next_pulse -= TICK;
                if s.next_pulse <= 0.0 && s.left > 0.0 {
                    s.next_pulse = rules.sensor_every;
                    pulses.push((s.id, s.team, s.pos));
                }
            }
            let s = self.sensors[i];
            if s.left <= 0.0 {
                self.events.push(SimEvent::SensorDown {
                    id: s.id,
                    pos: s.pos,
                });
                self.sensors.swap_remove(i);
            } else {
                i += 1;
            }
        }
        let range = self.rules.sensor_range;
        for (id, team, pos) in pulses {
            self.pulse(id, team, pos, range, 0.0);
        }
    }

    /// A pulse (a sensor's, `id`, or a sensor mast's) of `team`'s from `pos`: the enemies within
    /// `range` show to the team (through walls) and lose their cloaks. `shown`: the range the
    /// clients draw (0: a sensor's, `Rules::sensor_range`).
    pub(crate) fn pulse(&mut self, id: u32, team: Team, pos: Vec3, range: f32, shown: f32) {
        let infection = self.infection();
        let reveal = self.rules.sensor_reveal;
        let mut seen = 0u64;
        for p in &mut self.players {
            if p.team != team && p.alive() && p.chest().distance(pos) <= range {
                p.revealed = reveal;
                seen |= 1u64 << (p.id % 64);
                // (A cloak in range is gone for good; not a phantom's, which is what it is.)
                let phantom = infection && p.team == Team::Red;
                if p.cloak > 0.0 && !phantom {
                    p.cloak = 0.0;
                    p.decloak = 0.0;
                    self.events.push(SimEvent::Uncloaked { player: p.id });
                }
            }
        }
        self.events.push(SimEvent::Pulse {
            id,
            team,
            pos,
            seen,
            range: shown,
        });
    }

    /// Med beams held: on the team mate nearest the aim (within its cone and reach, in sight),
    /// healing them a hit at a time and refilling their blaster.
    fn step_medbeams(&mut self) {
        let rules = &self.rules;
        for i in 0..self.players.len() {
            let p = &self.players[i];
            let held = p.cmd.gadget
                && p.alive()
                && p.gadget_kind() == Some(Gadget::Medbeam)
                && p.medbeam_left > 0.0;
            let target = held
                .then(|| {
                    let eye = p.eye();
                    let (_, dir) = crate::shot::muzzle(p.feet, eye, &p.cmd);
                    self.players
                        .iter()
                        .enumerate()
                        .filter(|(j, o)| *j != i && o.team == p.team && o.alive())
                        .filter_map(|(j, o)| {
                            let to = o.chest() - eye;
                            let d = to.length();
                            let off = dir.angle_between(to);
                            (d <= rules.medbeam_range
                                && off <= rules.medbeam_cone
                                && self.world.clear(eye, o.chest()))
                            .then_some((j, off))
                        })
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(j, _)| j)
                })
                .flatten();
            let target_id = target.map(|j| self.players[j].id);
            let p = &mut self.players[i];
            let was = p.mending;
            p.mending = target_id;
            if p.mending != was {
                p.mend = 0.0;
            }
            let Some(j) = target else { continue };
            p.medbeam_left -= TICK;
            p.mend += TICK;
            p.mend_ammo += rules.medbeam_ammo * TICK;
            let (by, heal, refill) = (p.id, p.mend >= rules.medbeam_tick, p.mend_ammo.floor());
            if heal {
                p.mend = 0.0;
            }
            p.mend_ammo -= refill;
            if p.medbeam_left <= 0.0 {
                p.medbeam_left = 0.0;
                p.gadget = None;
                p.mending = None;
            }
            let o = &mut self.players[j];
            o.ammo = (o.ammo + refill as u16).min(rules.ammo);
            if heal && o.hits > 0 {
                o.hits -= 1;
                self.events.push(SimEvent::Mended {
                    player: o.id,
                    by,
                    hits: o.hits,
                });
            }
        }
    }

    /// How close a cloaked `p` has to come to be seen (`cloak_fade`).
    pub fn cloak_near(&self, p: &Player) -> f32 {
        cloak_fade(&self.rules, self.infected && p.team == Team::Red).0
    }

    /// Infection: whether it's on (`Rules::mode`).
    pub fn infection(&self) -> bool {
        self.level.mode == GameMode::Infection
    }

    /// Infection: the phantoms picked once the match is under way (a share of the players, at
    /// least one, at random; the rest survivors), phantoms kept cloaked, survivors scoring
    /// while they last, and the match over once none are left.
    fn step_infection(&mut self) {
        if !self.infection() || self.phase != MatchPhase::Live {
            return;
        }
        if !self.infected {
            let n = self.players.len();
            if n < 2 {
                return;
            }
            self.infected = true;
            self.survive_t = self.rules.survive_every;
            let k = self.rules.infect_start.clamp(1, n - 1);
            let mut order: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = self.rng.random_range(0..=i);
                order.swap(i, j);
            }
            for (rank, &i) in order.iter().enumerate() {
                let team = if rank < k { Team::Red } else { Team::Blue };
                if self.players[i].team != team {
                    self.players[i].team = team;
                    let s = self.pick_spawn(team);
                    let p = &mut self.players[i];
                    p.feet = s.pos;
                    p.cmd.yaw = s.yaw;
                    p.claim_block = 30;
                }
                if team == Team::Red {
                    let id = self.players[i].id;
                    self.events.push(SimEvent::Infected { player: id });
                }
            }
        }
        for p in &mut self.players {
            if p.team == Team::Red {
                p.cloak = p.cloak.max(1.0);
            }
        }
        self.survive_t -= TICK;
        if self.survive_t <= 0.0 {
            self.survive_t = self.rules.survive_every;
            let alive: Vec<PlayerId> = self
                .players
                .iter()
                .filter(|p| p.team == Team::Blue && p.alive())
                .map(|p| p.id)
                .collect();
            for id in alive {
                self.award(Team::Blue, Some(id), self.rules.survive_points);
            }
        }
        if !self.players.iter().any(|p| p.team == Team::Blue) {
            self.award(Team::Red, None, self.rules.infect_all_points);
            self.over = true;
            self.events.push(SimEvent::MatchOver {
                scores: self.scores,
            });
        }
    }

    /// Whether player `id` carries a prism (and can't fire or throw grenades).
    pub fn carrying(&self, id: PlayerId) -> bool {
        self.prisms.iter().any(|p| p.carrier == Some(id))
    }

    /// The prisms (prism mode): taken by an enemy touching one, carried overhead (thrown with a
    /// grenade throw, caught by the carrier's team mates), dropped by a carrier tagged out,
    /// home again when its own team touches it or after `Rules::prism_return` on the floor, and
    /// captured when carried to its carrier's own home while that team's prism is there.
    fn step_prisms(&mut self) {
        let Some(homes) = self.level.prisms else {
            return;
        };
        if self.prisms.is_empty() {
            self.prisms = [Team::Red, Team::Blue]
                .iter()
                .map(|&team| Prism {
                    team,
                    home: homes[team.index()],
                    pos: homes[team.index()],
                    vel: Vec3::ZERO,
                    carrier: None,
                    dropped: None,
                    thrower: None,
                })
                .collect();
        }
        let rules = self.rules.clone();
        let gravity = self.gravity();
        for k in 0..self.prisms.len() {
            let pr = self.prisms[k];
            let team = pr.team;
            // Carried: overhead, until thrown or its carrier's tagged out.
            if let Some(id) = pr.carrier {
                let Some(ci) = self.players.iter().position(|p| p.id == id) else {
                    self.drop_prism(k, pr.pos);
                    continue;
                };
                let c = &mut self.players[ci];
                if !c.alive() || c.team == team {
                    let at = c.feet + Vec3::Y * 0.3;
                    self.drop_prism(k, at);
                    continue;
                }
                let over = c.eye() + Vec3::Y * 0.45;
                if let Some((rel, vel)) = c.cmd.throw.take() {
                    let vel = if vel.is_finite() {
                        let f = if c.cmd.vr { rules.vr_throw_factor } else { 1.0 };
                        (vel * f).clamp_length_max(rules.grenade_max_speed)
                    } else {
                        Vec3::ZERO
                    };
                    let from = c.feet + rel.clamp_length_max(2.0);
                    let from = if from.is_finite() && self.world.clear(c.eye(), from) {
                        from
                    } else {
                        c.eye()
                    };
                    let p = &mut self.prisms[k];
                    p.carrier = None;
                    p.pos = from;
                    p.vel = vel + Vec3::Y * 0.01;
                    p.thrower = Some((id, 0.4));
                    self.events.push(SimEvent::PrismThrown {
                        team,
                        player: id,
                        pos: from,
                    });
                    continue;
                }
                self.prisms[k].pos = over;
                // Home with it, one's own prism there: captured.
                let own = c.team;
                let feet = c.feet;
                let own_home = homes[own.index()];
                let own_there = self.prisms.iter().any(|p| p.team == own && p.at_home());
                let near_home = (feet - own_home).with_y(0.0).length() < rules.prism_reach * 1.5
                    && (own_home.y - feet.y).abs() < 2.0;
                if own_there && near_home {
                    self.award(own, Some(id), rules.prism_points);
                    self.events.push(SimEvent::PrismCaptured {
                        team,
                        by: id,
                        pos: own_home,
                    });
                    self.send_prism_home(k);
                }
                continue;
            }
            // Flying: falling, bouncing, caught by the thrower's team mates.
            let mut pr = pr;
            if let Some((who, t)) = pr.thrower {
                pr.thrower = (t > TICK).then_some((who, t - TICK));
            }
            if pr.vel != Vec3::ZERO {
                let (pos, vel, _) = grenade_step(&self.world, pr.pos, pr.vel, 0.3, gravity);
                pr.pos = pos;
                pr.vel = vel;
                if pr.vel == Vec3::ZERO && pr.dropped.is_none() {
                    pr.dropped = Some(rules.prism_return);
                }
                if pr.pos.y < -20.0 {
                    self.prisms[k] = pr;
                    self.send_prism_home(k);
                    self.events.push(SimEvent::PrismReturned {
                        team,
                        by: None,
                        pos: pr.home,
                    });
                    continue;
                }
            }
            // On the floor: home again in a while.
            if let Some(t) = pr.dropped
                && pr.vel == Vec3::ZERO
            {
                if t <= TICK {
                    self.prisms[k] = pr;
                    self.send_prism_home(k);
                    self.events.push(SimEvent::PrismReturned {
                        team,
                        by: None,
                        pos: pr.home,
                    });
                    continue;
                }
                pr.dropped = Some(t - TICK);
            }
            self.prisms[k] = pr;
            // Touched: by its own team, home it goes (unless it's home already); by the enemy,
            // taken (caught in flight from further).
            let flying = pr.vel != Vec3::ZERO;
            let toucher = self
                .players
                .iter()
                .filter(|p| p.alive() && !self.carrying(p.id))
                .filter(|p| pr.thrower.is_none_or(|(who, _)| who != p.id))
                // (Near it across, and within reach up and down.)
                .filter(|p| (pr.pos.y - (p.feet.y + 1.0)).abs() < 1.3)
                .map(|p| (p.id, p.team, (p.feet - pr.pos).with_y(0.0).length()))
                .filter(|&(_, t, d)| {
                    let reach = if flying && t != team {
                        rules.prism_catch
                    } else {
                        rules.prism_reach
                    };
                    d < reach
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            match toucher {
                Some((id, t, _)) if t == team => {
                    if !pr.at_home() {
                        self.send_prism_home(k);
                        self.events.push(SimEvent::PrismReturned {
                            team,
                            by: Some(id),
                            pos: pr.home,
                        });
                    }
                }
                Some((id, _, _)) => {
                    let p = &mut self.prisms[k];
                    p.carrier = Some(id);
                    p.vel = Vec3::ZERO;
                    p.dropped = None;
                    p.thrower = None;
                    self.events.push(if flying {
                        SimEvent::PrismCaught {
                            team,
                            player: id,
                            pos: pr.pos,
                        }
                    } else {
                        SimEvent::PrismTaken {
                            team,
                            player: id,
                            pos: pr.pos,
                        }
                    });
                    // (Taking it gives away a cloak.)
                    if let Some(c) = self.players.iter_mut().find(|p| p.id == id) {
                        c.protect = 0.0;
                    }
                }
                None => {}
            }
        }
    }

    /// Prism `k` falls where its carrier was.
    fn drop_prism(&mut self, k: usize, at: Vec3) {
        let rules = &self.rules;
        let p = &mut self.prisms[k];
        p.carrier = None;
        p.pos = at;
        p.vel = Vec3::Y * 0.5;
        p.dropped = Some(rules.prism_return);
        p.thrower = None;
        self.events.push(SimEvent::PrismDropped {
            team: p.team,
            pos: at,
        });
    }

    fn send_prism_home(&mut self, k: usize) {
        let p = &mut self.prisms[k];
        p.pos = p.home;
        p.vel = Vec3::ZERO;
        p.carrier = None;
        p.dropped = None;
        p.thrower = None;
    }

    /// Decoys: running on ahead at a player's pace (stopped by walls) until their time is up.
    fn step_decoys(&mut self) {
        let speed = self.rules.move_speed;
        let gravity = self.gravity();
        for d in &mut self.decoys {
            let ahead = Quat::from_rotation_y(d.cmd.yaw) * Vec3::NEG_Z;
            let delta = Vec2::new(ahead.x, ahead.z) * speed * TICK;
            (d.feet, d.vy) = self.world.move_body_g(d.feet, delta, d.vy, TICK, gravity);
            d.left -= TICK;
        }
        let events = &mut self.events;
        self.decoys.retain(|d| {
            let keep = d.left > 0.0 && d.feet.y > -20.0;
            if !keep {
                events.push(SimEvent::DecoyGone {
                    id: d.id,
                    pos: d.feet,
                });
            }
            keep
        });
    }

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
            mirror: false,
        }
    }

    /// Whether a barrier stops a bolt of `team` (`None`: anyone's) from `a` to `b`.
    pub fn barrier_blocks(&self, a: Vec3, b: Vec3, team: Option<Team>) -> bool {
        let (w, h) = (self.rules.barrier_w, self.rules.barrier_h);
        self.barriers
            .iter()
            .any(|br| (br.mirror || Some(br.team) != team) && br.crossed(a, b, w, h).is_some())
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
                if !p.alive()
                    || p.cooldown > 0.0
                    || p.protect > 0.0
                    || p.shield > 0.0
                    || !gate.touches(p.feet)
                {
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
                    p.core_live = false;
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
                self.zap(i, TagHow::Field);
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
                        && p.seen_from(p.chest().distance(muzzle), self.cloak_near(p))
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
    /// Tags player `victim` out at once, as a blaster bolt of `by`'s would (testing).
    pub fn tag_out(&mut self, victim: PlayerId, by: PlayerId) {
        let (Some(vi), Some(s)) = (
            self.players.iter().position(|p| p.id == victim),
            self.player(by),
        ) else {
            return;
        };
        let bolt = Bolt {
            id: 0,
            owner: by,
            owner_team: s.team,
            team: Some(s.team),
            pos: s.eye(),
            vel: Vec3::X,
            life: 1.0,
            extra: BoltExtra::default(),
        };
        let p = &mut self.players[vi];
        p.hits = self.rules.hits_to_tag.saturating_sub(1);
        p.cooldown = 0.0;
        p.protect = 0.0;
        p.shield = 0.0;
        let at = p.chest();
        self.hit_player(vi, &bolt, at);
    }

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
            pool: None,
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
    pub(crate) fn zap(&mut self, pi: usize, how: TagHow) {
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
        p.core_live = false;
        p.lose_powerups();
        self.events.push(SimEvent::TaggedOut {
            player: p.id,
            by: HAZARD,
            pos: p.eye() - Vec3::Y * 0.4,
            respawn_in: wait,
            how,
        });
    }

    /// Sends a player back to one of its team's spawn pads at once, healed and refilled.
    fn send_home(&mut self, pi: usize) {
        let team = self.players[pi].team;
        let s = self.pick_spawn(team);
        let (ammo, protect) = (self.rules.ammo, self.rules.spawn_protect);
        let p = &mut self.players[pi];
        p.hits = 0;
        p.cooldown = 0.0;
        p.protect = protect;
        p.ammo = ammo;
        p.reloading = None;
        p.mega_hits = 0;
        p.power = None;
        p.core_live = false;
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
        // (The secondary targets that are puzzles don't count hits: `Rules::secondary`.)
        let side = mega.secondaries.iter().position(on);
        let side = side.filter(|&i| self.secondary[i] == crate::rules::SecondaryKind::Shoot);
        if side.is_none() && !on(&mega.core) && mega.secondaries.iter().any(on) {
            self.events.push(SimEvent::BoltImpact {
                pos: q,
                normal: -(b.vel.normalize_or_zero()),
                team: b.team,
            });
            return true;
        }
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
                None => p.power.is_some() && in_room(p) && p.core_live,
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
        match side {
            Some(i) => {
                self.charge_secondary(i, pi, q);
            }
            None => {
                self.award(b.owner_team, Some(b.owner), self.rules.megatarget_points);
                self.players[pi].target_hits += 1;
                self.events.push(SimEvent::MegaHit {
                    by: b.owner,
                    pos: q,
                });
                self.send_home(pi);
            }
        }
        true
    }

    /// Megatarget mode: player `pi` charged at secondary target `side` (a hit on it, or its
    /// puzzle solved; at `pos`): the points, and once both are, the power. Nothing if they
    /// have the power already or this one charged them already.
    pub(crate) fn charge_secondary(&mut self, side: usize, pi: usize, pos: Vec3) -> bool {
        let p = &self.players[pi];
        if p.power.is_some() || p.mega_hits & (1 << side) != 0 || !p.alive() {
            return false;
        }
        let (team, id) = (p.team, p.id);
        self.award(team, Some(id), self.rules.secondary_points);
        let power_time = self.rules.power_time;
        let p = &mut self.players[pi];
        p.target_hits += 1;
        p.mega_hits |= 1 << side;
        let charged = p.mega_hits == 0b11;
        if charged {
            p.power = Some(power_time);
        }
        self.events.push(SimEvent::SecondaryHit {
            side: side as u8,
            by: id,
            pos,
            charged,
        });
        true
    }
}

/// Distance from `p` to the segment `a`..`b`.
fn dist_to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    let t = if d.length_squared() > 1e-12 {
        ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + d * t).distance(p)
}

impl Sim {
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
                    protect: p.protect,
                    dash_ready: p.moves.dash_ready,
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
                    core_live: p.core_live,
                    rapid: p.rapid,
                    shields: p.shields,
                    shield: p.shield,
                    weapon: p.weapon,
                    cloak: p.cloak,
                    decloak: p.decloak,
                    speed: p.speed,
                    revealed: p.revealed,
                    charging: p.charging.unwrap_or(0.0),
                    gadget: p.gadget,
                    gun2_hand: p.cmd.gun2_hand.filter(|_| p.weapon == Weapon::Dual),
                    medbeam_left: p.medbeam_left,
                    mending: p.mending,
                    shot: p.cmd.shot,
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
                    owner: b.owner,
                    bounces: b.extra.bounces,
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
            boxes: self.boxes.iter().map(|b| b.snap()).collect(),
            consoles: self.consoles.iter().map(|c| c.snap()).collect(),
            gadgets: self.gadgets.iter().map(|g| g.snap()).collect(),
            barriers: self.barriers.clone(),
            sensors: self.sensors.clone(),
            decoys: self.decoys.iter().map(|d| d.snap()).collect(),
            prisms: self.prisms.clone(),
            event: self.event,
            double: self.double,
            phase: self.phase,
            bounty: self.bounty,
            infected: self.infected,
        }
    }
}

/// A body's jumping, kept alike by the sim and the client's prediction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moves {
    /// The command's `PlayerCmd::jumps` last seen (none yet: the first one only counts from).
    pub jumps: Option<u32>,
    /// Seconds a press still waits to jump (pressed just before landing).
    pub wait: f32,
    /// Seconds since the feet were last on the ground.
    pub off_ground: f32,
    /// Jumped and not landed yet.
    pub jumped: bool,
    /// The command's `PlayerCmd::dashes` last seen, the seconds left of a dash and its
    /// direction, and the seconds until the next dash may start.
    pub dashes: Option<u32>,
    pub dash_left: f32,
    pub dash_dir: Vec2,
    pub dash_ready: f32,
}

/// A body's dash, after `jump_step`: with `dashes` the command's count of presses, `stick`
/// where it moves (or, standing, `yaw` where it faces) and `air` a jump pad's throw, the
/// horizontal velocity of a dash under way (the stick's movement instead of), and whether one
/// starts now. Dashes start on the ground (or within `Rules::coyote_time` of leaving it), not
/// flying from a pad, at most once every `Rules::dash_cooldown` seconds.
pub fn dash_step(
    rules: &Rules,
    m: &mut Moves,
    dashes: u32,
    stick: Vec2,
    yaw: f32,
    air: Vec2,
    dt: f32,
) -> (Option<Vec2>, bool) {
    let pressed = m.dashes.is_some_and(|seen| seen != dashes);
    m.dashes = Some(dashes);
    m.dash_ready = (m.dash_ready - dt).max(0.0);
    let mut started = false;
    if pressed
        && m.dash_ready <= 0.0
        && m.dash_left <= 0.0
        && air == Vec2::ZERO
        && m.off_ground <= rules.coyote_time
    {
        let facing = Vec2::new(-yaw.sin(), -yaw.cos());
        m.dash_dir = stick.try_normalize().unwrap_or(facing);
        m.dash_left = rules.dash_time;
        m.dash_ready = rules.dash_cooldown;
        started = true;
    }
    if m.dash_left <= 0.0 {
        return (None, started);
    }
    // (The last step only as long as what's left.)
    let k = (m.dash_left / dt).min(1.0);
    m.dash_left = (m.dash_left - dt).max(0.0);
    (Some(m.dash_dir * rules.dash_speed * k), started)
}

/// A body's jumping before its move (`World::move_body`): with `jumps` the command's count of
/// presses, `vy` its vertical speed (0: on the ground) and `air` a jump pad's throw, the
/// upward speed to jump with now, if it jumps. A press counts for `Rules::jump_buffer` seconds
/// (landing meanwhile jumps), and a body that walked off an edge may still jump for
/// `Rules::coyote_time` seconds; never twice before landing, nor flying from a pad.
pub fn jump_step(
    rules: &Rules,
    m: &mut Moves,
    jumps: u32,
    vy: f32,
    air: Vec2,
    dt: f32,
) -> Option<f32> {
    if let Some(seen) = m.jumps
        && jumps != seen
    {
        m.wait = rules.jump_buffer;
    }
    m.jumps = Some(jumps);
    if vy == 0.0 {
        m.off_ground = 0.0;
        m.jumped = false;
    } else {
        m.off_ground += dt;
    }
    if m.wait <= 0.0 {
        return None;
    }
    if !m.jumped && air == Vec2::ZERO && m.off_ground <= rules.coyote_time {
        m.wait = 0.0;
        m.jumped = true;
        return Some(rules.jump_speed);
    }
    m.wait = (m.wait - dt).max(0.0);
    None
}

/// How fast the stick moves a player (m/s), with a speed boost running and crouching (the
/// sim's and the client's prediction alike).
pub fn move_speed(rules: &Rules, boosted: bool, crouch: bool) -> f32 {
    rules.move_speed
        * if boosted { rules.speed_factor } else { 1.0 }
        * if crouch { rules.crouch_factor } else { 1.0 }
}

/// How much of the stick's movement counts while flying from a jump pad (all of it on the
/// ground).
pub fn air_control(air: Vec2) -> f32 {
    if air == Vec2::ZERO { 1.0 } else { 0.25 }
}

/// Jump pads, after a body's move from `from` to `feet` (the sim's and the client's prediction
/// alike): on landing the flight's horizontal velocity `air` is gone; on the ground, having
/// crossed a pad within `radius` of its middle, the vertical speed and horizontal velocity it
/// throws the body with. The sim takes `PAD_RADIUS`, the client's prediction
/// [`PREDICT_PAD_RADIUS`]: a little less, so that whenever it throws the player the sim does too
/// (it follows the predicted feet but samples them only every tick).
pub fn pad_step(
    level: &Level,
    from: Vec3,
    feet: Vec3,
    radius: f32,
    vy: f32,
    air: &mut Vec2,
) -> Option<(f32, Vec2)> {
    if vy != 0.0 {
        return None;
    }
    *air = Vec2::ZERO;
    level
        .pad_crossed(from, feet, radius)
        .map(|p| (p.launch.y, Vec2::new(p.launch.x, p.launch.z)))
}

/// The client's prediction's jump pad radius (see [`pad_step`]).
pub const PREDICT_PAD_RADIUS: f32 = crate::level::PAD_RADIUS - 0.15;

/// Yaw (0 = facing -Z) of a horizontal direction.
pub fn yaw_of(dir: Vec3) -> f32 {
    (-dir.x).atan2(-dir.z)
}

/// Where the eyes are from the top of the neck, looking straight ahead: in front of and above it.
const NECK_TO_EYE: Vec3 = Vec3::new(0.0, 0.1, -0.08);
/// The neck's length, from the chest's top up to the head.
const NECK: f32 = crate::hitbox::CHEST_DROP - NECK_TO_EYE.y;
/// How far the neck tilts (pitch and roll, radians); the head bends the rest at its top.
const NECK_BEND: f32 = 0.7;

/// The neck's rotation under a head turned to `rot`: turned with it, tilted with it up to
/// [`NECK_BEND`]. It stands on the chest's top and carries the head at its top, so the two move
/// as one.
pub fn neck_rot(rot: Quat) -> Quat {
    let (yaw, pitch, roll) = rot.to_euler(glam::EulerRot::YXZ);
    Quat::from_euler(
        glam::EulerRot::YXZ,
        yaw,
        pitch.clamp(-NECK_BEND, NECK_BEND),
        roll.clamp(-NECK_BEND, NECK_BEND),
    )
}

/// The eyes of a head turned to `rot` on its neck, which stands on the chest above the body's
/// middle (the feet, for a desktop player or a bot): `level` is the height of the eyes looking
/// straight ahead. The neck tilts from its base, so looking down brings the eyes forward and
/// down and turning swings them round, the chest staying over the feet. Tracked heads move so
/// by themselves.
pub fn eye_on_neck(level: Vec3, rot: Quat) -> Vec3 {
    let (yaw, pitch, _) = rot.to_euler(glam::EulerRot::YXZ);
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    level - Vec3::Y * crate::hitbox::CHEST_DROP + neck_rot(rot) * Vec3::Y * NECK + rot * NECK_TO_EYE
}

/// The top of the neck under the eyes at `eye` of a head turned to `rot` (any head, tracked or
/// not): where the head sits on it.
pub fn neck_of(eye: Vec3, rot: Quat) -> Vec3 {
    eye - rot * NECK_TO_EYE
}

/// The top of the floating chest, at the neck's base under the eyes at `eye` turned to `rot`
/// (`CHEST_DROP` below the eyes when they look straight ahead).
pub fn chest_top(eye: Vec3, rot: Quat) -> Vec3 {
    neck_of(eye, rot) - neck_rot(rot) * Vec3::Y * NECK
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

/// The gravity (m/s²) with arena event `event` going on (`Sim::gravity`; a client's from its
/// snapshot), or low gravity on all match long.
pub fn gravity_for(event: Option<(ArenaEvent, f32)>, rules: &Rules) -> f32 {
    if rules.in_effect(event, ArenaEvent::LowGravity) {
        GRAVITY * rules.lowgrav_factor
    } else {
        GRAVITY
    }
}

/// One tick of a flying grenade: falling, bouncing off the arena (losing speed), coming to rest
/// on a floor. Returns the new position and velocity, and where it bounced and how hard.
pub fn grenade_step(
    world: &World,
    pos: Vec3,
    vel: Vec3,
    restitution: f32,
    gravity: f32,
) -> (Vec3, Vec3, Option<(Vec3, f32)>) {
    let mut vel = vel;
    vel.y -= gravity * TICK;
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
/// sim will fly it, the arena's moving pieces aside, at `gravity`): a point a tick, and the
/// bounces.
pub fn grenade_path(
    world: &World,
    rules: &Rules,
    from: Vec3,
    vel: Vec3,
    gravity: f32,
) -> (Vec<Vec3>, Vec<Vec3>) {
    let vel = if vel.is_finite() {
        vel.clamp_length_max(rules.grenade_max_speed)
    } else {
        Vec3::ZERO
    };
    let (mut pos, mut vel) = (from, vel);
    let mut path = vec![pos];
    let mut bounces = Vec::new();
    for _ in 0..(rules.grenade_fuse / TICK) as usize {
        let (p, v, b) = grenade_step(world, pos, vel, rules.grenade_restitution, gravity);
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

    #[test]
    fn heads_turn_on_a_tilting_neck() {
        let level = Vec3::Y * EYE;
        let straight = eye_on_neck(level, Quat::IDENTITY);
        assert!((straight.y - EYE).abs() < 1e-6, "{straight}");
        assert!(
            straight.z < -0.05,
            "the eyes in front of the neck: {straight}"
        );
        let chest = chest_top(straight, Quat::IDENTITY);
        assert!(chest.abs_diff_eq(Vec3::Y * (EYE - crate::hitbox::CHEST_DROP), 1e-6));
        // Looking anywhere, the chest stays over the feet.
        for (yaw, pitch) in [(0.0, -1.0), (0.0, 1.2), (1.5, 0.0), (-2.5, -0.6)] {
            let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
            let eye = eye_on_neck(level, rot);
            assert!(
                chest_top(eye, rot).abs_diff_eq(chest, 1e-5),
                "{yaw} {pitch}"
            );
        }
    }

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
    fn a_jump_rises_about_three_quarters_of_a_metre_and_lands() {
        let (mut sim, a, _) = duel();
        sim.step();
        let ground = sim.player(a).unwrap().feet.y;
        let jump = |sim: &mut Sim, n: u32| {
            sim.set_cmd(
                a,
                PlayerCmd {
                    jumps: n,
                    ..Default::default()
                },
            )
        };
        jump(&mut sim, 0);
        sim.step();
        sim.drain_events();
        jump(&mut sim, 1);
        let mut top = ground;
        for _ in 0..120 {
            sim.step();
            top = top.max(sim.player(a).unwrap().feet.y);
        }
        let h = top - ground;
        assert!((h - 0.775).abs() < 0.05, "{h} m");
        let p = sim.player(a).unwrap();
        assert_eq!(p.feet.y, ground);
        assert_eq!(p.vy, 0.0);
        let landed: Vec<f32> = sim
            .drain_events()
            .iter()
            .filter_map(|e| match *e {
                SimEvent::Landed { player, speed, .. } if player == a => Some(speed),
                _ => None,
            })
            .collect();
        assert_eq!(landed.len(), 1, "landed once: {landed:?}");
        assert!(landed[0] > LAND_SPEED && landed[0] < 4.5, "{landed:?}");
    }

    #[test]
    fn jumps_wait_for_landing_allow_a_late_one_and_never_double() {
        let rules = Rules::default();
        let mut m = Moves::default();
        let dt = TICK;
        // The first count seen doesn't jump.
        assert!(jump_step(&rules, &mut m, 5, 0.0, Vec2::ZERO, dt).is_none());
        // Walked off an edge 0.08 s ago: still jumps.
        let mut t = 0.0;
        while t < 0.08 {
            assert!(jump_step(&rules, &mut m, 5, -1.0, Vec2::ZERO, dt).is_none());
            t += dt;
        }
        assert!(jump_step(&rules, &mut m, 6, -1.0, Vec2::ZERO, dt).is_some());
        // In the air: a second press waits, and doesn't jump again before landing.
        assert!(jump_step(&rules, &mut m, 7, 2.0, Vec2::ZERO, dt).is_none());
        for _ in 0..3 {
            assert!(jump_step(&rules, &mut m, 7, -2.0, Vec2::ZERO, dt).is_none());
        }
        // Landing within the buffer jumps.
        assert!(jump_step(&rules, &mut m, 7, 0.0, Vec2::ZERO, dt).is_some());
        // Too long off the ground: no jump.
        let mut m = Moves::default();
        jump_step(&rules, &mut m, 0, 0.0, Vec2::ZERO, dt);
        for _ in 0..12 {
            jump_step(&rules, &mut m, 0, -1.0, Vec2::ZERO, dt);
        }
        assert!(jump_step(&rules, &mut m, 1, -1.0, Vec2::ZERO, dt).is_none());
        // Flying from a pad: no jump.
        let mut m = Moves::default();
        jump_step(&rules, &mut m, 0, 0.0, Vec2::ZERO, dt);
        assert!(jump_step(&rules, &mut m, 1, 0.0, Vec2::ONE, dt).is_none());
    }

    #[test]
    fn a_dash_covers_three_metres_once_every_two_seconds() {
        let (mut sim, a, _) = duel();
        // (Facing -Z, away from b, along the open row.)
        sim.players[0].feet = sim.players[1].feet;
        sim.players[1].feet = sim.level.base_centre(Team::Blue);
        sim.step();
        let dash = |sim: &mut Sim, n: u32| {
            sim.set_cmd(
                a,
                PlayerCmd {
                    dashes: n,
                    ..Default::default()
                },
            )
        };
        let from = sim.player(a).unwrap().feet;
        dash(&mut sim, 1);
        for _ in 0..30 {
            sim.step();
        }
        let d = sim.player(a).unwrap().feet - from;
        assert!((d.length() - 3.0).abs() < 0.1, "{d}");
        assert!(d.z < 0.0, "{d}");
        // Again at once: nothing.
        let from = sim.player(a).unwrap().feet;
        dash(&mut sim, 2);
        for _ in 0..30 {
            sim.step();
        }
        assert!((sim.player(a).unwrap().feet - from).length() < 0.01);
        // Not in the air.
        let mut m = Moves::default();
        let r = Rules::default();
        dash_step(&r, &mut m, 0, Vec2::ZERO, 0.0, Vec2::ZERO, TICK);
        m.off_ground = 0.5;
        assert!(
            dash_step(&r, &mut m, 1, Vec2::X, 0.0, Vec2::ZERO, TICK)
                .0
                .is_none()
        );
    }

    #[test]
    fn the_countdown_holds_the_clock_and_the_triggers() {
        let (mut sim, a, _) = duel();
        sim.begin();
        assert!(matches!(sim.phase, MatchPhase::Countdown(_)));
        let left = sim.time_left;
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                fire: true,
                ..Default::default()
            },
        );
        let mut counts = Vec::new();
        for _ in 0..(sim.rules.countdown / TICK) as usize - 2 {
            sim.step();
            for e in sim.drain_events() {
                match e {
                    SimEvent::Countdown { n } => counts.push(n),
                    SimEvent::Fired { .. } => panic!("fired in the countdown"),
                    _ => {}
                }
            }
        }
        assert_eq!(sim.time_left, left);
        assert!(sim.bolts.is_empty());
        assert_eq!(counts, vec![3, 2, 1]);
        for _ in 0..4 {
            sim.step();
        }
        assert_eq!(sim.phase, MatchPhase::Live);
        assert!(sim.time_left < left);
    }

    #[test]
    fn a_warmup_doesnt_count() {
        let (mut sim, a, _) = duel();
        sim.rules.warmup = 5.0;
        sim.begin();
        sim.players[0].score = 300;
        sim.players[0].tags = 3;
        sim.scores = [300, 0];
        sim.step();
        assert_eq!(sim.time_left, sim.rules.match_time);
        sim.end_warmup();
        assert_eq!(sim.scores, [0, 0]);
        let p = sim.player(a).unwrap();
        assert_eq!((p.score, p.tags), (0, 0));
        assert!(matches!(sim.phase, MatchPhase::Countdown(_)));
    }

    #[test]
    fn tag_outs_say_what_did_it() {
        let (mut sim, a, b) = duel();
        let how = |sim: &mut Sim, team: Option<Team>, pierce: bool| {
            let bolt = Bolt {
                id: 1,
                owner: b,
                owner_team: Team::Blue,
                team,
                pos: Vec3::ZERO,
                vel: Vec3::X,
                life: 1.0,
                extra: BoltExtra {
                    pierce,
                    ..BoltExtra::default()
                },
            };
            sim.players[0].respawn = None;
            sim.players[0].cooldown = 0.0;
            sim.players[0].protect = 0.0;
            sim.players[0].hits = 2;
            sim.drain_events();
            sim.hit_player(0, &bolt, Vec3::ZERO);
            sim.drain_events().into_iter().find_map(|e| match e {
                SimEvent::TaggedOut { player, how, .. } if player == a => Some(how),
                _ => None,
            })
        };
        assert_eq!(
            how(&mut sim, Some(Team::Blue), false),
            Some(TagHow::Blaster)
        );
        assert_eq!(how(&mut sim, None, false), Some(TagHow::Grenade));
        assert_eq!(how(&mut sim, Some(Team::Blue), true), Some(TagHow::Beam));
        sim.give(b, PickupKind::Shotgun);
        assert_eq!(
            how(&mut sim, Some(Team::Blue), false),
            Some(TagHow::Shotgun)
        );
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
    fn crouching_halves_the_speed() {
        let (mut sim, a, _) = duel();
        let mut walk = |crouch: bool| {
            sim.set_cmd(
                a,
                PlayerCmd {
                    stick: Vec2::new(0.0, -1.0),
                    crouch,
                    ..Default::default()
                },
            );
            let from = sim.player(a).unwrap().feet;
            for _ in 0..10 {
                sim.step();
            }
            let d = sim.player(a).unwrap().feet - from;
            sim.players[0].feet = from;
            d.length()
        };
        let plain = walk(false);
        let low = walk(true);
        assert!(
            (low / plain - 0.5).abs() < 0.05,
            "{low} m against {plain} m"
        );
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
    fn a_respawned_player_is_protected_until_it_runs_out_or_they_fire() {
        let (mut sim, a, b) = duel();
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
        sim.players[0].respawn = Some(TICK);
        sim.step();
        assert!(sim.player(a).unwrap().alive());
        assert!(sim.player(a).unwrap().protect > 1.0);
        sim.hit_player(0, &bolt, Vec3::ZERO);
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // Firing ends it.
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                fire: true,
                ..Default::default()
            },
        );
        sim.step();
        assert_eq!(sim.player(a).unwrap().protect, 0.0);
        sim.hit_player(0, &bolt, Vec3::ZERO);
        assert_eq!(sim.player(a).unwrap().hits, 1);
        // And it runs out by itself.
        sim.players[0].respawn = Some(TICK);
        sim.set_cmd(a, PlayerCmd::default());
        for _ in 0..(sim.rules.spawn_protect / TICK) as usize + 2 {
            sim.step();
        }
        assert_eq!(sim.player(a).unwrap().protect, 0.0);
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
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Charge);
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
        // Another special gun replaces it.
        sim.players[0].hits = 0;
        assert!(
            sim.player(a)
                .unwrap()
                .wants(PickupKind::Shotgun, &sim.rules)
        );
        sim.give(a, PickupKind::Shotgun);
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Shotgun);
    }

    #[test]
    fn ricochet_bolts_bounce_off_any_wall_but_not_off_targets() {
        let (mut sim, a, _) = duel();
        sim.give(a, PickupKind::Ricochet);
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Ricochet);
        assert!(sim.player(a).unwrap().fire_interval(&sim.rules) > sim.rules.fire_cooldown);
        let level = sim.level.clone();
        let rules = sim.rules.clone();
        let bolt = |ricochet| Bolt {
            id: 0,
            owner: a,
            owner_team: Team::Red,
            team: Some(Team::Red),
            pos: Vec3::ZERO,
            vel: Vec3::new(10.0, 0.0, 0.0),
            life: 1.0,
            extra: BoltExtra {
                ricochet,
                ..BoltExtra::default()
            },
        };
        // A plain wall, away from any mirror or panel.
        let q = Vec3::new(1000.0, 1.0, 1000.0);
        let mut b = bolt(true);
        for k in 0..rules.ricochet_bounces {
            assert!(
                crate::shot::bounce(&level, &rules, &mut b, q, Vec3::NEG_X, false).is_some(),
                "bounce {k}"
            );
        }
        assert!(
            crate::shot::bounce(&level, &rules, &mut b, q, Vec3::NEG_X, false).is_none(),
            "no more"
        );
        // A blaster bolt stops at a wall.
        let mut b = bolt(false);
        assert!(crate::shot::bounce(&level, &rules, &mut b, q, Vec3::NEG_X, false).is_none());
        // ... but not in a mirror storm: once.
        let mut b = bolt(false);
        assert!(crate::shot::bounce(&level, &rules, &mut b, q, Vec3::NEG_X, true).is_some());
        assert!(crate::shot::bounce(&level, &rules, &mut b, q, Vec3::NEG_X, true).is_none());
        // A ricochet bolt stops at a target panel (and scores there).
        let t = level.targets[0];
        let mut b = bolt(true);
        let on_t = (t.min + t.max) / 2.0;
        assert!(level.scores_at(on_t));
        assert!(crate::shot::bounce(&level, &rules, &mut b, on_t, Vec3::NEG_X, false).is_none());

        // Fired along a wall's normal, it comes back.
        sim.players[1].feet = Vec3::new(0.0, 0.0, 1000.0);
        let me = sim.player(a).unwrap();
        let from = me.eye();
        let (_, q, n) = (1..36)
            .find_map(|k| {
                let d = Quat::from_rotation_y(k as f32 * 0.17) * Vec3::NEG_Z;
                sim.world
                    .ray(from, from + d * 12.0)
                    .filter(|(_, q, _)| !level.scores_at(*q))
            })
            .expect("a wall near the spawn");
        let _ = q;
        let dir = -n;
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: from - me.feet,
                    rot: look_rot(dir),
                },
                fire: true,
                ..Default::default()
            },
        );
        sim.drain_events();
        for _ in 0..60 {
            sim.step();
        }
        let bounced = sim
            .drain_events()
            .iter()
            .filter(|e| matches!(e, SimEvent::Reflected { .. }))
            .count();
        assert!(bounced >= 1, "bounced {bounced}");
    }

    #[test]
    fn the_lance_hits_at_once_and_head_hits_count_double() {
        let (mut sim, a, b) = duel();
        sim.rules.reload_rate = 0.0;
        sim.give(a, PickupKind::Lance);
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Lance);
        let aim = |sim: &Sim, at: Vec3| {
            let me = sim.player(a).unwrap();
            let muzzle_pos = me.eye() - Vec3::Y * 0.2 - me.feet;
            let dir = (at - (me.feet + muzzle_pos)).normalize();
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: muzzle_pos,
                    rot: look_rot(dir),
                },
                fire: true,
                ..Default::default()
            }
        };
        // At the chest: one hit, in the tick it's fired, and no bolt left flying.
        let chest = sim.player(b).unwrap().chest();
        sim.set_cmd(a, aim(&sim, chest));
        sim.drain_events();
        sim.step();
        assert_eq!(sim.player(b).unwrap().hits, 1);
        assert_eq!(
            sim.player(a).unwrap().ammo,
            sim.rules.ammo - sim.rules.lance_cost
        );
        let ev = sim.drain_events();
        assert!(ev.iter().any(|e| matches!(e, SimEvent::Streak { .. })));
        assert!(!ev.iter().any(|e| matches!(e, SimEvent::Headshot { .. })));
        assert!(sim.bolts.iter().all(|x| x.owner != a));
        // At the head (once the hit cooldown and the lance's own are over): two more.
        let cmd = aim(&sim, chest);
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..(sim.rules.hit_cooldown.max(sim.rules.lance_cooldown) / TICK) as usize + 2 {
            sim.step();
        }
        let head = sim.player(b).unwrap().eye();
        sim.set_cmd(a, aim(&sim, head));
        sim.step();
        let ev = sim.drain_events();
        assert!(
            ev.iter().any(|e| matches!(e, SimEvent::Headshot { .. })),
            "{ev:?}"
        );
        assert_eq!(sim.player(b).unwrap().hits, 3);
        assert!(ev.iter().any(|e| matches!(e, SimEvent::TaggedOut { .. })));
    }

    #[test]
    fn mirrors_put_up_turn_back_every_bolt() {
        let (mut sim, a, b) = duel();
        sim.give(a, PickupKind::Mirror);
        assert_eq!(sim.player(a).unwrap().gadget, Some((Gadget::Mirror, 1)));
        // Taking a barrier replaces it.
        assert!(
            sim.player(a)
                .unwrap()
                .wants(PickupKind::Barrier, &sim.rules)
        );
        assert!(!sim.player(a).unwrap().wants(PickupKind::Mirror, &sim.rules));
        sim.set_cmd(
            a,
            PlayerCmd {
                yaw: 0.0,
                gadget: true,
                ..Default::default()
            },
        );
        sim.step();
        sim.step();
        assert_eq!(sim.barriers.len(), 1);
        assert!(sim.barriers[0].mirror);
        assert!((sim.barriers[0].left - (sim.rules.mirror_time - TICK * 2.0)).abs() < 1e-4);
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
        // b's bolt at a comes back off it (and passes b, its shooter).
        sim.drain_events();
        sim.bolts.push(bolt(b, Team::Blue, cb, ca));
        for _ in 0..5 {
            sim.step();
        }
        let back = sim.bolts.iter().find(|x| x.id == 7).expect("still flying");
        assert!(back.vel.dot(ca - cb) < 0.0, "turned back");
        for _ in 0..10 {
            sim.step();
        }
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Reflected { .. }))
        );
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // a's own bolt at b comes back too.
        sim.bolts.push(bolt(a, Team::Red, ca, cb));
        for _ in 0..10 {
            sim.step();
        }
        assert_eq!(sim.player(b).unwrap().hits, 0);
        assert!(
            sim.barrier_blocks(ca, cb, Some(Team::Red)),
            "the bots see it in the way"
        );
        for _ in 0..(sim.rules.mirror_time / TICK) as usize {
            sim.step();
        }
        assert!(sim.barriers.is_empty());
    }

    #[test]
    fn pulse_sensors_stick_show_enemies_nearby_and_uncloak_them() {
        let (mut sim, a, b) = duel();
        let c = sim.add_player("c", Some(Team::Blue), false).unwrap();
        sim.give(a, PickupKind::Sensor);
        sim.give(b, PickupKind::Cloak);
        assert!(sim.player(b).unwrap().cloaked());
        // Thrown towards b (3 m off along -z).
        let fb = sim.player(b).unwrap().feet;
        sim.players[2].feet = fb + Vec3::new(0.0, 0.0, -200.0);
        let me = sim.player(a).unwrap();
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                muzzle: Pose {
                    pos: me.eye() - Vec3::Y * 0.2 - me.feet,
                    rot: look_rot(Vec3::new(0.0, -0.5, -1.0).normalize()),
                },
                gadget: true,
                ..Default::default()
            },
        );
        sim.drain_events();
        sim.step();
        assert_eq!(sim.sensors.len(), 1);
        assert_eq!(sim.player(a).unwrap().gadget, None);
        for _ in 0..90 {
            sim.step();
        }
        let s = sim.sensors[0];
        assert!(s.stuck, "landed: {s:?}");
        let ev = sim.drain_events();
        assert!(ev.iter().any(|e| matches!(e, SimEvent::SensorStuck { .. })));
        let seen = ev.iter().find_map(|e| match *e {
            SimEvent::Pulse { seen, team, .. } => Some((seen, team)),
            _ => None,
        });
        let (seen, team) = seen.expect("a pulse");
        assert_eq!(team, Team::Red);
        assert!(seen & (1 << b) != 0, "b seen, cloaked as it is");
        assert_eq!(sim.player(b).unwrap().cloak, 0.0, "b's cloak gone");
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::Uncloaked { player } if *player == b))
        );
        assert!(seen & (1 << c) == 0, "c too far");
        assert!(seen & (1 << a) == 0, "not its own team");
        assert!(sim.player(b).unwrap().revealed > 0.0);
        assert_eq!(sim.player(c).unwrap().revealed, 0.0);
        for _ in 0..(sim.rules.sensor_time / TICK) as usize + 2 {
            sim.step();
        }
        assert!(sim.sensors.is_empty());
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::SensorDown { .. }))
        );
    }

    #[test]
    fn decoys_run_ahead_and_pop_at_an_enemy_bolt() {
        let (mut sim, a, b) = duel();
        sim.give(a, PickupKind::Decoy);
        let fa = sim.player(a).unwrap().feet;
        // Sent off along -z (towards b), into the open.
        sim.players[1].feet = fa + Vec3::new(30.0, 0.0, 0.0);
        sim.set_cmd(
            a,
            PlayerCmd {
                yaw: 0.0,
                gadget: true,
                ..Default::default()
            },
        );
        sim.step();
        assert_eq!(sim.decoys.len(), 1);
        for _ in 0..20 {
            sim.step();
        }
        let d = sim.decoys[0];
        assert!(d.feet.z < fa.z - 0.5, "ran ahead: {:?} from {fa:?}", d.feet);
        assert!(
            sim.snapshot()
                .decoys
                .iter()
                .any(|s| s.id == d.id && s.owner == a)
        );
        // A's own bolt passes it; b's pops it.
        let bolt = |owner, team| Bolt {
            id: 9,
            owner,
            owner_team: team,
            team: Some(team),
            pos: d.chest() + Vec3::new(0.8, 0.0, 0.0),
            vel: Vec3::new(-40.0, 0.0, 0.0),
            life: 0.2,
            extra: BoltExtra::default(),
        };
        sim.drain_events();
        sim.bolts.push(bolt(a, Team::Red));
        for _ in 0..4 {
            sim.step();
        }
        assert_eq!(sim.decoys.len(), 1);
        let d = sim.decoys[0];
        sim.bolts.push(Bolt {
            pos: d.chest() + Vec3::new(0.8, 0.0, 0.0),
            ..bolt(b, Team::Blue)
        });
        for _ in 0..4 {
            sim.step();
        }
        assert!(sim.decoys.is_empty());
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::DecoyPopped { by, .. } if *by == b))
        );
        assert_eq!(sim.player(a).unwrap().hits, 0);
        // Another runs out of time.
        sim.give(a, PickupKind::Decoy);
        sim.set_cmd(a, PlayerCmd::default());
        sim.step();
        sim.set_cmd(
            a,
            PlayerCmd {
                gadget: true,
                ..Default::default()
            },
        );
        sim.step();
        assert_eq!(sim.decoys.len(), 1);
        for _ in 0..(sim.rules.decoy_time / TICK) as usize + 1 {
            sim.step();
        }
        assert!(sim.decoys.is_empty());
    }

    #[test]
    fn the_med_beam_heals_a_team_mate_in_front() {
        let (mut sim, a, _) = duel();
        sim.rules.reload_rate = 0.0;
        let c = sim.add_player("c", Some(Team::Red), false).unwrap();
        // a and c (hurt, 1.5 m in front) on open floor off the spawn pads (which heal).
        let nav = &sim.level.nav;
        let fa = (0..nav.pos.len())
            .filter(|&i| nav.valid[i] && sim.level.spawn_at(nav.pos[i]).is_none())
            .map(|i| nav.pos[i])
            .find(|p| {
                let q = *p + Vec3::new(0.0, 0.0, -1.5);
                sim.world.clear(*p + Vec3::Y * EYE, q + Vec3::Y * 1.0)
                    && sim
                        .world
                        .ground(q.x, q.z, q.y + 0.5)
                        .is_some_and(|y| (y - q.y).abs() < 0.05)
                    && sim.level.spawn_at(q).is_none()
            })
            .expect("open floor");
        sim.players[0].feet = fa;
        let ci = sim.players.iter().position(|p| p.id == c).unwrap();
        sim.players[ci].feet = fa + Vec3::new(0.0, 0.0, -1.5);
        sim.players[ci].hits = 2;
        sim.players[ci].ammo = 10;
        sim.players[1].feet = fa + Vec3::new(40.0, 0.0, 0.0);
        sim.give(a, PickupKind::Medbeam);
        assert_eq!(sim.player(a).unwrap().medbeam_left, sim.rules.medbeam_time);
        let me = sim.player(a).unwrap();
        let (eye, feet) = (me.eye(), me.feet);
        let to = sim.players[ci].chest() - eye;
        let held = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: eye - feet,
                rot: look_rot(to),
            },
            gadget: true,
            ..Default::default()
        };
        sim.set_cmd(a, held);
        sim.drain_events();
        for _ in 0..(sim.rules.medbeam_tick / TICK) as usize + 2 {
            sim.step();
        }
        assert_eq!(sim.player(a).unwrap().mending, Some(c));
        assert_eq!(sim.player(c).unwrap().hits, 1, "a hit healed");
        assert!(sim.player(c).unwrap().ammo > 10, "refilled some");
        assert!(sim.drain_events().iter().any(
            |e| matches!(e, SimEvent::Mended { player, by, hits: 1 } if *player == c && *by == a)
        ));
        // Let go: no beam, none used.
        sim.set_cmd(
            a,
            PlayerCmd {
                gadget: false,
                ..held
            },
        );
        sim.step();
        let left = sim.player(a).unwrap().medbeam_left;
        sim.step();
        assert_eq!(sim.player(a).unwrap().mending, None);
        assert_eq!(sim.player(a).unwrap().medbeam_left, left);
        // Aimed away: nobody.
        sim.set_cmd(
            a,
            PlayerCmd {
                muzzle: Pose {
                    pos: eye - feet,
                    rot: look_rot(Vec3::X),
                },
                ..held
            },
        );
        sim.step();
        assert_eq!(sim.player(a).unwrap().mending, None);
        // Held on until the beam's used up: the gadget's gone.
        sim.set_cmd(a, held);
        for _ in 0..(sim.rules.medbeam_time / TICK) as usize + 2 {
            sim.step();
        }
        assert_eq!(sim.player(c).unwrap().hits, 0);
        assert_eq!(sim.player(a).unwrap().gadget, None);
    }

    #[test]
    fn arena_events_never_come_twice_in_a_row() {
        let mut sim = Sim::new(3, Rules::default());
        // (A bounty needs someone to be on.)
        sim.add_player("a", Some(Team::Red), false).unwrap();
        let mut last = None;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..40 {
            sim.event = None;
            sim.next_event = 0.0;
            sim.step();
            let kind = sim.event.expect("one started").0;
            assert_ne!(Some(kind), last);
            last = Some(kind);
            seen.insert(kind);
        }
        assert_eq!(seen.len(), ArenaEvent::ALL.len(), "{seen:?}");
        sim.start_event(ArenaEvent::MirrorStorm);
        assert!(sim.storm());
        assert_eq!(ArenaEvent::parse("storm"), Some(ArenaEvent::MirrorStorm));
    }

    #[test]
    fn match_modifiers_stay_on_and_never_come_as_events() {
        let modifiers = ArenaEvent::parse_modifiers("dark,lowgrav,mirror").unwrap();
        assert_eq!(modifiers, ArenaEvent::MODIFIERS);
        assert_eq!(ArenaEvent::parse_modifiers("bounty"), None);
        let rules = Rules {
            modifiers,
            ..Rules::default()
        };
        let mut sim = Sim::new(3, rules);
        sim.add_player("a", Some(Team::Red), false).unwrap();
        assert!(sim.storm());
        assert!(sim.gravity() < crate::collide::GRAVITY);
        assert!(sim.rules.in_effect(None, ArenaEvent::Blackout));
        for _ in 0..20 {
            sim.event = None;
            sim.next_event = 0.0;
            sim.step();
            let kind = sim.event.expect("one started").0;
            assert!(!ArenaEvent::MODIFIERS.contains(&kind), "{kind:?}");
        }
    }

    #[test]
    fn low_gravity_jumps_higher() {
        let apex = |low: bool| {
            let (mut sim, a, _) = duel();
            if low {
                sim.start_event(ArenaEvent::LowGravity);
                assert!(sim.gravity() < crate::collide::GRAVITY);
            }
            sim.players[1].feet += Vec3::new(0.0, 0.0, -50.0);
            sim.step();
            let y0 = sim.player(a).unwrap().feet.y;
            sim.set_cmd(
                a,
                PlayerCmd {
                    jumps: 1,
                    ..Default::default()
                },
            );
            let mut top = y0;
            for _ in 0..240 {
                sim.step();
                top = top.max(sim.player(a).unwrap().feet.y);
            }
            top - y0
        };
        let (normal, low) = (apex(false), apex(true));
        assert!(normal > 0.3, "{normal}");
        assert!(low > normal * 2.0, "{low} vs {normal}");
    }

    #[test]
    fn a_bounty_on_the_top_scorer_pays_out_either_way() {
        let (mut sim, a, b) = duel();
        sim.players[1].score = 500;
        sim.start_event(ArenaEvent::Bounty);
        assert_eq!(sim.bounty, Some(b));
        assert_eq!(sim.snapshot().bounty, Some(b));
        // a tags b out: the bounty's points, and it's over.
        let before = sim.scores[Team::Red.index()];
        sim.players[1].hits = sim.rules.hits_to_tag - 1;
        sim.players[1].protect = 0.0;
        sim.players[1].feet = sim.player(a).unwrap().feet + Vec3::new(0.0, 0.0, -3.0);
        shoot(&mut sim, a, b);
        assert!(sim.player(b).unwrap().respawn.is_some(), "tagged out");
        assert_eq!(
            sim.scores[Team::Red.index()] - before,
            sim.rules.bounty_points
        );
        assert_eq!(sim.bounty, None);
        assert!(sim.event.is_none());
        let ev = sim.drain_events();
        assert!(ev.iter().any(
            |e| matches!(e, SimEvent::BountyClaimed { player, by } if *player == b && *by == a)
        ));
        // Lasted out: the bounty's player gets it.
        sim.players[0].score = 900;
        sim.start_event(ArenaEvent::Bounty);
        assert_eq!(sim.bounty, Some(a));
        let before = sim.player(a).unwrap().score;
        for _ in 0..(sim.rules.bounty_time / TICK) as usize + 2 {
            sim.step();
        }
        assert_eq!(
            sim.player(a).unwrap().score - before,
            sim.rules.bounty_points
        );
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::BountySurvived { player } if *player == a))
        );
    }

    #[test]
    fn dual_pistols_fire_from_each_hand_sharing_the_shots() {
        let (mut sim, a, _) = duel();
        sim.rules.reload_rate = 0.0;
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.give(a, PickupKind::Dual);
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Dual);
        let right = Vec3::new(0.25, 1.4, -0.2);
        let left = Vec3::new(-0.25, 1.4, -0.2);
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            gun2_hand: Some(Hand::Left),
            muzzle: Pose {
                pos: right,
                rot: Quat::IDENTITY,
            },
            muzzle2: Pose {
                pos: left,
                rot: Quat::IDENTITY,
            },
            ..Default::default()
        };
        let feet = sim.player(a).unwrap().feet;
        let mine = |sim: &Sim| -> Vec<Bolt> {
            sim.bolts.iter().filter(|b| b.owner == a).copied().collect()
        };
        // The left one alone: a bolt from the left hand.
        sim.set_cmd(a, PlayerCmd { fire2: true, ..cmd });
        sim.step();
        let b = mine(&sim);
        assert_eq!(b.len(), 1);
        assert!(
            (b[0].pos - (feet + left)).length() < 1.7,
            "{:?}",
            b[0].pos - feet
        );
        assert!(b[0].pos.x < feet.x, "from the left");
        assert_eq!(sim.player(a).unwrap().ammo, sim.rules.ammo - 1);
        // Both at once (the left one's still cooling down: only the right fires).
        sim.set_cmd(
            a,
            PlayerCmd {
                fire: true,
                fire2: false,
                ..cmd
            },
        );
        sim.step();
        assert_eq!(sim.player(a).unwrap().ammo, sim.rules.ammo - 2);
        // Later, both together: two shots, numbered in turn.
        sim.set_cmd(a, cmd);
        for _ in 0..(sim.rules.dual_cooldown / TICK) as usize + 2 {
            sim.step();
        }
        sim.bolts.clear();
        sim.set_cmd(
            a,
            PlayerCmd {
                fire: true,
                fire2: true,
                shot: 9,
                ..cmd
            },
        );
        sim.step();
        assert_eq!(mine(&sim).len(), 2);
        assert_eq!(sim.player(a).unwrap().ammo, sim.rules.ammo - 4);
        // Lost with a tag-out: the blaster again, one hand.
        sim.players[0].lose_powerups();
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Blaster);
        assert_eq!(sim.snapshot().players[0].gun2_hand, None);
    }

    #[test]
    fn prisms_are_taken_carried_home_dropped_and_returned() {
        let rules = Rules {
            mode: crate::level::GameMode::Prism,
            ..Rules::default()
        };
        let mut sim = Sim::new(5, rules);
        assert!(sim.level.targets.is_empty(), "no targets in prism mode");
        let homes = sim.level.prisms.expect("prism homes");
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        let b = sim.add_player("b", Some(Team::Blue), false).unwrap();
        sim.step();
        assert_eq!(sim.prisms.len(), 2);
        assert!(sim.prisms.iter().all(|p| p.at_home()));
        let blue = homes[Team::Blue.index()];
        let red = homes[Team::Red.index()];
        // a (Red) walks up to Blue's prism: taken.
        sim.players[0].feet = blue;
        sim.players[0].protect = 0.0;
        sim.players[1].feet = red + Vec3::new(0.0, 0.0, 3.0);
        sim.drain_events();
        sim.step();
        assert_eq!(sim.prisms[Team::Blue.index()].carrier, Some(a));
        assert!(sim.carrying(a));
        // It can't fire with it.
        sim.set_cmd(
            a,
            PlayerCmd {
                gun_hand: Some(Hand::Right),
                fire: true,
                ..Default::default()
            },
        );
        sim.step();
        assert!(sim.bolts.iter().all(|x| x.owner != a));
        sim.set_cmd(a, PlayerCmd::default());
        // Tagged out: dropped; b touches it: home again.
        let i = sim.players.iter().position(|p| p.id == a).unwrap();
        sim.players[i].respawn = Some(5.0);
        for _ in 0..30 {
            sim.step();
        }
        let pr = sim.prisms[Team::Blue.index()];
        assert!(pr.carrier.is_none() && pr.dropped.is_some(), "{pr:?}");
        sim.players[1].feet = pr.pos - Vec3::Y * 0.5;
        sim.step();
        assert!(sim.prisms[Team::Blue.index()].at_home());
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::PrismReturned { by: Some(x), .. } if *x == b))
        );
        // Taken again and carried to Red's home (its own prism there): captured.
        sim.players[i].respawn = None;
        sim.players[i].feet = blue;
        sim.players[1].feet = red + Vec3::new(0.0, 0.0, 40.0);
        sim.step();
        assert_eq!(sim.prisms[Team::Blue.index()].carrier, Some(a));
        let before = sim.scores[Team::Red.index()];
        sim.players[i].feet = red;
        sim.step();
        assert_eq!(
            sim.scores[Team::Red.index()] - before,
            sim.rules.prism_points
        );
        assert!(sim.prisms.iter().all(|p| p.at_home()));
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::PrismCaptured { by, .. } if *by == a))
        );
    }

    #[test]
    fn infection_turns_survivors_into_phantoms_until_none_are_left() {
        let rules = Rules {
            mode: crate::level::GameMode::Infection,
            ..Rules::default()
        };
        let mut sim = Sim::new(4, rules);
        assert!(sim.level.targets.is_empty());
        let ids: Vec<PlayerId> = (0..8)
            .map(|k| sim.add_player(&format!("p{k}"), None, false).unwrap())
            .collect();
        sim.step();
        assert!(sim.infected);
        let red = |sim: &Sim| sim.players.iter().filter(|p| p.team == Team::Red).count();
        assert_eq!(red(&sim), 1, "one starts as a phantom");
        assert!(sim.snapshot().infected);
        let phantom = sim.players.iter().find(|p| p.team == Team::Red).unwrap().id;
        assert!(sim.player(phantom).unwrap().cloaked());
        // A late comer is a phantom.
        let late = sim.add_player("late", Some(Team::Blue), false).unwrap();
        assert_eq!(sim.player(late).unwrap().team, Team::Red);
        // A phantom's bolt: short, and a tag-out at once.
        let bolts = crate::shot::as_phantom(
            crate::shot::shot_bolts(Shot::Bolt, Vec3::NEG_Z, &sim.rules, 0),
            &sim.rules,
            Team::Red,
            true,
        );
        assert_eq!(bolts[0].2.damage, sim.rules.phantom_damage);
        assert!(bolts[0].0.length() * bolts[0].1 <= sim.rules.phantom_range + 0.01);
        // A phantom goes down in one survivor's hit, and shows further off than a cloak.
        {
            let pi = sim.players.iter().position(|p| p.id == phantom).unwrap();
            let by = sim
                .players
                .iter()
                .find(|p| p.team == Team::Blue)
                .unwrap()
                .id;
            sim.players[pi].protect = 0.0;
            let b = Bolt {
                id: 98,
                owner: by,
                owner_team: Team::Blue,
                team: Some(Team::Blue),
                pos: Vec3::ZERO,
                vel: Vec3::X,
                life: 1.0,
                extra: BoltExtra::default(),
            };
            let at = sim.players[pi].chest();
            sim.hit_player(pi, &b, at);
            assert!(!sim.players[pi].alive(), "phantom tagged out");
            assert!(sim.cloak_near(&sim.players[pi]) > CLOAK_NEAR);
        }
        // Survivors score while they last.
        let before = sim.scores[Team::Blue.index()];
        for _ in 0..(sim.rules.survive_every / TICK) as usize + 1 {
            sim.step();
        }
        assert!(sim.scores[Team::Blue.index()] > before);
        // Every survivor tagged out: all phantoms, and it's over.
        let survivors: Vec<PlayerId> = sim
            .players
            .iter()
            .filter(|p| p.team == Team::Blue)
            .map(|p| p.id)
            .collect();
        for id in survivors {
            let i = sim.players.iter().position(|p| p.id == id).unwrap();
            sim.players[i].protect = 0.0;
            sim.players[i].cooldown = 0.0;
            let b = Bolt {
                id: 99,
                owner: phantom,
                owner_team: Team::Red,
                team: Some(Team::Red),
                pos: Vec3::ZERO,
                vel: Vec3::X,
                life: 1.0,
                extra: BoltExtra {
                    damage: sim.rules.hits_to_tag,
                    ..BoltExtra::default()
                },
            };
            let at = sim.players[i].chest();
            sim.hit_player(i, &b, at);
            assert_eq!(sim.players[i].team, Team::Red, "infected");
        }
        assert!(
            sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::Infected { .. }))
        );
        sim.step();
        assert!(sim.over);
        let _ = ids;
    }

    #[test]
    fn barriers_stop_enemy_bolts_only() {
        let (mut sim, a, b) = duel();
        sim.give(a, PickupKind::Barrier);
        assert_eq!(sim.player(a).unwrap().gadget, Some((Gadget::Barrier, 1)));
        // Facing b (3 m away along -z): up between them.
        sim.set_cmd(
            a,
            PlayerCmd {
                yaw: 0.0,
                gadget: true,
                ..Default::default()
            },
        );
        sim.step();
        sim.step();
        assert_eq!(sim.barriers.len(), 1, "one barrier for one press");
        assert_eq!(sim.player(a).unwrap().gadget, None);
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
    fn vr_players_throw_harder() {
        let thrown = |vr: bool, speed: f32| {
            let (mut sim, a, _) = duel();
            sim.give(a, PickupKind::Grenade);
            sim.set_cmd(
                a,
                PlayerCmd {
                    vr,
                    throw: Some((Vec3::new(0.0, 1.5, 0.0), Vec3::new(0.0, 0.0, -speed))),
                    ..Default::default()
                },
            );
            sim.step();
            // (Its first tick of flight: gravity aside, as thrown.)
            sim.grenades[0].vel.with_y(0.0).length()
        };
        assert!((thrown(false, 5.0) - 5.0).abs() < 0.01);
        assert!((thrown(true, 5.0) - 15.0).abs() < 0.01);
        let max = Rules::default().grenade_max_speed;
        assert!((thrown(false, 30.0) - max).abs() < 0.01);
        assert!((thrown(true, 30.0) - 3.0 * max).abs() < 0.01);
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
        sim.player_mut(a).unwrap().claim_block = 0;
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

    #[test]
    fn a_new_player_ignores_claims_from_before() {
        // A client's prediction from the last arena, still coming in after the next match
        // began, doesn't drag its player off the spawn pad.
        let mut sim = Sim::new(3, Rules::default());
        let a = sim.add_player("A", Some(Team::Red), false).unwrap();
        let start = sim.player(a).unwrap().feet;
        for _ in 0..20 {
            sim.set_cmd(
                a,
                PlayerCmd {
                    feet: Some(Vec3::ZERO),
                    ..Default::default()
                },
            );
            sim.step();
        }
        let moved = (sim.player(a).unwrap().feet - start).length();
        assert!(moved < 0.05, "{moved}");
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
        // Off the spawn pads (which would heal the hit), in the open near the middle.
        let world = sim.level.world();
        let open = |p: Vec3| {
            let eye = p + Vec3::Y * EYE;
            sim.level.spawn_at(p).is_none() && world.clear(eye, eye + Vec3::X * 2.5)
        };
        sim.players[1].feet = (0..sim.level.dims().cells())
            .filter(|&n| sim.level.nav.valid[n])
            .map(|n| sim.level.nav.pos[n])
            .filter(|p| open(*p))
            .min_by(|a, b| {
                a.distance(Vec3::Z * 6.0)
                    .total_cmp(&b.distance(Vec3::Z * 6.0))
            })
            .unwrap();
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
        // Blue on red's pads: not healed (tagged out).
        sim.players[1].feet = sim.level.spawns[Team::Red.index()][0].pos;
        sim.step();
        assert_eq!(sim.player(a).unwrap().hits, 2);
        assert!(!sim.player(b).unwrap().alive());
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

    /// `lagged_shot`'s scene: a Red shooter, and a Blue player 3.5 m away walking along x.
    fn strafing() -> (Sim, PlayerId, PlayerId) {
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
        (sim, a, b)
    }

    /// A Blue player strafing across Red's base, a Red shooter 3.5 m away aiming where the
    /// target was `lag` ticks ago (leading it as it then moved); the command made at
    /// `view_tick` = now - `lag` if `compensated`, else 0. Returns the target's hits.
    fn lagged_shot(lag: u64, compensated: bool) -> u8 {
        let (mut sim, a, b) = strafing();
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
    fn client_hit_claims_are_checked() {
        let (mut sim, a, b) = strafing();
        sim.players[0].client_hits = true;
        for _ in 0..40 {
            sim.step();
        }
        let lag = 20;
        let view = sim.tick + 1 - lag;
        let me = sim.player(a).unwrap();
        let muzzle = me.eye() - Vec3::Y * 0.2;
        let ghost = |sim: &Sim, t: u64| {
            let h = sim.past(t).unwrap();
            *sim.history[h].1.iter().find(|g| g.id == b).unwrap()
        };
        let chest =
            |g: &Ghost| g.eye - Vec3::Y * (crate::hitbox::CHEST_DROP + crate::hitbox::CHEST_HALF.y);
        // Aimed at where the target was shown two ticks of flight after the view.
        let dir = (chest(&ghost(&sim, view + 2)) - muzzle).normalize();
        let cmd = PlayerCmd {
            gun_hand: Some(Hand::Right),
            muzzle: Pose {
                pos: muzzle - me.feet,
                rot: look_rot(dir),
            },
            fire: true,
            view_tick: view,
            shot: 1,
            ..Default::default()
        };
        sim.set_cmd(a, cmd);
        // What the client saw its bolt do: the k-th tick of flight against the view + k.
        let speed = sim.rules.blaster_bolt_speed;
        let (k, at) = (0..lag)
            .find_map(|k| {
                let g = ghost(&sim, view + k);
                let from = muzzle + dir * speed * TICK * k as f32;
                let to = from + dir * speed * TICK;
                first_hit(g.feet, g.eye, &g.cmd, from, to).map(|(t, _)| (k, from.lerp(to, t)))
            })
            .expect("the client's bolt hits");
        sim.step();
        sim.set_cmd(a, PlayerCmd { fire: false, ..cmd });
        for _ in 0..3 {
            sim.step();
        }
        assert_eq!(
            sim.player(b).unwrap().hits,
            0,
            "the sim's own bolt doesn't hit"
        );
        let claim = HitClaim {
            shot: 1,
            pellet: 0,
            victim: b,
            view: (view + k) as f32 + 0.3,
            at,
            head: false,
        };
        let bad = [
            HitClaim {
                at: at + Vec3::Y * 2.5,
                ..claim
            },
            HitClaim {
                at: at + dir * 6.0,
                ..claim
            },
            HitClaim { victim: a, ..claim },
            HitClaim { pellet: 1, ..claim },
            HitClaim {
                view: (sim.tick - MAX_LAG - 2) as f32,
                ..claim
            },
        ];
        for c in bad {
            assert!(
                matches!(sim.claim_hit(a, c), Claim::Rejected(_)),
                "rejected: {c:?}"
            );
        }
        assert_eq!(
            sim.claim_hit(a, HitClaim { shot: 2, ..claim }),
            Claim::Early
        );
        assert_eq!(sim.claim_hit(a, claim), Claim::Accepted);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        assert!(
            matches!(sim.claim_hit(a, claim), Claim::Rejected(_)),
            "once only"
        );
        // A player the sim judges can't claim.
        assert!(matches!(sim.claim_hit(b, claim), Claim::Rejected(_)));
    }

    #[test]
    fn shotgun_spread_is_the_same_from_the_seed() {
        let rules = Rules::default();
        let seed = crate::shot::shot_seed(3, 17);
        let a = crate::shot::shot_bolts(Shot::Spread, Vec3::NEG_Z, &rules, seed);
        let b = crate::shot::shot_bolts(Shot::Spread, Vec3::NEG_Z, &rules, seed);
        assert_eq!(a.len(), rules.shotgun_bolts as usize);
        assert_eq!(a, b);
        let c = crate::shot::shot_bolts(
            Shot::Spread,
            Vec3::NEG_Z,
            &rules,
            crate::shot::shot_seed(3, 18),
        );
        assert_ne!(a, c);
    }

    #[test]
    fn the_enemy_spawn_tags_out_at_once() {
        let mut sim = Sim::new(3, Rules::default());
        let a = sim.add_player("a", Some(Team::Red), false).unwrap();
        sim.step();
        // On one's own pads: fine.
        let own = sim.level.spawns[Team::Red.index()][0];
        sim.teleport(a, own.pos, own.yaw);
        sim.players[0].shield = 5.0;
        sim.step();
        assert!(sim.player(a).unwrap().alive());
        // On the enemy's: tagged out, shield or not.
        let theirs = sim.level.spawns[Team::Blue.index()][0];
        sim.teleport(a, theirs.pos, theirs.yaw);
        sim.drain_events();
        sim.step();
        assert!(!sim.player(a).unwrap().alive());
        assert!(
            sim.drain_events().iter().any(
                |e| matches!(e, SimEvent::TaggedOut { player, by: HAZARD, .. } if *player == a)
            )
        );
    }

    #[test]
    fn bots_keep_off_the_enemy_spawn() {
        let mut zapped = 0;
        for seed in 1..=4 {
            let mut sim = Sim::new(seed, Rules::default());
            let mut bots = crate::bots::Bots::default();
            bots.fill(&mut sim, 8, 0.6);
            for _ in 0..(120.0 / TICK) as usize {
                bots.think(&mut sim);
                sim.step();
                for e in sim.drain_events() {
                    if let SimEvent::TaggedOut {
                        player, by: HAZARD, ..
                    } = e
                        && let Some(p) = sim.player(player)
                        && sim.level.spawn_at(p.feet) == Some(p.team.other())
                    {
                        println!("seed {seed} tick {}: {} at {}", sim.tick, p.name, p.feet);
                        zapped += 1;
                    }
                }
            }
        }
        println!("zapped on enemy spawns: {zapped}");
        assert_eq!(zapped, 0, "bots walked onto the enemy's spawn pads");
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
        for pool in Pool::ALL {
            for half in [Team::Red, Team::Blue] {
                let n = sim
                    .pickups()
                    .filter(|p| p.kind.pool() == Some(pool) && Team::of_z(p.pos.z) == half)
                    .count();
                assert_eq!(n, pool.slots(), "{pool:?} in {half:?}");
            }
        }
        // Each pool's slots in a half roll different kinds (while there are enough).
        for half in [Team::Red, Team::Blue] {
            for pool in Pool::ALL {
                let mut kinds: Vec<PickupKind> = sim
                    .pickups()
                    .filter(|p| p.kind.pool() == Some(pool) && Team::of_z(p.pos.z) == half)
                    .map(|p| p.kind)
                    .collect();
                kinds.sort_by_key(|k| k.id());
                kinds.dedup();
                let want = pool.slots().min(pool.kinds().count());
                assert_eq!(kinds.len(), want, "{half:?} {pool:?}: {kinds:?}");
            }
        }
        let pos = |z: f32| Vec3::new(0.0, PICKUP_HEIGHT, z);
        sim.drop_pickup(PickupKind::Rapid, pos(3.0));
        sim.drop_pickup(PickupKind::Rapid, pos(-3.0));
        let rapid: Vec<Pickup> = sim
            .pickups()
            .filter(|p| p.kind == PickupKind::Rapid && p.pos.x == 0.0)
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
    fn a_hit_while_recovering_tells_the_shooter() {
        let (mut sim, a, b) = duel();
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 1);
        sim.drain_events();
        shoot(&mut sim, a, b);
        assert_eq!(sim.player(b).unwrap().hits, 1, "still recovering");
        assert!(
            sim.drain_events().iter().any(
                |e| matches!(e, SimEvent::Immune { player, by, .. } if *player == b && *by == a)
            )
        );
    }

    #[test]
    fn the_shotgun_sprays_short_lived_bolts_and_refills() {
        let (mut sim, a, _) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.players[0].ammo = 3;
        sim.give(a, PickupKind::Shotgun);
        let pa = sim.player(a).unwrap();
        assert!(
            pa.weapon == Weapon::Shotgun && pa.ammo == 50,
            "taking it fills it up"
        );
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
        assert_eq!(sim.player(a).unwrap().weapon, Weapon::Blaster);
    }

    #[test]
    fn a_cloak_shows_after_firing_and_hits_and_runs_out() {
        let (mut sim, a, b) = duel();
        sim.players[0].feet = Vec3::new(0.0, 0.0, 6.0);
        sim.players[1].feet = Vec3::new(0.0, 0.0, 100.0);
        sim.give(a, PickupKind::Cloak);
        let pa = sim.player(a).unwrap();
        assert!(pa.cloaked());
        assert!(!pa.seen_from(CLOAK_FAR, CLOAK_NEAR) && pa.seen_from(CLOAK_NEAR - 0.5, CLOAK_NEAR));
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
                secondary: crate::rules::SecondaryKind::Shoot,
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
        // Dormant until its console is solved: a hit does nothing.
        fire_at(&mut sim, a, m.core.centre());
        assert!(
            !sim.drain_events()
                .iter()
                .any(|e| matches!(e, SimEvent::MegaHit { .. }))
        );
        assert_eq!(sim.scores, [200, 0]);
        // Its console solved (from in front of it): energized for them.
        let k = sim
            .console_defs()
            .iter()
            .position(|d| d.kind == crate::console::ConsoleKind::Core)
            .unwrap();
        let f = sim.console_defs()[k].faces[0];
        let mut stand = f.centre + f.normal * 0.6;
        stand.y = 0.0;
        sim.players[i].feet = stand;
        sim.dev_console_poke(a, k, None, true);
        assert!(sim.player(a).unwrap().core_live);
        sim.drain_events();
        fire_at(&mut sim, a, m.core.centre());
        let ev = sim.drain_events();
        assert!(
            ev.iter()
                .any(|e| matches!(e, SimEvent::MegaHit { by, .. } if *by == a))
        );
        assert!(!sim.player(a).unwrap().core_live, "spent");
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

//! Types shared by the simulation, the client, the bots and the network protocol.

use glam::{Quat, Vec2, Vec3};
use serde::{Deserialize, Serialize};

pub type PlayerId = u8;

/// The shooter of turret bolts and the tagger of players a forcefield or the megatarget room's
/// timer got (never a player's id: there are at most `sim::MAX_PLAYERS`).
pub const HAZARD: PlayerId = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Team {
    Red,
    Blue,
}

impl Team {
    pub const ALL: [Team; 2] = [Team::Red, Team::Blue];

    pub fn index(self) -> usize {
        match self {
            Team::Red => 0,
            Team::Blue => 1,
        }
    }

    pub fn other(self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }

    /// Red owns the +z half of the arena, Blue the -z half.
    pub fn side(self) -> f32 {
        match self {
            Team::Red => 1.0,
            Team::Blue => -1.0,
        }
    }

    /// The team whose half of the arena contains `z`.
    pub fn of_z(z: f32) -> Team {
        if z >= 0.0 { Team::Red } else { Team::Blue }
    }
}

/// What lies in the arena to be picked up: a grenade or a powerup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PickupKind {
    Grenade,
    /// Twice the blaster's rate of fire, until tagged out.
    Rapid,
    /// Carried until used (`PlayerCmd::shield`): no hits taken for `Rules::shield_time`.
    Shield,
    /// A shotgun instead of the blaster until tagged out (`Rules::shotgun_bolts` short-lived
    /// bolts a shot, in a spread); taking one also fills it up.
    Shotgun,
    /// Unseen from afar and see-through up close for `Rules::cloak_time` or until tagged out
    /// (showing for a moment after firing or a hit).
    Cloak,
    /// `Rules::speed_factor` times as fast on one's feet for `Rules::speed_time` or until
    /// tagged out.
    Speed,
    /// The charge gun instead of the blaster until tagged out: a tap fires a bolt, holding the
    /// trigger for `Rules::charge_time` and letting go a beam that flies through players and
    /// deals `Rules::beam_damage` hits (for `Rules::beam_cost` shots); taking one also fills it
    /// up.
    Charge,
    /// The ricochet rifle instead of the blaster until tagged out: slower bolts that bounce
    /// off every wall up to `Rules::ricochet_bounces` times; taking one also fills it up.
    Ricochet,
    /// The prism lance instead of the blaster until tagged out: a slow-firing marksman rifle
    /// whose shot reaches `Rules::lance_range` at once and deals `Rules::lance_head_damage`
    /// hits to the head (for `Rules::lance_cost` shots); taking one also fills it up.
    Lance,
    /// Dual pistols instead of the blaster until tagged out: one in each hand, each firing at
    /// `Rules::dual_cooldown`, sharing the shots; taking them also fills them up.
    Dual,
    /// A gadget: a mirror put up like a barrier (`PlayerCmd::gadget`) for `Rules::mirror_time`,
    /// turning back every bolt that meets it, anyone's.
    Mirror,
    /// A gadget: thrown (`PlayerCmd::gadget`), it sticks where it lands and for
    /// `Rules::sensor_time` shows the enemies within `Rules::sensor_range` to its team every
    /// `Rules::sensor_every` seconds, through walls and cloaks.
    Sensor,
    /// A gadget: a copy of its player (`PlayerCmd::gadget`) that runs on ahead for
    /// `Rules::decoy_time`, drawing fire (a bolt pops it).
    Decoy,
    /// A gadget: held (`PlayerCmd::gadget`) on a team mate in front within
    /// `Rules::medbeam_range`, a beam that heals a hit every `Rules::medbeam_tick` seconds and
    /// refills their blaster, for `Rules::medbeam_time` seconds of beam in all.
    Medbeam,
    /// A gadget, carried until put up (`PlayerCmd::gadget`): a wall of hard light in front of the
    /// player for `Rules::barrier_time`, stopping every bolt but its team's.
    Barrier,
}

impl PickupKind {
    pub const ALL: [PickupKind; 15] = [
        PickupKind::Grenade,
        PickupKind::Rapid,
        PickupKind::Shield,
        PickupKind::Shotgun,
        PickupKind::Cloak,
        PickupKind::Speed,
        PickupKind::Charge,
        PickupKind::Barrier,
        PickupKind::Ricochet,
        PickupKind::Lance,
        PickupKind::Mirror,
        PickupKind::Sensor,
        PickupKind::Decoy,
        PickupKind::Medbeam,
        PickupKind::Dual,
    ];
    /// The powerups (everything but the grenade).
    pub const POWERUPS: [PickupKind; 14] = [
        PickupKind::Rapid,
        PickupKind::Shield,
        PickupKind::Shotgun,
        PickupKind::Cloak,
        PickupKind::Speed,
        PickupKind::Charge,
        PickupKind::Barrier,
        PickupKind::Ricochet,
        PickupKind::Lance,
        PickupKind::Mirror,
        PickupKind::Sensor,
        PickupKind::Decoy,
        PickupKind::Medbeam,
        PickupKind::Dual,
    ];

    pub fn name(self) -> &'static str {
        match self {
            PickupKind::Grenade => "grenade",
            PickupKind::Rapid => "rapid fire",
            PickupKind::Shield => "shield",
            PickupKind::Shotgun => "shotgun",
            PickupKind::Cloak => "cloak",
            PickupKind::Speed => "speed boost",
            PickupKind::Charge => "charge gun",
            PickupKind::Barrier => "barrier",
            PickupKind::Ricochet => "ricochet rifle",
            PickupKind::Lance => "prism lance",
            PickupKind::Mirror => "mirror",
            PickupKind::Sensor => "pulse sensor",
            PickupKind::Decoy => "decoy",
            PickupKind::Medbeam => "med beam",
            PickupKind::Dual => "dual pistols",
        }
    }

    /// A short name for scripts (`LT_DEV`).
    pub fn id(self) -> &'static str {
        match self {
            PickupKind::Grenade => "grenade",
            PickupKind::Rapid => "rapid",
            PickupKind::Shield => "shield",
            PickupKind::Shotgun => "shotgun",
            PickupKind::Cloak => "cloak",
            PickupKind::Speed => "speed",
            PickupKind::Charge => "charge",
            PickupKind::Barrier => "barrier",
            PickupKind::Ricochet => "ricochet",
            PickupKind::Lance => "lance",
            PickupKind::Mirror => "mirror",
            PickupKind::Sensor => "sensor",
            PickupKind::Decoy => "decoy",
            PickupKind::Medbeam => "medbeam",
            PickupKind::Dual => "dual",
        }
    }

    pub fn parse(s: &str) -> Option<PickupKind> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// The pool a powerup slot rolls it from (none: the grenade has slots of its own).
    pub fn pool(self) -> Option<Pool> {
        match self {
            PickupKind::Grenade => None,
            PickupKind::Shotgun
            | PickupKind::Charge
            | PickupKind::Ricochet
            | PickupKind::Lance
            | PickupKind::Dual => Some(Pool::Weapon),
            PickupKind::Barrier
            | PickupKind::Mirror
            | PickupKind::Sensor
            | PickupKind::Decoy
            | PickupKind::Medbeam => Some(Pool::Gadget),
            PickupKind::Rapid | PickupKind::Shield | PickupKind::Cloak | PickupKind::Speed => {
                Some(Pool::Power)
            }
        }
    }

    /// The gun it gives.
    pub fn weapon(self) -> Option<Weapon> {
        match self {
            PickupKind::Shotgun => Some(Weapon::Shotgun),
            PickupKind::Charge => Some(Weapon::Charge),
            PickupKind::Ricochet => Some(Weapon::Ricochet),
            PickupKind::Lance => Some(Weapon::Lance),
            PickupKind::Dual => Some(Weapon::Dual),
            _ => None,
        }
    }

    /// The gadget it gives.
    pub fn gadget(self) -> Option<Gadget> {
        match self {
            PickupKind::Barrier => Some(Gadget::Barrier),
            PickupKind::Mirror => Some(Gadget::Mirror),
            PickupKind::Sensor => Some(Gadget::Sensor),
            PickupKind::Decoy => Some(Gadget::Decoy),
            PickupKind::Medbeam => Some(Gadget::Medbeam),
            _ => None,
        }
    }
}

/// Which gun a player holds: the blaster, or the special one a pickup gave until tagged out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weapon {
    #[default]
    Blaster,
    Shotgun,
    Charge,
    /// Bolts bounce off every wall (`Rules::ricochet_bounces`).
    Ricochet,
    /// Hitscan: the shot crosses `Rules::lance_range` at once, a head hit counts double.
    Lance,
    /// A pistol in each hand (`PlayerCmd::gun2_hand`), sharing the shots.
    Dual,
}

/// A carried gadget (one at a time, used with `PlayerCmd::gadget`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Gadget {
    Barrier,
    Mirror,
    Sensor,
    Decoy,
    Medbeam,
}

impl Gadget {
    pub fn name(self) -> &'static str {
        match self {
            Gadget::Barrier => "barrier",
            Gadget::Mirror => "mirror",
            Gadget::Sensor => "pulse sensor",
            Gadget::Decoy => "decoy",
            Gadget::Medbeam => "med beam",
        }
    }
}

/// The kinds of pickup a powerup slot rolls from each time it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pool {
    Weapon,
    Gadget,
    Power,
}

impl Pool {
    pub const ALL: [Pool; 3] = [Pool::Weapon, Pool::Gadget, Pool::Power];

    /// Slots of this pool in each half per `Rules::powerup_pairs`.
    pub fn slots(self) -> usize {
        match self {
            Pool::Weapon => 2,
            Pool::Gadget => 2,
            Pool::Power => 3,
        }
    }

    pub fn kinds(self) -> impl Iterator<Item = PickupKind> {
        PickupKind::POWERUPS
            .into_iter()
            .filter(move |k| k.pool() == Some(self))
    }
}

/// Where a match is: warming up (seconds left; nothing counts and the clock stands, see
/// `Sim::end_warmup`), counting down to the start (seconds left; nobody can fire), or under
/// way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum MatchPhase {
    Warmup(f32),
    Countdown(f32),
    #[default]
    Live,
}

/// What tagged a player out (`SimEvent::TaggedOut`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagHow {
    #[default]
    Blaster,
    Shotgun,
    Beam,
    Grenade,
    Turret,
    /// A laser gate.
    Gate,
    /// A Megatarget forcefield, or the room's time running out.
    Field,
    /// The enemy team's spawn pads.
    Pad,
}

/// Something that happens to the whole arena for a while during a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArenaEvent {
    /// The arena's lights go dark for `Rules::blackout_time` (the bolts, players, pickups and
    /// targets still glow; the bots see less far).
    Blackout,
    /// Every pickup missing from the arena comes back at once.
    Overdrive,
    /// For `Rules::storm_time` every wall turns a bolt back like a mirror (once).
    MirrorStorm,
    /// For `Rules::lowgrav_time` everything falls at `Rules::lowgrav_factor` of the gravity
    /// (jump pads' flights aside).
    LowGravity,
    /// For `Rules::bounty_time` the top scorer (`Sim::bounty`) is marked for everyone, and
    /// tagging them out is worth `Rules::bounty_points`; lasting it out pays them as much.
    Bounty,
}

impl ArenaEvent {
    pub const ALL: [ArenaEvent; 5] = [
        ArenaEvent::Blackout,
        ArenaEvent::Overdrive,
        ArenaEvent::MirrorStorm,
        ArenaEvent::LowGravity,
        ArenaEvent::Bounty,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ArenaEvent::Blackout => "BLACKOUT",
            ArenaEvent::Overdrive => "OVERDRIVE",
            ArenaEvent::MirrorStorm => "MIRROR STORM",
            ArenaEvent::LowGravity => "LOW GRAVITY",
            ArenaEvent::Bounty => "BOUNTY",
        }
    }

    /// A short name for scripts (`LT_DEV`).
    pub fn id(self) -> &'static str {
        match self {
            ArenaEvent::Blackout => "blackout",
            ArenaEvent::Overdrive => "overdrive",
            ArenaEvent::MirrorStorm => "storm",
            ArenaEvent::LowGravity => "lowgrav",
            ArenaEvent::Bounty => "bounty",
        }
    }

    pub fn parse(s: &str) -> Option<ArenaEvent> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    pub fn index(self) -> usize {
        match self {
            Hand::Left => 0,
            Hand::Right => 1,
        }
    }
}

/// A position and orientation. Forward is -Z (Bevy's convention).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub pos: Vec3,
    pub rot: Quat,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
        }
    }
}

impl Pose {
    pub fn forward(&self) -> Vec3 {
        self.rot * Vec3::NEG_Z
    }
}

/// One tick of a player's intent: the same for the local player (desktop or VR), a remote
/// player (from the network) and a bot. Positions are relative to the player's feet, in world
/// orientation (not rotated by `yaw`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerCmd {
    /// Locomotion stick in world XZ, length at most 1 (scaled by the move speed).
    pub stick: Vec2,
    /// Physical walking this tick in world XZ metres (VR room scale), clamped by the sim.
    pub walk: Vec2,
    /// Where the client's own prediction has the feet (instead of `stick` and `walk`): the sim
    /// moves there as far as its speed limit and the walls allow.
    pub feet: Option<Vec3>,
    /// Body yaw (radians, 0 = facing -Z).
    pub yaw: f32,
    /// Head (eyes) relative to the feet; its height is the hit capsule's top (VR crouching).
    pub head: Pose,
    /// Both hands, for drawing remote players.
    pub hands: [Pose; 2],
    /// The hand holding the blaster, if any.
    pub gun_hand: Option<Hand>,
    /// The blaster's muzzle relative to the feet; bolts leave from here along its forward.
    pub muzzle: Pose,
    pub fire: bool,
    /// Dual pistols: the hand holding the second, its muzzle (as `muzzle`) and its trigger.
    #[serde(default)]
    pub gun2_hand: Option<Hand>,
    #[serde(default)]
    pub muzzle2: Pose,
    #[serde(default)]
    pub fire2: bool,
    /// A grenade is in this hand (drawn, not yet thrown).
    pub grenade_hand: Option<Hand>,
    /// The player is in VR (its throws get `Rules::vr_throw_factor`).
    #[serde(default)]
    pub vr: bool,
    /// Throw this tick: start position (relative to the feet) and velocity.
    pub throw: Option<(Vec3, Vec3)>,
    /// Switch on the carried shield (powerup).
    pub shield: bool,
    /// Jump presses so far (a count, so a lost command loses none; `sim::jump_step`).
    #[serde(default)]
    pub jumps: u32,
    /// Dash presses so far (a count, as `jumps`; `sim::dash_step`).
    #[serde(default)]
    pub dashes: u32,
    /// Crouching (moves at `Rules::crouch_factor` of the speed).
    #[serde(default)]
    pub crouch: bool,
    /// Use the carried gadget (on pressing; some work while held).
    #[serde(default)]
    pub gadget: bool,
    /// The sim tick the client was showing (the snapshot tick it rendered, after its
    /// interpolation delay) when it made this command; 0 = unknown or local (no lag
    /// compensation). The server flies a bolt fired with it through the other players as they
    /// were at the ticks the shooter saw (up to `sim::MAX_LAG` ticks back).
    pub view_tick: u64,
    /// The shots a remote client has fired so far by its own reckoning (`shot::Gun`), counting
    /// the one fired with this command: its hit claims name shots by this.
    #[serde(default)]
    pub shot: u32,
}

/// What happened during a tick: drives sounds, haptics, effects and the score display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SimEvent {
    Fired {
        player: PlayerId,
        pos: Vec3,
        dir: Vec3,
    },
    /// A charge gun has been held long enough: letting go fires a beam.
    ChargeFull {
        player: PlayerId,
    },
    /// A charge gun's beam (as well as its `Fired`).
    BeamFired {
        player: PlayerId,
        pos: Vec3,
        dir: Vec3,
    },
    /// The trigger was pulled while the blaster is down (after a hit).
    Denied {
        player: PlayerId,
    },
    /// The trigger was pulled with no shots left.
    Empty {
        player: PlayerId,
    },
    /// A blaster started refilling (at a reload station or a spawn pad), and is full again.
    Reloading {
        player: PlayerId,
    },
    Reloaded {
        player: PlayerId,
    },
    /// A player switched its shield on, and it ran out.
    ShieldUp {
        player: PlayerId,
    },
    ShieldDown {
        player: PlayerId,
    },
    /// A player's cloak ran out.
    Uncloaked {
        player: PlayerId,
    },
    /// A barrier went up (`Sim::barriers`), stopped a bolt, and went down.
    BarrierUp {
        id: u32,
        player: PlayerId,
        pos: Vec3,
    },
    BarrierHit {
        id: u32,
        pos: Vec3,
    },
    BarrierDown {
        id: u32,
        pos: Vec3,
    },
    /// A pulse sensor thrown (`Sim::sensors`), stuck where it landed, pulsing (the enemies
    /// within its range, by player id bit), and gone.
    SensorThrown {
        id: u32,
        player: PlayerId,
        pos: Vec3,
    },
    SensorStuck {
        id: u32,
        pos: Vec3,
    },
    Pulse {
        id: u32,
        team: Team,
        pos: Vec3,
        seen: u64,
    },
    SensorDown {
        id: u32,
        pos: Vec3,
    },
    /// A decoy sent off (`Sim::decoys`), popped by a bolt (`by` its shooter), and gone with
    /// its time up.
    DecoyOut {
        id: u32,
        player: PlayerId,
        pos: Vec3,
    },
    DecoyPopped {
        id: u32,
        by: PlayerId,
        pos: Vec3,
    },
    DecoyGone {
        id: u32,
        pos: Vec3,
    },
    /// A player's speed boost ran out.
    SpeedDown {
        player: PlayerId,
    },
    /// A bolt hit a shielded player (and did nothing).
    Blocked {
        player: PlayerId,
        pos: Vec3,
    },
    /// A bounty on `player` (`Sim::bounty`), claimed by `by` tagging them out, or lasted out.
    BountyOn {
        player: PlayerId,
    },
    BountyClaimed {
        player: PlayerId,
        by: PlayerId,
    },
    BountySurvived {
        player: PlayerId,
    },
    /// A team mate's med beam took a hit off `player` (`hits` left).
    Mended {
        player: PlayerId,
        by: PlayerId,
        hits: u8,
    },
    /// A hit player stepped onto its team's spawn pads and is unhurt again.
    Healed {
        player: PlayerId,
    },
    /// A pickup (a grenade or a powerup) appeared, and was taken.
    PickupSpawned {
        id: u32,
        pos: Vec3,
        kind: PickupKind,
    },
    PickedUp {
        id: u32,
        player: PlayerId,
        pos: Vec3,
        kind: PickupKind,
    },
    /// A prism lance's shot: its way from `from` to `to` (one leg of it, a mirror turning it).
    Streak {
        player: PlayerId,
        from: Vec3,
        to: Vec3,
    },
    /// A hit to the head that counted extra (before its `Hit`).
    Headshot {
        player: PlayerId,
        by: PlayerId,
        pos: Vec3,
    },
    /// A bolt hit a player (`hits` taken so far, 1..=hits_to_tag); `by` is the shooter.
    Hit {
        player: PlayerId,
        by: PlayerId,
        pos: Vec3,
        hits: u8,
    },
    TaggedOut {
        player: PlayerId,
        by: PlayerId,
        pos: Vec3,
        respawn_in: f32,
        #[serde(default)]
        how: TagHow,
    },
    Respawned {
        player: PlayerId,
        pos: Vec3,
    },
    /// The countdown to a match's start: `n` seconds to go.
    Countdown {
        n: u8,
    },
    /// The match starts (the countdown is over).
    Go,
    /// A player jumped (`PlayerCmd::jumps`).
    Jumped {
        player: PlayerId,
        pos: Vec3,
    },
    /// A player dashed (`PlayerCmd::dashes`).
    Dashed {
        player: PlayerId,
        pos: Vec3,
    },
    /// A jump pad threw a player up.
    Launched {
        player: PlayerId,
        pos: Vec3,
    },
    /// A player helped tag `victim` out (`Rules::assist_points`).
    Assist {
        player: PlayerId,
        victim: PlayerId,
    },
    /// A player tagged `victim` out near their own targets (`Rules::defend_points`).
    Defended {
        player: PlayerId,
        victim: PlayerId,
    },
    TargetHit {
        target: u8,
        by: PlayerId,
        pos: Vec3,
    },
    /// A bolt bounced off a mirror.
    Reflected {
        pos: Vec3,
        normal: Vec3,
        team: Option<Team>,
    },
    /// A bolt hit a wall (spark).
    BoltImpact {
        pos: Vec3,
        normal: Vec3,
        team: Option<Team>,
    },
    GrenadeThrown {
        player: PlayerId,
        pos: Vec3,
    },
    GrenadeBounce {
        pos: Vec3,
        speed: f32,
    },
    GrenadeArmed {
        pos: Vec3,
    },
    GrenadeBurst {
        pos: Vec3,
    },
    GrenadeDone {
        pos: Vec3,
    },
    MatchOver {
        scores: [u32; 2],
    },
    /// An arena event began, and ended.
    EventStart {
        kind: ArenaEvent,
    },
    EventEnd {
        kind: ArenaEvent,
    },
    /// Double points from now on (the last `Rules::double_last` seconds).
    DoublePoints,
    /// Megatarget mode: a secondary target hit that counts (`side` 0 at -x, 1 at +x);
    /// `charged`: it was the shooter's second, the megatarget's room is open to them now.
    SecondaryHit {
        side: u8,
        by: PlayerId,
        pos: Vec3,
        charged: bool,
    },
    /// The megatarget hit; the shooter is sent back to its base straight after.
    MegaHit {
        by: PlayerId,
        pos: Vec3,
    },
    /// A player's time to get into the room ran out (outside it: they start over).
    PowerLost {
        player: PlayerId,
    },
    TurretFired {
        turret: u8,
        pos: Vec3,
        dir: Vec3,
    },
    /// A turret's deactivation target hit: it is off for `Rules::turret_off` seconds.
    TurretOff {
        turret: u8,
        by: PlayerId,
        pos: Vec3,
    },
    TurretOn {
        turret: u8,
    },
    /// A turret found a player to shoot at (its first shot comes `TURRET_WARMUP` later).
    TurretEngaged {
        turret: u8,
    },
    /// A player walked into a laser gate while it was on (and takes a hit).
    GateZap {
        gate: u8,
        player: PlayerId,
        pos: Vec3,
    },
    /// A player walked into a forcefield (and is tagged out by it).
    Shocked {
        player: PlayerId,
        pos: Vec3,
    },
}

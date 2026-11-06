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
}

impl PickupKind {
    pub const ALL: [PickupKind; 3] = [PickupKind::Grenade, PickupKind::Rapid, PickupKind::Shield];
    /// The powerups (everything but the grenade).
    pub const POWERUPS: [PickupKind; 2] = [PickupKind::Rapid, PickupKind::Shield];

    pub fn name(self) -> &'static str {
        match self {
            PickupKind::Grenade => "grenade",
            PickupKind::Rapid => "rapid fire",
            PickupKind::Shield => "shield",
        }
    }

    /// A short name for scripts (`LT_DEV`).
    pub fn id(self) -> &'static str {
        match self {
            PickupKind::Grenade => "grenade",
            PickupKind::Rapid => "rapid",
            PickupKind::Shield => "shield",
        }
    }

    pub fn parse(s: &str) -> Option<PickupKind> {
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
    /// A grenade is in this hand (drawn, not yet thrown).
    pub grenade_hand: Option<Hand>,
    /// Throw this tick: start position (relative to the feet) and velocity.
    pub throw: Option<(Vec3, Vec3)>,
    /// Switch on the carried shield (powerup).
    pub shield: bool,
    /// The sim tick the client was showing (the snapshot tick it rendered, after its
    /// interpolation delay) when it made this command; 0 = unknown or local (no lag
    /// compensation). The server flies a bolt fired with it through the other players as they
    /// were at the ticks the shooter saw (up to `sim::MAX_LAG` ticks back).
    pub view_tick: u64,
}

/// What happened during a tick: drives sounds, haptics, effects and the score display.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SimEvent {
    Fired {
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
    /// A bolt hit a shielded player (and did nothing).
    Blocked {
        player: PlayerId,
        pos: Vec3,
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
    },
    Respawned {
        player: PlayerId,
        pos: Vec3,
    },
    TargetHit {
        target: u8,
        by: PlayerId,
        pos: Vec3,
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
    /// A player walked into a forcefield (and is tagged out by it).
    Shocked {
        player: PlayerId,
        pos: Vec3,
    },
}

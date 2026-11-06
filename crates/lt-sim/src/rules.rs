//! Tunable game rules (sent to clients in the welcome message, loadable from RON).

use serde::{Deserialize, Serialize};

use crate::level::{ArenaSize, GameMode};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Hits that tag a player out.
    pub hits_to_tag: u8,
    /// After each hit: no hits taken and no firing for this long (moving is allowed).
    pub hit_cooldown: f32,
    /// Respawn wait: `respawn_min` when tagged at one's own base, up to `respawn_max` far away.
    pub respawn_min: f32,
    pub respawn_max: f32,
    pub tag_points: u32,
    pub target_points: u32,
    /// Least time between shots (three a second).
    pub fire_cooldown: f32,
    /// Shots in a full blaster.
    pub ammo: u16,
    /// Shots a second refilled at a reload station or a spawn pad.
    pub reload_rate: f32,
    /// Grenade and turret bolts' speed.
    pub bolt_speed: f32,
    /// Blaster bolts' speed (players' and bots').
    pub blaster_bolt_speed: f32,
    pub bolt_life: f32,
    /// Times a bolt bounces off mirrors (the next one stops it).
    pub mirror_bounces: u8,
    pub move_speed: f32,
    /// Most physical (room-scale) walking accepted per second.
    pub walk_speed_max: f32,
    pub grenade_max: u8,
    /// Grenade pickups lying in the arena at once (in mirrored pairs: this many pairs).
    pub pickup_pairs: u8,
    /// A taken pickup's pair comes back after this long (random in the range).
    pub pickup_respawn: (f32, f32),
    /// Powerup pickups lying in the arena at once, per kind (in mirrored pairs), and how long a
    /// taken one's pair takes to come back (random in the range).
    pub powerup_pairs: u8,
    pub powerup_respawn: (f32, f32),
    /// Seconds a shield lasts.
    pub shield_time: f32,
    /// The shotgun: bolts a shot, their spread (the cone's half angle, radians) and how far
    /// they fly (m).
    pub shotgun_bolts: u8,
    pub shotgun_spread: f32,
    pub shotgun_range: f32,
    /// Seconds a cloak lasts, and how long one shows after firing or a hit.
    pub cloak_time: f32,
    pub cloak_flash: f32,
    /// The charge gun: seconds to hold the trigger for a beam, the shots a beam takes, its
    /// speed (m/s), reach (m) and hits dealt.
    pub charge_time: f32,
    pub beam_cost: u16,
    pub beam_speed: f32,
    pub beam_range: f32,
    pub beam_damage: u8,
    /// Seconds a speed boost lasts, and how many times as fast it makes one.
    pub speed_time: f32,
    pub speed_factor: f32,
    pub grenade_fuse: f32,
    pub grenade_active: f32,
    pub grenade_hop: f32,
    pub grenade_burst_every: f32,
    pub grenade_burst_bolts: (u8, u8),
    /// The share of a grenade's bolts aimed at a player it can see (the rest fly anywhere).
    pub grenade_aimed: f32,
    pub grenade_restitution: f32,
    pub grenade_max_speed: f32,
    /// Match length (seconds).
    pub match_time: f32,
    /// The arena's size (with the seed and the mode, all a client needs to build the same
    /// arena).
    pub arena: ArenaSize,
    pub mode: GameMode,
    /// Megatarget mode: points for a secondary target hit that counts (the shooter's first on
    /// it since starting over) and for the megatarget.
    pub secondary_points: u32,
    pub megatarget_points: u32,
    /// Megatarget mode: seconds to get into the megatarget's room after hitting both secondary
    /// targets.
    pub power_time: f32,
    /// Turrets: seconds off after their deactivation target is hit, seconds between shots,
    /// reach (m), aiming error (radians, standard deviation) and turning speed (radians a
    /// second).
    pub turret_off: f32,
    pub turret_every: f32,
    pub turret_range: f32,
    pub turret_spread: f32,
    pub turret_turn: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            hits_to_tag: 3,
            hit_cooldown: 3.0,
            respawn_min: 5.0,
            respawn_max: 15.0,
            tag_points: 100,
            target_points: 100,
            fire_cooldown: 1.0 / 3.0,
            ammo: 50,
            reload_rate: 25.0,
            bolt_speed: 48.0,
            blaster_bolt_speed: 96.0,
            bolt_life: 2.0,
            mirror_bounces: 1,
            move_speed: 4.0,
            walk_speed_max: 4.0,
            grenade_max: 1,
            pickup_pairs: 2,
            pickup_respawn: (8.0, 16.0),
            powerup_pairs: 1,
            powerup_respawn: (25.0, 45.0),
            shield_time: 10.0,
            shotgun_bolts: 10,
            shotgun_spread: 0.07,
            shotgun_range: 14.0,
            cloak_time: 100.0,
            cloak_flash: 0.5,
            charge_time: 1.0,
            beam_cost: 5,
            beam_speed: 400.0,
            beam_range: 60.0,
            beam_damage: 2,
            speed_time: 15.0,
            speed_factor: 1.5,
            grenade_fuse: 3.0,
            grenade_active: 4.0,
            grenade_hop: 1.0,
            grenade_burst_every: 0.12,
            grenade_burst_bolts: (2, 4),
            grenade_aimed: 0.2,
            grenade_restitution: 0.45,
            grenade_max_speed: 48.0,
            match_time: 600.0,
            arena: ArenaSize::Small,
            mode: GameMode::Classic,
            secondary_points: 100,
            megatarget_points: 1000,
            power_time: 60.0,
            turret_off: 15.0,
            turret_every: 0.6,
            turret_range: 16.0,
            turret_spread: 0.035,
            turret_turn: 2.2,
        }
    }
}

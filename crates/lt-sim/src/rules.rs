//! Tunable game rules (sent to clients in the welcome message, loadable from RON).

use serde::{Deserialize, Serialize};

use crate::level::ArenaSize;

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
    pub bolt_speed: f32,
    pub bolt_life: f32,
    pub move_speed: f32,
    /// Most physical (room-scale) walking accepted per second.
    pub walk_speed_max: f32,
    pub grenade_max: u8,
    /// Grenade pickups lying in the arena at once (in mirrored pairs: this many pairs).
    pub pickup_pairs: u8,
    /// A taken pickup's pair comes back after this long (random in the range).
    pub pickup_respawn: (f32, f32),
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
    /// The arena's size (with the seed, all a client needs to build the same arena).
    pub arena: ArenaSize,
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
            bolt_life: 2.0,
            move_speed: 4.0,
            walk_speed_max: 4.0,
            grenade_max: 1,
            pickup_pairs: 2,
            pickup_respawn: (8.0, 16.0),
            grenade_fuse: 3.0,
            grenade_active: 4.0,
            grenade_hop: 1.0,
            grenade_burst_every: 0.12,
            grenade_burst_bolts: (2, 4),
            grenade_aimed: 0.2,
            grenade_restitution: 0.45,
            grenade_max_speed: 16.0,
            match_time: 600.0,
            arena: ArenaSize::Small,
        }
    }
}

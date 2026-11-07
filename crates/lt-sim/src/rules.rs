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
    /// After respawning: no hits taken for this long, or until firing, throwing or putting up a
    /// shield or barrier.
    pub spawn_protect: f32,
    /// Respawn wait: `respawn_min` when tagged at one's own base, up to `respawn_max` far away.
    pub respawn_min: f32,
    pub respawn_max: f32,
    pub tag_points: u32,
    pub target_points: u32,
    /// Points for an assist (hitting an enemy whom a team mate tags out within `assist_window`
    /// seconds) and for tagging out an enemy within `defend_range` metres of one's own targets.
    pub assist_points: u32,
    pub assist_window: f32,
    pub defend_points: u32,
    pub defend_range: f32,
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
    /// Times a ricochet rifle's bolt bounces off any wall, and its least time between shots.
    pub ricochet_bounces: u8,
    pub ricochet_cooldown: f32,
    /// The prism lance: how far its shot reaches (m), its least time between shots, the shots
    /// one costs, and the hits a head hit deals.
    pub lance_range: f32,
    pub lance_cooldown: f32,
    pub lance_cost: u16,
    pub lance_head_damage: u8,
    /// Each of the dual pistols' least time between shots.
    pub dual_cooldown: f32,
    pub move_speed: f32,
    /// Jumping: the upward speed (m/s), how long after walking off an edge one still may (s),
    /// and how long before landing a press still counts (s).
    pub jump_speed: f32,
    pub coyote_time: f32,
    pub jump_buffer: f32,
    /// Dashing: its speed (m/s), how long it lasts (s), and the least time between dashes (s).
    pub dash_speed: f32,
    pub dash_time: f32,
    pub dash_cooldown: f32,
    /// Seconds of the countdown before a match (`Sim::begin`; nobody can fire meanwhile), and
    /// of the warmup before it (0: none; free play that doesn't count, the clock standing).
    pub countdown: f32,
    pub warmup: f32,
    /// Crouching players' share of `move_speed`.
    pub crouch_factor: f32,
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
    /// Barriers: seconds one stands, how far in front of the player it goes up (m), and its
    /// width and height (m).
    pub barrier_time: f32,
    /// Seconds a mirror put up stands (its size is a barrier's).
    pub mirror_time: f32,
    /// Pulse sensors: seconds one pulses once stuck, between pulses, how far one sees (m), how
    /// long a pulse shows the enemies it saw (s), and how fast one is thrown (m/s).
    pub sensor_time: f32,
    pub sensor_every: f32,
    pub sensor_range: f32,
    pub sensor_reveal: f32,
    pub sensor_speed: f32,
    /// Seconds a decoy runs.
    pub decoy_time: f32,
    /// The med beam: seconds of beam a pickup holds, its reach (m), the widest angle off the
    /// aim a team mate may be (radians), the seconds of beam a hit takes to heal, and the shots
    /// a second it refills.
    pub medbeam_time: f32,
    pub medbeam_range: f32,
    pub medbeam_cone: f32,
    pub medbeam_tick: f32,
    pub medbeam_ammo: f32,
    pub barrier_dist: f32,
    pub barrier_w: f32,
    pub barrier_h: f32,
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
    /// VR players throw this many times as hard as their hands move (and as fast as this many
    /// times `grenade_max_speed`): a real throw in a small room is gentle.
    pub vr_throw_factor: f32,
    /// Match length (seconds).
    pub match_time: f32,
    /// Arena events (`ArenaEvent`): whether there are any, when the first comes (seconds into
    /// the match, random in the range) and the time from one to the next, none starting in
    /// the last `event_quiet` seconds; a blackout's length; and the last seconds of a match
    /// in which every point counts double (0: never).
    pub events: bool,
    pub event_first: (f32, f32),
    pub event_every: (f32, f32),
    pub event_quiet: f32,
    pub blackout_time: f32,
    /// Seconds a mirror storm lasts.
    pub storm_time: f32,
    /// Seconds low gravity lasts, and how much of the gravity is left.
    pub lowgrav_time: f32,
    pub lowgrav_factor: f32,
    /// Seconds a bounty runs, and the points it's worth (for the tag-out, all in).
    pub bounty_time: f32,
    pub bounty_points: u32,
    /// Prism mode: points for carrying the enemy's prism home, seconds a dropped one lies
    /// before it goes home, how close a player has to come to take one (m), and to catch a
    /// thrown one.
    pub prism_points: u32,
    pub prism_return: f32,
    pub prism_reach: f32,
    pub prism_catch: f32,
    pub double_last: f32,
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
            spawn_protect: 1.5,
            respawn_min: 5.0,
            respawn_max: 15.0,
            tag_points: 100,
            target_points: 100,
            assist_points: 50,
            assist_window: 10.0,
            defend_points: 25,
            defend_range: 12.0,
            fire_cooldown: 1.0 / 3.0,
            ammo: 50,
            reload_rate: 25.0,
            bolt_speed: 48.0,
            blaster_bolt_speed: 96.0,
            bolt_life: 2.0,
            mirror_bounces: 1,
            ricochet_bounces: 3,
            ricochet_cooldown: 0.5,
            lance_range: 90.0,
            lance_cooldown: 1.2,
            lance_cost: 2,
            lance_head_damage: 2,
            dual_cooldown: 0.45,
            move_speed: 4.0,
            crouch_factor: 0.5,
            countdown: 3.0,
            warmup: 0.0,
            dash_speed: 12.0,
            dash_time: 0.25,
            dash_cooldown: 2.0,
            jump_speed: 3.9,
            coyote_time: 0.1,
            jump_buffer: 0.15,
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
            mirror_time: 15.0,
            sensor_time: 20.0,
            sensor_every: 2.0,
            sensor_range: 10.0,
            sensor_reveal: 2.5,
            sensor_speed: 14.0,
            decoy_time: 6.0,
            medbeam_time: 12.0,
            medbeam_range: 8.0,
            medbeam_cone: 0.35,
            medbeam_tick: 2.5,
            medbeam_ammo: 6.0,
            barrier_time: 8.0,
            barrier_dist: 1.5,
            barrier_w: 2.4,
            barrier_h: 2.0,
            speed_time: 15.0,
            speed_factor: 1.5,
            grenade_fuse: 3.0,
            grenade_active: 4.0,
            grenade_hop: 1.0,
            grenade_burst_every: 0.12,
            grenade_burst_bolts: (2, 4),
            grenade_aimed: 0.2,
            grenade_restitution: 0.45,
            grenade_max_speed: 16.0,
            vr_throw_factor: 3.0,
            match_time: 600.0,
            events: true,
            event_first: (120.0, 180.0),
            event_every: (120.0, 180.0),
            event_quiet: 75.0,
            storm_time: 20.0,
            lowgrav_time: 20.0,
            lowgrav_factor: 0.35,
            bounty_time: 30.0,
            bounty_points: 300,
            prism_points: 1000,
            prism_return: 20.0,
            prism_reach: 1.6,
            prism_catch: 2.0,
            blackout_time: 20.0,
            double_last: 60.0,
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

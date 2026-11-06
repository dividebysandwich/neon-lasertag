//! Sounds (assets/sounds, synthesized by tools/gen_sfx.py): game events in space around the
//! listener, footsteps from how far each player walks.
//!
//! Spatial sounds fade with the square of the distance (rodio), measured in units of
//! `SPATIAL_SCALE` metres' reciprocal: full volume up to 1 / SPATIAL_SCALE metres, a quarter at
//! twice that. (At the default 1 m a grenade bouncing 10 m away was 40 dB down.) Team target
//! hits are heard everywhere.
//!
//! Megatarget mode: a player who may enter the megatarget's room ticks like a clock where they
//! are (faster in the last ten seconds), and the megatarget hit plays a fanfare for everyone.

use std::collections::HashMap;

use bevy::audio::{DefaultSpatialScale, PlaybackMode, SpatialScale, Volume};
use bevy::prelude::*;
use lt_sim::{PlayerId, SimEvent};

use crate::game::{Game, GameEvent};
use crate::settings::Settings;

#[derive(Resource)]
struct Sounds {
    /// Blasters firing: Blue's and Red's sound different.
    fire: Handle<AudioSource>,
    fire_red: Handle<AudioSource>,
    deny: Handle<AudioSource>,
    /// The local player hit, and someone else hit.
    hit: Handle<AudioSource>,
    other_hit: Handle<AudioSource>,
    tagged: Handle<AudioSource>,
    target: Handle<AudioSource>,
    bounce: Handle<AudioSource>,
    throw: Handle<AudioSource>,
    arm: Handle<AudioSource>,
    burst: Handle<AudioSource>,
    respawn: Handle<AudioSource>,
    empty: Handle<AudioSource>,
    reload: Handle<AudioSource>,
    reload_done: Handle<AudioSource>,
    pickup: Handle<AudioSource>,
    /// The local player scored: hit an enemy or a target, tagged an enemy out.
    bleep: Handle<AudioSource>,
    bleep_tag: Handle<AudioSource>,
    steps: [Handle<AudioSource>; 3],
    mega_win: Handle<AudioSource>,
    tick: Handle<AudioSource>,
    power_up: Handle<AudioSource>,
    turret_fire: Handle<AudioSource>,
    turret_down: Handle<AudioSource>,
    turret_active: Handle<AudioSource>,
    zap: Handle<AudioSource>,
}

/// Seconds until each powered player's next tick.
#[derive(Resource, Default)]
struct Ticking(HashMap<PlayerId, f32>);

#[derive(Resource, Default)]
struct Walked {
    last: HashMap<PlayerId, (Vec3, f32)>,
    n: usize,
}

/// Spatial sounds play at full volume within 1 / SPATIAL_SCALE metres (see above).
const SPATIAL_SCALE: f32 = 0.12;

/// The listener for the camera or head (ears 20 cm apart). rodio's `Spatial` (through 0.22) pans
/// backwards: of the two ears it gives the one farther from the sound the louder channel (a
/// sound at the left ear plays at 0.5 left, 1.0 right). So the ears given to it are swapped: the
/// "left" one on the listener's right, which comes out of the left channel the right way round.
pub fn listener() -> SpatialListener {
    let ears = SpatialListener::new(0.2);
    SpatialListener {
        left_ear_offset: ears.right_ear_offset,
        right_ear_offset: ears.left_ear_offset,
    }
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Walked>()
            .init_resource::<Ticking>()
            .insert_resource(DefaultSpatialScale(SpatialScale::new(SPATIAL_SCALE)))
            .add_systems(Startup, load)
            .add_systems(
                Update,
                (play_events, footsteps, ticking).run_if(resource_exists::<Game>),
            );
    }
}

fn load(mut commands: Commands, assets: Res<AssetServer>) {
    let s = |n: &str| assets.load(format!("sounds/{n}.ogg"));
    commands.insert_resource(Sounds {
        fire: s("laser_fire"),
        fire_red: s("laser_fire_red"),
        deny: s("deny"),
        hit: s("hit_player"),
        other_hit: s("player_hit"),
        tagged: s("tagged_out"),
        target: s("hit_target"),
        bounce: s("grenade_bounce"),
        throw: s("grenade_throw"),
        arm: s("grenade_arm"),
        burst: s("grenade_burst"),
        respawn: s("respawn"),
        empty: s("empty"),
        reload: s("reload"),
        reload_done: s("reload_done"),
        pickup: s("pickup"),
        bleep: s("score_bleep"),
        bleep_tag: s("score_tag"),
        steps: [s("footstep_1"), s("footstep_2"), s("footstep_3")],
        mega_win: s("mega_win"),
        tick: s("power_tick"),
        power_up: s("power_up"),
        turret_fire: s("turret_fire"),
        turret_down: s("turret_down"),
        turret_active: s("turretactive"),
        zap: s("shield_zap"),
    });
}

/// Plays `sound` (`gain`: the settings' sound volume).
fn play(
    commands: &mut Commands,
    gain: f32,
    sound: &Handle<AudioSource>,
    at: Option<Vec3>,
    volume: f32,
    speed: f32,
) {
    let settings = PlaybackSettings {
        mode: PlaybackMode::Despawn,
        volume: Volume::Linear(volume * gain),
        speed,
        spatial: at.is_some(),
        ..default()
    };
    commands.spawn((
        AudioPlayer(sound.clone()),
        settings,
        Transform::from_translation(at.unwrap_or_default()),
    ));
}

fn play_events(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    sounds: Res<Sounds>,
    settings: Option<Res<Settings>>,
) {
    let gain = settings.map_or(1.0, |s| s.sound_volume);
    let me = game.me;
    let team = |id| {
        game.snap
            .players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.team)
    };
    let enemy = |id| me.is_some_and(|m| m != id && team(id).is_some() && team(id) != team(m));
    for GameEvent(e) in events.read() {
        // The reward for scoring (the tag-out's own, rather than the hit's, for the last hit).
        match *e {
            SimEvent::Hit {
                player, by, hits, ..
            } if Some(by) == me && enemy(player) && hits < game.rules.hits_to_tag => {
                play(&mut commands, gain, &sounds.bleep, None, 0.8, 1.0)
            }
            SimEvent::TaggedOut { player, by, .. } if Some(by) == me && enemy(player) => {
                play(&mut commands, gain, &sounds.bleep_tag, None, 0.8, 1.0)
            }
            SimEvent::TargetHit { by, .. }
            | SimEvent::SecondaryHit { by, .. }
            | SimEvent::TurretOff { by, .. }
                if Some(by) == me =>
            {
                play(&mut commands, gain, &sounds.bleep, None, 0.8, 1.0)
            }
            _ => {}
        }
        match *e {
            SimEvent::Fired { player, pos, .. } => {
                let mine = Some(player) == me;
                let red = game
                    .snap
                    .players
                    .iter()
                    .any(|p| p.id == player && p.team == lt_sim::Team::Red);
                let sound = if red { &sounds.fire_red } else { &sounds.fire };
                play(
                    &mut commands,
                    gain,
                    sound,
                    (!mine).then_some(pos),
                    if mine { 0.55 } else { 1.0 },
                    1.0,
                );
            }
            SimEvent::Denied { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.deny, None, 0.6, 1.0)
            }
            SimEvent::Empty { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.empty, None, 0.8, 1.0)
            }
            SimEvent::Reloading { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.reload, None, 0.6, 1.0)
            }
            SimEvent::Reloaded { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.reload_done, None, 0.7, 1.0)
            }
            SimEvent::Healed { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.reload_done, None, 0.8, 1.5)
            }
            SimEvent::PickedUp { player, pos, .. } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.pickup,
                    (!mine).then_some(pos),
                    if mine { 0.8 } else { 1.0 },
                    1.0,
                );
            }
            SimEvent::PickupSpawned { pos, .. } => {
                play(&mut commands, gain, &sounds.pickup, Some(pos), 0.5, 0.7)
            }
            SimEvent::Hit { player, pos, .. } => {
                if Some(player) == me {
                    play(&mut commands, gain, &sounds.hit, None, 0.9, 1.0);
                } else {
                    play(&mut commands, gain, &sounds.other_hit, Some(pos), 1.2, 1.0);
                }
            }
            SimEvent::TaggedOut { player, pos, .. } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.tagged,
                    (!mine).then_some(pos),
                    1.0,
                    1.0,
                );
            }
            SimEvent::TargetHit { pos, .. } => {
                // Across the whole arena, and from where it is.
                play(&mut commands, gain, &sounds.target, None, 0.6, 1.0);
                play(&mut commands, gain, &sounds.target, Some(pos), 1.0, 1.0);
            }
            SimEvent::SecondaryHit {
                by, pos, charged, ..
            } => {
                let mine = Some(by) == me;
                play(&mut commands, gain, &sounds.target, Some(pos), 1.0, 1.15);
                if charged {
                    play(
                        &mut commands,
                        gain,
                        &sounds.power_up,
                        (!mine).then_some(pos),
                        1.0,
                        1.0,
                    );
                }
            }
            SimEvent::MegaHit { .. } => {
                play(&mut commands, gain, &sounds.mega_win, None, 1.0, 1.0);
            }
            SimEvent::PowerLost { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.turret_down, None, 0.7, 1.3)
            }
            SimEvent::TurretFired { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.turret_fire,
                Some(pos),
                1.0,
                1.0,
            ),
            SimEvent::TurretOff { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.turret_down,
                Some(pos),
                1.2,
                1.0,
            ),
            SimEvent::TurretOn { turret } | SimEvent::TurretEngaged { turret } => {
                if let Some(t) = game
                    .level
                    .mega
                    .as_ref()
                    .and_then(|m| m.turrets.get(turret as usize))
                {
                    play(
                        &mut commands,
                        gain,
                        &sounds.turret_active,
                        Some(t.pos),
                        1.0,
                        1.0,
                    )
                }
            }
            SimEvent::Shocked { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.zap,
                    (!mine).then_some(pos),
                    1.0,
                    1.0,
                )
            }
            SimEvent::GrenadeThrown { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.throw,
                    (!mine).then_some(pos),
                    if mine { 0.7 } else { 1.0 },
                    1.0,
                );
            }
            SimEvent::GrenadeBounce { pos, speed } => play(
                &mut commands,
                gain,
                &sounds.bounce,
                Some(pos),
                (speed / 5.0).clamp(0.35, 1.2),
                1.0,
            ),
            SimEvent::GrenadeArmed { pos } => {
                play(&mut commands, gain, &sounds.arm, Some(pos), 1.0, 1.0)
            }
            SimEvent::GrenadeBurst { pos } => {
                play(&mut commands, gain, &sounds.burst, Some(pos), 0.9, 1.0)
            }
            SimEvent::Respawned { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.respawn,
                    (!mine).then_some(pos),
                    0.8,
                    1.0,
                );
            }
            _ => {}
        }
    }
}

fn footsteps(
    mut commands: Commands,
    game: Res<Game>,
    sounds: Res<Sounds>,
    mut walked: ResMut<Walked>,
    settings: Option<Res<Settings>>,
) {
    let gain = settings.map_or(1.0, |s| s.sound_volume);
    let walked = &mut *walked;
    for p in &game.snap.players {
        let entry = walked.last.entry(p.id).or_insert((p.feet, 0.0));
        let d = Vec2::new(p.feet.x - entry.0.x, p.feet.z - entry.0.z).length();
        // Respawns and falls aren't steps.
        let step = if d < 1.0 && (p.feet.y - entry.0.y).abs() < 0.3 {
            d
        } else {
            0.0
        };
        entry.0 = p.feet;
        entry.1 += step;
        if entry.1 > 0.8 && p.respawn.is_none() {
            entry.1 = 0.0;
            walked.n += 1;
            let mine = Some(p.id) == game.me;
            let pitch = 0.9 + (walked.n * 37 % 20) as f32 / 100.0;
            play(
                &mut commands,
                gain,
                &sounds.steps[walked.n % 3],
                (!mine).then_some(p.feet),
                if mine { 0.35 } else { 0.8 },
                pitch,
            );
        }
    }
    walked
        .last
        .retain(|id, _| game.snap.players.iter().any(|p| p.id == *id));
}

/// Megatarget mode: every player who may enter the megatarget's room ticks where they are,
/// once a second, four times a second in the last ten (higher, too).
fn ticking(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    sounds: Res<Sounds>,
    mut ticking: ResMut<Ticking>,
    settings: Option<Res<Settings>>,
) {
    let gain = settings.map_or(1.0, |s| s.sound_volume);
    let dt = time.delta_secs();
    let ticking = &mut ticking.0;
    for p in &game.snap.players {
        let Some(left) = p.power.filter(|_| p.respawn.is_none()) else {
            ticking.remove(&p.id);
            continue;
        };
        let next = ticking.entry(p.id).or_insert(0.0);
        *next -= dt;
        if *next > 0.0 {
            continue;
        }
        let hurry = left < 10.0;
        *next = if left < 5.0 {
            0.25
        } else if hurry {
            0.5
        } else {
            1.0
        };
        let mine = Some(p.id) == game.me;
        play(
            &mut commands,
            gain,
            &sounds.tick,
            (!mine).then_some(p.feet + Vec3::Y * 1.2),
            if mine { 0.7 } else { 1.0 },
            if hurry { 1.25 } else { 1.0 },
        );
    }
    ticking.retain(|id, _| game.snap.players.iter().any(|p| p.id == *id));
}

//! Sounds (assets/sounds, synthesized by tools/gen_sfx.py): game events in space around the
//! listener, footsteps from how far each player walks.
//!
//! Spatial sounds fade with the square of the distance (rodio), measured in units of
//! `SPATIAL_SCALE` metres' reciprocal: full volume up to 1 / SPATIAL_SCALE metres, a quarter at
//! twice that. (At the default 1 m a grenade bouncing 10 m away was 40 dB down.) Team target
//! hits are heard everywhere.

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
}

#[derive(Resource, Default)]
struct Walked {
    last: HashMap<PlayerId, (Vec3, f32)>,
    n: usize,
}

/// Spatial sounds play at full volume within 1 / SPATIAL_SCALE metres (see above).
const SPATIAL_SCALE: f32 = 0.12;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Walked>()
            .insert_resource(DefaultSpatialScale(SpatialScale::new(SPATIAL_SCALE)))
            .add_systems(Startup, load)
            .add_systems(Update, (play_events, footsteps).run_if(resource_exists::<Game>));
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
    });
}

/// Plays `sound` (`gain`: the settings' sound volume).
fn play(commands: &mut Commands, gain: f32, sound: &Handle<AudioSource>, at: Option<Vec3>, volume: f32, speed: f32) {
    let settings = PlaybackSettings {
        mode: PlaybackMode::Despawn,
        volume: Volume::Linear(volume * gain),
        speed,
        spatial: at.is_some(),
        ..default()
    };
    commands.spawn((AudioPlayer(sound.clone()), settings, Transform::from_translation(at.unwrap_or_default())));
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
    let team = |id| game.snap.players.iter().find(|p| p.id == id).map(|p| p.team);
    let enemy = |id| me.is_some_and(|m| m != id && team(id).is_some() && team(id) != team(m));
    for GameEvent(e) in events.read() {
        // The reward for scoring (the tag-out's own, rather than the hit's, for the last hit).
        match *e {
            SimEvent::Hit { player, by, hits, .. } if Some(by) == me && enemy(player) && hits < game.rules.hits_to_tag => {
                play(&mut commands, gain, &sounds.bleep, None, 0.8, 1.0)
            }
            SimEvent::TaggedOut { player, by, .. } if Some(by) == me && enemy(player) => {
                play(&mut commands, gain, &sounds.bleep_tag, None, 0.8, 1.0)
            }
            SimEvent::TargetHit { by, .. } if Some(by) == me => play(&mut commands, gain, &sounds.bleep, None, 0.8, 1.0),
            _ => {}
        }
        match *e {
            SimEvent::Fired { player, pos, .. } => {
                let mine = Some(player) == me;
                let red = game.snap.players.iter().any(|p| p.id == player && p.team == lt_sim::Team::Red);
                let sound = if red { &sounds.fire_red } else { &sounds.fire };
                play(&mut commands, gain, sound, (!mine).then_some(pos), if mine { 0.55 } else { 1.0 }, 1.0);
            }
            SimEvent::Denied { player } if Some(player) == me => play(&mut commands, gain, &sounds.deny, None, 0.6, 1.0),
            SimEvent::Empty { player } if Some(player) == me => play(&mut commands, gain, &sounds.empty, None, 0.8, 1.0),
            SimEvent::Reloading { player } if Some(player) == me => play(&mut commands, gain, &sounds.reload, None, 0.6, 1.0),
            SimEvent::Reloaded { player } if Some(player) == me => play(&mut commands, gain, &sounds.reload_done, None, 0.7, 1.0),
            SimEvent::PickedUp { player, pos, .. } => {
                let mine = Some(player) == me;
                play(&mut commands, gain, &sounds.pickup, (!mine).then_some(pos), if mine { 0.8 } else { 1.0 }, 1.0);
            }
            SimEvent::PickupSpawned { pos, .. } => play(&mut commands, gain, &sounds.pickup, Some(pos), 0.5, 0.7),
            SimEvent::Hit { player, pos, .. } => {
                if Some(player) == me {
                    play(&mut commands, gain, &sounds.hit, None, 0.9, 1.0);
                } else {
                    play(&mut commands, gain, &sounds.other_hit, Some(pos), 1.2, 1.0);
                }
            }
            SimEvent::TaggedOut { player, pos, .. } => {
                let mine = Some(player) == me;
                play(&mut commands, gain, &sounds.tagged, (!mine).then_some(pos), 1.0, 1.0);
            }
            SimEvent::TargetHit { pos, .. } => {
                // Across the whole arena, and from where it is.
                play(&mut commands, gain, &sounds.target, None, 0.6, 1.0);
                play(&mut commands, gain, &sounds.target, Some(pos), 1.0, 1.0);
            }
            SimEvent::GrenadeThrown { player, pos } => {
                let mine = Some(player) == me;
                play(&mut commands, gain, &sounds.throw, (!mine).then_some(pos), if mine { 0.7 } else { 1.0 }, 1.0);
            }
            SimEvent::GrenadeBounce { pos, speed } => {
                play(&mut commands, gain, &sounds.bounce, Some(pos), (speed / 5.0).clamp(0.35, 1.2), 1.0)
            }
            SimEvent::GrenadeArmed { pos } => play(&mut commands, gain, &sounds.arm, Some(pos), 1.0, 1.0),
            SimEvent::GrenadeBurst { pos } => play(&mut commands, gain, &sounds.burst, Some(pos), 0.9, 1.0),
            SimEvent::Respawned { player, pos } => {
                let mine = Some(player) == me;
                play(&mut commands, gain, &sounds.respawn, (!mine).then_some(pos), 0.8, 1.0);
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
        let step = if d < 1.0 && (p.feet.y - entry.0.y).abs() < 0.3 { d } else { 0.0 };
        entry.0 = p.feet;
        entry.1 += step;
        if entry.1 > 0.8 && p.respawn.is_none() {
            entry.1 = 0.0;
            walked.n += 1;
            let mine = Some(p.id) == game.me;
            let pitch = 0.9 + (walked.n * 37 % 20) as f32 / 100.0;
            play(&mut commands, gain, &sounds.steps[walked.n % 3], (!mine).then_some(p.feet), if mine { 0.35 } else { 0.8 }, pitch);
        }
    }
    walked.last.retain(|id, _| game.snap.players.iter().any(|p| p.id == *id));
}

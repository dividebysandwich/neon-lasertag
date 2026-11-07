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
use lt_sim::{PlayerId, SimEvent, Weapon};

use crate::game::{Game, GameEvent};
use crate::settings::Settings;

#[derive(Resource)]
struct Sounds {
    /// Blasters firing: Blue's and Red's sound different.
    fire: Handle<AudioSource>,
    fire_red: Handle<AudioSource>,
    shotgun: Handle<AudioSource>,
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
    mirror_ping: Handle<AudioSource>,
    jump_pad: Handle<AudioSource>,
    charge_loop: Handle<AudioSource>,
    charge_full: Handle<AudioSource>,
    beam_fire: Handle<AudioSource>,
    lance_fire: Handle<AudioSource>,
    headshot: Handle<AudioSource>,
    sensor_pulse: Handle<AudioSource>,
    barrier_up: Handle<AudioSource>,
    barrier_hit: Handle<AudioSource>,
    barrier_down: Handle<AudioSource>,
    event_blackout: Handle<AudioSource>,
    event_overdrive: Handle<AudioSource>,
    event_storm: Handle<AudioSource>,
    event_lowgrav: Handle<AudioSource>,
    event_double: Handle<AudioSource>,
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
                ((play_events, footsteps), cap_mix)
                    .chain()
                    .run_if(resource_exists::<Game>),
            )
            .add_systems(
                Update,
                (ticking, charge_hum).run_if(resource_exists::<Game>),
            );
    }
}

fn load(mut commands: Commands, assets: Res<AssetServer>) {
    let s = |n: &str| assets.load(format!("sounds/{n}.ogg"));
    commands.insert_resource(Sounds {
        fire: s("laser_fire"),
        fire_red: s("laser_fire_red"),
        lance_fire: s("lance_fire"),
        headshot: s("headshot"),
        sensor_pulse: s("sensor_pulse"),
        shotgun: s("shotgun"),
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
        mirror_ping: s("mirror_ping"),
        jump_pad: s("jump_pad"),
        charge_loop: s("charge_loop"),
        charge_full: s("charge_full"),
        beam_fire: s("beam_fire"),
        barrier_up: s("barrier_up"),
        barrier_hit: s("barrier_hit"),
        barrier_down: s("barrier_down"),
        event_blackout: s("event_blackout"),
        event_overdrive: s("event_overdrive"),
        event_storm: s("event_storm"),
        event_lowgrav: s("event_lowgrav"),
        event_double: s("event_double"),
        tick: s("power_tick"),
        power_up: s("power_up"),
        turret_fire: s("turret_fire"),
        turret_down: s("turret_down"),
        turret_active: s("turretactive"),
        zap: s("shield_zap"),
    });
}

/// Sounds that pile up when many play at once: all of a group together are held to at most
/// `CAP` times one at its usual volume.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mix {
    Steps,
    Fire,
}

impl Mix {
    /// One of the group at its usual volume (another player's, close by).
    fn usual(self) -> f32 {
        match self {
            Mix::Steps => 0.8,
            Mix::Fire => 1.0,
        }
    }
}

const CAP: f32 = 2.0;

/// A sound of a capped group: its volume before the sound setting and the cap.
#[derive(Component)]
struct Capped {
    mix: Mix,
    volume: f32,
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
    spawn(commands, gain, sound, at, volume, speed);
}

/// Plays `sound` as one of `mix`'s group, held down with the rest of it (see `cap_mix`).
#[allow(clippy::too_many_arguments)]
fn play_capped(
    commands: &mut Commands,
    gain: f32,
    sound: &Handle<AudioSource>,
    at: Option<Vec3>,
    volume: f32,
    speed: f32,
    mix: Mix,
) {
    spawn(commands, gain, sound, at, volume, speed).insert(Capped { mix, volume });
}

fn spawn<'a>(
    commands: &'a mut Commands,
    gain: f32,
    sound: &Handle<AudioSource>,
    at: Option<Vec3>,
    volume: f32,
    speed: f32,
) -> EntityCommands<'a> {
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
    ))
}

type CappedSound = (
    &'static Capped,
    &'static mut PlaybackSettings,
    // (Not the global one: that's still at the origin for those spawned this frame. They have
    // no parent.)
    &'static Transform,
    Option<&'static mut AudioSink>,
    Option<&'static mut SpatialAudioSink>,
);

/// Holds each capped group (steps, blaster fire) to `CAP` times one of it at its usual volume:
/// sums how loud each of its sounds is heard (spatial ones fading with distance as rodio fades
/// them) and turns all of them down alike when the sum is over. Sounds just spawned start at the
/// capped volume; playing ones follow as others start and end.
fn cap_mix(
    settings: Option<Res<Settings>>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut sounds: Query<CappedSound>,
) {
    let gain = crate::settings::sound_gain(settings.as_deref());
    let ear = listener.iter().next().map(|t| t.translation());
    let heard = |spatial: bool, at: Vec3| match (spatial, ear) {
        (true, Some(ear)) => {
            let d = ear.distance(at) * SPATIAL_SCALE;
            (1.0 / (d * d)).min(1.0)
        }
        _ => 1.0,
    };
    for mix in [Mix::Steps, Mix::Fire] {
        let sum: f32 = sounds
            .iter()
            .filter(|(c, ..)| c.mix == mix)
            .map(|(c, s, t, ..)| c.volume * heard(s.spatial, t.translation))
            .sum();
        let k = (CAP * mix.usual() / sum.max(f32::EPSILON)).min(1.0);
        for (c, mut s, _, sink, spatial_sink) in &mut sounds {
            if c.mix != mix {
                continue;
            }
            let v = Volume::Linear(c.volume * gain * k);
            if let Some(mut sink) = sink {
                sink.set_volume(v);
            } else if let Some(mut sink) = spatial_sink {
                sink.set_volume(v);
            } else {
                s.volume = v;
            }
        }
    }
}

/// The local player's charge gun charging: a whine rising as it charges.
#[derive(Component)]
struct ChargeHum;

fn charge_hum(
    mut commands: Commands,
    game: Res<Game>,
    sounds: Res<Sounds>,
    settings: Option<Res<Settings>>,
    mut hum: Query<&mut AudioSink, With<ChargeHum>>,
    mut spawned: Local<bool>,
) {
    if !*spawned {
        *spawned = true;
        commands.spawn((
            AudioPlayer(sounds.charge_loop.clone()),
            PlaybackSettings {
                mode: PlaybackMode::Loop,
                volume: Volume::Linear(0.0),
                ..default()
            },
            ChargeHum,
        ));
        return;
    }
    let gain = crate::settings::sound_gain(settings.as_deref());
    let charging = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
        .map_or(0.0, |p| p.charging);
    let k = (charging / game.rules.charge_time).min(1.0);
    for mut sink in &mut hum {
        sink.set_volume(Volume::Linear(if charging > 0.0 {
            (0.3 + 0.4 * k) * gain
        } else {
            0.0
        }));
        sink.set_speed(0.7 + 0.8 * k);
    }
}

fn play_events(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    sounds: Res<Sounds>,
    settings: Option<Res<Settings>>,
) {
    let gain = crate::settings::sound_gain(settings.as_deref());
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
            | SimEvent::Assist { player: by, .. }
            | SimEvent::Defended { player: by, .. }
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
                let weapon = game
                    .snap
                    .players
                    .iter()
                    .find(|p| p.id == player)
                    .map(|p| p.weapon);
                let shotgun = weapon == Some(Weapon::Shotgun);
                let sound = if shotgun {
                    &sounds.shotgun
                } else if weapon == Some(Weapon::Lance) {
                    &sounds.lance_fire
                } else if red {
                    &sounds.fire_red
                } else {
                    &sounds.fire
                };
                play_capped(
                    &mut commands,
                    gain,
                    sound,
                    (!mine).then_some(pos),
                    // (The shotgun's sample is quieter than the lasers' at the same peak.)
                    if mine { 0.55 } else { 1.0 } * if shotgun { 1.4 } else { 1.0 },
                    1.0,
                    Mix::Fire,
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
            SimEvent::ShieldUp { player } => {
                let at = (Some(player) != me)
                    .then(|| {
                        game.snap
                            .players
                            .iter()
                            .find(|p| p.id == player)
                            .map(|p| p.feet + Vec3::Y * 1.2)
                    })
                    .flatten();
                play(&mut commands, gain, &sounds.power_up, at, 0.7, 1.5);
            }
            SimEvent::ShieldDown { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.zap, None, 0.4, 0.6)
            }
            SimEvent::SpeedDown { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.zap, None, 0.4, 0.7)
            }
            SimEvent::Uncloaked { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.zap, None, 0.4, 0.8)
            }
            SimEvent::ChargeFull { player } => {
                let at = (Some(player) != me)
                    .then(|| {
                        game.snap
                            .players
                            .iter()
                            .find(|p| p.id == player)
                            .map(|p| p.feet + Vec3::Y * 1.3)
                    })
                    .flatten();
                play(&mut commands, gain, &sounds.charge_full, at, 0.8, 1.0)
            }
            SimEvent::DecoyOut { pos, .. } => {
                play(&mut commands, gain, &sounds.power_up, Some(pos), 0.6, 1.4)
            }
            SimEvent::DecoyPopped { pos, .. } => {
                play(&mut commands, gain, &sounds.other_hit, Some(pos), 1.0, 1.35)
            }
            SimEvent::SensorThrown { pos, .. } => {
                play(&mut commands, gain, &sounds.throw, Some(pos), 0.7, 1.3)
            }
            SimEvent::SensorStuck { pos, .. } => {
                play(&mut commands, gain, &sounds.bounce, Some(pos), 0.8, 1.6)
            }
            SimEvent::Pulse { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.sensor_pulse,
                Some(pos),
                0.9,
                1.0,
            ),
            SimEvent::Headshot { by, pos, .. } => {
                let mine = Some(by) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.headshot,
                    (!mine).then_some(pos),
                    if mine { 0.9 } else { 0.7 },
                    1.0,
                )
            }
            SimEvent::BeamFired { player, pos, .. } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.beam_fire,
                    (!mine).then_some(pos),
                    if mine { 0.8 } else { 1.0 },
                    1.0,
                )
            }
            // Arena events, for everyone (and the lights coming back after a blackout).
            SimEvent::EventStart { kind } => {
                let sound = match kind {
                    lt_sim::ArenaEvent::Blackout => &sounds.event_blackout,
                    lt_sim::ArenaEvent::Overdrive => &sounds.event_overdrive,
                    lt_sim::ArenaEvent::MirrorStorm => &sounds.event_storm,
                    lt_sim::ArenaEvent::LowGravity => &sounds.event_lowgrav,
                };
                play(&mut commands, gain, sound, None, 1.0, 1.0)
            }
            SimEvent::EventEnd {
                kind: lt_sim::ArenaEvent::Blackout,
            } => play(&mut commands, gain, &sounds.power_up, None, 0.7, 0.8),
            SimEvent::Countdown { .. } => play(&mut commands, gain, &sounds.bleep, None, 1.0, 0.8),
            SimEvent::Go => play(&mut commands, gain, &sounds.bleep_tag, None, 1.0, 1.2),
            SimEvent::DoublePoints => {
                play(&mut commands, gain, &sounds.event_double, None, 1.0, 1.0)
            }
            SimEvent::BarrierUp { pos, .. } => {
                play(&mut commands, gain, &sounds.barrier_up, Some(pos), 1.0, 1.0)
            }
            SimEvent::BarrierHit { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.barrier_hit,
                Some(pos),
                0.9,
                1.0,
            ),
            SimEvent::BarrierDown { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.barrier_down,
                Some(pos),
                0.8,
                1.0,
            ),
            SimEvent::GateZap { pos, .. } => {
                play(&mut commands, gain, &sounds.zap, Some(pos), 0.8, 1.3)
            }
            SimEvent::Launched { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.jump_pad,
                    (!mine).then_some(pos),
                    if mine { 0.7 } else { 1.0 },
                    1.0,
                )
            }
            SimEvent::Dashed { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.jump_pad,
                    (!mine).then_some(pos),
                    if mine { 0.4 } else { 0.6 },
                    0.7,
                )
            }
            SimEvent::Jumped { player, pos } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.jump_pad,
                    (!mine).then_some(pos),
                    if mine { 0.3 } else { 0.45 },
                    1.5,
                )
            }
            SimEvent::Reflected { pos, .. } => play(
                &mut commands,
                gain,
                &sounds.mirror_ping,
                Some(pos),
                0.9,
                1.0,
            ),
            SimEvent::Blocked { pos, .. } => {
                play(&mut commands, gain, &sounds.zap, Some(pos), 0.6, 1.6)
            }
            SimEvent::Healed { player } if Some(player) == me => {
                play(&mut commands, gain, &sounds.reload_done, None, 0.8, 1.5)
            }
            SimEvent::Mended { player, by, .. } if Some(player) == me || Some(by) == me => {
                play(&mut commands, gain, &sounds.reload_done, None, 0.6, 1.8)
            }
            SimEvent::PickedUp {
                player, pos, kind, ..
            } => {
                let mine = Some(player) == me;
                play(
                    &mut commands,
                    gain,
                    &sounds.pickup,
                    (!mine).then_some(pos),
                    if mine { 0.8 } else { 1.0 },
                    if kind == lt_sim::PickupKind::Grenade {
                        1.0
                    } else {
                        1.3
                    },
                );
                if mine && kind != lt_sim::PickupKind::Grenade {
                    play(&mut commands, gain, &sounds.power_up, None, 0.5, 1.2);
                }
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
    let gain = crate::settings::sound_gain(settings.as_deref());
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
            play_capped(
                &mut commands,
                gain,
                &sounds.steps[walked.n % 3],
                (!mine).then_some(p.feet),
                if mine { 0.35 } else { 0.8 },
                pitch,
                Mix::Steps,
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
    let gain = crate::settings::sound_gain(settings.as_deref());
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

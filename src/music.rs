//! Music during play (assets/music/menu01.ogg, looped while a `Game` runs, quietly enough for
//! the sound effects) and its spectrum, `MusicSpectrum`, for the scenery's displays and lights.
//!
//! The spectrum is precomputed by tools/gen_spectrum.py into assets/music/menu01.spectrum
//! (little endian): b"LTSP", u16 version (1), u16 channels per frame C (7), f32 frames per
//! second, u32 frames N, f32 duration (s), then N * C u8 values (value / 255 is 0..1). The
//! channels are the 6 log-spaced bands (20-120, 120-350, 350-1000, 1k-2.8k, 2.8k-7k, 7k-16k Hz),
//! each auto-gained to 0..1 on a dB scale, then the bass pulse (an onset envelope of the lowest
//! band). It follows the sink's playback position, so it stays in step through the loops.
//! Without the file the spectrum stays at zero.

use bevy::audio::{AudioSinkPlayback, Volume};
use bevy::prelude::*;

use crate::game::Game;

const MUSIC: &str = "music/menu01.ogg";
const SPECTRUM: &str = "music/menu01.spectrum";
const VOLUME: f32 = 0.22;
pub const BANDS: usize = 6;

/// The music's spectrum now, smoothed for display (zeros without music).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct MusicSpectrum {
    /// Low to high, 0..1.
    pub bands: [f32; BANDS],
    /// Beat-ish pulses of the bass, 0..1 (instant rise, fast fall).
    pub bass: f32,
}

/// The precomputed spectrum file.
#[derive(Resource, Default)]
struct SpectrumData {
    fps: f32,
    channels: usize,
    duration: f32,
    values: Vec<u8>,
}

impl SpectrumData {
    fn parse(b: &[u8]) -> Option<SpectrumData> {
        if b.len() < 20 || &b[0..4] != b"LTSP" {
            return None;
        }
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let channels = u16_at(6) as usize;
        let fps = f32::from_bits(u32_at(8));
        let frames = u32_at(12) as usize;
        let duration = f32::from_bits(u32_at(16));
        if u16_at(4) != 1 || channels < BANDS + 1 || fps <= 0.0 || b.len() < 20 + frames * channels {
            return None;
        }
        Some(SpectrumData { fps, channels, duration, values: b[20..20 + frames * channels].to_vec() })
    }

    fn frames(&self) -> usize {
        self.values.len().checked_div(self.channels).unwrap_or(0)
    }

    /// Channel `c` at `t` seconds into the music, interpolated.
    fn sample(&self, t: f32, c: usize) -> f32 {
        let n = self.frames();
        if n == 0 {
            return 0.0;
        }
        let f = (t * self.fps).max(0.0);
        let i = (f as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        let k = f.fract();
        let v = |i: usize| self.values[i * self.channels + c] as f32 / 255.0;
        v(i) * (1.0 - k) + v(j) * k
    }
}

#[derive(Component)]
struct Music;

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MusicSpectrum>()
            .add_systems(Startup, load)
            .add_systems(Update, (start_stop, follow).chain());
    }
}

fn load(mut commands: Commands) {
    let path = crate::assets::asset_dir().join(SPECTRUM);
    let data = match std::fs::read(&path) {
        Ok(b) => SpectrumData::parse(&b).unwrap_or_else(|| {
            warn!("{}: not a spectrum file", path.display());
            SpectrumData::default()
        }),
        Err(e) => {
            warn!("{}: {e}; the spectrum displays stay dark", path.display());
            SpectrumData::default()
        }
    };
    commands.insert_resource(data);
}

fn start_stop(
    mut commands: Commands,
    game: Option<Res<Game>>,
    music: Query<Entity, With<Music>>,
    assets: Res<AssetServer>,
    mut spectrum: ResMut<MusicSpectrum>,
) {
    match (game.is_some(), music.iter().next()) {
        (true, None) => {
            commands.spawn((
                Music,
                AudioPlayer::new(assets.load(MUSIC)),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(VOLUME)),
                Name::new("music"),
            ));
        }
        (false, Some(e)) => {
            commands.entity(e).despawn();
            *spectrum = MusicSpectrum::default();
        }
        _ => {}
    }
}

/// Follows the music's playback position through the spectrum.
fn follow(
    time: Res<Time>,
    data: Res<SpectrumData>,
    sinks: Query<&AudioSink, With<Music>>,
    mut spectrum: ResMut<MusicSpectrum>,
) {
    let Some(sink) = sinks.iter().next() else { return };
    if data.frames() == 0 || sink.is_paused() {
        return;
    }
    let duration = if data.duration > 0.0 { data.duration } else { data.frames() as f32 / data.fps };
    // The position may or may not restart with each loop: wrap it either way.
    let t = sink.position().as_secs_f32() % duration;
    let dt = time.delta_secs();
    for (b, level) in spectrum.bands.iter_mut().enumerate() {
        let target = data.sample(t, b);
        // Quick to rise, slower to fall, like a meter.
        let rate = if target > *level { 25.0 } else { 7.0 };
        *level += (target - *level) * (1.0 - (-rate * dt).exp());
    }
    let bass = data.sample(t, BANDS);
    spectrum.bass = if bass > spectrum.bass { bass } else { spectrum.bass + (bass - spectrum.bass) * (1.0 - (-12.0 * dt).exp()) };
}

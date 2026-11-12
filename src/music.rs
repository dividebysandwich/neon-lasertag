//! The music, chosen by what's on: `menu*.ogg` in the menus, `game*.ogg` during a match,
//! `won*.ogg` or `lost*.ogg` once at the end of one (assets/music; a draw counts as lost). Each
//! time a track is due, one of the mood's files is picked at random (not the one just played, if
//! there are others); the menu's music goes on with another when one ends, while a match's track
//! loops until the match is over. Quietly
//! enough for the sound effects. Its spectrum, `MusicSpectrum`, drives the scenery's displays and
//! lights and the border walls' pulse.
//!
//! Each track's spectrum is precomputed by tools/gen_spectrum.py into X.spectrum beside X.ogg
//! (little endian): b"LTSP", u16 version (1), u16 channels per frame C (7), f32 frames per
//! second, u32 frames N, f32 duration (s), then N * C u8 values (value / 255 is 0..1). The
//! channels are the 6 log-spaced bands (20-120, 120-350, 350-1000, 1k-2.8k, 2.8k-7k, 7k-16k Hz),
//! each auto-gained to 0..1 on a dB scale, then the bass pulse (an onset envelope of the lowest
//! band). It follows the sink's playback position. Without the file the spectrum stays at zero.

use bevy::audio::{AudioSinkPlayback, Volume};
use bevy::prelude::*;

use rand::Rng;

use crate::endscreen::winner;
use crate::game::Game;
use crate::settings::Settings;

/// The music's volume at the settings' full volume.
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
        if u16_at(4) != 1 || channels < BANDS + 1 || fps <= 0.0 || b.len() < 20 + frames * channels
        {
            return None;
        }
        Some(SpectrumData {
            fps,
            channels,
            duration,
            values: b[20..20 + frames * channels].to_vec(),
        })
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

/// What the music is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mood {
    Menu,
    Game,
    Won,
    Lost,
}

impl Mood {
    const ALL: [Mood; 4] = [Mood::Menu, Mood::Game, Mood::Won, Mood::Lost];

    /// The start of its files' names.
    fn prefix(self) -> &'static str {
        match self {
            Mood::Menu => "menu",
            Mood::Game => "game",
            Mood::Won => "won",
            Mood::Lost => "lost",
        }
    }

    /// Whether another track follows when one ends (the end of a match plays once).
    fn goes_on(self) -> bool {
        matches!(self, Mood::Menu | Mood::Game)
    }

    /// Whether its track loops until the mood changes (a match keeps its track).
    fn loops(self) -> bool {
        self == Mood::Game
    }
}

/// The music files, by mood (asset paths, sorted), and their spectra.
#[derive(Resource, Default)]
struct Tracks {
    files: Vec<(Mood, String, SpectrumData)>,
}

impl Tracks {
    fn of(&self, mood: Mood) -> impl Iterator<Item = usize> + '_ {
        self.files
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.0 == mood)
            .map(|(i, _)| i)
    }
}

/// The track playing (an index into `Tracks::files`), and whether it loops.
#[derive(Component)]
pub(crate) struct Music(usize, bool);

/// The mood the music is in, whether its track for it has started, and the last track played.
#[derive(Resource, Default)]
struct Playing {
    mood: Option<Mood>,
    started: bool,
    last: Option<usize>,
}

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MusicSpectrum>()
            .init_resource::<Playing>()
            .add_systems(Startup, load)
            .add_systems(Update, (choose, volume, follow).chain());
    }
}

/// The mood for the file called `name`, if it is music: `<prefix><anything>.ogg`.
fn mood_of(name: &str) -> Option<Mood> {
    let stem = name.strip_suffix(".ogg")?;
    Mood::ALL.into_iter().find(|m| stem.starts_with(m.prefix()))
}

/// The music's file names, and a spectrum file's bytes (by the track's name).
#[cfg(not(target_arch = "wasm32"))]
fn music_files() -> (Vec<String>, impl Fn(&str) -> Result<Vec<u8>, String>) {
    let dir = crate::assets::asset_dir().join("music");
    let names = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(e) => {
            warn!("{}: {e}; no music", dir.display());
            Vec::new()
        }
    };
    let read = move |name: &str| {
        let path = dir.join(name).with_extension("spectrum");
        std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
    };
    (names, read)
}

/// The browser build: the tracks are the compiled-in spectra's (the .ogg files are fetched
/// with the page's assets).
#[cfg(target_arch = "wasm32")]
fn music_files() -> (Vec<String>, impl Fn(&str) -> Result<Vec<u8>, String>) {
    let names = crate::assets::embedded_names()
        .filter_map(|n| n.strip_prefix("music/")?.strip_suffix(".spectrum"))
        .map(|stem| format!("{stem}.ogg"))
        .collect();
    let read = |name: &str| {
        let rel = format!("music/{}", name.replace(".ogg", ".spectrum"));
        crate::assets::embedded(&rel)
            .map(<[u8]>::to_vec)
            .ok_or(format!("{rel}: not compiled in"))
    };
    (names, read)
}

fn load(mut commands: Commands) {
    let (mut names, read) = music_files();
    names.sort();
    let mut tracks = Tracks::default();
    for name in names {
        let Some(mood) = mood_of(&name) else { continue };
        let data = match read(&name) {
            Ok(b) => SpectrumData::parse(&b).unwrap_or_else(|| {
                warn!("{name}: not a spectrum file");
                SpectrumData::default()
            }),
            Err(e) => {
                warn!("{e}; the spectrum displays stay dark for it");
                SpectrumData::default()
            }
        };
        tracks.files.push((mood, format!("music/{name}"), data));
    }
    for m in Mood::ALL {
        if tracks.of(m).next().is_none() {
            warn!("no music/{}*.ogg", m.prefix());
        }
    }
    commands.insert_resource(tracks);
}

/// The local player's team won (a draw isn't a win).
fn won(game: &Game) -> bool {
    let mine = game
        .me
        .and_then(|me| game.snap.players.iter().find(|p| p.id == me))
        .map(|p| p.team);
    mine.is_some() && winner(game.snap.scores).1 == mine
}

/// Starts the music for what's on, and the next track when one ends.
#[allow(clippy::too_many_arguments)]
fn choose(
    mut commands: Commands,
    game: Option<Res<Game>>,
    tracks: Res<Tracks>,
    music: Query<Entity, With<Music>>,
    assets: Res<AssetServer>,
    settings: Option<Res<Settings>>,
    mut playing: ResMut<Playing>,
    mut spectrum: ResMut<MusicSpectrum>,
) {
    let mood = match game.as_deref() {
        None => Mood::Menu,
        Some(g) if g.snap.over => {
            if won(g) {
                Mood::Won
            } else {
                Mood::Lost
            }
        }
        Some(_) => Mood::Game,
    };
    if playing.mood != Some(mood) {
        for e in &music {
            commands.entity(e).despawn();
        }
        *spectrum = MusicSpectrum::default();
        playing.mood = Some(mood);
        playing.started = false;
    } else if !music.is_empty() || (playing.started && !mood.goes_on()) {
        return;
    }
    // A track (another than the last, if there are others) for the mood.
    let choices: Vec<usize> = tracks.of(mood).collect();
    let fresh: Vec<usize> = choices
        .iter()
        .copied()
        .filter(|&i| Some(i) != playing.last)
        .collect();
    let pool = if fresh.is_empty() { &choices } else { &fresh };
    playing.started = true;
    if pool.is_empty() {
        return;
    }
    let i = pool[rand::rng().random_range(0..pool.len())];
    playing.last = Some(i);
    info!("music: {}", tracks.files[i].1);
    let level = settings.map_or(1.0, |s| s.music_volume);
    let playback = if mood.loops() {
        PlaybackSettings::LOOP
    } else {
        PlaybackSettings::DESPAWN
    };
    commands.spawn((
        Music(i, mood.loops()),
        AudioPlayer::new(assets.load(tracks.files[i].1.clone())),
        playback.with_volume(Volume::Linear(VOLUME * level)),
        Name::new("music"),
    ));
}

/// Follows the settings' music volume.
fn volume(settings: Option<Res<Settings>>, mut sinks: Query<&mut AudioSink, With<Music>>) {
    let Some(settings) = settings else { return };
    if !settings.is_changed() {
        return;
    }
    for mut sink in &mut sinks {
        sink.set_volume(Volume::Linear(VOLUME * settings.music_volume));
    }
}

/// Follows the music's playback position through its spectrum.
fn follow(
    time: Res<Time>,
    tracks: Res<Tracks>,
    sinks: Query<(&Music, &AudioSink)>,
    mut spectrum: ResMut<MusicSpectrum>,
) {
    let Some((music, sink)) = sinks.iter().next() else {
        return;
    };
    let data = &tracks.files[music.0].2;
    if data.frames() == 0 || sink.is_paused() {
        return;
    }
    let duration = if data.duration > 0.0 {
        data.duration
    } else {
        data.frames() as f32 / data.fps
    };
    let pos = sink.position().as_secs_f32();
    let t = if music.1 {
        pos % duration
    } else {
        pos.min(duration)
    };
    let dt = time.delta_secs();
    for (b, level) in spectrum.bands.iter_mut().enumerate() {
        let target = data.sample(t, b);
        // Quick to rise, slower to fall, like a meter.
        let rate = if target > *level { 25.0 } else { 7.0 };
        *level += (target - *level) * (1.0 - (-rate * dt).exp());
    }
    let bass = data.sample(t, BANDS);
    spectrum.bass = if bass > spectrum.bass {
        bass
    } else {
        spectrum.bass + (bass - spectrum.bass) * (1.0 - (-12.0 * dt).exp())
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_files_by_name() {
        assert_eq!(mood_of("menu01.ogg"), Some(Mood::Menu));
        assert_eq!(mood_of("game02.ogg"), Some(Mood::Game));
        assert_eq!(mood_of("won01.ogg"), Some(Mood::Won));
        assert_eq!(mood_of("lost01.ogg"), Some(Mood::Lost));
        assert_eq!(mood_of("menu01.spectrum"), None);
        assert_eq!(mood_of("other.ogg"), None);
    }
}

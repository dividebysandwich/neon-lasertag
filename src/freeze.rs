//! Offline botmatches (no one else in the game): F11 or Pause freezes everything. The game's
//! clock (`Time<Virtual>`) stops, so the simulation's ticks, the bots, bolts in flight, the
//! match clock, effects and animations all hold where they are, and the sounds playing hold
//! too (the music plays on). The free camera (src/freecam.rs) still flies about.

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink};
use bevy::prelude::*;

use crate::game::{Game, Source};
use crate::music::Music;
use crate::neonui;

/// Whether the game is frozen, and a toggle asked for other than by the keys (testing).
#[derive(Resource, Default)]
pub struct Frozen(pub bool, pub bool);

/// A sound held by the freeze, to go on when it's over.
#[derive(Component)]
struct Held;

#[derive(Component)]
struct Banner;

pub struct FreezePlugin;

impl Plugin for FreezePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Frozen>().add_systems(
            Update,
            (toggle, hold_sounds, banner)
                .chain()
                .before(crate::game::InputSet),
        );
    }
}

/// A game of one's own with no one joining: frozen time harms nobody.
fn offline(game: &Game) -> bool {
    matches!(game.source, Source::Local { server: None, .. })
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    game: Option<Res<Game>>,
    mut frozen: ResMut<Frozen>,
    mut time: ResMut<Time<Virtual>>,
) {
    let allowed = game.as_deref().is_some_and(offline);
    if !allowed {
        frozen.0 = false;
    } else if keys.any_just_pressed([KeyCode::F11, KeyCode::Pause]) || frozen.1 {
        frozen.0 = !frozen.0;
    }
    frozen.1 = false;
    if frozen.0 != time.is_paused() {
        if frozen.0 {
            time.pause();
        } else {
            time.unpause();
        }
    }
}

/// Holds the sounds playing when the game freezes, and lets them go on after. (Only those:
/// the menus' own sounds play while frozen.)
fn hold_sounds(
    mut commands: Commands,
    frozen: Res<Frozen>,
    mut was: Local<bool>,
    sinks: Query<(Entity, &AudioSink, Has<Held>), Without<Music>>,
    spatial: Query<(Entity, &SpatialAudioSink, Has<Held>)>,
) {
    if frozen.0 == *was {
        return;
    }
    *was = frozen.0;
    let mut each = |e: Entity, s: &dyn AudioSinkPlayback, held: bool| {
        if frozen.0 && !s.is_paused() {
            s.pause();
            commands.entity(e).insert(Held);
        } else if !frozen.0 && held {
            s.play();
            commands.entity(e).remove::<Held>();
        }
    };
    for (e, s, held) in &sinks {
        each(e, s, held);
    }
    for (e, s, held) in &spatial {
        each(e, s, held);
    }
}

/// "PAUSED" across the top of the screen while frozen.
fn banner(mut commands: Commands, frozen: Res<Frozen>, shown: Query<Entity, With<Banner>>) {
    match (frozen.0, shown.single()) {
        (true, Err(_)) => {
            commands
                .spawn((
                    Banner,
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Percent(14.0),
                        width: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(10.0),
                        ..default()
                    },
                    GlobalZIndex(15),
                ))
                .with_children(|p| {
                    neonui::title(p, "PAUSED", 56.0, neonui::CYAN);
                    p.spawn(neonui::text(
                        "F11 / Pause: resume    F12: free camera",
                        16.0,
                        neonui::DIM,
                    ));
                });
        }
        (false, Ok(e)) => commands.entity(e).despawn(),
        _ => {}
    }
}

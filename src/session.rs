//! The game's course: after a local match ends the next one starts on a new arena (after
//! `NEXT_MATCH_DELAY`, or when the player asks), and leaving a game takes everything of it
//! down and shows the start menu again. Remote games move on when the server sends the next
//! arena (src/game.rs).
//!
//! The end of match screens (src/endscreen.rs, src/vrui.rs) and the in-game menus
//! (src/pause.rs, src/vrui.rs) ask for these with `SessionCmd`.
//!
//! Testing: `LT_DEV="6:menu,8:leave,10:play-hard,14:hit"` does those at those seconds: toggle
//! the in-game menu, leave, start a game vs bots from the start menu (`play`, `play-easy`, ...),
//! the next match (`next`), a hit on the local player from the nearest enemy, the options
//! screen (`options`, flat screen), the held grenade's throw arc (`arc`, flat screen), a hit on the first
//! team target (`target`).

use bevy::prelude::*;
use lt_sim::bots::Difficulty;

use crate::arena::ArenaPart;
use crate::game::{Game, InputSet, LocalCmd, Mover, Source};
use crate::scenery::Scenery;

/// Seconds from the end of a local match to the next one.
pub const NEXT_MATCH_DELAY: f32 = 15.0;

#[derive(Resource, Default)]
pub struct Session {
    /// The bots' difficulty in this (local) game.
    pub difficulty: Difficulty,
    /// The arenas' size in this (local) game.
    pub arena: lt_sim::level::SizeChoice,
    /// How long ago the match ended (None while it runs).
    pub over_for: Option<f32>,
    /// Leaving: the frames of taking the game down done so far.
    leaving: u8,
}

impl Session {
    /// The rules for the next local match: the command line's, in an arena of the chosen size.
    pub fn rules(&self, args: &crate::Args) -> lt_sim::Rules {
        lt_sim::Rules { arena: self.arena.pick(rand::random()), ..args.rules() }
    }

    /// Seconds until a local game's next match.
    pub fn next_in(&self) -> Option<f32> {
        self.over_for.map(|t| (NEXT_MATCH_DELAY - t).max(0.0))
    }
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionCmd {
    /// The next match on a new arena (local games).
    NextMatch,
    /// Back to the start menu (flat screen).
    Leave,
    Quit,
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .add_message::<SessionCmd>()
            .add_systems(Update, (dev_script, run_commands, match_over, leave).chain().before(InputSet));
    }
}

/// A bot plays the local player (`--autopilot`).
pub fn autopilot(game: &mut Game, seed: u64) {
    if let (Some(me), Source::Local { bots, .. }) = (game.me, &mut game.source) {
        bots.bots.insert(me, Box::new(lt_sim::bots::WanderBot::new(seed, 0.7)));
    }
}

/// `LT_DEV`: scripted actions for testing.
#[allow(clippy::too_many_arguments)]
fn dev_script(
    mut commands: Commands,
    time: Res<Time>,
    mut script: Local<Option<Vec<(f32, String)>>>,
    mut cmds: MessageWriter<SessionCmd>,
    mut events: MessageWriter<crate::game::GameEvent>,
    game: Option<Res<Game>>,
    overlays: Option<ResMut<crate::desktop::Overlays>>,
    vr: Option<ResMut<crate::vr::VrState>>,
) {
    let script = script.get_or_insert_with(|| {
        std::env::var("LT_DEV")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.split_once(':').and_then(|(t, a)| Some((t.trim().parse().ok()?, a.trim().to_string()))))
            .collect()
    });
    let now = time.elapsed_secs();
    let Some(k) = script.iter().position(|(t, _)| *t <= now) else { return };
    let (_, action) = script.remove(k);
    info!("LT_DEV: {action}");
    match action.as_str() {
        "menu" => {
            if let Some(mut o) = overlays {
                o.menu = !o.menu;
            }
            if let Some(mut vr) = vr {
                vr.menu_requests += 1;
            }
        }
        "arc" => commands.queue(|w: &mut World| {
            if let Some(mut a) = w.get_resource_mut::<crate::desktop::ThrowArc>() {
                a.1 = !a.1;
            }
        }),
        "target" => {
            let Some(game) = game else { return };
            if let (Some(t), Some(me)) = (game.level.targets.first(), game.me) {
                events.write(crate::game::GameEvent(lt_sim::SimEvent::TargetHit { target: t.id, by: me, pos: t.centre() }));
            }
        }
        "options" => {
            if let Some(mut o) = overlays {
                o.options = !o.options;
            }
        }
        "leave" => {
            cmds.write(SessionCmd::Leave);
        }
        "next" => {
            cmds.write(SessionCmd::NextMatch);
        }
        "hit" => {
            let Some(game) = game else { return };
            let Some(me) = game.me.and_then(|me| game.snap.players.iter().find(|p| p.id == me)) else { return };
            let enemy = game
                .snap
                .players
                .iter()
                .filter(|p| p.team != me.team)
                .min_by(|a, b| a.feet.distance(me.feet).total_cmp(&b.feet.distance(me.feet)));
            if let Some(e) = enemy {
                events.write(crate::game::GameEvent(lt_sim::SimEvent::Hit {
                    player: me.id,
                    by: e.id,
                    pos: me.feet + me.head.pos,
                    hits: 1,
                }));
            }
        }
        a if a.starts_with("play") => {
            let d = a.strip_prefix("play-").and_then(Difficulty::parse).unwrap_or_default();
            commands.insert_resource(crate::menu::DevPick(d));
        }
        a => warn!("LT_DEV: unknown action {a}"),
    }
}

fn next_match(game: &mut Game, local: &LocalCmd, session: &mut Session) {
    if !game.is_local() {
        return;
    }
    let seed = rand::random::<u32>() as u64;
    game.rules.arena = session.arena.pick(rand::random());
    game.next_match(seed);
    if local.autopilot {
        autopilot(game, seed);
    }
    session.over_for = None;
    info!("next match: arena {seed} ({})\n{}", game.rules.arena.name(), lt_sim::level::ascii(&game.level));
}

fn run_commands(
    mut cmds: MessageReader<SessionCmd>,
    mut game: Option<ResMut<Game>>,
    mut session: ResMut<Session>,
    local: Res<LocalCmd>,
    mut exit: MessageWriter<AppExit>,
) {
    for cmd in cmds.read() {
        match cmd {
            SessionCmd::NextMatch => {
                if let Some(game) = game.as_deref_mut() {
                    next_match(game, &local, &mut session);
                }
            }
            SessionCmd::Leave => {
                if game.is_some() && session.leaving == 0 {
                    session.leaving = 1;
                }
            }
            SessionCmd::Quit => {
                if let Some(game) = game.as_deref_mut() {
                    game.close();
                }
                exit.write(AppExit::Success);
            }
        }
    }
}

fn match_over(game: Option<ResMut<Game>>, mut session: ResMut<Session>, local: Res<LocalCmd>, time: Res<Time>) {
    let Some(mut game) = game else {
        session.over_for = None;
        return;
    };
    if !game.snap.over || session.leaving > 0 {
        session.over_for = None;
        return;
    }
    let t = session.over_for.unwrap_or(0.0) + time.delta_secs();
    session.over_for = Some(t);
    if t >= NEXT_MATCH_DELAY && game.is_local() {
        next_match(&mut game, &local, &mut session);
    }
}

/// Leaving takes two frames: first the game shows nothing (so the actors take their own
/// entities down) and the connection closes, then the game and the arena go.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn leave(
    mut commands: Commands,
    game: Option<ResMut<Game>>,
    mut session: ResMut<Session>,
    mut local: ResMut<LocalCmd>,
    mut mover: ResMut<Mover>,
    overlays: Option<ResMut<crate::desktop::Overlays>>,
    parts: Query<Entity, Or<(With<ArenaPart>, With<Scenery>)>>,
) {
    if session.leaving == 0 {
        return;
    }
    let Some(mut game) = game else {
        session.leaving = 0;
        return;
    };
    game.snap = Default::default();
    game.prev = Default::default();
    if session.leaving == 1 {
        game.close();
        session.leaving = 2;
        return;
    }
    info!("left the game");
    for e in &parts {
        commands.entity(e).despawn();
    }
    commands.remove_resource::<Game>();
    commands.remove_resource::<crate::Connecting>();
    commands.remove_resource::<crate::menu::Start>();
    commands.insert_resource(crate::menu::ShowMenu);
    let autopilot = local.autopilot;
    *local = LocalCmd { autopilot, ..default() };
    mover.valid = false;
    if let Some(mut o) = overlays {
        o.menu = false;
        o.end = false;
    }
    session.leaving = 0;
    session.over_for = None;
}

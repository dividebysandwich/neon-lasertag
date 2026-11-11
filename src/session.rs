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
//! team target (`target`). Megatarget mode in local games: both secondary targets hit at once
//! (`power`), and going to Red's doorway into the megatarget's room (`warp-door`), into the
//! room (`warp-room`), in front of the first turret (`warp-turret`) or into the first pocket
//! (`warp-pocket`); in front of the nearest mirror (`warp-mirror`) jump pad (`warp-pad`) or laser gate (`warp-gate`); turning the flat screen's view (`yaw=90`, `pitch=-20`, in degrees); holding the trigger
//! down for S seconds (`fire=S`); putting up a carried barrier (`barrier`); turret boxes: in
//! front of the nearest one (`warp-box`, `warp-box=M`), every player but the bots at the first
//! one (`warp-box-all`, hosting), clicking its puzzle (`box-hint=N`, `box-poke=J`, and
//! `box-click`, which works in a remote game too) or solving it at once (`solve-box`); consoles:
//! in front of one (`warp-console`, `warp-console=K`, `warp-console=vault`), at a light
//! bridge's end (`warp-bridge`), clicking the
//! nearest one's puzzle (`console-hint=N`), giving it a puzzle of a kind (`console-game=circuit`)
//! or solving it (`solve-console`); arena events (`blackout`, `overdrive`) and the last
//! minute's double points (`double`); freezing the game (`freeze`), the free camera
//! (`freecam`), its view's width (`fov=DEG`) and aperture (`dof=N`).

use bevy::prelude::*;
use lt_sim::bots::Difficulty;

use crate::arena::ArenaPart;
use crate::game::{Game, GameEvent, InputSet, LocalCmd, Mover, Source};
use crate::scenery::Scenery;

/// Seconds from the end of a local match to the next one.
pub const NEXT_MATCH_DELAY: f32 = 30.0;

#[derive(Resource, Default)]
pub struct Session {
    /// The bots' difficulty in this (local) game.
    pub difficulty: Difficulty,
    /// The arenas' size in this (local) game.
    pub arena: lt_sim::level::SizeChoice,
    /// What this (local) game is played for.
    pub mode: lt_sim::level::GameMode,
    /// How long ago the match ended (None while it runs).
    pub over_for: Option<f32>,
    /// Leaving: the frames of taking the game down done so far.
    leaving: u8,
}

impl Session {
    /// The rules for the next local match: the command line's, in an arena of the chosen size.
    pub fn rules(&self, args: &crate::Args) -> lt_sim::Rules {
        lt_sim::Rules {
            arena: self.arena.pick(rand::random()),
            ..args.rules(self.mode)
        }
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
    /// Play on the other team (`Game::switch_team`).
    SwitchTeam(lt_sim::Team),
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .add_message::<SessionCmd>()
            .add_systems(
                Update,
                (dev_script, run_commands, match_over)
                    .chain()
                    .before(InputSet),
            )
            // Not in `Update`: a group's `run_if(resource_exists::<Game>)` is checked once, so
            // removing the game between two of its systems would leave the later one without it.
            .add_systems(Last, (close_on_exit, leave));
    }
}

/// A bot plays the local player (`--autopilot`).
pub fn autopilot(game: &mut Game, seed: u64) {
    if let (Some(me), Source::Local { bots, .. }) = (game.me, &mut game.source) {
        bots.bots
            .insert(me, Box::new(lt_sim::bots::WanderBot::new(seed, 0.7)));
    }
}

/// `LT_DEV`: scripted actions for testing.
#[allow(clippy::too_many_arguments)]
/// Turns the flat screen's view to `yaw` (after moving the local player for testing).
fn face(w: &mut World, yaw: f32) {
    if let Some(mut look) = w.get_resource_mut::<crate::desktop::Look>() {
        look.yaw = yaw;
        look.pitch = 0.0;
    }
}

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
            .filter_map(|s| {
                s.split_once(':')
                    .and_then(|(t, a)| Some((t.trim().parse().ok()?, a.trim().to_string())))
            })
            .collect()
    });
    let now = time.elapsed_secs();
    let Some(k) = script.iter().position(|(t, _)| *t <= now) else {
        return;
    };
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
                events.write(crate::game::GameEvent(lt_sim::SimEvent::TargetHit {
                    target: t.id,
                    by: me,
                    pos: t.centre(),
                }));
            }
        }
        // A hit on the local player's own team's target, by an enemy (only shown and heard).
        "target-us" => {
            let Some(game) = game else { return };
            let Some(me) = game
                .me
                .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
            else {
                return;
            };
            let by = game
                .snap
                .players
                .iter()
                .find(|p| p.team != me.team)
                .map(|p| p.id);
            if let (Some(t), Some(by)) = (game.level.targets.iter().find(|t| t.team == me.team), by)
            {
                events.write(crate::game::GameEvent(lt_sim::SimEvent::TargetHit {
                    target: t.id,
                    by,
                    pos: t.centre(),
                }));
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
        // A hit on the local player by the nearest enemy (`hit`), or (only shown and heard, the
        // match goes on as it was) the local player's on them (`hit-them`) or tagging them out
        // (`tag-them`).
        "hit" | "hit-them" | "tag-them" => {
            let Some(game) = game else { return };
            let Some(me) = game
                .me
                .and_then(|me| game.snap.players.iter().find(|p| p.id == me))
            else {
                return;
            };
            let enemy = game
                .snap
                .players
                .iter()
                .filter(|p| p.team != me.team)
                .min_by(|a, b| {
                    a.feet
                        .distance(me.feet)
                        .total_cmp(&b.feet.distance(me.feet))
                });
            let Some(e) = enemy else { return };
            let event = match action.as_str() {
                "hit" => lt_sim::SimEvent::Hit {
                    player: me.id,
                    by: e.id,
                    pos: me.feet + me.head.pos,
                    hits: 1,
                },
                "hit-them" => lt_sim::SimEvent::Hit {
                    player: e.id,
                    by: me.id,
                    pos: e.feet + e.head.pos,
                    hits: 1,
                },
                _ => lt_sim::SimEvent::TaggedOut {
                    player: e.id,
                    by: me.id,
                    pos: e.feet,
                    respawn_in: game.rules.respawn_min,
                    how: lt_sim::TagHow::Blaster,
                },
            };
            events.write(crate::game::GameEvent(event));
        }
        // Local games: the nearest enemy tags the player out (`tag-me`).
        "tag-me" => {
            commands.queue(move |w: &mut World| {
                let Some(mut game) = w.get_resource_mut::<Game>() else {
                    return;
                };
                let Some(me) = game.me else { return };
                let Source::Local { sim, .. } = &mut game.source else {
                    return;
                };
                let Some(p) = sim.player(me).cloned() else {
                    return;
                };
                let by = sim
                    .players
                    .iter()
                    .filter(|o| o.team != p.team && o.alive())
                    .min_by(|a, b| a.feet.distance(p.feet).total_cmp(&b.feet.distance(p.feet)))
                    .map(|o| o.id);
                if let Some(by) = by {
                    sim.tag_out(me, by);
                }
            });
        }
        // Megatarget mode, local games: hit both secondary targets at once, switch the first
        // turret off, or go to Red's doorway, into the room, before the first turret or into the
        // first pocket.
        "power" | "turret-off" | "warp-door" | "warp-room" | "warp-turret" | "warp-pocket" => {
            let action = action.clone();
            commands.queue(move |w: &mut World| {
                let Some(mut game) = w.get_resource_mut::<Game>() else {
                    return;
                };
                let (Some(me), Some(m)) = (game.me, game.level.mega.clone()) else {
                    return;
                };
                let Source::Local { sim, .. } = &mut game.source else {
                    return;
                };
                let (s, t, p) = (m.shields[0], m.turrets[0], m.secondaries[0]);
                let yaw = match action.as_str() {
                    "power" => {
                        if let Some(p) = sim.players.iter_mut().find(|p| p.id == me) {
                            p.mega_hits = 3;
                            p.power = Some(sim.rules.power_time);
                        }
                        return;
                    }
                    // The first turret switched off.
                    "turret-off" => {
                        let off = sim.rules.turret_off;
                        if let Some(t) = sim.turrets.first_mut() {
                            t.off = off;
                        }
                        return;
                    }
                    "warp-door" => (s.outside + Vec3::Z * 3.0, 0.0),
                    "warp-room" => (s.inside + Vec3::new(1.5, 0.0, -0.2), 0.6),
                    "warp-turret" => (
                        Vec3::new(t.pos.x + 4.5, 0.0, t.pos.z + 1.5),
                        std::f32::consts::FRAC_PI_2 - 0.2,
                    ),
                    _ => (Vec3::new(p.centre().x - 1.6, 0.0, p.centre().z + 3.2), -0.5),
                };
                sim.teleport(me, yaw.0, yaw.1);
                face(w, yaw.1);
            });
        }
        // Local games, turret boxes: in front of the nearest one's puzzle (`warp-box`, or
        // `warp-box=M` metres from it), facing it; clicking the piece its puzzle's hint says
        // (`box-hint`, or `box-hint=N` N times), or a given piece (`box-poke=J`); solving it at
        // once (`solve-box`).
        a if a == "warp-box"
            || a.starts_with("warp-box=")
            || a.starts_with("box-hint")
            || a.starts_with("box-poke=")
            || a == "solve-box" =>
        {
            let a = a.to_string();
            commands.queue(move |w: &mut World| {
                let yaw =
                    {
                        let Some(mut game) = w.get_resource_mut::<Game>() else {
                            return;
                        };
                        let Some(me) = game.me else { return };
                        let Source::Local { sim, .. } = &mut game.source else {
                            return;
                        };
                        let Some(at) = sim.player(me).map(|p| p.feet) else {
                            return;
                        };
                        let Some((k, b)) =
                            sim.level.boxes.iter().copied().enumerate().min_by(|a, b| {
                                a.1.pos.distance(at).total_cmp(&b.1.pos.distance(at))
                            })
                        else {
                            return;
                        };
                        let arg = |p: &str| a.strip_prefix(p).and_then(|m| m.parse::<f32>().ok());
                        if a == "solve-box" {
                            sim.dev_poke(me, k, None, true);
                            return;
                        }
                        if let Some(j) = arg("box-poke=") {
                            sim.dev_poke(me, k, Some(j as usize), false);
                            return;
                        }
                        if a.starts_with("box-hint") {
                            for _ in 0..arg("box-hint=").unwrap_or(1.0) as usize {
                                sim.dev_poke(me, k, None, false);
                            }
                            return;
                        }
                        // The side towards the player.
                        let n = if at.z >= b.pos.z { 1.0 } else { -1.0 };
                        let away = arg("warp-box=").unwrap_or(1.0);
                        let feet = b.pos + Vec3::Z * n * (lt_sim::level::BOX_HALF + away);
                        let yaw = if n > 0.0 { 0.0 } else { std::f32::consts::PI };
                        sim.teleport(me, feet, yaw);
                        yaw
                    };
                face(w, yaw);
            });
        }
        // Local games, consoles: in front of console K's first puzzle (`warp-console=K`), of
        // the first one of a kind (`warp-console=vault`, ...), or of the nearest one
        // (`warp-console`), facing it; clicking the nearest one's hint (`console-hint`, or
        // `console-hint=N` N times) or solving it at once (`solve-console`).
        a if a == "warp-console"
            || a == "warp-bridge"
            || a.starts_with("warp-console=")
            || a.starts_with("console-hint")
            || a.starts_with("console-game=")
            || a == "solve-console" =>
        {
            let a = a.to_string();
            commands.queue(move |w: &mut World| {
                let yaw = {
                    let Some(mut game) = w.get_resource_mut::<Game>() else {
                        return;
                    };
                    let Some(me) = game.me else { return };
                    let Source::Local { sim, .. } = &mut game.source else {
                        return;
                    };
                    let Some(at) = sim.player(me).map(|p| p.feet) else {
                        return;
                    };
                    let defs = sim.console_defs().to_vec();
                    let nearest = defs
                        .iter()
                        .enumerate()
                        .min_by(|a, b| a.1.pos.distance(at).total_cmp(&b.1.pos.distance(at)))
                        .map(|(k, _)| k);
                    let Some(near) = nearest else { return };
                    if a == "solve-console" {
                        sim.dev_console_poke(me, near, None, true);
                        return;
                    }
                    if let Some(kind) = a.strip_prefix("console-game=") {
                        if let Some(kind) = lt_sim::minigame::MinigameKind::ALL
                            .into_iter()
                            .find(|k| k.id() == kind)
                        {
                            sim.dev_console_game(near, kind);
                        }
                        return;
                    }
                    if a.starts_with("console-hint") {
                        let n = a
                            .strip_prefix("console-hint=")
                            .and_then(|n| n.parse::<usize>().ok())
                            .unwrap_or(1);
                        for _ in 0..n {
                            sim.dev_console_poke(me, near, None, false);
                        }
                        return;
                    }
                    if a == "warp-bridge" {
                        // Behind the nearest light bridge's end, on the upper floor, facing over it.
                        let Some(b) = sim
                            .level
                            .bridges
                            .iter()
                            .min_by(|a, b| a.mid().distance(at).total_cmp(&b.mid().distance(at)))
                            .copied()
                        else {
                            return;
                        };
                        let dir = (b.ends[1] - b.ends[0]).normalize_or(Vec3::X);
                        let yaw = lt_sim::sim::yaw_of(dir);
                        sim.teleport(me, b.ends[0] - dir * 1.0, yaw);
                        yaw
                    } else {
                        let k = match a.strip_prefix("warp-console=") {
                            Some(arg) => match arg.parse::<usize>() {
                                Ok(k) => k,
                                Err(_) => match defs.iter().position(|d| d.kind.id() == arg) {
                                    Some(k) => k,
                                    None => return,
                                },
                            },
                            None => near,
                        };
                        let Some(d) = defs.get(k) else { return };
                        info!(
                            "dev: console {k} ({}) at {:.1}, its bridge {:?}",
                            d.kind.name(),
                            d.pos,
                            lt_sim::console::bridge_for(&sim.level, d)
                        );
                        let f = d.faces[0];
                        let mut feet = f.centre + f.normal * 1.0;
                        feet.y = d.floor;
                        let yaw = lt_sim::sim::yaw_of(-f.normal);
                        sim.teleport(me, feet, yaw);
                        yaw
                    }
                };
                face(w, yaw);
            });
        }
        // Any game (a remote one too): one click on the nearest turret box's puzzle, on the
        // piece its hint says, sent as a player's click is (`box-click`); nothing once the box
        // is the player's team's.
        "box-click" => {
            commands.queue(|w: &mut World| {
                let Some(game) = w.get_resource::<Game>() else {
                    return;
                };
                let Some(me) = game
                    .me
                    .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
                else {
                    return;
                };
                let Some((k, s)) = game
                    .level
                    .boxes
                    .iter()
                    .zip(&game.snap.boxes)
                    .enumerate()
                    .min_by(|a, b| {
                        let d = |b: &lt_sim::level::TurretBox| b.pos.distance(me.feet);
                        d(a.1.0).total_cmp(&d(b.1.0))
                    })
                    .map(|(k, (_, s))| (k, s))
                else {
                    return;
                };
                let ours = if s.phase == lt_sim::sim::BoxPhase::Closing {
                    s.next == Some(me.team)
                } else {
                    s.owner == Some(me.team)
                };
                if ours {
                    return;
                }
                let Some(j) = lt_sim::puzzle::Puzzle::generate(s.seed).hint(&s.pieces) else {
                    return;
                };
                info!(
                    "dev: clicking piece {j} of turret box {k} from {:.1}",
                    me.feet
                );
                if let Some(mut local) = w.get_resource_mut::<LocalCmd>() {
                    local.cmd.pokes = local.cmd.pokes.wrapping_add(1);
                    local.cmd.poke = (k as u8, j as u8);
                }
            });
        }
        // Hosting: every player who isn't a bot to the first turret box, Red in front of its
        // +z side and Blue of its -z side, facing it (`warp-box-all`).
        "warp-box-all" => {
            commands.queue(|w: &mut World| {
                let yaw = {
                    let Some(mut game) = w.get_resource_mut::<Game>() else {
                        return;
                    };
                    let me = game.me;
                    let Source::Local { sim, .. } = &mut game.source else {
                        return;
                    };
                    let Some(b) = sim.level.boxes.first().copied() else {
                        return;
                    };
                    let people: Vec<(lt_sim::PlayerId, lt_sim::Team)> = sim
                        .players
                        .iter()
                        .filter(|p| !p.bot)
                        .map(|p| (p.id, p.team))
                        .collect();
                    let mut mine = 0.0;
                    for (id, team) in people {
                        let n = if team == lt_sim::Team::Red { 1.0 } else { -1.0 };
                        let feet = b.pos + Vec3::Z * n * (lt_sim::level::BOX_HALF + 1.0);
                        let yaw = if n > 0.0 { 0.0 } else { std::f32::consts::PI };
                        info!("dev: player {id} ({team:?}) to {feet:.1}, at turret box 0");
                        sim.teleport(id, feet, yaw);
                        if Some(id) == me {
                            mine = yaw;
                        }
                    }
                    mine
                };
                face(w, yaw);
            });
        }
        // Local games: in front of the nearest mirror, facing it at an angle (`warp-mirror`).
        // Local games: a little way (`warp-pad=M`: M metres) before the nearest jump pad,
        // facing the way it throws.
        a if a == "warp-pad" || a.starts_with("warp-pad=") => {
            let back: f32 = a
                .strip_prefix("warp-pad=")
                .and_then(|m| m.parse().ok())
                .unwrap_or(1.0);
            commands.queue(move |w: &mut World| {
                let yaw = {
                    let Some(mut game) = w.get_resource_mut::<Game>() else {
                        return;
                    };
                    let Some(me) = game.me else { return };
                    let Source::Local { sim, .. } = &mut game.source else {
                        return;
                    };
                    let Some(at) = sim.player(me).map(|p| p.feet) else {
                        return;
                    };
                    let Some(p) = sim
                        .level
                        .pads
                        .iter()
                        .min_by(|a, b| a.pos.distance(at).total_cmp(&b.pos.distance(at)))
                        .copied()
                    else {
                        return;
                    };
                    let dir = (p.land - p.pos).with_y(0.0).normalize_or(Vec3::NEG_Z);
                    let feet = p.pos - dir * back;
                    let yaw = lt_sim::sim::yaw_of(dir);
                    sim.teleport(me, feet, yaw);
                    yaw
                };
                face(w, yaw);
            });
        }
        // Local games: in the cell before the nearest laser gate, facing it.
        "warp-gate" => {
            commands.queue(move |w: &mut World| {
                let yaw = {
                    let Some(mut game) = w.get_resource_mut::<Game>() else {
                        return;
                    };
                    let Some(me) = game.me else { return };
                    let Source::Local { sim, .. } = &mut game.source else {
                        return;
                    };
                    let Some(at) = sim.player(me).map(|p| p.feet) else {
                        return;
                    };
                    let Some(g) = sim
                        .level
                        .gates
                        .iter()
                        .min_by(|a, b| a.centre().distance(at).total_cmp(&b.centre().distance(at)))
                        .copied()
                    else {
                        return;
                    };
                    // (The side without a jump pad.)
                    let nav = &sim.level.nav;
                    let (a, b) = (nav.pos[g.link.0 as usize], nav.pos[g.link.1 as usize]);
                    let feet = if sim.level.pad_at(a).is_some() { b } else { a };
                    let yaw = lt_sim::sim::yaw_of(g.centre().with_y(feet.y) - feet);
                    sim.teleport(me, feet, yaw);
                    yaw
                };
                face(w, yaw);
            });
        }
        "warp-mirror" => {
            commands.queue(move |w: &mut World| {
                let yaw = {
                    let Some(mut game) = w.get_resource_mut::<Game>() else {
                        return;
                    };
                    let Some(me) = game.me else { return };
                    let Source::Local { sim, .. } = &mut game.source else {
                        return;
                    };
                    let Some(at) = sim.player(me).map(|p| p.feet) else {
                        return;
                    };
                    let Some(m) = sim
                        .level
                        .mirrors
                        .iter()
                        .min_by(|a, b| a.centre().distance(at).total_cmp(&b.centre().distance(at)))
                        .copied()
                    else {
                        return;
                    };
                    let along = Vec3::Y.cross(m.normal);
                    let c = m.centre();
                    let feet = Vec3::new(c.x, m.min.y - 0.2, c.z) + m.normal * 1.8 + along * 0.6;
                    let yaw = lt_sim::sim::yaw_of(c - feet);
                    sim.teleport(me, feet, yaw);
                    yaw
                };
                face(w, yaw);
            });
        }
        // Local games: `give-KIND` hands the local player a pickup's contents (`give-all-KIND`:
        // everyone; `take-KIND`: as if walked over one, announced), `drop-KIND` lays one 2.5 m
        // in front of them (KIND: grenade, rapid, ...).
        a if a.starts_with("give-") || a.starts_with("drop-") || a.starts_with("take-") => {
            let all = a.starts_with("give-all-");
            let Some(kind) = lt_sim::PickupKind::parse(&a[if all { 9 } else { 5 }..]) else {
                warn!("LT_DEV: unknown pickup in {a}");
                return;
            };
            let take = a.starts_with("take-");
            let give = take || a.starts_with("give-");
            commands.queue(move |w: &mut World| {
                let Some(mut game) = w.get_resource_mut::<Game>() else {
                    return;
                };
                let Some(me) = game.me else { return };
                let Source::Local { sim, .. } = &mut game.source else {
                    return;
                };
                if all {
                    let ids: Vec<_> = sim.players.iter().map(|p| p.id).collect();
                    for id in ids {
                        sim.give(id, kind);
                    }
                } else if give {
                    sim.give(me, kind);
                    if take && let Some(p) = sim.player(me) {
                        let pos = p.feet;
                        w.write_message(GameEvent(lt_sim::SimEvent::PickedUp {
                            id: u32::MAX,
                            player: me,
                            pos,
                            kind,
                        }));
                    }
                } else if let Some(p) = sim.player(me) {
                    let ahead = Quat::from_rotation_y(p.cmd.yaw) * Vec3::NEG_Z * 2.5;
                    let pos = p.feet + ahead + Vec3::Y * lt_sim::sim::PICKUP_HEIGHT;
                    sim.drop_pickup(kind, pos);
                }
            });
        }
        // Local games: the nearest other player stands 4 m in front of the local player, facing
        // them (`summon`, or `summon=M` for M metres).
        a if a == "summon" || a.starts_with("summon=") => {
            let dist: f32 = a
                .strip_prefix("summon=")
                .and_then(|v| v.parse().ok())
                .unwrap_or(4.0);
            commands.queue(move |w: &mut World| {
                let Some(mut game) = w.get_resource_mut::<Game>() else {
                    return;
                };
                let Some(me) = game.me else { return };
                let Source::Local { sim, .. } = &mut game.source else {
                    return;
                };
                let Some((feet, yaw)) = sim.player(me).map(|p| (p.feet, p.cmd.yaw)) else {
                    return;
                };
                let other = sim
                    .players
                    .iter()
                    .filter(|p| p.id != me && p.alive())
                    .min_by(|a, b| a.feet.distance(feet).total_cmp(&b.feet.distance(feet)))
                    .map(|p| p.id);
                if let Some(id) = other {
                    let at = feet + Quat::from_rotation_y(yaw) * Vec3::NEG_Z * dist;
                    sim.teleport(id, at, yaw + std::f32::consts::PI);
                }
            });
        }
        // Local games: the player's blaster down to 5 shots (`drain`).
        "drain" => commands.queue(|w: &mut World| {
            let Some(mut game) = w.get_resource_mut::<Game>() else {
                return;
            };
            let Some(me) = game.me else { return };
            let Source::Local { sim, .. } = &mut game.source else {
                return;
            };
            if let Some(p) = sim.players.iter_mut().find(|p| p.id == me) {
                p.ammo = 5;
            }
        }),
        // Local games: the nearest enemy hit once and put 5 m in front of the player (`hurt`).
        "hurt" => commands.queue(|w: &mut World| {
            let Some(mut game) = w.get_resource_mut::<Game>() else {
                return;
            };
            let Some(me) = game.me else { return };
            let Source::Local { sim, .. } = &mut game.source else {
                return;
            };
            let Some((feet, yaw, team)) = sim.player(me).map(|p| (p.feet, p.cmd.yaw, p.team))
            else {
                return;
            };
            let other = sim
                .players
                .iter()
                .filter(|p| p.team != team && p.alive())
                .min_by(|a, b| a.feet.distance(feet).total_cmp(&b.feet.distance(feet)))
                .map(|p| p.id);
            if let Some(id) = other {
                let at = feet + Quat::from_rotation_y(yaw) * Vec3::NEG_Z * 5.0;
                sim.teleport(id, at, yaw + std::f32::consts::PI);
                if let Some(p) = sim.players.iter_mut().find(|p| p.id == id) {
                    p.hits = 1;
                }
            }
        }),
        // Local games: everyone's shield on at once.
        "shields-up" => commands.queue(|w: &mut World| {
            let Some(mut game) = w.get_resource_mut::<Game>() else {
                return;
            };
            let Source::Local { sim, .. } = &mut game.source else {
                return;
            };
            let t = sim.rules.shield_time;
            for p in sim.players.iter_mut().filter(|p| p.alive()) {
                p.shield = t;
            }
        }),
        // Local games: start an arena event now (`blackout`, `overdrive`, `storm`...), or go to
        // the last minute's double points (`double`).
        a if a == "double" || lt_sim::ArenaEvent::parse(a).is_some() => {
            let action = action.clone();
            commands.queue(move |w: &mut World| {
                let Some(mut game) = w.get_resource_mut::<Game>() else {
                    return;
                };
                let Source::Local { sim, .. } = &mut game.source else {
                    return;
                };
                match lt_sim::ArenaEvent::parse(&action) {
                    Some(kind) => sim.start_event(kind),
                    None => sim.time_left = sim.time_left.min(sim.rules.double_last + 0.5),
                }
            });
        }
        // Put up a carried barrier (`barrier`).
        "barrier" => commands.queue(|w: &mut World| {
            if let Some(mut local) = w.get_resource_mut::<LocalCmd>() {
                local.dev_barrier = true;
            }
        }),
        // Hold the trigger down for S seconds (`fire=S`).
        a if a.starts_with("fire=") => {
            let secs: f32 = a[5..].parse().unwrap_or(0.1);
            commands.queue(move |w: &mut World| {
                if let Some(mut local) = w.get_resource_mut::<LocalCmd>() {
                    local.dev_fire = secs;
                }
            });
        }
        // The flat screen's view: turn to `yaw=DEG` (0: facing -z, 90: -x) or `pitch=DEG`.
        a if a.starts_with("yaw=") || a.starts_with("pitch=") => {
            let (key, v) = a.split_once('=').unwrap_or_default();
            let Ok(deg) = v.parse::<f32>() else { return };
            let yaw = key == "yaw";
            commands.queue(move |w: &mut World| {
                if let Some(mut look) = w.get_resource_mut::<crate::desktop::Look>() {
                    if yaw {
                        look.yaw = deg.to_radians();
                    } else {
                        look.pitch = deg.to_radians();
                    }
                }
            });
        }
        // Freezing an offline game (as F11; the script waits while frozen), and the free camera
        // (as F12), its view's width (`fov=DEG`) and aperture (`dof=N`, a step of the wheel each).
        "freeze" => commands.queue(|w: &mut World| {
            if let Some(mut f) = w.get_resource_mut::<crate::freeze::Frozen>() {
                f.1 = true;
            }
        }),
        "freecam" => commands.queue(|w: &mut World| {
            if let Some(mut f) = w.get_resource_mut::<crate::freecam::FreeCam>() {
                f.request = true;
            }
        }),
        a if a.starts_with("fov=") || a.starts_with("dof=") => {
            let (key, v) = a.split_once('=').unwrap_or_default();
            let Ok(n) = v.parse::<f32>() else { return };
            let fov = key == "fov";
            commands.queue(move |w: &mut World| {
                if let Some(mut f) = w.get_resource_mut::<crate::freecam::FreeCam>() {
                    if fov {
                        f.set_fov(n);
                    } else {
                        f.set_stop(n as usize);
                    }
                }
            });
        }
        a if a.starts_with("play") => {
            let d = a
                .strip_prefix("play-")
                .and_then(Difficulty::parse)
                .unwrap_or_default();
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
    info!(
        "next match: arena {seed} ({})\n{}",
        game.rules.arena.name(),
        lt_sim::level::ascii(&game.level)
    );
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
            SessionCmd::SwitchTeam(team) => {
                if let Some(game) = game.as_deref_mut() {
                    game.switch_team(*team);
                }
            }
        }
    }
}

fn match_over(
    game: Option<ResMut<Game>>,
    mut session: ResMut<Session>,
    local: Res<LocalCmd>,
    time: Res<Time>,
) {
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

/// Quitting however (the window closed too): the others are told, so this player leaves their
/// field (or, hosting, they go back to their menus).
fn close_on_exit(mut exits: MessageReader<AppExit>, game: Option<ResMut<Game>>) {
    if exits.read().next().is_some()
        && let Some(mut game) = game
    {
        game.close();
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
    *local = LocalCmd {
        autopilot,
        ..default()
    };
    mover.valid = false;
    if let Some(mut o) = overlays {
        o.menu = false;
        o.end = false;
    }
    session.leaving = 0;
    session.over_for = None;
}

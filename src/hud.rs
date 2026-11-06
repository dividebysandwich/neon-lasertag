//! The flat-screen HUD: scores and clock, crosshair, hits left, grenade, respawn countdown,
//! the last few tag-outs, and where hits come from (red arcs at the edge of the screen,
//! pointing at the shooter).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use lt_sim::{PlayerId, SimEvent, Team};

use crate::desktop::{MainCamera, Overlays};
use crate::game::{Game, GameEvent};
use crate::session::Session;

#[derive(Component)]
struct ScoreText(Team);
#[derive(Component)]
struct ClockText;
#[derive(Component)]
struct CenterText;
#[derive(Component)]
struct StatusText;
#[derive(Component)]
struct FeedText;
#[derive(Component)]
struct HitFlash;
/// One of the three hits-left lights.
#[derive(Component)]
struct Pip(u8);
/// The HUD's top level nodes (hidden without a game and on the end screen).
#[derive(Component)]
struct HudRoot;
#[derive(Component)]
struct DifficultyText;
/// A piece of hit arc `mark` (of `MARKS`), `piece` (of `PIECES`).
#[derive(Component)]
struct ArcPiece {
    mark: usize,
    piece: usize,
}

const MARKS: usize = 4;
const PIECES: usize = 9;
/// Seconds a hit arc shows.
const MARK_LIFE: f32 = 1.1;

/// Where a hit came from: the shooter (followed while it moves) or the place it was.
#[derive(Clone, Copy)]
struct HitMark {
    by: Option<PlayerId>,
    from: Vec3,
    life: f32,
}

#[derive(Resource, Default)]
struct Feed {
    lines: Vec<(String, f32)>,
    flash: f32,
    marks: Vec<HitMark>,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Feed>()
            .add_systems(Startup, spawn)
            .add_systems(Update, reset.run_if(resource_added::<Game>))
            .add_systems(
                Update,
                (events, update, hit_arcs)
                    .chain()
                    .run_if(resource_exists::<Game>),
            )
            .add_systems(Update, shown);
    }
}

fn reset(mut feed: ResMut<Feed>) {
    *feed = Feed::default();
}

/// Hidden in the start menu and on the end screen.
fn shown(
    game: Option<Res<Game>>,
    overlays: Res<Overlays>,
    mut q: Query<&mut Visibility, With<HudRoot>>,
) {
    let on = game.is_some() && !overlays.end;
    let want = if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut q {
        if *v != want {
            *v = want;
        }
    }
}

/// Where the shot that hit `me` came from: the shooter's head (None for one's own grenade).
pub fn shooter_pos(game: &Game, by: PlayerId) -> Option<Vec3> {
    if Some(by) == game.me {
        return None;
    }
    let p = game.snap.players.iter().find(|p| p.id == by)?;
    Some(p.feet + p.head.pos)
}

fn text(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn spawn(mut commands: Commands) {
    let red = Color::srgb(1.0, 0.25, 0.25);
    let blue = Color::srgb(0.35, 0.6, 1.0);
    // Red flash when hit.
    commands.spawn((
        HudRoot,
        HitFlash,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 0.0, 0.0, 0.0)),
    ));
    // The hit arcs.
    for mark in 0..MARKS {
        for piece in 0..PIECES {
            commands.spawn((
                HudRoot,
                ArcPiece { mark, piece },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(10.0),
                    height: Val::Px(12.0),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                UiTransform::IDENTITY,
            ));
        }
    }
    // Top: RED score, clock, BLUE score; the bots' difficulty below.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(12.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(40.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((ScoreText(Team::Red), text("RED 0", 34.0, red)));
            p.spawn((ClockText, text("10:00", 34.0, Color::WHITE)));
            p.spawn((ScoreText(Team::Blue), text("BLUE 0", 34.0, blue)));
        });
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(54.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((DifficultyText, text("", 15.0, Color::srgb(0.6, 0.6, 0.7))));
        });
    // Crosshair.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                Node {
                    width: Val::Px(6.0),
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            ));
        });
    // Middle: respawn countdown and messages.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(30.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                CenterText,
                text("", 44.0, Color::WHITE),
                TextLayout::justify(Justify::Center),
            ));
        });
    // Bottom left: hits left and the grenade.
    commands.spawn((
        HudRoot,
        StatusText,
        text("", 28.0, Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            bottom: Val::Px(16.0),
            ..default()
        },
    ));
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                bottom: Val::Px(124.0),
                column_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|p| {
            for k in 1..=3 {
                p.spawn((
                    Pip(k),
                    Node {
                        width: Val::Px(34.0),
                        height: Val::Px(14.0),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                ));
            }
        });
    // Right: the feed.
    commands.spawn((
        HudRoot,
        FeedText,
        text("", 18.0, Color::srgb(0.85, 0.85, 0.9)),
        TextLayout::justify(Justify::Right),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(20.0),
            top: Val::Px(70.0),
            ..default()
        },
    ));
}

fn events(mut events: MessageReader<GameEvent>, game: Res<Game>, mut feed: ResMut<Feed>) {
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::TaggedOut { player, by, .. } => {
                let line = if player == by {
                    format!("{} tagged themself out", game.name_of(player))
                } else {
                    format!("{} tagged out {}", game.name_of(by), game.name_of(player))
                };
                feed.lines.push((line, 6.0));
            }
            SimEvent::TargetHit { by, target, .. } => {
                let t = game
                    .level
                    .targets
                    .iter()
                    .find(|t| t.id == target)
                    .map(|t| t.team);
                feed.lines.push((
                    format!(
                        "{} hit the {:?} target +100",
                        game.name_of(by),
                        t.unwrap_or(Team::Red)
                    ),
                    3.0,
                ));
            }
            SimEvent::Hit {
                player, by, pos, ..
            } if Some(player) == game.me => {
                feed.flash = 0.35;
                let from = shooter_pos(&game, by);
                // Several hits from one shooter: one arc.
                feed.marks.retain(|m| m.by != Some(by) || from.is_none());
                if feed.marks.len() >= MARKS {
                    feed.marks.remove(0);
                }
                feed.marks.push(HitMark {
                    by: from.map(|_| by),
                    from: from.unwrap_or(pos),
                    life: MARK_LIFE,
                });
            }
            SimEvent::MatchOver { scores } => {
                let w = if scores[0] > scores[1] {
                    "RED WINS"
                } else if scores[1] > scores[0] {
                    "BLUE WINS"
                } else {
                    "DRAW"
                };
                feed.lines.push((w.to_string(), 15.0));
            }
            _ => {}
        }
    }
    if feed.lines.len() > 6 {
        let n = feed.lines.len() - 6;
        feed.lines.drain(..n);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update(
    game: Res<Game>,
    time: Res<Time>,
    mut feed: ResMut<Feed>,
    mut scores: Query<
        (&ScoreText, &mut Text),
        (
            Without<ClockText>,
            Without<CenterText>,
            Without<StatusText>,
            Without<FeedText>,
        ),
    >,
    mut clock: Query<
        &mut Text,
        (
            With<ClockText>,
            Without<CenterText>,
            Without<StatusText>,
            Without<FeedText>,
        ),
    >,
    mut center: Query<&mut Text, (With<CenterText>, Without<StatusText>, Without<FeedText>)>,
    mut status: Query<&mut Text, (With<StatusText>, Without<FeedText>)>,
    mut feed_text: Query<&mut Text, With<FeedText>>,
    mut flash: Query<&mut BackgroundColor, (With<HitFlash>, Without<Pip>)>,
    mut pips: Query<(&Pip, &mut BackgroundColor), Without<HitFlash>>,
    mut difficulty: Query<
        &mut Text,
        (
            With<DifficultyText>,
            Without<ScoreText>,
            Without<ClockText>,
            Without<CenterText>,
            Without<StatusText>,
            Without<FeedText>,
        ),
    >,
    session: Res<Session>,
) {
    if let Ok(mut t) = difficulty.single_mut() {
        let want = if game.is_local() {
            format!("bots: {}", session.difficulty.name())
        } else {
            String::new()
        };
        if t.0 != want {
            t.0 = want;
        }
    }
    let dt = time.delta_secs();
    for (s, mut t) in &mut scores {
        let name = if s.0 == Team::Red { "RED" } else { "BLUE" };
        t.0 = format!("{name} {}", game.snap.scores[s.0.index()]);
    }
    if let Ok(mut t) = clock.single_mut() {
        let s = game.snap.time_left.max(0.0) as u32;
        t.0 = format!("{}:{:02}", s / 60, s % 60);
    }
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    if let Ok(mut t) = center.single_mut() {
        t.0 = match me {
            _ if !game.status.is_empty() => game.status.clone(),
            _ if game.snap.over => String::new(),
            Some(p) if p.respawn.is_some() => {
                format!("TAGGED OUT\nback in {:.0}", p.respawn.unwrap().ceil())
            }
            Some(p) if p.cooldown > 0.0 => format!("HIT! blaster down {:.1}", p.cooldown),
            Some(p) if p.ammo == 0 => {
                "OUT OF AMMO\nrefill at your base or a white station".to_string()
            }
            _ => String::new(),
        };
    }
    if let Ok(mut t) = status.single_mut()
        && let Some(p) = me
    {
        let gren = if p.grenades > 0 {
            "grenade ready (RMB/G)"
        } else {
            "no grenade: pick one up"
        };
        let team = if p.team == Team::Red { "RED" } else { "BLUE" };
        let reload = if p.reloading { "  refilling" } else { "" };
        t.0 = format!(
            "{team}  score {}\nammo {} / {}{reload}\n{gren}",
            p.score, p.ammo, game.rules.ammo
        );
    }
    if let Some(p) = me {
        let left = 3u8.saturating_sub(p.hits);
        let blink = p.cooldown <= 0.0 || (time.elapsed_secs() * 8.0).fract() < 0.5;
        let c = if p.team == Team::Red {
            Color::srgb(1.0, 0.2, 0.2)
        } else {
            Color::srgb(0.3, 0.55, 1.0)
        };
        for (pip, mut bg) in &mut pips {
            bg.0 = if pip.0 <= left && blink {
                c
            } else {
                Color::srgba(0.3, 0.3, 0.3, 0.5)
            };
        }
    }
    for l in feed.lines.iter_mut() {
        l.1 -= dt;
    }
    feed.lines.retain(|l| l.1 > 0.0);
    if let Ok(mut t) = feed_text.single_mut() {
        t.0 = feed
            .lines
            .iter()
            .map(|l| l.0.as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }
    feed.flash = (feed.flash - dt).max(0.0);
    if let Ok(mut b) = flash.single_mut() {
        b.0 = Color::srgba(1.0, 0.0, 0.0, feed.flash * 0.3);
    }
}

/// The hit arcs: at the edge of a circle around the crosshair, towards the shooter as seen
/// from above (ahead: top, behind: bottom), fading.
fn hit_arcs(
    game: Res<Game>,
    time: Res<Time>,
    mut feed: ResMut<Feed>,
    cam: Query<&Transform, With<MainCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut pieces: Query<(&ArcPiece, &mut Node, &mut BackgroundColor, &mut UiTransform)>,
) {
    let dt = time.delta_secs();
    for m in feed.marks.iter_mut() {
        m.life -= dt;
        if let Some(p) =
            m.by.and_then(|by| game.snap.players.iter().find(|p| p.id == by))
        {
            m.from = p.feet + p.head.pos;
        }
    }
    feed.marks.retain(|m| m.life > 0.0);
    let (Ok(cam), Ok(window)) = (cam.single(), window.single()) else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    let radius = 0.36 * size.min_element();
    let fwd = cam.rotation * Vec3::NEG_Z;
    let right = cam.rotation * Vec3::X;
    let me = cam.translation;
    let span = 0.55f32;
    let step = span / (PIECES - 1) as f32;
    for (piece, mut node, mut bg, mut tr) in &mut pieces {
        let Some(m) = feed.marks.get(piece.mark) else {
            if bg.0 != Color::NONE {
                bg.0 = Color::NONE;
            }
            continue;
        };
        let d = m.from - me;
        let d = Vec2::new(
            d.dot(right),
            d.dot(Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero()),
        );
        let theta = d.x.atan2(d.y);
        let k = piece.piece as f32 - (PIECES - 1) as f32 / 2.0;
        let a = theta + k * step;
        let len = radius * step * 1.15;
        let thick = 14.0 - k.abs() * 1.2;
        let c = size / 2.0 + radius * Vec2::new(a.sin(), -a.cos());
        node.width = Val::Px(len);
        node.height = Val::Px(thick);
        node.left = Val::Px(c.x - len / 2.0);
        node.top = Val::Px(c.y - thick / 2.0);
        tr.rotation = Rot2::radians(a);
        let fade = (m.life / MARK_LIFE).clamp(0.0, 1.0);
        let edge = 1.0 - (k.abs() / ((PIECES as f32) / 2.0)).powi(2);
        bg.0 = Color::srgba(1.0, 0.08, 0.05, 0.9 * fade.sqrt() * edge);
    }
}

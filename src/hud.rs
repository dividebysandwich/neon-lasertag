//! The flat-screen HUD: scores and clock, crosshair, hits left, grenade, respawn countdown,
//! the last few tag-outs, and where hits come from (red arcs at the edge of the screen,
//! pointing at the shooter).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use lt_sim::{PlayerId, SimEvent, Team};

use crate::desktop::{MainCamera, Overlays};
use crate::game::{Game, GameEvent};
use crate::neonui;
use crate::session::Session;

#[derive(Component)]
struct ScoreText(Team);
#[derive(Component)]
struct ClockText;
/// Under the clock: a blackout's seconds left, and "2X POINTS" while points count double.
#[derive(Component)]
struct DoubleText;

/// The big line announcing an arena event (BLACKOUT, OVERDRIVE, DOUBLE POINTS).
#[derive(Component, Default)]
struct EventBanner;
/// The line under the banner saying what the event does.
#[derive(Component)]
struct EventBannerSub;
#[derive(Component)]
struct CenterText;
/// The bottom left panel (framed in the local player's team colour).
#[derive(Component)]
struct StatusPanel;
/// A line of the bottom left panel.
#[derive(Component, Clone, Copy, PartialEq)]
enum StatusPart {
    Team,
    Score,
    Ammo,
    Grenade,
}
/// The ammo bar's filled part.
#[derive(Component)]
struct AmmoFill;
/// Line `0` of the feed (a pill), and its text.
#[derive(Component)]
struct FeedLine(usize);
#[derive(Component)]
struct FeedLineText(usize);
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
/// Megatarget mode: the local player's progress (bottom middle).
#[derive(Component)]
struct MegaText;
/// The frame around `MegaText` (hidden outside Megatarget mode).
#[derive(Component)]
struct MegaBox;
/// The local player's powerups (bottom left, above the hit lights).
#[derive(Component)]
struct PowerText;
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
/// Lines the feed shows at most.
const FEED_LINES: usize = 6;

/// Where a hit came from: the shooter (followed while it moves) or the place it was.
#[derive(Clone, Copy)]
struct HitMark {
    by: Option<PlayerId>,
    from: Vec3,
    life: f32,
}

#[derive(Resource, Default)]
struct Feed {
    /// Each line, its colour and the seconds it has left.
    lines: Vec<(String, Color, f32)>,
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
                (
                    events,
                    update,
                    status_panel,
                    feed_lines,
                    hit_arcs,
                    mega_status,
                    powerup_status,
                    event_banner,
                )
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
    if by == lt_sim::HAZARD {
        // A turret: the nearest switched on.
        let me = game
            .me
            .and_then(|id| game.snap.players.iter().find(|p| p.id == id))?;
        let m = game.level.mega.as_ref()?;
        return m
            .turrets
            .iter()
            .zip(&game.snap.turrets)
            .filter(|(_, st)| st.off <= 0.0)
            .map(|(t, _)| t.pos)
            .min_by(|a, b| a.distance(me.feet).total_cmp(&b.distance(me.feet)));
    }
    let p = game.snap.players.iter().find(|p| p.id == by)?;
    Some(p.feet + p.head.pos)
}

/// Text in the HUD: the menus' font, with a dark outline-ish shadow to read over the arena.
fn text(s: &str, size: f32, color: Color, weight: u16) -> impl Bundle {
    (
        neonui::text_w(s, size, color, weight),
        TextShadow {
            offset: Vec2::new(0.0, 2.0),
            color: Color::srgba(0.0, 0.0, 0.02, 0.85),
        },
    )
}

fn glow(color: Color, alpha: f32, blur: f32) -> BoxShadow {
    BoxShadow::new(
        color.with_alpha(alpha),
        Val::Px(0.0),
        Val::Px(0.0),
        Val::Px(0.0),
        Val::Px(blur),
    )
}

/// A small framed box glowing in `color`, its children in a column, `pad` px inside.
fn hud_panel(color: Color, pad: (f32, f32)) -> impl Bundle {
    framed(
        color,
        Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(pad.0), Val::Px(pad.1)),
            ..default()
        },
    )
}

/// `node` framed and glowing in `color`.
fn framed(color: Color, node: Node) -> impl Bundle {
    (
        Node {
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..node
        },
        BackgroundColor(neonui::PANEL.with_alpha(0.72)),
        BorderColor::all(color.with_alpha(0.8)),
        glow(color, 0.45, 16.0),
    )
}

/// A glowing bar of the crosshair, `w` by `h` px at (`x`, `y`) in its 30 px box.
fn reticle_bar(x: f32, y: f32, w: f32, h: f32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(w),
            height: Val::Px(h),
            border_radius: BorderRadius::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.95, 1.0, 1.0, 0.9)),
        glow(neonui::CYAN, 0.8, 6.0),
    )
}

fn spawn(mut commands: Commands) {
    let red = team_color(Team::Red);
    let blue = team_color(Team::Blue);
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
    // An arena event's announcement, under the scores.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(138.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((EventBanner, text("", 60.0, Color::NONE, 900)));
            p.spawn((EventBannerSub, text("", 20.0, Color::NONE, 700)));
        });
    // Top: RED score, clock, BLUE score; the bots' difficulty below.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(14.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn(neonui::row(14.0)).with_children(|r| {
                for (team, name, c) in [(Team::Red, "RED", red), (Team::Blue, "BLUE", blue)] {
                    if team == Team::Blue {
                        r.spawn(hud_panel(neonui::CYAN, (22.0, 6.0)))
                            .with_children(|b| {
                                b.spawn((ClockText, text("10:00", 34.0, neonui::TEXT, 800)));
                                b.spawn((DoubleText, text("", 12.0, neonui::AMBER, 800)));
                            });
                    }
                    r.spawn(framed(
                        c,
                        Node {
                            min_width: Val::Px(120.0),
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(18.0), Val::Px(6.0)),
                            ..default()
                        },
                    ))
                    .with_children(|b| {
                        b.spawn(text(name, 12.0, c, 700));
                        b.spawn((ScoreText(team), text("0", 30.0, c, 900)));
                    });
                }
            });
            p.spawn((DifficultyText, text("", 13.0, neonui::DIM, 600)));
        });
    // Crosshair: four glowing bars around a dot.
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
            p.spawn(Node {
                width: Val::Px(30.0),
                height: Val::Px(30.0),
                ..default()
            })
            .with_children(|c| {
                c.spawn(reticle_bar(0.0, 14.0, 9.0, 2.0));
                c.spawn(reticle_bar(21.0, 14.0, 9.0, 2.0));
                c.spawn(reticle_bar(14.0, 0.0, 2.0, 9.0));
                c.spawn(reticle_bar(14.0, 21.0, 2.0, 9.0));
                c.spawn(reticle_bar(13.5, 13.5, 3.0, 3.0));
            });
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
                text("", 40.0, neonui::TEXT, 800),
                TextLayout::justify(Justify::Center),
            ));
        });
    // Bottom middle: the Megatarget mode's progress.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(18.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((MegaBox, hud_panel(neonui::AMBER, (18.0, 7.0))))
                .with_children(|b| {
                    b.spawn((
                        MegaText,
                        text("", 18.0, neonui::AMBER, 700),
                        TextLayout::justify(Justify::Center),
                    ));
                });
        });
    // Bottom left: the powerups above a panel with the score, hits left, ammo and grenade.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                bottom: Val::Px(18.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((PowerText, text("", 17.0, neonui::AMBER, 800)));
            p.spawn((
                StatusPanel,
                framed(
                    neonui::CYAN,
                    Node {
                        width: Val::Px(270.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(12.0)),
                        row_gap: Val::Px(9.0),
                        ..default()
                    },
                ),
            ))
            .with_children(|b| {
                b.spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Baseline,
                    ..default()
                })
                .with_children(|r| {
                    r.spawn((StatusPart::Team, text("", 18.0, neonui::TEXT, 900)));
                    r.spawn((StatusPart::Score, text("", 16.0, neonui::TEXT, 700)));
                });
                b.spawn(neonui::row(8.0)).with_children(|r| {
                    for k in 1..=3 {
                        r.spawn((
                            Pip(k),
                            Node {
                                flex_grow: 1.0,
                                height: Val::Px(12.0),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(Color::WHITE),
                            BoxShadow::default(),
                        ));
                    }
                });
                b.spawn(Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Baseline,
                    ..default()
                })
                .with_children(|r| {
                    r.spawn(text("AMMO", 12.0, neonui::DIM, 700));
                    r.spawn((StatusPart::Ammo, text("", 15.0, neonui::TEXT, 700)));
                });
                b.spawn((
                    Node {
                        height: Val::Px(5.0),
                        border_radius: BorderRadius::all(Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.4, 0.42, 0.6, 0.3)),
                ))
                .with_children(|t| {
                    t.spawn((
                        AmmoFill,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            border_radius: BorderRadius::all(Val::Px(3.0)),
                            ..default()
                        },
                        BackgroundColor(neonui::CYAN),
                        glow(neonui::CYAN, 0.7, 6.0),
                    ));
                });
                b.spawn((StatusPart::Grenade, text("", 13.0, neonui::DIM, 700)));
            });
        });
    // Top right: the feed, a pill per line.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                top: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: Val::Px(5.0),
                ..default()
            },
        ))
        .with_children(|p| {
            for i in 0..FEED_LINES {
                p.spawn((
                    FeedLine(i),
                    Node {
                        display: Display::None,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                        border: UiRect::right(Val::Px(3.0)),
                        border_radius: BorderRadius::all(Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(neonui::PANEL.with_alpha(0.65)),
                    BorderColor::all(neonui::CYAN),
                ))
                .with_children(|l| {
                    l.spawn((FeedLineText(i), text("", 15.0, neonui::TEXT, 600)));
                });
            }
        });
}

/// A team's colour in the HUD.
fn team_color(team: Team) -> Color {
    crate::endscreen::team_ui_color(Some(team))
}

/// The colour of `id`'s team (for the feed).
fn color_of(game: &Game, id: PlayerId) -> Color {
    game.snap
        .players
        .iter()
        .find(|p| p.id == id)
        .map_or(neonui::TEXT, |p| team_color(p.team))
}

fn events(mut events: MessageReader<GameEvent>, game: Res<Game>, mut feed: ResMut<Feed>) {
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::TaggedOut { player, by, .. } => {
                let c = if by == lt_sim::HAZARD || player == by {
                    neonui::PINK
                } else {
                    color_of(&game, by)
                };
                let line = if by == lt_sim::HAZARD {
                    format!("{} got zapped", game.name_of(player))
                } else if player == by {
                    format!("{} tagged themself out", game.name_of(player))
                } else {
                    format!("{} tagged out {}", game.name_of(by), game.name_of(player))
                };
                feed.lines.push((line, c, 6.0));
            }
            SimEvent::Assist { player, .. } => {
                feed.lines.push((
                    format!(
                        "{} assist +{}",
                        game.name_of(player),
                        game.rules.assist_points
                    ),
                    neonui::GREEN,
                    3.0,
                ));
            }
            SimEvent::Defended { player, .. } => {
                feed.lines.push((
                    format!(
                        "{} defended +{}",
                        game.name_of(player),
                        game.rules.defend_points
                    ),
                    neonui::GREEN,
                    3.0,
                ));
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
                    neonui::AMBER,
                    3.0,
                ));
            }
            SimEvent::SecondaryHit { by, charged, .. } => {
                let who = game.name_of(by);
                feed.lines.push((
                    format!("{who} hit a secondary target +100"),
                    neonui::AMBER,
                    3.0,
                ));
                if charged {
                    feed.lines.push((
                        format!("{who} may enter the megatarget room!"),
                        neonui::GREEN,
                        5.0,
                    ));
                }
            }
            SimEvent::MegaHit { by, .. } => {
                feed.lines.push((
                    format!(
                        "{} HIT THE MEGATARGET +{}",
                        game.name_of(by),
                        game.rules.megatarget_points
                    ),
                    neonui::MAGENTA,
                    8.0,
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
                let (w, c) = if scores[0] > scores[1] {
                    ("RED WINS", team_color(Team::Red))
                } else if scores[1] > scores[0] {
                    ("BLUE WINS", team_color(Team::Blue))
                } else {
                    ("DRAW", neonui::TEXT)
                };
                feed.lines.push((w.to_string(), c, 15.0));
            }
            _ => {}
        }
    }
    if feed.lines.len() > FEED_LINES {
        let n = feed.lines.len() - FEED_LINES;
        feed.lines.drain(..n);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update(
    game: Res<Game>,
    time: Res<Time>,
    mut feed: ResMut<Feed>,
    mut scores: Query<(&ScoreText, &mut Text), (Without<ClockText>, Without<CenterText>)>,
    mut clock: Query<
        (&mut Text, &mut TextColor),
        (With<ClockText>, Without<CenterText>, Without<DoubleText>),
    >,
    mut double: Query<
        (&mut Text, &mut TextColor),
        (
            With<DoubleText>,
            Without<ScoreText>,
            Without<ClockText>,
            Without<CenterText>,
        ),
    >,
    mut center: Query<(&mut Text, &mut TextColor), (With<CenterText>, Without<DoubleText>)>,
    mut flash: Query<&mut BackgroundColor, With<HitFlash>>,
    mut difficulty: Query<
        &mut Text,
        (
            With<DifficultyText>,
            Without<ScoreText>,
            Without<ClockText>,
            Without<CenterText>,
            Without<DoubleText>,
        ),
    >,
    session: Res<Session>,
) {
    if let Ok(mut t) = difficulty.single_mut() {
        let want = if game.is_local() {
            format!("BOTS: {}", session.difficulty.name().to_uppercase())
        } else {
            String::new()
        };
        if t.0 != want {
            t.0 = want;
        }
    }
    let dt = time.delta_secs();
    for (s, mut t) in &mut scores {
        let want = game.snap.scores[s.0.index()].to_string();
        if t.0 != want {
            t.0 = want;
        }
    }
    if let Ok((mut t, mut c)) = clock.single_mut() {
        let left = game.snap.time_left.max(0.0);
        let s = left as u32;
        let want = format!("{}:{:02}", s / 60, s % 60);
        if t.0 != want {
            t.0 = want;
        }
        // Amber in the last minute, throbbing pink in the last ten seconds.
        c.0 = if game.snap.over || left > 60.0 {
            neonui::TEXT
        } else if left > 10.0 {
            neonui::AMBER
        } else {
            neonui::PINK.mix(
                &neonui::TEXT,
                0.5 - 0.5 * (left.fract() * std::f32::consts::TAU).cos(),
            )
        };
    }
    if let Ok((mut t, mut c)) = double.single_mut() {
        let (want, color) = clock_events(&game.snap);
        if t.0 != want {
            t.0 = want;
        }
        c.0 = color;
    }
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    if let Ok((mut t, mut c)) = center.single_mut() {
        let (want, color) = match me {
            _ if !game.status.is_empty() => (game.status.clone(), neonui::TEXT),
            _ if game.snap.over => (String::new(), neonui::TEXT),
            Some(p) if p.respawn.is_some() => (
                format!("TAGGED OUT\nback in {:.0}", p.respawn.unwrap().ceil()),
                neonui::PINK,
            ),
            Some(p) if p.cooldown > 0.0 => (
                format!("HIT! blaster down {:.1}", p.cooldown),
                neonui::AMBER,
            ),
            Some(p) if p.ammo == 0 => (
                "OUT OF AMMO\nrefill at your base or a white station".to_string(),
                neonui::AMBER,
            ),
            Some(p) if p.power.is_some() => {
                let left = p.power.unwrap_or(0.0).ceil();
                let inside = game.level.mega.as_ref().is_some_and(|m| m.in_room(p.feet));
                let s = if inside {
                    format!("SHOOT THE MEGATARGET ABOVE YOU!  {left:.0}")
                } else {
                    format!("GET INTO THE MEGATARGET ROOM  {left:.0}")
                };
                (s, neonui::GREEN)
            }
            _ => (String::new(), neonui::TEXT),
        };
        if t.0 != want {
            t.0 = want;
        }
        c.0 = color;
    }
    feed.flash = (feed.flash - dt).max(0.0);
    if let Ok(mut b) = flash.single_mut() {
        b.0 = Color::srgba(1.0, 0.0, 0.0, feed.flash * 0.3);
    }
}

/// What shows under the clock: a running blackout with its seconds left, and double points
/// (in the blackout's pale blue while one runs, else amber).
fn clock_events(snap: &lt_sim::sim::Snapshot) -> (String, Color) {
    let mut parts = Vec::new();
    let mut color = neonui::AMBER;
    if let Some((lt_sim::ArenaEvent::Blackout, left)) = snap.event {
        parts.push(format!("BLACKOUT {:.0}", left.ceil()));
        color = Color::srgb(0.75, 0.78, 0.95);
    }
    if snap.double {
        parts.push("2X POINTS".to_string());
    }
    (parts.join("  "), color)
}

/// The bottom left panel: team and score, hits left (blinking while the blaster is down), ammo
/// and the grenade.
#[allow(clippy::type_complexity)]
fn status_panel(
    game: Res<Game>,
    time: Res<Time>,
    mut parts: Query<(&StatusPart, &mut Text, &mut TextColor)>,
    mut panel: Query<
        (&mut BorderColor, &mut BoxShadow),
        (With<StatusPanel>, Without<Pip>, Without<AmmoFill>),
    >,
    mut pips: Query<(&Pip, &mut BackgroundColor, &mut BoxShadow), Without<AmmoFill>>,
    mut fill: Query<(&mut Node, &mut BackgroundColor, &mut BoxShadow), With<AmmoFill>>,
) {
    let Some(p) = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))
    else {
        return;
    };
    let c = team_color(p.team);
    if let Ok((mut border, mut shadow)) = panel.single_mut() {
        *border = BorderColor::all(c.with_alpha(0.8));
        *shadow = glow(c, 0.45, 16.0);
    }
    let reload = if p.reloading { "REFILLING  " } else { "" };
    for (part, mut t, mut color) in &mut parts {
        let (want, col) = match part {
            StatusPart::Team => (
                if p.team == Team::Red { "RED" } else { "BLUE" }.to_string(),
                c,
            ),
            StatusPart::Score => (format!("SCORE {}", p.score), neonui::TEXT),
            StatusPart::Ammo => (
                format!("{reload}{} / {}", p.ammo, game.rules.ammo),
                if p.ammo == 0 {
                    neonui::PINK
                } else {
                    neonui::TEXT
                },
            ),
            StatusPart::Grenade => {
                if p.grenades > 0 {
                    ("GRENADE READY  (RMB / G)".to_string(), neonui::GREEN)
                } else {
                    ("NO GRENADE".to_string(), neonui::DIM)
                }
            }
        };
        if t.0 != want {
            t.0 = want;
        }
        color.0 = col;
    }
    let left = 3u8.saturating_sub(p.hits);
    let blink = p.cooldown <= 0.0 || (time.elapsed_secs() * 8.0).fract() < 0.5;
    for (pip, mut bg, mut shadow) in &mut pips {
        let lit = pip.0 <= left && blink;
        bg.0 = if lit {
            c
        } else {
            Color::srgba(0.4, 0.42, 0.6, 0.25)
        };
        *shadow = glow(c, if lit { 0.8 } else { 0.0 }, 10.0);
    }
    if let Ok((mut node, mut bg, mut shadow)) = fill.single_mut() {
        let k = p.ammo as f32 / game.rules.ammo.max(1) as f32;
        node.width = Val::Percent(100.0 * k.clamp(0.0, 1.0));
        let c = if k <= 0.25 {
            neonui::PINK
        } else {
            neonui::CYAN
        };
        bg.0 = c;
        *shadow = glow(c, 0.7, 6.0);
    }
}

/// The feed: a pill per line, framed in its colour, fading out at the end.
fn feed_lines(
    time: Res<Time>,
    mut feed: ResMut<Feed>,
    mut pills: Query<(&FeedLine, &mut Node, &mut BorderColor, &mut BackgroundColor)>,
    mut texts: Query<(&FeedLineText, &mut Text, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    for l in feed.lines.iter_mut() {
        l.2 -= dt;
    }
    feed.lines.retain(|l| l.2 > 0.0);
    let fade = |life: f32| (life / 0.5).clamp(0.0, 1.0);
    for (i, mut node, mut border, mut bg) in &mut pills {
        let want = if feed.lines.get(i.0).is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
        if let Some((_, c, life)) = feed.lines.get(i.0) {
            let a = fade(*life);
            *border = BorderColor::all(c.with_alpha(a));
            bg.0 = neonui::PANEL.with_alpha(0.65 * a);
        }
    }
    for (i, mut t, mut color) in &mut texts {
        if let Some((s, c, life)) = feed.lines.get(i.0) {
            if t.0 != *s {
                t.0 = s.clone();
            }
            color.0 = c.mix(&neonui::TEXT, 0.35).with_alpha(fade(*life));
        }
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

/// Megatarget mode: which secondary targets the local player has hit, or the time left to get
/// into the megatarget's room.
fn powerup_status(game: Res<Game>, mut q: Query<&mut Text, With<PowerText>>) {
    let Ok(mut t) = q.single_mut() else { return };
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    t.0 = me.map(|p| powerup_line(p, &game.rules)).unwrap_or_default();
}

/// An announcement across the middle of the view: what happened, in a word or two, and what
/// it means for the player (upper case letters, digits and `- . : / ( ) + %` only, so that VR's
/// stroke font draws it too).
pub struct Banner {
    pub title: &'static str,
    pub sub: String,
    pub color: Color,
    /// Seconds it shows.
    pub time: f32,
}

/// The banner for an arena event starting or ending, if `e` is one.
pub fn event_banner_for(e: &SimEvent, rules: &lt_sim::Rules) -> Option<Banner> {
    let pale = Color::srgb(0.75, 0.78, 0.95);
    let gold = Color::srgb(1.0, 0.82, 0.25);
    let banner = |title, sub: String, color| Banner {
        title,
        sub,
        color,
        time: BANNER_TIME,
    };
    match *e {
        SimEvent::EventStart {
            kind: lt_sim::ArenaEvent::Blackout,
        } => Some(banner(
            "BLACKOUT",
            format!(
                "LIGHTS OUT FOR {:.0} S - EVERY SHOT LIGHTS UP THE WALLS",
                rules.blackout_time
            ),
            pale,
        )),
        SimEvent::EventEnd {
            kind: lt_sim::ArenaEvent::Blackout,
        } => Some(Banner {
            time: 2.0,
            ..banner("LIGHTS BACK ON", String::new(), pale)
        }),
        SimEvent::EventStart {
            kind: lt_sim::ArenaEvent::Overdrive,
        } => Some(banner(
            "OVERDRIVE",
            "EVERY GRENADE AND POWERUP IS BACK IN THE ARENA".to_string(),
            gold,
        )),
        SimEvent::DoublePoints => Some(banner(
            "DOUBLE POINTS",
            format!("LAST {:.0} S - EVERY POINT COUNTS TWICE", rules.double_last),
            gold,
        )),
        _ => None,
    }
}

/// Seconds an arena event's banner shows.
pub const BANNER_TIME: f32 = 4.0;

/// Banners waiting their turn, so that two announced at once both show: the one showing gives
/// way to the next after `BANNER_MIN` seconds.
#[derive(Default)]
pub struct BannerQueue {
    waiting: std::collections::VecDeque<Banner>,
    /// The showing banner's seconds left (0: none), and seconds shown.
    pub left: f32,
    pub shown: f32,
}

/// Seconds a banner shows at least when another one waits.
const BANNER_MIN: f32 = 2.5;

impl BannerQueue {
    pub fn push(&mut self, b: Banner) {
        self.waiting.push_back(b);
    }

    /// Runs the clock; the next banner if it's its turn now.
    pub fn tick(&mut self, dt: f32) -> Option<Banner> {
        self.shown += dt;
        if !self.waiting.is_empty() {
            self.left = self.left.min((BANNER_MIN - self.shown).max(0.5));
        }
        self.left = (self.left - dt).max(0.0);
        if self.left > 0.0 {
            return None;
        }
        let b = self.waiting.pop_front()?;
        self.left = b.time;
        self.shown = 0.0;
        Some(b)
    }
}

#[allow(clippy::type_complexity)]
fn event_banner(
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    time: Res<Time>,
    mut queue: Local<BannerQueue>,
    mut q: Query<(&mut Text, &mut TextColor), (With<EventBanner>, Without<EventBannerSub>)>,
    mut sub: Query<(&mut Text, &mut TextColor), With<EventBannerSub>>,
) {
    let Ok((mut text, mut color)) = q.single_mut() else {
        return;
    };
    let Ok((mut sub_text, mut sub_color)) = sub.single_mut() else {
        return;
    };
    for GameEvent(e) in events.read() {
        if let Some(banner) = event_banner_for(e, &game.rules) {
            queue.push(banner);
        }
    }
    if let Some(banner) = queue.tick(time.delta_secs()) {
        text.0 = banner.title.to_string();
        sub_text.0 = banner.sub;
        color.0 = banner.color;
    }
    // Fading out in the last half second, a little pulse meanwhile.
    let a = (queue.left / 0.5).min(1.0);
    color
        .0
        .set_alpha(a * (0.85 + 0.15 * (time.elapsed_secs() * 8.0).sin()));
    sub_color.0 = neonui::TEXT.with_alpha(a);
}

/// What the HUD says about a player's powerups ("" with none).
pub fn powerup_line(p: &lt_sim::sim::PlayerSnap, rules: &lt_sim::Rules) -> String {
    let mut parts = Vec::new();
    if p.rapid {
        parts.push("RAPID FIRE".to_string());
    }
    if p.shotgun {
        parts.push("SHOTGUN".to_string());
    }
    if p.charge {
        parts.push(if p.charging >= rules.charge_time {
            "CHARGE GUN: CHARGED, LET GO".to_string()
        } else if p.charging > 0.0 {
            format!("CHARGE GUN {:.0}%", p.charging / rules.charge_time * 100.0)
        } else {
            "CHARGE GUN".to_string()
        });
    }
    if p.cloak > 0.0 {
        let state = if p.cloaked() { "CLOAKED" } else { "SEEN" };
        parts.push(format!("{state} {:.0}", p.cloak.ceil()));
    }
    if p.speed > 0.0 {
        parts.push(format!("SPEED {:.0}", p.speed.ceil()));
    }
    if p.barriers > 0 {
        parts.push("BARRIER READY (E)".to_string());
    }
    if p.shield > 0.0 {
        parts.push(format!("SHIELD {:.0}", p.shield.ceil()));
    } else if p.shields > 0 {
        parts.push("SHIELD READY (Q)".to_string());
    }
    parts.join("   ")
}

#[allow(clippy::type_complexity)]
fn mega_status(
    game: Res<Game>,
    mut q: Query<(&mut Text, &mut TextColor), With<MegaText>>,
    mut frame: Query<(&mut Node, &mut BorderColor, &mut BoxShadow), With<MegaBox>>,
) {
    let Ok((mut t, mut colour)) = q.single_mut() else {
        return;
    };
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let (want, c) = match me {
        Some(p) if game.level.mega.is_some() && p.respawn.is_none() => match p.power {
            Some(left) => (
                format!("MEGATARGET ROOM OPEN  {:.0} s", left.ceil()),
                neonui::GREEN,
            ),
            None => {
                let mark = |i: u8| {
                    if p.mega_hits & (1 << i) != 0 {
                        "HIT"
                    } else {
                        "- - -"
                    }
                };
                (
                    format!("SECONDARY TARGETS   LEFT {}   RIGHT {}", mark(0), mark(1)),
                    neonui::AMBER,
                )
            }
        },
        _ => (String::new(), Color::WHITE),
    };
    if let Ok((mut node, mut border, mut shadow)) = frame.single_mut() {
        let display = if want.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
        *border = BorderColor::all(c.with_alpha(0.8));
        *shadow = glow(c, 0.45, 16.0);
    }
    if t.0 != want {
        t.0 = want;
    }
    colour.0 = c;
}

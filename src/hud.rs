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
/// Megatarget mode: the local player's progress (bottom middle).
#[derive(Component)]
struct MegaText;
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
                (events, update, hit_arcs, mega_status, powerup_status)
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
            p.spawn((
                MegaText,
                text("", 24.0, Color::srgb(1.0, 0.55, 0.15)),
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
    // Above the hit pips: the powerups.
    commands.spawn((
        HudRoot,
        PowerText,
        text("", 24.0, Color::srgb(1.0, 0.85, 0.3)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(20.0),
            bottom: Val::Px(148.0),
            ..default()
        },
    ));
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
                let line = if by == lt_sim::HAZARD {
                    format!("{} got zapped", game.name_of(player))
                } else if player == by {
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
            SimEvent::SecondaryHit { by, charged, .. } => {
                let who = game.name_of(by);
                feed.lines
                    .push((format!("{who} hit a secondary target +100"), 3.0));
                if charged {
                    feed.lines
                        .push((format!("{who} may enter the megatarget room!"), 5.0));
                }
            }
            SimEvent::MegaHit { by, .. } => {
                feed.lines.push((
                    format!(
                        "{} HIT THE MEGATARGET +{}",
                        game.name_of(by),
                        game.rules.megatarget_points
                    ),
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
            Some(p) if p.power.is_some() => {
                let left = p.power.unwrap_or(0.0).ceil();
                let inside = game.level.mega.as_ref().is_some_and(|m| m.in_room(p.feet));
                if inside {
                    format!("SHOOT THE MEGATARGET ABOVE YOU!  {left:.0}")
                } else {
                    format!("GET INTO THE MEGATARGET ROOM  {left:.0}")
                }
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

/// Megatarget mode: which secondary targets the local player has hit, or the time left to get
/// into the megatarget's room.
fn powerup_status(game: Res<Game>, mut q: Query<&mut Text, With<PowerText>>) {
    let Ok(mut t) = q.single_mut() else { return };
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    t.0 = me.map(|p| powerup_line(p, &game.rules)).unwrap_or_default();
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
    if p.shield > 0.0 {
        parts.push(format!("SHIELD {:.0}", p.shield.ceil()));
    } else if p.shields > 0 {
        parts.push("SHIELD READY (Q)".to_string());
    }
    parts.join("   ")
}

fn mega_status(game: Res<Game>, mut q: Query<(&mut Text, &mut TextColor), With<MegaText>>) {
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
                Color::srgb(0.3, 1.0, 0.4),
            ),
            None => {
                let mark = |i: u8| {
                    if p.mega_hits & (1 << i) != 0 {
                        "HIT"
                    } else {
                        "---"
                    }
                };
                (
                    format!("secondary targets   left {}   right {}", mark(0), mark(1)),
                    Color::srgb(1.0, 0.55, 0.15),
                )
            }
        },
        _ => (String::new(), Color::WHITE),
    };
    if t.0 != want {
        t.0 = want;
    }
    colour.0 = c;
}

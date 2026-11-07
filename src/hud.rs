//! The flat-screen HUD: scores and clock, crosshair, hits left, grenade, respawn countdown,
//! the last few tag-outs, and where hits come from (red arcs at the edge of the screen,
//! pointing at the shooter).

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use lt_sim::{PickupKind, PlayerId, SimEvent, Team, Weapon};

use crate::desktop::{MainCamera, Overlays};
use crate::game::{Game, GameEvent};
use crate::neonui;
use crate::session::Session;
use crate::settings::{Action, Settings};

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
/// One of the four strokes of the hit marker, an X around the crosshair.
#[derive(Component)]
struct HitMarkStroke;
/// Line `0` of the points just scored, rising beside the crosshair.
#[derive(Component)]
struct PointsLine(usize);
/// Under the crosshair: the pickup just taken, and what it does.
#[derive(Component)]
struct PickupNote;
#[derive(Component)]
struct PickupNoteSub;
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
    Dash,
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
/// A glow round the edges of the screen while a shield runs (cyan) or cloaked (purple), and a
/// moment after respawning (white).
#[derive(Component)]
struct EdgeTint;
/// One of the hits-left lights (as many shown as hits tag a player out).
#[derive(Component)]
struct Pip(u8);
/// Hits-left lights at most.
const MAX_PIPS: u8 = 6;
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
/// A chip for one of the local player's powerups (bottom left, above the hit lights): framed
/// in the powerup's colour, its name or state, and a bar draining with its time left.
#[derive(Component)]
struct PowerChip(PickupKind);
#[derive(Component)]
struct PowerChipText(PickupKind);
#[derive(Component)]
struct PowerChipFill(PickupKind);
/// The track the fill runs in (hidden for powerups without a time).
#[derive(Component)]
struct PowerChipBar(PickupKind);
/// A marked place (0: one's own target under attack, 1: where to refill, low on shots, then
/// the goals: `goals`): its diamond, and the line under it.
#[derive(Component)]
struct ObjMarker(usize);
#[derive(Component)]
struct ObjLabel(usize);
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
    /// The hit marker's seconds left, and whether it marks a tag-out.
    hit_mark: f32,
    tag_mark: bool,
    /// The points just scored: text and seconds left.
    points: Vec<(String, f32)>,
}

/// Little enough of a full blaster to warn about.
pub const LOW_AMMO: f32 = 0.25;

/// Seconds the hit marker shows (a tag-out's, longer), and a points line.
const HIT_MARK: f32 = 0.22;
const TAG_MARK: f32 = 0.5;
const POINTS_LIFE: f32 = 1.6;
const POINTS_LINES: usize = 4;

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
                    pickup_note,
                    hit_marker,
                    edge_glow,
                    fade_shadows,
                    objective_markers,
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
    commands.spawn((
        HudRoot,
        EdgeTint,
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundGradient::default(),
    ));
    // The marked places.
    for k in 0..2 + MAX_GOALS {
        commands.spawn((
            HudRoot,
            ObjMarker(k),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(16.0),
                height: Val::Px(16.0),
                border: UiRect::all(Val::Px(3.0)),
                display: Display::None,
                ..default()
            },
            BorderColor::all(Color::WHITE),
            BackgroundColor(Color::NONE),
            UiTransform::from_rotation(Rot2::degrees(45.0)),
        ));
        commands.spawn((
            HudRoot,
            ObjLabel(k),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(200.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            text("", 15.0, Color::WHITE, 800),
            TextLayout::justify(Justify::Center),
        ));
    }
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
                // The hit marker: four strokes on the diagonals, pointing in.
                for k in 0..4 {
                    let a = std::f32::consts::FRAC_PI_4 + k as f32 * std::f32::consts::FRAC_PI_2;
                    let (x, y) = (15.0 + 19.0 * a.cos(), 15.0 + 19.0 * a.sin());
                    c.spawn((
                        HitMarkStroke,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(x - 6.0),
                            top: Val::Px(y - 1.5),
                            width: Val::Px(12.0),
                            height: Val::Px(3.0),
                            border_radius: BorderRadius::all(Val::Px(1.5)),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        UiTransform {
                            rotation: Rot2::radians(a),
                            ..default()
                        },
                    ));
                }
            });
            // The points just scored, rising to the right of the crosshair.
            p.spawn(Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                ..default()
            })
            .with_children(|c| {
                for i in 0..POINTS_LINES {
                    c.spawn((
                        PointsLine(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(36.0),
                            ..default()
                        },
                        text("", 18.0, Color::NONE, 900),
                        TextLayout::new(Justify::Left, LineBreak::NoWrap),
                    ));
                }
            });
        });
    // Under the crosshair: the pickup just taken.
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(60.0),
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((PickupNote, text("", 26.0, Color::NONE, 900)));
            p.spawn((PickupNoteSub, text("", 16.0, Color::NONE, 700)));
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
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                max_width: Val::Px(560.0),
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|r| {
                for kind in PickupKind::POWERUPS {
                    let c = powerup_color(kind);
                    r.spawn((
                        PowerChip(kind),
                        framed(
                            c,
                            Node {
                                display: Display::None,
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Stretch,
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                                row_gap: Val::Px(4.0),
                                ..default()
                            },
                        ),
                    ))
                    .with_children(|chip| {
                        chip.spawn((
                            PowerChipText(kind),
                            text("", 15.0, c.mix(&neonui::TEXT, 0.3), 800),
                        ));
                        chip.spawn((
                            PowerChipBar(kind),
                            Node {
                                height: Val::Px(4.0),
                                border_radius: BorderRadius::all(Val::Px(2.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.4, 0.42, 0.6, 0.25)),
                        ))
                        .with_children(|bar| {
                            bar.spawn((
                                PowerChipFill(kind),
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    border_radius: BorderRadius::all(Val::Px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(c),
                                glow(c, 0.7, 6.0),
                            ));
                        });
                    });
                }
            });
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
                    for k in 1..=MAX_PIPS {
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
                b.spawn((StatusPart::Dash, text("", 13.0, neonui::DIM, 700)));
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
pub fn team_color(team: Team) -> Color {
    crate::endscreen::team_ui_color(Some(team))
}

/// The colour of `id`'s team (for the feed).
/// What tagged the local player out, for while they wait: the headline (`TAGGED OUT BY NAME`),
/// its colour (the tagger's team's), and a line on how (the weapon, and the tagger's hits
/// left).
pub fn recap_text(game: &Game, r: &crate::game::Recap) -> (String, Color, String) {
    use lt_sim::TagHow;
    let me = game.me;
    let what = match r.how {
        TagHow::Blaster => "WITH THE BLASTER",
        TagHow::Shotgun => "WITH THE SHOTGUN",
        TagHow::Beam => "WITH THE CHARGE BEAM",
        TagHow::Grenade => "WITH A GRENADE",
        TagHow::Turret => "BY A TURRET",
        TagHow::Gate => "BY A LASER GATE",
        TagHow::Field => "BY THE FORCEFIELD",
        TagHow::Pad => "ON THE ENEMY'S SPAWN PADS",
    };
    match r.by {
        Some(by) if Some(by) == me => (
            "TAGGED OUT".to_string(),
            neonui::PINK,
            "BY YOUR OWN GRENADE".to_string(),
        ),
        Some(by) => {
            let them = game.snap.players.iter().find(|p| p.id == by);
            let left = match them {
                Some(p) if p.respawn.is_some() => "  -  THEY'RE OUT TOO".to_string(),
                Some(p) => {
                    let n = game.rules.hits_to_tag.saturating_sub(p.hits);
                    format!(
                        "  -  THEY HAD {n} HIT{} LEFT",
                        if n == 1 { "" } else { "S" }
                    )
                }
                None => String::new(),
            };
            (
                format!("TAGGED OUT BY {}", game.name_of(by).to_uppercase()),
                color_of(game, by),
                format!("{what}{left}"),
            )
        }
        None => ("TAGGED OUT".to_string(), neonui::PINK, what.to_string()),
    }
}

fn color_of(game: &Game, id: PlayerId) -> Color {
    game.snap
        .players
        .iter()
        .find(|p| p.id == id)
        .map_or(neonui::TEXT, |p| team_color(p.team))
}

/// The kill feed's lines for `e` (the flat screen's, and VR's on the wrist): text, colour and
/// seconds shown.
pub fn feed_entries(e: &SimEvent, game: &Game) -> Vec<(String, Color, f32)> {
    // (Twice as many in the last minute.)
    let pts = |n: u32| if game.snap.double { n * 2 } else { n };
    let mut out = Vec::new();
    match *e {
        SimEvent::TaggedOut { player, by, .. } => {
            let c = if by == lt_sim::HAZARD || player == by {
                neonui::PINK
            } else {
                color_of(game, by)
            };
            let line = if by == lt_sim::HAZARD {
                format!("{} got zapped", game.name_of(player))
            } else if player == by {
                format!("{} tagged themself out", game.name_of(player))
            } else {
                // (Points only for tagging out an enemy.)
                let team = |id| {
                    game.snap
                        .players
                        .iter()
                        .find(|p| p.id == id)
                        .map(|p| p.team)
                };
                let points = if team(by) != team(player) {
                    format!(" +{}", pts(game.rules.tag_points))
                } else {
                    String::new()
                };
                format!(
                    "{} tagged out {}{points}",
                    game.name_of(by),
                    game.name_of(player)
                )
            };
            out.push((line, c, 6.0));
        }
        SimEvent::Assist { player, .. } => {
            out.push((
                format!(
                    "{} assist +{}",
                    game.name_of(player),
                    pts(game.rules.assist_points)
                ),
                neonui::GREEN,
                3.0,
            ));
        }
        SimEvent::Defended { player, .. } => {
            out.push((
                format!(
                    "{} defended +{}",
                    game.name_of(player),
                    pts(game.rules.defend_points)
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
            out.push((
                format!(
                    "{} hit the {:?} target +{}",
                    game.name_of(by),
                    t.unwrap_or(Team::Red),
                    pts(game.rules.target_points)
                ),
                neonui::AMBER,
                3.0,
            ));
        }
        SimEvent::SecondaryHit { by, charged, .. } => {
            let who = game.name_of(by);
            out.push((
                format!(
                    "{who} hit a secondary target +{}",
                    pts(game.rules.secondary_points)
                ),
                neonui::AMBER,
                3.0,
            ));
            if charged {
                out.push((
                    format!("{who} may enter the megatarget room!"),
                    neonui::GREEN,
                    5.0,
                ));
            }
        }
        SimEvent::MegaHit { by, .. } => {
            out.push((
                format!(
                    "{} HIT THE MEGATARGET +{}",
                    game.name_of(by),
                    pts(game.rules.megatarget_points)
                ),
                neonui::MAGENTA,
                8.0,
            ));
        }
        SimEvent::MatchOver { scores } => {
            let (w, c) = if scores[0] > scores[1] {
                ("RED WINS", team_color(Team::Red))
            } else if scores[1] > scores[0] {
                ("BLUE WINS", team_color(Team::Blue))
            } else {
                ("DRAW", neonui::TEXT)
            };
            out.push((w.to_string(), c, 15.0));
        }
        _ => {}
    }
    out
}

fn events(mut events: MessageReader<GameEvent>, game: Res<Game>, mut feed: ResMut<Feed>) {
    for GameEvent(e) in events.read() {
        // (A hit doesn't cut a tag-out's marker short.)
        if let Some(tag) = hit_confirm(e, &game)
            && (tag || !feed.tag_mark || feed.hit_mark <= 0.0)
        {
            feed.hit_mark = if tag { TAG_MARK } else { HIT_MARK };
            feed.tag_mark = tag;
        }
        if let Some((what, points)) = my_points(e, &game) {
            let x2 = if game.snap.double { "  X2" } else { "" };
            feed.points
                .push((format!("+{points} {what}{x2}"), POINTS_LIFE));
            if feed.points.len() > POINTS_LINES {
                feed.points.remove(0);
            }
        }
        feed.lines.extend(feed_entries(e, &game));
        match *e {
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
    recap: Res<crate::game::Recap>,
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
            Some(p) if p.respawn.is_some() => {
                let (title, color, how) = recap_text(&game, &recap);
                (
                    format!(
                        "{title}\n{}\nback in {:.0}",
                        how.to_lowercase(),
                        p.respawn.unwrap().ceil()
                    ),
                    color,
                )
            }
            Some(p) if p.protect > 0.0 => ("PROTECTED  until you fire".to_string(), neonui::TEXT),
            Some(p) if p.cooldown > 0.0 => (
                format!("HIT!  you can't shoot or be hit for {:.1} s", p.cooldown),
                neonui::AMBER,
            ),
            Some(p) if p.ammo == 0 => (
                "OUT OF AMMO\nrefill at your base or a white station".to_string(),
                neonui::AMBER,
            ),
            Some(p) if near_forcefield(&game, p) => (
                "FORCEFIELD! it tags you out\nhit both secondary targets to get through"
                    .to_string(),
                neonui::PINK,
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
    if let lt_sim::MatchPhase::Warmup(left) = snap.phase {
        parts.push(format!("WARMUP {:.0}", left.ceil()));
    }
    if let Some((lt_sim::ArenaEvent::Blackout, left)) = snap.event {
        parts.push(format!("BLACKOUT {:.0}", left.ceil()));
        color = Color::srgb(0.75, 0.78, 0.95);
    }
    if snap.double {
        parts.push("2X POINTS".to_string());
    }
    (parts.join("  "), color)
}

/// The colour and strength (0..1) of the glow round the edge of the view for `p`: a moment
/// after respawning (white, `since_respawn` seconds), cloaked (purple, blinking while seen),
/// shielded (cyan, flickering in its last two seconds).
pub fn edge_tint(p: &lt_sim::sim::PlayerSnap, since_respawn: f32, t: f32) -> (Color, f32) {
    if since_respawn < 0.6 {
        return (Color::WHITE, 1.0 - since_respawn / 0.6);
    }
    if p.respawn.is_some() {
        return (Color::NONE, 0.0);
    }
    if p.protect > 0.0 {
        // Spawn protection: a soft white pulse, fading out with it.
        return (
            Color::WHITE,
            (0.15 + 0.1 * (t * 6.0).sin()) * p.protect.min(0.5) * 2.0,
        );
    }
    if p.cloak > 0.0 {
        let seen = !p.cloaked();
        let k = if seen {
            if (t * 6.0).fract() < 0.5 { 0.7 } else { 0.2 }
        } else {
            0.3 + 0.1 * (t * 2.5).sin()
        };
        return (powerup_color(PickupKind::Cloak), k);
    }
    if p.shield > 0.0 {
        let on = p.shield > 2.0 || (t * 10.0).fract() < 0.6;
        return (
            powerup_color(PickupKind::Shield),
            if on { 0.3 } else { 0.1 },
        );
    }
    (Color::NONE, 0.0)
}

/// The edge glow (see `edge_tint`).
fn edge_glow(
    game: Res<Game>,
    time: Res<Time>,
    mut events: MessageReader<GameEvent>,
    mut since: Local<f32>,
    mut q: Query<&mut BackgroundGradient, With<EdgeTint>>,
) {
    *since += time.delta_secs();
    for GameEvent(e) in events.read() {
        if let SimEvent::Respawned { player, .. } = *e
            && Some(player) == game.me
        {
            *since = 0.0;
        }
    }
    let Ok(mut gradient) = q.single_mut() else {
        return;
    };
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let (c, k) = me.map_or((Color::NONE, 0.0), |p| {
        edge_tint(p, *since, time.elapsed_secs())
    });
    if k <= 0.0 {
        if !gradient.0.is_empty() {
            gradient.0.clear();
        }
        return;
    }
    // From each edge inwards, fading out.
    let stops = || {
        vec![
            ColorStop::new(c.with_alpha(0.55 * k), Val::Px(0.0)),
            ColorStop::new(c.with_alpha(0.0), Val::Px(110.0)),
        ]
    };
    gradient.0 = vec![
        LinearGradient::to_right(stops()).into(),
        LinearGradient::to_left(stops()).into(),
        LinearGradient::to_bottom(stops()).into(),
        LinearGradient::to_top(stops()).into(),
    ];
}

/// A hint for a newcomer that's due now and wasn't shown before: its id (to remember it by)
/// and banner. `since` is the seconds since the match began; `holding`: VR, the blaster is in
/// hand.
pub fn next_hint(
    game: &Game,
    settings: &Settings,
    controls: Controls,
    since: f32,
    holding: bool,
) -> Option<(&'static str, Banner)> {
    let p = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id))?;
    if p.respawn.is_some() || game.snap.over {
        return None;
    }
    let vr = matches!(controls, Controls::Vr);
    let mega = game.level.mega.is_some();
    let low = p.ammo as f32 <= game.rules.ammo as f32 * LOW_AMMO;
    let hints: [(&'static str, bool, &'static str, String); 5] = [
        (
            "grab",
            vr && since > 3.0 && !holding,
            "GRAB YOUR BLASTER",
            "REACH TO YOUR HIP AND SQUEEZE THE GRIP".into(),
        ),
        (
            "goal",
            since > 5.0,
            "YOUR GOAL",
            if mega {
                "TAG OUT ENEMIES - HIT BOTH SECONDARY TARGETS AT THE SIDES TO OPEN THE MEGATARGET ROOM"
            } else {
                "TAG OUT ENEMIES AND HIT THE TARGETS UNDER THE ROOFS IN THEIR HALF"
            }
            .into(),
        ),
        (
            "heal",
            p.hits > 0 && p.cooldown <= 0.0,
            "HURT",
            "STEP ONTO YOUR SPAWN PADS TO HEAL".into(),
        ),
        (
            "refill",
            low && !p.reloading,
            "LOW ON SHOTS",
            "REFILL ON YOUR SPAWN PADS OR AT A WHITE STATION".into(),
        ),
        (
            "keys",
            since > 25.0,
            "TIP",
            if vr {
                "THE RIGHT MENU BUTTON SHOWS THE SCORES - THE CONTROLS AND THE RULES".to_string()
            } else {
                format!(
                    "HOLD {} FOR THE SCORES - ESC FOR THE MENU - THE CONTROLS AND THE RULES",
                    key_hint(settings, Action::Scores)
                )
            },
        ),
    ];
    let (id, _, title, sub) = hints
        .into_iter()
        .find(|(id, due, ..)| *due && !settings.hinted(id))?;
    Some((
        id,
        Banner {
            title,
            sub,
            color: neonui::CYAN,
            time: 5.0,
            pickup: None,
            small: true,
        },
    ))
}

/// The rules in short, for the in-game menus (capitals, digits and `- : ' ( )` only, for VR's
/// stroke font).
pub fn rules_lines(rules: &lt_sim::Rules, mega: bool) -> Vec<String> {
    let mut out = vec![
        format!(
            "{} HITS TAG YOU OUT - AFTER A HIT YOU CAN'T SHOOT OR BE HIT FOR {:.0} S",
            rules.hits_to_tag, rules.hit_cooldown
        ),
        format!(
            "TAG OUT AN ENEMY: {} - ASSIST: {} - DEFEND YOUR {}: +{}",
            rules.tag_points,
            rules.assist_points,
            if mega { "DOORWAY" } else { "TARGETS" },
            rules.defend_points
        ),
    ];
    if mega {
        out.push(format!(
            "HIT BOTH SECONDARY TARGETS ({} EACH) TO PASS THE FORCEFIELDS FOR {:.0} S",
            rules.secondary_points, rules.power_time
        ));
        out.push(format!(
            "THEN THE MEGATARGET IN THE MIDDLE ROOM: {}",
            rules.megatarget_points
        ));
    } else {
        out.push(format!(
            "ENEMY TARGETS: {} A HIT - SHOOT THEM FROM BEHIND OR BELOW",
            rules.target_points
        ));
    }
    out.push("YOUR SPAWN PADS HEAL YOU - THEY AND THE WHITE STATIONS REFILL YOUR BLASTER".into());
    out.push(format!("THE LAST {:.0} S COUNT DOUBLE", rules.double_last));
    out
}

/// The HUD's text shadows fading with their texts (so that no dark ghost stays behind).
fn fade_shadows(mut q: Query<(&TextColor, &mut TextShadow), Changed<TextColor>>) {
    for (c, mut shadow) in &mut q {
        let a = 0.85 * c.0.alpha();
        if shadow.color.alpha() != a {
            shadow.color.set_alpha(a);
        }
    }
}

/// Megatarget mode: whether `p` stands close to a forcefield it may not pass, outside the room.
pub fn near_forcefield(game: &Game, p: &lt_sim::sim::PlayerSnap) -> bool {
    let Some(m) = game.level.mega.as_ref() else {
        return false;
    };
    p.power.is_none()
        && p.respawn.is_none()
        && !m.in_room(p.feet)
        && m.shields.iter().any(|s| {
            let c = (s.min + s.max) / 2.0;
            Vec2::new(c.x - p.feet.x, c.z - p.feet.z).length() < 3.5
        })
}

/// Whether `e` is the local player's bolt hitting an enemy: `Some(true)` for a tag-out.
pub fn hit_confirm(e: &SimEvent, game: &Game) -> Option<bool> {
    let me = game.me?;
    let team = |id| {
        game.snap
            .players
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.team)
    };
    let enemy = |id| id != me && team(id).is_some() && team(id) != team(me);
    match *e {
        SimEvent::Hit {
            player, by, hits, ..
        } if by == me && enemy(player) && hits < game.rules.hits_to_tag => Some(false),
        SimEvent::TaggedOut { player, by, .. } if by == me && enemy(player) => Some(true),
        _ => None,
    }
}

/// The points `e` gives the local player, if any, and what for (twice as many while points
/// count double).
pub fn my_points(e: &SimEvent, game: &Game) -> Option<(&'static str, u32)> {
    let me = game.me?;
    let r = &game.rules;
    let (what, points) = match *e {
        SimEvent::TaggedOut { .. } if hit_confirm(e, game) == Some(true) => ("TAG", r.tag_points),
        SimEvent::Assist { player, .. } if player == me => ("ASSIST", r.assist_points),
        SimEvent::Defended { player, .. } if player == me => ("DEFEND", r.defend_points),
        SimEvent::TargetHit { by, .. } if by == me => ("TARGET", r.target_points),
        SimEvent::SecondaryHit { by, .. } if by == me => ("SECONDARY", r.secondary_points),
        SimEvent::MegaHit { by, .. } if by == me => ("MEGATARGET", r.megatarget_points),
        _ => return None,
    };
    Some((what, if game.snap.double { points * 2 } else { points }))
}

/// The hit marker (white, a tag-out's bigger and pink) and the points rising beside the
/// crosshair.
#[allow(clippy::type_complexity)]
fn hit_marker(
    time: Res<Time>,
    mut feed: ResMut<Feed>,
    mut strokes: Query<(&mut BackgroundColor, &mut UiTransform), With<HitMarkStroke>>,
    mut lines: Query<(
        &PointsLine,
        &mut Node,
        &mut Text,
        &mut TextColor,
        &mut TextShadow,
    )>,
) {
    let dt = time.delta_secs();
    feed.hit_mark = (feed.hit_mark - dt).max(0.0);
    let total = if feed.tag_mark { TAG_MARK } else { HIT_MARK };
    let k = feed.hit_mark / total;
    let c = if feed.tag_mark {
        neonui::PINK
    } else {
        Color::WHITE
    };
    for (mut bg, mut tr) in &mut strokes {
        bg.0 = c.with_alpha(k.sqrt());
        // (Springing out a little, further for a tag-out.)
        let grow = if feed.tag_mark { 1.6 } else { 1.0 };
        tr.scale = Vec2::splat(grow * (0.8 + 0.4 * k));
    }
    for l in feed.points.iter_mut() {
        l.1 -= dt;
    }
    feed.points.retain(|l| l.1 > 0.0);
    let n = feed.points.len();
    for (line, mut node, mut text, mut color, mut shadow) in &mut lines {
        let Some((s, life)) = feed.points.get(line.0) else {
            color.0 = Color::NONE;
            shadow.color.set_alpha(0.0);
            continue;
        };
        if text.0 != *s {
            text.0 = s.clone();
        }
        let age = POINTS_LIFE - life;
        // (The newest lowest; rising as they age, fading at the end.)
        node.top = Val::Px(-12.0 + 22.0 * (line.0 as f32 - (n - 1) as f32) - 18.0 * age);
        let a = (life / 0.4).min(1.0);
        color.0 = neonui::AMBER.with_alpha(a);
        shadow.color.set_alpha(0.85 * a);
    }
}

/// The bottom left panel: team and score, hits left (blinking while the blaster is down), ammo
/// and the grenade.
#[allow(clippy::type_complexity)]
fn status_panel(
    game: Res<Game>,
    settings: Res<Settings>,
    time: Res<Time>,
    mut parts: Query<(&StatusPart, &mut Text, &mut TextColor)>,
    mut panel: Query<
        (&mut BorderColor, &mut BoxShadow),
        (With<StatusPanel>, Without<Pip>, Without<AmmoFill>),
    >,
    mut pips: Query<(&Pip, &mut Node, &mut BackgroundColor, &mut BoxShadow), Without<AmmoFill>>,
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
    let flash = (time.elapsed_secs() * 3.0).fract() < 0.6;
    for (part, mut t, mut color) in &mut parts {
        let (want, col) = match part {
            StatusPart::Team => (
                if p.team == Team::Red { "RED" } else { "BLUE" }.to_string(),
                c,
            ),
            StatusPart::Score => (format!("SCORE {}", p.score), neonui::TEXT),
            StatusPart::Ammo => {
                let low = p.ammo as f32 <= game.rules.ammo as f32 * LOW_AMMO;
                let warn = if low && !p.reloading && p.ammo > 0 {
                    "LOW  "
                } else {
                    ""
                };
                (
                    format!("{reload}{warn}{} / {}", p.ammo, game.rules.ammo),
                    if p.ammo == 0 || (low && !p.reloading && flash) {
                        neonui::PINK
                    } else {
                        neonui::TEXT
                    },
                )
            }
            StatusPart::Grenade => {
                if p.grenades > 0 {
                    (
                        format!("GRENADE READY  [{}]", key_hint(&settings, Action::Grenade)),
                        neonui::GREEN,
                    )
                } else {
                    ("NO GRENADE".to_string(), neonui::DIM)
                }
            }
            StatusPart::Dash => {
                if p.dash_ready <= 0.0 {
                    (
                        format!("DASH READY  [{}]", key_hint(&settings, Action::Dash)),
                        neonui::CYAN,
                    )
                } else {
                    (format!("DASH  {:.1}", p.dash_ready), neonui::DIM)
                }
            }
        };
        if t.0 != want {
            t.0 = want;
        }
        color.0 = col;
    }
    let left = game.rules.hits_to_tag.saturating_sub(p.hits);
    let blink =
        (p.cooldown <= 0.0 && p.protect <= 0.0) || (time.elapsed_secs() * 8.0).fract() < 0.5;
    for (pip, mut node, mut bg, mut shadow) in &mut pips {
        let want = if pip.0 <= game.rules.hits_to_tag {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
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
        let c = if k <= LOW_AMMO {
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

/// Where one's own team's target is under attack, (low on shots) the nearest place to
/// refill, and (unless turned off) the goals (`goals`): a diamond on the place, or at the edge
/// of the screen towards it, with a word and the distance.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn objective_markers(
    game: Res<Game>,
    settings: Res<Settings>,
    alert: Res<crate::game::TargetAlert>,
    time: Res<Time>,
    cam: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut marks: Query<(&ObjMarker, &mut Node, &mut BorderColor), Without<ObjLabel>>,
    mut labels: Query<(&ObjLabel, &mut Node, &mut Text, &mut TextColor), Without<ObjMarker>>,
) {
    let (Ok((camera, gt)), Ok(window)) = (cam.single(), window.single()) else {
        return;
    };
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let eye = gt.translation();
    let pulse = 0.65 + 0.35 * (time.elapsed_secs() * 6.0).sin();
    let alert_at = (alert.left > 0.0 && me.is_some_and(|p| p.respawn.is_none())).then(|| {
        (
            alert.pos,
            neonui::PINK.with_alpha(pulse),
            "DEFEND".to_string(),
        )
    });
    let refill = me.and_then(|p| refill_spot(&game, p)).map(|at| {
        let d = at.distance(eye);
        (at + Vec3::Y * 0.6, Color::WHITE, format!("REFILL {d:.0} M"))
    });
    let mut shown = vec![alert_at, refill];
    if let Some(p) = me.filter(|_| settings.objective_markers) {
        for (pos, kind) in goals(&game, p) {
            // (One's own target under attack: marked already.)
            let alerted = kind == GoalKind::Defend
                && alert.left > 0.0
                && (pos - Vec3::Y * 0.8).distance(alert.pos) < 0.1;
            if alerted {
                continue;
            }
            let d = pos.distance(eye);
            let c = goal_color(kind, p.team).with_alpha(GOAL_ALPHA);
            shown.push(Some((pos, c, format!("{} {d:.0} M", kind.word()))));
        }
    }
    shown.resize(2 + MAX_GOALS, None);
    let size = Vec2::new(window.width(), window.height());
    let inv = gt.affine().inverse();
    for (m, mut node, mut border) in &mut marks {
        let at = shown[m.0]
            .as_ref()
            .map(|(pos, c, _)| (place_on_screen(camera, gt, &inv, *pos, size), *c));
        let want = if at.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
        if let Some((v, c)) = at {
            node.left = Val::Px(v.x - 8.0);
            node.top = Val::Px(v.y - 8.0);
            *border = BorderColor::all(c);
        }
        for (l, mut ln, mut t, mut tc) in &mut labels {
            if l.0 != m.0 {
                continue;
            }
            if ln.display != want {
                ln.display = want;
            }
            if let (Some((v, c)), Some((_, _, s))) = (at, shown[m.0].as_ref()) {
                // (On the screen, even at its edge; the goals' over their diamonds.)
                ln.left = Val::Px((v.x - 100.0).clamp(0.0, (size.x - 200.0).max(0.0)));
                ln.top = Val::Px(if m.0 >= 2 {
                    (v.y - 34.0).max(0.0)
                } else {
                    (v.y + 12.0).min(size.y - 24.0)
                });
                if t.0 != *s {
                    t.0 = s.clone();
                }
                tc.0 = c;
            }
        }
    }
}

/// Where on the screen `pos` shows (`size`: the window's), or, off the screen or behind, at
/// its edge towards it.
fn place_on_screen(
    camera: &Camera,
    gt: &GlobalTransform,
    inv: &bevy::math::Affine3A,
    pos: Vec3,
    size: Vec2,
) -> Vec2 {
    let margin = 48.0;
    let local = inv.transform_point3(pos);
    if local.z < 0.0
        && let Ok(v) = camera.world_to_viewport(gt, pos)
        && v.cmpge(Vec2::splat(margin)).all()
        && v.cmple(size - margin).all()
    {
        return v;
    }
    // (Screen y runs down; straight behind: the bottom.)
    let dir = Vec2::new(local.x, -local.y);
    let dir = if dir.length() < 1e-3 {
        Vec2::Y
    } else {
        dir.normalize()
    };
    let half = size / 2.0 - margin;
    let t = (half.x / dir.x.abs().max(1e-4)).min(half.y / dir.y.abs().max(1e-4));
    size / 2.0 + dir * t
}

/// Low on shots (and not refilling): the nearest reload station or own spawn pad, unless
/// it's close by.
pub fn refill_spot(game: &Game, p: &lt_sim::sim::PlayerSnap) -> Option<Vec3> {
    let low = p.ammo as f32 <= game.rules.ammo as f32 * LOW_AMMO;
    if p.respawn.is_some() || !low || p.reloading {
        return None;
    }
    let at = game
        .level
        .reloads
        .iter()
        .copied()
        .chain(game.level.spawns[p.team.index()].iter().map(|s| s.pos))
        .min_by(|a, b| a.distance(p.feet).total_cmp(&b.distance(p.feet)))?;
    (at.distance(p.feet) > 3.0).then_some(at)
}

/// What a goal marker marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalKind {
    /// Classic mode: an enemy team target, and one of one's own team's.
    Attack,
    Defend,
    /// Megatarget mode: a secondary target still to hit, and (both hit) the megatarget.
    Secondary,
    Core,
}

impl GoalKind {
    pub fn word(self) -> &'static str {
        match self {
            GoalKind::Attack => "ATTACK",
            GoalKind::Defend => "DEFEND",
            GoalKind::Secondary => "SECONDARY",
            GoalKind::Core => "MEGATARGET",
        }
    }
}

/// Goal markers at most (two targets a team, or two secondary targets).
pub const MAX_GOALS: usize = 4;

/// The places `p` (alive) has to go for: Classic mode, every team target (marked over it);
/// Megatarget mode, the secondary targets it hasn't hit yet, or, both hit, the megatarget
/// (marked under it).
pub fn goals(game: &Game, p: &lt_sim::sim::PlayerSnap) -> Vec<(Vec3, GoalKind)> {
    if p.respawn.is_some() {
        return Vec::new();
    }
    let mut out: Vec<(Vec3, GoalKind)> = game
        .level
        .targets
        .iter()
        .map(|t| {
            let kind = if t.team == p.team {
                GoalKind::Defend
            } else {
                GoalKind::Attack
            };
            (t.centre() + Vec3::Y * 0.8, kind)
        })
        .collect();
    if let Some(m) = &game.level.mega {
        if p.power.is_some() {
            let below = Vec3::Y * (lt_sim::level::CORE_HALF + 0.6);
            out.push((m.core.centre() - below, GoalKind::Core));
        } else {
            out.extend((0..2).filter(|i| p.mega_hits & (1 << i) == 0).map(|i| {
                (
                    m.secondaries[i].centre() + Vec3::Y * 0.8,
                    GoalKind::Secondary,
                )
            }));
        }
    }
    out.truncate(MAX_GOALS);
    out
}

/// A goal marker's opacity.
pub const GOAL_ALPHA: f32 = 0.6;

/// The flat screen's colour for a goal marker of `kind` (`mine`: the local player's team).
fn goal_color(kind: GoalKind, mine: Team) -> Color {
    match kind {
        GoalKind::Attack => crate::endscreen::team_ui_color(Some(mine.other())),
        GoalKind::Defend => crate::endscreen::team_ui_color(Some(mine)),
        GoalKind::Secondary => neonui::AMBER,
        GoalKind::Core => neonui::MAGENTA,
    }
}

/// Announces the pickups the local player takes (saying what they do the first few times).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn pickup_note(
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    mut settings: ResMut<Settings>,
    time: Res<Time>,
    mut queue: Local<BannerQueue>,
    mut refill_from: Local<Option<f32>>,
    mut since: Local<f32>,
    mut q: Query<(&mut Text, &mut TextColor), (With<PickupNote>, Without<PickupNoteSub>)>,
    mut sub: Query<(&mut Text, &mut TextColor), With<PickupNoteSub>>,
) {
    let (Ok((mut text, mut color)), Ok((mut sub_text, mut sub_color))) =
        (q.single_mut(), sub.single_mut())
    else {
        return;
    };
    for GameEvent(e) in events.read() {
        if let SimEvent::PickedUp { player, kind, .. } = *e
            && Some(player) == game.me
        {
            let explain = settings.take(kind);
            queue.push(pickup_banner(
                kind,
                &game.rules,
                &settings,
                Controls::Keys,
                explain,
            ));
        }
        // (A refill only when it took a while: firing on a pad refills after every shot.)
        let now = time.elapsed_secs();
        if let SimEvent::Reloading { player } = *e
            && Some(player) == game.me
            && refill_from.is_none()
        {
            *refill_from = Some(now);
        }
        if let Some(b) = expiry_banner(e, &game) {
            queue.push(b);
        }
        if let Some(b) = resupply_banner(e, &game) {
            let healed = b.title == "HEALED";
            if healed || refill_from.take().is_some_and(|t| now - t > 1.0) {
                queue.push(b);
            }
        }
    }
    // A newcomer's hints, one at a time when nothing else shows.
    if game.is_added() {
        *since = 0.0;
    }
    *since += time.delta_secs();
    if queue.idle()
        && let Some((id, b)) = next_hint(&game, &settings, Controls::Keys, *since, true)
    {
        settings.hint(id);
        queue.push(b);
    }
    if let Some(b) = queue.tick(time.delta_secs()) {
        text.0 = b.title.to_uppercase();
        sub_text.0 = b.sub;
        color.0 = b.color;
    }
    let a = (queue.left / 0.4).min(1.0);
    color.0.set_alpha(a);
    sub_color.0 = neonui::TEXT.with_alpha(a);
}

/// A short note when one of the local player's running powerups has run out.
pub fn expiry_banner(e: &SimEvent, game: &Game) -> Option<Banner> {
    let me = game.me?;
    let (kind, title, sub) = match *e {
        SimEvent::ShieldDown { player } if player == me => (PickupKind::Shield, "SHIELD OFF", ""),
        SimEvent::Uncloaked { player } if player == me => (
            PickupKind::Cloak,
            "CLOAK GONE",
            "EVERYONE CAN SEE YOU AGAIN",
        ),
        SimEvent::SpeedDown { player } if player == me => {
            (PickupKind::Speed, "SPEED BOOST OVER", "")
        }
        _ => return None,
    };
    Some(Banner {
        title,
        sub: sub.to_string(),
        color: powerup_color(kind),
        time: 2.0,
        pickup: Some(kind),
        small: true,
    })
}

/// A short note when the local player's blaster is full again or their hits are healed.
pub fn resupply_banner(e: &SimEvent, game: &Game) -> Option<Banner> {
    let me = game.me?;
    let (title, color) = match *e {
        SimEvent::Reloaded { player } if player == me => ("BLASTER FULL", neonui::CYAN),
        SimEvent::Healed { player } if player == me => ("HEALED", neonui::GREEN),
        _ => return None,
    };
    Some(Banner {
        title,
        sub: String::new(),
        color,
        time: 1.2,
        pickup: None,
        small: true,
    })
}

/// The local player's powerups as chips: shown while held, flashing in their last seconds.
#[allow(clippy::type_complexity)]
fn powerup_status(
    game: Res<Game>,
    settings: Res<Settings>,
    time: Res<Time>,
    mut chips: Query<(&PowerChip, &mut Node, &mut BorderColor, &mut BoxShadow)>,
    mut texts: Query<(&PowerChipText, &mut Text)>,
    mut fills: Query<(&PowerChipFill, &mut Node), (Without<PowerChip>, Without<PowerChipBar>)>,
    mut bars: Query<(&PowerChipBar, &mut Node), Without<PowerChip>>,
) {
    let me = game
        .me
        .and_then(|id| game.snap.players.iter().find(|p| p.id == id));
    let state = |kind| me.and_then(|p| powerup_chip(p, kind, &game.rules, &settings));
    let blink = (time.elapsed_secs() * 6.0).fract() < 0.5;
    for (chip, mut node, mut border, mut shadow) in &mut chips {
        let st = state(chip.0);
        let want = if st.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
        let c = powerup_color(chip.0);
        let on = !st.is_some_and(|s| s.flash) || blink;
        *border = BorderColor::all(c.with_alpha(if on { 0.85 } else { 0.25 }));
        *shadow = glow(c, if on { 0.5 } else { 0.1 }, 14.0);
    }
    for (t, mut text) in &mut texts {
        if let Some(st) = state(t.0)
            && text.0 != st.label
        {
            text.0 = st.label;
        }
    }
    for (b, mut node) in &mut bars {
        let want = if state(b.0).is_some_and(|s| s.bar.is_some()) {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != want {
            node.display = want;
        }
    }
    for (f, mut node) in &mut fills {
        let st = state(f.0);
        let k = st.as_ref().and_then(|s| s.bar);
        let want = Val::Percent(100.0 * k.unwrap_or(0.0).clamp(0.0, 1.0));
        if node.width != want {
            node.width = want;
        }
    }
}

/// The first key bound to `a`, in capitals, for a hint ("Q"), or "-" if none is.
pub fn key_hint(settings: &Settings, a: Action) -> String {
    settings
        .get(a)
        .iter()
        .flatten()
        .next()
        .map_or("-".into(), |b| b.name().to_uppercase())
}

/// How a held powerup shows: its label, how full its bar is (`None`: no bar, it lasts until
/// tagged out), and whether it flashes (about to run out, or charged).
struct ChipState {
    label: String,
    bar: Option<f32>,
    flash: bool,
}

/// A powerup's colour on the flat screen (as in the arena).
fn powerup_color(kind: PickupKind) -> Color {
    let c = crate::materials::pickup_color(kind);
    Color::srgb(c.x, c.y, c.z)
}

/// How `p`'s powerup `kind` shows, if `p` has it.
fn powerup_chip(
    p: &lt_sim::sim::PlayerSnap,
    kind: PickupKind,
    rules: &lt_sim::Rules,
    settings: &Settings,
) -> Option<ChipState> {
    let chip = |label: String, bar: Option<f32>, flash| Some(ChipState { label, bar, flash });
    let timed = |name: &str, left: f32, total: f32| {
        chip(
            format!("{name} {:.0}", left.ceil()),
            Some(left / total.max(0.01)),
            left <= 2.0,
        )
    };
    let carried = |name: &str, n: u8, a| {
        let count = if n > 1 {
            format!("{n}x ")
        } else {
            String::new()
        };
        chip(
            format!("{count}{name}  [{}]", key_hint(settings, a)),
            None,
            false,
        )
    };
    match kind {
        PickupKind::Rapid if p.rapid => chip("RAPID FIRE".into(), None, false),
        PickupKind::Shotgun if p.weapon == Weapon::Shotgun => chip("SHOTGUN".into(), None, false),
        PickupKind::Lance if p.weapon == Weapon::Lance => chip("PRISM LANCE".into(), None, false),
        PickupKind::Ricochet if p.weapon == Weapon::Ricochet => {
            chip("RICOCHET".into(), None, false)
        }
        PickupKind::Charge if p.weapon == Weapon::Charge => {
            if p.charging >= rules.charge_time {
                chip("CHARGED - LET GO".into(), Some(1.0), true)
            } else if p.charging > 0.0 {
                chip(
                    "CHARGING".into(),
                    Some(p.charging / rules.charge_time),
                    false,
                )
            } else {
                chip("CHARGE GUN - HOLD FIRE".into(), None, false)
            }
        }
        PickupKind::Cloak if p.cloak > 0.0 => timed(
            if p.cloaked() { "CLOAKED" } else { "SEEN" },
            p.cloak,
            rules.cloak_time,
        ),
        PickupKind::Speed if p.speed > 0.0 => timed("SPEED", p.speed, rules.speed_time),
        PickupKind::Shield if p.shield > 0.0 => timed("SHIELD", p.shield, rules.shield_time),
        PickupKind::Shield if p.shields > 0 => carried("SHIELD", p.shields, Action::Shield),
        _ if let Some((g, n)) = p.gadget
            && kind.gadget() == Some(g) =>
        {
            carried(&g.name().to_uppercase(), n, Action::Gadget)
        }
        _ => None,
    }
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
    /// The pickup it announces (in its colour).
    pub pickup: Option<PickupKind>,
    /// About the player's own doings (a pickup, a secondary target): shown smaller.
    pub small: bool,
}

/// How a pickup's announcement names the controls.
pub enum Controls {
    Keys,
    Vr,
}

/// The announcement of a pickup taken: its name, and (`explain`) what it does and how to use it.
pub fn pickup_banner(
    kind: PickupKind,
    rules: &lt_sim::Rules,
    settings: &Settings,
    controls: Controls,
    explain: bool,
) -> Banner {
    let key = |a: Action, vr: &str| match controls {
        Controls::Keys => format!("PRESS {}", key_hint(settings, a)),
        Controls::Vr => vr.to_string(),
    };
    let sub = match kind {
        PickupKind::Grenade => format!(
            "{} - IT SPINS AND FIRES BOLTS ALL AROUND",
            match controls {
                Controls::Keys => format!(
                    "HOLD {} AND LET GO TO THROW",
                    key_hint(settings, Action::Grenade)
                ),
                Controls::Vr => "HOLD B OR Y - THROW - LET GO".to_string(),
            }
        ),
        PickupKind::Rapid => {
            "YOUR BLASTER FIRES TWICE AS FAST UNTIL YOU ARE TAGGED OUT".to_string()
        }
        PickupKind::Shield => format!(
            "{}: NO BOLT HURTS YOU FOR {:.0} S",
            key(Action::Shield, "TRIGGER OF THE FREE HAND"),
            rules.shield_time
        ),
        PickupKind::Shotgun => format!(
            "{} BOLTS A SHOT IN A SPREAD - THEY FLY ONLY {:.0} M",
            rules.shotgun_bolts, rules.shotgun_range
        ),
        PickupKind::Cloak => format!(
            "UNSEEN FROM AFAR FOR {:.0} S - FIRING OR A HIT SHOWS YOU",
            rules.cloak_time
        ),
        PickupKind::Speed => format!(
            "YOU RUN {:.0}% FASTER FOR {:.0} S",
            (rules.speed_factor - 1.0) * 100.0,
            rules.speed_time
        ),
        PickupKind::Charge => format!(
            "HOLD FIRE {:.0} S AND LET GO: A BEAM THROUGH PLAYERS ({} SHOTS)",
            rules.charge_time, rules.beam_cost
        ),
        PickupKind::Barrier => format!(
            "{}: A WALL THAT STOPS ENEMY BOLTS FOR {:.0} S",
            key(Action::Gadget, "GRIP OF THE FREE HAND"),
            rules.barrier_time
        ),
        PickupKind::Mirror => format!(
            "{}: A MIRROR THAT TURNS BACK EVERY BOLT FOR {:.0} S",
            key(Action::Gadget, "GRIP OF THE FREE HAND"),
            rules.mirror_time
        ),
        PickupKind::Lance => format!(
            "{}A SHOT THAT HITS AT ONCE - A HEAD HIT COUNTS {} ({} SHOTS)",
            match controls {
                Controls::Keys => format!("HOLD {} TO ZOOM - ", key_hint(settings, Action::Zoom)),
                Controls::Vr => String::new(),
            },
            rules.lance_head_damage,
            rules.lance_cost
        ),
        PickupKind::Ricochet => format!(
            "BOLTS BOUNCE OFF EVERY WALL - UP TO {} TIMES",
            rules.ricochet_bounces
        ),
    };
    Banner {
        title: kind.name(),
        sub: if explain { sub } else { String::new() },
        color: if kind == PickupKind::Grenade {
            neonui::TEXT
        } else {
            powerup_color(kind)
        },
        time: if explain { 4.0 } else { 1.5 },
        pickup: Some(kind),
        small: true,
    }
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
        pickup: None,
        small: false,
    };
    match *e {
        SimEvent::EventStart {
            kind: lt_sim::ArenaEvent::Blackout,
        } => Some(banner(
            "BLACKOUT",
            format!("LIGHTS OUT FOR {:.0} S", rules.blackout_time),
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
            "EVERY GRENADE AND POWERUP RESPAWNED".to_string(),
            gold,
        )),
        SimEvent::Countdown { n } => Some(Banner {
            time: 0.95,
            ..banner(
                match n {
                    1 => "1",
                    2 => "2",
                    3 => "3",
                    4 => "4",
                    _ => "5",
                },
                "GET READY".to_string(),
                gold,
            )
        }),
        SimEvent::Go => Some(Banner {
            time: 1.2,
            ..banner("GO!", String::new(), Color::srgb(0.35, 1.0, 0.6))
        }),
        SimEvent::DoublePoints => Some(banner(
            "DOUBLE POINTS",
            format!("LAST {:.0} S - EVERY POINT COUNTS TWICE", rules.double_last),
            gold,
        )),
        _ => None,
    }
}

/// One's own team's target under attack (`game::TargetAlert`): its banner, once per alert
/// (`seen`: the alerts announced so far).
pub fn alert_banner(alert: &crate::game::TargetAlert, seen: &mut u32) -> Option<Banner> {
    if alert.count == *seen {
        return None;
    }
    *seen = alert.count;
    Some(Banner {
        title: "TARGET UNDER ATTACK",
        sub: "DEFEND YOUR TEAM'S TARGET".to_string(),
        color: neonui::PINK,
        time: 2.5,
        pickup: None,
        small: true,
    })
}

/// Megatarget mode: the banner for what `e` means to the local player, if anything.
pub fn mega_banner_for(e: &SimEvent, game: &Game) -> Option<Banner> {
    let me = game.me?;
    let r = &game.rules;
    let banner = |title, sub: String, color, small| Banner {
        title,
        sub,
        color,
        time: if small { 2.5 } else { BANNER_TIME },
        pickup: None,
        small,
    };
    match *e {
        SimEvent::SecondaryHit {
            by, charged: true, ..
        } if by == me => Some(banner(
            "ROOM OPEN",
            format!(
                "ENTER ROOM WITHIN {:.0} S - SHOOT THE MEGATARGET",
                r.power_time
            ),
            neonui::GREEN,
            false,
        )),
        SimEvent::SecondaryHit { by, .. } if by == me => Some(banner(
            "SECONDARY TARGET 1 OF 2",
            "NOW HIT THE OTHER ONE".to_string(),
            neonui::AMBER,
            true,
        )),
        SimEvent::PowerLost { player } if player == me => Some(banner(
            "POWER LOST",
            "HIT BOTH SECONDARY TARGETS AGAIN".to_string(),
            neonui::PINK,
            false,
        )),
        SimEvent::TurretOff { by, .. } if by == me => Some(banner(
            "TURRET OFF",
            format!("FOR {:.0} S", r.turret_off),
            neonui::AMBER,
            true,
        )),
        SimEvent::MegaHit { by, .. } if by == me => Some(banner(
            "MEGATARGET HIT",
            "".to_string(),
            neonui::GREEN,
            false,
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
    /// Nothing showing or waiting.
    pub fn idle(&self) -> bool {
        self.left <= 0.0 && self.waiting.is_empty()
    }

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
#[allow(clippy::too_many_arguments)]
fn event_banner(
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    time: Res<Time>,
    alert: Res<crate::game::TargetAlert>,
    mut alerted: Local<u32>,
    mut queue: Local<BannerQueue>,
    mut q: Query<
        (&mut Text, &mut TextColor, &mut TextFont),
        (With<EventBanner>, Without<EventBannerSub>),
    >,
    mut sub: Query<(&mut Text, &mut TextColor), With<EventBannerSub>>,
) {
    let Ok((mut text, mut color, mut font)) = q.single_mut() else {
        return;
    };
    let Ok((mut sub_text, mut sub_color)) = sub.single_mut() else {
        return;
    };
    for GameEvent(e) in events.read() {
        if let Some(banner) = event_banner_for(e, &game.rules).or_else(|| mega_banner_for(e, &game))
        {
            queue.push(banner);
        }
    }
    if let Some(b) = alert_banner(&alert, &mut alerted) {
        queue.push(b);
    }
    if let Some(banner) = queue.tick(time.delta_secs()) {
        text.0 = banner.title.to_uppercase();
        sub_text.0 = banner.sub;
        color.0 = banner.color;
        font.font_size = bevy::text::FontSize::Px(if banner.small { 34.0 } else { 60.0 });
    }
    // Fading out in the last half second, a little pulse meanwhile.
    let a = (queue.left / 0.5).min(1.0);
    color
        .0
        .set_alpha(a * (0.85 + 0.15 * (time.elapsed_secs() * 8.0).sin()));
    sub_color.0 = neonui::TEXT.with_alpha(a);
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

//! The end of a match on the flat screen: the winner, the teams' scores and a table per team
//! of its players by score (tags, times tagged out, accuracy, target hits), one's own row
//! highlighted. Local games go on to the next match after a while or when asked; remote ones
//! when the server sends the next arena. (VR's panel is in src/vrui.rs; both use
//! `standings`.)

use bevy::prelude::*;
use lt_sim::{PlayerId, Team};

use bevy::window::{CursorOptions, PrimaryWindow};

use crate::desktop::{Overlays, set_grab};
use crate::game::Game;
use crate::session::{Session, SessionCmd};

/// One player's line in the results.
#[derive(Clone, Debug)]
pub struct Row {
    pub id: PlayerId,
    pub name: String,
    pub score: u32,
    pub tags: u32,
    pub tagged: u32,
    /// Bolts that hit an enemy, in percent of those fired (None: never fired).
    pub accuracy: Option<u32>,
    pub target_hits: u32,
    pub me: bool,
}

/// Each team's players (red, blue), best first.
pub fn standings(game: &Game) -> [Vec<Row>; 2] {
    let mut out = [Vec::new(), Vec::new()];
    for p in &game.snap.players {
        out[p.team.index()].push(Row {
            id: p.id,
            name: game.name_of(p.id).to_string(),
            score: p.score,
            tags: p.tags,
            tagged: p.tagged,
            accuracy: (p.shots > 0).then(|| (p.landed * 100 + p.shots / 2) / p.shots),
            target_hits: p.target_hits,
            me: Some(p.id) == game.me,
        });
    }
    for team in &mut out {
        team.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then(b.tags.cmp(&a.tags))
                .then(a.id.cmp(&b.id))
        });
    }
    out
}

/// "RED WINS", "BLUE WINS" or "DRAW", and the winner.
pub fn winner(scores: [u32; 2]) -> (&'static str, Option<Team>) {
    match scores[0].cmp(&scores[1]) {
        std::cmp::Ordering::Greater => ("RED WINS", Some(Team::Red)),
        std::cmp::Ordering::Less => ("BLUE WINS", Some(Team::Blue)),
        std::cmp::Ordering::Equal => ("DRAW", None),
    }
}

pub fn team_ui_color(team: Option<Team>) -> Color {
    match team {
        Some(Team::Red) => Color::srgb(1.0, 0.25, 0.25),
        Some(Team::Blue) => Color::srgb(0.35, 0.6, 1.0),
        None => Color::srgb(0.95, 0.95, 1.0),
    }
}

#[derive(Component)]
struct EndRoot;

#[derive(Component)]
struct Countdown;

#[derive(Component, Clone, Copy)]
enum EndButton {
    Next,
    Menu,
}

pub struct EndScreenPlugin;

impl Plugin for EndScreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (show, countdown, buttons).chain());
    }
}

const BUTTON: Color = Color::srgb(0.08, 0.08, 0.14);
const BUTTON_HOVER: Color = Color::srgb(0.16, 0.12, 0.3);
/// Column widths: name, score, tags, tagged, accuracy, targets.
const COLUMNS: [f32; 6] = [190.0, 76.0, 64.0, 84.0, 104.0, 88.0];

fn label(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s.into()),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn row(
    p: &mut ChildSpawnerCommands,
    cells: [String; 6],
    color: Color,
    size: f32,
    highlight: Option<Color>,
) {
    p.spawn((
        Node {
            padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(highlight.unwrap_or(Color::NONE)),
    ))
    .with_children(|r| {
        for (k, (text, w)) in cells.into_iter().zip(COLUMNS).enumerate() {
            r.spawn(Node {
                width: Val::Px(w),
                justify_content: if k == 0 {
                    JustifyContent::FlexStart
                } else {
                    JustifyContent::FlexEnd
                },
                overflow: Overflow::clip(),
                ..default()
            })
            .with_children(|c| {
                c.spawn(label(text, size, color));
            });
        }
    });
}

fn table(p: &mut ChildSpawnerCommands, team: Team, score: u32, rows: &[Row]) {
    let color = team_ui_color(Some(team));
    let name = if team == Team::Red { "RED" } else { "BLUE" };
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(2.0),
        ..default()
    })
    .with_children(|t| {
        t.spawn((
            Node {
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                ..default()
            },
            BorderColor::all(color),
        ))
        .with_children(|h| {
            h.spawn(label(name, 34.0, color));
            h.spawn(label(score.to_string(), 34.0, color));
        });
        t.spawn((
            Node {
                height: Val::Px(3.0),
                ..default()
            },
            BackgroundColor(color),
        ));
        let grey = Color::srgb(0.6, 0.6, 0.7);
        row(
            t,
            ["player", "score", "tags", "tagged", "accuracy", "targets"].map(String::from),
            grey,
            16.0,
            None,
        );
        for r in rows {
            let cells = [
                r.name.clone(),
                r.score.to_string(),
                r.tags.to_string(),
                r.tagged.to_string(),
                r.accuracy.map_or("-".to_string(), |a| format!("{a}%")),
                r.target_hits.to_string(),
            ];
            let hl = r.me.then(|| color.with_alpha(0.3));
            row(
                t,
                cells,
                if r.me {
                    Color::WHITE
                } else {
                    Color::srgb(0.85, 0.85, 0.9)
                },
                20.0,
                hl,
            );
        }
    });
}

fn button(p: &mut ChildSpawnerCommands, text: &str, b: EndButton) {
    p.spawn((
        Button,
        b,
        Node {
            padding: UiRect::axes(Val::Px(26.0), Val::Px(10.0)),
            border: UiRect::all(Val::Px(2.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BorderColor::all(Color::srgb(0.5, 0.2, 0.9)),
        BackgroundColor(BUTTON),
    ))
    .with_children(|b| {
        b.spawn(label(text, 24.0, Color::WHITE));
    });
}

fn show(
    mut commands: Commands,
    game: Option<Res<Game>>,
    root: Query<Entity, With<EndRoot>>,
    mut overlays: ResMut<Overlays>,
    session: Res<Session>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let over = game.as_ref().is_some_and(|g| g.snap.over);
    overlays.end = over;
    match (over, root.single()) {
        (true, Err(_)) => {
            let game = game.unwrap();
            let (text, team) = winner(game.snap.scores);
            let rows = standings(&game);
            let local = game.is_local();
            let difficulty = local.then(|| session.difficulty.name());
            commands
                .spawn((
                    EndRoot,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(14.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.01, 0.01, 0.03, 0.93)),
                    GlobalZIndex(15),
                ))
                .with_children(|p| {
                    p.spawn(label("MATCH OVER", 22.0, Color::srgb(0.6, 0.6, 0.7)));
                    p.spawn(label(text, 72.0, team_ui_color(team)));
                    let [r, b] = game.snap.scores;
                    p.spawn(Node {
                        column_gap: Val::Px(24.0),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|s| {
                        s.spawn(label(r.to_string(), 40.0, team_ui_color(Some(Team::Red))));
                        s.spawn(label(":", 40.0, Color::WHITE));
                        s.spawn(label(b.to_string(), 40.0, team_ui_color(Some(Team::Blue))));
                    });
                    if let Some(d) = difficulty {
                        p.spawn(label(
                            format!("bots: {d}"),
                            16.0,
                            Color::srgb(0.6, 0.6, 0.7),
                        ));
                    }
                    p.spawn(Node {
                        height: Val::Px(6.0),
                        ..default()
                    });
                    p.spawn(Node {
                        column_gap: Val::Px(40.0),
                        align_items: AlignItems::FlexStart,
                        ..default()
                    })
                    .with_children(|t| {
                        table(t, Team::Red, r, &rows[0]);
                        table(t, Team::Blue, b, &rows[1]);
                    });
                    p.spawn(Node {
                        height: Val::Px(6.0),
                        ..default()
                    });
                    p.spawn((Countdown, label("", 20.0, Color::srgb(0.8, 0.8, 0.9))));
                    p.spawn(Node {
                        column_gap: Val::Px(14.0),
                        ..default()
                    })
                    .with_children(|b| {
                        if local {
                            button(b, "Next match", EndButton::Next);
                        }
                        button(b, "Main menu", EndButton::Menu);
                    });
                });
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
            // The next match: back into the game.
            if game.is_some()
                && !overlays.menu
                && let Ok(mut c) = cursor.single_mut()
            {
                set_grab(&mut c, true);
            }
        }
        _ => {}
    }
}

fn countdown(
    game: Option<Res<Game>>,
    session: Res<Session>,
    mut q: Query<&mut Text, With<Countdown>>,
) {
    let (Some(game), Ok(mut t)) = (game, q.single_mut()) else {
        return;
    };
    let want = match session.next_in() {
        Some(s) if game.is_local() => format!("next match on a new arena in {:.0} s", s.ceil()),
        _ if !game.is_local() => "the server opens the next arena shortly".to_string(),
        _ => String::new(),
    };
    if t.0 != want {
        t.0 = want;
    }
}

#[allow(clippy::type_complexity)]
fn buttons(
    q: Query<(&Interaction, &EndButton), Changed<Interaction>>,
    mut colors: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<EndButton>),
    >,
    mut cmds: MessageWriter<SessionCmd>,
) {
    for (i, mut bg) in &mut colors {
        bg.0 = if *i == Interaction::None {
            BUTTON
        } else {
            BUTTON_HOVER
        };
    }
    for (i, b) in &q {
        if *i == Interaction::Pressed {
            cmds.write(match b {
                EndButton::Next => SessionCmd::NextMatch,
                EndButton::Menu => SessionCmd::Leave,
            });
        }
    }
}

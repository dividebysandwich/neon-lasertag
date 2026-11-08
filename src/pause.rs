//! The flat screen's in-game menu: Esc opens it (the match goes on underneath), Esc again or
//! Resume closes it and takes the mouse again. Leave goes back to the start menu, Quit ends
//! the program. Where the game allows it (`Rules::team_switch`, not Infection) a team row
//! moves the player to the other side. (The VR menu is in src/vrui.rs.)

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};
use lt_sim::Team;

use crate::desktop::{Overlays, set_grab};
use crate::game::Game;
use crate::neonui::{self, NeonButton};
use crate::session::SessionCmd;
use crate::settings::{Action, Settings};

#[derive(Component)]
struct PauseRoot;

#[derive(Component, Clone, Copy)]
enum PauseButton {
    Resume,
    Options,
    Leave,
    Quit,
    Team(Team),
}

pub struct PausePlugin;

impl Plugin for PausePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (toggle, buttons, show)
                .chain()
                .before(crate::options::OptionsSet),
        );
    }
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    game: Option<Res<Game>>,
    mut overlays: ResMut<Overlays>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if game.is_none() {
        overlays.menu = false;
        return;
    }
    // Esc on the options screen is its own.
    if keys.just_pressed(KeyCode::Escape) && !overlays.options {
        overlays.menu = !overlays.menu;
        if !overlays.menu {
            resume(&mut overlays, &mut cursor);
        }
    }
}

/// Back into the game, with the mouse taken (unless the end screen wants it).
fn resume(overlays: &mut Overlays, cursor: &mut Query<&mut CursorOptions, With<PrimaryWindow>>) {
    overlays.menu = false;
    if !overlays.end
        && let Ok(mut c) = cursor.single_mut()
    {
        set_grab(&mut c, true);
    }
}

fn buttons(
    q: Query<(&Interaction, &PauseButton), Changed<Interaction>>,
    mut overlays: ResMut<Overlays>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut cmds: MessageWriter<SessionCmd>,
) {
    for (i, b) in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        match b {
            PauseButton::Resume => resume(&mut overlays, &mut cursor),
            PauseButton::Options => overlays.options = true,
            PauseButton::Leave => {
                overlays.menu = false;
                cmds.write(SessionCmd::Leave);
            }
            PauseButton::Quit => {
                cmds.write(SessionCmd::Quit);
            }
            PauseButton::Team(t) => {
                cmds.write(SessionCmd::SwitchTeam(*t));
                resume(&mut overlays, &mut cursor);
            }
        }
    }
}

fn show(
    mut commands: Commands,
    overlays: Res<Overlays>,
    root: Query<Entity, With<PauseRoot>>,
    game: Option<Res<Game>>,
    settings: Res<Settings>,
) {
    let open = overlays.menu && game.is_some();
    match (open, root.single()) {
        (true, Err(_)) => {
            commands
                .spawn((PauseRoot, neonui::backdrop(neonui::BACKDROP_DEEP, 20)))
                .with_children(|p| {
                    neonui::title(p, "MENU", 52.0, neonui::PINK);
                    p.spawn(neonui::row(24.0)).with_children(|p| {
                        p.spawn(neonui::panel(neonui::MAGENTA, None))
                            .with_children(|p| {
                                for (text, b, color) in [
                                    ("RESUME", PauseButton::Resume, neonui::CYAN),
                                    ("OPTIONS", PauseButton::Options, neonui::VIOLET),
                                    ("LEAVE TO MAIN MENU", PauseButton::Leave, neonui::MAGENTA),
                                    ("QUIT GAME", PauseButton::Quit, neonui::PINK),
                                ] {
                                    neonui::button(
                                        p,
                                        text,
                                        b,
                                        NeonButton::new(color),
                                        Some(360.0),
                                        22.0,
                                    );
                                }
                                if let Some(game) = game.as_deref().filter(|g| g.can_switch_team())
                                {
                                    team_row(p, game);
                                }
                            });
                        if let Some(game) = game.as_deref() {
                            help(p, &settings, game);
                        }
                    });
                    if let Some(status) = game.as_ref().and_then(|g| g.hosting()) {
                        p.spawn(neonui::text(status, 15.0, neonui::CYAN));
                    }
                    p.spawn(neonui::text("Esc: back to the game", 15.0, neonui::DIM));
                });
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
        }
        _ => {}
    }
}

/// Under the buttons: the teams, one's own lit; the other moves the player over.
fn team_row(p: &mut ChildSpawnerCommands, game: &Game) {
    let mine = game.my_team();
    p.spawn(neonui::gap(4.0));
    p.spawn(neonui::text_w("YOUR TEAM", 15.0, neonui::DIM, 700));
    p.spawn(neonui::row(12.0)).with_children(|r| {
        for t in Team::ALL {
            let color = crate::endscreen::team_ui_color(Some(t));
            let neon = NeonButton {
                color,
                selected: mine == Some(t),
            };
            let label = if t == Team::Red { "RED" } else { "BLUE" };
            neonui::button(r, label, PauseButton::Team(t), neon, Some(174.0), 18.0);
        }
    });
}

/// Beside the buttons: the controls as bound now, and the rules in short, side by side.
fn help(p: &mut ChildSpawnerCommands, settings: &Settings, game: &Game) {
    let column = |w: f32| Node {
        width: Val::Px(w),
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(4.0),
        ..default()
    };
    p.spawn(neonui::panel(neonui::VIOLET, None))
        .insert(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::FlexStart,
            column_gap: Val::Px(28.0),
            padding: UiRect::axes(Val::Px(24.0), Val::Px(18.0)),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            max_width: Val::Percent(60.0),
            ..default()
        })
        .with_children(|p| {
            p.spawn(column(270.0)).with_children(|p| {
                p.spawn(neonui::text_w("CONTROLS", 17.0, neonui::VIOLET, 800));
                let mut lines: Vec<(String, String)> = vec![("Look".into(), "Mouse".into())];
                lines.extend(
                    Action::ALL
                        .iter()
                        .map(|&a| (a.name().to_string(), settings.describe(a))),
                );
                lines.push(("Menu".into(), "Esc".into()));
                for (what, keys) in lines {
                    p.spawn(Node {
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|r| {
                        r.spawn(neonui::text(what, 13.0, neonui::DIM));
                        r.spawn(neonui::text(keys, 13.0, neonui::TEXT));
                    });
                }
            });
            p.spawn(column(300.0)).with_children(|p| {
                p.spawn(neonui::text_w("RULES", 17.0, neonui::VIOLET, 800));
                for line in crate::hud::rules_lines(&game.rules, game.level.mode) {
                    p.spawn(neonui::text(line, 12.0, neonui::TEXT));
                    p.spawn(neonui::gap(2.0));
                }
            });
        });
}

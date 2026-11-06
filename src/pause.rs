//! The flat screen's in-game menu: Esc opens it (the match goes on underneath), Esc again or
//! Resume closes it and takes the mouse again. Leave goes back to the start menu, Quit ends
//! the program. (The VR menu is in src/vrui.rs.)

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::desktop::{Overlays, set_grab};
use crate::game::Game;
use crate::session::SessionCmd;

#[derive(Component)]
struct PauseRoot;

#[derive(Component, Clone, Copy)]
enum PauseButton {
    Resume,
    Options,
    Leave,
    Quit,
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

const BUTTON: Color = Color::srgb(0.08, 0.08, 0.14);
const BUTTON_HOVER: Color = Color::srgb(0.16, 0.12, 0.3);

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

#[allow(clippy::type_complexity)]
fn buttons(
    q: Query<(&Interaction, &PauseButton), Changed<Interaction>>,
    mut colors: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<PauseButton>),
    >,
    mut overlays: ResMut<Overlays>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
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
        }
    }
}

fn label(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn show(
    mut commands: Commands,
    overlays: Res<Overlays>,
    root: Query<Entity, With<PauseRoot>>,
    game: Option<Res<Game>>,
) {
    let open = overlays.menu && game.is_some();
    match (open, root.single()) {
        (true, Err(_)) => {
            commands
                .spawn((
                    PauseRoot,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(16.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.01, 0.01, 0.03, 0.75)),
                    GlobalZIndex(20),
                ))
                .with_children(|p| {
                    p.spawn(label("MENU", 56.0, Color::srgb(1.0, 0.3, 0.4)));
                    p.spawn(Node {
                        height: Val::Px(12.0),
                        ..default()
                    });
                    for (text, b) in [
                        ("Resume", PauseButton::Resume),
                        ("Options", PauseButton::Options),
                        ("Leave to main menu", PauseButton::Leave),
                        ("Quit game", PauseButton::Quit),
                    ] {
                        p.spawn((
                            Button,
                            b,
                            Node {
                                width: Val::Px(340.0),
                                padding: UiRect::axes(Val::Px(22.0), Val::Px(12.0)),
                                border: UiRect::all(Val::Px(2.0)),
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BorderColor::all(Color::srgb(0.5, 0.2, 0.9)),
                            BackgroundColor(BUTTON),
                        ))
                        .with_children(|b| {
                            b.spawn(label(text, 26.0, Color::WHITE));
                        });
                    }
                    p.spawn(label(
                        "Esc: back to the game",
                        16.0,
                        Color::srgb(0.6, 0.6, 0.7),
                    ));
                });
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
        }
        _ => {}
    }
}

fn hosting(game: &Game) -> bool {
    matches!(
        &game.source,
        crate::game::Source::Local {
            server: Some(_),
            ..
        }
    )
}

//! The flat screen's in-game menu: Esc opens it (the match goes on underneath), Esc again or
//! Resume closes it and takes the mouse again. Leave goes back to the start menu, Quit ends
//! the program. (The VR menu is in src/vrui.rs.)

use bevy::prelude::*;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::desktop::{Overlays, set_grab};
use crate::game::Game;
use crate::neonui::{self, NeonButton};
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
        }
    }
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
                .spawn((PauseRoot, neonui::backdrop(neonui::BACKDROP_DEEP, 20)))
                .with_children(|p| {
                    neonui::title(p, "MENU", 52.0, neonui::PINK);
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

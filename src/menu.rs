//! The flat screen's start menu: play against bots, host a LAN game, join one by address or
//! from the master server's list, with the bots' number and difficulty and the arenas' size.
//! (VR modes start from the command line.) Shown at the start (nothing chosen on the command line) and whenever a
//! game is left (src/session.rs inserts `ShowMenu`).

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lt_net::{ListSlot, ServerEntry, fetch_list_into};
use lt_sim::bots::Difficulty;
use lt_sim::level::{GameMode, SizeChoice};

use crate::Args;
use crate::desktop::Overlays;

#[derive(Component)]
struct MenuRoot;

#[derive(Component, Clone)]
enum Action {
    Bots,
    Host,
    Join,
    JoinServer(String),
    MoreBots(i32),
    Difficulty(Difficulty),
    Arena(SizeChoice),
    Mode(GameMode),
    Refresh,
    Options,
    Quit,
}

#[derive(Component)]
struct AddressText;
#[derive(Component)]
struct BotsText;
#[derive(Component)]
struct ServerList;

#[derive(Resource)]
struct MenuState {
    list: ListSlot,
    shown: Option<Vec<ServerEntry>>,
    master: String,
}

/// What was chosen last (kept while playing, for the next time the menu shows).
#[derive(Resource)]
struct Choices {
    address: String,
    bots: usize,
    difficulty: Difficulty,
    arena: SizeChoice,
    mode: GameMode,
}

/// The menu has made its choice: start this game.
#[derive(Resource, Clone)]
pub enum Start {
    Local {
        host: bool,
        bots: usize,
        difficulty: Difficulty,
        arena: SizeChoice,
        mode: GameMode,
    },
    Join(String),
}

/// Testing (`LT_DEV`, src/session.rs): play vs bots at this difficulty, as if clicked.
#[derive(Resource)]
pub struct DevPick(pub Difficulty);

/// Show the start menu (taken away when it is shown).
#[derive(Resource)]
pub struct ShowMenu;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, first)
            .add_systems(Update, spawn.run_if(resource_exists::<ShowMenu>))
            .add_systems(
                Update,
                (buttons, typing, show_list, hover).run_if(resource_exists::<MenuState>),
            );
    }
}

fn first(mut commands: Commands, args: Res<Args>) {
    commands.insert_resource(Choices {
        address: String::new(),
        bots: args.bots,
        difficulty: args.difficulty,
        arena: args.arena,
        mode: args.game_mode,
    });
    if args.menu {
        commands.insert_resource(ShowMenu);
    }
}

/// Translucent: the empty arena and its fireworks (src/lobby.rs) show behind the menu.
const BG: Color = Color::srgba(0.02, 0.02, 0.05, 0.45);
const BUTTON: Color = Color::srgb(0.08, 0.08, 0.14);
const BUTTON_HOVER: Color = Color::srgb(0.16, 0.12, 0.3);
const BORDER: Color = Color::srgb(0.5, 0.2, 0.9);
const BORDER_ON: Color = Color::srgb(1.0, 0.85, 0.3);

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

fn button(p: &mut ChildSpawnerCommands, text: &str, action: Action) {
    p.spawn((
        Button,
        action,
        Node {
            padding: UiRect::axes(Val::Px(22.0), Val::Px(10.0)),
            border: UiRect::all(Val::Px(2.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BorderColor::all(BORDER),
        BackgroundColor(BUTTON),
    ))
    .with_children(|b| {
        b.spawn(label(text, 24.0, Color::WHITE));
    });
}

fn spawn(mut commands: Commands, old: Query<Entity, With<MenuRoot>>) {
    commands.remove_resource::<ShowMenu>();
    if !old.is_empty() {
        return;
    }
    let master = std::env::var("LT_MASTER").unwrap_or_else(|_| lt_net::DEFAULT_MASTER.to_string());
    let list = ListSlot::default();
    fetch_list_into(&master, list.clone());
    commands.insert_resource(MenuState {
        list,
        shown: None,
        master,
    });
    commands
        .spawn((
            MenuRoot,
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
            BackgroundColor(BG),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            p.spawn(label("NEON LASERTAG", 64.0, Color::srgb(1.0, 0.3, 0.4)));
            p.spawn(label("RED  vs  BLUE", 22.0, Color::srgb(0.5, 0.65, 1.0)));
            p.spawn(Node { height: Val::Px(20.0), ..default() });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                button(r, "Play vs bots", Action::Bots);
                button(r, "Host LAN game", Action::Host);
                button(r, "-", Action::MoreBots(-1));
                r.spawn((BotsText, label("", 22.0, Color::WHITE)));
                button(r, "+", Action::MoreBots(1));
            });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(label("Bots:", 22.0, Color::WHITE));
                for d in Difficulty::ALL {
                    button(r, d.name(), Action::Difficulty(d));
                }
            });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(label("Arena:", 22.0, Color::WHITE));
                for a in SizeChoice::ALL {
                    let n = a.name();
                    button(r, &format!("{}{}", n[..1].to_uppercase(), &n[1..]), Action::Arena(a));
                }
            });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(label("Mode:", 22.0, Color::WHITE));
                for m in GameMode::ALL {
                    let n = m.name();
                    button(r, &format!("{}{}", n[..1].to_uppercase(), &n[1..]), Action::Mode(m));
                }
            });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(label("Join address:", 22.0, Color::WHITE));
                r.spawn((
                    Node { width: Val::Px(320.0), padding: UiRect::all(Val::Px(8.0)), ..default() },
                    BackgroundColor(Color::srgb(0.0, 0.0, 0.0)),
                ))
                .with_children(|f| {
                    f.spawn((AddressText, label("", 22.0, Color::srgb(0.8, 1.0, 0.8))));
                });
                button(r, "Join", Action::Join);
            });
            p.spawn(Node { height: Val::Px(10.0), ..default() });
            p.spawn(Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() }).with_children(|r| {
                r.spawn(label("Servers", 26.0, Color::WHITE));
                button(r, "Refresh", Action::Refresh);
            });
            p.spawn((
                ServerList,
                Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), min_height: Val::Px(120.0), ..default() },
            ));
            p.spawn(Node { height: Val::Px(10.0), ..default() });
            p.spawn(Node { column_gap: Val::Px(10.0), ..default() }).with_children(|r| {
                button(r, "Options", Action::Options);
                button(r, "Quit", Action::Quit);
            });
            p.spawn(label(
                "WASD move / Ctrl crouch / mouse look / left click fire / hold G or right click for a grenade, let go to throw / Esc menu\n\
                 (keys and mouse in Options)",
                16.0,
                Color::srgb(0.6, 0.6, 0.7),
            ));
        });
}

#[allow(clippy::type_complexity)]
fn hover(mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>) {
    for (i, mut bg) in &mut q {
        bg.0 = if *i == Interaction::None {
            BUTTON
        } else {
            BUTTON_HOVER
        };
    }
}

#[allow(clippy::too_many_arguments)]
fn buttons(
    mut commands: Commands,
    q: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut borders: Query<(&Action, &mut BorderColor)>,
    mut state: ResMut<MenuState>,
    mut choices: ResMut<Choices>,
    root: Query<Entity, With<MenuRoot>>,
    mut bots_text: Query<&mut Text, (With<BotsText>, Without<AddressText>)>,
    mut addr_text: Query<&mut Text, (With<AddressText>, Without<BotsText>)>,
    mut exit: MessageWriter<AppExit>,
    time: Res<Time>,
    pick: Option<Res<DevPick>>,
    mut overlays: ResMut<Overlays>,
) {
    if let Some(p) = pick {
        choices.difficulty = p.0;
        commands.remove_resource::<DevPick>();
        for e in &root {
            commands.entity(e).despawn();
        }
        commands.remove_resource::<MenuState>();
        commands.insert_resource(Start::Local {
            host: false,
            bots: choices.bots,
            difficulty: p.0,
            arena: choices.arena,
            mode: choices.mode,
        });
        return;
    }
    if let Ok(mut t) = bots_text.single_mut() {
        t.0 = format!("{} bots", choices.bots);
    }
    if let Ok(mut t) = addr_text.single_mut() {
        let caret = if (time.elapsed_secs() * 2.0).fract() < 0.5 {
            "_"
        } else {
            " "
        };
        t.0 = format!("{}{caret}", choices.address);
    }
    for (a, mut b) in &mut borders {
        let on = match a {
            Action::Difficulty(d) => *d == choices.difficulty,
            Action::Arena(s) => *s == choices.arena,
            Action::Mode(m) => *m == choices.mode,
            _ => continue,
        };
        let want = BorderColor::all(if on { BORDER_ON } else { BORDER });
        if *b != want {
            *b = want;
        }
    }
    for (i, action) in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        let start = match action.clone() {
            Action::Bots => Some(Start::Local {
                host: false,
                bots: choices.bots,
                difficulty: choices.difficulty,
                arena: choices.arena,
                mode: choices.mode,
            }),
            Action::Host => Some(Start::Local {
                host: true,
                bots: choices.bots,
                difficulty: choices.difficulty,
                arena: choices.arena,
                mode: choices.mode,
            }),
            Action::Join if !choices.address.trim().is_empty() => {
                Some(Start::Join(choices.address.trim().to_string()))
            }
            Action::JoinServer(addr) => Some(Start::Join(addr)),
            Action::MoreBots(d) => {
                choices.bots = (choices.bots as i32 + d).clamp(0, 15) as usize;
                None
            }
            Action::Difficulty(d) => {
                choices.difficulty = d;
                None
            }
            Action::Arena(a) => {
                choices.arena = a;
                None
            }
            Action::Mode(m) => {
                choices.mode = m;
                None
            }
            Action::Refresh => {
                state.shown = None;
                fetch_list_into(&state.master, state.list.clone());
                None
            }
            Action::Options => {
                overlays.options = true;
                None
            }
            Action::Quit => {
                exit.write(AppExit::Success);
                None
            }
            _ => None,
        };
        if let Some(s) = start {
            for e in &root {
                commands.entity(e).despawn();
            }
            commands.remove_resource::<MenuState>();
            commands.insert_resource(s);
            return;
        }
    }
}

fn typing(
    mut keys: MessageReader<KeyboardInput>,
    mut state: ResMut<Choices>,
    mut commands: Commands,
    root: Query<Entity, With<MenuRoot>>,
    overlays: Res<Overlays>,
) {
    for k in keys.read() {
        // Typing on the options screen isn't the address.
        if overlays.options {
            continue;
        }
        if !k.state.is_pressed() {
            continue;
        }
        match &k.logical_key {
            Key::Character(c) => {
                for ch in c.chars() {
                    if (ch.is_ascii_alphanumeric() || ".:-[]".contains(ch))
                        && state.address.len() < 60
                    {
                        state.address.push(ch);
                    }
                }
            }
            Key::Backspace => {
                state.address.pop();
            }
            Key::Enter if !state.address.trim().is_empty() => {
                for e in &root {
                    commands.entity(e).despawn();
                }
                let addr = state.address.trim().to_string();
                commands.remove_resource::<MenuState>();
                commands.insert_resource(Start::Join(addr));
                return;
            }
            _ => {}
        }
    }
}

fn show_list(
    mut commands: Commands,
    mut state: ResMut<MenuState>,
    list: Query<Entity, With<ServerList>>,
) {
    let Ok(parent) = list.single() else { return };
    let fetched = state.list.lock().ok().and_then(|mut s| s.take());
    let Some(result) = fetched else {
        if state.shown.is_none() {
            state.shown = Some(Vec::new());
            commands.entity(parent).despawn_children();
            commands.spawn((
                label(
                    "asking the master server...",
                    18.0,
                    Color::srgb(0.6, 0.6, 0.7),
                ),
                ChildOf(parent),
            ));
        }
        return;
    };
    commands.entity(parent).despawn_children();
    match result {
        Ok(servers) if servers.is_empty() => {
            commands.spawn((
                label("no servers listed", 18.0, Color::srgb(0.6, 0.6, 0.7)),
                ChildOf(parent),
            ));
        }
        Ok(servers) => {
            for s in servers.iter().take(8) {
                let r = &s.report;
                let text = format!(
                    "{}  /  {} players + {} bots  /  {} : {}  /  {}:{:02} left",
                    r.name,
                    r.players.len(),
                    r.bots,
                    r.scores[0],
                    r.scores[1],
                    r.time_left_s / 60,
                    r.time_left_s % 60
                );
                let mut row = commands.spawn((ChildOf(parent), Node::default()));
                row.with_children(|p| button(p, &text, Action::JoinServer(s.addr.clone())));
            }
            state.shown = Some(servers);
        }
        Err(e) => {
            commands.spawn((
                label(
                    format!("server list unavailable: {e}"),
                    18.0,
                    Color::srgb(0.8, 0.5, 0.5),
                ),
                ChildOf(parent),
            ));
        }
    }
}

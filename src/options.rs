//! The flat screen's options screen (from the start menu and the in-game menu): the player's
//! name, sound and music volume, key bindings, mouse sensitivity and inverted mouse look, the field of view, holding
//! or toggling crouch, head bob and reflections, saved as soon as they change (src/settings.rs).
//!
//! A binding is changed by clicking it and pressing a key or a mouse button; Esc cancels,
//! Backspace or Delete clears it. The name is changed by clicking it and typing; Enter or a
//! click keeps it, Esc cancels. Esc otherwise closes the screen.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use crate::Args;
use crate::menu::MenuModel;

use crate::desktop::Overlays;
use crate::neonui::{self, NeonButton};
use crate::settings::{Action, Binding, Settings};

#[derive(Component)]
struct OptionsRoot;

/// The panel of options, scrolled by the mouse wheel where the window is too low for it.
#[derive(Component)]
struct OptionsScroll;

#[derive(Component, Clone, Copy, PartialEq)]
enum Opt {
    Name,
    Slot(Action, usize),
    SensDown,
    SensUp,
    Invert,
    /// The field of view, narrower (false) or wider (true).
    Fov(bool),
    ToggleCrouch,
    HeadBob,
    ObjectiveMarkers,
    Reflections,
    /// Sound effects' and music volume, down (false) or up (true).
    Sound(bool),
    Music(bool),
    Reset,
    Back,
}

/// A text showing a setting.
#[derive(Component, Clone, Copy, PartialEq)]
enum OptText {
    Name,
    Slot(Action, usize),
    Sens,
    Invert,
    Fov,
    ToggleCrouch,
    HeadBob,
    ObjectiveMarkers,
    Reflections,
    Sound,
    Music,
}

/// Waiting for a key or button for this binding (and the frame the wait started).
#[derive(Resource, Default)]
struct Capture(Option<(Action, usize, u32)>);

/// The name being typed.
#[derive(Resource, Default)]
struct Editing(Option<String>);

/// Runs before the in-game menu's Esc handling (src/pause.rs), which leaves Esc alone while
/// this screen is up.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OptionsSet;

pub struct OptionsPlugin;

impl Plugin for OptionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Capture>()
            .init_resource::<Editing>()
            .add_systems(
                Update,
                (show, buttons, capture, typing, scroll, refresh)
                    .chain()
                    .in_set(OptionsSet),
            );
    }
}

const GREY: Color = neonui::DIM;

fn label(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    neonui::text(s, size, color)
}

fn button(p: &mut ChildSpawnerCommands, opt: Opt, width: f32, text: &str, shows: Option<OptText>) {
    p.spawn(neonui::button_node(
        opt,
        NeonButton::new(neonui::VIOLET),
        Some(width),
    ))
    // Compact: the screen has many rows.
    .insert(Node {
        width: Val::Px(width),
        padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)),
        border: UiRect::all(Val::Px(2.0)),
        border_radius: BorderRadius::all(Val::Px(5.0)),
        justify_content: JustifyContent::Center,
        ..default()
    })
    .with_children(|b| {
        let mut t = b.spawn(neonui::text_w(text, 17.0, neonui::TEXT, 600));
        if let Some(s) = shows {
            t.insert(s);
        }
    });
}

fn show(
    mut commands: Commands,
    overlays: Res<Overlays>,
    root: Query<Entity, With<OptionsRoot>>,
    mut capture: ResMut<Capture>,
    mut editing: ResMut<Editing>,
) {
    match (overlays.options, root.single()) {
        (true, Err(_)) => {
            commands
                .spawn((
                    OptionsRoot,
                    neonui::backdrop(Color::srgba(0.01, 0.0, 0.03, 0.97), 30),
                    // Clicks stop here, not at the menu underneath.
                    bevy::ui::FocusPolicy::Block,
                ))
                .with_children(|p| {
                    neonui::title(p, "OPTIONS", 44.0, neonui::PINK);
                    p.spawn(neonui::panel(neonui::VIOLET, None))
                        .insert((
                            OptionsScroll,
                            Node {
                                flex_direction: FlexDirection::Column,
                                align_items: AlignItems::Center,
                                padding: UiRect::axes(Val::Px(32.0), Val::Px(16.0)),
                                row_gap: Val::Px(5.0),
                                border: UiRect::all(Val::Px(2.0)),
                                border_radius: BorderRadius::all(Val::Px(10.0)),
                                // Shrinks to the window, scrolling (`scroll`).
                                min_height: Val::Px(0.0),
                                flex_shrink: 1.0,
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            ScrollPosition::default(),
                        ))
                        .with_children(|p| {
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    let row = || Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, flex_shrink: 0.0, ..default() };
                    let name = |r: &mut ChildSpawnerCommands, s: &str| {
                        r.spawn((Node { width: Val::Px(330.0), ..default() },)).with_children(|n| {
                            n.spawn(label(s, 17.0, neonui::TEXT));
                        });
                    };
                    let stepper = |p: &mut ChildSpawnerCommands, title: &str, down: Opt, up: Opt, shows: OptText| {
                        p.spawn(row()).with_children(|r| {
                            name(r, title);
                            button(r, down, 48.0, "-", None);
                            r.spawn((Node { width: Val::Px(84.0), justify_content: JustifyContent::Center, ..default() },))
                                .with_children(|n| {
                                    n.spawn((shows, label("", 17.0, neonui::TEXT)));
                                });
                            button(r, up, 48.0, "+", None);
                            r.spawn(Node { width: Val::Px(8.0), ..default() });
                        });
                    };
                    // (Wider: a name of NAME_MAX letters fits.)
                    p.spawn(row()).with_children(|r| {
                        r.spawn((Node { width: Val::Px(150.0), ..default() },)).with_children(|n| {
                            n.spawn(label("Your name", 17.0, neonui::TEXT));
                        });
                        button(r, Opt::Name, 384.0, "", Some(OptText::Name));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    stepper(p, "Sound volume", Opt::Sound(false), Opt::Sound(true), OptText::Sound);
                    stepper(p, "Music volume", Opt::Music(false), Opt::Music(true), OptText::Music);
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    stepper(p, "Mouse sensitivity", Opt::SensDown, Opt::SensUp, OptText::Sens);
                    p.spawn(row()).with_children(|r| {
                        name(r, "Invert mouse (up/down)");
                        button(r, Opt::Invert, 204.0, "", Some(OptText::Invert));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    stepper(p, "Field of view", Opt::Fov(false), Opt::Fov(true), OptText::Fov);
                    p.spawn(row()).with_children(|r| {
                        name(r, "Crouch key");
                        button(r, Opt::ToggleCrouch, 204.0, "", Some(OptText::ToggleCrouch));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    p.spawn(row()).with_children(|r| {
                        name(r, "Head bob and blaster sway");
                        button(r, Opt::HeadBob, 204.0, "", Some(OptText::HeadBob));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    p.spawn(row()).with_children(|r| {
                        name(r, "Objective markers");
                        button(r, Opt::ObjectiveMarkers, 204.0, "", Some(OptText::ObjectiveMarkers));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    // WebGL2 has no deferred rendering: no reflections there.
                    if !crate::WEBGL2 {
                        p.spawn(row()).with_children(|r| {
                            name(r, "Reflections (off: faster)");
                            button(r, Opt::Reflections, 204.0, "", Some(OptText::Reflections));
                            r.spawn(Node { width: Val::Px(8.0), ..default() });
                        });
                    }
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    for a in Action::ALL {
                        p.spawn(row()).with_children(|r| {
                            name(r, a.name());
                            for slot in 0..2 {
                                button(r, Opt::Slot(a, slot), 160.0, "", Some(OptText::Slot(a, slot)));
                            }
                        });
                    }
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    p.spawn(label(
                        "click a binding, then press a key or mouse button (Esc cancels, Backspace clears)",
                        16.0,
                        GREY,
                    ));
                    p.spawn(Node { height: Val::Px(8.0), flex_shrink: 0.0, ..default() });
                    p.spawn(row()).with_children(|r| {
                        button(r, Opt::Reset, 260.0, "Reset to defaults", None);
                        button(r, Opt::Back, 260.0, "Back", None);
                    });
                    });
                });
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
            capture.0 = None;
            editing.0 = None;
        }
        _ => {}
    }
}

/// Keeps the typed name (if it was changed).
fn commit(
    editing: &mut Editing,
    args: &mut Args,
    settings: &mut Settings,
    menu: Option<&mut MenuModel>,
) {
    if let Some(name) = editing.0.take()
        && name.trim() != args.name
    {
        args.set_name(settings, menu, &name);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn buttons(
    q: Query<(&Interaction, &Opt), Changed<Interaction>>,
    mut colors: Query<(&Opt, &mut NeonButton)>,
    mut overlays: ResMut<Overlays>,
    mut settings: ResMut<Settings>,
    mut capture: ResMut<Capture>,
    mut editing: ResMut<Editing>,
    mut args: ResMut<Args>,
    mut menu: Option<ResMut<MenuModel>>,
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<bevy::diagnostic::FrameCount>,
) {
    if !overlays.options {
        return;
    }
    for (opt, mut neon) in &mut colors {
        let waiting = matches!((capture.0, opt), (Some((a, s, _)), Opt::Slot(b, t)) if a == *b && s == *t)
            || *opt == Opt::Name && editing.0.is_some();
        let want = NeonButton {
            color: if waiting {
                neonui::AMBER
            } else {
                neonui::VIOLET
            },
            selected: waiting,
        };
        if *neon != want {
            *neon = want;
        }
    }
    // (Esc while typing cancels the name: `typing`.)
    if capture.0.is_none() && editing.0.is_none() && keys.just_pressed(KeyCode::Escape) {
        overlays.options = false;
        return;
    }
    let before = settings.clone();
    for (i, opt) in &q {
        if *i != Interaction::Pressed || capture.0.is_some() {
            continue;
        }
        // A click anywhere keeps the name typed.
        if editing.0.is_some() {
            commit(&mut editing, &mut args, &mut settings, menu.as_deref_mut());
            if *opt == Opt::Name {
                continue;
            }
        }
        match *opt {
            Opt::Name => editing.0 = Some(args.name.clone()),
            Opt::Slot(a, slot) => capture.0 = Some((a, slot, frames.0)),
            Opt::SensDown => {
                settings.sensitivity = ((settings.sensitivity - 0.1) * 10.0).round().max(1.0) / 10.0
            }
            Opt::SensUp => {
                settings.sensitivity =
                    ((settings.sensitivity + 0.1) * 10.0).round().min(50.0) / 10.0
            }
            Opt::Invert => settings.invert_y = !settings.invert_y,
            Opt::Fov(wider) => {
                let (lo, hi) = crate::settings::FOV_RANGE;
                let step = if wider { 5.0 } else { -5.0 };
                settings.fov = (((settings.fov + step) / 5.0).round() * 5.0).clamp(lo, hi);
            }
            Opt::ToggleCrouch => settings.toggle_crouch = !settings.toggle_crouch,
            Opt::HeadBob => settings.head_bob = !settings.head_bob,
            Opt::ObjectiveMarkers => settings.objective_markers = !settings.objective_markers,
            Opt::Reflections => settings.reflections = !settings.reflections,
            Opt::Sound(up) => {
                settings.sound_volume = Settings::step_volume(settings.sound_volume, up)
            }
            Opt::Music(up) => {
                settings.music_volume = Settings::step_volume(settings.music_volume, up)
            }
            // (Not the name: it isn't a setting one resets.)
            Opt::Reset => {
                *settings = Settings {
                    name: std::mem::take(&mut settings.name),
                    ..Settings::default()
                }
            }
            Opt::Back => overlays.options = false,
        }
    }
    if *settings != before {
        settings.save();
    }
}

fn capture(
    mut capture: ResMut<Capture>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut settings: ResMut<Settings>,
    frames: Res<bevy::diagnostic::FrameCount>,
) {
    let Some((action, slot, since)) = capture.0 else {
        return;
    };
    let mut chosen = None;
    for k in keys.get_just_pressed() {
        match k {
            KeyCode::Escape => {
                capture.0 = None;
                return;
            }
            KeyCode::Backspace | KeyCode::Delete => chosen = Some(None),
            k if Binding::bindable_key(*k) => chosen = Some(Some(Binding::Key(*k))),
            _ => {}
        }
    }
    // Not the click that started the wait.
    if frames.0 > since
        && let Some(b) = mouse.get_just_pressed().next()
    {
        chosen = Some(Some(Binding::Mouse(*b)));
    }
    if frames.0 > since && scroll.delta.y != 0.0 {
        chosen = Some(Some(Binding::Wheel));
    }
    if let Some(b) = chosen {
        settings.set(action, slot, b);
        settings.save();
        capture.0 = None;
    }
}

/// The mouse wheel scrolls the panel (unless it is binding the wheel).
fn scroll(
    wheel: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    capture: Res<Capture>,
    mut panel: Query<(&ComputedNode, &mut ScrollPosition), With<OptionsScroll>>,
) {
    use bevy::input::mouse::MouseScrollUnit;
    let Ok((node, mut pos)) = panel.single_mut() else {
        return;
    };
    let scale = node.inverse_scale_factor();
    let room = ((node.content_size().y - node.size().y) * scale).max(0.0);
    let dy = if capture.0.is_none() {
        match wheel.unit {
            MouseScrollUnit::Line => wheel.delta.y * 40.0,
            MouseScrollUnit::Pixel => wheel.delta.y,
        }
    } else {
        0.0
    };
    let y = (pos.y - dy).clamp(0.0, room);
    if y != pos.y {
        pos.y = y;
    }
}

/// Typing the name: Enter keeps it, Esc cancels.
fn typing(
    mut input: MessageReader<KeyboardInput>,
    mut editing: ResMut<Editing>,
    mut args: ResMut<Args>,
    mut settings: ResMut<Settings>,
    mut menu: Option<ResMut<MenuModel>>,
) {
    for k in input.read() {
        if !k.state.is_pressed() {
            continue;
        }
        let Some(name) = editing.0.as_mut() else {
            continue;
        };
        match &k.logical_key {
            Key::Enter => {
                commit(&mut editing, &mut args, &mut settings, menu.as_deref_mut());
                return;
            }
            Key::Escape => {
                editing.0 = None;
                return;
            }
            Key::Backspace => {
                name.pop();
            }
            Key::Space if name.chars().count() < crate::settings::NAME_MAX => name.push(' '),
            Key::Character(c) => {
                for ch in c.chars() {
                    if crate::settings::name_char(ch)
                        && name.chars().count() < crate::settings::NAME_MAX
                    {
                        name.push(ch);
                    }
                }
            }
            _ => {}
        }
    }
}

fn refresh(
    settings: Res<Settings>,
    capture: Res<Capture>,
    editing: Res<Editing>,
    args: Res<Args>,
    time: Res<Time>,
    mut texts: Query<(&OptText, &mut Text)>,
) {
    for (t, mut text) in &mut texts {
        let s = match *t {
            OptText::Name => match &editing.0 {
                Some(n) => {
                    let caret = if time.elapsed_secs().fract() < 0.5 {
                        "_"
                    } else {
                        " "
                    };
                    format!("{n}{caret}")
                }
                None => args.name.clone(),
            },
            OptText::Sens => format!("{:.1}x", settings.sensitivity),
            OptText::Invert => (if settings.invert_y { "On" } else { "Off" }).to_string(),
            OptText::Fov => format!("{:.0}°", settings.fov),
            OptText::ToggleCrouch => (if settings.toggle_crouch {
                "Toggle"
            } else {
                "Hold"
            })
            .to_string(),
            OptText::HeadBob => (if settings.head_bob { "On" } else { "Off" }).to_string(),
            OptText::ObjectiveMarkers => (if settings.objective_markers {
                "On"
            } else {
                "Off"
            })
            .to_string(),
            OptText::Reflections => (if settings.reflections { "On" } else { "Off" }).to_string(),
            OptText::Sound => format!("{:.0} %", settings.sound_volume * 100.0),
            OptText::Music => format!("{:.0} %", settings.music_volume * 100.0),
            OptText::Slot(a, slot) => {
                if matches!(capture.0, Some((b, s, _)) if b == a && s == slot) {
                    "press a key...".into()
                } else {
                    settings.get(a)[slot].map_or_else(|| "-".to_string(), |b| b.name())
                }
            }
        };
        if text.0 != s {
            text.0 = s;
        }
    }
}

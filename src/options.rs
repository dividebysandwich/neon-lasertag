//! The flat screen's options screen (from the start menu and the in-game menu): key bindings,
//! mouse sensitivity and inverted mouse look, saved as soon as they change (src/settings.rs).
//!
//! A binding is changed by clicking it and pressing a key or a mouse button; Esc cancels,
//! Backspace or Delete clears it. Esc otherwise closes the screen.

use bevy::prelude::*;

use crate::desktop::Overlays;
use crate::settings::{Action, Binding, Settings};

#[derive(Component)]
struct OptionsRoot;

#[derive(Component, Clone, Copy, PartialEq)]
enum Opt {
    Slot(Action, usize),
    SensDown,
    SensUp,
    Invert,
    Reset,
    Back,
}

/// A text showing a setting.
#[derive(Component, Clone, Copy, PartialEq)]
enum OptText {
    Slot(Action, usize),
    Sens,
    Invert,
}

/// Waiting for a key or button for this binding (and the frame the wait started).
#[derive(Resource, Default)]
struct Capture(Option<(Action, usize, u32)>);

/// Runs before the in-game menu's Esc handling (src/pause.rs), which leaves Esc alone while
/// this screen is up.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OptionsSet;

pub struct OptionsPlugin;

impl Plugin for OptionsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Settings::load())
            .init_resource::<Capture>()
            .add_systems(Update, (show, buttons, capture, refresh).chain().in_set(OptionsSet));
    }
}

const BUTTON: Color = Color::srgb(0.08, 0.08, 0.14);
const BUTTON_HOVER: Color = Color::srgb(0.16, 0.12, 0.3);
const BUTTON_WAIT: Color = Color::srgb(0.35, 0.25, 0.05);
const BORDER: Color = Color::srgb(0.5, 0.2, 0.9);
const GREY: Color = Color::srgb(0.6, 0.6, 0.7);

fn label(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(s.into()), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(p: &mut ChildSpawnerCommands, opt: Opt, width: f32, text: &str, shows: Option<OptText>) {
    p.spawn((
        Button,
        opt,
        Node {
            width: Val::Px(width),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(7.0)),
            border: UiRect::all(Val::Px(2.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BorderColor::all(BORDER),
        BackgroundColor(BUTTON),
    ))
    .with_children(|b| {
        let mut t = b.spawn(label(text, 20.0, Color::WHITE));
        if let Some(s) = shows {
            t.insert(s);
        }
    });
}

fn show(mut commands: Commands, overlays: Res<Overlays>, root: Query<Entity, With<OptionsRoot>>, mut capture: ResMut<Capture>) {
    match (overlays.options, root.single()) {
        (true, Err(_)) => {
            commands
                .spawn((
                    OptionsRoot,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.01, 0.01, 0.03)),
                    // Clicks stop here, not at the menu underneath.
                    bevy::ui::FocusPolicy::Block,
                    GlobalZIndex(30),
                ))
                .with_children(|p| {
                    p.spawn(label("OPTIONS", 48.0, Color::srgb(1.0, 0.3, 0.4)));
                    p.spawn(Node { height: Val::Px(8.0), ..default() });
                    let row = || Node { column_gap: Val::Px(10.0), align_items: AlignItems::Center, ..default() };
                    let name = |r: &mut ChildSpawnerCommands, s: &str| {
                        r.spawn((Node { width: Val::Px(330.0), ..default() },)).with_children(|n| {
                            n.spawn(label(s, 20.0, Color::WHITE));
                        });
                    };
                    p.spawn(row()).with_children(|r| {
                        name(r, "Mouse sensitivity");
                        button(r, Opt::SensDown, 48.0, "-", None);
                        r.spawn((Node { width: Val::Px(84.0), justify_content: JustifyContent::Center, ..default() },))
                            .with_children(|n| {
                                n.spawn((OptText::Sens, label("", 20.0, Color::WHITE)));
                            });
                        button(r, Opt::SensUp, 48.0, "+", None);
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    p.spawn(row()).with_children(|r| {
                        name(r, "Invert mouse (up/down)");
                        button(r, Opt::Invert, 204.0, "", Some(OptText::Invert));
                        r.spawn(Node { width: Val::Px(8.0), ..default() });
                    });
                    p.spawn(Node { height: Val::Px(8.0), ..default() });
                    for a in Action::ALL {
                        p.spawn(row()).with_children(|r| {
                            name(r, a.name());
                            for slot in 0..2 {
                                button(r, Opt::Slot(a, slot), 160.0, "", Some(OptText::Slot(a, slot)));
                            }
                        });
                    }
                    p.spawn(Node { height: Val::Px(8.0), ..default() });
                    p.spawn(label(
                        "click a binding, then press a key or mouse button (Esc cancels, Backspace clears)",
                        16.0,
                        GREY,
                    ));
                    p.spawn(Node { height: Val::Px(8.0), ..default() });
                    p.spawn(row()).with_children(|r| {
                        button(r, Opt::Reset, 260.0, "Reset to defaults", None);
                        button(r, Opt::Back, 260.0, "Back", None);
                    });
                });
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
            capture.0 = None;
        }
        _ => {}
    }
}

#[allow(clippy::type_complexity)]
fn buttons(
    q: Query<(&Interaction, &Opt), Changed<Interaction>>,
    mut colors: Query<(&Interaction, &Opt, &mut BackgroundColor)>,
    mut overlays: ResMut<Overlays>,
    mut settings: ResMut<Settings>,
    mut capture: ResMut<Capture>,
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<bevy::diagnostic::FrameCount>,
) {
    if !overlays.options {
        return;
    }
    for (i, opt, mut bg) in &mut colors {
        let waiting = matches!((capture.0, opt), (Some((a, s, _)), Opt::Slot(b, t)) if a == *b && s == *t);
        let want = if waiting {
            BUTTON_WAIT
        } else if *i == Interaction::None {
            BUTTON
        } else {
            BUTTON_HOVER
        };
        if bg.0 != want {
            bg.0 = want;
        }
    }
    if capture.0.is_none() && keys.just_pressed(KeyCode::Escape) {
        overlays.options = false;
        return;
    }
    let before = settings.clone();
    for (i, opt) in &q {
        if *i != Interaction::Pressed || capture.0.is_some() {
            continue;
        }
        match *opt {
            Opt::Slot(a, slot) => capture.0 = Some((a, slot, frames.0)),
            Opt::SensDown => settings.sensitivity = ((settings.sensitivity - 0.1) * 10.0).round().max(1.0) / 10.0,
            Opt::SensUp => settings.sensitivity = ((settings.sensitivity + 0.1) * 10.0).round().min(50.0) / 10.0,
            Opt::Invert => settings.invert_y = !settings.invert_y,
            Opt::Reset => *settings = Settings::default(),
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
    mut settings: ResMut<Settings>,
    frames: Res<bevy::diagnostic::FrameCount>,
) {
    let Some((action, slot, since)) = capture.0 else { return };
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
    if let Some(b) = chosen {
        settings.set(action, slot, b);
        settings.save();
        capture.0 = None;
    }
}

fn refresh(settings: Res<Settings>, capture: Res<Capture>, mut texts: Query<(&OptText, &mut Text)>) {
    for (t, mut text) in &mut texts {
        let s = match *t {
            OptText::Sens => format!("{:.1}x", settings.sensitivity),
            OptText::Invert => (if settings.invert_y { "On" } else { "Off" }).to_string(),
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

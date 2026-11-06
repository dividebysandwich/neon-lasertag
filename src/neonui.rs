//! The flat screen's neon look, shared by the start menu (src/menu.rs), the options screen
//! (src/options.rs), the in-game menu (src/pause.rs) and the end of a match (src/endscreen.rs):
//! the Orbitron font (SIL OFL, assets/fonts, compiled in), the palette, and glowing buttons that
//! light up when pointed at ([`NeonButton`], painted by [`NeonUiPlugin`]).

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::text::FontWeight;

/// Orbitron (a variable font: any weight from 400 to 900).
pub const FONT: Handle<Font> = uuid_handle!("6d0c1c55-34a8-4c4e-9b0e-2f6b8a8e5a11");

/// A dark, translucent backdrop: the empty arena (src/lobby.rs) or the match shows behind.
pub const BACKDROP: Color = Color::srgba(0.01, 0.0, 0.04, 0.55);
pub const BACKDROP_DEEP: Color = Color::srgba(0.01, 0.0, 0.03, 0.82);
pub const PANEL: Color = Color::srgba(0.02, 0.01, 0.06, 0.92);
pub const MAGENTA: Color = Color::srgb(1.0, 0.18, 0.72);
pub const CYAN: Color = Color::srgb(0.1, 0.9, 1.0);
pub const PINK: Color = Color::srgb(1.0, 0.32, 0.5);
pub const VIOLET: Color = Color::srgb(0.62, 0.35, 1.0);
pub const AMBER: Color = Color::srgb(1.0, 0.72, 0.25);
pub const GREEN: Color = Color::srgb(0.35, 1.0, 0.6);
pub const TEXT: Color = Color::srgb(0.9, 0.93, 1.0);
pub const DIM: Color = Color::srgb(0.55, 0.58, 0.78);

pub struct NeonUiPlugin;

impl Plugin for NeonUiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_font)
            .add_systems(Update, (paint, pulse));
    }
}

fn load_font(mut fonts: ResMut<Assets<Font>>) {
    let bytes = include_bytes!("../assets/fonts/Orbitron.ttf");
    let _ = fonts.insert(&FONT, Font::from_bytes(bytes.to_vec()));
}

/// Text in the neon font at `size` px, weight 400 (regular) to 900 (black).
pub fn text_w(s: impl Into<String>, size: f32, color: Color, weight: u16) -> impl Bundle {
    (
        Text::new(s.into()),
        TextFont {
            font: FONT.into(),
            font_size: FontSize::Px(size),
            weight: FontWeight(weight),
            ..default()
        },
        TextColor(color),
    )
}

/// Regular text in the neon font.
pub fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    text_w(s, size, color, 500)
}

fn glow(color: Color, alpha: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow::new(
        color.with_alpha(alpha),
        Val::Px(0.0),
        Val::Px(0.0),
        Val::Px(spread),
        Val::Px(blur),
    )
}

/// A button's neon: its colour, and whether it shows the current choice (lit like a pressed
/// one). Its look follows its `Interaction`.
#[derive(Component, Clone, Copy, PartialEq)]
pub struct NeonButton {
    pub color: Color,
    pub selected: bool,
}

impl NeonButton {
    pub fn new(color: Color) -> Self {
        Self {
            color,
            selected: false,
        }
    }
}

fn look(b: &NeonButton, i: Interaction) -> (BorderColor, BackgroundColor, BoxShadow) {
    let c = b.color;
    let (border, fill, shadow, blur) = match (i, b.selected) {
        (Interaction::Pressed, _) => (1.0, 0.45, 0.9, 26.0),
        (Interaction::Hovered, true) => (1.0, 0.6, 0.9, 24.0),
        (Interaction::Hovered, false) => (1.0, 0.2, 0.7, 20.0),
        (Interaction::None, true) => (1.0, 0.5, 0.75, 18.0),
        (Interaction::None, false) => (0.5, 0.04, 0.18, 8.0),
    };
    (
        BorderColor::all(c.with_alpha(border)),
        BackgroundColor(c.with_alpha(fill)),
        glow(c, shadow, blur, 0.0),
    )
}

#[allow(clippy::type_complexity)]
fn paint(
    mut q: Query<
        (
            &NeonButton,
            &Interaction,
            &mut BorderColor,
            &mut BackgroundColor,
            &mut BoxShadow,
        ),
        Or<(Changed<Interaction>, Changed<NeonButton>)>,
    >,
) {
    for (b, i, mut border, mut bg, mut shadow) in &mut q {
        let (bo, f, s) = look(b, *i);
        *border = bo;
        *bg = f;
        *shadow = s;
    }
}

/// A glow that breathes (titles' underlines).
#[derive(Component)]
pub struct Pulse(pub Color);

fn pulse(time: Res<Time>, mut q: Query<(&Pulse, &mut BoxShadow)>) {
    let k = 0.5 + 0.5 * (time.elapsed_secs() * 2.2).sin();
    for (p, mut s) in &mut q {
        *s = glow(p.0, 0.55 + 0.35 * k, 14.0 + 10.0 * k, 1.0 + 2.0 * k);
    }
}

/// A button: `marker` says what it does; `width` in px (or as wide as its text).
pub fn button<'a>(
    p: &'a mut ChildSpawnerCommands,
    label: &str,
    marker: impl Bundle,
    neon: NeonButton,
    width: Option<f32>,
    size: f32,
) -> EntityCommands<'a> {
    let mut e = p.spawn(button_node(marker, neon, width));
    e.with_children(|b| {
        b.spawn(text_w(label, size, TEXT, 600));
    });
    e
}

/// A button without its text (for buttons whose text changes: spawn it as a child).
pub fn button_node(marker: impl Bundle, neon: NeonButton, width: Option<f32>) -> impl Bundle {
    let (border, bg, shadow) = look(&neon, Interaction::None);
    (
        Button,
        marker,
        neon,
        Node {
            width: width.map_or(Val::Auto, Val::Px),
            padding: UiRect::axes(Val::Px(18.0), Val::Px(9.0)),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        border,
        bg,
        shadow,
    )
}

/// A full-screen layer centring its children in a column.
pub fn backdrop(color: Color, z: i32) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(12.0),
            ..default()
        },
        BackgroundColor(color),
        GlobalZIndex(z),
    )
}

/// A framed panel (a column) glowing faintly in `color`.
pub fn panel(color: Color, width: Option<f32>) -> impl Bundle {
    (
        Node {
            width: width.map_or(Val::Auto, Val::Px),
            max_width: Val::Percent(96.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(36.0), Val::Px(26.0)),
            row_gap: Val::Px(12.0),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(color.with_alpha(0.7)),
        glow(color, 0.35, 28.0, 0.0),
    )
}

/// A big title with a breathing neon line under it.
pub fn title(p: &mut ChildSpawnerCommands, s: &str, size: f32, color: Color) {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|t| {
        t.spawn(text_w(s, size, color, 900));
        t.spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(3.0),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(color),
            glow(color, 0.7, 16.0, 1.0),
            Pulse(color),
        ));
    });
}

/// A row of children, `gap` px apart, vertically centred.
pub fn row(gap: f32) -> Node {
    Node {
        column_gap: Val::Px(gap),
        align_items: AlignItems::Center,
        ..default()
    }
}

/// Empty space `h` px high.
pub fn gap(h: f32) -> Node {
    Node {
        height: Val::Px(h),
        ..default()
    }
}

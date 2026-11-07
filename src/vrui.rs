//! VR panels and the hit flash, built from neon meshes so both VR renderers draw them (the GL
//! headset renderer has no UI or text).
//!
//! - The start menu, out of a game (in the empty arena under fireworks, src/lobby.rs): the
//!   levels of src/menu.rs (botmatch, internet and LAN server lists, hosting), with their
//!   choices lit; no typing in VR.
//! - The menu (the right menu button, or B / Y held; see src/vr.rs): Resume, New match (local
//!   games: the next match on a new arena), Main menu and Quit, floating in front of the player.
//! - A panel stays where it was placed (in front of the eyes when it opened; the start menu keeps
//!   its place as its levels change) until the left menu button recenters it.
//! - The end of a match: the winner, the scores and each team's players with their score,
//!   tags, times tagged out, accuracy and target hits (one's own row lit), with the time to
//!   the next match and a Next match button (local games).
//! - While a panel is up, each hand points a thin laser; pulling the trigger on a button
//!   presses it (the blaster doesn't fire meanwhile).
//! - Hits: a red glow at the edge of the view on the side the shot came from (ahead: top,
//!   behind: bottom), fading.
//!
//! `LT_VR_MENU=1` opens the menu at the start (testing).

use bevy::prelude::*;
use lt_sim::bots::Difficulty;
use lt_sim::level::{GameMode, SizeChoice};
use lt_sim::{PlayerId, SimEvent, Team};

use crate::assets::MeshData;
use crate::endscreen::{Row, standings, winner};
use crate::game::{Game, GameEvent};
use crate::materials::{AccentMaterial, GlowMaterial, NeonUniform, Palette, team_color};
use crate::menu::{InternetList, MenuAct, MenuModel, Outcome, Screen};
use crate::segtext::{self, Align};
use crate::session::{Session, SessionCmd};
use crate::vfont;
use crate::vr::{PoseSet, Rumble, VrState, XrInput};

/// Distance of the panels from the eyes.
const PANEL_DISTANCE: f32 = 1.4;
/// Sectors of the hit flash's ring.
const SECTORS: usize = 12;
/// Seconds a hit glows.
const FLASH_LIFE: f32 = 1.0;

pub struct VrUiPlugin;

impl Plugin for VrUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VrUi>()
            .add_plugins(crate::lobby::LobbyPlugin)
            .add_systems(
                Update,
                make_mats
                    .run_if(resource_exists::<Palette>.and_then(not(resource_exists::<UiMats>))),
            )
            .add_systems(
                Update,
                (
                    panels,
                    pointer,
                    hit_flash.run_if(resource_exists::<Game>),
                    event_banner.run_if(resource_exists::<Game>),
                )
                    .chain()
                    .after(PoseSet)
                    .before(crate::game::DrawSet)
                    .run_if(resource_exists::<UiMats>.and_then(resource_exists::<MenuModel>)),
            );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The start menu, out of a game.
    Lobby,
    Menu,
    End,
}

#[derive(Clone, Debug, PartialEq)]
enum Act {
    Resume,
    NewMatch,
    /// Leave the game for the start menu.
    Leave,
    Quit,
    /// The start menu's (src/menu.rs).
    Menu(MenuAct),
}

/// What the start menu panel shows: its level, the choices and the lists' revision (it is
/// built anew when they change).
type LobbyKey = (Screen, usize, Difficulty, SizeChoice, GameMode, u32);

fn lobby_key(m: &MenuModel) -> LobbyKey {
    (m.screen, m.bots, m.difficulty, m.arena, m.mode, m.rev)
}

struct Button {
    act: Act,
    center: Vec2,
    half: Vec2,
    /// The glow lit while pointed at.
    hover: Entity,
}

struct Panel {
    kind: Kind,
    root: Entity,
    /// In the tracking space (follows the rig as the player moves and turns).
    pose: Transform,
    buttons: Vec<Button>,
    /// The end panel's countdown: its text and the seconds shown.
    countdown: Option<(Entity, i32)>,
    countdown_at: Vec3,
    /// The start menu: the choices it shows, and whether it was placed before the head was
    /// tracked (it is placed again then).
    shows: Option<LobbyKey>,
    headless: bool,
}

#[derive(Resource, Default)]
struct VrUi {
    menu: bool,
    panel: Option<Panel>,
    lasers: Option<[Entity; 2]>,
    /// Per hand: the trigger held, the button pointed at.
    trigger: [bool; 2],
    pointed: [Option<usize>; 2],
    /// The hit flash's ring and the hits glowing: where they came from (None: all round), time
    /// left.
    ring: Option<Entity>,
    hits: Vec<(Option<HitFrom>, f32)>,
    started: bool,
}

#[derive(Clone, Copy)]
struct HitFrom {
    by: PlayerId,
    pos: Vec3,
}

#[derive(Resource)]
struct UiMats {
    white: Handle<AccentMaterial>,
    dim: Handle<AccentMaterial>,
    /// The awards' titles.
    gold: Handle<AccentMaterial>,
    laser: Handle<AccentMaterial>,
    laser_hot: Handle<AccentMaterial>,
    /// Text in team colours (red, blue, white).
    team: [Handle<AccentMaterial>; 3],
    glow: [Handle<GlowMaterial>; 3],
    /// One's own row (red, blue, white).
    mine: [Handle<GlowMaterial>; 3],
    back: Handle<StandardMaterial>,
    flash: Vec<Handle<GlowMaterial>>,
    /// A unit cube (strips, lasers) and a unit square of even glow.
    cube: Handle<Mesh>,
    fill: Handle<Mesh>,
    /// The flat screen's neon colours (src/neonui.rs), by `Hue`.
    tints: Vec<Tint>,
}

/// The flat screen's neon colours, for the panels to match its menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hue {
    Cyan,
    Magenta,
    Violet,
    Pink,
    Amber,
    Green,
}

impl Hue {
    const ALL: [Hue; 6] = [
        Hue::Cyan,
        Hue::Magenta,
        Hue::Violet,
        Hue::Pink,
        Hue::Amber,
        Hue::Green,
    ];

    fn color(self) -> Vec3 {
        let c = match self {
            Hue::Cyan => crate::neonui::CYAN,
            Hue::Magenta => crate::neonui::MAGENTA,
            Hue::Violet => crate::neonui::VIOLET,
            Hue::Pink => crate::neonui::PINK,
            Hue::Amber => crate::neonui::AMBER,
            Hue::Green => crate::neonui::GREEN,
        };
        let l = c.to_linear();
        Vec3::new(l.red, l.green, l.blue)
    }
}

/// A colour's materials: lines and text, and fills (a button as it is, pointed at, chosen)
/// and a soft glow around a panel.
struct Tint {
    line: Handle<AccentMaterial>,
    text: Handle<AccentMaterial>,
    idle: Handle<GlowMaterial>,
    hot: Handle<GlowMaterial>,
    chosen: Handle<GlowMaterial>,
    aura: Handle<GlowMaterial>,
}

fn make_mats(
    mut commands: Commands,
    mut accents: ResMut<Assets<AccentMaterial>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let fixed = |accents: &mut Assets<AccentMaterial>, c: Vec3, i: f32| {
        accents.add(AccentMaterial {
            neon: NeonUniform::fixed(c, i),
        })
    };
    let glow = |glows: &mut Assets<GlowMaterial>, c: Vec3, i: f32| {
        glows.add(GlowMaterial {
            neon: NeonUniform::fixed(c, i),
        })
    };
    let teams = [Some(Team::Red), Some(Team::Blue), None];
    // A square glowing evenly (uv.y sets a halo's strength).
    let mut fill = MeshData::default();
    for (x, y) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
        fill.pos.push([x, y, 0.0]);
        fill.nrm.push([0.0, 0.0, 1.0]);
        fill.uv.push([0.0, 0.3]);
    }
    fill.idx.extend([0, 1, 2, 0, 2, 3]);
    let mats = UiMats {
        white: fixed(&mut accents, Vec3::ONE, 3.0),
        dim: fixed(&mut accents, Vec3::new(0.4, 0.44, 0.75), 2.0),
        gold: fixed(&mut accents, Vec3::new(1.0, 0.75, 0.2), 3.0),
        laser: fixed(&mut accents, Vec3::new(0.7, 0.8, 1.0), 1.5),
        laser_hot: fixed(&mut accents, Vec3::new(1.0, 0.85, 0.3), 4.0),
        team: teams.map(|t| fixed(&mut accents, team_color(t), 3.5)),
        glow: teams.map(|t| glow(&mut glows, team_color(t), 1.5)),
        mine: teams.map(|t| glow(&mut glows, team_color(t), 2.0)),
        back: std.add(StandardMaterial {
            base_color: Color::linear_rgb(0.004, 0.004, 0.012),
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            ..default()
        }),
        flash: (0..SECTORS)
            .map(|_| glow(&mut glows, Vec3::new(1.0, 0.04, 0.02), 0.0))
            .collect(),
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        fill: meshes.add(fill.to_mesh()),
        tints: Hue::ALL
            .iter()
            .map(|h| {
                let c = h.color();
                Tint {
                    line: fixed(&mut accents, c, 2.6),
                    text: fixed(&mut accents, c, 3.2),
                    idle: glow(&mut glows, c, 0.3),
                    hot: glow(&mut glows, c, 1.8),
                    chosen: glow(&mut glows, c, 2.6),
                    aura: glow(&mut glows, c, 0.9),
                }
            })
            .collect(),
    };
    commands.insert_resource(mats);
}

/// Builds panels' parts.
struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    meshes: &'a mut Assets<Mesh>,
    mats: &'a UiMats,
    root: Entity,
}

/// A rectangle glowing softly, strongest inside, fading out over `feather` past `size`.
fn soft_rect(size: Vec2, feather: f32) -> MeshData {
    let mut m = MeshData::default();
    let (i, o) = (size / 2.0, size / 2.0 + Vec2::splat(feather));
    for (h, v) in [(i, 0.0), (o, 1.0)] {
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            m.pos.push([x * h.x, y * h.y, 0.0]);
            m.nrm.push([0.0, 0.0, 1.0]);
            m.uv.push([0.0, v]);
        }
    }
    m.idx.extend([0, 1, 2, 0, 2, 3]);
    for k in 0..4u32 {
        let n = (k + 1) % 4;
        m.idx.extend([k, 4 + k, 4 + n, k, 4 + n, n]);
    }
    m
}

/// Text weights (as the flat screen's: src/neonui.rs).
const TEXT: u16 = 600;
const BOLD: u16 = 800;
const BLACK: u16 = 900;

impl Builder<'_, '_, '_> {
    fn tint(&self, h: Hue) -> &Tint {
        &self.mats.tints[h as usize]
    }

    /// Text in the neon font, its capitals `h` high, with a soft glow behind it if asked.
    fn text(
        &mut self,
        s: &str,
        h: f32,
        at: Vec2,
        align: Align,
        mat: &Handle<AccentMaterial>,
        glow: Option<&Handle<GlowMaterial>>,
    ) -> Entity {
        self.text_w(s, h, TEXT, at, align, mat, glow)
    }

    #[allow(clippy::too_many_arguments)]
    fn text_w(
        &mut self,
        s: &str,
        h: f32,
        weight: u16,
        at: Vec2,
        align: Align,
        mat: &Handle<AccentMaterial>,
        glow: Option<&Handle<GlowMaterial>>,
    ) -> Entity {
        let w = vfont::width(s, h, weight);
        let x = match align {
            Align::Left => 0.0,
            Align::Center => -w / 2.0,
            Align::Right => -w,
        };
        let root = self
            .commands
            .spawn((
                Transform::from_translation(Vec3::new(at.x + x, at.y, 0.004)),
                Visibility::default(),
                ChildOf(self.root),
            ))
            .id();
        let m = vfont::mesh(s, h, weight);
        if !m.idx.is_empty() {
            self.commands.spawn((
                Mesh3d(self.meshes.add(m.to_mesh())),
                MeshMaterial3d(mat.clone()),
                Transform::IDENTITY,
                ChildOf(root),
            ));
        }
        if let Some(g) = glow {
            self.commands.spawn((
                Mesh3d(
                    self.meshes
                        .add(soft_rect(Vec2::new(w, h * 0.6), h).to_mesh()),
                ),
                MeshMaterial3d(g.clone()),
                Transform::from_xyz(w / 2.0, 0.0, -0.002),
                ChildOf(root),
            ));
        }
        root
    }

    /// A strip from `a` to `b`.
    fn strip(&mut self, a: Vec2, b: Vec2, w: f32, mat: &Handle<AccentMaterial>) {
        let d = b - a;
        self.commands.spawn((
            Mesh3d(self.mats.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(((a + b) / 2.0).extend(0.002))
                .with_rotation(Quat::from_rotation_z(d.y.atan2(d.x)))
                .with_scale(Vec3::new(d.length() + w, w, 0.004)),
            ChildOf(self.root),
        ));
    }

    fn outline(&mut self, c: Vec2, size: Vec2, w: f32, mat: &Handle<AccentMaterial>) {
        let h = size / 2.0;
        let corners = [
            c + Vec2::new(-h.x, -h.y),
            c + Vec2::new(h.x, -h.y),
            c + Vec2::new(h.x, h.y),
            c + Vec2::new(-h.x, h.y),
        ];
        for k in 0..4 {
            self.strip(corners[k], corners[(k + 1) % 4], w, mat);
        }
    }

    fn fill(&mut self, c: Vec2, size: Vec2, mat: &Handle<GlowMaterial>, shown: bool) -> Entity {
        self.commands
            .spawn((
                Mesh3d(self.mats.fill.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(c.extend(0.001)).with_scale(size.extend(1.0)),
                if shown {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                ChildOf(self.root),
            ))
            .id()
    }

    /// A soft glow around a rectangle (as the flat screen's box shadows).
    fn aura(&mut self, c: Vec2, size: Vec2, feather: f32, mat: &Handle<GlowMaterial>, z: f32) {
        self.commands.spawn((
            Mesh3d(self.meshes.add(soft_rect(size, feather).to_mesh())),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(c.extend(z)),
            ChildOf(self.root),
        ));
    }

    /// The dark backing, framed and glowing in `hue`.
    fn backing(&mut self, size: Vec2, hue: Hue) {
        self.commands.spawn((
            Mesh3d(self.meshes.add(Rectangle::new(size.x, size.y))),
            MeshMaterial3d(self.mats.back.clone()),
            Transform::from_xyz(0.0, 0.0, -0.004),
            ChildOf(self.root),
        ));
        let (line, aura) = (self.tint(hue).line.clone(), self.tint(hue).aura.clone());
        self.aura(Vec2::ZERO, size, 0.07, &aura, -0.006);
        self.outline(Vec2::ZERO, size, 0.005, &line);
    }

    /// A title in pink, black weight, with a glowing line under it (as the flat screen's).
    fn title(&mut self, s: &str, h: f32, y: f32) {
        let pink = self.tint(Hue::Pink).text.clone();
        let (line, aura) = (
            self.tint(Hue::Pink).line.clone(),
            self.tint(Hue::Pink).aura.clone(),
        );
        self.text_w(s, h, BLACK, Vec2::new(0.0, y), Align::Center, &pink, None);
        let w = vfont::width(s, h, BLACK);
        let ly = y - h * 0.85;
        self.strip(
            Vec2::new(-w / 2.0, ly),
            Vec2::new(w / 2.0, ly),
            0.004,
            &line,
        );
        self.aura(Vec2::new(0.0, ly), Vec2::new(w, 0.004), 0.025, &aura, 0.001);
    }

    /// A button in `hue`: a thin frame, a faint fill, brighter when pointed at, lit when it
    /// shows the current choice.
    fn button(
        &mut self,
        label: &str,
        act: Act,
        c: Vec2,
        size: Vec2,
        hue: Hue,
        chosen: bool,
    ) -> Button {
        let t = self.tint(hue);
        let (line, idle, hot, lit) = (
            t.line.clone(),
            t.idle.clone(),
            t.hot.clone(),
            t.chosen.clone(),
        );
        let white = self.mats.white.clone();
        self.outline(c, size, 0.004, &line);
        self.fill(c, size, if chosen { &lit } else { &idle }, true);
        let hover = self.fill(c + Vec2::new(0.0, 0.0), size, &hot, false);
        // As big as fits.
        let h = (size.y * 0.36).min(size.x * 0.84 / vfont::width(label, 1.0, TEXT).max(0.01));
        self.text_w(label, h, TEXT, c, Align::Center, &white, None);
        Button {
            act,
            center: c,
            half: size / 2.0,
            hover,
        }
    }
}

fn team_index(t: Option<Team>) -> usize {
    t.map_or(2, |t| t.index())
}

/// The name as the segment font shows it, at most `n` characters.
fn short(name: &str, n: usize) -> String {
    name.chars().take(n).collect::<String>().to_uppercase()
}

fn build_menu(b: &mut Builder, local: bool) -> Vec<Button> {
    let size = Vec2::new(0.8, if local { 0.88 } else { 0.72 });
    b.backing(size, Hue::Magenta);
    let top = size.y / 2.0;
    b.title("MENU", 0.06, top - 0.1);
    let mut acts = vec![("RESUME", Act::Resume, Hue::Cyan)];
    if local {
        acts.push(("NEW MATCH", Act::NewMatch, Hue::Amber));
    }
    acts.push(("MAIN MENU", Act::Leave, Hue::Magenta));
    acts.push(("QUIT", Act::Quit, Hue::Pink));
    acts.iter()
        .enumerate()
        .map(|(k, (label, act, hue))| {
            b.button(
                label,
                act.clone(),
                Vec2::new(0.0, top - 0.25 - k as f32 * 0.16),
                Vec2::new(0.56, 0.12),
                *hue,
                false,
            )
        })
        .collect()
}

/// A row of buttons `w` wide, centred at `y` (the chosen ones lit).
fn button_row(
    b: &mut Builder,
    buttons: &mut Vec<Button>,
    items: Vec<(String, Act, Hue, bool)>,
    y: f32,
    w: f32,
    h: f32,
) {
    let pitch = w + 0.02;
    let x0 = -(items.len() as f32 - 1.0) * pitch / 2.0;
    for (k, (name, act, hue, on)) in items.into_iter().enumerate() {
        let c = Vec2::new(x0 + k as f32 * pitch, y);
        buttons.push(b.button(&name, act, c, Vec2::new(w, h), hue, on));
    }
}

fn menu(a: MenuAct) -> Act {
    Act::Menu(a)
}

/// The bots' number and skill, the arena's size, the mode, from `y` down; returns the y below.
fn lobby_choices(b: &mut Builder, buttons: &mut Vec<Button>, m: &MenuModel, mut y: f32) -> f32 {
    let dim = b.mats.dim.clone();
    let white = b.mats.white.clone();
    let label = |b: &mut Builder, s: &str, y: f32| {
        b.text_w(s, 0.022, BOLD, Vec2::new(0.0, y), Align::Center, &dim, None);
    };
    label(b, "BOTS", y);
    y -= 0.07;
    buttons.push(b.button(
        "-",
        menu(MenuAct::MoreBots(-1)),
        Vec2::new(-0.2, y),
        Vec2::new(0.12, 0.09),
        Hue::Violet,
        false,
    ));
    b.text_w(
        &m.bots.to_string(),
        0.04,
        BOLD,
        Vec2::new(0.0, y),
        Align::Center,
        &white,
        None,
    );
    buttons.push(b.button(
        "+",
        menu(MenuAct::MoreBots(1)),
        Vec2::new(0.2, y),
        Vec2::new(0.12, 0.09),
        Hue::Violet,
        false,
    ));
    y -= 0.09;
    let item = |name: &str, a: MenuAct| (name.to_uppercase(), a.clone(), m.selected(&a));
    label(b, "SKILL", y);
    let skills = Difficulty::ALL
        .iter()
        .map(|&d| item(d.name(), MenuAct::Difficulty(d)))
        .collect::<Vec<_>>();
    let skills = skills
        .into_iter()
        .map(|(n, a, on)| (n, menu(a), Hue::Cyan, on))
        .collect();
    button_row(b, buttons, skills, y - 0.07, 0.3, 0.09);
    y -= 0.16;
    label(b, "ARENA", y);
    let arenas = SizeChoice::ALL
        .iter()
        .map(|&a| item(a.name(), MenuAct::Arena(a)))
        .map(|(n, a, on)| (n, menu(a), Hue::Cyan, on))
        .collect();
    button_row(b, buttons, arenas, y - 0.07, 0.24, 0.09);
    y -= 0.16;
    label(b, "MODE", y);
    let modes = GameMode::ALL
        .iter()
        .map(|&g| item(g.name(), MenuAct::Mode(g)))
        .map(|(n, a, on)| (n, menu(a), Hue::Cyan, on))
        .collect();
    button_row(b, buttons, modes, y - 0.07, 0.42, 0.09);
    y - 0.16
}

/// A server list's rows (at most `ROWS`), from `y` down.
fn lobby_list(b: &mut Builder, buttons: &mut Vec<Button>, m: &MenuModel, y: f32, width: f32) {
    const ROWS: usize = 6;
    let dim = b.mats.dim.clone();
    let say = |b: &mut Builder, s: &str| {
        b.text(
            s,
            0.026,
            Vec2::new(0.0, y - 0.05),
            Align::Center,
            &dim,
            None,
        );
    };
    let mut rows: Vec<(String, String)> = Vec::new();
    let row = |r: &lt_net::ServerReport| {
        let mode = if r.mode.is_empty() {
            String::new()
        } else {
            format!(" {}", r.mode.to_uppercase())
        };
        format!(
            "{}  {}+{}{mode}",
            short(&r.name, 20),
            r.players.len(),
            r.bots
        )
    };
    if m.screen == Screen::Lan {
        if m.lan_error.is_some() {
            say(b, "CAN'T LISTEN ON THE NETWORK");
        } else if m.lan_servers.is_empty() {
            say(b, "SEARCHING THE NETWORK...");
        }
        for s in m.lan_servers.iter().filter(|s| s.joinable()).take(ROWS) {
            rows.push((row(&s.report), s.addr.clone()));
        }
    } else {
        match &m.internet {
            InternetList::Idle | InternetList::Asking => say(b, "ASKING THE MASTER SERVER..."),
            InternetList::Failed(_) => say(b, "SERVER LIST UNAVAILABLE"),
            InternetList::Listed(l) if l.is_empty() => say(b, "NO SERVERS: HOST ONE!"),
            InternetList::Listed(l) => {
                for s in l
                    .iter()
                    .filter(|s| s.report.protocol == lt_net::protocol::PROTOCOL)
                    .take(ROWS)
                {
                    rows.push((row(&s.report), s.addr.clone()));
                }
            }
        }
    }
    for (k, (text, addr)) in rows.into_iter().enumerate() {
        buttons.push(b.button(
            &text,
            menu(MenuAct::Join(addr)),
            Vec2::new(0.0, y - 0.05 - k as f32 * 0.085),
            Vec2::new(width, 0.075),
            Hue::Cyan,
            false,
        ));
    }
}

/// The start menu (src/menu.rs's levels; no typing, so no join by address and the server's
/// name as it is).
fn build_lobby(b: &mut Builder, m: &MenuModel) -> Vec<Button> {
    let size = Vec2::new(1.1, 1.12);
    // The panel's colour, as the flat screen's for this level.
    let hue = match m.screen {
        Screen::Botmatch | Screen::Internet | Screen::Host { internet: true } => Hue::Cyan,
        _ => Hue::Magenta,
    };
    b.backing(size, hue);
    let top = size.y / 2.0;
    let (cyan, dim) = (b.tint(Hue::Cyan).text.clone(), b.mats.dim.clone());
    b.title("NEON LASERTAG", 0.058, top - 0.085);
    b.text_w(
        m.screen.crumb(),
        0.025,
        BOLD,
        Vec2::new(0.0, top - 0.17),
        Align::Center,
        &cyan,
        None,
    );
    let mut buttons = Vec::new();
    let big = |b: &mut Builder, buttons: &mut Vec<Button>, items: &[(&str, MenuAct, Hue)]| {
        for (k, (label, a, hue)) in items.iter().enumerate() {
            buttons.push(b.button(
                label,
                menu(a.clone()),
                Vec2::new(0.0, top - 0.34 - k as f32 * 0.16),
                Vec2::new(0.6, 0.12),
                *hue,
                false,
            ));
        }
    };
    // Back (and the level's way on) at the bottom.
    let bottom = top - 1.03;
    let back = |b: &mut Builder, buttons: &mut Vec<Button>, go: Option<(&str, MenuAct, Hue)>| {
        let items = std::iter::once(("BACK".to_string(), menu(MenuAct::Back), Hue::Violet, false))
            .chain(go.map(|(l, a, h)| (l.to_string(), menu(a), h, false)))
            .collect();
        button_row(b, buttons, items, bottom, 0.46, 0.09);
    };
    match m.screen {
        Screen::Top => big(
            b,
            &mut buttons,
            &[
                ("BOTMATCH", MenuAct::Go(Screen::Botmatch), Hue::Cyan),
                (
                    "MULTIPLAYER",
                    MenuAct::Go(Screen::Multiplayer),
                    Hue::Magenta,
                ),
                ("QUIT", MenuAct::Quit, Hue::Pink),
            ],
        ),
        Screen::Botmatch => {
            lobby_choices(b, &mut buttons, m, top - 0.27);
            back(
                b,
                &mut buttons,
                Some(("START", MenuAct::StartBots, Hue::Cyan)),
            );
        }
        Screen::Multiplayer => {
            big(
                b,
                &mut buttons,
                &[
                    ("INTERNET", MenuAct::Go(Screen::Internet), Hue::Cyan),
                    ("LAN", MenuAct::Go(Screen::Lan), Hue::Magenta),
                ],
            );
            back(b, &mut buttons, None);
        }
        Screen::Internet | Screen::Lan => {
            let internet = m.screen == Screen::Internet;
            lobby_list(b, &mut buttons, m, top - 0.22, 0.96);
            if internet {
                buttons.push(b.button(
                    "REFRESH",
                    menu(MenuAct::Refresh),
                    Vec2::new(0.0, bottom + 0.12),
                    Vec2::new(0.3, 0.08),
                    Hue::Violet,
                    false,
                ));
            }
            let host = if internet { "HOST ONLINE" } else { "HOST LAN" };
            back(
                b,
                &mut buttons,
                Some((host, MenuAct::Go(Screen::Host { internet }), Hue::Amber)),
            );
        }
        Screen::Host { internet } => {
            let green = b.tint(Hue::Green).text.clone();
            b.text(
                &short(&m.server_name, 28),
                0.03,
                Vec2::new(0.0, top - 0.23),
                Align::Center,
                &green,
                None,
            );
            lobby_choices(b, &mut buttons, m, top - 0.3);
            let note = if internet {
                "LISTED ONLINE, PORT OPENED BY UPNP"
            } else {
                "FOUND UNDER LAN"
            };
            b.text(
                note,
                0.022,
                Vec2::new(0.0, bottom + 0.09),
                Align::Center,
                &dim,
                None,
            );
            back(
                b,
                &mut buttons,
                Some(("START HOSTING", MenuAct::StartHost, Hue::Amber)),
            );
        }
    }
    buttons
}

/// Column right edges (score, tags, out, accuracy, targets) on the end panel.
const COLUMNS: [f32; 6] = [-0.06, 0.05, 0.15, 0.25, 0.36, 0.46];
const ROW: f32 = 0.042;
const AWARD_ROW: f32 = 0.038;

fn team_block(b: &mut Builder, team: Team, score: u32, rows: &[Row], top: f32, width: f32) -> f32 {
    let k = team.index();
    let (mat, glow, dim) = (
        b.mats.team[k].clone(),
        b.mats.glow[k].clone(),
        b.mats.dim.clone(),
    );
    let left = -width / 2.0 + 0.05;
    let right = width / 2.0 - 0.04;
    let name = if team == Team::Red { "RED" } else { "BLUE" };
    let mut y = top - 0.03;
    b.text(
        name,
        0.04,
        Vec2::new(left, y),
        Align::Left,
        &mat,
        Some(&glow),
    );
    b.text(
        &score.to_string(),
        0.04,
        Vec2::new(right, y),
        Align::Right,
        &mat,
        Some(&glow),
    );
    y -= 0.04;
    b.strip(Vec2::new(left, y), Vec2::new(right, y), 0.005, &mat);
    y -= 0.03;
    b.text("PLAYER", 0.018, Vec2::new(left, y), Align::Left, &dim, None);
    for (label, x) in ["SCORE", "TAGS", "OUT", "AST", "ACC", "TGT"]
        .iter()
        .zip(COLUMNS)
    {
        b.text(label, 0.018, Vec2::new(x, y), Align::Right, &dim, None);
    }
    y -= ROW;
    for r in rows {
        let text = if r.me {
            b.mats.white.clone()
        } else {
            b.mats.dim.clone()
        };
        if r.me {
            let mine = b.mats.mine[k].clone();
            b.fill(
                Vec2::new(0.0, y),
                Vec2::new(width - 0.04, ROW * 0.9),
                &mine,
                true,
            );
        }
        b.text(
            &short(&r.name, 10),
            0.024,
            Vec2::new(left, y),
            Align::Left,
            &text,
            None,
        );
        let acc = r.accuracy.map_or("-".to_string(), |a| format!("{a}%"));
        let cells = [
            r.score.to_string(),
            r.tags.to_string(),
            r.tagged.to_string(),
            r.assists.to_string(),
            acc,
            r.target_hits.to_string(),
        ];
        for (c, x) in cells.iter().zip(COLUMNS) {
            b.text(c, 0.024, Vec2::new(x, y), Align::Right, &text, None);
        }
        y -= ROW;
    }
    y - 0.02
}

/// The end panel; returns its buttons and where the countdown goes.
fn build_end(b: &mut Builder, game: &Game) -> (Vec<Button>, Vec3) {
    let rows = standings(game);
    let won = crate::endscreen::awards(&game.snap.players);
    let local = game.is_local();
    let width = 1.0;
    let n = rows[0].len() + rows[1].len();
    let height = 0.06
        + 0.13
        + 0.09
        + 2.0 * (0.12 + 0.02)
        + n as f32 * ROW
        + if won.is_empty() {
            0.0
        } else {
            0.02 + won.len() as f32 * AWARD_ROW
        }
        + 0.06
        + if local { 0.14 } else { 0.0 }
        + 0.03;
    b.backing(Vec2::new(width, height), Hue::Violet);
    let (text, team) = winner(game.snap.scores);
    let ti = team_index(team);
    let (mat, glow) = (b.mats.team[ti].clone(), b.mats.glow[ti].clone());
    let mut y = height / 2.0 - 0.06 - 0.055;
    b.text(
        text,
        0.11,
        Vec2::new(0.0, y),
        Align::Center,
        &mat,
        Some(&glow),
    );
    y -= 0.12;
    let [r, bl] = game.snap.scores;
    let (red, blue, white) = (
        b.mats.team[0].clone(),
        b.mats.team[1].clone(),
        b.mats.white.clone(),
    );
    b.text(
        &r.to_string(),
        0.055,
        Vec2::new(-0.06, y),
        Align::Right,
        &red,
        None,
    );
    b.text(":", 0.055, Vec2::new(0.0, y), Align::Center, &white, None);
    b.text(
        &bl.to_string(),
        0.055,
        Vec2::new(0.06, y),
        Align::Left,
        &blue,
        None,
    );
    y -= 0.06;
    y = team_block(b, Team::Red, r, &rows[0], y, width);
    y = team_block(b, Team::Blue, bl, &rows[1], y, width);
    // The awards: title, player (in their team's colour) and what for.
    if !won.is_empty() {
        y -= 0.02;
        let gold = b.mats.gold.clone();
        let dim = b.mats.dim.clone();
        for a in &won {
            let k = a.team.index();
            let team = b.mats.team[k].clone();
            b.text(a.title, 0.02, Vec2::new(-0.45, y), Align::Left, &gold, None);
            b.text(
                &short(game.name_of(a.id), 10),
                0.022,
                Vec2::new(-0.13, y),
                Align::Left,
                &team,
                None,
            );
            b.text(
                &a.detail,
                0.016,
                Vec2::new(0.46, y),
                Align::Right,
                &dim,
                None,
            );
            y -= AWARD_ROW;
        }
    }
    let countdown_at = Vec3::new(0.0, y - 0.02, 0.004);
    let mut buttons = Vec::new();
    if local {
        buttons.push(b.button(
            "NEXT MATCH",
            Act::NewMatch,
            Vec2::new(0.0, y - 0.12),
            Vec2::new(0.44, 0.1),
            Hue::Cyan,
            false,
        ));
    }
    (buttons, countdown_at)
}

#[allow(clippy::too_many_arguments)]
fn panels(
    mut commands: Commands,
    mut ui: ResMut<VrUi>,
    mut vr: ResMut<VrState>,
    input: Res<XrInput>,
    game: Option<Res<Game>>,
    starting: Option<Res<crate::menu::Start>>,
    connecting: Option<Res<crate::Connecting>>,
    model: Res<MenuModel>,
    session: Res<Session>,
    mats: Res<UiMats>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<&mut Transform>,
) {
    if !ui.started {
        ui.started = true;
        ui.menu = std::env::var("LT_VR_MENU").is_ok_and(|v| v == "1");
    }
    if vr.menu_requests > 0 {
        vr.menu_requests = 0;
        ui.menu = !ui.menu;
    }
    let lobby = game.is_none() && starting.is_none() && connecting.is_none();
    let want = match game.as_deref() {
        _ if lobby => Some(Kind::Lobby),
        None => None,
        Some(_) if ui.menu => Some(Kind::Menu),
        Some(g) if g.snap.over => Some(Kind::End),
        Some(_) => None,
    };
    if lobby {
        ui.menu = false;
    }
    // The start menu is built anew when its choices change, or once the head is tracked.
    let stale = ui.panel.as_ref().is_some_and(|p| {
        p.kind == Kind::Lobby
            && (p.shows != Some(lobby_key(&model)) || p.headless && input.head.is_some())
    });
    // In front of the eyes, level, a little below them.
    let in_front = || {
        let head = input
            .head
            .unwrap_or(Transform::from_xyz(0.0, lt_sim::sim::EYE, 0.0));
        let f = head.rotation * Vec3::NEG_Z;
        let rot = Quat::from_rotation_y(lt_sim::sim::yaw_of(Vec3::new(f.x, 0.0, f.z)));
        Transform::from_translation(head.translation + rot * Vec3::new(0.0, -0.12, -PANEL_DISTANCE))
            .with_rotation(rot)
    };
    if ui.panel.as_ref().map(|p| p.kind) != want || stale {
        // (The start menu built anew for its choices stays where it was, unless it was placed
        // before the head was tracked.)
        let kept = ui
            .panel
            .as_ref()
            .filter(|p| Some(p.kind) == want && !(p.headless && input.head.is_some()))
            .map(|p| (p.pose, p.headless));
        if let Some(p) = ui.panel.take() {
            commands.entity(p.root).despawn();
        }
        ui.pointed = [None; 2];
        if let Some(kind) = want {
            let (pose, headless) = kept.unwrap_or_else(|| (in_front(), input.head.is_none()));
            let root = commands
                .spawn((Name::new("VR panel"), vr.rig * pose, Visibility::default()))
                .id();
            let mut b = Builder {
                commands: &mut commands,
                meshes: &mut meshes,
                mats: &mats,
                root,
            };
            let (buttons, countdown_at) = match (kind, game.as_deref()) {
                (Kind::Lobby, _) => (build_lobby(&mut b, &model), Vec3::ZERO),
                (Kind::Menu, Some(g)) => (build_menu(&mut b, g.is_local()), Vec3::ZERO),
                (Kind::End, Some(g)) => build_end(&mut b, g),
                _ => (Vec::new(), Vec3::ZERO),
            };
            ui.panel = Some(Panel {
                kind,
                root,
                pose,
                buttons,
                countdown: None,
                countdown_at,
                shows: (kind == Kind::Lobby).then(|| lobby_key(&model)),
                headless,
            });
        }
    }
    vr.ui = ui.panel.is_some();
    let Some(p) = ui.panel.as_mut() else { return };
    if vr.recenter {
        p.pose = in_front();
        p.headless = input.head.is_none();
    }
    if let Ok(mut t) = transforms.get_mut(p.root) {
        *t = vr.rig * p.pose;
    }
    // The end panel's countdown.
    if p.kind == Kind::End
        && let Some(game) = game
    {
        let secs = if game.is_local() {
            session.next_in().map_or(-1, |s| s.ceil() as i32)
        } else {
            -2
        };
        if p.countdown.map(|c| c.1) != Some(secs) {
            if let Some((e, _)) = p.countdown.take() {
                commands.entity(e).despawn();
            }
            let text = match secs {
                -2 => "NEXT ARENA SOON".to_string(),
                s if s >= 0 => format!("NEXT MATCH IN {s}"),
                _ => String::new(),
            };
            let mut b = Builder {
                commands: &mut commands,
                meshes: &mut meshes,
                mats: &mats,
                root: p.root,
            };
            let dim = b.mats.dim.clone();
            let e = b.text(
                &text,
                0.026,
                p.countdown_at.truncate(),
                Align::Center,
                &dim,
                None,
            );
            p.countdown = Some((e, secs));
        }
    }
}

/// The lasers from the hands, pointing at the panel's buttons; the trigger presses them.
#[allow(clippy::too_many_arguments)]
fn pointer(
    mut commands: Commands,
    mut ui: ResMut<VrUi>,
    vr: Res<VrState>,
    input: Res<XrInput>,
    mats: Res<UiMats>,
    mut rumble: ResMut<Rumble>,
    mut cmds: MessageWriter<SessionCmd>,
    mut model: ResMut<MenuModel>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
    mut laser_mats: Query<&mut MeshMaterial3d<AccentMaterial>>,
) {
    let lasers = *ui.lasers.get_or_insert_with(|| {
        [0, 1].map(|_| {
            commands
                .spawn((
                    Mesh3d(mats.cube.clone()),
                    MeshMaterial3d(mats.laser.clone()),
                    Transform::IDENTITY,
                    Visibility::Hidden,
                ))
                .id()
        })
    });
    let panel_pose = ui.panel.as_ref().map(|p| {
        (
            vr.rig * p.pose,
            p.buttons
                .iter()
                .map(|b| (b.center, b.half))
                .collect::<Vec<_>>(),
        )
    });
    let mut pressed = None;
    let mut pointed = [None; 2];
    for i in 0..2 {
        let hand = &input.hands[i];
        let aim = hand.aim.or(hand.grip);
        let mut shown = false;
        if let (Some((pose, buttons)), Some(aim)) = (&panel_pose, aim) {
            let w = vr.rig * aim;
            let o = w.translation;
            let d = w.rotation * Vec3::NEG_Z;
            // Into the panel's plane (it faces +Z).
            let inv = pose.compute_affine().inverse();
            let lo = inv.transform_point3(o);
            let ld = inv.transform_vector3(d);
            let mut len = 3.0;
            if ld.z < -1e-4 && lo.z > 0.0 {
                let t = -lo.z / ld.z;
                let hit = (lo + ld * t).truncate();
                len = t;
                pointed[i] = buttons
                    .iter()
                    .position(|(c, h)| (hit - *c).abs().cmplt(*h).all());
            }
            if let Ok(mut t) = transforms.get_mut(lasers[i]) {
                *t = Transform::from_translation(o + d * len / 2.0)
                    .with_rotation(w.rotation)
                    .with_scale(Vec3::new(0.003, 0.003, len));
            }
            shown = true;
            // Pressed with the trigger.
            let held = if ui.trigger[i] {
                hand.trigger > 0.5
            } else {
                hand.trigger > 0.75
            };
            if held
                && !ui.trigger[i]
                && let Some(k) = pointed[i]
            {
                pressed = Some(k);
                rumble.play(i, 0.0, 0.03, 0.6);
            }
            if pointed[i].is_some() && pointed[i] != ui.pointed[i] {
                rumble.play(i, 0.0, 0.01, 0.15);
            }
        }
        ui.trigger[i] = if ui.trigger[i] {
            hand.trigger > 0.5
        } else {
            hand.trigger > 0.75
        };
        if let Ok(mut v) = vis.get_mut(lasers[i]) {
            let want = if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
        }
        if let Ok(mut m) = laser_mats.get_mut(lasers[i]) {
            let want = if pointed[i].is_some() {
                mats.laser_hot.clone()
            } else {
                mats.laser.clone()
            };
            if m.0 != want {
                m.0 = want;
            }
        }
    }
    ui.pointed = pointed;
    let Some(panel) = ui.panel.as_ref() else {
        return;
    };
    for (k, b) in panel.buttons.iter().enumerate() {
        if let Ok(mut v) = vis.get_mut(b.hover) {
            let want = if pointed.contains(&Some(k)) {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *v != want {
                *v = want;
            }
        }
    }
    let Some(act) = pressed
        .and_then(|k| panel.buttons.get(k))
        .map(|b| b.act.clone())
    else {
        return;
    };
    match act {
        Act::Resume => ui.menu = false,
        Act::NewMatch => {
            ui.menu = false;
            cmds.write(SessionCmd::NextMatch);
        }
        Act::Leave => {
            ui.menu = false;
            cmds.write(SessionCmd::Leave);
        }
        Act::Quit => {
            cmds.write(SessionCmd::Quit);
        }
        Act::Menu(a) => match model.apply(&a) {
            Outcome::Start(start) => commands.insert_resource(start),
            Outcome::Quit => {
                cmds.write(SessionCmd::Quit);
            }
            Outcome::Stay | Outcome::Options => {}
        },
    }
}

/// A patch of the hit ring, centred on `angle` (clockwise from the top), `span` wide, in front
/// of the eyes at `z`: brightest a third of the way in from its outer edge, fading to the
/// edges (neighbours overlap by half).
fn sector(angle: f32, span: f32, r0: f32, r1: f32, z: f32) -> MeshData {
    let mut m = MeshData::default();
    let (n, rings) = (8, 3);
    for k in 0..=n {
        let s = k as f32 / n as f32 * 2.0 - 1.0;
        let a = angle + span / 2.0 * s;
        let dir = Vec2::new(a.sin(), a.cos());
        for j in 0..=rings {
            let t = j as f32 / rings as f32;
            let p = dir * (r1 + (r0 - r1) * t);
            m.pos.push([p.x, p.y, z]);
            m.nrm.push([0.0, 0.0, 1.0]);
            // (A halo's strength falls with uv.y.)
            let radial = if t < 1.0 / 3.0 {
                t * 3.0
            } else {
                (1.0 - t) * 1.5
            };
            m.uv.push([0.0, 1.0 - radial * (1.0 - s.abs())]);
        }
    }
    let w = rings as u32 + 1;
    for k in 0..n as u32 {
        for j in 0..rings as u32 {
            let b = k * w + j;
            m.idx.extend([b, b + 1, b + w + 1, b, b + w + 1, b + w]);
        }
    }
    m
}

/// An arena event's announcement (as the flat screen's: `hud::event_banner_for`) floating in
/// front of the eyes for a few seconds: the banner's root and its seconds left.
#[allow(clippy::too_many_arguments)]
fn event_banner(
    mut commands: Commands,
    mut events: MessageReader<GameEvent>,
    vr: Res<VrState>,
    input: Res<XrInput>,
    mats: Res<UiMats>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<&mut Transform>,
    mut banner: Local<Option<Entity>>,
    mut queue: Local<crate::hud::BannerQueue>,
    game: Res<Game>,
) {
    for GameEvent(e) in events.read() {
        if let Some(b) = crate::hud::event_banner_for(e, &game.rules) {
            queue.push(b);
        }
    }
    if let Some(b) = queue.tick(time.delta_secs()) {
        if let Some(old) = banner.take()
            && let Ok(mut c) = commands.get_entity(old)
        {
            c.despawn();
        }
        let root = commands
            .spawn((
                Name::new("event banner"),
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .id();
        // (Pale blue for the blackout's: white.)
        let c = b.color.to_srgba();
        let mat = if c.blue > c.red {
            &mats.white
        } else {
            &mats.gold
        };
        segtext::spawn(
            &mut commands,
            &mut meshes,
            root,
            b.title,
            0.045,
            Vec3::ZERO,
            Align::Center,
            mat,
            Some(&mats.glow[2]),
        );
        if !b.sub.is_empty() {
            segtext::spawn(
                &mut commands,
                &mut meshes,
                root,
                &b.sub,
                0.014,
                Vec3::Y * -0.05,
                Align::Center,
                &mats.white,
                None,
            );
        }
        *banner = Some(root);
    }
    let Some(root) = *banner else {
        return;
    };
    let left = queue.left;
    if left <= 0.0 {
        if let Ok(mut c) = commands.get_entity(root) {
            c.despawn();
        }
        *banner = None;
        return;
    }
    // In front of the eyes, a little above, level, turned to them; swelling in.
    let head = vr.rig
        * input
            .head
            .unwrap_or(Transform::from_xyz(0.0, lt_sim::sim::EYE, 0.0));
    let fwd = head.rotation * Vec3::NEG_Z;
    let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
    if let Ok(mut t) = transforms.get_mut(root) {
        // (Near, so that walls close by don't hide it.)
        t.translation = head.translation + fwd * 0.8 + Vec3::Y * 0.12;
        // (Its text faces +Z: turned about the vertical towards the eyes.)
        t.rotation = Quat::from_rotation_y((-fwd.x).atan2(-fwd.z));
        t.scale = Vec3::splat((queue.shown / 0.15).min(1.0) * (left / 0.3).min(1.0));
    }
}

#[allow(clippy::too_many_arguments)]
fn hit_flash(
    mut commands: Commands,
    mut ui: ResMut<VrUi>,
    mut events: MessageReader<GameEvent>,
    game: Res<Game>,
    vr: Res<VrState>,
    input: Res<XrInput>,
    mats: Res<UiMats>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut glows: ResMut<Assets<GlowMaterial>>,
    mut transforms: Query<&mut Transform>,
    mut vis: Query<&mut Visibility>,
) {
    let ring = *ui.ring.get_or_insert_with(|| {
        let root = commands
            .spawn((
                Name::new("hit flash"),
                Transform::IDENTITY,
                Visibility::Hidden,
            ))
            .id();
        let span = std::f32::consts::TAU / SECTORS as f32;
        for (k, m) in mats.flash.iter().enumerate() {
            let mesh = meshes.add(sector(k as f32 * span, span * 2.0, 0.42, 1.1, -0.9).to_mesh());
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(m.clone()),
                Transform::IDENTITY,
                ChildOf(root),
            ));
        }
        root
    });
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::Hit { player, by, .. } if Some(player) == game.me => {
                let from = crate::hud::shooter_pos(&game, by).map(|pos| HitFrom { by, pos });
                ui.hits
                    .retain(|h| h.0.is_none_or(|f| f.by != by) || from.is_none());
                ui.hits.push((from, FLASH_LIFE));
            }
            SimEvent::TaggedOut { player, .. } if Some(player) == game.me => {
                ui.hits.push((None, FLASH_LIFE * 1.5))
            }
            _ => {}
        }
    }
    let dt = time.delta_secs();
    for h in ui.hits.iter_mut() {
        h.1 -= dt;
        if let Some(f) = h.0.as_mut()
            && let Some(p) = game.snap.players.iter().find(|p| p.id == f.by)
        {
            f.pos = p.feet + p.head.pos;
        }
    }
    ui.hits.retain(|h| h.1 > 0.0);
    let head = vr.rig
        * input
            .head
            .unwrap_or(Transform::from_xyz(0.0, lt_sim::sim::EYE, 0.0));
    if let Ok(mut v) = vis.get_mut(ring) {
        let want = if ui.hits.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want {
            *v = want;
        }
    }
    if ui.hits.is_empty() {
        return;
    }
    if let Ok(mut t) = transforms.get_mut(ring) {
        *t = head;
    }
    let fwd = head.rotation * Vec3::NEG_Z;
    let fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
    let right = Vec3::new(-fwd.z, 0.0, fwd.x);
    let span = std::f32::consts::TAU / SECTORS as f32;
    for (k, m) in mats.flash.iter().enumerate() {
        let a = k as f32 * span;
        let mut level = 0.0;
        for (from, life) in &ui.hits {
            let fade = (life / FLASH_LIFE).min(1.0);
            let w = match from {
                Some(f) => {
                    let d = f.pos - head.translation;
                    let theta = d.dot(right).atan2(d.dot(fwd));
                    (a - theta).cos().max(0.0).powi(3)
                }
                None => 0.45,
            };
            level += w * fade * fade;
        }
        if let Some(mut g) = glows.get_mut(m) {
            g.neon.color.w = level.min(1.5) * 3.0;
        }
    }
}

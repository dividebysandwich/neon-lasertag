//! VR panels and the hit flash, built from neon meshes so both VR renderers draw them (the GL
//! headset renderer has no UI or text).
//!
//! - The start menu, out of a game (in the empty arena under fireworks, src/lobby.rs): play
//!   against bots or host a game, with the bots' number and skill and the arenas' size, or quit.
//! - The menu (the menu button, or B / Y held; see src/vr.rs): Resume, New match (local
//!   games: the next match on a new arena), Main menu and Quit, floating in front of the player.
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
use crate::segtext::{self, Align};
use crate::session::{Session, SessionCmd};
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
            .add_systems(Startup, |mut commands: Commands, args: Res<crate::Args>| {
                commands.insert_resource(LobbyChoices {
                    bots: args.bots,
                    difficulty: args.difficulty,
                    arena: args.arena,
                    mode: args.game_mode,
                });
            })
            .add_systems(
                Update,
                make_mats
                    .run_if(resource_exists::<Palette>.and_then(not(resource_exists::<UiMats>))),
            )
            .add_systems(
                Update,
                (panels, pointer, hit_flash.run_if(resource_exists::<Game>))
                    .chain()
                    .after(PoseSet)
                    .before(crate::game::DrawSet)
                    .run_if(resource_exists::<UiMats>.and_then(resource_exists::<LobbyChoices>)),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Resume,
    NewMatch,
    /// Leave the game for the start menu.
    Leave,
    Quit,
    /// The start menu's: play against bots or host, fewer or more bots, the bots' skill, the
    /// arenas' size.
    Play {
        host: bool,
    },
    Bots(i32),
    Skill(Difficulty),
    Arena(SizeChoice),
    Mode(GameMode),
}

/// The start menu's choices (kept for the next time it shows).
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
pub struct LobbyChoices {
    pub bots: usize,
    pub difficulty: Difficulty,
    pub arena: SizeChoice,
    pub mode: GameMode,
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
    shows: Option<LobbyChoices>,
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
    frame: Handle<AccentMaterial>,
    laser: Handle<AccentMaterial>,
    laser_hot: Handle<AccentMaterial>,
    /// Text in team colours (red, blue, white).
    team: [Handle<AccentMaterial>; 3],
    glow: [Handle<GlowMaterial>; 3],
    hover: Handle<GlowMaterial>,
    /// One's own row (red, blue, white).
    mine: [Handle<GlowMaterial>; 3],
    back: Handle<StandardMaterial>,
    flash: Vec<Handle<GlowMaterial>>,
    /// A unit cube (strips, lasers) and a unit square of even glow.
    cube: Handle<Mesh>,
    fill: Handle<Mesh>,
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
    let purple = Vec3::new(0.5, 0.15, 1.0);
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
        dim: fixed(&mut accents, Vec3::new(0.6, 0.62, 0.75), 1.2),
        frame: fixed(&mut accents, purple, 2.5),
        laser: fixed(&mut accents, Vec3::new(0.7, 0.8, 1.0), 1.5),
        laser_hot: fixed(&mut accents, Vec3::new(1.0, 0.85, 0.3), 4.0),
        team: teams.map(|t| fixed(&mut accents, team_color(t), 3.5)),
        glow: teams.map(|t| glow(&mut glows, team_color(t), 1.5)),
        hover: glow(&mut glows, Vec3::new(1.0, 0.8, 0.3), 1.6),
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

impl Builder<'_, '_, '_> {
    fn text(
        &mut self,
        s: &str,
        h: f32,
        at: Vec2,
        align: Align,
        mat: &Handle<AccentMaterial>,
        glow: Option<&Handle<GlowMaterial>>,
    ) -> Entity {
        segtext::spawn(
            self.commands,
            self.meshes,
            self.root,
            s,
            h,
            at.extend(0.004),
            align,
            mat,
            glow,
        )
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

    /// The dark backing and the frame.
    fn backing(&mut self, size: Vec2) {
        self.commands.spawn((
            Mesh3d(self.meshes.add(Rectangle::new(size.x, size.y))),
            MeshMaterial3d(self.mats.back.clone()),
            Transform::from_xyz(0.0, 0.0, -0.004),
            ChildOf(self.root),
        ));
        let frame = self.mats.frame.clone();
        self.outline(Vec2::ZERO, size, 0.008, &frame);
    }

    fn button(&mut self, label: &str, act: Act, c: Vec2, size: Vec2) -> Button {
        let (white, frame, hover) = (
            self.mats.white.clone(),
            self.mats.frame.clone(),
            self.mats.hover.clone(),
        );
        self.outline(c, size, 0.006, &frame);
        let hover = self.fill(c, size, &hover, false);
        self.text(label, size.y * 0.42, c, Align::Center, &white, None);
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
    b.backing(size);
    let (white, glow) = (b.mats.white.clone(), b.mats.glow[2].clone());
    let top = size.y / 2.0;
    b.text(
        "MENU",
        0.07,
        Vec2::new(0.0, top - 0.1),
        Align::Center,
        &white,
        Some(&glow),
    );
    let mut acts = vec![("RESUME", Act::Resume)];
    if local {
        acts.push(("NEW MATCH", Act::NewMatch));
    }
    acts.push(("MAIN MENU", Act::Leave));
    acts.push(("QUIT", Act::Quit));
    acts.iter()
        .enumerate()
        .map(|(k, (label, act))| {
            b.button(
                label,
                *act,
                Vec2::new(0.0, top - 0.25 - k as f32 * 0.16),
                Vec2::new(0.56, 0.12),
            )
        })
        .collect()
}

/// The start menu: the title, Play vs bots and Host game, the bots' number and skill, the
/// arenas' size (the chosen ones lit) and Quit.
fn build_lobby(b: &mut Builder, choices: &LobbyChoices) -> Vec<Button> {
    let size = Vec2::new(1.1, 1.12);
    b.backing(size);
    let top = size.y / 2.0;
    let (red, red_glow) = (b.mats.team[0].clone(), b.mats.glow[0].clone());
    let (blue, dim, lit) = (
        b.mats.team[1].clone(),
        b.mats.dim.clone(),
        b.mats.mine[2].clone(),
    );
    b.text(
        "NEON LASERTAG",
        0.075,
        Vec2::new(0.0, top - 0.08),
        Align::Center,
        &red,
        Some(&red_glow),
    );
    b.text(
        "RED VS BLUE",
        0.028,
        Vec2::new(0.0, top - 0.15),
        Align::Center,
        &blue,
        None,
    );
    let mut buttons = vec![
        b.button(
            "PLAY VS BOTS",
            Act::Play { host: false },
            Vec2::new(-0.26, top - 0.27),
            Vec2::new(0.48, 0.1),
        ),
        b.button(
            "HOST GAME",
            Act::Play { host: true },
            Vec2::new(0.26, top - 0.27),
            Vec2::new(0.48, 0.1),
        ),
    ];
    let label = |b: &mut Builder, s: &str, y: f32| {
        b.text(s, 0.026, Vec2::new(0.0, y), Align::Center, &dim, None);
    };
    label(b, "BOTS", top - 0.37);
    let y = top - 0.44;
    buttons.push(b.button(
        "-",
        Act::Bots(-1),
        Vec2::new(-0.2, y),
        Vec2::new(0.12, 0.09),
    ));
    let white = b.mats.white.clone();
    b.text(
        &choices.bots.to_string(),
        0.045,
        Vec2::new(0.0, y),
        Align::Center,
        &white,
        None,
    );
    buttons.push(b.button("+", Act::Bots(1), Vec2::new(0.2, y), Vec2::new(0.12, 0.09)));
    // A row of choices, the chosen one lit.
    let mut row = |b: &mut Builder, items: Vec<(String, Act, bool)>, y: f32, w: f32| {
        let pitch = w + 0.02;
        let x0 = -(items.len() as f32 - 1.0) * pitch / 2.0;
        for (k, (name, act, on)) in items.into_iter().enumerate() {
            let c = Vec2::new(x0 + k as f32 * pitch, y);
            let sz = Vec2::new(w, 0.09);
            if on {
                b.fill(c, sz, &lit, true);
            }
            buttons.push(b.button(&name, act, c, sz));
        }
    };
    label(b, "SKILL", top - 0.53);
    let skills = Difficulty::ALL
        .iter()
        .map(|&d| {
            (
                d.name().to_uppercase(),
                Act::Skill(d),
                d == choices.difficulty,
            )
        })
        .collect();
    row(b, skills, top - 0.6, 0.3);
    label(b, "ARENA", top - 0.69);
    let arenas = SizeChoice::ALL
        .iter()
        .map(|&a| (a.name().to_uppercase(), Act::Arena(a), a == choices.arena))
        .collect();
    row(b, arenas, top - 0.76, 0.24);
    label(b, "MODE", top - 0.85);
    let modes = GameMode::ALL
        .iter()
        .map(|&m| (m.name().to_uppercase(), Act::Mode(m), m == choices.mode))
        .collect();
    row(b, modes, top - 0.92, 0.42);
    buttons.push(b.button(
        "QUIT",
        Act::Quit,
        Vec2::new(0.0, top - 1.03),
        Vec2::new(0.4, 0.09),
    ));
    buttons
}

/// Column right edges (score, tags, out, accuracy, targets) on the end panel.
const COLUMNS: [f32; 5] = [0.02, 0.13, 0.24, 0.36, 0.46];
const ROW: f32 = 0.042;

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
    for (label, x) in ["SCORE", "TAGS", "OUT", "ACC", "TGT"].iter().zip(COLUMNS) {
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
            &short(&r.name, 12),
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
    let local = game.is_local();
    let width = 1.0;
    let n = rows[0].len() + rows[1].len();
    let height = 0.06
        + 0.13
        + 0.09
        + 2.0 * (0.12 + 0.02)
        + n as f32 * ROW
        + 0.06
        + if local { 0.14 } else { 0.0 }
        + 0.03;
    b.backing(Vec2::new(width, height));
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
    let countdown_at = Vec3::new(0.0, y - 0.02, 0.004);
    let mut buttons = Vec::new();
    if local {
        buttons.push(b.button(
            "NEXT MATCH",
            Act::NewMatch,
            Vec2::new(0.0, y - 0.12),
            Vec2::new(0.44, 0.1),
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
    choices: Res<LobbyChoices>,
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
        p.kind == Kind::Lobby && (p.shows != Some(*choices) || p.headless && input.head.is_some())
    });
    if ui.panel.as_ref().map(|p| p.kind) != want || stale {
        if let Some(p) = ui.panel.take() {
            commands.entity(p.root).despawn();
        }
        ui.pointed = [None; 2];
        if let Some(kind) = want {
            // In front of the eyes, level, a little below them.
            let head = input
                .head
                .unwrap_or(Transform::from_xyz(0.0, lt_sim::sim::EYE, 0.0));
            let f = head.rotation * Vec3::NEG_Z;
            let rot = Quat::from_rotation_y(lt_sim::sim::yaw_of(Vec3::new(f.x, 0.0, f.z)));
            let pose = Transform::from_translation(
                head.translation + rot * Vec3::new(0.0, -0.12, -PANEL_DISTANCE),
            )
            .with_rotation(rot);
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
                (Kind::Lobby, _) => (build_lobby(&mut b, &choices), Vec3::ZERO),
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
                shows: (kind == Kind::Lobby).then_some(*choices),
                headless: input.head.is_none(),
            });
        }
    }
    vr.ui = ui.panel.is_some();
    let Some(p) = ui.panel.as_mut() else { return };
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
    mut choices: ResMut<LobbyChoices>,
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
    let Some(act) = pressed.and_then(|k| panel.buttons.get(k)).map(|b| b.act) else {
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
        Act::Play { host } => {
            commands.insert_resource(crate::menu::Start::Local {
                host,
                bots: choices.bots,
                difficulty: choices.difficulty,
                arena: choices.arena,
                mode: choices.mode,
            });
        }
        Act::Bots(d) => choices.bots = (choices.bots as i32 + d).clamp(0, 15) as usize,
        Act::Skill(d) => choices.difficulty = d,
        Act::Arena(a) => choices.arena = a,
        Act::Mode(m) => choices.mode = m,
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

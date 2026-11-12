//! Neon lasertag: Red vs Blue in procedurally built arenas, in VR or on a flat screen.
//!
//! ```text
//! neon-lasertag [--vr | --gl-vr | --flat] [--seed N] [--bots N] [--name NAME]
//!          [--host [PORT]] [--public] [--server-name NAME] [--connect HOST[:PORT]] [--autopilot]
//!          [--difficulty easy|normal|hard] [--arena small|medium|large|random]
//!          [--mode classic|megatarget|prism|infection]
//!          [--secondary random|shoot|relay|circuit|pattern|sequence]
//!          [--left-handed] [--match-time SECS]
//!          [--screenshot FILE --after SECS] [--quit-after SECS]
//! ```
//!
//! Modes (picked at startup, as OpenXR has to replace the renderer):
//! - default: VR on a Steam Frame (`--gl-vr` there) or while a VR compositor runs (SteamVR's
//!   vrcompositor or Monado's service: `--vr`), the flat screen otherwise;
//! - `--flat`: the flat screen with mouse and keyboard (src/desktop.rs);
//! - `--vr`: any OpenXR headset through bevy_mod_openxr (src/xr.rs);
//! - `--gl-vr`: our own OpenGL renderer drawing straight into the headset's swapchain, eye
//!   tracked foveated on the Steam Frame (src/glvr, feature `glvr`, Linux).

pub mod actors;
pub mod ambience;
pub mod arena;
pub mod assets;
pub mod audio;
pub mod consoles;
pub mod desktop;
pub mod endscreen;
pub mod eyes;
pub mod features;
pub mod freecam;
pub mod freeze;
pub mod game;
#[cfg(all(feature = "glvr", target_os = "linux"))]
pub mod glvr;
pub mod hud;
pub mod lobby;
pub mod materials;
pub mod mega;
pub mod menu;
pub mod music;
pub mod neonui;
pub mod options;
pub mod pause;
pub mod reflections;
pub mod rig;
pub mod scenery;
pub mod scope;
pub mod segtext;
pub mod session;
pub mod settings;
pub mod sevenseg;
pub mod shots;
pub mod tail;
pub mod targets;
pub mod turretbox;
pub mod vfont;
pub mod vr;
pub mod vrui;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(windows)]
mod win_console;
#[cfg(feature = "xr")]
pub mod xr;

use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use lt_net::client::{Client, ClientEvent};
use lt_sim::bots::Difficulty;
use lt_sim::level::Level;

use crate::arena::{ArenaPart, Kit};
use crate::game::Game;
use crate::materials::Palette;

/// The browser build (docs/web.md): no VR, no hosting, joining through the master server's relay.
pub const WEB: bool = cfg!(target_arch = "wasm32");
/// The browser build without WebGPU draws with WebGL2: no screen-space reflections there
/// (they need the deferred prepass), src/reflections.rs.
pub const WEBGL2: bool = cfg!(all(target_arch = "wasm32", not(feature = "webgpu")));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// VR if there is a headset to use (see `auto_mode`), else the flat screen.
    Auto,
    Desktop,
    Vr,
    GlVr,
}

#[derive(Clone, Debug, Resource)]
pub struct Args {
    pub mode: Mode,
    pub seed: u64,
    pub bots: usize,
    pub name: String,
    pub host: Option<u16>,
    /// `--host` on the internet: listed on the master server, the port opened on the router.
    pub public: bool,
    /// The hosted server's name (`--server-name`; else the player's arena).
    pub server_name: Option<String>,
    pub connect: Option<String>,
    pub autopilot: bool,
    /// The bots' skill in local and hosted games.
    pub difficulty: Difficulty,
    /// The arenas' size in local and hosted games (`Random`: a new one every match).
    pub arena: lt_sim::level::SizeChoice,
    /// What local and hosted games are played for (`--mode`).
    pub game_mode: lt_sim::level::GameMode,
    /// Megatarget mode: how the secondary targets charge players (`--secondary`).
    pub secondary: lt_sim::rules::SecondaryKind,
    /// One's own team in local and hosted games (`--team`; none: the smaller one), and whether
    /// players may switch teams during a match (`--lock-teams` turns it off).
    pub team: Option<lt_sim::Team>,
    pub team_switch: bool,
    /// Match modifiers in local and hosted games (`--modifiers dark,lowgrav,mirror`).
    pub modifiers: Vec<lt_sim::ArenaEvent>,
    /// VR: the holster on the left hip (`--left-handed` or `LT_LEFT_HANDED=1`).
    pub left_handed: bool,
    /// Local games: the match's length in seconds (testing).
    pub match_time: Option<f32>,
    /// File and the times (s) to take it at (`--after 5,10,15`: file_0.png, file_1.png, ...).
    pub screenshot: Option<(String, Vec<f32>)>,
    pub quit_after: Option<f32>,
    /// Show the start menu (nothing chosen on the command line).
    pub menu: bool,
}

impl Args {
    pub fn parse() -> Args {
        let mut a = Args {
            mode: Mode::Auto,
            seed: rand::random::<u32>() as u64,
            bots: 7,
            name: std::env::var("USER").unwrap_or_else(|_| "Player".into()),
            host: None,
            public: false,
            server_name: None,
            connect: None,
            autopilot: false,
            difficulty: Difficulty::default(),
            arena: lt_sim::level::SizeChoice::default(),
            game_mode: lt_sim::level::GameMode::Megatarget,
            secondary: lt_sim::rules::SecondaryKind::Random,
            team: None,
            team_switch: true,
            modifiers: Vec::new(),
            left_handed: std::env::var("LT_LEFT_HANDED").is_ok_and(|v| v == "1"),
            match_time: None,
            screenshot: None,
            quit_after: None,
            menu: true,
        };
        #[cfg(target_arch = "wasm32")]
        if let Some(name) = web::param("name") {
            a.name = name;
        }
        let argv: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        let next = |i: &mut usize| -> Option<String> {
            *i += 1;
            argv.get(*i).cloned()
        };
        let mut after = vec![3.0];
        let mut shot = None;
        while i < argv.len() {
            match argv[i].as_str() {
                "--vr" => a.mode = Mode::Vr,
                "--flat" => a.mode = Mode::Desktop,
                "--gl-vr" => a.mode = Mode::GlVr,
                "--seed" => a.seed = next(&mut i).and_then(|v| v.parse().ok()).unwrap_or(a.seed),
                "--bots" => a.bots = next(&mut i).and_then(|v| v.parse().ok()).unwrap_or(a.bots),
                "--name" => a.name = next(&mut i).unwrap_or(a.name),
                "--host" => {
                    let port = argv.get(i + 1).and_then(|v| v.parse().ok());
                    if port.is_some() {
                        i += 1;
                    }
                    a.host = Some(port.unwrap_or(lt_net::protocol::DEFAULT_PORT));
                }
                "--public" => a.public = true,
                "--server-name" => a.server_name = next(&mut i),
                "--connect" => a.connect = next(&mut i),
                "--autopilot" => a.autopilot = true,
                "--difficulty" => match next(&mut i).as_deref().and_then(Difficulty::parse) {
                    Some(d) => a.difficulty = d,
                    None => eprintln!("--difficulty: easy, normal or hard"),
                },
                "--arena" => match next(&mut i)
                    .as_deref()
                    .and_then(lt_sim::level::SizeChoice::parse)
                {
                    Some(s) => a.arena = s,
                    None => eprintln!("--arena: small, medium, large or random"),
                },
                "--secondary" => match next(&mut i)
                    .as_deref()
                    .and_then(lt_sim::rules::SecondaryKind::parse)
                {
                    Some(k) => a.secondary = k,
                    None => {
                        eprintln!("--secondary: random, shoot, relay, circuit, pattern or sequence")
                    }
                },
                "--mode" => match next(&mut i)
                    .as_deref()
                    .and_then(lt_sim::level::GameMode::parse)
                {
                    Some(m) => a.game_mode = m,
                    None => eprintln!("--mode: classic, megatarget, prism or infection"),
                },
                "--team" => match next(&mut i).as_deref().map(str::to_ascii_lowercase) {
                    Some(t) if t == "red" => a.team = Some(lt_sim::Team::Red),
                    Some(t) if t == "blue" => a.team = Some(lt_sim::Team::Blue),
                    Some(t) if t == "auto" => a.team = None,
                    _ => eprintln!("--team: red, blue or auto"),
                },
                "--lock-teams" => a.team_switch = false,
                "--modifiers" => match next(&mut i)
                    .as_deref()
                    .map(lt_sim::ArenaEvent::parse_modifiers)
                {
                    Some(Some(m)) => a.modifiers = m,
                    _ => eprintln!("--modifiers: a comma list of dark, lowgrav and mirror"),
                },
                "--left-handed" => a.left_handed = true,
                "--match-time" => a.match_time = next(&mut i).and_then(|v| v.parse().ok()),
                "--screenshot" => shot = next(&mut i),
                "--after" => {
                    after = next(&mut i)
                        .map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect())
                        .unwrap_or(after)
                }
                "--quit-after" => a.quit_after = next(&mut i).and_then(|v| v.parse().ok()),
                other => eprintln!("unknown argument {other}"),
            }
            i += 1;
        }
        a.screenshot = shot.map(|s| (s, after));
        let chosen = ["--seed", "--bots", "--host", "--connect", "--autopilot"];
        a.menu = !argv.iter().any(|v| chosen.contains(&v.as_str()));
        a
    }

    /// The name a game hosted here goes by.
    pub fn hosted_name(&self) -> String {
        self.server_name
            .clone()
            .unwrap_or_else(|| format!("{}'s arena", self.name))
    }

    /// The rules of local games in `mode`.
    pub fn rules(&self, mode: lt_sim::level::GameMode) -> lt_sim::Rules {
        let mut r = lt_sim::Rules::for_mode(mode);
        if let Some(t) = self.match_time {
            r.match_time = t;
        }
        r.secondary = self.secondary;
        r
    }
}

/// Waiting for the server's welcome.
#[derive(Resource)]
pub struct Connecting(Option<Client>);

#[derive(Resource, Default)]
struct BuiltArena(Option<(u64, lt_sim::level::GameMode)>);

/// Whether this is a Steam Frame: SteamOS on arm64 (Valve's other SteamOS machines are x86-64).
fn steam_frame() -> bool {
    cfg!(all(target_os = "linux", target_arch = "aarch64"))
        && std::fs::read_to_string("/etc/os-release").is_ok_and(|s| {
            s.lines()
                .any(|l| l.trim() == "ID=steamos" || l.trim() == "ID=\"steamos\"")
        })
}

/// Whether a VR compositor is running: SteamVR's (vrcompositor) or Monado's service. Asking
/// OpenXR itself would start SteamVR wherever it is installed.
fn compositor_running() -> bool {
    const NAMES: [&str; 2] = ["vrcompositor", "monado-service"];
    #[cfg(target_os = "linux")]
    {
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
            && std::path::Path::new(&dir).join("monado_comp_ipc").exists()
        {
            return true;
        }
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return false;
        };
        procs.flatten().any(|p| {
            std::fs::read_to_string(p.path().join("comm")).is_ok_and(|c| NAMES.contains(&c.trim()))
        })
    }
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/NH", "/FI", "IMAGENAME eq vrcompositor.exe"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("vrcompositor"))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = NAMES;
        false
    }
}

/// The mode to start in without one on the command line: VR on a Steam Frame (our own GL
/// renderer, foveated) or while a VR compositor runs (bevy_mod_openxr, or the GL renderer if the
/// runtime only offers OpenGL), the flat screen otherwise.
fn auto_mode() -> Mode {
    let frame = steam_frame();
    if !frame && !compositor_running() {
        return Mode::Desktop;
    }
    #[cfg(all(feature = "glvr", target_os = "linux"))]
    if frame && glvr::runtime_available() {
        return Mode::GlVr;
    }
    #[cfg(feature = "xr")]
    if xr::runtime_available() {
        return Mode::Vr;
    }
    #[cfg(all(feature = "glvr", target_os = "linux"))]
    if glvr::runtime_available() {
        return Mode::GlVr;
    }
    Mode::Desktop
}

pub fn run() {
    #[cfg(windows)]
    win_console::leave_explorer_console();
    let mut args = Args::parse();
    if args.mode == Mode::Auto {
        args.mode = auto_mode();
        eprintln!("starting in {:?} mode", args.mode);
    }
    let mut app = App::new();
    // In the browser the assets are beside the page (web/index.html copies them there).
    let asset_root = if WEB {
        "assets".to_string()
    } else {
        assets::asset_dir().to_string_lossy().to_string()
    };
    let vr = match args.mode {
        #[cfg(feature = "xr")]
        Mode::Vr => xr::add_plugins(&mut app, &asset_root),
        #[cfg(all(feature = "glvr", target_os = "linux"))]
        Mode::GlVr => glvr::add_plugins(&mut app, &asset_root),
        _ => false,
    };
    if !vr {
        {
            if args.mode != Mode::Desktop {
                warn!("no headset for {:?}; playing on the flat screen", args.mode);
                args.mode = Mode::Desktop;
            }
            app.add_plugins(
                DefaultPlugins
                    .set(WindowPlugin {
                        primary_window: Some(Window {
                            title: "Neon Lasertag".into(),
                            // The browser build draws into the page's canvas (web/index.html),
                            // as big as the page, and keeps the browser's keys (F5, Tab, ...)
                            // for the game.
                            canvas: WEB.then(|| "#lasertag".into()),
                            fit_canvas_to_parent: true,
                            prevent_default_event_handling: true,
                            ..default()
                        }),
                        ..default()
                    })
                    .set(AssetPlugin {
                        file_path: asset_root,
                        // In the browser each asset is fetched: no .meta files to ask for
                        // (a 404 each).
                        meta_check: if WEB {
                            bevy::asset::AssetMetaCheck::Never
                        } else {
                            default()
                        },
                        ..default()
                    }),
            );
            app.add_plugins((
                desktop::DesktopPlugin {
                    autopilot: args.autopilot,
                },
                hud::HudPlugin,
                menu::MenuPlugin,
                neonui::NeonUiPlugin,
                lobby::LobbyPlugin,
                pause::PausePlugin,
                options::OptionsPlugin,
                reflections::ReflectionsPlugin,
                endscreen::EndScreenPlugin,
                freeze::FreezePlugin,
                freecam::FreeCamPlugin,
            ));
        }
    }
    app.add_plugins((
        materials::MaterialsPlugin,
        game::GamePlugin,
        menu::MenuModelPlugin,
        actors::ActorsPlugin,
        eyes::EyesPlugin,
        audio::SoundPlugin,
        sevenseg::ScoreboardPlugin,
        music::MusicPlugin,
        scenery::SceneryPlugin,
        ambience::AmbiencePlugin,
        session::SessionPlugin,
        targets::TargetsPlugin,
        mega::MegaPlugin,
        turretbox::TurretBoxPlugin,
        features::FeaturesPlugin,
    ))
    .add_plugins(consoles::ConsolePlugin)
    .insert_resource(ClearColor(Color::linear_rgb(0.002, 0.002, 0.006)))
    .insert_resource(GlobalAmbientLight {
        color: Color::linear_rgb(0.6, 0.65, 1.0),
        brightness: 60.0,
        ..default()
    })
    .init_resource::<BuiltArena>()
    .insert_resource(args.clone())
    .insert_resource(settings::Settings::load())
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            start_game.run_if(resource_added::<menu::Start>),
            connect,
            build_arena,
        ),
    );
    if let Some((path, times)) = args.screenshot.clone() {
        app.add_systems(
            Update,
            // (On the wall clock: the game's stops while frozen, src/freeze.rs.)
            move |mut commands: Commands, time: Res<Time<Real>>, mut done: Local<usize>| {
                if *done < times.len() && time.elapsed_secs() > times[*done] {
                    let file = if times.len() > 1 {
                        path.replace(".png", &format!("_{}.png", *done))
                    } else {
                        path.clone()
                    };
                    *done += 1;
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(file));
                }
            },
        );
    }
    if let Some(secs) = args.quit_after {
        app.add_systems(
            Update,
            move |time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>| {
                if time.elapsed() > Duration::from_secs_f32(secs) {
                    exit.write(AppExit::Success);
                }
            },
        );
    }
    app.run();
}

fn setup(
    mut commands: Commands,
    args: Res<Args>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut local: ResMut<game::LocalCmd>,
) {
    local.autopilot = args.autopilot;
    commands.insert_resource(assets::load_models(&mut meshes));
    commands.insert_resource(Kit::load());
    commands.spawn((
        DirectionalLight {
            illuminance: 500.0,
            color: Color::linear_rgb(0.7, 0.75, 1.0),
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Without the menu (the flat screen's, or VR's in the empty arena: src/lobby.rs), the
    // command line says what to play.
    if !args.menu {
        commands.insert_resource(match &args.connect {
            Some(addr) => menu::Start::Join(addr.clone()),
            None => menu::Start::Local {
                host: args.host.map(|port| game::Hosting {
                    internet: args.public,
                    name: args.hosted_name(),
                    port,
                }),
                bots: args.bots,
                difficulty: args.difficulty,
                arena: args.arena,
                mode: args.game_mode,
                team: args.team,
                team_switch: args.team_switch,
                modifiers: args.modifiers.clone(),
            },
        });
    }
}

fn start_game(
    mut commands: Commands,
    args: Res<Args>,
    start: Res<menu::Start>,
    mut session: ResMut<session::Session>,
    mut games: Local<u32>,
) {
    // The command line's arena first, then new ones.
    let seed = if *games == 0 {
        args.seed
    } else {
        rand::random::<u32>() as u64
    };
    *games += 1;
    match start.clone() {
        // In the browser `addr` is the relay's WebSocket URL to the server (src/menu.rs).
        menu::Start::Join(addr) => match join(&addr, &args.name) {
            Ok(c) => {
                info!("connecting to {addr}");
                commands.insert_resource(Connecting(Some(c)));
            }
            Err(e) => {
                error!("can't connect to {addr}: {e}");
                session.difficulty = args.difficulty;
                session.arena = args.arena;
                session.mode = args.game_mode;
                commands.insert_resource(Game::local(
                    seed,
                    args.bots,
                    args.difficulty.skill(),
                    session.rules(&args),
                    &args.name,
                    args.team,
                    None,
                ));
            }
        },
        menu::Start::Local {
            host,
            bots,
            difficulty,
            arena,
            mode,
            team,
            team_switch,
            modifiers,
        } => {
            session.difficulty = difficulty;
            session.arena = arena;
            session.mode = mode;
            let rules = lt_sim::Rules {
                team_switch,
                modifiers,
                ..session.rules(&args)
            };
            let mut game = Game::local(
                seed,
                bots,
                difficulty.skill(),
                rules,
                &args.name,
                team,
                host,
            );
            if args.autopilot {
                session::autopilot(&mut game, seed);
            }
            info!(
                "arena {seed} ({}, {}, {})\n{}",
                game.rules.arena.name(),
                game.rules.mode.name(),
                difficulty.name(),
                lt_sim::level::ascii(&game.level)
            );
            commands.insert_resource(game);
        }
    }
}

/// Starts connecting to a server: over UDP, or in the browser through the master server's
/// relay (`addr` is then its WebSocket URL).
fn join(addr: &str, name: &str) -> std::io::Result<Client> {
    #[cfg(target_arch = "wasm32")]
    return Client::relayed(addr, name);
    #[cfg(not(target_arch = "wasm32"))]
    Client::connect(addr, name)
}

fn connect(
    mut commands: Commands,
    connecting: Option<ResMut<Connecting>>,
    game: Option<ResMut<Game>>,
    mut menu: Option<ResMut<menu::MenuModel>>,
) {
    let Some(mut c) = connecting else { return };
    let Some(client) = c.0.as_mut() else { return };
    for ev in client.poll() {
        match ev {
            ClientEvent::Welcome { id, seed, rules } => {
                info!("joined as player {id} in arena {seed}");
                let level = std::sync::Arc::new(Level::generate_for(seed, rules.arena, rules.mode));
                let client = c.0.take().unwrap();
                commands.insert_resource(Game::remote(client, level, rules, id));
                commands.remove_resource::<Connecting>();
                return;
            }
            ClientEvent::Disconnected(why) => {
                error!("couldn't join: {why}");
                if let Some(m) = menu.as_deref_mut() {
                    m.join_error = Some(format!("couldn't join: {why}"));
                    m.rev += 1;
                    // The browser's players see why on the server list.
                    if WEB {
                        m.go(menu::Screen::Internet);
                    }
                }
                commands.remove_resource::<Connecting>();
                // Back to the start menu (flat screen).
                commands.remove_resource::<menu::Start>();
                commands.insert_resource(menu::ShowMenu);
                return;
            }
            _ => {}
        }
    }
    let _ = game;
}

#[allow(clippy::too_many_arguments)]
fn build_arena(
    mut commands: Commands,
    game: Option<Res<Game>>,
    kit: Option<Res<Kit>>,
    palette: Res<Palette>,
    models: Res<assets::Models>,
    mut built: ResMut<BuiltArena>,
    mut glows: ResMut<Assets<materials::GlowMaterial>>,
    mut accents: ResMut<Assets<materials::AccentMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut std: ResMut<Assets<StandardMaterial>>,
    assets: Res<AssetServer>,
    old: Query<Entity, With<ArenaPart>>,
) {
    let (Some(game), Some(kit)) = (game, kit) else {
        // Left the game (src/session.rs took the arena down): the next one is built anew.
        built.0 = None;
        return;
    };
    if built.0 == Some((game.level.seed, game.level.mode)) {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    arena::spawn_arena(&mut commands, &game.level, &kit, &palette, &mut meshes);
    for r in &game.level.reloads {
        let e = commands
            .spawn((
                ArenaPart,
                Transform::from_translation(*r),
                Visibility::default(),
            ))
            .id();
        actors::spawn_model(&mut commands, e, &models.reload, &palette, None);
    }
    targets::spawn(&mut commands, &game.level, &palette, &mut meshes);
    mega::spawn(
        &mut commands,
        &game.level,
        lt_sim::console::secondary_kinds(&game.level, &game.rules),
        &palette,
        &mut meshes,
        &mut accents,
        &mut glows,
        &assets,
    );
    turretbox::spawn(
        &mut commands,
        &game.level,
        &palette,
        &mut meshes,
        &mut accents,
    );
    consoles::spawn(&mut commands, &game, &palette, &mut meshes, &mut accents);
    features::spawn(
        &mut commands,
        &game.level,
        &palette,
        &mut meshes,
        &mut std,
        &mut accents,
        &mut glows,
        &assets,
    );
    ambience::spawn(
        &mut commands,
        &game.level,
        &mut meshes,
        &mut glows,
        &mut accents,
    );
    sevenseg::spawn_scoreboards(&mut commands, &game.level, &palette, &mut meshes);
    built.0 = Some((game.level.seed, game.level.mode));
}

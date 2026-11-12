//! The start menu, shown at the start (nothing chosen on the command line) and whenever a game
//! is left (src/session.rs inserts `ShowMenu`). Its levels:
//!
//! ```text
//! top          Botmatch | Multiplayer | Options | Quit
//! botmatch     the bots' number and difficulty, the arenas' size, the mode, one's team and
//!              whether teams may be switched during a match, the match modifiers; Start
//! multiplayer  Internet | LAN
//! internet     the master server's list (join one), join by address, host an internet game
//! lan          the games found on the local network (lt-net's lan.rs), join by address,
//!              host a LAN game
//! host         the server's name, the bots, arena and mode; Start hosting
//! ```
//!
//! [`MenuModel`] holds where the menu is and what was chosen, for both the flat screen's menu
//! (here, in the neon look of src/neonui.rs) and VR's (src/vrui.rs, no typing there); both act
//! through [`MenuModel::apply`]. `LT_MENU=lan` (any level's name) opens the menu there.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lt_net::{LanBrowser, LanServer, ListSlot, ServerEntry, fetch_list_into};
use lt_sim::bots::Difficulty;
use lt_sim::level::{GameMode, SizeChoice};
use lt_sim::{ArenaEvent, Team};

use crate::Args;
use crate::desktop::Overlays;
use crate::game::Hosting;
use crate::neonui::{self, NeonButton};

/// A level of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    Top,
    Botmatch,
    Multiplayer,
    Internet,
    Lan,
    Host { internet: bool },
}

impl Screen {
    /// Where Back goes.
    pub fn parent(self) -> Option<Screen> {
        match self {
            Screen::Top => None,
            Screen::Botmatch | Screen::Multiplayer => Some(Screen::Top),
            // The browser build has only the internet's servers (no LAN): straight there.
            Screen::Internet if crate::WEB => Some(Screen::Top),
            Screen::Internet | Screen::Lan => Some(Screen::Multiplayer),
            Screen::Host { internet: true } => Some(Screen::Internet),
            Screen::Host { internet: false } => Some(Screen::Lan),
        }
    }

    pub fn parse(s: &str) -> Option<Screen> {
        Some(match s {
            "top" => Screen::Top,
            "botmatch" => Screen::Botmatch,
            "multiplayer" => Screen::Multiplayer,
            "internet" => Screen::Internet,
            "lan" => Screen::Lan,
            "host-internet" => Screen::Host { internet: true },
            "host-lan" => Screen::Host { internet: false },
            _ => return None,
        })
    }

    /// Where in the menu this is (under the title).
    pub fn crumb(self) -> &'static str {
        match self {
            Screen::Top => "RED  VS  BLUE",
            Screen::Botmatch => "BOTMATCH",
            Screen::Multiplayer => "MULTIPLAYER",
            Screen::Internet => "MULTIPLAYER  /  INTERNET",
            Screen::Lan => "MULTIPLAYER  /  LAN",
            Screen::Host { internet: true } => "INTERNET  /  HOST A GAME",
            Screen::Host { internet: false } => "LAN  /  HOST A GAME",
        }
    }
}

/// What a menu button does.
#[derive(Component, Clone, Debug, PartialEq)]
pub enum MenuAct {
    Go(Screen),
    Back,
    /// Play against bots with the choices.
    StartBots,
    /// Host with the choices (on the host screen's network).
    StartHost,
    Join(String),
    /// Join the typed address.
    JoinTyped,
    Refresh,
    MoreBots(i32),
    Difficulty(Difficulty),
    Arena(SizeChoice),
    Mode(GameMode),
    /// One's own team (`None`: the smaller one).
    Team(Option<Team>),
    /// Whether players may switch teams during a match.
    TeamSwitch(bool),
    /// A match modifier (of `ArenaEvent::MODIFIERS`) on or off.
    Modifier(ArenaEvent),
    Options,
    Quit,
}

/// What a mode is about, in a line (the mode buttons' tooltips; capitals for VR's panels).
pub fn mode_blurb(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Classic => "HIT THE ENEMY TEAM'S TARGETS - THEY HANG DEEP IN ITS HALF",
        GameMode::Megatarget => {
            "HIT BOTH SECONDARY TARGETS TO OPEN THE MIDDLE ROOM - THEN THE MEGATARGET"
        }
        GameMode::Prism => "STEAL THE ENEMY'S PRISM AND CARRY IT HOME - NO SHOOTING WHILE CARRYING",
        GameMode::Infection => {
            "CLOAKED RED PHANTOMS HUNT BLUE SURVIVORS - TAGGED OUT YOU TURN PHANTOM"
        }
    }
}

/// What a match modifier does, in a line (its button's tooltip; capitals for VR's panels).
pub fn modifier_blurb(kind: ArenaEvent) -> &'static str {
    match kind {
        ArenaEvent::Blackout => {
            "THE ARENA'S LIGHTS STAY LOW ALL MATCH - BOLTS AND PLAYERS STILL GLOW"
        }
        ArenaEvent::LowGravity => "EVERYONE JUMPS HIGHER AND FALLS SLOWER ALL MATCH",
        ArenaEvent::MirrorStorm => "EVERY WALL TURNS A BOLT BACK ONCE, ALL MATCH",
        _ => "",
    }
}

/// A team's name as the menus show it (`None`: auto).
pub fn team_name(team: Option<Team>) -> &'static str {
    match team {
        None => "AUTO",
        Some(Team::Red) => "RED",
        Some(Team::Blue) => "BLUE",
    }
}

/// A server's players as the lists show them: "3 / 32" (just "3" from servers that don't say
/// how many fit).
pub fn players_text(r: &lt_net::ServerReport) -> String {
    if r.max_players > 0 {
        format!("{} / {}", r.players.len(), r.max_players)
    } else {
        r.players.len().to_string()
    }
}

/// Whether a server has bots, as the lists show it: "BOTS ON (3)", "BOTS ON" (all their
/// places taken by players), "NO BOTS".
pub fn bots_text(r: &lt_net::ServerReport) -> String {
    match (r.bots_enabled || r.bots > 0, r.bots) {
        (true, 0) => "BOTS ON".into(),
        (true, n) => format!("BOTS ON ({n})"),
        (false, _) => "NO BOTS".into(),
    }
}

/// What an act leads to beyond the menu.
pub enum Outcome {
    Stay,
    Start(Start),
    Options,
    Quit,
}

/// The master server's list as the menu knows it.
#[derive(Clone, Debug, PartialEq)]
pub enum InternetList {
    /// Not asked yet.
    Idle,
    Asking,
    Listed(Vec<ServerEntry>),
    Failed(String),
}

/// Where the menu is, what was chosen (kept while playing, for the next time it shows) and
/// the servers found.
#[derive(Resource)]
pub struct MenuModel {
    pub screen: Screen,
    pub bots: usize,
    pub difficulty: Difficulty,
    pub arena: SizeChoice,
    pub mode: GameMode,
    /// One's own team in local and hosted games (`None`: the smaller one), and whether
    /// players may switch teams during a match.
    pub team: Option<Team>,
    pub team_switch: bool,
    /// The match modifiers on (`Rules::modifiers`).
    pub modifiers: Vec<ArenaEvent>,
    /// The hosted game's name, and the address typed to join.
    pub server_name: String,
    pub address: String,
    pub port: u16,
    master: String,
    /// The browser build joins through this master server's relay (`?relay=` for another).
    #[cfg(target_arch = "wasm32")]
    relay: String,
    slot: ListSlot,
    pub internet: InternetList,
    lan: Option<LanBrowser>,
    pub lan_servers: Vec<LanServer>,
    /// The LAN can't be listened to (the port is taken?).
    pub lan_error: Option<String>,
    /// Why the last server couldn't be joined (shown over the list).
    pub join_error: Option<String>,
    /// Counts changes to the lists (to redraw them).
    pub rev: u32,
}

impl MenuModel {
    pub fn new(args: &Args) -> Self {
        // The browser build's page may name another (`?master=`).
        #[cfg(target_arch = "wasm32")]
        let master = crate::web::param("master");
        #[cfg(not(target_arch = "wasm32"))]
        let master = std::env::var("LT_MASTER").ok();
        let master = master.unwrap_or_else(|| lt_net::DEFAULT_MASTER.to_string());
        MenuModel {
            screen: Screen::Top,
            bots: args.bots,
            difficulty: args.difficulty,
            arena: args.arena,
            mode: args.game_mode,
            team: args.team,
            team_switch: args.team_switch,
            modifiers: args.modifiers.clone(),
            server_name: args.hosted_name(),
            address: String::new(),
            port: args.host.unwrap_or(lt_net::protocol::DEFAULT_PORT),
            master: master.clone(),
            #[cfg(target_arch = "wasm32")]
            relay: crate::web::param("relay").unwrap_or(master),
            slot: ListSlot::default(),
            internet: InternetList::Idle,
            lan: None,
            lan_servers: Vec::new(),
            lan_error: None,
            join_error: None,
            rev: 0,
        }
    }

    /// Goes to `screen`: asks the master server for its list on the way into Internet,
    /// listens to the LAN while on LAN.
    pub fn go(&mut self, screen: Screen) {
        self.screen = screen;
        if screen == Screen::Internet && self.internet == InternetList::Idle {
            self.refresh();
        }
        if screen == Screen::Lan {
            if self.lan.is_none() {
                match LanBrowser::new() {
                    Ok(b) => {
                        self.lan = Some(b);
                        self.lan_error = None;
                    }
                    Err(e) => self.lan_error = Some(format!("can't listen on the network: {e}")),
                }
                self.rev += 1;
            }
        } else if self.lan.take().is_some() {
            self.lan_servers.clear();
            self.rev += 1;
        }
    }

    fn refresh(&mut self) {
        self.internet = InternetList::Asking;
        self.slot = ListSlot::default();
        fetch_list_into(&self.master, self.slot.clone());
        self.rev += 1;
    }

    /// Takes in the lists' news (every frame).
    pub fn poll(&mut self) {
        let fetched = self.slot.lock().ok().and_then(|mut s| s.take());
        if let Some(r) = fetched {
            self.internet = match r {
                Ok(l) => InternetList::Listed(l),
                Err(e) => InternetList::Failed(e),
            };
            self.rev += 1;
        }
        if let Some(b) = &mut self.lan
            && b.poll()
        {
            self.lan_servers = b.servers();
            self.rev += 1;
        }
    }

    /// Whether `act` shows a current choice.
    pub fn selected(&self, act: &MenuAct) -> bool {
        match act {
            MenuAct::Difficulty(d) => *d == self.difficulty,
            MenuAct::Arena(a) => *a == self.arena,
            MenuAct::Mode(m) => *m == self.mode,
            MenuAct::Team(t) => *t == self.team,
            MenuAct::TeamSwitch(on) => *on == self.team_switch,
            MenuAct::Modifier(k) => self.modifiers.contains(k),
            _ => false,
        }
    }

    fn local(&self, host: Option<Hosting>) -> Start {
        Start::Local {
            host,
            bots: self.bots,
            difficulty: self.difficulty,
            arena: self.arena,
            mode: self.mode,
            team: self.team,
            team_switch: self.team_switch,
            modifiers: self.modifiers.clone(),
        }
    }

    /// Joins the server at `addr` ("host[:port]").
    #[cfg(not(target_arch = "wasm32"))]
    fn join(&mut self, addr: &str) -> Option<Start> {
        self.join_error = None;
        Some(Start::Join(addr.to_string()))
    }

    /// The browser build joins through the master server's relay, so only a server on its
    /// list (by address, or by name): the [`Start::Join`] is the relay's WebSocket URL to it.
    #[cfg(target_arch = "wasm32")]
    fn join(&mut self, addr: &str) -> Option<Start> {
        let with_port = format!("{addr}:{}", lt_net::protocol::DEFAULT_PORT);
        let found = match &self.internet {
            InternetList::Listed(l) => l
                .iter()
                .find(|e| {
                    e.addr == addr
                        || e.addr == with_port
                        || e.report.name.eq_ignore_ascii_case(addr)
                })
                .map(|e| e.id.clone()),
            _ => None,
        };
        self.rev += 1;
        match found {
            Some(id) => {
                self.join_error = None;
                Some(Start::Join(lt_net::relay_url(&self.relay, &id)))
            }
            None => {
                self.join_error = Some(format!(
                    "{addr} is not on the server list: in the browser only listed servers \
                     can be joined"
                ));
                None
            }
        }
    }

    pub fn apply(&mut self, act: &MenuAct) -> Outcome {
        let start = match act {
            MenuAct::Go(s) => {
                self.go(*s);
                None
            }
            MenuAct::Back => {
                if let Some(p) = self.screen.parent() {
                    self.go(p);
                }
                None
            }
            MenuAct::StartBots => Some(self.local(None)),
            MenuAct::StartHost => {
                let internet = matches!(self.screen, Screen::Host { internet: true });
                let name = self.server_name.trim();
                Some(self.local(Some(Hosting {
                    internet,
                    name: if name.is_empty() {
                        "Lasertag".into()
                    } else {
                        name.to_string()
                    },
                    port: self.port,
                })))
            }
            MenuAct::Join(addr) => self.join(addr),
            MenuAct::JoinTyped if !self.address.trim().is_empty() => {
                let addr = self.address.trim().to_string();
                self.join(&addr)
            }
            MenuAct::JoinTyped => None,
            MenuAct::Refresh => {
                self.refresh();
                None
            }
            MenuAct::MoreBots(d) => {
                self.bots = (self.bots as i32 + d).clamp(0, 15) as usize;
                None
            }
            MenuAct::Difficulty(d) => {
                self.difficulty = *d;
                None
            }
            MenuAct::Arena(a) => {
                self.arena = *a;
                None
            }
            MenuAct::Mode(m) => {
                self.mode = *m;
                None
            }
            MenuAct::Team(t) => {
                self.team = *t;
                None
            }
            MenuAct::TeamSwitch(on) => {
                self.team_switch = *on;
                None
            }
            MenuAct::Modifier(k) => {
                if let Some(i) = self.modifiers.iter().position(|m| m == k) {
                    self.modifiers.remove(i);
                } else {
                    self.modifiers.push(*k);
                }
                None
            }
            MenuAct::Options => return Outcome::Options,
            MenuAct::Quit => return Outcome::Quit,
        };
        match start {
            Some(s) => {
                // Next time the menu opens at the top.
                self.go(Screen::Top);
                Outcome::Start(s)
            }
            None => Outcome::Stay,
        }
    }
}

/// The menu has made its choice: start this game.
#[derive(Resource, Clone)]
pub enum Start {
    Local {
        host: Option<Hosting>,
        bots: usize,
        difficulty: Difficulty,
        arena: SizeChoice,
        mode: GameMode,
        /// One's own team (`None`: the smaller one), and whether teams may be switched
        /// during a match (`Rules::team_switch`).
        team: Option<Team>,
        team_switch: bool,
        /// The match modifiers (`Rules::modifiers`).
        modifiers: Vec<ArenaEvent>,
    },
    Join(String),
}

/// Testing (`LT_DEV`, src/session.rs): play vs bots at this difficulty, as if clicked.
#[derive(Resource)]
pub struct DevPick(pub Difficulty);

/// Show the start menu (taken away when it is shown).
#[derive(Resource)]
pub struct ShowMenu;

/// The menu's model, for the flat screen and VR alike.
pub struct MenuModelPlugin;

impl Plugin for MenuModelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, |mut commands: Commands, args: Res<Args>| {
            let mut m = MenuModel::new(&args);
            if let Some(s) = std::env::var("LT_MENU")
                .ok()
                .as_deref()
                .and_then(Screen::parse)
            {
                m.go(s);
            }
            commands.insert_resource(m);
        })
        .add_systems(Update, |mut m: ResMut<MenuModel>| m.poll());
    }
}

/// The flat screen's menu.
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, first)
            .add_systems(Update, spawn.run_if(resource_exists::<ShowMenu>))
            .add_systems(
                Update,
                (
                    dev_pick,
                    keys,
                    buttons,
                    build,
                    show_lists,
                    scroll_list,
                    live,
                    selected,
                    tips,
                )
                    .chain()
                    .before(crate::options::OptionsSet)
                    .run_if(any_with_component::<MenuRoot>),
            );
    }
}

#[derive(Component)]
struct MenuRoot;

/// Holds the current level's contents (rebuilt when the level changes).
#[derive(Component)]
struct Page(Screen);

/// The server list's box (scrolled with the mouse wheel), and its scroll bar's track and thumb.
#[derive(Component)]
struct ServerList;

#[derive(Component)]
struct ScrollTrack;

#[derive(Component)]
struct ScrollThumb;

/// The server list's columns' widths: players, bots, score, time left.
const COLUMNS: [(&str, f32); 4] = [
    ("PLAYERS", 100.0),
    ("BOTS", 130.0),
    ("SCORE", 80.0),
    ("TIME", 64.0),
];
/// The list's height, and its scroll bar's width.
const LIST_H: f32 = 300.0;
const BAR_W: f32 = 6.0;

/// Texts that follow the model.
#[derive(Component, Clone, Copy, PartialEq)]
enum Live {
    Bots,
    Address,
    ServerName,
}

/// A button's tooltip: shown under it while the mouse is over it.
#[derive(Component, Clone, Copy)]
struct Tip(&'static str);

/// The tooltip shown (over everything).
#[derive(Component)]
struct TipPopup;

/// The team row: Infection hands out the sides itself, so it hides there.
#[derive(Component)]
struct TeamRow;

fn first(mut commands: Commands, args: Res<Args>) {
    if args.menu {
        commands.insert_resource(ShowMenu);
    }
}

fn spawn(mut commands: Commands, old: Query<Entity, With<MenuRoot>>) {
    commands.remove_resource::<ShowMenu>();
    if !old.is_empty() {
        return;
    }
    commands
        .spawn((MenuRoot, neonui::backdrop(neonui::BACKDROP, 10)))
        .with_children(|p| {
            neonui::title(p, "NEON LASERTAG", 64.0, neonui::PINK);
        });
}

fn close(commands: &mut Commands, root: &Query<Entity, With<MenuRoot>>) {
    for e in root {
        commands.entity(e).despawn();
    }
}

fn dev_pick(
    mut commands: Commands,
    pick: Option<Res<DevPick>>,
    mut model: ResMut<MenuModel>,
    root: Query<Entity, With<MenuRoot>>,
) {
    let Some(p) = pick else { return };
    commands.remove_resource::<DevPick>();
    model.difficulty = p.0;
    if let Outcome::Start(s) = model.apply(&MenuAct::StartBots) {
        close(&mut commands, &root);
        commands.insert_resource(s);
    }
}

fn act(
    a: &MenuAct,
    model: &mut MenuModel,
    commands: &mut Commands,
    root: &Query<Entity, With<MenuRoot>>,
    overlays: &mut Overlays,
    exit: &mut MessageWriter<AppExit>,
) {
    match model.apply(a) {
        Outcome::Stay => {}
        Outcome::Start(s) => {
            close(commands, root);
            commands.insert_resource(s);
        }
        Outcome::Options => overlays.options = true,
        Outcome::Quit => {
            exit.write(AppExit::Success);
        }
    }
}

fn buttons(
    mut commands: Commands,
    q: Query<(&Interaction, &MenuAct), Changed<Interaction>>,
    mut model: ResMut<MenuModel>,
    root: Query<Entity, With<MenuRoot>>,
    mut overlays: ResMut<Overlays>,
    mut exit: MessageWriter<AppExit>,
) {
    if overlays.options {
        return;
    }
    for (i, a) in &q {
        if *i == Interaction::Pressed {
            act(
                a,
                &mut model,
                &mut commands,
                &root,
                &mut overlays,
                &mut exit,
            );
            return;
        }
    }
}

/// Typing into the level's text field (the address, or the hosted game's name), Enter to go
/// on, Esc to go back.
fn keys(
    mut commands: Commands,
    mut input: MessageReader<KeyboardInput>,
    mut model: ResMut<MenuModel>,
    root: Query<Entity, With<MenuRoot>>,
    mut overlays: ResMut<Overlays>,
    mut exit: MessageWriter<AppExit>,
) {
    for k in input.read() {
        // Typing on the options screen isn't the menu's (and Esc closes that).
        if overlays.options || !k.state.is_pressed() {
            continue;
        }
        let naming = matches!(model.screen, Screen::Host { .. });
        let typing = naming || matches!(model.screen, Screen::Internet | Screen::Lan);
        let a = match &k.logical_key {
            Key::Escape => Some(MenuAct::Back),
            Key::Enter if naming => Some(MenuAct::StartHost),
            Key::Enter if typing => Some(MenuAct::JoinTyped),
            Key::Backspace if typing => {
                if naming {
                    model.server_name.pop();
                } else {
                    model.address.pop();
                }
                None
            }
            Key::Character(c) if typing => {
                for ch in c.chars() {
                    if naming {
                        if !ch.is_control() && model.server_name.chars().count() < 32 {
                            model.server_name.push(ch);
                        }
                    } else if (ch.is_ascii_alphanumeric() || ".:-[]".contains(ch))
                        && model.address.len() < 60
                    {
                        model.address.push(ch);
                    }
                }
                None
            }
            Key::Space if naming => {
                if model.server_name.chars().count() < 32 {
                    model.server_name.push(' ');
                }
                None
            }
            _ => None,
        };
        if let Some(a) = a {
            act(
                &a,
                &mut model,
                &mut commands,
                &root,
                &mut overlays,
                &mut exit,
            );
            return;
        }
    }
}

/// The choices' widths: a row's name, a choice.
const NAME_W: f32 = 170.0;
const CHOICE_W: f32 = 150.0;

/// The choices of a game: bots, difficulty, arena, mode (rows as wide as the arena's four).
fn choices(p: &mut ChildSpawnerCommands, model: &MenuModel) {
    let full = CHOICE_W * 4.0 + 30.0;
    let name = |r: &mut ChildSpawnerCommands, s: &str| {
        r.spawn(Node {
            width: Val::Px(NAME_W),
            ..default()
        })
        .with_children(|n| {
            n.spawn(neonui::text_w(s, 17.0, neonui::DIM, 700));
        });
    };
    let options = |r: &mut ChildSpawnerCommands,
                   items: Vec<(&'static str, MenuAct, Option<Tip>)>| {
        r.spawn((Node {
            width: Val::Px(full),
            column_gap: Val::Px(10.0),
            ..default()
        },))
            .with_children(|o| {
                for (s, a, tip) in items {
                    let neon = NeonButton {
                        color: neonui::CYAN,
                        selected: model.selected(&a),
                    };
                    let mut b = neonui::button(o, &s.to_uppercase(), a, neon, Some(CHOICE_W), 16.0);
                    if let Some(t) = tip {
                        b.insert(t);
                    }
                }
            });
    };
    fn plain(items: Vec<(&'static str, MenuAct)>) -> Vec<(&'static str, MenuAct, Option<Tip>)> {
        items.into_iter().map(|(s, a)| (s, a, None)).collect()
    }
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "BOTS");
        r.spawn((Node {
            width: Val::Px(full),
            column_gap: Val::Px(10.0),
            align_items: AlignItems::Center,
            ..default()
        },))
            .with_children(|o| {
                let step = NeonButton::new(neonui::VIOLET);
                neonui::button(o, "-", MenuAct::MoreBots(-1), step, Some(52.0), 18.0);
                o.spawn(Node {
                    width: Val::Px(70.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_children(|n| {
                    n.spawn((Live::Bots, neonui::text_w("", 22.0, neonui::TEXT, 800)));
                });
                neonui::button(o, "+", MenuAct::MoreBots(1), step, Some(52.0), 18.0);
            });
    });
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "DIFFICULTY");
        options(
            r,
            plain(
                Difficulty::ALL
                    .iter()
                    .map(|d| (d.name(), MenuAct::Difficulty(*d)))
                    .collect(),
            ),
        );
    });
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "ARENA");
        options(
            r,
            plain(
                SizeChoice::ALL
                    .iter()
                    .map(|a| (a.name(), MenuAct::Arena(*a)))
                    .collect(),
            ),
        );
    });
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "MODE");
        options(
            r,
            GameMode::ALL
                .iter()
                .map(|m| (m.name(), MenuAct::Mode(*m), Some(Tip(mode_blurb(*m)))))
                .collect(),
        );
    });
    let mut team_row = p.spawn((TeamRow, neonui::row(10.0)));
    if model.mode == GameMode::Infection {
        team_row.insert(Node {
            display: Display::None,
            ..neonui::row(10.0)
        });
    }
    team_row.with_children(|r| {
        name(r, "YOUR TEAM");
        options(
            r,
            plain(
                [None, Some(Team::Red), Some(Team::Blue)]
                    .iter()
                    .map(|t| (team_name(*t), MenuAct::Team(*t)))
                    .collect(),
            ),
        );
    });
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "SWITCH TEAMS");
        options(
            r,
            vec![
                (
                    "allowed",
                    MenuAct::TeamSwitch(true),
                    Some(Tip(
                        "PLAYERS MAY CHANGE SIDES FROM THE IN-GAME MENU - BOTS EVEN THE TEAMS OUT",
                    )),
                ),
                (
                    "locked",
                    MenuAct::TeamSwitch(false),
                    Some(Tip("EVERYONE STAYS ON THE TEAM THEY STARTED ON")),
                ),
            ],
        );
    });
    // (Any number of them, each on or off.)
    p.spawn(neonui::row(10.0)).with_children(|r| {
        name(r, "MODIFIERS");
        options(
            r,
            ArenaEvent::MODIFIERS
                .iter()
                .map(|k| {
                    (
                        k.modifier_name(),
                        MenuAct::Modifier(*k),
                        Some(Tip(modifier_blurb(*k))),
                    )
                })
                .collect(),
        );
    });
}

/// A text field showing `live`.
fn field(p: &mut ChildSpawnerCommands, title: &str, live: Live, width: f32) {
    p.spawn(neonui::row(12.0)).with_children(|r| {
        r.spawn(neonui::text_w(title, 17.0, neonui::DIM, 700));
        r.spawn((
            Node {
                width: Val::Px(width),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            BorderColor::all(neonui::CYAN.with_alpha(0.6)),
            BoxShadow::new(
                neonui::CYAN.with_alpha(0.25),
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(0.0),
                Val::Px(10.0),
            ),
        ))
        .with_children(|f| {
            f.spawn((live, neonui::text("", 18.0, neonui::GREEN)));
        });
    });
}

/// Back, and the level's way on.
fn bottom_row(p: &mut ChildSpawnerCommands, go: &[(&str, MenuAct, Color, f32)]) {
    p.spawn(neonui::gap(4.0));
    p.spawn(neonui::row(16.0)).with_children(|r| {
        let back = NeonButton::new(neonui::VIOLET);
        neonui::button(r, "BACK", MenuAct::Back, back, Some(180.0), 20.0);
        for (label, a, c, w) in go {
            neonui::button(r, label, a.clone(), NeonButton::new(*c), Some(*w), 20.0);
        }
    });
}

/// A level's big buttons, one under the other.
fn big_buttons(p: &mut ChildSpawnerCommands, items: &[(&str, MenuAct, Color)]) {
    for (label, a, c) in items {
        neonui::button(p, label, a.clone(), NeonButton::new(*c), Some(400.0), 26.0);
    }
}

fn page(p: &mut ChildSpawnerCommands, model: &MenuModel) {
    let screen = model.screen;
    p.spawn(neonui::text_w(screen.crumb(), 20.0, neonui::CYAN, 700));
    p.spawn(neonui::gap(4.0));
    match screen {
        Screen::Top => {
            p.spawn(neonui::panel(neonui::MAGENTA, None))
                .with_children(|p| {
                    // The browser build: the internet's servers only, and no quitting (the tab
                    // is closed).
                    let multiplayer = if crate::WEB {
                        Screen::Internet
                    } else {
                        Screen::Multiplayer
                    };
                    let items = [
                        ("BOTMATCH", MenuAct::Go(Screen::Botmatch), neonui::CYAN),
                        ("MULTIPLAYER", MenuAct::Go(multiplayer), neonui::MAGENTA),
                        ("OPTIONS", MenuAct::Options, neonui::VIOLET),
                        ("QUIT", MenuAct::Quit, neonui::PINK),
                    ];
                    let n = if crate::WEB { 3 } else { 4 };
                    big_buttons(p, &items[..n]);
                });
            p.spawn(neonui::gap(8.0));
            p.spawn(neonui::text(
                "WASD move  /  Ctrl crouch  /  mouse look  /  left click fire  /  \
                 hold G or right click: grenade  /  Esc menu  (keys in Options)",
                13.0,
                neonui::DIM,
            ));
        }
        Screen::Botmatch => {
            p.spawn(neonui::panel(neonui::CYAN, None))
                .with_children(|p| {
                    choices(p, model);
                    bottom_row(p, &[("START", MenuAct::StartBots, neonui::CYAN, 320.0)]);
                });
        }
        Screen::Multiplayer => {
            p.spawn(neonui::panel(neonui::MAGENTA, None))
                .with_children(|p| {
                    big_buttons(
                        p,
                        &[
                            ("INTERNET", MenuAct::Go(Screen::Internet), neonui::CYAN),
                            ("LAN", MenuAct::Go(Screen::Lan), neonui::MAGENTA),
                        ],
                    );
                    bottom_row(p, &[]);
                });
        }
        Screen::Internet | Screen::Lan => {
            let internet = screen == Screen::Internet;
            let color = if internet {
                neonui::CYAN
            } else {
                neonui::MAGENTA
            };
            p.spawn(neonui::panel(color, Some(940.0)))
                .with_children(|p| {
                    p.spawn(Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|r| {
                        let head = if internet {
                            "SERVERS"
                        } else {
                            "GAMES ON YOUR NETWORK"
                        };
                        r.spawn(neonui::text_w(head, 20.0, neonui::TEXT, 800));
                        if internet {
                            let n = NeonButton::new(neonui::VIOLET);
                            neonui::button(r, "REFRESH", MenuAct::Refresh, n, None, 15.0);
                        }
                    });
                    // The columns' heads (over the rows' right parts: the rows' padding, and the
                    // scroll bar beside them).
                    p.spawn(Node {
                        width: Val::Percent(100.0),
                        padding: UiRect {
                            left: Val::Px(18.0),
                            right: Val::Px(18.0 + BAR_W + 8.0),
                            ..default()
                        },
                        column_gap: Val::Px(12.0),
                        ..default()
                    })
                    .with_children(|h| {
                        h.spawn(Node {
                            flex_grow: 1.0,
                            ..default()
                        })
                        .with_child(neonui::text_w(
                            "SERVER",
                            12.0,
                            neonui::DIM,
                            700,
                        ));
                        for (name, w) in COLUMNS {
                            h.spawn(Node {
                                width: Val::Px(w),
                                justify_content: JustifyContent::FlexEnd,
                                ..default()
                            })
                            .with_child(neonui::text_w(
                                name,
                                12.0,
                                neonui::DIM,
                                700,
                            ));
                        }
                    });
                    p.spawn(Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(LIST_H),
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|r| {
                        r.spawn((
                            ServerList,
                            Node {
                                flex_grow: 1.0,
                                min_width: Val::Px(0.0),
                                height: Val::Percent(100.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            ScrollPosition::default(),
                            bevy::ui::RelativeCursorPosition::default(),
                        ));
                        r.spawn((
                            ScrollTrack,
                            Node {
                                width: Val::Px(BAR_W),
                                height: Val::Percent(100.0),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(color.with_alpha(0.12)),
                            Visibility::Hidden,
                        ))
                        .with_child((
                            ScrollThumb,
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Percent(100.0),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
                                ..default()
                            },
                            BackgroundColor(color.with_alpha(0.8)),
                        ));
                    });
                    p.spawn(neonui::row(12.0)).with_children(|r| {
                        field(r, "JOIN ADDRESS", Live::Address, 360.0);
                        let n = NeonButton::new(neonui::GREEN);
                        neonui::button(r, "JOIN", MenuAct::JoinTyped, n, Some(120.0), 18.0);
                    });
                    let host = if internet {
                        "HOST INTERNET GAME"
                    } else {
                        "HOST LAN GAME"
                    };
                    let go = MenuAct::Go(Screen::Host { internet });
                    // A browser can't host (no UDP).
                    let n = if crate::WEB { 0 } else { 1 };
                    bottom_row(p, &[(host, go, neonui::AMBER, 360.0)][..n]);
                });
        }
        Screen::Host { internet } => {
            let color = if internet {
                neonui::CYAN
            } else {
                neonui::MAGENTA
            };
            p.spawn(neonui::panel(color, None)).with_children(|p| {
                field(p, "SERVER NAME", Live::ServerName, 520.0);
                p.spawn(neonui::gap(4.0));
                choices(p, model);
                p.spawn(neonui::gap(2.0));
                let port = model.port;
                let note = if internet {
                    format!(
                        "Listed on the master server. UDP port {port} is opened on your router \
                         (UPnP / NAT-PMP);\nif the router doesn't allow it, forward that port \
                         to this computer."
                    )
                } else {
                    format!("Players on your network find it under LAN (UDP port {port}).")
                };
                p.spawn((
                    neonui::text(note, 13.0, neonui::DIM),
                    TextLayout::justify(Justify::Center),
                ));
                bottom_row(
                    p,
                    &[("START HOSTING", MenuAct::StartHost, neonui::AMBER, 320.0)],
                );
            });
        }
    }
}

/// Builds the level's page when the level changes.
fn build(
    mut commands: Commands,
    model: Res<MenuModel>,
    root: Query<Entity, With<MenuRoot>>,
    pages: Query<(Entity, &Page)>,
) {
    let Ok(root) = root.single() else { return };
    if pages.iter().any(|(_, p)| p.0 == model.screen) {
        return;
    }
    for (e, _) in &pages {
        commands.entity(e).despawn();
    }
    commands
        .spawn((
            Page(model.screen),
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                ..default()
            },
            ChildOf(root),
        ))
        .with_children(|p| page(p, &model));
}

/// One server: a button to join it (greyed, not a button, if it speaks another protocol): its
/// name over its mode and arena, then the columns (`COLUMNS`): players, bots, score, time left.
fn server_row(
    commands: &mut Commands,
    parent: Entity,
    addr: &str,
    r: &lt_net::ServerReport,
    joinable: bool,
) {
    let color = if joinable { neonui::CYAN } else { neonui::DIM };
    let mut row = commands.spawn((
        ChildOf(parent),
        neonui::button_node(
            MenuAct::Join(addr.to_string()),
            NeonButton::new(color),
            None,
        ),
    ));
    row.insert(Node {
        width: Val::Percent(100.0),
        // (Kept whole in the scrolled box.)
        flex_shrink: 0.0,
        padding: UiRect::axes(Val::Px(16.0), Val::Px(6.0)),
        border: UiRect::all(Val::Px(2.0)),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        column_gap: Val::Px(12.0),
        align_items: AlignItems::Center,
        ..default()
    });
    if !joinable {
        row.remove::<(Button, MenuAct)>();
    }
    let what = if r.mode.is_empty() {
        String::new()
    } else {
        format!(
            "{}  /  {} ARENA",
            r.mode.to_uppercase(),
            r.arena.to_uppercase()
        )
    };
    row.with_children(|b| {
        b.spawn(Node {
            flex_grow: 1.0,
            min_width: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip_x(),
            ..default()
        })
        .with_children(|n| {
            n.spawn(neonui::text_w(r.name.clone(), 17.0, neonui::TEXT, 700));
            n.spawn(neonui::text(what, 12.0, neonui::DIM));
        });
        if !joinable {
            b.spawn(neonui::text(
                format!("another version ({})", r.game_version),
                14.0,
                neonui::DIM,
            ));
            return;
        }
        let bots_on = r.bots_enabled || r.bots > 0;
        let cells = [
            (crate::menu::players_text(r), neonui::TEXT),
            (
                bots_text(r),
                if bots_on { neonui::CYAN } else { neonui::DIM },
            ),
            (format!("{} : {}", r.scores[0], r.scores[1]), neonui::DIM),
            (
                format!("{}:{:02}", r.time_left_s / 60, r.time_left_s % 60),
                neonui::DIM,
            ),
        ];
        for ((text, c), (_, w)) in cells.into_iter().zip(COLUMNS) {
            b.spawn(Node {
                width: Val::Px(w),
                flex_shrink: 0.0,
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_child(neonui::text(text, 14.0, c));
        }
    });
}

/// The server list: the mouse wheel over it scrolls it; its scroll bar shows when it has more
/// than fits.
#[allow(clippy::type_complexity)]
fn scroll_list(
    wheel: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut list: Query<
        (
            &ComputedNode,
            &mut ScrollPosition,
            &bevy::ui::RelativeCursorPosition,
        ),
        With<ServerList>,
    >,
    mut track: Query<&mut Visibility, With<ScrollTrack>>,
    mut thumb: Query<&mut Node, With<ScrollThumb>>,
) {
    use bevy::input::mouse::MouseScrollUnit;
    let Ok((node, mut pos, cursor)) = list.single_mut() else {
        return;
    };
    let scale = node.inverse_scale_factor();
    let seen = node.size().y * scale;
    let content = node.content_size().y * scale;
    let room = (content - seen).max(0.0);
    if cursor.cursor_over() && wheel.delta.y != 0.0 {
        let dy = match wheel.unit {
            MouseScrollUnit::Line => wheel.delta.y * 40.0,
            MouseScrollUnit::Pixel => wheel.delta.y,
        };
        pos.y = (pos.y - dy).clamp(0.0, room);
    } else if pos.y > room {
        pos.y = room;
    }
    let shows = room > 0.5;
    if let Ok(mut v) = track.single_mut() {
        let want = if shows {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    if shows && let Ok(mut t) = thumb.single_mut() {
        let h = (seen / content * 100.0).clamp(10.0, 100.0);
        let top = pos.y / room * (100.0 - h);
        let (h, top) = (Val::Percent(h), Val::Percent(top));
        if t.height != h || t.top != top {
            t.height = h;
            t.top = top;
        }
    }
}

/// Fills the server list when it changes.
fn show_lists(
    mut commands: Commands,
    model: Res<MenuModel>,
    list: Query<Entity, With<ServerList>>,
    mut shown: Local<Option<(u32, Entity)>>,
) {
    let Ok(parent) = list.single() else { return };
    if *shown == Some((model.rev, parent)) {
        return;
    }
    *shown = Some((model.rev, parent));
    commands.entity(parent).despawn_children();
    let say = |commands: &mut Commands, s: String, c: Color| {
        commands.spawn((
            ChildOf(parent),
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                ..default()
            },
            children![neonui::text(s, 16.0, c)],
        ));
    };
    if model.screen == Screen::Lan {
        if let Some(e) = &model.lan_error {
            say(&mut commands, e.clone(), neonui::PINK);
        } else if model.lan_servers.is_empty() {
            let s = "searching the network...".to_string();
            say(&mut commands, s, neonui::DIM);
        }
        for s in &model.lan_servers {
            server_row(&mut commands, parent, &s.addr, &s.report, s.joinable());
        }
        return;
    }
    if let Some(e) = &model.join_error {
        say(&mut commands, e.clone(), neonui::PINK);
    }
    match &model.internet {
        InternetList::Idle | InternetList::Asking => {
            let s = "asking the master server...".to_string();
            say(&mut commands, s, neonui::DIM);
        }
        InternetList::Failed(e) => {
            let s = format!("server list unavailable: {e}");
            say(&mut commands, s, neonui::PINK);
        }
        InternetList::Listed(l) if l.is_empty() => {
            let s = "no servers listed: host one!".to_string();
            say(&mut commands, s, neonui::DIM);
        }
        InternetList::Listed(l) => {
            for s in l {
                let joinable = s.report.protocol == lt_net::protocol::PROTOCOL;
                server_row(&mut commands, parent, &s.addr, &s.report, joinable);
            }
        }
    }
}

/// The texts that follow the model (with a blinking caret in the fields).
fn live(model: Res<MenuModel>, time: Res<Time>, mut q: Query<(&Live, &mut Text)>) {
    let caret = if (time.elapsed_secs() * 2.0).fract() < 0.5 {
        "_"
    } else {
        " "
    };
    for (l, mut t) in &mut q {
        let want = match l {
            Live::Bots => model.bots.to_string(),
            Live::Address => format!("{}{caret}", model.address),
            Live::ServerName => format!("{}{caret}", model.server_name),
        };
        if t.0 != want {
            t.0 = want;
        }
    }
}

/// The current choices lit (and the team row away in Infection).
fn selected(
    model: Res<MenuModel>,
    mut q: Query<(&MenuAct, &mut NeonButton)>,
    mut rows: Query<&mut Node, With<TeamRow>>,
) {
    if !model.is_changed() {
        return;
    }
    for (a, mut n) in &mut q {
        let on = model.selected(a);
        if n.selected != on {
            n.selected = on;
        }
    }
    let display = if model.mode == GameMode::Infection {
        Display::None
    } else {
        Display::Flex
    };
    for mut n in &mut rows {
        if n.display != display {
            n.display = display;
        }
    }
}

/// The tooltip of the button the mouse is over, under it (spawned and taken down as the mouse
/// moves; its text is in the button's `Tip`).
fn tips(
    mut commands: Commands,
    root: Query<Entity, With<MenuRoot>>,
    hovered: Query<(&Interaction, &Tip, &ComputedNode, &UiGlobalTransform)>,
    popups: Query<Entity, With<TipPopup>>,
    mut shown: Local<Option<&'static str>>,
) {
    let Ok(root) = root.single() else { return };
    // (A button not laid out yet has no place to hang the tooltip under.)
    let over = hovered
        .iter()
        .filter(|(_, _, n, _)| n.size != Vec2::ZERO)
        .find(|(i, ..)| matches!(i, Interaction::Hovered | Interaction::Pressed));
    if over.map(|(_, t, ..)| t.0) == *shown {
        return;
    }
    for e in &popups {
        commands.entity(e).despawn();
    }
    *shown = over.map(|(_, t, ..)| t.0);
    let Some((_, tip, node, transform)) = over else {
        return;
    };
    // (The transform is in physical pixels: the node's middle.)
    let k = node.inverse_scale_factor;
    let centre = transform.translation * k;
    let half = node.size * k / 2.0;
    const W: f32 = 340.0;
    commands.spawn((
        TipPopup,
        ChildOf(root),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(centre.x - W / 2.0),
            top: Val::Px(centre.y + half.y + 8.0),
            width: Val::Px(W),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.02, 0.01, 0.08, 0.96)),
        BorderColor::all(neonui::CYAN.with_alpha(0.7)),
        BoxShadow::new(
            neonui::CYAN.with_alpha(0.3),
            Val::Px(0.0),
            Val::Px(0.0),
            Val::Px(0.0),
            Val::Px(12.0),
        ),
        GlobalZIndex(15),
        children![(
            neonui::text(tip.0, 12.0, neonui::TEXT),
            TextLayout::justify(Justify::Center),
        )],
    ));
}

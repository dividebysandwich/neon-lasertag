//! The running game: a local simulation (single player with bots, or a listen host) or a
//! connection to a server. Stepped in `FixedUpdate` at the simulation's tick; the rest of the
//! client only reads `Game::snap` and the `GameEvent` messages.

use std::collections::VecDeque;
use std::sync::Arc;

use bevy::prelude::*;
use lt_net::client::{Client, ClientEvent};
use lt_net::server::Server;
use lt_net::{LanAnnouncer, PortMapper, PortStatus, Reporter};
use lt_sim::bots::Bots;
use lt_sim::collide::World;
use lt_sim::level::Level;
use lt_sim::sim::{PlayerSnap, Snapshot};
use lt_sim::{PlayerCmd, PlayerId, Rules, Sim, SimEvent, Team};

#[allow(clippy::large_enum_variant)]
pub enum Source {
    Local {
        sim: Sim,
        bots: Bots,
        server: Option<Server>,
        /// A hosting game's announcements (LAN, master server) and router port.
        net: Option<HostNet>,
    },
    Remote(Client),
}

/// Host a game that others join: on the local network (announced there), or on the internet
/// too (listed on the master server, its port opened on the router).
#[derive(Clone, Debug, PartialEq)]
pub struct Hosting {
    pub internet: bool,
    pub name: String,
    pub port: u16,
}

/// Ticks between a hosting game's reports to the LAN and the master server.
const REPORT_TICKS: u64 = 120;

pub struct HostNet {
    port: u16,
    lan: LanAnnouncer,
    internet: Option<(Reporter, PortMapper)>,
    /// The status last logged.
    said: String,
}

impl HostNet {
    fn new(h: &Hosting, port: u16) -> HostNet {
        let internet = h.internet.then(|| {
            let master =
                std::env::var("LT_MASTER").unwrap_or_else(|_| lt_net::DEFAULT_MASTER.to_string());
            (Reporter::new(&master), PortMapper::open(port))
        });
        HostNet {
            port,
            lan: LanAnnouncer::new(),
            internet,
            said: String::new(),
        }
    }

    /// Tells the network about the server now and then; logs what happens.
    fn tick(&mut self, server: &Server, sim: &Sim) {
        if sim.tick % REPORT_TICKS != 1 {
            return;
        }
        self.lan.update(server.report(sim, self.port, false));
        if let Some((reporter, mapper)) = &self.internet {
            // Outside, players join on the port the router opened.
            let port = mapper.external_port().unwrap_or(self.port);
            reporter.update(server.report(sim, port, false));
            for line in reporter.take_log() {
                info!("master server: {line}");
            }
        }
        let now = self.status();
        if now != self.said {
            info!("{now}");
            if let Some((_, m)) = &self.internet
                && let PortStatus::Failed(e) = m.status()
            {
                warn!("port mapping: {e}");
            }
            self.said = now;
        }
    }

    /// Where the server can be reached (for the menus).
    pub fn status(&self) -> String {
        let port = self.port;
        match &self.internet {
            None => format!("Hosting on the local network, UDP port {port}"),
            Some((_, m)) => match m.status() {
                PortStatus::Trying => {
                    format!("Hosting on UDP port {port}: opening it on the router...")
                }
                PortStatus::Open { via, external } => {
                    format!("Hosting on the internet at {external} (port opened with {via})")
                }
                PortStatus::Failed(_) => {
                    format!(
                        "The router didn't open the port: forward UDP port {port} to this computer"
                    )
                }
            },
        }
    }
}

#[derive(Resource)]
pub struct Game {
    pub level: Arc<Level>,
    /// The arena's collision, for the local player's prediction and aiming.
    pub world: Arc<World>,
    pub rules: Rules,
    pub me: Option<PlayerId>,
    pub source: Source,
    /// The latest state, and the one before (to interpolate between ticks).
    pub snap: Snapshot,
    pub prev: Snapshot,
    /// Player names by id.
    pub names: Vec<(PlayerId, String, Team)>,
    pub status: String,
    /// Remote games: the snapshots received, played back a little behind (see `Interp`).
    pub interp: Interp,
    /// Remote games: the local player's shots, worked out at once (src/shots.rs).
    pub shots: crate::shots::Shots,
}

/// Remote games are drawn a few ticks behind the newest snapshot, interpolating between the
/// two around that moment, so other players move smoothly however the packets arrive. The
/// delay follows the measured jitter. Each tick it makes one interpolated snapshot (`snap`,
/// the previous one in `prev`), so drawing works as in a local game. Events wait until the
/// view reaches the tick they came with.
#[derive(Default)]
pub struct Interp {
    buffer: VecDeque<Snapshot>,
    /// The tick being shown (fractional), advancing about one per tick.
    view: f64,
    /// Ticks counted locally, to measure when snapshots arrive against their ticks.
    clock: u64,
    /// Lowest (arrival - tick) seen lately, and how much later than that they come (jitter).
    base: Option<f64>,
    jitter: f64,
    pending: VecDeque<(u64, SimEvent)>,
}

/// Snapshots come every other tick: this many ticks behind the newest at least.
const MIN_DELAY: f64 = 3.0;
const MAX_DELAY: f64 = 14.0;

impl Interp {
    /// The tick the player is looking at (0 before anything arrived): what lag compensation
    /// rewinds to.
    pub fn view_tick(&self) -> u64 {
        self.view.max(0.0) as u64
    }

    fn reset(&mut self) {
        *self = Interp::default();
    }

    /// A player as of the newest snapshot received.
    pub fn newest(&self, id: PlayerId) -> Option<&PlayerSnap> {
        self.buffer.back()?.players.iter().find(|p| p.id == id)
    }

    fn receive(&mut self, s: Snapshot) {
        if self.buffer.back().is_some_and(|b| b.tick >= s.tick) {
            return;
        }
        let offset = self.clock as f64 - s.tick as f64;
        let base = self.base.map_or(offset, |b| (b + 0.002).min(offset));
        self.base = Some(base);
        let late = offset - base;
        self.jitter = if late > self.jitter {
            late
        } else {
            self.jitter * 0.995 + late * 0.005
        };
        if self.buffer.is_empty() {
            self.view = s.tick as f64 - MIN_DELAY;
        }
        self.buffer.push_back(s);
        while self.buffer.len() > 64 {
            self.buffer.pop_front();
        }
    }

    /// Advances the view by a tick and gives the snapshot to show, or None if nothing came yet.
    fn advance(&mut self, me: Option<PlayerId>) -> Option<Snapshot> {
        self.clock += 1;
        let newest = self.buffer.back()?.tick as f64;
        let delay = (MIN_DELAY + self.jitter * 1.2).clamp(MIN_DELAY, MAX_DELAY);
        let target = newest - delay;
        // Drift towards the target smoothly; jump when far off (a stall, a new level).
        let error = target - self.view;
        self.view += if error.abs() > 30.0 {
            error + 1.0
        } else {
            1.0 + (error * 0.05).clamp(-0.15, 0.15)
        };
        let view = self.view.min(newest);
        while self.buffer.len() > 2 && self.buffer[1].tick as f64 <= view {
            self.buffer.pop_front();
        }
        let a = &self.buffer[0];
        let b = self.buffer.get(1).unwrap_or(a);
        let k = if b.tick > a.tick {
            ((view - a.tick as f64) / (b.tick - a.tick) as f64).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        let mut out = blend(a, b, k, view);
        // The local player as of the newest snapshot (its HUD, its prediction's correction).
        if let Some(me) = me
            && let Some(p) = self
                .buffer
                .back()
                .and_then(|n| n.players.iter().find(|p| p.id == me))
        {
            out.players.retain(|q| q.id != me);
            out.players.push(p.clone());
        }
        Some(out)
    }

    /// Events due now (their tick reached by the view).
    fn due(&mut self) -> Vec<SimEvent> {
        let view = self.view_tick();
        let mut out = Vec::new();
        while self.pending.front().is_some_and(|(t, _)| *t <= view) {
            out.push(self.pending.pop_front().unwrap().1);
        }
        out
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * t
}

/// The state between snapshots `a` and `b` (`k` of the way), shown as tick `view`.
fn blend(a: &Snapshot, b: &Snapshot, k: f32, view: f64) -> Snapshot {
    let dt = |tick: u64| (view - tick as f64) as f32 * lt_sim::TICK;
    let mut out = a.clone();
    out.tick = view.max(0.0) as u64;
    out.time_left = a.time_left + (b.time_left - a.time_left) * k;
    out.players = a
        .players
        .iter()
        .map(|p| match b.players.iter().find(|q| q.id == p.id) {
            // Not across a respawn (a jump).
            Some(q)
                if (q.feet - p.feet).length() < 3.0
                    && p.respawn.is_none() == q.respawn.is_none() =>
            {
                let mut r: PlayerSnap = p.clone();
                r.feet = p.feet.lerp(q.feet, k);
                r.yaw = lerp_angle(p.yaw, q.yaw, k);
                r.head.pos = p.head.pos.lerp(q.head.pos, k);
                r.head.rot = p.head.rot.slerp(q.head.rot, k);
                for i in 0..2 {
                    r.hands[i].pos = p.hands[i].pos.lerp(q.hands[i].pos, k);
                    r.hands[i].rot = p.hands[i].rot.slerp(q.hands[i].rot, k);
                }
                r.cooldown = p.cooldown + (q.cooldown - p.cooldown) * k;
                r.protect = p.protect + (q.protect - p.protect) * k;
                r
            }
            _ => p.clone(),
        })
        .collect();
    // Bolts: by id where both have them, else moved along their velocity.
    out.bolts = b
        .bolts
        .iter()
        .map(|q| {
            let mut r = *q;
            r.pos = match a.bolts.iter().find(|p| p.id == q.id) {
                Some(p) => p.pos.lerp(q.pos, k),
                None => q.pos + q.vel * dt(b.tick),
            };
            r
        })
        .filter(|r| r.pos.is_finite())
        .collect();
    out.grenades = a
        .grenades
        .iter()
        .map(|p| {
            let mut r = *p;
            if let Some(q) = b.grenades.iter().find(|q| q.id == p.id) {
                r.pos = p.pos.lerp(q.pos, k);
                r.spin = p.spin + (q.spin - p.spin) * k;
            }
            r
        })
        .collect();
    out
}

impl Game {
    /// A local game: `bots` bots of `skill` (`Difficulty::skill`).
    pub fn local(
        seed: u64,
        bots: usize,
        skill: f32,
        rules: Rules,
        name: &str,
        host: Option<Hosting>,
    ) -> Game {
        let mut sim = Sim::new(seed, rules.clone());
        let me = sim.add_player(name, None, false);
        let mut b = Bots::default();
        b.fill(&mut sim, bots, skill);
        sim.begin();
        let server = host
            .as_ref()
            .and_then(|h| match Server::listen(h.port, &h.name, seed) {
                Ok(s) => Some(s),
                Err(e) => {
                    error!("can't host on port {}: {e}", h.port);
                    None
                }
            });
        let net = host
            .as_ref()
            .zip(server.as_ref())
            .map(|(h, s)| HostNet::new(h, s.port().unwrap_or(h.port)));
        let snap = sim.snapshot();
        let names = sim
            .players
            .iter()
            .map(|p| (p.id, p.name.clone(), p.team))
            .collect();
        Game {
            world: Arc::new(sim.level.world()),
            level: sim.level.clone(),
            rules,
            me,
            snap: snap.clone(),
            prev: snap,
            names,
            status: String::new(),
            interp: Interp::default(),
            shots: Default::default(),
            source: Source::Local {
                sim,
                bots: b,
                server,
                net,
            },
        }
    }

    pub fn remote(client: Client, level: Arc<Level>, rules: Rules, me: PlayerId) -> Game {
        Game {
            world: Arc::new(level.world()),
            level,
            rules,
            me: Some(me),
            snap: Snapshot::default(),
            prev: Snapshot::default(),
            names: Vec::new(),
            status: String::new(),
            interp: Interp::default(),
            shots: Default::default(),
            source: Source::Remote(client),
        }
    }

    pub fn my_team(&self) -> Option<Team> {
        let me = self.me?;
        self.snap
            .players
            .iter()
            .find(|p| p.id == me)
            .map(|p| p.team)
    }

    pub fn name_of(&self, id: PlayerId) -> &str {
        self.names
            .iter()
            .find(|n| n.0 == id)
            .map_or("?", |n| n.1.as_str())
    }
}

/// What the local player wants this tick (desktop or VR input fills it in).
#[derive(Resource, Default)]
pub struct LocalCmd {
    pub cmd: PlayerCmd,
    /// The trigger was pulled since the last tick (so a quick click isn't lost between ticks).
    pub fire_latch: bool,
    /// Seconds more to hold the trigger down (testing: `LT_DEV` `fire=S`), and a barrier to
    /// put up (`barrier`).
    pub dev_fire: f32,
    pub dev_barrier: bool,
    /// A bot plays the local player (testing).
    pub autopilot: bool,
    /// Physical walking (VR) since the last frame's prediction, world XZ metres.
    pub walk_frame: Vec2,
}

/// The local player's own prediction of where its feet are: moves at once with the input
/// (and with the head in VR), with the same collision as the simulation, which accepts it.
#[derive(Resource, Default)]
pub struct Mover {
    pub feet: Vec3,
    pub vy: f32,
    /// Flying from a jump pad: the horizontal velocity it gave (as `lt_sim::Player::air`).
    pub air: Vec2,
    /// Stick movement (world XZ, m/s): eases towards what the stick asks for (a little inertia).
    pub vel: Vec2,
    /// Jumping (as `lt_sim::Player::moves`).
    pub moves: lt_sim::sim::Moves,
    pub valid: bool,
    seed: u64,
}

/// Drawing the actors (after the prediction and the VR poses).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct DrawSet;

/// Prediction runs after the input and before anything that places the camera or hands.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PredictSet;

#[derive(Message, Clone, Copy, Debug)]
pub struct GameEvent(pub SimEvent);

/// Systems that fill in `LocalCmd` before the step.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputSet;

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_seconds(lt_sim::TICK as f64))
            .init_resource::<LocalCmd>()
            .init_resource::<Mover>()
            .init_resource::<Recap>()
            .configure_sets(
                Update,
                (PredictSet.after(InputSet), DrawSet.after(PredictSet)),
            )
            .add_systems(
                Update,
                (predict, track_recap)
                    .in_set(PredictSet)
                    .run_if(resource_exists::<Game>),
            )
            .add_message::<GameEvent>()
            .add_systems(FixedUpdate, step_game.run_if(resource_exists::<Game>));
    }
}

fn step_game(
    mut game: ResMut<Game>,
    mut local: ResMut<LocalCmd>,
    mut events: MessageWriter<GameEvent>,
    mut session: MessageWriter<crate::session::SessionCmd>,
) {
    let game = &mut *game;
    let mut cmd = local.cmd;
    if local.fire_latch {
        cmd.fire = true;
        local.fire_latch = false;
    }
    if local.dev_fire > 0.0 {
        cmd.fire = true;
        local.dev_fire -= lt_sim::TICK;
    }
    if std::mem::take(&mut local.dev_barrier) {
        cmd.barrier = true;
    }
    // (Nobody fires in the countdown: no bolt shown that the sim won't fire.)
    if matches!(game.snap.phase, lt_sim::MatchPhase::Countdown(_)) {
        cmd.fire = false;
    }
    // A throw is sent once.
    local.cmd.throw = None;
    match &mut game.source {
        Source::Local {
            sim,
            bots,
            server,
            net,
        } => {
            if let Some(me) = game.me {
                sim.set_cmd(me, cmd);
            }
            bots.think(sim);
            if let Some(server) = server {
                server.before_step(sim, bots);
            }
            sim.step();
            let evs = sim.drain_events();
            if let Some(server) = server {
                server.after_step(sim, &evs);
                for line in server.log.drain(..) {
                    info!("server: {line}");
                }
                if let Some(n) = net {
                    n.tick(server, sim);
                }
            }
            for e in evs {
                events.write(GameEvent(e));
            }
            game.prev = std::mem::replace(&mut game.snap, sim.snapshot());
            if sim.tick % 30 == 0 || game.names.len() != sim.players.len() {
                game.names = sim
                    .players
                    .iter()
                    .map(|p| (p.id, p.name.clone(), p.team))
                    .collect();
            }
        }
        Source::Remote(client) => {
            // What the player sees, for the server's lag compensation.
            cmd.view_tick = game.interp.view_tick();
            // The shot, at once (src/shots.rs).
            let mut own = Vec::new();
            if let Some(me) = game.me {
                // (As the server last told: `game.snap` has the prediction's ammo.)
                let p = game.interp.newest(me);
                let feet = cmd.feet.or(p.map(|p| p.feet)).unwrap_or_default();
                game.shots
                    .trigger(&mut cmd, p, feet, &game.world, &game.rules, me, &mut own);
            }
            client.send_input(cmd);
            for ev in client.poll() {
                match ev {
                    ClientEvent::Snapshot(s) => game.interp.receive(s),
                    ClientEvent::Events(evs) => {
                        // Shown when the view gets to the newest tick received with them.
                        let tick = game.interp.buffer.back().map_or(0, |s| s.tick);
                        game.interp
                            .pending
                            .extend(evs.into_iter().map(|e| (tick, e)));
                    }
                    ClientEvent::Roster(r) => game.names = r,
                    // The host left (or the connection was lost): back to the start menu.
                    // (Told twice when the host leaves: its message and the connection's close.)
                    ClientEvent::Disconnected(_) if game.status.starts_with("Disconnected") => {}
                    ClientEvent::Disconnected(why) => {
                        warn!("disconnected from the server: {why}");
                        game.status = format!("Disconnected: {why}");
                        session.write(crate::session::SessionCmd::Leave);
                    }
                    // The next match (the server moved everyone into a new arena).
                    ClientEvent::Welcome { id, seed, rules }
                    | ClientEvent::NewLevel { seed, rules, id } => {
                        if seed != game.level.seed
                            || rules.arena != game.rules.arena
                            || rules.mode != game.level.mode
                        {
                            game.level =
                                Arc::new(Level::generate_for(seed, rules.arena, rules.mode));
                            game.world = Arc::new(game.level.world());
                        }
                        game.rules = rules;
                        game.me = Some(id);
                        game.snap = Snapshot::default();
                        game.prev = Snapshot::default();
                        let pending = std::mem::take(&mut game.interp.pending);
                        game.interp.reset();
                        // Events from the old arena (the match's end) still play.
                        for (_, e) in pending {
                            if !game.shots.echo(&e, id) {
                                events.write(GameEvent(e));
                            }
                        }
                        game.shots = Default::default();
                    }
                }
            }
            if let Some(s) = game.interp.advance(game.me) {
                game.prev = std::mem::replace(&mut game.snap, s);
            }
            // The own bolts' flight against the players as now shown; what they hit goes to
            // the server.
            game.shots.fly(
                &game.snap,
                game.interp.view,
                &game.world,
                &game.level,
                &game.rules,
                &mut own,
            );
            client.send_hits(game.shots.take_claims());
            if let Some(me) = game.me {
                game.shots.show(&mut game.snap, me);
            }
            for e in own {
                events.write(GameEvent(e));
            }
            for e in game.interp.due() {
                if game.me.is_some_and(|me| game.shots.echo(&e, me)) {
                    continue;
                }
                events.write(GameEvent(e));
            }
        }
    }
    // The doors and panels where they are in the tick shown, for the prediction and aiming.
    if !game.level.movers.is_empty() {
        let colliders = game.level.mover_colliders(game.snap.tick);
        Arc::make_mut(&mut game.world).set_dynamic(colliders);
    }
}

/// How the local player was last tagged out (until respawning): by whom (`None`: nobody, e.g.
/// a forcefield) and with what.
#[derive(Resource, Default, Clone, Copy)]
pub struct Recap {
    pub out: bool,
    pub by: Option<PlayerId>,
    pub how: lt_sim::TagHow,
}

fn track_recap(game: Res<Game>, mut recap: ResMut<Recap>, mut events: MessageReader<GameEvent>) {
    for GameEvent(e) in events.read() {
        match *e {
            SimEvent::TaggedOut {
                player, by, how, ..
            } if Some(player) == game.me => {
                *recap = Recap {
                    out: true,
                    by: (by != lt_sim::HAZARD).then_some(by),
                    how,
                };
            }
            SimEvent::Respawned { player, .. } if Some(player) == game.me => recap.out = false,
            _ => {}
        }
    }
    if game.is_changed()
        && game.me.is_some_and(|me| {
            game.snap
                .players
                .iter()
                .any(|p| p.id == me && p.respawn.is_none())
        })
    {
        recap.out = false;
    }
}

/// Movement inertia (time constants, s): getting up to speed, and stopping.
const ACCEL_TIME: f32 = 0.12;
const STOP_TIME: f32 = 0.09;

fn predict(
    game: Res<Game>,
    mut local: ResMut<LocalCmd>,
    mut mover: ResMut<Mover>,
    time: Res<Time>,
    mut events: MessageReader<GameEvent>,
) {
    let walk = std::mem::take(&mut local.walk_frame);
    let Some(me) = game.me else { return };
    let respawned = events
        .read()
        .any(|e| matches!(e.0, SimEvent::Respawned { player, .. } if player == me));
    let Some(p) = game.snap.players.iter().find(|p| p.id == me) else {
        mover.valid = false;
        local.cmd.feet = None;
        return;
    };
    if local.autopilot {
        local.cmd.feet = None;
        mover.feet = p.feet;
        return;
    }
    if !mover.valid
        || mover.seed != game.level.seed
        || p.respawn.is_some()
        || respawned
        || (mover.feet - p.feet).length() > 2.0
    {
        mover.feet = p.feet;
        mover.vy = 0.0;
        mover.air = Vec2::ZERO;
        mover.vel = Vec2::ZERO;
        mover.moves = lt_sim::sim::Moves {
            jumps: Some(local.cmd.jumps),
            dashes: Some(local.cmd.dashes),
            ..default()
        };
        mover.valid = true;
        mover.seed = game.level.seed;
    }
    if p.respawn.is_some() {
        local.cmd.feet = None;
        mover.vel = Vec2::ZERO;
        return;
    }
    let dt = time.delta_secs().min(0.05);
    // (As fast as the sim moves this player.)
    let speed = lt_sim::sim::move_speed(&game.rules, p.speed > 0.0, local.cmd.crouch);
    let want = local.cmd.stick.clamp_length_max(1.0) * speed;
    // Speeding up takes a little longer than stopping.
    let tau = if want.length_squared() >= mover.vel.length_squared() {
        ACCEL_TIME
    } else {
        STOP_TIME
    };
    mover.vel = mover.vel.lerp(want, 1.0 - (-dt / tau).exp());
    if mover.vel.length() < 0.01 && want == Vec2::ZERO {
        mover.vel = Vec2::ZERO;
    }
    let (mut vy, air) = (mover.vy, mover.air);
    if let Some(up) =
        lt_sim::sim::jump_step(&game.rules, &mut mover.moves, local.cmd.jumps, vy, air, dt)
    {
        vy = up;
    }
    let (cmd_stick, yaw) = (local.cmd.stick, local.cmd.yaw);
    let (dash, _) = lt_sim::sim::dash_step(
        &game.rules,
        &mut mover.moves,
        local.cmd.dashes,
        cmd_stick,
        yaw,
        air,
        dt,
    );
    // (Flying from a jump pad: mostly the pad's throw, a little steering; dashing, the dash.)
    let delta = match dash {
        Some(v) => v * dt,
        None => mover.vel * dt * lt_sim::sim::air_control(mover.air),
    } + mover.air * dt
        + walk;
    let (feet, mut vy) = game.world.move_body(mover.feet, delta, vy, dt);
    let flying = mover.air != Vec2::ZERO;
    let mut air = mover.air;
    if let Some((up, throw)) = lt_sim::sim::pad_step(&game.level, feet, vy, &mut air) {
        vy = up;
        air = throw;
    }
    mover.air = air;
    // Walking into a wall stops the movement into it.
    if dt > 0.0 && !flying && dash.is_none() {
        let moved = Vec2::new(feet.x - mover.feet.x, feet.z - mover.feet.z) - walk;
        let along = moved / dt;
        if along.length() < mover.vel.length() * 0.5 {
            mover.vel = mover.vel.lerp(along, 0.5);
        }
    }
    mover.feet = feet;
    mover.vy = vy;
    local.cmd.feet = Some(feet);
}

/// Starting over and leaving (src/session.rs).
impl Game {
    /// A local game on arena `seed` again: the local player again, `bots`
    /// bots of `skill`, `rules`; a listen server moves its remote players over.
    pub fn restart(&mut self, seed: u64, bots: usize, skill: f32, rules: Rules) {
        let name = self
            .me
            .map_or_else(|| "Player".to_string(), |id| self.name_of(id).to_string());
        let Source::Local {
            sim,
            bots: old_bots,
            server,
            ..
        } = &mut self.source
        else {
            return;
        };
        let mut next = Sim::new(seed, rules.clone());
        let me = next.add_player(&name, None, false);
        let mut next_bots = Bots::default();
        next_bots.fill(&mut next, bots, skill);
        next.begin();
        if let Some(server) = server {
            server.move_to(&mut next, &mut next_bots, sim);
        }
        *sim = next;
        *old_bots = next_bots;
        self.level = sim.level.clone();
        self.world = Arc::new(sim.level.world());
        self.rules = rules;
        self.me = me;
        self.snap = sim.snapshot();
        self.prev = self.snap.clone();
        self.names = sim
            .players
            .iter()
            .map(|p| (p.id, p.name.clone(), p.team))
            .collect();
    }

    /// The next match of a local game on arena `seed`, with as many players and the same bots.
    pub fn next_match(&mut self, seed: u64) {
        let Source::Local { sim, bots, .. } = &self.source else {
            return;
        };
        let others = sim.players.len().saturating_sub(1);
        let skill = bots.skill();
        self.restart(seed, others, skill, self.rules.clone());
    }

    /// Stops hosting (telling the players) or leaves the server.
    pub fn close(&mut self) {
        match &mut self.source {
            Source::Local { server, net, .. } => {
                if let Some(mut s) = server.take() {
                    s.shutdown("the host left");
                }
                // Off the LAN and the master's list, the router's port closed.
                *net = None;
            }
            Source::Remote(client) => client.close(),
        }
    }

    /// A hosting game's state (where it can be reached).
    pub fn hosting(&self) -> Option<String> {
        match &self.source {
            Source::Local { net: Some(n), .. } => Some(n.status()),
            _ => None,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.source, Source::Local { .. })
    }
}

/// Interpolation factor between `prev` and `snap` for this frame.
pub fn alpha(time: &Time<Fixed>) -> f32 {
    time.overstep_fraction().clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u64, x: f32) -> Snapshot {
        thread_local!(static PLAYER: PlayerSnap = {
            let mut sim = Sim::new(1, Rules::default());
            sim.add_player("p", Some(Team::Red), false);
            sim.snapshot().players[0].clone()
        });
        let mut p = PLAYER.with(|p| p.clone());
        p.id = 1;
        p.feet = Vec3::new(x, 0.0, 0.0);
        Snapshot {
            tick,
            players: vec![p],
            ..Default::default()
        }
    }

    #[test]
    fn jittery_snapshots_play_back_smoothly() {
        // A player walking at 0.05 m a tick; the server sends every other tick, and the
        // packets arrive bunched up irregularly.
        let mut interp = Interp::default();
        let mut sent = 0u64;
        let mut xs = Vec::new();
        let arrivals = [0usize, 0, 2, 1, 0, 3, 0, 1, 2, 0, 0, 1];
        for t in 0..600u64 {
            // How many snapshots arrive this tick (on average one every two ticks).
            let n = if t % 2 == 0 {
                arrivals[(t as usize / 2) % arrivals.len()]
            } else {
                0
            };
            for _ in 0..n {
                sent += 2;
                interp.receive(snap(sent, sent as f32 * 0.05));
            }
            while sent < t.saturating_sub(8) {
                sent += 2;
                interp.receive(snap(sent, sent as f32 * 0.05));
            }
            if let Some(s) = interp.advance(None) {
                xs.push(s.players[0].feet.x);
            }
        }
        let steps: Vec<f32> = xs.windows(2).skip(100).map(|w| w[1] - w[0]).collect();
        let (lo, hi) = steps
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &s| (a.min(s), b.max(s)));
        assert!(
            lo > 0.03 && hi < 0.07,
            "steady movement, no stalls or jumps: {lo}..{hi}"
        );
        assert!(
            interp.view_tick() + 20 > sent,
            "not too far behind: {} vs {sent}",
            interp.view_tick()
        );
    }
}

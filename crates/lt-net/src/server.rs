//! The server side of a game: remote players joining a `Sim` (in the dedicated server, or a
//! client hosting). Bots make room for people: one leaves the team a player joins, and comes
//! back when the player goes.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use lt_sim::bots::Bots;
use lt_sim::shot::HitClaim;
use lt_sim::sim::Claim;
use lt_sim::{PlayerCmd, PlayerId, Sim, SimEvent, Team};

use lt_master_proto::{ReportPlayer, ServerReport, TEAM_BLUE, TEAM_RED};

use crate::protocol::{PROTOCOL, ToClient, ToServer, decode, encode};
use crate::transport::{Channel, ConnId, Endpoint, NetEvent};

/// Snapshots go out every this many ticks.
pub const SEND_EVERY: u64 = 2;
/// Commands buffered per client at most (beyond: the oldest are dropped).
const QUEUE: usize = 4;
/// Ticks over which the least queued is watched: commands that stay queued all that time only
/// add delay, so one is dropped.
const QUEUE_WINDOW: u32 = 60;
/// Ticks a claim on a shot not fired yet waits for its command.
const CLAIM_WAIT: u32 = 30;

struct Remote {
    player: Option<PlayerId>,
    name: String,
    last_seq: u32,
    queue: VecDeque<PlayerCmd>,
    last: PlayerCmd,
    /// The least commands queued over the current `QUEUE_WINDOW`, and the ticks into it.
    least: usize,
    window: u32,
    /// Hit claims waiting for their shot, with ticks waited.
    claims: Vec<(HitClaim, u32)>,
}

impl Remote {
    fn new() -> Remote {
        Remote {
            player: None,
            name: String::new(),
            last_seq: 0,
            queue: VecDeque::new(),
            last: PlayerCmd::default(),
            least: usize::MAX,
            window: 0,
            claims: Vec::new(),
        }
    }

    /// Drops the oldest queued command, keeping its one-shots (a throw, a trigger pull, the
    /// shot count) in the next.
    fn drop_oldest(&mut self) {
        let Some(old) = self.queue.pop_front() else {
            return;
        };
        if let Some(next) = self.queue.front_mut() {
            next.throw = next.throw.or(old.throw);
            // (A pull held for a tick only would be lost.)
            if old.fire && !self.last.fire {
                next.fire = true;
            }
            next.shot = next.shot.max(old.shot);
        }
    }
}

pub struct Server {
    pub name: String,
    pub seed: u64,
    endpoint: Endpoint,
    remotes: HashMap<ConnId, Remote>,
    /// Teams whose bot gave up its place to a remote player, per player.
    displaced: HashMap<PlayerId, Team>,
    pub log: Vec<String>,
}

impl Server {
    pub fn listen(port: u16, name: &str, seed: u64) -> std::io::Result<Server> {
        Ok(Server {
            name: name.to_string(),
            seed,
            endpoint: Endpoint::server(&format!("0.0.0.0:{port}"), lt_sim::sim::MAX_PLAYERS)?,
            remotes: HashMap::new(),
            displaced: HashMap::new(),
            log: Vec::new(),
        })
    }

    pub fn port(&self) -> Option<u16> {
        self.endpoint.local_addr().map(|a| a.port())
    }

    pub fn humans(&self) -> usize {
        self.remotes.values().filter(|r| r.player.is_some()).count()
    }

    fn send(&mut self, id: ConnId, channel: Channel, msg: &ToClient) {
        self.endpoint.send(id, channel, &encode(msg));
    }

    fn broadcast(&mut self, channel: Channel, msg: &ToClient) {
        let bytes = encode(msg);
        let ids: Vec<ConnId> = self
            .remotes
            .iter()
            .filter(|(_, r)| r.player.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.endpoint.send(id, channel, &bytes);
        }
    }

    /// Takes in connections and inputs; sets the remote players' commands for the next step.
    pub fn before_step(&mut self, sim: &mut Sim, bots: &mut Bots) {
        let now = Instant::now();
        for ev in self.endpoint.poll(now) {
            match ev {
                NetEvent::Connected(id) => {
                    self.remotes.insert(id, Remote::new());
                }
                NetEvent::Disconnected(id, why) => self.drop_remote(id, &why, sim, bots),
                NetEvent::Message(id, _, bytes) => {
                    let Some(msg) = decode::<ToServer>(&bytes) else {
                        continue;
                    };
                    self.message(id, msg, sim, bots);
                }
            }
        }
        for r in self.remotes.values_mut() {
            let Some(p) = r.player else { continue };
            // A queue that never runs dry is only delay: one less.
            r.least = r.least.min(r.queue.len());
            r.window += 1;
            if r.window >= QUEUE_WINDOW {
                if r.least > 1 && r.least != usize::MAX {
                    r.drop_oldest();
                }
                r.least = usize::MAX;
                r.window = 0;
            }
            if let Some(cmd) = r.queue.pop_front() {
                r.last = cmd;
            } else {
                // Nothing new: keep going as before, but don't repeat one-shots.
                r.last.throw = None;
            }
            sim.set_cmd(p, r.last);
            // Hit claims (the shots they're about may come with this command or later).
            let claims = std::mem::take(&mut r.claims);
            for (c, waited) in claims {
                match sim.claim_hit(p, c) {
                    Claim::Early if waited < CLAIM_WAIT => r.claims.push((c, waited + 1)),
                    Claim::Accepted => {}
                    other => {
                        self.log.push(format!("{}: hit claim {other:?}", r.name));
                    }
                }
            }
        }
    }

    /// A remote player gone: off the field, and the bot they displaced back.
    fn drop_remote(&mut self, id: ConnId, why: &str, sim: &mut Sim, bots: &mut Bots) {
        if let Some(r) = self.remotes.remove(&id)
            && let Some(p) = r.player
        {
            self.log.push(format!("{} left ({why})", r.name));
            sim.remove_player(p);
            if let Some(team) = self.displaced.remove(&p) {
                bots.fill_team(sim, team, bots.skill());
            }
        }
    }

    fn message(&mut self, id: ConnId, msg: ToServer, sim: &mut Sim, bots: &mut Bots) {
        match msg {
            ToServer::Hello { protocol, name } => {
                if protocol != PROTOCOL {
                    self.send(
                        id,
                        Channel::Reliable,
                        &ToClient::Reject(format!(
                            "Protocol {protocol}, the server speaks {PROTOCOL}"
                        )),
                    );
                    self.endpoint.close(id, "protocol");
                    return;
                }
                if self.remotes.get(&id).is_some_and(|r| r.player.is_some()) {
                    return;
                }
                let name: String = name.chars().filter(|c| !c.is_control()).take(24).collect();
                let name = if name.trim().is_empty() {
                    "Player".to_string()
                } else {
                    name
                };
                // Make room: a bot of the smaller team leaves.
                let team = smaller_team(sim);
                let freed = bots.remove_one(sim, team);
                match sim.add_player(&name, Some(team), false) {
                    Some(p) => {
                        if freed {
                            self.displaced.insert(p, team);
                        }
                        if let Some(r) = self.remotes.get_mut(&id) {
                            r.player = Some(p);
                            r.name = name.clone();
                        }
                        set_client_hits(sim, p);
                        self.log.push(format!("{name} joined {team:?}"));
                        let welcome = ToClient::Welcome {
                            id: p,
                            seed: sim.level.seed,
                            rules: sim.rules.clone(),
                        };
                        self.send(id, Channel::Reliable, &welcome);
                    }
                    None => {
                        if freed {
                            bots.fill_team(sim, team, bots.skill());
                        }
                        self.send(
                            id,
                            Channel::Reliable,
                            &ToClient::Reject("The server is full".into()),
                        );
                        self.endpoint.close(id, "full");
                    }
                }
            }
            ToServer::Input { recent } => {
                let Some(r) = self.remotes.get_mut(&id) else {
                    return;
                };
                for (seq, cmd) in recent {
                    if seq > r.last_seq {
                        r.last_seq = seq;
                        r.queue.push_back(cmd);
                    }
                }
                while r.queue.len() > QUEUE {
                    // Behind: drop the oldest, keeping its one-shots.
                    r.drop_oldest();
                }
            }
            ToServer::Hits(hits) => {
                let Some(r) = self.remotes.get_mut(&id) else {
                    return;
                };
                // (Tried when the commands are taken in, before the next step.)
                r.claims.extend(hits.into_iter().take(64).map(|c| (c, 0)));
                r.claims.truncate(256);
            }
            ToServer::SwitchTeam(team) => {
                let Some(r) = self.remotes.get(&id) else {
                    return;
                };
                let Some(p) = r.player else { return };
                if !sim.rules.team_switch {
                    return;
                }
                if sim.switch_team(p, team) {
                    let moved = bots.balance(sim);
                    self.log.push(format!(
                        "{} switched to {team:?} ({moved} bots moved)",
                        r.name
                    ));
                }
            }
            // (Closing this end tells nobody here: the player goes now.)
            ToServer::Bye => {
                self.drop_remote(id, "quit", sim, bots);
                self.endpoint.close(id, "Bye");
            }
        }
    }

    /// Sends what happened and the new state.
    pub fn after_step(&mut self, sim: &Sim, events: &[SimEvent]) {
        if !events.is_empty() {
            self.broadcast(Channel::Reliable, &ToClient::Events(events.to_vec()));
        }
        if sim.tick.is_multiple_of(SEND_EVERY) {
            self.broadcast(Channel::Unreliable, &ToClient::Snapshot(sim.snapshot()));
        }
        if sim.tick % 60 == 1 {
            let roster = sim
                .players
                .iter()
                .map(|p| (p.id, p.name.clone(), p.team))
                .collect();
            self.broadcast(Channel::Reliable, &ToClient::Roster(roster));
        }
        self.endpoint.flush(Instant::now());
    }

    /// Moves everyone into the next match (`sim`, a new arena): each remote player rejoins on
    /// the same team, taking a bot's place, and is welcomed again.
    pub fn move_to(&mut self, sim: &mut Sim, bots: &mut Bots, old: &Sim) {
        self.displaced.clear();
        let ids: Vec<ConnId> = self.remotes.keys().copied().collect();
        for id in ids {
            let Some(r) = self.remotes.get(&id) else {
                continue;
            };
            let Some(old_id) = r.player else { continue };
            let team = old
                .player(old_id)
                .map(|p| p.team)
                .unwrap_or_else(|| smaller_team(sim));
            let name = r.name.clone();
            let freed = bots.remove_one(sim, team);
            let new_id = sim.add_player(&name, Some(team), false);
            if let Some(p) = new_id
                && freed
            {
                self.displaced.insert(p, team);
            }
            if let Some(p) = new_id {
                set_client_hits(sim, p);
            }
            if let Some(r) = self.remotes.get_mut(&id) {
                r.player = new_id;
                r.queue.clear();
                r.claims.clear();
                r.last = PlayerCmd::default();
            }
            if let Some(p) = new_id {
                let welcome = ToClient::Welcome {
                    id: p,
                    seed: sim.level.seed,
                    rules: sim.rules.clone(),
                };
                self.send(id, Channel::Reliable, &welcome);
            }
        }
        self.endpoint.flush(Instant::now());
    }

    /// Tells everyone the game is over and closes their connections (there's no later flush
    /// to resend anything, so the close goes out too).
    pub fn shutdown(&mut self, why: &str) {
        self.broadcast(Channel::Reliable, &ToClient::Shutdown(why.to_string()));
        self.endpoint.flush(Instant::now());
        for id in self.remotes.keys().copied().collect::<Vec<_>>() {
            self.endpoint.close(id, why);
        }
        self.endpoint.flush(Instant::now());
        self.remotes.clear();
    }

    /// The server as the master server's list and LAN beacons show it: joined on `port`
    /// (outside a router: the port it opened), playing `sim`.
    pub fn report(&self, sim: &Sim, port: u16, dedicated: bool) -> ServerReport {
        ServerReport {
            protocol: PROTOCOL,
            game_version: env!("CARGO_PKG_VERSION").into(),
            name: self.name.clone(),
            port,
            dedicated,
            map_seed: sim.level.seed,
            mode: sim.rules.mode.name().into(),
            arena: sim.rules.arena.name().into(),
            players: sim
                .players
                .iter()
                .filter(|p| !p.bot)
                .map(|p| ReportPlayer {
                    name: p.name.clone(),
                    team: if p.team == Team::Red {
                        TEAM_RED.into()
                    } else {
                        TEAM_BLUE.into()
                    },
                })
                .collect(),
            max_players: lt_sim::sim::MAX_PLAYERS as u16,
            bots: sim.players.iter().filter(|p| p.bot).count() as u16,
            scores: sim.scores,
            time_left_s: sim.time_left as u32,
        }
    }

    /// The remote players' sim ids.
    pub fn players(&self) -> Vec<PlayerId> {
        self.remotes.values().filter_map(|r| r.player).collect()
    }
}

/// A remote player's client judges its hits on others.
fn set_client_hits(sim: &mut Sim, p: PlayerId) {
    if let Some(pl) = sim.players.iter_mut().find(|q| q.id == p) {
        pl.client_hits = true;
    }
}

fn smaller_team(sim: &Sim) -> Team {
    let humans = |t: Team| sim.players.iter().filter(|p| p.team == t && !p.bot).count();
    let all = |t: Team| sim.players.iter().filter(|p| p.team == t).count();
    let (r, b) = (humans(Team::Red), humans(Team::Blue));
    if r != b {
        return if r < b { Team::Red } else { Team::Blue };
    }
    if all(Team::Red) <= all(Team::Blue) {
        Team::Red
    } else {
        Team::Blue
    }
}

//! The server side of a game: remote players joining a `Sim` (in the dedicated server, or a
//! client hosting). Bots make room for people: one leaves the team a player joins, and comes
//! back when the player goes.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use lt_sim::bots::Bots;
use lt_sim::{PlayerCmd, PlayerId, Sim, SimEvent, Team};

use crate::protocol::{PROTOCOL, ToClient, ToServer, decode, encode};
use crate::transport::{Channel, ConnId, Endpoint, NetEvent};

/// Snapshots go out every this many ticks.
pub const SEND_EVERY: u64 = 2;
/// Commands buffered per client at most (beyond: the oldest are dropped).
const QUEUE: usize = 4;

struct Remote {
    player: Option<PlayerId>,
    name: String,
    last_seq: u32,
    queue: VecDeque<PlayerCmd>,
    last: PlayerCmd,
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
                    self.remotes.insert(
                        id,
                        Remote {
                            player: None,
                            name: String::new(),
                            last_seq: 0,
                            queue: VecDeque::new(),
                            last: PlayerCmd::default(),
                        },
                    );
                }
                NetEvent::Disconnected(id, why) => {
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
            if let Some(cmd) = r.queue.pop_front() {
                r.last = cmd;
            } else {
                // Nothing new: keep going as before, but don't repeat one-shots.
                r.last.throw = None;
            }
            sim.set_cmd(p, r.last);
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
                    let old = r.queue.pop_front().unwrap();
                    if let Some(next) = r.queue.front_mut() {
                        next.throw = next.throw.or(old.throw);
                    }
                }
            }
            ToServer::Bye => self.endpoint.close(id, "Bye"),
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
            if let Some(r) = self.remotes.get_mut(&id) {
                r.player = new_id;
                r.queue.clear();
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

    pub fn shutdown(&mut self, why: &str) {
        self.broadcast(Channel::Reliable, &ToClient::Shutdown(why.to_string()));
        self.endpoint.flush(Instant::now());
    }

    /// The remote players' sim ids.
    pub fn players(&self) -> Vec<PlayerId> {
        self.remotes.values().filter_map(|r| r.player).collect()
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

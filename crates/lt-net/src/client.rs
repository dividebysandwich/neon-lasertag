//! The client side: a connection to a server, sending this player's commands and receiving
//! the world.

use std::time::Instant;

use lt_sim::shot::HitClaim;
use lt_sim::sim::Snapshot;
use lt_sim::{PlayerCmd, PlayerId, Rules, SimEvent, Team};

use crate::protocol::{PROTOCOL, RECENT, ToClient, ToServer, decode, encode};
use crate::transport::{Channel, Endpoint, NetEvent};

#[derive(Clone, Debug)]
pub enum ClientEvent {
    Welcome {
        id: PlayerId,
        seed: u64,
        rules: Rules,
    },
    Snapshot(Snapshot),
    Events(Vec<SimEvent>),
    Roster(Vec<(PlayerId, String, Team)>),
    NewLevel {
        id: PlayerId,
        seed: u64,
        rules: Rules,
    },
    Disconnected(String),
}

pub struct Client {
    endpoint: Endpoint,
    name: String,
    seq: u32,
    recent: Vec<(u32, PlayerCmd)>,
    last_tick: u64,
    pub connected: bool,
}

impl Client {
    /// Starts connecting to `addr` ("host:port"; the default port if none is given).
    pub fn connect(addr: &str, name: &str) -> std::io::Result<Client> {
        let addr = if addr.contains(':') {
            addr.to_string()
        } else {
            format!("{addr}:{}", crate::protocol::DEFAULT_PORT)
        };
        Ok(Client {
            endpoint: Endpoint::client(&addr)?,
            name: name.to_string(),
            seq: 0,
            recent: Vec::new(),
            last_tick: 0,
            connected: false,
        })
    }

    pub fn rtt(&self) -> f32 {
        self.endpoint.rtt(0)
    }

    /// This tick's command.
    pub fn send_input(&mut self, cmd: PlayerCmd) {
        if !self.connected {
            return;
        }
        self.seq += 1;
        self.recent.push((self.seq, cmd));
        if self.recent.len() > RECENT {
            self.recent.remove(0);
        }
        let msg = ToServer::Input {
            recent: self.recent.clone(),
        };
        self.endpoint.send(0, Channel::Unreliable, &encode(&msg));
        self.endpoint.flush(Instant::now());
    }

    /// What this player's bolts hit (`lt_sim::shot`: the server checks and applies them).
    pub fn send_hits(&mut self, hits: Vec<HitClaim>) {
        if !self.connected || hits.is_empty() {
            return;
        }
        self.endpoint
            .send(0, Channel::Reliable, &encode(&ToServer::Hits(hits)));
        self.endpoint.flush(Instant::now());
    }

    pub fn poll(&mut self) -> Vec<ClientEvent> {
        let now = Instant::now();
        let mut out = Vec::new();
        for ev in self.endpoint.poll(now) {
            match ev {
                NetEvent::Connected(_) => {
                    self.connected = true;
                    let hello = ToServer::Hello {
                        protocol: PROTOCOL,
                        name: self.name.clone(),
                    };
                    self.endpoint.send(0, Channel::Reliable, &encode(&hello));
                }
                NetEvent::Disconnected(_, why) => {
                    self.connected = false;
                    out.push(ClientEvent::Disconnected(why));
                }
                NetEvent::Message(_, _, bytes) => match decode::<ToClient>(&bytes) {
                    Some(ToClient::Welcome { id, seed, rules }) => {
                        // Welcomed again: the next match, ticks start over.
                        self.last_tick = 0;
                        out.push(ClientEvent::Welcome { id, seed, rules });
                    }
                    Some(ToClient::Snapshot(s)) => {
                        // Late arrivals are dropped.
                        if s.tick > self.last_tick {
                            self.last_tick = s.tick;
                            out.push(ClientEvent::Snapshot(s));
                        }
                    }
                    Some(ToClient::Events(e)) => out.push(ClientEvent::Events(e)),
                    Some(ToClient::Roster(r)) => out.push(ClientEvent::Roster(r)),
                    Some(ToClient::NewLevel { id, seed, rules }) => {
                        self.last_tick = 0;
                        out.push(ClientEvent::NewLevel { id, seed, rules });
                    }
                    Some(ToClient::Reject(why)) | Some(ToClient::Shutdown(why)) => {
                        out.push(ClientEvent::Disconnected(why));
                    }
                    None => {}
                },
            }
        }
        self.endpoint.flush(now);
        out
    }

    /// Leaves: says so, then closes the connection (both go out now: there may be no later
    /// flush).
    pub fn close(&mut self) {
        self.endpoint
            .send(0, Channel::Reliable, &encode(&ToServer::Bye));
        self.endpoint.flush(Instant::now());
        self.endpoint.close(0, "Bye");
        self.endpoint.flush(Instant::now());
        self.connected = false;
    }
}

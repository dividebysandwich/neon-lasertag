//! The server list's REST API (JSON over HTTP(S)), shared by the master server (`lt-master`),
//! the game servers that report to it and the games that list them (`lt-net::master`).
//!
//! * `POST /api/servers` with a [`ServerReport`]: a game server is there (again); the answer
//!   is a [`Registered`]. The server's address is where the request came from, with the
//!   report's UDP port.
//! * `DELETE /api/servers/<port>`: the game server at the request's address goes.
//! * `GET /api/servers`: the servers there now, as [`ServerEntry`]s (most players first).
//! * `GET /api/server/<id>`: one of them by its [`ServerEntry::id`].
//! * `GET /`: the same as a plain web page.

use serde::{Deserialize, Serialize};

/// The master server the game reports to and lists from (placeholder host until deployed).
pub const DEFAULT_MASTER: &str = "https://lt-master.playspoon.com";
/// The master server's default listen address.
pub const DEFAULT_BIND: &str = "0.0.0.0:27500";
/// Seconds between a game server's reports, and after which one not heard of is gone.
pub const REPORT_EVERY: u64 = 30;
pub const EXPIRE_AFTER: u64 = 90;
/// Limits on what a report may carry.
pub const MAX_TEXT: usize = 64;
pub const MAX_PLAYERS: usize = 64;

/// `POST` a report here, `GET` the list here; `DELETE` `{API_SERVERS}/<port>`.
pub const API_SERVERS: &str = "/api/servers";
/// `GET` `{API_SERVER}<id>`: one server.
pub const API_SERVER: &str = "/api/server/";

/// The two teams' names as reported ([`ReportPlayer::team`]); index 0 and 1 of
/// [`ServerReport::scores`].
pub const TEAM_RED: &str = "red";
pub const TEAM_BLUE: &str = "blue";

/// A game server's state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerReport {
    /// The network protocol (a game joins only its own).
    pub protocol: u16,
    /// The game's version ("0.3.0").
    pub game_version: String,
    pub name: String,
    /// UDP port the players join on.
    pub port: u16,
    /// A dedicated server (else a player's game hosts).
    pub dedicated: bool,
    /// The arena's generator seed.
    pub map_seed: u64,
    /// What is played ("classic", "megatarget") and the arena's size ("small", ...).
    pub mode: String,
    pub arena: String,
    /// Human players (bots are only counted, in `bots`).
    pub players: Vec<ReportPlayer>,
    pub max_players: u16,
    pub bots: u16,
    /// Red's and blue's scores.
    pub scores: [u32; 2],
    /// Seconds left in the round.
    pub time_left_s: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportPlayer {
    pub name: String,
    /// [`TEAM_RED`] or [`TEAM_BLUE`].
    pub team: String,
}

/// The master server's answer to a report: when to report again (seconds).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Registered {
    pub every: u64,
}

/// A listed server: its address ("ip:port", to join) and its last report, seconds ago.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerEntry {
    /// From its name and address (so it stays while the server does): [`server_id`].
    pub id: String,
    pub addr: String,
    pub age: u64,
    pub report: ServerReport,
}

impl ServerReport {
    /// Cut to the limits (a master server takes nothing longer).
    pub fn trimmed(mut self) -> Self {
        let cut = |s: &mut String| {
            if let Some((i, _)) = s.char_indices().nth(MAX_TEXT) {
                s.truncate(i);
            }
        };
        cut(&mut self.game_version);
        cut(&mut self.name);
        cut(&mut self.mode);
        cut(&mut self.arena);
        self.players.truncate(MAX_PLAYERS);
        for p in &mut self.players {
            cut(&mut p.name);
            if p.team != TEAM_RED && p.team != TEAM_BLUE {
                p.team.clear();
            }
        }
        self
    }
}

/// A server's id: FNV-1a (64 bit) of its name and IP address, in hex.
pub fn server_id(name: &str, ip: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes().chain([0]).chain(ip.bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_differ() {
        let a = server_id("Arena", "203.0.113.5");
        assert_eq!(a.len(), 16);
        assert_eq!(a, server_id("Arena", "203.0.113.5"));
        assert_ne!(a, server_id("Arena", "203.0.113.6"));
        // The separator keeps "ab"+"c" apart from "a"+"bc".
        assert_ne!(server_id("ab", "c"), server_id("a", "bc"));
    }

    #[test]
    fn reports_are_trimmed_and_tolerate_missing_fields() {
        let r = ServerReport {
            name: "x".repeat(200),
            players: (0..100)
                .map(|k| ReportPlayer {
                    name: format!("P{k}"),
                    team: "green".into(),
                })
                .collect(),
            ..Default::default()
        }
        .trimmed();
        assert_eq!(r.name.chars().count(), MAX_TEXT);
        assert_eq!(r.players.len(), MAX_PLAYERS);
        assert!(r.players[0].team.is_empty());
        let r: ServerReport = serde_json::from_str(r#"{"name":"A","port":27515}"#).unwrap();
        assert_eq!((r.name.as_str(), r.port, r.scores), ("A", 27515, [0, 0]));
    }
}

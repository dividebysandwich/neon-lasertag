//! The game's messages, CBOR encoded (LZ4 compressed when large), as in stealth-net.

use lt_sim::sim::Snapshot;
use lt_sim::{PlayerCmd, PlayerId, Rules, SimEvent, Team};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const PROTOCOL: u16 = 4;
pub const DEFAULT_PORT: u16 = 27420;
/// How many of its latest commands a client sends with each input (a lost packet loses
/// nothing).
pub const RECENT: usize = 6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    Hello {
        protocol: u16,
        name: String,
    },
    /// The client's latest commands, oldest first, by sequence number (one per tick).
    Input {
        recent: Vec<(u32, PlayerCmd)>,
    },
    Bye,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToClient {
    Welcome {
        id: PlayerId,
        seed: u64,
        rules: Rules,
    },
    Reject(String),
    /// The world (unreliable, every other tick).
    Snapshot(Snapshot),
    /// What happened (reliable).
    Events(Vec<SimEvent>),
    Roster(Vec<(PlayerId, String, Team)>),
    /// The next match, in a new arena.
    NewLevel {
        id: PlayerId,
        seed: u64,
        rules: Rules,
    },
    Shutdown(String),
}

/// A message's bytes: a flag (1 compressed) and CBOR.
pub fn encode<T: Serialize>(msg: &T) -> Vec<u8> {
    let mut cbor = Vec::new();
    if ciborium::into_writer(msg, &mut cbor).is_err() {
        return Vec::new();
    }
    if cbor.len() > 200 {
        let mut out = vec![1];
        out.extend(lz4_flex::compress_prepend_size(&cbor));
        out
    } else {
        let mut out = vec![0];
        out.extend(cbor);
        out
    }
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
    let (&flag, body) = bytes.split_first()?;
    match flag {
        0 => ciborium::from_reader(body).ok(),
        1 => {
            let raw = lz4_flex::decompress_size_prepended(body).ok()?;
            ciborium::from_reader(raw.as_slice()).ok()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut sim = lt_sim::Sim::new(1, Rules::default());
        let mut bots = lt_sim::bots::Bots::default();
        bots.fill(&mut sim, 8, 0.5);
        for _ in 0..300 {
            bots.think(&mut sim);
            sim.step();
        }
        let msg = ToClient::Snapshot(sim.snapshot());
        let bytes = encode(&msg);
        assert!(bytes.len() < 4000, "{} bytes", bytes.len());
        assert_eq!(decode::<ToClient>(&bytes), Some(msg));
    }
}

//! The lasertag game itself, without a renderer: rules, arena generation, collision, bolts,
//! grenades and the bot interface. Run by the client (single player, listen host) and by the
//! dedicated server (crates/lt-server).

pub mod bots;
pub mod collide;
pub mod hitbox;
pub mod kit;
pub mod level;
pub mod rules;
pub mod sim;
pub mod types;

pub use rules::Rules;
pub use sim::{Sim, Snapshot, TICK};
pub use types::*;

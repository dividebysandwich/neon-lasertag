//! Lasertag networking.
//!
//! * [`transport`]: connections on UDP with a reliable and an unreliable channel.
//! * [`master`]: the public server list (reporting to it, fetching it).
//! * [`lan`]: finding games on the local network by broadcast.
//! * [`portmap`]: opening a hosting game's port on the router (UPnP, NAT-PMP).
//! * `websocket`: the browser build's datagrams, through the master server's relay.
//!
//! The browser build (wasm32) has no UDP: a client joins through the master server's
//! WebSocket relay ([`client::Client::relayed`]); hosting, reporting and port mapping are
//! native only, and the LAN browser can't listen there.
//!
//! The game's messages, server and client sessions build on these.

pub mod client;
pub mod lan;
pub mod master;
#[cfg(not(target_arch = "wasm32"))]
pub mod portmap;
pub mod protocol;
pub mod server;
pub mod transport;
#[cfg(target_arch = "wasm32")]
pub mod websocket;

pub use lan::{LAN_PORT, LanAnnouncer, LanBrowser, LanServer};
pub use lt_master_proto as master_proto;
pub use lt_master_proto::{
    DEFAULT_MASTER, REPORT_EVERY, ReportPlayer, ServerEntry, ServerReport, TEAM_BLUE, TEAM_RED,
};
pub use master::{ListSlot, fetch_list_into, relay_url};
#[cfg(not(target_arch = "wasm32"))]
pub use master::{Reporter, fetch_list};
#[cfg(not(target_arch = "wasm32"))]
pub use portmap::{PortMapper, PortStatus};
pub use transport::{Channel, ConnId, Datagrams, Endpoint, NetEvent};

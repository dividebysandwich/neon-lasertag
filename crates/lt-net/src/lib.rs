//! Lasertag networking.
//!
//! * [`transport`]: connections on UDP with a reliable and an unreliable channel.
//! * [`master`]: the public server list (reporting to it, fetching it).
//! * [`lan`]: finding games on the local network by broadcast.
//! * [`portmap`]: opening a hosting game's port on the router (UPnP, NAT-PMP).
//!
//! The game's messages, server and client sessions build on these.

pub mod client;
pub mod lan;
pub mod master;
pub mod portmap;
pub mod protocol;
pub mod server;
pub mod transport;

pub use lan::{LAN_PORT, LanAnnouncer, LanBrowser, LanServer};
pub use lt_master_proto as master_proto;
pub use lt_master_proto::{
    DEFAULT_MASTER, REPORT_EVERY, ReportPlayer, ServerEntry, ServerReport, TEAM_BLUE, TEAM_RED,
};
pub use master::{ListSlot, Reporter, fetch_list, fetch_list_into};
pub use portmap::{PortMapper, PortStatus};
pub use transport::{Channel, ConnId, Datagrams, Endpoint, NetEvent};

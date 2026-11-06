//! Finding games on the local network: a hosting game (or a dedicated server) broadcasts a
//! beacon every second ([`LanAnnouncer`]) on UDP port [`LAN_PORT`], and the start menu's LAN
//! screen listens for them ([`LanBrowser`]).
//!
//! A beacon is `MAGIC` and a [`Beacon`] (`protocol::encode`d): the server's
//! [`ServerReport`] (the same as the master server's list shows), an id that stays while the
//! server runs, and whether it is going away. It goes to the limited broadcast address, to
//! every IPv4 interface's broadcast address and to 127.0.0.1 (a game on the same machine with
//! no network).

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use lt_master_proto::ServerReport;
use serde::{Deserialize, Serialize};

use crate::protocol::{PROTOCOL, decode, encode};

/// The port beacons go to.
pub const LAN_PORT: u16 = 27421;
const MAGIC: &[u8; 6] = b"LTLAN1";
/// Seconds between beacons.
const EVERY: Duration = Duration::from_secs(1);
/// A server not heard from for this long is gone.
const EXPIRE: Duration = Duration::from_secs(4);
/// How often the interfaces' broadcast addresses are looked up again.
const TARGETS_EVERY: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Beacon {
    instance: u64,
    gone: bool,
    report: ServerReport,
}

enum Msg {
    Report(Box<ServerReport>),
    Stop,
}

/// Broadcasts a server's beacon every second (once it has a report), on a thread of its own;
/// says it's gone when stopped or dropped.
pub struct LanAnnouncer {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl LanAnnouncer {
    /// Beacons to [`LAN_PORT`].
    pub fn new() -> Self {
        Self::to_port(LAN_PORT)
    }

    /// Beacons to `port` (tests).
    pub fn to_port(port: u16) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("lan-announcer".into())
            .spawn(move || announce(port, &rx))
            .ok();
        Self { tx, thread }
    }

    /// The server as it is now (in the next beacon).
    pub fn update(&self, report: ServerReport) {
        let _ = self.tx.send(Msg::Report(Box::new(report)));
    }

    pub fn stop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Default for LanAnnouncer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LanAnnouncer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where beacons go: the limited broadcast, each interface's broadcast, the loopback.
fn targets(port: u16) -> Vec<SocketAddr> {
    let mut out = vec![
        SocketAddr::from((Ipv4Addr::BROADCAST, port)),
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
    ];
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        for i in ifs {
            if let if_addrs::IfAddr::V4(v4) = i.addr
                && !v4.ip.is_loopback()
                && let Some(b) = v4.broadcast
            {
                let a = SocketAddr::from((b, port));
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
    }
    out
}

fn announce(port: u16, rx: &Receiver<Msg>) {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
        return;
    };
    let _ = socket.set_broadcast(true);
    let instance = rand::random::<u64>();
    let mut latest: Option<ServerReport> = None;
    let mut to = targets(port);
    let mut looked = Instant::now();
    let mut next = Instant::now();
    let send = |to: &[SocketAddr], r: &ServerReport, gone: bool| {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(encode(&Beacon {
            instance,
            gone,
            report: r.clone(),
        }));
        for a in to {
            let _ = socket.send_to(&bytes, a);
        }
    };
    loop {
        match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(Msg::Report(r)) => {
                let first = latest.is_none();
                latest = Some(*r);
                if !first {
                    continue;
                }
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() < next {
            continue;
        }
        next = Instant::now() + EVERY;
        if looked.elapsed() >= TARGETS_EVERY {
            to = targets(port);
            looked = Instant::now();
        }
        if let Some(r) = &latest {
            send(&to, r, false);
        }
    }
    if let Some(r) = &latest {
        send(&to, r, true);
    }
}

/// A server found on the network.
#[derive(Clone, Debug, PartialEq)]
pub struct LanServer {
    /// Where to join it ("ip:port").
    pub addr: String,
    pub report: ServerReport,
    seen: Instant,
}

impl LanServer {
    /// It speaks this game's protocol.
    pub fn joinable(&self) -> bool {
        self.report.protocol == PROTOCOL
    }
}

/// Listens for beacons (non-blocking: [`poll`](Self::poll) it every frame).
pub struct LanBrowser {
    socket: UdpSocket,
    servers: HashMap<u64, LanServer>,
    buf: Vec<u8>,
}

impl LanBrowser {
    /// Listens on [`LAN_PORT`] (shared with other games on this machine).
    pub fn new() -> io::Result<Self> {
        Self::on_port(LAN_PORT)
    }

    /// Listens on `port` (tests).
    pub fn on_port(port: u16) -> io::Result<Self> {
        use socket2::{Domain, Protocol, Socket, Type};
        let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        // Several games on one machine all hear the broadcasts.
        s.set_reuse_address(true)?;
        #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
        s.set_reuse_port(true)?;
        s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
        s.set_nonblocking(true)?;
        Ok(Self {
            socket: s.into(),
            servers: HashMap::new(),
            buf: vec![0; 64 * 1024],
        })
    }

    /// Takes in the beacons that came; true if the list changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let now = Instant::now();
        while let Ok((n, from)) = self.socket.recv_from(&mut self.buf) {
            let Some(body) = self.buf[..n].strip_prefix(MAGIC) else {
                continue;
            };
            let Some(b) = decode::<Beacon>(body) else {
                continue;
            };
            if b.gone {
                changed |= self.servers.remove(&b.instance).is_some();
                continue;
            }
            let ip = match from.ip() {
                IpAddr::V4(v4) => v4,
                IpAddr::V6(_) => continue,
            };
            let addr = SocketAddr::from((ip, b.report.port)).to_string();
            match self.servers.get_mut(&b.instance) {
                Some(s) => {
                    s.seen = now;
                    // Another machine can't use the loopback address: keep the network's.
                    if !ip.is_loopback() && s.addr != addr {
                        s.addr = addr;
                        changed = true;
                    }
                    if s.report != b.report {
                        s.report = b.report;
                        changed = true;
                    }
                }
                None => {
                    self.servers.insert(
                        b.instance,
                        LanServer {
                            addr,
                            report: b.report,
                            seen: now,
                        },
                    );
                    changed = true;
                }
            }
        }
        let before = self.servers.len();
        self.servers.retain(|_, s| now - s.seen < EXPIRE);
        changed | (self.servers.len() != before)
    }

    /// The servers heard from, by name.
    pub fn servers(&self) -> Vec<LanServer> {
        let mut v: Vec<LanServer> = self.servers.values().cloned().collect();
        v.sort_by(|a, b| a.report.name.cmp(&b.report.name).then(a.addr.cmp(&b.addr)));
        v
    }
}

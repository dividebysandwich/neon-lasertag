//! Opening a hosting game's UDP port on the home router, so players on the internet reach it:
//! UPnP IGD first (`igd-next`), NAT-PMP if there is none (`natpmp`). On a thread of its own;
//! the mapping is renewed while it lives and removed when it's dropped.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use igd_next::{PortMappingProtocol, SearchOptions};

/// Seconds a mapping is asked for (renewed at half that).
const LEASE: u32 = 3600;
const DESCRIPTION: &str = "Neon Lasertag";

/// How the port is reached from outside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortStatus {
    /// Still asking the router.
    Trying,
    /// Open: `via` "UPnP" or "NAT-PMP", reachable at `external`.
    Open {
        via: &'static str,
        external: SocketAddrV4,
    },
    /// The router wouldn't (or there's no router that speaks either).
    Failed(String),
}

enum Mapping {
    Upnp(Box<igd_next::Gateway>, u16),
    NatPmp,
}

/// Keeps a UDP port open on the router (see the module documentation).
pub struct PortMapper {
    status: Arc<Mutex<PortStatus>>,
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl PortMapper {
    /// Starts opening UDP `port` (the same port outside if the router allows).
    pub fn open(port: u16) -> Self {
        let status = Arc::new(Mutex::new(PortStatus::Trying));
        let (stop, rx) = mpsc::channel();
        let s = status.clone();
        let thread = std::thread::Builder::new()
            .name("port-mapper".into())
            .spawn(move || run(port, &s, &rx))
            .ok();
        Self {
            status,
            stop,
            thread,
        }
    }

    pub fn status(&self) -> PortStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(PortStatus::Trying)
    }

    /// The port players outside join on, once open.
    pub fn external_port(&self) -> Option<u16> {
        match self.status() {
            PortStatus::Open { external, .. } => Some(external.port()),
            _ => None,
        }
    }
}

impl Drop for PortMapper {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn set(status: &Mutex<PortStatus>, s: PortStatus) {
    if let Ok(mut st) = status.lock() {
        *st = s;
    }
}

/// This machine's address towards `gateway` (where the router sends the traffic).
fn local_ip_towards(gateway: IpAddr) -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    s.connect((gateway, 1900)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}

fn upnp(port: u16) -> Result<(Mapping, SocketAddrV4), String> {
    let mut options = SearchOptions::default();
    options.timeout = Some(Duration::from_secs(3));
    options.single_search_timeout = Some(Duration::from_secs(3));
    let gw = igd_next::search_gateway(options).map_err(|e| format!("UPnP: {e}"))?;
    let local = local_ip_towards(gw.addr.ip()).ok_or("UPnP: no local address")?;
    let local = SocketAddr::from((local, port));
    let external = match gw.add_port(PortMappingProtocol::UDP, port, local, LEASE, DESCRIPTION) {
        Ok(()) => port,
        Err(igd_next::AddPortError::OnlyPermanentLeasesSupported) => gw
            .add_port(PortMappingProtocol::UDP, port, local, 0, DESCRIPTION)
            .map(|()| port)
            .map_err(|e| format!("UPnP: {e}"))?,
        // Taken (by another machine's game?): any free port outside.
        Err(_) => gw
            .add_any_port(PortMappingProtocol::UDP, local, LEASE, DESCRIPTION)
            .map_err(|e| format!("UPnP: {e}"))?,
    };
    let ip = match gw.get_external_ip() {
        Ok(IpAddr::V4(v4)) => v4,
        _ => Ipv4Addr::UNSPECIFIED,
    };
    Ok((
        Mapping::Upnp(Box::new(gw), external),
        SocketAddrV4::new(ip, external),
    ))
}

/// One NAT-PMP request and its answer (giving up after a few seconds).
fn natpmp_ask(
    n: &mut natpmp::Natpmp,
    send: impl FnOnce(&mut natpmp::Natpmp) -> natpmp::Result<()>,
) -> Result<natpmp::Response, String> {
    send(n).map_err(|e| format!("NAT-PMP: {e:?}"))?;
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        match n.read_response_or_retry() {
            Ok(r) => return Ok(r),
            Err(natpmp::Error::NATPMP_TRYAGAIN) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("NAT-PMP: {e:?}")),
        }
    }
}

fn natpmp_map(port: u16, lifetime: u32) -> Result<(Mapping, SocketAddrV4), String> {
    let mut n = natpmp::Natpmp::new().map_err(|e| format!("NAT-PMP: {e:?}"))?;
    let ip = match natpmp_ask(&mut n, |n| n.send_public_address_request())? {
        natpmp::Response::Gateway(g) => *g.public_address(),
        _ => return Err("NAT-PMP: unexpected answer".into()),
    };
    match natpmp_ask(&mut n, |n| {
        n.send_port_mapping_request(natpmp::Protocol::UDP, port, port, lifetime)
    })? {
        natpmp::Response::UDP(m) => Ok((Mapping::NatPmp, SocketAddrV4::new(ip, m.public_port()))),
        _ => Err("NAT-PMP: unexpected answer".into()),
    }
}

fn map(port: u16) -> Result<(Mapping, SocketAddrV4), String> {
    upnp(port).or_else(|a| natpmp_map(port, LEASE).map_err(|b| format!("{a}; {b}")))
}

fn run(port: u16, status: &Mutex<PortStatus>, stop: &Receiver<()>) {
    let mut mapping = None;
    loop {
        let wait = match map(port) {
            Ok((m, external)) => {
                let via = match m {
                    Mapping::Upnp(..) => "UPnP",
                    Mapping::NatPmp => "NAT-PMP",
                };
                set(status, PortStatus::Open { via, external });
                mapping = Some(m);
                Duration::from_secs(LEASE as u64 / 2)
            }
            Err(e) => {
                // Keep an older mapping's status if a renewal fails (it may still hold).
                if mapping.is_none() {
                    set(status, PortStatus::Failed(e));
                }
                Duration::from_secs(300)
            }
        };
        match stop.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => continue,
            _ => break,
        }
    }
    match mapping {
        Some(Mapping::Upnp(gw, external)) => {
            let _ = gw.remove_port(PortMappingProtocol::UDP, external);
        }
        Some(Mapping::NatPmp) => {
            // A lifetime of 0 removes the mapping.
            let _ = natpmp_map(port, 0);
        }
        None => {}
    }
}

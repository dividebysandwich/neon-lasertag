//! The master server's list and its answers, apart from the HTTP server around them
//! ([`http`]): [`List::handle`] takes a request and gives the response.
//!
//! A relay (for clients that can't reach a game server directly) is not here yet. SM89 had a
//! WebSocket-to-UDP one (`stealth-mission-89/crates/stealth-master/src/relay.rs`): it looks
//! the server up with [`List::addr_of`], passes on only packets starting with the transport's
//! magic (`LTAG` here), and caps relays per client address and their rate. A UDP forwarding
//! relay would go in a `relay` module of its own, started from `main.rs` beside the HTTP
//! workers and sharing the same `Arc<Mutex<List>>`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use lt_master_proto::{
    API_SERVER, API_SERVERS, EXPIRE_AFTER, REPORT_EVERY, Registered, ServerEntry, ServerReport,
    server_id,
};

pub mod http;
mod page;

/// Seconds a game server waits at least between two reports (more are turned down).
const MIN_REPORT_GAP: f64 = 5.0;
/// Servers listed at most, and per address.
const MAX_SERVERS: usize = 2000;
const MAX_PER_IP: usize = 16;
/// Largest request body taken.
pub const MAX_BODY: usize = 64 * 1024;

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    fn json<T: serde::Serialize>(status: u16, v: &T) -> Self {
        let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
        Response {
            status,
            content_type: "application/json",
            body,
        }
    }

    fn html(status: u16, body: String) -> Self {
        Response {
            status,
            content_type: "text/html; charset=utf-8",
            body: body.into_bytes(),
        }
    }

    /// The body as text.
    pub fn text(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    pub fn error(status: u16, why: &str) -> Self {
        Response::json(status, &serde_json::json!({ "error": why }))
    }
}

/// An address no game on the internet joins: this machine, the LAN, a container network.
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                // Carrier-grade NAT, 100.64.0.0/10.
                || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => {
            let s = v6.segments()[0];
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (s & 0xfe00) == 0xfc00
                || (s & 0xffc0) == 0xfe80
        }
    }
}

/// Where a request came from. From the internet: the peer. Through proxies on this machine
/// or its network (Caddy, cloudflared, a container's port): the client they forward,
/// Cloudflare's `CF-Connecting-IP` first, else the last address in `X-Forwarded-For` that is
/// not one of the proxies.
pub fn client_ip<'a>(peer: IpAddr, header: impl Fn(&str) -> Option<&'a str>) -> IpAddr {
    if !is_internal(peer) {
        return peer;
    }
    let parse = |s: &str| s.trim().parse::<IpAddr>().ok();
    if let Some(ip) = header("CF-Connecting-IP").and_then(parse) {
        return ip;
    }
    header("X-Forwarded-For")
        .and_then(|f| f.rsplit(',').filter_map(parse).find(|ip| !is_internal(*ip)))
        .unwrap_or(peer)
}

struct Listed {
    report: ServerReport,
    seen: Instant,
}

/// The servers there now, by address ("ip:port").
#[derive(Default)]
pub struct List {
    servers: HashMap<SocketAddr, Listed>,
}

impl List {
    /// One request: method, path (with the query), body, and the address it came from (after
    /// a reverse proxy: the client's).
    pub fn handle(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
        from: IpAddr,
        now: Instant,
    ) -> Response {
        self.expire(now);
        let path = path.split('?').next().unwrap_or("");
        let delete_prefix = format!("{API_SERVERS}/");
        match (method, path.trim_end_matches('/')) {
            ("GET", "" | "/index.html") => Response::html(200, page::render(&self.entries(now))),
            ("GET", API_SERVERS) => Response::json(200, &self.entries(now)),
            ("GET", p) if p.starts_with(API_SERVER) => {
                match self.entry(&p[API_SERVER.len()..], now) {
                    Some(e) => Response::json(200, &e),
                    None => Response::error(404, "not on the list"),
                }
            }
            ("POST", API_SERVERS) => self.report(body, from, now),
            ("DELETE", p) if p.starts_with(&delete_prefix) => {
                match p[delete_prefix.len()..].parse::<u16>() {
                    Ok(port) => {
                        let gone = self.servers.remove(&SocketAddr::new(from, port)).is_some();
                        Response::json(200, &serde_json::json!({ "removed": gone }))
                    }
                    Err(_) => Response::error(400, "not a port"),
                }
            }
            ("GET", "/health") => Response::json(200, &serde_json::json!({ "ok": true })),
            (_, API_SERVERS) => Response::error(405, "method not allowed"),
            _ => Response::error(404, "not found"),
        }
    }

    fn report(&mut self, body: &[u8], from: IpAddr, now: Instant) -> Response {
        if body.len() > MAX_BODY {
            return Response::error(413, "too large");
        }
        let report: ServerReport = match serde_json::from_slice(body) {
            Ok(r) => r,
            Err(e) => return Response::error(400, &format!("bad report: {e}")),
        };
        if report.port == 0 || report.name.trim().is_empty() {
            return Response::error(400, "a report needs a name and a port");
        }
        let addr = SocketAddr::new(from, report.port);
        if let Some(l) = self.servers.get(&addr)
            && now.duration_since(l.seen).as_secs_f64() < MIN_REPORT_GAP
        {
            return Response::error(429, "too often");
        }
        if !self.servers.contains_key(&addr) {
            let same_ip = self.servers.keys().filter(|a| a.ip() == from).count();
            if same_ip >= MAX_PER_IP || self.servers.len() >= MAX_SERVERS {
                return Response::error(429, "too many servers");
            }
        }
        self.servers.insert(
            addr,
            Listed {
                report: report.trimmed(),
                seen: now,
            },
        );
        Response::json(
            200,
            &Registered {
                every: REPORT_EVERY,
            },
        )
    }

    fn expire(&mut self, now: Instant) {
        let after = Duration::from_secs(EXPIRE_AFTER);
        self.servers
            .retain(|_, l| now.duration_since(l.seen) < after);
    }

    /// The address of the server with this id, if it is still listed (for a relay).
    pub fn addr_of(&mut self, id: &str, now: Instant) -> Option<SocketAddr> {
        self.expire(now);
        self.servers
            .iter()
            .find(|(addr, l)| server_id(&l.report.name, &addr.ip().to_string()) == id)
            .map(|(addr, _)| *addr)
    }

    /// The server with this id.
    fn entry(&self, id: &str, now: Instant) -> Option<ServerEntry> {
        self.entries(now).into_iter().find(|e| e.id == id)
    }

    /// The servers, the most players first.
    pub fn entries(&self, now: Instant) -> Vec<ServerEntry> {
        let mut out: Vec<ServerEntry> = self
            .servers
            .iter()
            .map(|(addr, l)| ServerEntry {
                id: server_id(&l.report.name, &addr.ip().to_string()),
                addr: addr.to_string(),
                age: now.duration_since(l.seen).as_secs(),
                report: l.report.clone(),
            })
            .collect();
        out.sort_by(|a, b| {
            b.report
                .players
                .len()
                .cmp(&a.report.players.len())
                .then_with(|| {
                    a.report
                        .name
                        .to_lowercase()
                        .cmp(&b.report.name.to_lowercase())
                })
                .then_with(|| a.addr.cmp(&b.addr))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lt_master_proto::ReportPlayer;

    fn report(name: &str, port: u16, players: usize) -> Vec<u8> {
        let r = ServerReport {
            name: name.into(),
            port,
            max_players: 16,
            map_seed: 42,
            players: (0..players)
                .map(|k| ReportPlayer {
                    name: format!("P{k}"),
                    team: if k % 2 == 0 { "red" } else { "blue" }.into(),
                })
                .collect(),
            ..Default::default()
        };
        serde_json::to_vec(&r).unwrap()
    }

    const A: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 5));
    const B: IpAddr = IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));

    #[test]
    fn servers_report_are_listed_and_expire() {
        let mut l = List::default();
        let t = Instant::now();
        let r = l.handle("POST", "/api/servers", &report("Alpha", 27515, 1), A, t);
        assert_eq!(r.status, 200, "{}", r.text());
        let reg: Registered = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(reg.every, REPORT_EVERY);
        let r = l.handle("POST", "/api/servers", &report("Bravo", 29000, 3), B, t);
        assert_eq!(r.status, 200);
        let r = l.handle("GET", "/api/servers", b"", A, t);
        let list: Vec<ServerEntry> = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].report.name, "Bravo", "the most players first");
        assert_eq!(list[0].addr, "[2001:db8::1]:29000");
        assert_eq!(list[1].addr, "203.0.113.5:27515");
        assert_eq!(list[1].report.map_seed, 42);
        // Too soon again; later it refreshes.
        let r = l.handle("POST", "/api/servers", &report("Alpha", 27515, 2), A, t);
        assert_eq!(r.status, 429);
        let later = t + Duration::from_secs(60);
        let r = l.handle("POST", "/api/servers", &report("Alpha", 27515, 2), A, later);
        assert_eq!(r.status, 200);
        // Bravo was not heard of for 90 s: gone.
        let r = l.handle("GET", "/api/servers", b"", A, t + Duration::from_secs(95));
        let list: Vec<ServerEntry> = serde_json::from_slice(&r.body).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].report.players.len(), 2);
    }

    #[test]
    fn one_address_lists_only_so_many_servers() {
        let mut l = List::default();
        let t = Instant::now();
        for k in 0..MAX_PER_IP as u16 {
            let r = l.handle("POST", "/api/servers", &report("S", 1000 + k, 0), A, t);
            assert_eq!(r.status, 200);
        }
        let r = l.handle("POST", "/api/servers", &report("S", 5000, 0), A, t);
        assert_eq!(r.status, 429);
        assert_eq!(
            l.handle("POST", "/api/servers", &report("S", 5000, 0), B, t)
                .status,
            200
        );
    }

    #[test]
    fn the_client_is_found_behind_proxies() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let headers = |h: &'static [(&'static str, &'static str)]| {
            move |n: &str| h.iter().find(|x| x.0.eq_ignore_ascii_case(n)).map(|x| x.1)
        };
        // From the internet: the peer, whatever it says.
        let spoof = headers(&[
            ("CF-Connecting-IP", "1.2.3.4"),
            ("X-Forwarded-For", "1.2.3.4"),
        ]);
        assert_eq!(client_ip(ip("203.0.113.5"), spoof), ip("203.0.113.5"));
        // cloudflared on the LAN, Caddy between: Cloudflare's header.
        let cf = headers(&[
            ("cf-connecting-ip", "81.189.183.15"),
            ("X-Forwarded-For", "192.168.178.2"),
        ]);
        assert_eq!(client_ip(ip("192.168.178.2"), cf), ip("81.189.183.15"));
        assert_eq!(client_ip(ip("::1"), cf), ip("81.189.183.15"));
        // Plain proxies: the last address not theirs.
        let xff = headers(&[("X-Forwarded-For", "9.9.9.9, 81.189.183.15, 172.17.0.1")]);
        assert_eq!(client_ip(ip("127.0.0.1"), xff), ip("81.189.183.15"));
        // Nothing forwarded: the peer.
        assert_eq!(client_ip(ip("10.0.0.7"), headers(&[])), ip("10.0.0.7"));
        assert!(is_internal(ip("100.100.1.1")) && is_internal(ip("fd00::1")));
        assert!(!is_internal(ip("2001:db8::1")) && !is_internal(ip("8.8.8.8")));
    }

    #[test]
    fn a_server_leaves_and_bad_reports_are_turned_down() {
        let mut l = List::default();
        let t = Instant::now();
        l.handle("POST", "/api/servers", &report("Alpha", 27515, 1), A, t);
        // Only from its own address.
        l.handle("DELETE", "/api/servers/27515", b"", B, t);
        assert_eq!(l.entries(t).len(), 1);
        l.handle("DELETE", "/api/servers/27515", b"", A, t);
        assert!(l.entries(t).is_empty());
        assert_eq!(l.handle("DELETE", "/api/servers/x", b"", A, t).status, 400);
        assert_eq!(l.handle("POST", "/api/servers", b"{nope", A, t).status, 400);
        assert_eq!(
            l.handle("POST", "/api/servers", &report("", 1, 0), A, t)
                .status,
            400
        );
        assert_eq!(l.handle("PUT", "/api/servers", b"", A, t).status, 405);
        assert_eq!(l.handle("GET", "/nothing", b"", A, t).status, 404);
    }

    #[test]
    fn one_server_by_its_id() {
        let mut l = List::default();
        let t = Instant::now();
        l.handle("POST", "/api/servers", &report("Alpha", 27515, 2), A, t);
        let id = l.entries(t)[0].id.clone();
        assert_eq!(id, server_id("Alpha", "203.0.113.5"));
        let one = l.handle("GET", &format!("/api/server/{id}"), b"", A, t);
        let e: ServerEntry = serde_json::from_slice(&one.body).unwrap();
        assert_eq!(e.report.players[1].team, "blue");
        assert_eq!(
            l.addr_of(&id, t),
            Some("203.0.113.5:27515".parse().unwrap())
        );
        assert_eq!(l.handle("GET", "/api/server/0000", b"", A, t).status, 404);
    }

    #[test]
    fn the_page_lists_the_servers_escaped() {
        let mut l = List::default();
        let t = Instant::now();
        l.handle(
            "POST",
            "/api/servers",
            &report("<b>Evil</b> & co", 27515, 2),
            A,
            t,
        );
        let r = l.handle("GET", "/", b"", A, t);
        assert_eq!(r.content_type, "text/html; charset=utf-8");
        assert!(r.text().contains("&lt;b&gt;Evil&lt;/b&gt; &amp; co"));
        assert!(!r.text().contains("<b>Evil"));
        assert!(r.text().contains("P1"));
        assert!(r.text().contains("203.0.113.5:27515"));
    }
}

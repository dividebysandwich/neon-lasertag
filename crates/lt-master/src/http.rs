//! The HTTP server around [`List`]: worker threads taking requests from a `tiny_http`
//! server. Plain HTTP; HTTPS comes from a reverse proxy in front (`deploy/`), whose
//! `CF-Connecting-IP` or `X-Forwarded-For` is believed when the request comes from this
//! machine or its LAN ([`client_ip`](crate::client_ip)).

use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use tiny_http::{Header, Method, Request, Server};

use crate::{List, MAX_BODY, Response, is_internal};

/// Serves `list` on `server` with `threads` workers (they end when the server is dropped
/// or unblocked).
pub fn spawn_workers(
    server: Arc<Server>,
    list: Arc<Mutex<List>>,
    threads: usize,
) -> Vec<JoinHandle<()>> {
    (0..threads.max(1))
        .map(|_| {
            let (server, list) = (server.clone(), list.clone());
            std::thread::spawn(move || {
                for req in server.incoming_requests() {
                    serve(req, &list);
                }
            })
        })
        .collect()
}

/// The client's address: the peer's, or behind proxies the one they forward.
fn client_ip(req: &Request) -> Option<IpAddr> {
    let peer = req.remote_addr()?.ip();
    Some(crate::client_ip(peer, |name| header(req, name)))
}

fn header<'r>(req: &'r Request, name: &str) -> Option<&'r str> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// One request.
pub fn serve(mut req: Request, list: &Mutex<List>) {
    let Some(from) = client_ip(&req) else { return };
    if is_internal(from) && req.method() == &Method::Post {
        eprintln!("{from} reports from a LAN address: a server not joinable from the internet");
    }
    let mut body = Vec::new();
    let ok = req.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body).is_ok();
    let method = req.method().as_str().to_string();
    let url = req.url().to_string();
    let r = if ok {
        match list.lock() {
            Ok(mut l) => l.handle(&method, &url, &body, from, Instant::now()),
            Err(_) => return,
        }
    } else {
        Response::error(400, "unreadable")
    };
    if r.status >= 400 && r.status != 404 {
        eprintln!("{from} {method} {url}: {} {}", r.status, r.text());
    }
    let header = |k: &str, v: &str| Header::from_bytes(k.as_bytes(), v.as_bytes()).ok();
    let content_type = r.content_type;
    let mut resp = tiny_http::Response::from_data(r.body).with_status_code(r.status);
    for h in [
        header("Content-Type", content_type),
        header("Cache-Control", "no-store"),
        header("Access-Control-Allow-Origin", "*"),
        header("X-Content-Type-Options", "nosniff"),
    ]
    .into_iter()
    .flatten()
    {
        resp.add_header(h);
    }
    let _ = req.respond(resp);
}

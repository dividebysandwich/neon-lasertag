//! The master server: game servers report to it, games list them (`lt-master-proto`).
//! It serves plain HTTP; HTTPS comes from a reverse proxy in front of it (`deploy/`).
//!
//! `lt-master [--bind <addr:port>] [--threads <n>] [--relay-bind <addr:port>|off]
//! [--relay-internal]` (default 0.0.0.0:27500, 4, 127.0.0.1:27501). The relay passes browsers'
//! WebSockets on to listed game servers over UDP (`relay`); `--relay-internal` lets it reach
//! servers on this machine and its LAN (for testing).

use std::sync::{Arc, Mutex};

use lt_master::{List, http, relay};
use lt_master_proto::{DEFAULT_BIND, DEFAULT_RELAY_BIND, RELAY_PATH};
use tiny_http::Server;

const USAGE: &str = "lt-master [--bind <addr:port>] [--threads <n>] [--relay-bind <addr:port>|off] [--relay-internal]
  (default 0.0.0.0:27500, 4, 127.0.0.1:27501)";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return;
    }
    let get = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let bind = get("--bind").unwrap_or_else(|| DEFAULT_BIND.into());
    let threads: usize = get("--threads")
        .and_then(|t| t.parse().ok())
        .unwrap_or(4)
        .max(1);
    let server = match Server::http(&bind) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("listening on {bind}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("lt-master listening on http://{bind}");
    let list = Arc::new(Mutex::new(List::default()));
    let relay_bind = get("--relay-bind").unwrap_or_else(|| DEFAULT_RELAY_BIND.into());
    if relay_bind != "off" {
        let listener = match std::net::TcpListener::bind(&relay_bind) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("relay listening on {relay_bind}: {e}");
                std::process::exit(1);
            }
        };
        eprintln!("relay listening on ws://{relay_bind}{RELAY_PATH}<server id>");
        let allow_internal = args.iter().any(|a| a == "--relay-internal");
        let list = list.clone();
        std::thread::spawn(move || relay::serve(listener, list, allow_internal));
    }
    for w in http::spawn_workers(server, list, threads) {
        let _ = w.join();
    }
}

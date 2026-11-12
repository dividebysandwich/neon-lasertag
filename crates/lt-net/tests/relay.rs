//! A player joins through the master server's relay, as the browser build does: a WebSocket to
//! the relay, which passes each message on as a datagram to the listed game server.

use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lt_master::List;
use lt_master_proto::{ServerReport, server_id};
use lt_net::client::{Client, ClientEvent};
use lt_net::server::Server;
use lt_net::transport::{Datagrams, Endpoint};
use lt_sim::bots::Bots;
use lt_sim::{PlayerCmd, Rules, Sim};
use tungstenite::{Message, WebSocket};

/// The browser's WebSocket, natively.
struct Ws(WebSocket<TcpStream>);

impl Datagrams for Ws {
    fn recv_from(&mut self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        let _ = self.0.flush();
        match self.0.read() {
            Ok(Message::Binary(b)) => {
                buf[..b.len()].copy_from_slice(&b);
                Ok((b.len(), "0.0.0.0:0".parse().unwrap()))
            }
            Ok(_) => Err(std::io::ErrorKind::WouldBlock.into()),
            Err(tungstenite::Error::Io(e)) => Err(e),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }

    fn send_to(&mut self, p: &[u8], _to: SocketAddr) {
        let _ = self.0.send(Message::Binary(p.to_vec().into()));
    }
}

#[test]
fn a_browser_player_joins_through_the_relay() {
    let mut sim = Sim::new(4, Rules::default());
    let mut bots = Bots::default();
    bots.fill(&mut sim, 4, 0.5);
    let mut server = Server::listen(0, "Relayed", 4).unwrap();
    let port = server.port().unwrap();

    // The server on the list, and the relay (to this machine: a test).
    let mut list = List::default();
    let report = ServerReport {
        name: "Relayed".into(),
        port,
        ..Default::default()
    };
    let local: IpAddr = "127.0.0.1".parse().unwrap();
    let body = serde_json::to_vec(&report).unwrap();
    let r = list.handle("POST", "/api/servers", &body, local, Instant::now());
    assert_eq!(r.status, 200);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let relay = listener.local_addr().unwrap();
    let list = Arc::new(Mutex::new(list));
    std::thread::spawn(move || lt_master::relay::serve(listener, list, true));

    let id = server_id("Relayed", "127.0.0.1");
    let url = lt_net::relay_url(&format!("http://{relay}"), &id);
    let stream = TcpStream::connect(relay).unwrap();
    let (ws, _) = tungstenite::client(url, stream).unwrap();
    ws.get_ref().set_nonblocking(true).unwrap();
    let net = Endpoint::client_over(Box::new(Ws(ws)), "0.0.0.0:0".parse().unwrap());
    let mut client = Client::over(net, "Browser");

    let (mut me, mut snaps) = (None, 0);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for ev in client.poll() {
            match ev {
                ClientEvent::Welcome { id, .. } => me = Some(id),
                ClientEvent::Snapshot(_) => snaps += 1,
                ClientEvent::Disconnected(why) => panic!("disconnected: {why}"),
                _ => {}
            }
        }
        client.send_input(PlayerCmd::default());
        bots.think(&mut sim);
        server.before_step(&mut sim, &mut bots);
        sim.step();
        let evs = sim.drain_events();
        server.after_step(&sim, &evs);
        if me.is_some() && snaps >= 10 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let me = me.expect("welcomed through the relay");
    assert!(snaps >= 10, "{snaps} snapshots");
    assert!(!sim.player(me).unwrap().bot);
}

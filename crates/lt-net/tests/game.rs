//! A client joins a server over UDP on localhost and plays a few seconds.

use std::time::{Duration, Instant};

use lt_net::client::{Client, ClientEvent};
use lt_net::server::Server;
use lt_sim::bots::Bots;
use lt_sim::{PlayerCmd, Rules, Sim};

#[test]
fn client_joins_and_receives_the_world() {
    let mut sim = Sim::new(4, Rules::default());
    let mut bots = Bots::default();
    bots.fill(&mut sim, 4, 0.5);
    let mut server = Server::listen(0, "test", 4).unwrap();
    let port = server.port().unwrap();
    let mut client = Client::connect(&format!("127.0.0.1:{port}"), "tester").unwrap();
    let mut me = None;
    let mut snaps = 0;
    let mut events = 0;
    // Count simulation steps rather than wall time: slow CI runners (macOS) oversleep and
    // step the sim far fewer times in a fixed window. The deadline is only a safety net.
    let mut steps = 0;
    let start = Instant::now();
    while steps < 240 && start.elapsed() < Duration::from_secs(60) {
        for ev in client.poll() {
            match ev {
                ClientEvent::Welcome { id, seed, .. } => {
                    assert_eq!(seed, 4);
                    me = Some(id);
                }
                ClientEvent::Snapshot(_) => snaps += 1,
                ClientEvent::Events(e) => events += e.len(),
                ClientEvent::Disconnected(why) => panic!("disconnected: {why}"),
                _ => {}
            }
        }
        client.send_input(PlayerCmd {
            stick: glam::Vec2::new(0.0, -1.0),
            ..Default::default()
        });
        bots.think(&mut sim);
        server.before_step(&mut sim, &mut bots);
        sim.step();
        let evs = sim.drain_events();
        server.after_step(&sim, &evs);
        if me.is_some() {
            steps += 1;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    let me = me.expect("welcomed");
    assert_eq!(sim.players.len(), 4, "a bot made room");
    let p = sim.player(me).unwrap();
    assert!(!p.bot);
    assert!(p.client_hits, "its client judges its hits");
    assert!(snaps > 50, "{snaps} snapshots");
    let _ = events;
    // The remote player moved.
    let spawn = sim.level.base_centre(p.team);
    assert!((p.feet - spawn).length() > 1.0, "{} vs {spawn}", p.feet);
}

/// Steps the server and polls `client` (its events kept) for `n` ticks.
fn run(
    server: &mut Server,
    sim: &mut Sim,
    bots: &mut Bots,
    client: &mut Client,
    seen: &mut Vec<ClientEvent>,
    n: usize,
) {
    for _ in 0..n {
        seen.extend(client.poll());
        client.send_input(PlayerCmd::default());
        bots.think(sim);
        server.before_step(sim, bots);
        sim.step();
        let evs = sim.drain_events();
        server.after_step(sim, &evs);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Joins a server with 4 bots; the server, sim, bots, client and its player.
fn joined() -> (Server, Sim, Bots, Client, lt_sim::PlayerId) {
    let mut sim = Sim::new(4, Rules::default());
    let mut bots = Bots::default();
    bots.fill(&mut sim, 4, 0.5);
    let mut server = Server::listen(0, "test", 4).unwrap();
    let port = server.port().unwrap();
    let mut client = Client::connect(&format!("127.0.0.1:{port}"), "tester").unwrap();
    let mut seen = Vec::new();
    let start = Instant::now();
    loop {
        run(&mut server, &mut sim, &mut bots, &mut client, &mut seen, 1);
        if let Some(id) = seen.iter().find_map(|e| match e {
            ClientEvent::Welcome { id, .. } => Some(*id),
            _ => None,
        }) {
            return (server, sim, bots, client, id);
        }
        assert!(start.elapsed() < Duration::from_secs(20), "welcomed");
    }
}

#[test]
fn a_player_who_quits_leaves_the_field_at_once() {
    let (mut server, mut sim, mut bots, mut client, me) = joined();
    assert!(sim.player(me).is_some_and(|p| p.name == "tester"));
    client.close();
    let mut seen = Vec::new();
    run(&mut server, &mut sim, &mut bots, &mut client, &mut seen, 20);
    // (The bot back in their place may have their id.)
    assert!(
        sim.players.iter().all(|p| p.name != "tester"),
        "gone well before the timeout: {:?}",
        server.log
    );
    assert_eq!(server.humans(), 0);
    assert_eq!(sim.players.len(), 4, "the bot is back");
}

#[test]
fn clients_are_told_when_the_host_leaves() {
    let (mut server, mut sim, mut bots, mut client, _) = joined();
    server.shutdown("the host left");
    let mut seen = Vec::new();
    for _ in 0..40 {
        seen.extend(client.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = (&mut sim, &mut bots);
    assert!(
        seen.iter()
            .any(|e| matches!(e, ClientEvent::Disconnected(why) if why == "the host left")),
        "{seen:?}"
    );
}

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
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(4) {
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
        client.send_input(PlayerCmd { stick: glam::Vec2::new(0.0, -1.0), ..Default::default() });
        bots.think(&mut sim);
        server.before_step(&mut sim, &mut bots);
        sim.step();
        let evs = sim.drain_events();
        server.after_step(&sim, &evs);
        std::thread::sleep(Duration::from_millis(16));
    }
    let me = me.expect("welcomed");
    assert_eq!(sim.players.len(), 4, "a bot made room");
    let p = sim.player(me).unwrap();
    assert!(!p.bot);
    assert!(snaps > 50, "{snaps} snapshots");
    let _ = events;
    // The remote player moved.
    let spawn = sim.level.base_centre(p.team);
    assert!((p.feet - spawn).length() > 1.0, "{} vs {spawn}", p.feet);
}

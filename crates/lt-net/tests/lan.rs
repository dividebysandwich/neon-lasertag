//! A server's LAN beacons are found by a browser on the same machine, and its going away is
//! noticed.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use lt_net::ServerReport;
use lt_net::lan::{LanAnnouncer, LanBrowser};
use lt_net::protocol::PROTOCOL;

fn wait_for(browser: &mut LanBrowser, what: impl Fn(&LanBrowser) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        browser.poll();
        if what(browser) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn beacons_are_found_and_go() {
    // A free port for this test's beacons (not the real one: a game may be running).
    let port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut browser = LanBrowser::on_port(port).unwrap();
    // A second browser on the same port (another game on this machine) can listen too.
    drop(LanBrowser::on_port(port).unwrap());
    let mut announcer = LanAnnouncer::to_port(port);
    announcer.update(ServerReport {
        protocol: PROTOCOL,
        name: "LAN test".into(),
        port: 27999,
        bots: 3,
        ..Default::default()
    });
    assert!(
        wait_for(&mut browser, |b| b.servers().len() == 1),
        "no beacon heard"
    );
    let s = &browser.servers()[0];
    assert_eq!(s.report.name, "LAN test");
    assert!(s.addr.ends_with(":27999"), "{}", s.addr);
    assert!(s.joinable());
    announcer.stop();
    assert!(
        wait_for(&mut browser, |b| b.servers().is_empty()),
        "still listed after it stopped"
    );
}

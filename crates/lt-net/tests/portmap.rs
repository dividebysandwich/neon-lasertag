//! Opens a port on this network's router (if it speaks UPnP or NAT-PMP). Needs a real
//! router: `cargo test -p lt-net --test portmap -- --ignored --nocapture`.

use std::time::{Duration, Instant};

use lt_net::{PortMapper, PortStatus};

#[test]
#[ignore]
fn opens_a_port_on_the_router() {
    let m = PortMapper::open(27455);
    let until = Instant::now() + Duration::from_secs(15);
    while m.status() == PortStatus::Trying && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("{:?}", m.status());
    assert!(
        matches!(m.status(), PortStatus::Open { .. }),
        "{:?}",
        m.status()
    );
}

/// Lists the router's UDP port mappings (to see that the game's are gone after it quits):
/// `cargo test -p lt-net --test portmap list -- --ignored --nocapture`.
#[test]
#[ignore]
fn list_router_mappings() {
    let gw = igd_next::search_gateway(Default::default()).expect("no UPnP router");
    for i in 0.. {
        match gw.get_generic_port_mapping_entry(i) {
            Ok(e) => println!(
                "{:?} {} -> {}:{} {}",
                e.protocol,
                e.external_port,
                e.internal_client,
                e.internal_port,
                e.port_mapping_description
            ),
            Err(_) => break,
        }
    }
}

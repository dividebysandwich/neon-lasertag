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

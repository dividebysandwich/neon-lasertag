//! A real master server (lt-master's HTTP server on an ephemeral port) and a real
//! [`Reporter`]: the report shows up in the list, and leaves it when the reporter stops.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lt_master::{List, http};
use lt_net::{ReportPlayer, Reporter, ServerEntry, ServerReport, fetch_list};

fn master() -> String {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = server.server_addr().to_ip().unwrap().port();
    http::spawn_workers(server, Arc::new(Mutex::new(List::default())), 2);
    format!("http://127.0.0.1:{port}")
}

fn wait_for(url: &str, what: impl Fn(&[ServerEntry]) -> bool) -> Vec<ServerEntry> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let list = fetch_list(url).unwrap();
        if what(&list) || Instant::now() > until {
            return list;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn report(port: u16) -> ServerReport {
    ServerReport {
        name: "Test arena".into(),
        port,
        protocol: 1,
        map_seed: 0xdead_beef,
        max_players: 8,
        bots: 2,
        scores: [3, 5],
        time_left_s: 300,
        players: vec![ReportPlayer {
            name: "Ann".into(),
            team: "red".into(),
        }],
        ..Default::default()
    }
}

#[test]
fn a_pushed_report_is_listed_and_removed_on_stop() {
    let url = master();
    let mut reporter = Reporter::new(&url);
    reporter.update(report(27515));
    let list = wait_for(&url, |l| !l.is_empty());
    assert_eq!(list.len(), 1, "{:?}", reporter.take_log());
    assert_eq!(list[0].addr, "127.0.0.1:27515");
    assert_eq!(list[0].report.map_seed, 0xdead_beef);
    assert_eq!(list[0].report.scores, [3, 5]);
    assert_eq!(list[0].report.players[0].team, "red");
    assert!(
        reporter
            .take_log()
            .iter()
            .any(|l| l.starts_with("listed on"))
    );
    reporter.stop();
    assert!(fetch_list(&url).unwrap().is_empty(), "DELETE on stop");
}

#[test]
fn a_closure_source_is_reported() {
    let url = master();
    let reporter = Reporter::with_source(&url, || Some(report(27516)));
    let list = wait_for(&url, |l| !l.is_empty());
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].addr, "127.0.0.1:27516");
    drop(reporter);
    assert!(fetch_list(&url).unwrap().is_empty(), "DELETE on drop");
}

#[test]
fn an_unreachable_master_is_an_error() {
    assert!(fetch_list("http://127.0.0.1:1").is_err());
}

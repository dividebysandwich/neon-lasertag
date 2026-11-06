//! The public server list (`lt-master-proto`): a server's reports to the master server on a
//! thread of their own ([`Reporter`]), and a game's look at the list ([`fetch_list`]).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Context;
use lt_master_proto::{API_SERVERS, REPORT_EVERY, Registered, ServerEntry, ServerReport};
use ureq::config::IpFamily;

pub use lt_master_proto::DEFAULT_MASTER;

/// Seconds a request to the master server may take (off the simulation's thread: a slow
/// network costs nothing but the wait), and its name lookup and connection each.
const TIMEOUT: u64 = 20;
const PHASE_TIMEOUT: u64 = 5;
/// After a failed report, the next try (doubling with each failure, up to `REPORT_EVERY`): one
/// lost request must not keep the server away longer than the master's `EXPIRE_AFTER`.
const RETRY: u64 = 5;
/// Seconds at least between reports when the players change.
const SOON: u64 = 10;
/// How often a [`Reporter::with_source`] asks its closure for the server's state.
const POLL_SOURCE: Duration = Duration::from_secs(1);

enum Msg {
    Report(Box<ServerReport>),
    Stop,
}

/// Where the reporting thread gets the server's state: pushed with [`Reporter::update`], or
/// pulled from a closure (`None`: nothing new).
type Source = Box<dyn FnMut() -> Option<ServerReport> + Send>;

/// Reports a server to the master server every `REPORT_EVERY` seconds (at once the first
/// time, and within `SOON` seconds when the player list changes), and takes it off the list
/// when it stops (or is dropped).
pub struct Reporter {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    /// What happened (for a dedicated server's log).
    log: Arc<Mutex<Vec<String>>>,
}

impl Reporter {
    /// Reports to `master` ("https://host" or "http://127.0.0.1:27500"); the server's state
    /// comes with [`update`](Self::update).
    pub fn new(master: &str) -> Self {
        Self::spawn(master, None)
    }

    /// Reports to `master`, asking `source` for the server's state about once a second
    /// (on the reporting thread; `None` keeps the last one).
    pub fn with_source(
        master: &str,
        source: impl FnMut() -> Option<ServerReport> + Send + 'static,
    ) -> Self {
        Self::spawn(master, Some(Box::new(source)))
    }

    fn spawn(master: &str, source: Option<Source>) -> Self {
        let (tx, rx) = mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let (url, l) = (master.trim_end_matches('/').to_string(), log.clone());
        let thread = std::thread::Builder::new()
            .name("master-server".into())
            .spawn(move || run(&url, &rx, source, &l))
            .ok();
        Self { tx, thread, log }
    }

    /// The server as it is now (sent with the next report).
    pub fn update(&self, report: ServerReport) {
        let _ = self.tx.send(Msg::Report(Box::new(report)));
    }

    /// What happened since the last call ("listed on ...", failures).
    pub fn take_log(&self) -> Vec<String> {
        self.log.lock().map(|mut l| std::mem::take(&mut *l)).unwrap_or_default()
    }

    /// Off the list (waits for the master server a moment).
    pub fn stop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A reporting server's agent speaks IPv4 to the master server: the address it sees is the
/// one listed, and game servers listen on IPv4 only.
/// A fresh connection each time: at a report every half minute a kept one saves nothing, and
/// one a NAT box forgot would only hang the next report.
fn new_agent(timeout: u64, ip: IpFamily) -> ureq::Agent {
    let phase = Some(Duration::from_secs(PHASE_TIMEOUT.min(timeout)));
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .timeout_resolve(phase)
        .timeout_connect(phase)
        .max_idle_connections(0)
        .ip_family(ip)
        .user_agent(concat!("lasertag/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn run(url: &str, rx: &Receiver<Msg>, mut source: Option<Source>, log: &Mutex<Vec<String>>) {
    let say = |s: String| {
        if let Ok(mut l) = log.lock() {
            l.push(s);
        }
    };
    let agent = new_agent(TIMEOUT, IpFamily::Ipv4Only);
    let mut latest: Option<ServerReport> = None;
    let mut next = Instant::now();
    let mut every = REPORT_EVERY;
    let mut listed = false;
    let mut fails = 0u32;
    // The players as last reported, and when.
    let mut sent_players = Vec::new();
    let mut sent_at = Instant::now();
    let mut polled_at: Option<Instant> = None;
    loop {
        // A closure source is asked about once a second (at once the first time).
        if let Some(f) = source.as_mut()
            && polled_at.is_none_or(|t| t.elapsed() >= POLL_SOURCE)
        {
            polled_at = Some(Instant::now());
            if let Some(r) = f() {
                let changed = listed
                    && r.players != sent_players
                    && sent_at.elapsed() >= Duration::from_secs(SOON);
                latest = Some(r);
                if changed {
                    next = Instant::now();
                }
            }
        }
        let mut wait = if latest.is_some() {
            next.saturating_duration_since(Instant::now())
        } else {
            Duration::from_secs(3600)
        };
        if let Some(t) = polled_at.filter(|_| source.is_some()) {
            wait = wait.min(POLL_SOURCE.saturating_sub(t.elapsed()));
        }
        match rx.recv_timeout(wait) {
            Ok(Msg::Report(r)) => {
                // Somebody came or went: the list learns soon, not at the next report.
                let changed = listed
                    && r.players != sent_players
                    && sent_at.elapsed() >= Duration::from_secs(SOON);
                latest = Some(*r);
                if Instant::now() < next && !changed {
                    continue;
                }
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() < next {
                    continue;
                }
            }
        }
        let Some(r) = &latest else { continue };
        sent_at = Instant::now();
        sent_players = r.players.clone();
        match agent.post(format!("{url}{API_SERVERS}")).send_json(r) {
            Ok(mut resp) => {
                fails = 0;
                if let Ok(reg) = resp.body_mut().read_json::<Registered>() {
                    every = reg.every.clamp(10, 300);
                }
                if !listed {
                    say(format!("listed on {url}"));
                    listed = true;
                }
                next = Instant::now() + Duration::from_secs(every);
            }
            Err(e) => {
                let retry = (RETRY << fails.min(3)).min(REPORT_EVERY);
                fails += 1;
                say(format!("the server list ({url}): {e} (again in {retry} s)"));
                listed = false;
                next = Instant::now() + Duration::from_secs(retry);
            }
        }
    }
    if let Some(r) = latest.filter(|_| listed) {
        let _ = new_agent(2, IpFamily::Ipv4Only)
            .delete(format!("{url}{API_SERVERS}/{}", r.port))
            .call();
    }
}

/// The servers on the list at `master`.
pub fn fetch_list(master: &str) -> anyhow::Result<Vec<ServerEntry>> {
    let url = format!("{}{API_SERVERS}", master.trim_end_matches('/'));
    new_agent(TIMEOUT, IpFamily::Any)
        .get(&url)
        .call()
        .with_context(|| format!("fetching {url}"))?
        .body_mut()
        .read_json::<Vec<ServerEntry>>()
        .with_context(|| format!("reading {url}"))
}

/// Where an answer to [`fetch_list_into`] goes.
pub type ListSlot = Arc<Mutex<Option<Result<Vec<ServerEntry>, String>>>>;

/// Fetches the list at `master` on a thread of its own, into `slot` when it is there (for a
/// game's menu: poll the slot each frame).
pub fn fetch_list_into(master: &str, slot: ListSlot) {
    let master = master.to_string();
    std::thread::spawn(move || {
        let list = fetch_list(&master).map_err(|e| format!("{e:#}"));
        if let Ok(mut s) = slot.lock() {
            *s = Some(list);
        }
    });
}

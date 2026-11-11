//! The lasertag dedicated server: runs matches on UDP with bots filling the teams, listed on
//! the master server unless `--private` and announced on the local network unless `--no-lan`.
//!
//!     lt-server [--port 27420] [--name NAME] [--seed N] [--bots 8] [--difficulty normal]
//!               [--arena small|medium|large|random] [--mode classic|megatarget|prism|infection]
//!               [--master URL] [--private] [--no-lan] [--lock-teams]
//!               [--secondary random|shoot|relay|circuit|pattern|sequence]

use std::time::{Duration, Instant};

use lt_net::protocol::DEFAULT_PORT;
use lt_net::server::Server;
use lt_net::{DEFAULT_MASTER, LanAnnouncer, Reporter};
use lt_sim::bots::{Bots, Difficulty};
use lt_sim::level::{GameMode, SizeChoice};
use lt_sim::{Rules, Sim, TICK};

struct Args {
    port: u16,
    name: String,
    seed: Option<u64>,
    bots: usize,
    difficulty: Difficulty,
    /// The arenas' size (`Random`: a new one every match).
    arena: SizeChoice,
    mode: GameMode,
    master: String,
    private: bool,
    /// Announce the server on the local network.
    lan: bool,
    /// Nobody changes sides during a match (`Rules::team_switch`).
    lock_teams: bool,
    /// Match modifiers (`Rules::modifiers`).
    modifiers: Vec<lt_sim::ArenaEvent>,
    /// Megatarget mode: how the secondary targets charge players (`Rules::secondary`).
    secondary: lt_sim::rules::SecondaryKind,
}

fn args() -> Args {
    let mut a = Args {
        port: DEFAULT_PORT,
        name: "Lasertag server".into(),
        seed: None,
        bots: 8,
        difficulty: Difficulty::default(),
        arena: SizeChoice::default(),
        mode: GameMode::Megatarget,
        master: DEFAULT_MASTER.into(),
        private: false,
        lan: true,
        lock_teams: false,
        modifiers: Vec::new(),
        secondary: lt_sim::rules::SecondaryKind::Random,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => a.port = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.port),
            "--name" => a.name = it.next().unwrap_or(a.name),
            "--seed" => a.seed = it.next().and_then(|v| v.parse().ok()),
            "--bots" => a.bots = it.next().and_then(|v| v.parse().ok()).unwrap_or(a.bots),
            "--difficulty" => match it.next().as_deref().and_then(Difficulty::parse) {
                Some(d) => a.difficulty = d,
                None => eprintln!("--difficulty takes easy, normal or hard"),
            },
            "--arena" => match it.next().as_deref().and_then(SizeChoice::parse) {
                Some(s) => a.arena = s,
                None => eprintln!("--arena takes small, medium, large or random"),
            },
            "--mode" => match it.next().as_deref().and_then(GameMode::parse) {
                Some(m) => a.mode = m,
                None => eprintln!("--mode takes classic, megatarget, prism or infection"),
            },
            "--secondary" => match it
                .next()
                .as_deref()
                .and_then(lt_sim::rules::SecondaryKind::parse)
            {
                Some(k) => a.secondary = k,
                None => {
                    eprintln!(
                        "--secondary takes random, shoot, relay, circuit, pattern or sequence"
                    )
                }
            },
            "--master" => a.master = it.next().unwrap_or(a.master),
            "--private" => a.private = true,
            "--no-lan" => a.lan = false,
            "--lock-teams" => a.lock_teams = true,
            "--modifiers" => match it
                .next()
                .as_deref()
                .map(lt_sim::ArenaEvent::parse_modifiers)
            {
                Some(Some(m)) => a.modifiers = m,
                _ => eprintln!("--modifiers takes a comma list of dark, lowgrav and mirror"),
            },
            "-h" | "--help" => {
                println!(
                    "lt-server [--port N] [--name NAME] [--seed N] [--bots N] [--difficulty easy|normal|hard] \
                     [--arena small|medium|large|random] [--mode classic|megatarget|prism|infection] [--master URL] [--private] [--no-lan] [--lock-teams] \
                     [--modifiers dark,lowgrav,mirror] [--secondary random|shoot|relay|circuit|pattern|sequence]"
                );
                std::process::exit(0);
            }
            other => eprintln!("unknown argument {other}"),
        }
    }
    a
}

/// Seconds of free play before a server's first match (cut short once `WARMUP_PLAYERS` people
/// are in).
const WARMUP: f32 = 30.0;
const WARMUP_PLAYERS: usize = 2;

/// A match; the first one (`warmup`) begins with a warmup, the next ones with a countdown.
fn new_match(seed: u64, args: &Args, warmup: bool) -> (Sim, Bots) {
    let mut sim = Sim::new(
        seed,
        Rules {
            arena: args.arena.pick(rand::random()),
            warmup: if warmup { WARMUP } else { 0.0 },
            team_switch: !args.lock_teams,
            modifiers: args.modifiers.clone(),
            secondary: args.secondary,
            ..Rules::for_mode(args.mode)
        },
    );
    let mut b = Bots::default();
    b.fill(&mut sim, args.bots, args.difficulty.skill());
    sim.begin();
    (sim, b)
}

fn main() {
    let args = args();
    let mut seed = args.seed.unwrap_or_else(rand::random);
    let (mut sim, mut bots) = new_match(seed, &args, true);
    let mut server = match Server::listen(args.port, &args.name, seed) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("can't listen on UDP port {}: {e}", args.port);
            std::process::exit(1);
        }
    };
    println!(
        "lasertag server \"{}\" on UDP {} (arena {seed}, {} ({}), {} bots, {})",
        args.name,
        args.port,
        sim.rules.arena.name(),
        args.arena.name(),
        args.bots,
        args.difficulty.name()
    );
    let reporter = (!args.private).then(|| Reporter::new(&args.master));
    let lan = args.lan.then(LanAnnouncer::new);
    let tick = Duration::from_secs_f32(TICK);
    let mut next = Instant::now();
    let mut over_since: Option<Instant> = None;
    let mut last_report = Instant::now() - Duration::from_secs(60);
    loop {
        let humans = server.humans();
        server.before_step(&mut sim, &mut bots);
        // Bots only play while someone watches.
        if server.humans() >= WARMUP_PLAYERS {
            sim.end_warmup();
        }
        if humans > 0 || server.humans() > 0 {
            bots.think(&mut sim);
            sim.step();
        }
        let events = sim.drain_events();
        server.after_step(&sim, &events);
        for line in server.log.drain(..) {
            println!("{line}");
        }
        if sim.over {
            let since = *over_since.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(10) {
                println!("match over {:?}; next arena", sim.scores);
                seed = rand::random();
                let (mut next_sim, mut next_bots) = new_match(seed, &args, false);
                server.move_to(&mut next_sim, &mut next_bots, &sim);
                sim = next_sim;
                bots = next_bots;
                println!("arena {seed}, {}", sim.rules.arena.name());
                over_since = None;
            }
        }
        if last_report.elapsed() > Duration::from_secs(2) {
            let report = server.report(&sim, &bots, args.port, true);
            if let Some(l) = &lan {
                l.update(report.clone());
            }
            if let Some(r) = &reporter {
                r.update(report);
            }
            last_report = Instant::now();
        }
        if let Some(r) = &reporter {
            for line in r.take_log() {
                println!("master: {line}");
            }
        }
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(250) {
            next = now;
        }
    }
}

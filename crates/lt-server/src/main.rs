//! The lasertag dedicated server: runs matches on UDP with bots filling the teams, listed on
//! the master server unless `--private`.
//!
//!     lt-server [--port 27420] [--name NAME] [--seed N] [--bots 8] [--difficulty normal]
//!               [--arena small|medium|large|random] [--master URL] [--private]

use std::time::{Duration, Instant};

use lt_net::master_proto::{ReportPlayer, ServerReport, TEAM_BLUE, TEAM_RED};
use lt_net::protocol::{DEFAULT_PORT, PROTOCOL};
use lt_net::server::Server;
use lt_net::{DEFAULT_MASTER, Reporter};
use lt_sim::bots::{Bots, Difficulty};
use lt_sim::level::SizeChoice;
use lt_sim::{Rules, Sim, TICK, Team};

struct Args {
    port: u16,
    name: String,
    seed: Option<u64>,
    bots: usize,
    difficulty: Difficulty,
    /// The arenas' size (`Random`: a new one every match).
    arena: SizeChoice,
    master: String,
    private: bool,
}

fn args() -> Args {
    let mut a = Args {
        port: DEFAULT_PORT,
        name: "Lasertag server".into(),
        seed: None,
        bots: 8,
        difficulty: Difficulty::default(),
        arena: SizeChoice::default(),
        master: DEFAULT_MASTER.into(),
        private: false,
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
            "--master" => a.master = it.next().unwrap_or(a.master),
            "--private" => a.private = true,
            "-h" | "--help" => {
                println!(
                    "lt-server [--port N] [--name NAME] [--seed N] [--bots N] [--difficulty easy|normal|hard] \
                     [--arena small|medium|large|random] [--master URL] [--private]"
                );
                std::process::exit(0);
            }
            other => eprintln!("unknown argument {other}"),
        }
    }
    a
}

fn new_match(seed: u64, bots: usize, difficulty: Difficulty, arena: SizeChoice) -> (Sim, Bots) {
    let mut sim = Sim::new(
        seed,
        Rules {
            arena: arena.pick(rand::random()),
            ..Rules::default()
        },
    );
    let mut b = Bots::default();
    b.fill(&mut sim, bots, difficulty.skill());
    (sim, b)
}

fn report(server: &Server, sim: &Sim, port: u16) -> ServerReport {
    ServerReport {
        protocol: PROTOCOL,
        game_version: env!("CARGO_PKG_VERSION").into(),
        name: server.name.clone(),
        port,
        dedicated: true,
        map_seed: sim.level.seed,
        players: sim
            .players
            .iter()
            .filter(|p| !p.bot)
            .map(|p| ReportPlayer {
                name: p.name.clone(),
                team: if p.team == Team::Red {
                    TEAM_RED.into()
                } else {
                    TEAM_BLUE.into()
                },
            })
            .collect(),
        max_players: lt_sim::sim::MAX_PLAYERS as u16,
        bots: sim.players.iter().filter(|p| p.bot).count() as u16,
        scores: sim.scores,
        time_left_s: sim.time_left as u32,
    }
}

fn main() {
    let args = args();
    let mut seed = args.seed.unwrap_or_else(rand::random);
    let (mut sim, mut bots) = new_match(seed, args.bots, args.difficulty, args.arena);
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
    let tick = Duration::from_secs_f32(TICK);
    let mut next = Instant::now();
    let mut over_since: Option<Instant> = None;
    let mut last_report = Instant::now() - Duration::from_secs(60);
    loop {
        let humans = server.humans();
        server.before_step(&mut sim, &mut bots);
        // Bots only play while someone watches.
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
                let (mut next_sim, mut next_bots) =
                    new_match(seed, args.bots, args.difficulty, args.arena);
                server.move_to(&mut next_sim, &mut next_bots, &sim);
                sim = next_sim;
                bots = next_bots;
                println!("arena {seed}, {}", sim.rules.arena.name());
                over_since = None;
            }
        }
        if let Some(r) = &reporter {
            if last_report.elapsed() > Duration::from_secs(5) {
                r.update(report(&server, &sim, args.port));
                last_report = Instant::now();
            }
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

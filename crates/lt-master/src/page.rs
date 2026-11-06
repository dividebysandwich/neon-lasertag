//! The server list as a plain web page: a table row per server (name, players, score, time
//! left, map seed, address) with its players by team below. It reloads itself every few
//! seconds.

use std::fmt::Write;

use lt_master_proto::{ServerEntry, TEAM_BLUE, TEAM_RED};

/// Seconds between the page's reloads.
const REFRESH_S: u32 = 10;

/// Text for HTML.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = r#"
:root { --bg: #f4f4f6; --panel: #fff; --ink: #1b1d22; --dim: #646a75; --line: #d8dbe0;
  --red: #c0392b; --blue: #2465c9; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #0c0e12; --panel: #141821; --ink: #dfe3ea; --dim: #8a93a3; --line: #262c38;
    --red: #ff6b5a; --blue: #6aa2ff; }
}
body { margin: 0; background: var(--bg); color: var(--ink);
  font: 15px/1.45 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 1000px; margin: 0 auto; padding: 28px 16px 48px; }
h1 { margin: 0 0 4px; font: 700 22px/1.2 ui-monospace, Menlo, Consolas, monospace;
  letter-spacing: .06em; text-transform: uppercase; }
.summary { color: var(--dim); margin-bottom: 18px; }
table { border-collapse: collapse; width: 100%; background: var(--panel);
  border: 1px solid var(--line); font-variant-numeric: tabular-nums; }
th, td { text-align: left; padding: 6px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
th { color: var(--dim); font-weight: 500; font-size: 12px; text-transform: uppercase; letter-spacing: .06em; }
.name { font-weight: 600; overflow-wrap: anywhere; }
.addr, .seed { font: 13px ui-monospace, Menlo, Consolas, monospace; color: var(--dim); }
.players { font-size: 13px; color: var(--dim); }
.red { color: var(--red); } .blue { color: var(--blue); }
.empty { padding: 28px 16px; text-align: center; color: var(--dim);
  border: 1px dashed var(--line); background: var(--panel); }
footer { margin-top: 20px; color: var(--dim); font-size: 13px; }
"#;

/// The page for these servers.
pub fn render(servers: &[ServerEntry]) -> String {
    let players: usize = servers.iter().map(|s| s.report.players.len()).sum();
    let mut h = String::new();
    let _ = write!(
        h,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta http-equiv=\"refresh\" content=\"{REFRESH_S}\">\
         <title>Lasertag servers</title><style>{STYLE}</style></head><body><main>\
         <h1>Lasertag servers</h1><div class=\"summary\">{} server{}, {players} player{}</div>",
        servers.len(),
        if servers.len() == 1 { "" } else { "s" },
        if players == 1 { "" } else { "s" },
    );
    if servers.is_empty() {
        h.push_str("<div class=\"empty\">No servers right now.</div>");
    } else {
        h.push_str(
            "<table><tr><th>Server</th><th>Players</th><th>Score</th><th>Time left</th>\
             <th>Map</th><th>Address</th></tr>",
        );
        for s in servers {
            row(&mut h, s);
        }
        h.push_str("</table>");
    }
    h.push_str(
        "<footer>JSON: <code>/api/servers</code>, <code>/api/server/&lt;id&gt;</code></footer>\
         </main></body></html>",
    );
    h
}

fn row(h: &mut String, s: &ServerEntry) {
    let r = &s.report;
    let bots = if r.bots > 0 {
        format!(" +{} bots", r.bots)
    } else {
        String::new()
    };
    let _ = write!(
        h,
        "<tr id=\"{}\"><td><span class=\"name\">{}</span>{}<div class=\"players\">",
        esc(&s.id),
        esc(&r.name),
        if r.dedicated {
            " <small>(dedicated)</small>"
        } else {
            ""
        },
    );
    for team in [TEAM_RED, TEAM_BLUE] {
        let names: Vec<String> = r
            .players
            .iter()
            .filter(|p| p.team == team)
            .map(|p| esc(&p.name))
            .collect();
        if !names.is_empty() {
            let _ = write!(h, "<div class=\"{team}\">{}</div>", names.join(", "));
        }
    }
    let others: Vec<String> = r
        .players
        .iter()
        .filter(|p| p.team != TEAM_RED && p.team != TEAM_BLUE)
        .map(|p| esc(&p.name))
        .collect();
    if !others.is_empty() {
        let _ = write!(h, "<div>{}</div>", others.join(", "));
    }
    let _ = write!(
        h,
        "</div></td><td>{}/{}{bots}</td>\
         <td><span class=\"red\">{}</span> : <span class=\"blue\">{}</span></td>\
         <td>{}:{:02}</td><td class=\"seed\">{:016x}</td><td class=\"addr\">{}<br>v{} p{}</td></tr>",
        r.players.len(),
        r.max_players,
        r.scores[0],
        r.scores[1],
        r.time_left_s / 60,
        r.time_left_s % 60,
        r.map_seed,
        esc(&s.addr),
        esc(&r.game_version),
        r.protocol,
    );
}
